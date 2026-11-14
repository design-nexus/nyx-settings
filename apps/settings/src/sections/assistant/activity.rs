//! What the assistant is asking, doing and has done: requests grouped by day on the
//! left, the chosen one's steps on the right. Redrawn while open when any of it changes.

use super::{chip, client, dim, more, when};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    /// The request being shown, kept while the page is redrawn.
    static PICKED: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Pending approvals, task states and the newest audit record: the page redraws when it changes.
pub fn signature() -> String {
    let pending = client::status().map(|s| s["pending_approvals"].to_string()).unwrap_or_default();
    let tasks = client::run(&["tasks", "--limit", "10", "--json"]).unwrap_or_default();
    let audit = client::run(&["audit", "--limit", "1", "--json"]).unwrap_or_default();
    format!("{pending}\n{tasks}\n{audit}")
}

fn state_label(state: &str) -> &str {
    match state {
        "queued" => "Waiting to start",
        "planning" => "Planning",
        "running" => "Running",
        "waiting_for_approval" => "Waiting for you",
        "paused" => "Paused",
        "cancelling" => "Stopping",
        "cancelled" => "Stopped",
        "failed" => "Didn't work",
        "partially_completed" => "Partly done",
        "completed" => "Done",
        s => s,
    }
}

fn active(state: &str) -> bool {
    matches!(state, "queued" | "planning" | "running" | "waiting_for_approval" | "paused")
}

/// The icon and colour class of a state.
fn state_mark(state: &str) -> (&'static str, &'static str) {
    match state {
        "completed" => ("object-select-symbolic", "asst-done"),
        "partially_completed" => ("dialog-information-symbolic", "asst-part"),
        "failed" => ("window-close-symbolic", "asst-failed"),
        "cancelled" | "cancelling" => ("process-stop-symbolic", "asst-failed"),
        "waiting_for_approval" | "paused" => ("alarm-symbolic", "asst-part"),
        _ => ("view-refresh-symbolic", "asst-running"),
    }
}

/// "3.3 s", "1 min 4 s".
fn duration(ms: u64) -> String {
    let s = ms as f64 / 1000.0;
    if s < 60.0 { format!("{s:.1} s") } else { format!("{} min {} s", (s / 60.0) as u64, (s % 60.0) as u64) }
}

fn steps(n: u64) -> String {
    if n == 1 { "1 step".into() } else { format!("{n} steps") }
}

/// How it went, in a few words.
fn outcome(t: &Value) -> String {
    let (calls, max) = (t["calls"].as_u64().unwrap_or(0), t["max_calls"].as_u64().unwrap_or(0));
    let took = duration(t["active_ms"].as_u64().unwrap_or(0));
    match t["state"].as_str().unwrap_or("") {
        "completed" => format!("{} · {took}", steps(calls)),
        "partially_completed" if max > 0 && calls >= max => format!("{calls} of {max} steps · ran out of steps"),
        "partially_completed" => format!("{calls} of {max} steps · partly done"),
        "failed" if calls == 0 => "Didn't work".into(),
        "failed" => format!("Didn't work after {}", steps(calls)),
        "cancelled" => "Stopped".into(),
        s => format!("{} · {calls} of {max} steps", state_label(s)),
    }
}

/// Today, yesterday, a weekday this week, else the date.
fn day_of(ms: u64) -> String {
    let Ok(t) = glib::DateTime::from_unix_local((ms / 1000) as i64) else { return String::new() };
    let Ok(now) = glib::DateTime::now_local() else { return String::new() };
    let midnight = |d: &glib::DateTime| glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 0, 0, 0.0).map(|x| x.to_unix()).unwrap_or(0);
    // Rounded, so a day with a clock change still counts as one.
    let ago = ((midnight(&now) - midnight(&t)) as f64 / 86400.0).round() as i64;
    match ago {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=6 => t.format("%A").map(|s| s.to_string()).unwrap_or_default(),
        _ => t.format("%e %B").map(|s| s.trim().to_string()).unwrap_or_default(),
    }
}

fn clock(ms: u64) -> String {
    glib::DateTime::from_unix_local((ms / 1000) as i64).ok().and_then(|t| t.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()
}

/// A tool's step, in words.
fn tool_words(tool: &str) -> String {
    let known = match tool {
        "canvas.show" => "Showed it on screen",
        "location.search" => "Looked up a place",
        "location.current" => "Found where you are",
        "weather.lookup" => "Looked up the weather",
        "web.search" | "duckduckgo.search" => "Searched the web",
        "web.read" => "Read a web page",
        "files.read" => "Read a file",
        "files.list" => "Looked in a folder",
        "files.write" => "Wrote a file",
        "input.type" => "Typed text",
        "input.key" => "Pressed a key",
        "ui.act" => "Used an app",
        "ui.find" => "Looked for something in an app",
        "screen.look" => "Looked at the screen",
        "windows.focus" => "Switched windows",
        "windows.list" => "Looked at the open windows",
        "notes.create" => "Made a note",
        "notes.list" => "Looked at your notes",
        "music.play" => "Played music",
        "music.search" => "Searched your music",
        "audio.volume_up" => "Turned the volume up",
        "audio.volume_down" => "Turned the volume down",
        "projects.current" => "Checked the current project",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    let text = tool.replace(['.', '_'], " ");
    let mut c = text.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Arguments as "key: value" pairs, short.
fn brief(args: &Value) -> String {
    let text = match args {
        Value::Object(m) => m
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| match v {
                Value::String(s) => format!("{k}: {s}"),
                Value::Object(_) => brief(v),
                Value::Array(a) => format!("{k}: {}", a.iter().map(brief).collect::<Vec<_>>().join(", ")),
                v => format!("{k}: {v}"),
            })
            .collect::<Vec<_>>()
            .join(" · "),
        Value::String(s) if !s.trim_start().starts_with(['{', '[']) => s.clone(),
        Value::String(s) => serde_json::from_str::<Value>(s).map(|v| brief(&v)).unwrap_or_else(|_| s.clone()),
        Value::Null => String::new(),
        v => v.to_string(),
    };
    let short: String = text.chars().take(160).collect();
    if short.len() < text.len() { format!("{short}…") } else { short }
}

/// A result cut short (no longer JSON) as plain "key: value" text.
fn loose(text: &str) -> String {
    let plain: String = text.chars().filter(|c| !matches!(c, '{' | '}' | '[' | ']' | '"' | '\\')).collect();
    let plain = plain.trim_start_matches("data:").replace(':', ": ").replace(',', ", ");
    let short: String = plain.chars().take(200).collect();
    short.split(", ").filter(|p| !p.ends_with(": null")).collect::<Vec<_>>().join(", ")
}

/// A step's result: its evidence when the tool gave some, else its text.
fn step_text(summary: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(summary) else { return loose(summary) };
    let data = &v["data"];
    if let Some(ev) = data["evidence"].as_array() {
        return ev.iter().filter_map(|e| e.as_str()).collect::<Vec<_>>().join("; ");
    }
    brief(data)
}

fn decide(id: String, allow: bool, task: bool) {
    cmd::background(
        move || {
            let mut args = vec![if allow { "approve" } else { "deny" }, id.as_str()];
            if task {
                args.push("--task");
            }
            client::run(&args)
        },
        |r| {
            if let Err(e) = r {
                window::toast(&format!("Couldn't answer: {e}"));
            }
            window::rebuild("assistant-activity");
        },
    );
}

fn approval_row(id: &str, a: &Value) -> gtk::Box {
    let tool = a["tool"].as_str().unwrap_or("a tool");
    let title = a["summary"].as_str().filter(|s| !s.is_empty()).unwrap_or(tool).to_string();
    let buttons = widgets::hbox(6);
    let add = |label: &str, allow: bool, task: bool| {
        let b = gtk::Button::with_label(label);
        if allow && !task {
            b.add_css_class("suggested-action");
        }
        let id = id.to_string();
        b.connect_clicked(move |b| {
            b.set_sensitive(false);
            decide(id.clone(), allow, task);
        });
        buttons.append(&b);
    };
    add("Deny", false, false);
    if a["task_scope_allowed"] == true {
        add("Allow for this task", true, true);
    }
    add("Allow", true, false);
    widgets::stacked_row(&title, &glib::markup_escape_text(&format!("{} · {}", tool_words(tool), brief(&a["arguments"]))), buttons.upcast_ref())
}

fn audit_row(r: &Value) -> gtk::Box {
    let tool = r["tool"].as_str().unwrap_or("");
    let how = match (r["decision"].as_str().unwrap_or(""), r["approval_id"].is_string()) {
        ("deny", _) => "Not allowed",
        (_, true) => "You allowed it",
        _ => "Allowed by your settings",
    };
    let ok = match r["outcome"].as_str().unwrap_or("") {
        "ok" => String::new(),
        o => format!(" · {o}"),
    };
    let mut desc = format!("{} · {how}{ok}", when(r["at_ms"].as_u64().unwrap_or(0)));
    if let Some(d) = r["duration_ms"].as_u64() {
        desc.push_str(&format!(" · {}", duration(d)));
    }
    let args = brief(&r["args"]);
    if !args.is_empty() {
        desc.push_str(&format!("\n{args}"));
    }
    if let Some(d) = r["detail"].as_str() {
        desc.push_str(&format!("\n{d}"));
    }
    let row = widgets::row(&tool_words(tool), &glib::markup_escape_text(&desc), None);
    row.set_tooltip_text(Some(&format!("{tool} · {} ({})", r["decision"].as_str().unwrap_or(""), r["rule"].as_str().unwrap_or(""))));
    row
}

fn actions_page(page: &Page, _: &super::Again) {
    let g = page.group("Recent actions");
    let rows = client::json(&["audit", "--limit", "30", "--json"]).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    if rows.is_empty() {
        g.note("Nothing yet.");
        return;
    }
    let filter = gtk::SearchEntry::new();
    filter.set_placeholder_text(Some("Filter by tool or outcome"));
    g.top(&filter);
    let mut shown = Vec::new();
    // Newest first.
    for r in rows.iter().rev() {
        let row = audit_row(r);
        let text = format!("{} {} {} {}", r["tool"], tool_words(r["tool"].as_str().unwrap_or("")), r["outcome"], r["decision"]).to_lowercase();
        g.add(&row);
        shown.push((row, text));
    }
    filter.connect_search_changed(move |e| {
        let q = e.text().to_lowercase();
        for (row, text) in &shown {
            row.set_visible(q.is_empty() || text.contains(q.as_str()));
        }
    });
}

// ---------- The chosen request ----------

/// A step's marker: its number or mark in a circle, and the line on to the next one.
fn marker(text: &str, class: &str, last: bool) -> gtk::Box {
    let col = widgets::vbox(0);
    col.add_css_class("asst-step-marker");
    // The line fills the marker's height without stretching the steps around it.
    col.set_vexpand(false);
    let dot = widgets::label(text, "asst-step-dot");
    dot.add_css_class(class);
    dot.set_halign(gtk::Align::Center);
    col.append(&dot);
    if !last {
        let line = widgets::vbox(0);
        line.add_css_class("asst-step-line");
        line.set_halign(gtk::Align::Center);
        line.set_vexpand(true);
        col.append(&line);
    }
    col
}

fn fill_steps(list: &gtk::Box, shown: Vec<Value>) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    if shown.is_empty() {
        list.append(&dim("No steps: it answered without doing anything."));
        return;
    }
    let n = shown.len();
    for (i, s) in shown.into_iter().enumerate() {
        let tool = s["tool"].as_str().unwrap_or("");
        let (mark, class) = match s["verification"].as_str() {
            Some("verified") => ("✓", "asst-done"),
            Some("failed") => ("✗", "asst-failed"),
            _ => ("", "asst-neutral"),
        };
        let number = (s["index"].as_u64().unwrap_or(i as u64) + 1).to_string();
        let line = widgets::hbox(12);
        line.append(&marker(if mark.is_empty() { &number } else { mark }, class, i + 1 == n));
        let card = widgets::hbox(10);
        card.add_css_class("asst-step");
        card.add_css_class(class);
        card.set_hexpand(true);
        let text = widgets::vbox(2);
        text.set_hexpand(true);
        let title = if mark.is_empty() { tool_words(tool).to_string() } else { format!("{number}. {}", tool_words(tool)) };
        let title = widgets::label(&title, "asst-step-title");
        title.set_wrap(true);
        text.append(&title);
        let what = step_text(s["summary"].as_str().unwrap_or(""));
        if !what.is_empty() {
            let d = widgets::label(&what, "asst-step-desc");
            d.set_wrap(true);
            d.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            d.set_lines(3);
            d.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.append(&d);
        }
        card.append(&text);
        card.set_tooltip_text(Some(tool));
        line.append(&card);
        line.set_margin_bottom(if i + 1 == n { 0 } else { 8 });
        list.append(&line);
    }
}

/// The chosen request on the right: what you said, its steps and what's left to do.
fn show_task(detail: &gtk::Box, t: &Value) {
    while let Some(c) = detail.first_child() {
        detail.remove(&c);
    }
    let id = t["id"].as_str().unwrap_or("").to_string();
    let state = t["state"].as_str().unwrap_or("");
    let request = t["request"].as_str().unwrap_or("");
    let created = t["created_ms"].as_u64().unwrap_or(0);

    let head = widgets::hbox(10);
    head.add_css_class("preview-head");
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    let title = widgets::label(request, "preview-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(10);
    title.set_tooltip_text(Some(request));
    text.append(&title);
    let line = format!(
        "{} at {} · {} of {} steps · {}",
        day_of(created),
        clock(created),
        t["calls"].as_u64().unwrap_or(0),
        t["max_calls"].as_u64().unwrap_or(0),
        duration(t["active_ms"].as_u64().unwrap_or(0))
    );
    let l = widgets::label(&line, "preview-line");
    l.set_wrap(true);
    l.set_max_width_chars(10);
    text.append(&l);
    head.append(&text);
    let (_, class) = state_mark(state);
    let pill = chip(state_label(state), class);
    pill.add_css_class("asst-state-pill");
    head.append(&pill);
    detail.append(&head);

    let said = widgets::hbox(10);
    said.add_css_class("preview-card");
    said.add_css_class("asst-said");
    let mic = gtk::Image::from_icon_name("audio-input-microphone-symbolic");
    mic.set_valign(gtk::Align::Start);
    said.append(&mic);
    let words = widgets::label(&format!("“{request}”"), "asst-said-text");
    words.set_wrap(true);
    words.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    words.set_max_width_chars(10);
    words.set_hexpand(true);
    said.append(&words);
    detail.append(&said);

    if let Some(d) = t["detail"].as_str() {
        let l = dim(d);
        l.set_max_width_chars(10);
        detail.append(&l);
    }

    let list = widgets::vbox(0);
    list.add_css_class("asst-steps");
    list.append(&dim("Reading its steps…"));
    detail.append(&list);
    {
        let (list, id) = (list.clone(), id.clone());
        cmd::background(
            move || client::json(&["tasks", "show", &id, "--json"]).and_then(|v| v["steps"].as_array().cloned()).unwrap_or_default(),
            move |shown| fill_steps(&list, shown),
        );
    }

    let actions = widgets::hbox(8);
    if matches!(state, "failed" | "partially_completed" | "paused") {
        let resume = widgets::labeled_button("media-playback-start-symbolic", "Carry on from where it stopped");
        let task_id = id.clone();
        resume.connect_clicked(move |button| {
            button.set_sensitive(false);
            let id = task_id.clone();
            cmd::background(
                move || client::run(&["tasks", "resume", &id]),
                |result| {
                    if let Err(error) = result {
                        window::toast(&format!("Couldn't resume: {error}"));
                    }
                    window::rebuild("assistant-activity");
                },
            );
        });
        actions.append(&resume);
    }
    if active(state) {
        let stop = widgets::labeled_button("process-stop-symbolic", "Stop this task");
        let id = id.clone();
        stop.connect_clicked(move |b| {
            b.set_sensitive(false);
            let id = id.clone();
            cmd::background(
                move || client::run(&["tasks", "cancel", &id]),
                |r| {
                    if let Err(e) = r {
                        window::toast(&format!("Couldn't stop the task: {e}"));
                    }
                },
            );
        });
        actions.append(&stop);
    }
    if actions.first_child().is_some() {
        actions.add_css_class("asst-task-actions");
        detail.append(&actions);
    }
}

/// A request in the list: how it went, what you asked, when.
fn task_item(t: &Value) -> gtk::Button {
    let state = t["state"].as_str().unwrap_or("");
    let b = gtk::Button::new();
    b.add_css_class("asst-task");
    b.add_css_class("flat");
    let line = widgets::hbox(10);
    let (icon, class) = state_mark(state);
    let mark = widgets::hbox(0);
    mark.add_css_class("asst-mark");
    mark.add_css_class(class);
    mark.set_valign(gtk::Align::Center);
    let i = gtk::Image::from_icon_name(icon);
    i.set_halign(gtk::Align::Center);
    i.set_valign(gtk::Align::Center);
    mark.append(&i);
    line.append(&mark);
    let text = widgets::vbox(1);
    text.set_hexpand(true);
    let request = t["request"].as_str().unwrap_or("");
    let title = widgets::label(request, "asst-task-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(10);
    text.append(&title);
    let desc = widgets::label(&outcome(t), "asst-task-desc");
    desc.set_ellipsize(gtk::pango::EllipsizeMode::End);
    desc.set_max_width_chars(10);
    text.append(&desc);
    line.append(&text);
    let time = widgets::label(&clock(t["created_ms"].as_u64().unwrap_or(0)), "asst-task-time");
    time.set_valign(gtk::Align::Start);
    line.append(&time);
    b.set_child(Some(&line));
    b.set_tooltip_text(Some(request));
    b
}

/// Which requests the filter keeps.
fn kept(filter: &str, state: &str) -> bool {
    match filter {
        "done" => state == "completed",
        "unfinished" => matches!(state, "failed" | "partially_completed" | "cancelled" | "cancelling"),
        "now" => active(state),
        _ => true,
    }
}

pub fn build(page: &Page) {
    let running = client::running();
    let tasks = client::json(&["tasks", "--limit", "10", "--json"]).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let pending = client::status().and_then(|s| s["pending_approvals"].as_object().cloned()).unwrap_or_default();

    // The picture: the chosen request and what it did.
    let pv = page.split();
    let detail = widgets::vbox(14);
    detail.add_css_class("asst-detail");
    pv.add(&detail);
    pv.foot("security-high-symbolic", "What it may do without asking is set on Permissions.");
    let picked = PICKED.with(|p| p.borrow().clone());
    let first = tasks
        .iter()
        .find(|t| t["id"].as_str() == Some(picked.as_str()))
        .or_else(|| tasks.iter().find(|t| active(t["state"].as_str().unwrap_or(""))))
        .or(tasks.first());
    match first {
        Some(t) => show_task(&detail, t),
        None => {
            let empty = widgets::vbox(6);
            empty.add_css_class("asst-empty");
            empty.set_valign(gtk::Align::Center);
            empty.set_vexpand(true);
            let title = widgets::label("Nothing to show yet", "preview-title");
            title.set_halign(gtk::Align::Center);
            empty.append(&title);
            let l = widgets::label("When you ask for something that takes several steps, what it did shows here.", "preview-line");
            l.set_wrap(true);
            l.set_justify(gtk::Justification::Center);
            l.set_halign(gtk::Align::Center);
            empty.append(&l);
            detail.append(&empty);
        }
    }

    // The controls: what it's waiting on, then requests by day.
    if !running {
        page.banner("The assistant isn't running. Start it on the Assistant page.", false);
    }
    if !pending.is_empty() {
        let g = page.group("Waiting for you");
        for (id, a) in &pending {
            g.add(&approval_row(id, a));
        }
        window::set_nav_badge("assistant-activity", Some(&pending.len().to_string()));
    } else {
        window::set_nav_badge("assistant-activity", None);
    }

    let g = page.plain_group("");
    let tools = widgets::hbox(8);
    tools.add_css_class("asst-tools");
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search what you asked"));
    search.set_hexpand(true);
    tools.append(&search);
    let filters = widgets::opts(&[("all", "Everything"), ("done", "Done"), ("unfinished", "Didn't finish"), ("now", "In progress")]);
    let filter = widgets::dropdown(&filters, "all");
    tools.append(&filter);
    g.add(&tools);

    if tasks.is_empty() {
        let g = page.group("Requests");
        g.note("No requests yet. Requests that take several steps, like “type a note in Write”, show here.");
    }
    let items: Rc<RefCell<Vec<(gtk::Button, String, String)>>> = Rc::default();
    let days: Rc<RefCell<Vec<(gtk::Box, Vec<gtk::Button>)>>> = Rc::default();
    let mut current_day = String::new();
    let mut group: Option<widgets::Group> = None;
    for t in &tasks {
        let day = day_of(t["created_ms"].as_u64().unwrap_or(0));
        if group.is_none() || day != current_day {
            let g = page.group(&day);
            g.list.add_css_class("asst-task-list");
            days.borrow_mut().push((g.wrapper.clone(), Vec::new()));
            group = Some(g);
            current_day = day;
        }
        let b = task_item(t);
        let id = t["id"].as_str().unwrap_or("").to_string();
        if first.and_then(|f| f["id"].as_str()) == Some(id.as_str()) {
            b.add_css_class("asst-picked");
        }
        {
            let (detail, t, items, id) = (detail.clone(), t.clone(), items.clone(), id.clone());
            b.connect_clicked(move |b| {
                for (other, ..) in items.borrow().iter() {
                    other.remove_css_class("asst-picked");
                }
                b.add_css_class("asst-picked");
                PICKED.with(|p| *p.borrow_mut() = id.clone());
                show_task(&detail, &t);
            });
        }
        if let Some(g) = &group {
            g.add(&b);
        }
        if let Some((_, list)) = days.borrow_mut().last_mut() {
            list.push(b.clone());
        }
        items.borrow_mut().push((b, t["request"].as_str().unwrap_or("").to_lowercase(), t["state"].as_str().unwrap_or("").to_string()));
    }
    let apply = {
        let (items, days, search, filter) = (items.clone(), days.clone(), search.clone(), filter.clone());
        let ids: Vec<String> = filters.iter().map(|(id, _)| id.clone()).collect();
        move || {
            let q = search.text().to_lowercase();
            let f = ids.get(filter.selected() as usize).cloned().unwrap_or_default();
            for (b, text, state) in items.borrow().iter() {
                b.set_visible(text.contains(&q) && kept(&f, state));
            }
            for (wrapper, list) in days.borrow().iter() {
                wrapper.set_visible(list.iter().any(|b| b.is_visible()));
            }
        }
    };
    {
        let apply = apply.clone();
        search.connect_search_changed(move |_| apply());
    }
    filter.connect_selected_notify(move |_| apply());

    let g = page.group("");
    let actions = client::json(&["audit", "--limit", "30", "--json"]).and_then(|v| v.as_array().map(|a| a.len())).unwrap_or(0);
    let (r, _) = widgets::nav_row(
        Some("document-open-recent-symbolic"),
        "Recent actions",
        "Each tool it used, and who allowed it.",
        &if actions == 0 { "None yet".to_string() } else { format!("Last {actions}") },
        || more("Recent actions", actions_page),
    );
    g.add(&r);
    widgets::keywords("agent task steps cancel stop approval approve deny audit log history actions");
}
