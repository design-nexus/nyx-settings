//! Background services (systemd units), for you and for the whole system.
//! System changes go through the root helper; your own services need no password.

use crate::backend::accounts as acc;
use crate::sections::accounts::require_helper;
use crate::sketch::{self, Pal};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

/// Where the list's own redraw function is kept, weakly so it can refer to itself.
type Slot = Rc<std::cell::RefCell<Option<Weak<dyn Fn()>>>>;

#[derive(Debug, PartialEq, Clone)]
pub struct Unit {
    pub name: String,
    pub active: String,
    pub sub: String,
    pub description: String,
    /// `enabled`, `disabled`, `static`… when the unit has an install state.
    pub enabled: String,
}

/// Join `systemctl list-units` (UNIT LOAD ACTIVE SUB DESCRIPTION…) with
/// `list-unit-files` (UNIT STATE). Only real services are kept.
pub fn parse_units(units: &str, files: &str) -> Vec<Unit> {
    let states: HashMap<&str, &str> = files
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some((f.next()?, f.next()?))
        })
        .collect();
    let mut out: Vec<Unit> = units
        .lines()
        .filter_map(|l| {
            let l = l.trim_start_matches(['●', '○', '×', '*', ' ']);
            let mut f = l.split_whitespace();
            let name = f.next()?;
            let load = f.next()?;
            let active = f.next()?;
            let sub = f.next()?;
            if !name.ends_with(".service") || load == "not-found" || name.contains("@.") {
                return None;
            }
            let description = f.collect::<Vec<_>>().join(" ");
            let enabled = states.get(name).copied().unwrap_or("").to_string();
            Some(Unit { name: name.to_string(), active: active.to_string(), sub: sub.to_string(), description, enabled })
        })
        .collect();
    // Running first, then by name.
    out.sort_by(|a, b| (b.active == "active").cmp(&(a.active == "active")).then(a.name.cmp(&b.name)));
    out
}

const UNITS: &[&str] = &["list-units", "--type=service", "--all", "--plain", "--no-legend", "--no-pager"];
const UNIT_FILES: &[&str] = &["list-unit-files", "--type=service", "--plain", "--no-legend", "--no-pager"];

/// What the page asks systemctl while being built, for [`cmd::fetch_ahead`].
pub fn reads() -> Vec<Vec<&'static str>> {
    [&["systemctl", "--user"][..], &["systemctl"][..]]
        .iter()
        .flat_map(|base| [UNITS, UNIT_FILES].map(|extra| [*base, extra].concat()))
        .collect()
}

pub(crate) fn list(user: bool) -> Vec<Unit> {
    let base: &[&str] = if user { &["systemctl", "--user"] } else { &["systemctl"] };
    let run = |extra: &[&str]| cmd::read(&[base, extra].concat()).unwrap_or_default();
    parse_units(&run(UNITS), &run(UNIT_FILES))
}

// ---------- The page: services by what they do, and the one picked ----------

thread_local! {
    /// The service shown on the right (yours or the system's, and its unit), kept across
    /// the page's redraws.
    static SELECTED: RefCell<Option<(bool, String)>> = const { RefCell::new(None) };
    /// Which list shows: yours (true) or the system's.
    static SHOW_MINE: Cell<bool> = const { Cell::new(true) };
}

/// What a service does, in its own words; its unit name when it has none.
fn friendly(u: &Unit) -> String {
    let name = u.name.trim_end_matches(".service");
    if u.description.is_empty() || u.description == u.name || u.description == name {
        name.to_string()
    } else {
        u.description.clone()
    }
}

fn state_class(u: &Unit) -> &'static str {
    match u.active.as_str() {
        "failed" => "fail",
        "active" | "reloading" | "activating" => "run",
        _ => "off",
    }
}

fn starts_text(user: bool) -> &'static str {
    if user { "At sign-in" } else { "At start-up" }
}

/// The value at a row's end: an error, when it starts, or whether it runs.
fn row_value(u: &Unit, user: bool) -> &'static str {
    match (state_class(u), u.enabled.as_str()) {
        ("fail", _) => "Error",
        (_, "enabled") => starts_text(user),
        ("run", _) => "Running",
        _ => "Off",
    }
}

fn dot(class: &str) -> gtk::Box {
    let d = widgets::hbox(0);
    d.add_css_class("sv-dot");
    d.add_css_class(class);
    d.set_valign(gtk::Align::Center);
    d
}

/// A unit's properties from `systemctl show`, and what its log says.
#[derive(Default)]
struct Detail {
    props: HashMap<String, String>,
    /// (unix seconds, message, priority) newest last.
    log: Vec<(i64, String, u8)>,
    history: History,
}

/// When it ran over the last week: (start, end, running) spans in unix seconds while
/// the computer was on, and the moments it failed. Before `known_from` the journal
/// doesn't say.
#[derive(Default, Clone)]
struct History {
    spans: Vec<(i64, i64, bool)>,
    failures: Vec<i64>,
    known_from: i64,
}

const STARTED: &str = "39f53479d3a045ac8e11786248231fbf";
const STOPPED: &str = "9d1aaa27d60140bd96365438aad20286";
const DEACTIVATED: &str = "7ad2d189f7e94e70a38c781354912448";
const FAILED: &str = "d9b373ed55a64feb8242e02dbe79a49c";
const START_FAILED: &str = "be02cf6855d2428ba40df7e9d022f03d";

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `journalctl -o json` lines: (unix seconds, MESSAGE_ID, MESSAGE, PRIORITY).
fn journal_lines(text: &str) -> Vec<(i64, String, String, u8)> {
    text.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .map(|j| {
            let us = j["__REALTIME_TIMESTAMP"].as_str().and_then(|t| t.parse::<i64>().ok()).unwrap_or(0);
            let msg = match &j["MESSAGE"] {
                serde_json::Value::String(s) => s.clone(),
                // Messages that aren't valid text come as a list of bytes.
                serde_json::Value::Array(b) => String::from_utf8_lossy(&b.iter().filter_map(|v| v.as_u64().map(|n| n as u8)).collect::<Vec<_>>()).into(),
                _ => String::new(),
            };
            let prio = j["PRIORITY"].as_str().and_then(|p| p.parse().ok()).unwrap_or(6);
            (us / 1_000_000, j["MESSAGE_ID"].as_str().unwrap_or("").to_string(), msg, prio)
        })
        .collect()
}

/// Spans from start and stop events, within each boot (`boots`: first and last entry).
/// `active_since`: when the current run started, if it's running now.
fn build_history(events: &[(i64, String)], boots: &[(i64, i64)], active_since: Option<i64>, since: i64, now: i64) -> History {
    let mut h = History { known_from: boots.first().map(|b| b.0).unwrap_or(now).max(since), ..Default::default() };
    let n = boots.len();
    for (i, (first, last)) in boots.iter().enumerate() {
        let end = if i + 1 == n { now } else { *last };
        if end < since {
            continue;
        }
        let mut in_boot: Vec<&(i64, String)> = events.iter().filter(|(t, _)| *t >= *first && *t <= end).collect();
        in_boot.sort_by_key(|e| e.0);
        // Stopped before it was ever seen starting: it was running from the start.
        let mut running = in_boot.first().is_some_and(|(_, id)| id == STOPPED || id == DEACTIVATED || id == FAILED);
        let mut from = *first;
        for (t, id) in in_boot {
            let starts = id == STARTED;
            if id == FAILED || id == START_FAILED {
                h.failures.push(*t);
            }
            if starts != running {
                h.spans.push((from, *t, running));
                from = *t;
                running = starts;
            }
        }
        if i + 1 == n
            && let Some(a) = active_since
            && !running
            && a >= from
        {
            h.spans.push((from, a, false));
            from = a;
            running = true;
        }
        h.spans.push((from, end, running));
    }
    h.spans.retain(|(a, b, _)| b > a && *b > since);
    h
}

fn read_detail(user: bool, name: &str) -> Detail {
    let base: Vec<&str> = if user { vec!["--user"] } else { vec![] };
    let mut show = vec!["systemctl"];
    show.extend(&base);
    show.extend([
        "show",
        name,
        "--timestamp=unix",
        "-p",
        "ActiveEnterTimestamp,InactiveEnterTimestamp,Result,ExecMainStatus,MemoryCurrent,NRestarts,MainPID",
    ]);
    let props = cmd::output(&show)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<HashMap<_, _>>();
    let mut j = vec!["journalctl"];
    j.extend(&base);
    j.extend(["-u", name, "-n", "14", "-p", "info", "-o", "json", "--output-fields=MESSAGE,PRIORITY", "--no-pager"]);
    let mut log: Vec<(i64, String, u8)> =
        journal_lines(&cmd::output(&j).unwrap_or_default()).into_iter().map(|(t, _, m, p)| (t, m, p)).collect();
    if log.is_empty() {
        // Some services only leave debugging lines.
        j.retain(|a| *a != "-p" && *a != "info");
        log = journal_lines(&cmd::output(&j).unwrap_or_default()).into_iter().map(|(t, _, m, p)| (t, m, p)).collect();
    }

    // A week of starts and stops, and the boots in it.
    let since = now() - 7 * 86_400;
    let since_arg = format!("--since=@{since}");
    let unit_match = format!("USER_UNIT={name}");
    let mut e = vec!["journalctl"];
    e.extend(&base);
    if user {
        e.push(&unit_match);
    } else {
        e.extend(["-u", name]);
    }
    let ids: Vec<String> = [STARTED, STOPPED, DEACTIVATED, FAILED, START_FAILED].iter().map(|i| format!("MESSAGE_ID={i}")).collect();
    e.extend(ids.iter().map(String::as_str));
    e.extend([since_arg.as_str(), "-o", "json", "--output-fields=MESSAGE_ID", "--no-pager"]);
    let events: Vec<(i64, String)> = journal_lines(&cmd::output(&e).unwrap_or_default()).into_iter().map(|(t, id, _, _)| (t, id)).collect();
    let boots: Vec<(i64, i64)> = cmd::output(&["journalctl", "--list-boots", "-o", "json"])
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|b| (b["first_entry"].as_i64().unwrap_or(0) / 1_000_000, b["last_entry"].as_i64().unwrap_or(0) / 1_000_000))
        .filter(|b| b.1 > 0)
        .collect();
    let active_since = props.get("ActiveEnterTimestamp").and_then(|t| t.trim_start_matches('@').parse().ok());
    let running_now = props.get("MainPID").is_some_and(|p| p != "0") || active_since.is_some_and(|a| {
        props.get("InactiveEnterTimestamp").and_then(|t| t.trim_start_matches('@').parse::<i64>().ok()).is_none_or(|i| i < a)
    });
    let history = build_history(&events, &boots, active_since.filter(|_| running_now), since, now());
    Detail { props, log, history }
}

fn clock(secs: i64) -> String {
    glib::DateTime::from_unix_local(secs).ok().and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()
}

/// When, in words: "at 13:20" today, "on Tuesday at 13:20" this week, else the date.
fn when(secs: i64) -> String {
    let (Ok(d), Ok(today)) = (glib::DateTime::from_unix_local(secs), glib::DateTime::now_local()) else { return String::new() };
    let days = today.ymd();
    if d.ymd() == days {
        format!("at {}", clock(secs))
    } else if now() - secs < 6 * 86_400 {
        format!("on {} at {}", d.format("%A").map(|s| s.to_string()).unwrap_or_default(), clock(secs))
    } else {
        format!("on {}", d.format("%e %B").map(|s| s.trim().to_string()).unwrap_or_default())
    }
}

/// Why it stopped, in plain words, from systemd's `Result`.
fn result_words(result: &str, status: &str) -> String {
    match result {
        "exit-code" if !status.is_empty() && status != "0" => format!("The program ended with an error (exit status {status})."),
        "exit-code" => "The program ended with an error.".into(),
        "signal" => "It was ended by a signal.".into(),
        "core-dump" => "It crashed.".into(),
        "timeout" => "It took too long to start or stop.".into(),
        "start-limit-hit" => "It failed too often in a row, so it isn't started again by itself.".into(),
        "resources" => "It couldn't get what it needs to start.".into(),
        "oom-kill" => "It ran out of memory.".into(),
        "watchdog" => "It stopped answering.".into(),
        "protocol" => "It didn't start the way it's meant to.".into(),
        _ => String::new(),
    }
}

fn run_action(user: bool, action: &'static str, unit: String, b: &gtk::Button) {
    b.set_sensitive(false);
    let b = b.clone();
    cmd::background(
        move || {
            if user {
                cmd::run(&["systemctl", "--user", action, &unit]).map(|_| ()).map_err(|e| e.to_string())
            } else {
                acc::helper(&["service", action, &unit], None).map(|_| ()).map_err(|e| e.to_string())
            }
        },
        move |r| {
            b.set_sensitive(true);
            if let Err(e) = r {
                window::toast(&e);
            }
            window::rebuild("services");
        },
    );
}

fn action_button(label: &str, icon: Option<&str>, user: bool, action: &'static str, unit: &str, can_edit: bool) -> gtk::Button {
    let b = gtk::Button::new();
    let inner = widgets::hbox(6);
    if let Some(i) = icon {
        inner.append(&gtk::Image::from_icon_name(i));
    }
    inner.append(&gtk::Label::new(Some(label)));
    b.set_child(Some(&inner));
    b.set_sensitive(can_edit);
    let unit = unit.to_string();
    b.connect_clicked(move |b| run_action(user, action, unit.clone(), b));
    b
}

/// A week of when it ran: a bar per day, green while running, a red mark where it
/// failed, and dim while it was off.
fn draw_week(cr: &gtk::cairo::Context, p: &Pal, w: f64, h: &History) {
    let Ok(today) = glib::DateTime::now_local() else { return };
    let (y, m, d) = today.ymd();
    let Ok(midnight) = glib::DateTime::from_local(y, m, d, 0, 0, 0.0) else { return };
    let label_w = 52.0;
    let track_w = w - label_w - 4.0;
    let row = 22.0;
    let green = (0.33, 0.74, 0.42);
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
    cr.set_font_size(11.0);
    for i in 0..7 {
        let back = 6 - i;
        let Ok(day) = midnight.add_days(-back) else { continue };
        let start = day.to_unix();
        let end = start + 86_400;
        let cy = 8.0 + i as f64 * row;
        let name = if back == 0 { "Today".to_string() } else { day.format("%a").map(|s| s.to_string()).unwrap_or_default() };
        sketch::set(cr, p.dim, 1.0);
        cr.move_to(0.0, cy + 4.0);
        let _ = cr.show_text(&name);
        let x0 = label_w;
        let px = |t: i64| x0 + ((t - start) as f64 / 86_400.0).clamp(0.0, 1.0) * track_w;
        sketch::set(cr, sketch::mix(p.bg, p.text, 0.08), 1.0);
        sketch::rounded(cr, x0, cy - 4.0, track_w, 8.0, 3.0);
        let _ = cr.fill();
        for (a, b, running) in &h.spans {
            if *b <= start || *a >= end {
                continue;
            }
            let (xa, xb) = (px(*a), px(*b));
            if *running {
                sketch::set(cr, green, 0.95);
            } else {
                sketch::set(cr, sketch::mix(p.bg, p.text, 0.2), 1.0);
            }
            sketch::rounded(cr, xa, cy - 4.0, (xb - xa).max(1.5), 8.0, 2.0);
            let _ = cr.fill();
        }
        for f in &h.failures {
            if *f >= start && *f < end {
                sketch::set(cr, p.danger, 1.0);
                cr.arc(px(*f), cy, 4.5, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
        }
        if back == 0 {
            let x = px(now());
            sketch::set(cr, p.text, 0.9);
            cr.rectangle(x - 1.0, cy - 7.0, 2.0, 14.0);
            let _ = cr.fill();
        }
    }
    let y = 8.0 + 7.0 * row;
    sketch::set(cr, p.dim, 0.9);
    cr.set_font_size(10.0);
    for (i, t) in ["0:00", "6:00", "12:00", "18:00", "24:00"].iter().enumerate() {
        let x = label_w + track_w * i as f64 / 4.0;
        let ext = cr.text_extents(t).map(|e| e.width()).unwrap_or(0.0);
        let x = (x - ext / 2.0).min(w - ext).max(label_w);
        cr.move_to(x, y);
        let _ = cr.show_text(t);
    }
}

fn card_head(title: &str, end: Option<&gtk::Widget>) -> gtk::Box {
    let h = widgets::hbox(8);
    let t = widgets::label(title, "sv-card-title");
    t.set_hexpand(true);
    h.append(&t);
    if let Some(e) = end {
        h.append(e);
    }
    h
}

/// The selected service: what it's doing, a week of when it ran, and its log.
fn show(holder: &gtk::Box, u: &Unit, user: bool, can_edit: bool, turn: Rc<Cell<u32>>) {
    widgets::forget_rows(holder);
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let my_turn = turn.get().wrapping_add(1);
    turn.set(my_turn);
    let unit = u.name.trim_end_matches(".service");
    let mut line = vec![unit.to_string(), if user { "runs as you".into() } else { "runs for the whole computer".into() }];
    if u.enabled == "enabled" {
        line.push(if user { "starts when you sign in".into() } else { "starts with the computer".into() });
    }
    let (pill, class) = match state_class(u) {
        "fail" => ("Stopped with an error", "fail"),
        "run" => ("Running", "run"),
        _ => ("Off", "off"),
    };
    let status = widgets::hbox(6);
    status.add_css_class("sv-pill");
    status.add_css_class(class);
    status.append(&dot(class));
    status.append(&gtk::Label::new(Some(pill)));
    holder.append(&super::extensions::preview_head(&friendly(u), &line.join(" · "), &[status.upcast()]));

    // What it's doing, and what you can do about it.
    let card = widgets::vbox(10);
    card.add_css_class("preview-card");
    card.add_css_class("sv-why");
    card.add_css_class(class);
    let top = widgets::hbox(12);
    let icon = match class {
        "fail" => "dialog-warning-symbolic",
        "run" => "media-playback-start-symbolic",
        _ => "media-playback-stop-symbolic",
    };
    let tile = widgets::tile(icon);
    tile.set_valign(gtk::Align::Start);
    top.append(&tile);
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    let title = widgets::label("", "sv-why-title");
    title.set_wrap(true);
    let detail = widgets::label("Reading what it's doing…", "sv-why-text");
    detail.set_wrap(true);
    text.append(&title);
    text.append(&detail);
    top.append(&text);
    let buttons = widgets::vbox(6);
    buttons.set_valign(gtk::Align::Start);
    if class == "run" {
        buttons.append(&action_button("Restart", Some("view-refresh-symbolic"), user, "restart", &u.name, can_edit));
        buttons.append(&action_button("Stop", None, user, "stop", &u.name, can_edit));
    } else {
        let start = action_button("Start", Some("media-playback-start-symbolic"), user, "start", &u.name, can_edit);
        if class == "fail" {
            start.add_css_class("suggested-action");
        }
        buttons.append(&start);
    }
    top.append(&buttons);
    card.append(&top);
    title.set_text(match class {
        "fail" => "It stopped because of an error",
        "run" => "It's running",
        _ => "It isn't running",
    });
    if matches!(u.enabled.as_str(), "enabled" | "disabled") {
        let sw = gtk::Switch::new();
        sw.set_active(u.enabled == "enabled");
        sw.set_sensitive(can_edit);
        let name = u.name.clone();
        sw.connect_active_notify(move |s| {
            let action = if s.is_active() { "enable" } else { "disable" };
            let (name, s2) = (name.clone(), s.clone());
            s.set_sensitive(false);
            cmd::background(
                move || {
                    if user {
                        cmd::run(&["systemctl", "--user", action, &name]).map(|_| ()).map_err(|e| e.to_string())
                    } else {
                        acc::helper(&["service", action, &name], None).map(|_| ()).map_err(|e| e.to_string())
                    }
                },
                move |r| {
                    s2.set_sensitive(true);
                    if let Err(e) = r {
                        window::toast(&e);
                    }
                    window::rebuild("services");
                },
            );
        });
        let r = widgets::row(
            if user { "Start when you sign in" } else { "Start with the computer" },
            "Off: it only runs when you start it, or when something asks for it.",
            Some(sw.upcast_ref()),
        );
        r.add_css_class("sv-boot");
        card.append(&r);
    }
    if !can_edit {
        card.append(&widgets::label("Changing system services needs Settings' system helper (see the top of this page).", "sv-why-text"));
    }
    holder.append(&card);

    // The week, and the log, once read.
    let week_card = widgets::vbox(8);
    week_card.add_css_class("preview-card");
    let legend = widgets::hbox(10);
    legend.add_css_class("sv-legend");
    for (c, t) in [("run", "Running"), ("fail", "Error"), ("off", "Off")] {
        let item = widgets::hbox(5);
        item.append(&dot(c));
        item.append(&gtk::Label::new(Some(t)));
        legend.append(&item);
    }
    week_card.append(&card_head("The last 7 days", Some(legend.upcast_ref())));
    let history: Rc<RefCell<History>> = Rc::default();
    let week = {
        let history = history.clone();
        widgets::drawing(8 + 7 * 22 + 8, move |cr, p, _, w, _| draw_week(cr, p, w, &history.borrow()))
    };
    week_card.append(&week);
    let known = widgets::label("", "sv-why-text");
    known.set_wrap(true);
    known.set_visible(false);
    week_card.append(&known);
    holder.append(&week_card);

    let log_card = widgets::vbox(8);
    log_card.add_css_class("preview-card");
    let full = gtk::Button::new();
    full.add_css_class("flat");
    full.add_css_class("sv-link");
    let inner = widgets::hbox(4);
    inner.append(&gtk::Label::new(Some("Full log")));
    inner.append(&gtk::Image::from_icon_name("go-next-symbolic"));
    full.set_child(Some(&inner));
    let journal = format!("journalctl {}-u {} -e", if user { "--user " } else { "" }, cmd::shell_quote(&u.name));
    full.connect_clicked(move |_| crate::platform::in_terminal(&journal));
    log_card.append(&card_head("Recent messages", Some(full.upcast_ref())));
    let lines = widgets::vbox(3);
    lines.add_css_class("sv-log");
    lines.append(&widgets::label("Reading the log…", "sv-why-text"));
    // The log takes what height is left and scrolls inside it, newest at the bottom.
    let log_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(110)
        .vexpand(true)
        .child(&lines)
        .build();
    log_scroll.add_css_class("sv-log-scroll");
    log_card.set_vexpand(true);
    log_card.append(&log_scroll);
    {
        let adj = log_scroll.vadjustment();
        adj.connect_changed(|a| a.set_value(a.upper() - a.page_size()));
    }
    holder.append(&log_card);
    // The preview's rows aren't settings of their own for search to find.
    widgets::forget_rows(holder);

    let name = u.name.clone();
    let class = class.to_string();
    cmd::background(
        move || read_detail(user, &name),
        move |d: Detail| {
            if turn.get() != my_turn {
                return;
            }
            let prop = |k: &str| d.props.get(k).cloned().unwrap_or_default();
            let ts = |k: &str| prop(k).trim_start_matches('@').parse::<i64>().ok();
            let mut text = Vec::new();
            match class.as_str() {
                "fail" => {
                    if let Some(t) = ts("InactiveEnterTimestamp") {
                        let mut s = format!("It stopped {}", when(t));
                        if let Some(a) = ts("ActiveEnterTimestamp").filter(|a| *a < t) {
                            s.push_str(&format!(" after running for {}", crate::backend::sysinfo::duration_text((t - a) as u64)));
                        }
                        text.push(format!("{s}."));
                    }
                    let why = result_words(&prop("Result"), &prop("ExecMainStatus"));
                    if !why.is_empty() {
                        text.push(why);
                    }
                    if let Some((_, m, _)) = d.log.iter().rev().find(|(_, _, p)| *p <= 3) {
                        text.push(format!("The log says: “{m}”"));
                    }
                }
                "run" => {
                    if let Some(a) = ts("ActiveEnterTimestamp") {
                        text.push(format!(
                            "Since {} ({}).",
                            when(a).trim_start_matches("at ").trim_start_matches("on "),
                            crate::backend::sysinfo::duration_text((now() - a).max(0) as u64)
                        ));
                    }
                    if let Ok(m) = prop("MemoryCurrent").parse::<u64>() {
                        text.push(format!("Using {} of memory.", crate::backend::sysinfo::bytes_text(m)));
                    }
                    if let Ok(n) = prop("NRestarts").parse::<u32>()
                        && n > 0
                    {
                        text.push(format!("Restarted {n} time{} after stopping by itself.", if n == 1 { "" } else { "s" }));
                    }
                }
                _ => {
                    if let Some(t) = ts("InactiveEnterTimestamp") {
                        text.push(format!("It last stopped {}.", when(t)));
                    }
                }
            }
            detail.set_text(&text.join(" "));
            detail.set_visible(!text.is_empty());

            let from = d.history.known_from;
            *history.borrow_mut() = d.history;
            week.queue_draw();
            if from > now() - 7 * 86_400 + 3600 {
                known.set_text(&format!("The log starts {}; before that there's no record.", when(from)));
                known.set_visible(true);
            }

            while let Some(c) = lines.first_child() {
                lines.remove(&c);
            }
            if d.log.is_empty() {
                lines.append(&widgets::label("Nothing in the log.", "sv-why-text"));
            }
            for (t, m, p) in d.log {
                let r = widgets::hbox(12);
                r.append(&widgets::label(
                    &glib::DateTime::from_unix_local(t).ok().and_then(|d| d.format("%H:%M:%S").ok()).map(|s| s.to_string()).unwrap_or_default(),
                    "sv-log-time",
                ));
                let l = widgets::label(&m, "sv-log-text");
                l.set_wrap(true);
                l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                l.set_hexpand(true);
                l.set_selectable(true);
                l.set_can_focus(false);
                match p {
                    0..=3 => l.add_css_class("bad"),
                    4 => l.add_css_class("warn"),
                    _ => {}
                }
                r.append(&l);
                lines.append(&r);
            }
        },
    );
}

/// One service in a list: a dot for its state, what it does, its unit, and when it starts.
fn list_row(u: &Unit, user: bool) -> gtk::Box {
    let value = widgets::label(row_value(u, user), "row-value");
    if state_class(u) == "fail" {
        value.add_css_class("sv-bad");
    }
    let r = widgets::row(&friendly(u), &gtk::glib::markup_escape_text(u.name.trim_end_matches(".service")), Some(value.upcast_ref()));
    r.add_css_class("sv-row");
    r.prepend(&dot(state_class(u)));
    cap_text(&r, 22);
    widgets::keywords(&format!("{} service daemon systemd unit start stop restart enable disable", u.name));
    r
}

/// How many show before the rest wait for a search.
const SHOWN: usize = 60;

pub fn build(page: &Page) {
    let can_edit = require_helper(page, "services", "starting and stopping system services");
    let mine = list(true);
    let system = list(false);
    let all: Rc<Vec<(bool, Unit)>> =
        Rc::new(mine.iter().map(|u| (true, u.clone())).chain(system.iter().map(|u| (false, u.clone()))).collect());

    let chosen: Option<(bool, Unit)> = SELECTED
        .with(|s| s.borrow().clone())
        .and_then(|(user, name)| all.iter().find(|(m, u)| *m == user && u.name == name).cloned())
        .or_else(|| all.iter().find(|(_, u)| u.active == "failed").cloned())
        .or_else(|| mine.iter().find(|u| u.active == "active").map(|u| (true, u.clone())));

    let pv = page.split();
    let holder = widgets::vbox(14);
    // So the log can take the height left over.
    pv.fill(&super::extensions::shrinkable(&holder));
    let turn: Rc<Cell<u32>> = Rc::default();
    let show_unit: Rc<dyn Fn(bool, &Unit)> = {
        let (holder, turn) = (holder.clone(), turn.clone());
        Rc::new(move |user: bool, u: &Unit| show(&holder, u, user, user || can_edit, turn.clone()))
    };
    if let Some((user, u)) = &chosen {
        show_unit(*user, u);
    }

    // Find, and which list.
    let g = page.plain_group("");
    let top = widgets::hbox(8);
    let find = gtk::SearchEntry::new();
    find.set_placeholder_text(Some("Find a service"));
    find.set_hexpand(true);
    top.append(&find);
    let showing_mine = SHOW_MINE.with(|m| m.get());
    let which = nyx_ui::widgets::segmented(
        &widgets::opts(&[("mine", "Yours"), ("system", "System")]),
        if showing_mine { "mine" } else { "system" },
        |_| {},
    );
    top.append(&which);
    g.add(&top);

    // Every row, to filter and to mark the picked one.
    let rows: Rc<RefCell<Vec<(bool, String, String, gtk::Box)>>> = Rc::default();
    let add_row = {
        let (rows, show_unit) = (rows.clone(), show_unit.clone());
        move |g: &widgets::Group, user: bool, u: &Unit, hidden: bool| {
            let r = list_row(u, user);
            r.set_visible(!hidden);
            let (rows2, show_unit, u2, r2) = (rows.clone(), show_unit.clone(), u.clone(), r.clone());
            pickable(&r, move || {
                SELECTED.with(|s| *s.borrow_mut() = Some((user, u2.name.clone())));
                mark_picked(rows2.borrow().iter().map(|x| &x.3), Some(&r2));
                show_unit(user, &u2);
            });
            g.add(&r);
            let text = crate::search::normalise(&format!("{} {}", u.name, u.description));
            rows.borrow_mut().push((user, u.name.clone(), text, r));
        }
    };

    let failed: Vec<&(bool, Unit)> = all.iter().filter(|(_, u)| u.active == "failed").collect();
    let attention = if failed.is_empty() {
        None
    } else {
        let g = page.group("Needs attention");
        g.wrapper.add_css_class("sv-attention");
        for (user, u) in &failed {
            add_row(&g, *user, u, false);
        }
        Some(g)
    };
    let attention_rows = rows.borrow().len();

    let mut lists = Vec::new();
    for (user, units) in [(true, &mine), (false, &system)] {
        let title = if user { "Your services" } else { "System services" };
        let g = page.group(title);
        let running = units.iter().filter(|u| u.active == "active").count();
        g.note(&format!(
            "{running} running · {}",
            if user { "they run as you, from when you sign in" } else { "changing them asks for your password" }
        ));
        let mut n = 0;
        for u in units.iter().filter(|u| u.active != "failed") {
            add_row(&g, user, u, n >= SHOWN);
            n += 1;
        }
        let more = widgets::label("", "sv-more");
        more.set_xalign(0.0);
        let hidden = n.saturating_sub(SHOWN);
        more.set_text(&format!("{hidden} more. Type to find one."));
        more.set_visible(hidden > 0);
        g.add(&more);
        g.wrapper.set_visible(user == showing_mine);
        lists.push((user, g, more));
    }
    let lists = Rc::new(lists);

    // The other list, a row away.
    let toggles: Vec<gtk::ToggleButton> = {
        let mut out = Vec::new();
        let mut c = which.first_child();
        while let Some(w) = c {
            c = w.next_sibling();
            if let Ok(t) = w.downcast::<gtk::ToggleButton>() {
                out.push(t);
            }
        }
        out
    };
    let other = |showing_mine: bool| {
        if showing_mine {
            ("System services", "Changing them asks for your password", system.len())
        } else {
            ("Your services", "They run as you, from when you sign in", mine.len())
        }
    };
    let g = page.group("");
    let (t, d, n) = other(showing_mine);
    let (switch_row, other_value) = widgets::nav_row(Some("network-server-symbolic"), t, d, &n.to_string(), {
        let toggles = toggles.clone();
        move || {
            let mine_now = SHOW_MINE.with(|m| m.get());
            if let Some(t) = toggles.get(if mine_now { 1 } else { 0 }) {
                t.set_active(true);
            }
        }
    });
    widgets::keywords("system services your services user list");
    let set_other: Rc<dyn Fn(bool)> = {
        let (row, value) = (switch_row.clone(), other_value.clone());
        let texts = [other(true), other(false)];
        Rc::new(move |showing_mine: bool| {
            let (t, d, n) = texts[if showing_mine { 0 } else { 1 }];
            let labels = labels_in(&row);
            if let Some(l) = labels.first() {
                l.set_text(t);
            }
            if let Some(l) = labels.get(1) {
                l.set_text(d);
            }
            value.set_text(&n.to_string());
        })
    };
    g.add(&switch_row);

    // Filtering and switching lists.
    let apply: Rc<dyn Fn()> = {
        let (rows, lists, find, set_other) = (rows.clone(), lists.clone(), find.clone(), set_other.clone());
        let attention = attention.clone();
        Rc::new(move || {
            let mine_shown = SHOW_MINE.with(|m| m.get());
            let terms = crate::search::terms(&find.text());
            let mut counts = [0usize; 2];
            let mut hidden = [0usize; 2];
            let mut any_attention = false;
            for (i, (user, _, text, r)) in rows.borrow().iter().enumerate() {
                let hit = terms.is_empty() || crate::search::matches(text, &terms);
                if i < attention_rows {
                    r.set_visible(hit);
                    any_attention |= hit;
                    continue;
                }
                let k = if *user { 0 } else { 1 };
                let show = hit && counts[k] < SHOWN;
                if hit {
                    if counts[k] >= SHOWN {
                        hidden[k] += 1;
                    }
                    counts[k] += 1;
                }
                r.set_visible(show);
            }
            if let Some(g) = &attention {
                g.wrapper.set_visible(any_attention);
            }
            for (user, g, more) in lists.iter() {
                let k = if *user { 0 } else { 1 };
                g.wrapper.set_visible(*user == mine_shown);
                more.set_text(&if counts[k] == 0 {
                    "No services match.".to_string()
                } else {
                    format!("{} more. Type more to narrow it down.", hidden[k])
                });
                more.set_visible(counts[k] == 0 || hidden[k] > 0);
                widgets::mark_first_rows(g.wrapper.upcast_ref());
            }
            set_other(mine_shown);
        })
    };
    {
        let apply = apply.clone();
        find.connect_search_changed(move |_| apply());
    }
    for (i, t) in toggles.iter().enumerate() {
        let apply = apply.clone();
        t.connect_toggled(move |t| {
            if t.is_active() {
                SHOW_MINE.with(|m| m.set(i == 0));
                apply();
            }
        });
    }
    let picked = chosen.as_ref().and_then(|(user, u)| {
        rows.borrow().iter().find(|(m, n, _, _)| m == user && *n == u.name).map(|x| x.3.clone())
    });
    mark_picked(rows.borrow().iter().map(|x| &x.3), picked.as_ref());
    apply();
    if chosen.is_none() {
        holder.append(&super::extensions::preview_head("Services", "Pick one to see what it's doing and its log.", &[]));
    }
}

/// The labels of a row's text, title first.
fn labels_in(row: &gtk::Box) -> Vec<gtk::Label> {
    let mut out = Vec::new();
    let mut c = row.first_child();
    while let Some(w) = c {
        if w.has_css_class("row-text") {
            let mut l = w.first_child();
            while let Some(x) = l {
                if let Some(x) = x.downcast_ref::<gtk::Label>() {
                    out.push(x.clone());
                }
                l = x.next_sibling();
            }
        }
        c = w.next_sibling();
    }
    out
}

/// NyxOS: your own services, and the system's to look at (they're turned on and off as
/// system settings).
pub fn user_and_system_lists(page: &Page) {
    section(page, "Your services", "Run as you, started when you sign in.", true, list(true), true);
    section(page, "Running on this computer", "The whole computer's services, for reference.", false, list(false), false);
}

fn section(page: &Page, title: &str, note: &str, user: bool, units: Vec<Unit>, can_edit: bool) {
    // The system list is long and rarely needed: it folds away.
    let g = if user { page.group(title) } else { page.collapsible(title, false) };
    g.note(note);
    let results = widgets::vbox(6);
    let units = Rc::new(std::cell::RefCell::new(units));
    let query = Rc::new(std::cell::RefCell::new(String::new()));
    let refill: Slot = Rc::default();

    let fill: Rc<dyn Fn()> = {
        let (results, units, query, refill) = (results.clone(), units.clone(), query.clone(), refill.clone());
        Rc::new(move || {
            // The list is redrawn after the page was built: drop the old rows from
            // search and file the new ones under this page.
            widgets::forget_rows(&results);
            widgets::begin_section("services");
            while let Some(c) = results.first_child() {
                results.remove(&c);
            }
            let q = query.borrow().to_lowercase();
            let shown: Vec<Unit> = units
                .borrow()
                .iter()
                .filter(|u| q.is_empty() || u.name.to_lowercase().contains(&q) || u.description.to_lowercase().contains(&q))
                .take(20)
                .cloned()
                .collect();
            if shown.is_empty() {
                results.append(&widgets::label("No services match.", "dim"));
            }
            let again: Rc<dyn Fn()> = {
                let (units, refill) = (units.clone(), refill.clone());
                Rc::new(move || {
                    // Read the units again off the main thread, then redraw the list.
                    let (units, refill) = (units.clone(), refill.clone());
                    cmd::background(
                        move || list(user),
                        move |fresh| {
                            *units.borrow_mut() = fresh;
                            if let Some(f) = refill.borrow().as_ref().and_then(Weak::upgrade) {
                                f();
                            }
                        },
                    );
                })
            };
            for u in shown {
                results.append(&unit_row(&u, user, can_edit, again.clone()));
            }
        })
    };
    *refill.borrow_mut() = Some(Rc::downgrade(&fill));

    let (r, _) = widgets::entry_row("Find a service", "Part of a name or description. Press Enter.", "", "e.g. bluetooth", {
        let (query, fill) = (query.clone(), fill.clone());
        move |q| {
            *query.borrow_mut() = q.trim().to_string();
            fill();
        }
    });
    widgets::keywords("service daemon systemd unit start stop restart enable disable");
    g.add(&r);
    g.add(&results);
    fill();
}

/// One service: its state, and buttons that fit it.
fn unit_row(u: &Unit, user: bool, can_edit: bool, refill: Rc<dyn Fn()>) -> gtk::Box {
    let running = u.active == "active";
    let controls = widgets::hbox(6);
    if can_edit {
        let act = |label: &str, action: &'static str, unit: String, refill: Rc<dyn Fn()>| {
            let b = gtk::Button::with_label(label);
            b.connect_clicked(move |b| {
                b.set_sensitive(false);
                let (unit, refill) = (unit.clone(), refill.clone());
                cmd::background(
                    move || {
                        if user {
                            cmd::run(&["systemctl", "--user", action, &unit]).map(|_| ()).map_err(|e| e.to_string())
                        } else {
                            acc::helper(&["service", action, &unit], None).map(|_| ()).map_err(|e| e.to_string())
                        }
                    },
                    move |r| {
                        if let Err(e) = r {
                            window::toast(&e);
                        }
                        refill();
                    },
                );
            });
            b
        };
        controls.append(&act(
            if running { "Stop" } else { "Start" },
            if running { "stop" } else { "start" },
            u.name.clone(),
            refill.clone(),
        ));
        if running {
            controls.append(&act("Restart", "restart", u.name.clone(), refill.clone()));
        }
        match u.enabled.as_str() {
            "enabled" => controls.append(&act("Disable", "disable", u.name.clone(), refill.clone())),
            "disabled" => controls.append(&act("Enable", "enable", u.name.clone(), refill.clone())),
            _ => {}
        }
    }
    let state = if u.active == "failed" {
        "failed".to_string()
    } else if running {
        "running".to_string()
    } else {
        "stopped".to_string()
    };
    let boot = match u.enabled.as_str() {
        "enabled" => " · starts at boot",
        _ => "",
    };
    let desc = format!("{}{}", gtk::glib::markup_escape_text(&u.description), boot);
    let r = widgets::row(u.name.trim_end_matches(".service"), &desc, Some(controls.upcast_ref()));
    widgets::tag_row(&r, &state);
    r
}

/// Keep a row's text from asking for more width than the controls column has: long
/// names wrap instead of widening the column.
pub(crate) fn cap_text(row: &gtk::Box, chars: i32) {
    let mut c = row.first_child();
    while let Some(w) = c {
        if w.has_css_class("row-text") {
            let mut l = w.first_child();
            while let Some(x) = l {
                if let Some(x) = x.downcast_ref::<gtk::Label>() {
                    x.set_max_width_chars(chars);
                }
                l = x.next_sibling();
            }
        }
        c = w.next_sibling();
    }
}

/// Make a list row selectable: clicking it, or Enter or Space on it, calls `pick`.
/// Its controls (switches, buttons) keep their clicks.
pub(crate) fn pickable(row: &gtk::Box, pick: impl Fn() + 'static) {
    row.add_css_class("sys-pick");
    row.set_focusable(true);
    row.set_cursor_from_name(Some("pointer"));
    let pick = Rc::new(pick);
    let click = gtk::GestureClick::new();
    let p = pick.clone();
    click.connect_released(move |_, _, _, _| p());
    row.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::space) {
            pick();
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });
    row.add_controller(keys);
}

/// Mark `chosen` as the picked row among `rows`.
pub(crate) fn mark_picked<'a>(rows: impl IntoIterator<Item = &'a gtk::Box>, chosen: Option<&gtk::Box>) {
    for r in rows {
        if Some(r) == chosen {
            r.add_css_class("sys-picked");
        } else {
            r.remove_css_class("sys-picked");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNITS: &str = "alsa-restore.service loaded active exited Save/Restore Sound Card State\nbluetooth.service loaded active running Bluetooth service\nfoo.service not-found inactive dead foo.service\n● bad.service loaded failed failed A broken one\ntemplate@.service loaded inactive dead x\nsome.socket loaded active running A socket\n";
    const FILES: &str = "alsa-restore.service static -\nbluetooth.service enabled disabled\nbad.service disabled -\n";

    #[test]
    fn parses_and_sorts() {
        let u = parse_units(UNITS, FILES);
        let names: Vec<&str> = u.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["alsa-restore.service", "bluetooth.service", "bad.service"]);
        assert_eq!(u[1].enabled, "enabled");
        assert_eq!(u[1].description, "Bluetooth service");
        assert_eq!(u[2].active, "failed");
        assert_eq!(u[0].enabled, "static");
    }
}
