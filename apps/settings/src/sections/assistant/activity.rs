//! What the assistant is asking, doing and has done. Redrawn while open when any of it changes.

use super::{client, dim, when};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use serde_json::Value;

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
        "cancelled" => "Cancelled",
        "failed" => "Failed",
        "partially_completed" => "Partly done",
        "completed" => "Done",
        s => s,
    }
}

fn active(state: &str) -> bool {
    matches!(state, "queued" | "planning" | "running" | "waiting_for_approval" | "paused")
}

/// Arguments as "key: value" pairs, short.
fn brief(args: &Value) -> String {
    let text = match args {
        Value::Object(m) => m
            .iter()
            .map(|(k, v)| format!("{k}: {}", v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())))
            .collect::<Vec<_>>()
            .join(" · "),
        Value::String(s) => serde_json::from_str::<Value>(s).map(|v| brief(&v)).unwrap_or_else(|_| s.clone()),
        Value::Null => String::new(),
        v => v.to_string(),
    };
    let short: String = text.chars().take(160).collect();
    if short.len() < text.len() { format!("{short}…") } else { short }
}

/// A step's result: its evidence when the tool gave some, else its text.
fn step_text(summary: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(summary) else { return summary.chars().take(200).collect() };
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
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
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
    add("Allow", true, false);
    if a["task_scope_allowed"] == true {
        add("Allow for this task", true, true);
    }
    add("Deny", false, false);
    widgets::stacked_row(
        &title,
        &gtk::glib::markup_escape_text(&format!("{tool} · {}", brief(&a["arguments"]))),
        buttons.upcast_ref(),
    )
}

fn task_row(t: &Value) -> gtk::Box {
    let id = t["id"].as_str().unwrap_or("").to_string();
    let state = t["state"].as_str().unwrap_or("");
    let desc = format!(
        "{} · {} of {} steps · {:.1} s · {}",
        state_label(state),
        t["calls"].as_u64().unwrap_or(0),
        t["max_calls"].as_u64().unwrap_or(0),
        t["active_ms"].as_u64().unwrap_or(0) as f64 / 1000.0,
        when(t["created_ms"].as_u64().unwrap_or(0))
    );
    let (wrapper, content) = widgets::disclosure(t["request"].as_str().unwrap_or(""), &desc);
    if let Some(d) = t["detail"].as_str() {
        content.append(&dim(d));
    }
    if matches!(state, "failed" | "partially_completed" | "paused") {
        let resume = gtk::Button::with_label("Resume from checkpoint");
        let task_id = id.clone();
        resume.set_halign(gtk::Align::Start);
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
        content.append(&resume);
    }
    if active(state) {
        let id = id.clone();
        let stop = gtk::Button::with_label("Stop this task");
        stop.set_halign(gtk::Align::Start);
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
        content.append(&stop);
    }
    // Steps are read when the row is first opened.
    let steps = gtk::Box::new(gtk::Orientation::Vertical, 4);
    content.append(&steps);
    if let Some(button) = wrapper.first_child().and_downcast::<gtk::Button>() {
        button.connect_clicked(move |_| {
            if steps.first_child().is_some() {
                return;
            }
            let shown =
                client::json(&["tasks", "show", &id, "--json"]).and_then(|v| v["steps"].as_array().cloned()).unwrap_or_default();
            if shown.is_empty() {
                steps.append(&dim("No steps."));
            }
            for s in shown {
                let mark = match s["verification"].as_str() {
                    Some("verified") => "✓",
                    Some("failed") => "✗",
                    _ => "·",
                };
                let line = format!(
                    "{mark} {}. {} — {}",
                    s["index"].as_u64().unwrap_or(0) + 1,
                    s["tool"].as_str().unwrap_or(""),
                    step_text(s["summary"].as_str().unwrap_or(""))
                );
                steps.append(&dim(&line));
            }
        });
    }
    wrapper
}

fn audit_row(r: &Value) -> gtk::Box {
    let title = format!("{} · {}", r["tool"].as_str().unwrap_or(""), r["outcome"].as_str().unwrap_or(""));
    let mut desc = format!(
        "{} · {} ({})",
        when(r["at_ms"].as_u64().unwrap_or(0)),
        r["decision"].as_str().unwrap_or(""),
        r["rule"].as_str().unwrap_or("")
    );
    if let Some(d) = r["duration_ms"].as_u64() {
        desc.push_str(&format!(" · {d} ms"));
    }
    let args = brief(&r["args"]);
    if !args.is_empty() {
        desc.push_str(&format!("\n{args}"));
    }
    if let Some(d) = r["detail"].as_str() {
        desc.push_str(&format!("\n{d}"));
    }
    widgets::row(&title, &gtk::glib::markup_escape_text(&desc), None)
}

pub fn build(page: &Page) {
    if !client::running() {
        page.banner("The assistant isn't running. Start it on the Assistant page.", false);
    }
    let pending = client::status().and_then(|s| s["pending_approvals"].as_object().cloned()).unwrap_or_default();
    if !pending.is_empty() {
        let g = page.group("Waiting for you");
        for (id, a) in &pending {
            g.add(&approval_row(id, a));
        }
        window::set_nav_badge("assistant-activity", Some(&pending.len().to_string()));
    } else {
        window::set_nav_badge("assistant-activity", None);
    }

    let g = page.group("Tasks");
    let tasks = client::json(&["tasks", "--limit", "10", "--json"]).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    if tasks.is_empty() {
        g.note("No tasks yet. Tasks are requests that take several steps, like “type a note in Write”.");
    }
    for t in &tasks {
        g.add(&task_row(t));
    }
    widgets::keywords("agent task steps cancel stop approval approve deny");

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
        let text = format!("{} {} {}", r["tool"], r["outcome"], r["decision"]).to_lowercase();
        g.add(&row);
        shown.push((row, text));
    }
    filter.connect_search_changed(move |e| {
        let q = e.text().to_lowercase();
        for (row, text) in &shown {
            row.set_visible(q.is_empty() || text.contains(q.as_str()));
        }
    });
    widgets::keywords("audit log history");
}
