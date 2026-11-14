//! System › Updates on NyxOS: what has updates, by kind (the system, applications,
//! developer tools, firmware) and one by one, updating any of them, each kind's update
//! setting, the history and going back. It's the engine `nyx update` uses, asked through
//! `nyx-system update … --json`, so Settings and the command line plan and update the
//! same way; nothing here updates anything itself.

use super::pages;
use super::{busy, set_busy};
use crate::sketch;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use serde::Deserialize;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    #[serde(default)]
    status: Summary,
    #[serde(default)]
    targets: Vec<Target>,
    #[serde(default)]
    firmware: Firmware,
    /// When it was checked (seconds since the epoch).
    #[serde(default)]
    last_checked: i64,
    /// There's no network: this is the last check's.
    #[serde(default)]
    offline: bool,
    /// Each kind's update setting: off, check, notify or auto.
    #[serde(default)]
    policies: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
struct Summary {
    #[serde(default)]
    notes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    #[serde(default)]
    id: String,
    name: String,
    category: String,
    provider: String,
    installed: Option<String>,
    available: Option<String>,
    #[serde(default)]
    needs_channel: bool,
    /// Whether a newer version could be known (otherwise it's checked when it updates).
    #[serde(default = "yes")]
    checkable: bool,
    /// `app` (reopen it) or `computer` (restart).
    #[serde(default)]
    restart: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
enum Firmware {
    #[default]
    NotChecked,
    Unavailable {
        reason: String,
    },
    Devices {
        devices: Vec<Device>,
    },
}

#[derive(Debug, Deserialize)]
struct Device {
    name: String,
    version: String,
    update: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Outcome {
    #[serde(default)]
    steps: Vec<StepOutcome>,
    #[serde(default)]
    reboot: bool,
    #[serde(default)]
    restart: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct StepOutcome {
    ok: bool,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryEntry {
    time: i64,
    request: String,
    result: String,
    #[serde(default)]
    changes: Vec<serde_json::Value>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    reboot: bool,
    /// How long each stage took; `total` is the whole.
    #[serde(default)]
    seconds: std::collections::BTreeMap<String, u64>,
}

/// `4 min`, `42 s`.
fn took(secs: u64) -> String {
    if secs < 60 { format!("{secs} s") } else { format!("{} min", (secs + 30) / 60) }
}

/// What came of an update, in one line.
fn describe(o: &Outcome) -> String {
    if let Some(failed) = o.steps.iter().find(|s| !s.ok) {
        let why = failed.message.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(&failed.message);
        let some = o.steps.iter().any(|s| s.ok);
        return format!("{}{}", if some { "Partly updated. " } else { "" }, why.trim());
    }
    if o.steps.is_empty() {
        return "Everything is up to date".into();
    }
    if o.reboot {
        return "Updated. Restart the computer to finish.".into();
    }
    if !o.restart.is_empty() {
        return format!("Updated. Reopen {} to use the new versions.", o.restart.join(", "));
    }
    o.steps.iter().map(|s| s.message.as_str()).collect::<Vec<_>>().join(". ")
}

/// `nyx-system update WHAT… --yes --json`, then a toast saying what came of it.
pub fn update(what: &[&str], working: &str) {
    if busy() {
        window::toast("Another system change is still running.");
        return;
    }
    set_busy(true);
    window::toast(working);
    let mut args = vec!["nyx-system", "update"];
    args.extend_from_slice(what);
    args.extend(["--yes", "--json"]);
    cmd::run_async(&args, |r| {
        set_busy(false);
        match r {
            Ok(out) => {
                let outcome: Outcome = serde_json::from_str(&out).unwrap_or_default();
                window::toast(&describe(&outcome));
            }
            Err(e) => {
                let text = format!("{e:#}");
                let last = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(&text);
                window::toast(last.trim().trim_start_matches("nyx-system: "));
            }
        }
        window::rebuild("updates");
    });
}

fn yes() -> bool {
    true
}

/// How a target is updated, in a few words (nyx-update's `how`).
fn how(t: &Target) -> &'static str {
    match t.provider.as_str() {
        "channel" if t.category == "system" => "The system channel",
        "channel" => "Comes with the system",
        "tool-pin" => "A NyxOS tool, for everyone",
        "profile" => "Yours (your Nix profile)",
        "flatpak" => "A Flatpak",
        "mise" => "Your mise tools",
        "uv-tool" => "Your uv tools",
        "firmware" => "Firmware",
        _ => "",
    }
}

/// The kinds, in order: the category `nyx update` takes, its title, and what it covers.
const KINDS: [(&str, &str, &str); 4] = [
    ("system", "System", "NyxOS and the Nyx apps built into the desktop, as one new generation."),
    ("apps", "Applications", "Your apps (Nyx apps, apps from the catalog and Flatpaks), each updated on its own."),
    ("tools", "Developer Tools", "Coding agents and your mise and uv tools, managed by NyxOS rather than their own updaters."),
    ("firmware", "Firmware", "From fwupd and LVFS, for devices that support it. Firmware can't be undone."),
];

/// The update settings a kind can have.
fn policies(kind: &str) -> Vec<(String, String)> {
    let mut v = vec![
        ("off".to_string(), "Don't check".to_string()),
        ("check".to_string(), "Only check".to_string()),
        ("notify".to_string(), "Notify me".to_string()),
    ];
    // The system and firmware are never installed unattended.
    if matches!(kind, "apps" | "tools") {
        v.push(("auto".to_string(), "Automatically".to_string()));
    }
    v
}

/// `nyx-system config get updates KEY`.
fn text_setting(key: &str) -> Option<String> {
    cmd::read(&["nyx-system", "config", "get", "updates", key]).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn set_text_setting(key: &'static str, value: String) {
    cmd::run_async(&["nyx-system", "config", "set", "updates", key, &format!("\"{value}\"")], |r| {
        if let Err(e) = r {
            window::toast(&format!("Couldn't save: {e:#}"));
        }
    });
}

/// The update setting key for a kind (`firmware` is taken by "include firmware").
fn policy_key(kind: &str) -> &'static str {
    match kind {
        "system" => "system",
        "apps" => "apps",
        "tools" => "tools",
        _ => "firmware-policy",
    }
}

fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "system" => "computer-symbolic",
        "apps" => "view-app-grid-symbolic",
        "tools" => "utilities-terminal-symbolic",
        _ => "media-flash-symbolic",
    }
}

/// The last check, once it's back.
type Checked = Rc<RefCell<Option<Rc<Status>>>>;

/// Undo the last install, removal or update of your own apps (your Nix profile).
pub(super) fn undo_button(section: &'static str) -> gtk::Button {
    let undo = widgets::confirm_button("Undo your last app change", "Undo your last app change?", move |b| {
        b.set_sensitive(false);
        cmd::run_async(&["nix", "profile", "rollback"], move |r| {
            window::toast(match r {
                Ok(_) => "Your apps are back as they were before the last change.",
                Err(_) => "Your apps couldn't be put back. Run nyx rollback in a terminal to see why.",
            });
            window::rebuild(section);
        });
    });
    undo.add_css_class("nx2-link");
    undo.set_valign(gtk::Align::Start);
    undo
}

/// What a kind has, in a line: what's waiting, or that it's up to date.
fn kind_summary(s: &Status, kind: &str) -> (String, bool) {
    if s.policies.get(kind).is_some_and(|p| p == "off") {
        return ("Not checked (its update setting is Don't check).".into(), false);
    }
    // The channel (and so the tools' newest pins) couldn't be asked: not "up to date".
    let unchecked = s.status.notes.iter().any(|n| n.starts_with("The system channel couldn't be checked"));
    let list: Vec<&Target> = s.targets.iter().filter(|t| t.category == kind).collect();
    let news: Vec<&&Target> = list.iter().filter(|t| t.available.is_some()).collect();
    let restart = news.iter().any(|t| t.restart == "computer");
    let text = match news.as_slice() {
        [one] => format!("{} {}{}", one.name, one.available.as_deref().unwrap_or(""), if restart { " · restart needed" } else { "" }),
        [] if kind == "firmware" => match &s.firmware {
            Firmware::Unavailable { reason } => reason.clone(),
            Firmware::NotChecked => "Not checked.".into(),
            Firmware::Devices { devices } if devices.is_empty() => "No device here gets firmware through fwupd.".into(),
            _ => "Up to date.".into(),
        },
        [] if list.is_empty() => "Nothing here yet.".into(),
        [] if unchecked && kind != "apps" => "Couldn't be checked right now.".into(),
        [] => "Up to date.".into(),
        many => format!("{} with updates{}", many.len(), if restart { " · restart needed" } else { "" }),
    };
    // Apps that come with the system update with it: one line, not one row each.
    let with_system = list.iter().filter(|t| t.provider == "channel" && t.category != "system").count();
    let text = if with_system > 0 {
        format!(
            "{text} {with_system} app{} built into the system update{} with it.",
            if with_system == 1 { "" } else { "s" },
            if with_system == 1 { "s" } else { "" }
        )
    } else {
        text
    };
    (text, !news.is_empty())
}

/// One kind on a page of its own: everything in it, each with its own Update.
fn kind_page(kind: &'static str, title: &'static str, what: &'static str, checked: Checked) {
    widgets::subpage(title, 600, move |p| {
        let g = p.group(title);
        g.note(what);
        let Some(s) = checked.borrow().clone() else {
            g.note("Still checking for updates. Open this again in a moment.");
            return;
        };
        let (summary, has) = kind_summary(&s, kind);
        let button = if kind == "firmware" {
            widgets::confirm_button("Update", "Can't be undone. Update?", |b| {
                b.set_sensitive(false);
                update(&["firmware"], "Updating firmware. Keep the computer on and plugged in.");
            })
        } else {
            let b = gtk::Button::with_label("Update");
            b.connect_clicked(move |b| {
                b.set_sensitive(false);
                update(&[kind], &format!("Updating {}…", title.to_lowercase()));
            });
            b
        };
        button.set_visible(has && !busy());
        g.add(&widgets::row(&format!("All {}", title.to_lowercase()), &summary, Some(button.upcast_ref())));
        for t in s.targets.iter().filter(|t| t.category == kind && (t.provider != "channel" || t.category == "system")) {
            g.add(&target_row(t));
        }
        if kind == "firmware"
            && let Firmware::Devices { devices } = &s.firmware
        {
            for d in devices.iter().filter(|d| d.update.is_none()) {
                g.add(&widgets::row(&d.name, &format!("{}, up to date", d.version), None));
            }
        }
    });
}

/// A generation on the timeline: its dot on the line, and its card.
#[derive(Clone, Copy, PartialEq)]
enum Dot {
    /// What the waiting update builds.
    Next,
    Running,
    Earlier,
    /// "N earlier generations".
    More,
}

fn timeline_node(kind: Dot, up: Rc<Cell<bool>>, down: bool, body: &impl IsA<gtk::Widget>) -> (gtk::Box, gtk::DrawingArea) {
    let row = widgets::hbox(10);
    row.add_css_class("nx2-upd-node");
    let rail = gtk::DrawingArea::new();
    rail.set_content_width(22);
    rail.set_draw_func(move |_, cr, w, h| {
        let p = sketch::pal();
        let (x, y) = (w as f64 / 2.0, if kind == Dot::More { 12.0 } else { 24.0 });
        sketch::set(cr, p.dim, 0.35);
        cr.set_line_width(2.0);
        cr.move_to(x, if up.get() { 0.0 } else { y });
        cr.line_to(x, if down { h as f64 } else { y });
        let _ = cr.stroke();
        let green = (0.33, 0.78, 0.45);
        match kind {
            Dot::Next => {
                cr.set_dash(&[3.0, 2.5], 0.0);
                sketch::set(cr, p.accent, 1.0);
                cr.arc(x, y, 9.0, 0.0, std::f64::consts::TAU);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
                sketch::set(cr, p.bg, 1.0);
                cr.arc(x, y, 5.5, 0.0, std::f64::consts::TAU);
                let _ = cr.fill_preserve();
                sketch::set(cr, p.accent, 1.0);
                let _ = cr.stroke();
            }
            Dot::Running => {
                sketch::set(cr, green, 0.3);
                cr.arc(x, y, 9.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
                sketch::set(cr, green, 1.0);
                cr.arc(x, y, 6.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
            Dot::Earlier | Dot::More => {
                let r = if kind == Dot::More { 3.0 } else { 5.5 };
                sketch::set(cr, p.bg, 1.0);
                cr.arc(x, y, r, 0.0, std::f64::consts::TAU);
                let _ = cr.fill_preserve();
                sketch::set(cr, p.dim, 0.9);
                cr.set_line_width(1.8);
                let _ = cr.stroke();
            }
        }
    });
    row.append(&rail);
    body.set_hexpand(true);
    row.append(body);
    (row, rail)
}

/// A generation's card: its number, when, what's special about it, and a button.
fn generation_card(title: &str, when: &str, pills: &[gtk::Label], button: Option<&gtk::Widget>, lines: &[&str]) -> gtk::Box {
    let c = widgets::vbox(4);
    c.add_css_class("preview-card");
    c.add_css_class("nx2-upd-gen");
    let top = widgets::hbox(8);
    top.append(&widgets::label(title, "nx2-upd-gen-title"));
    let w = widgets::label(when, "nx2-dim");
    w.set_valign(gtk::Align::Center);
    top.append(&w);
    for p in pills {
        top.append(p);
    }
    let gap = widgets::hbox(0);
    gap.set_hexpand(true);
    top.append(&gap);
    if let Some(b) = button {
        b.set_valign(gtk::Align::Center);
        top.append(b);
    }
    c.append(&top);
    for l in lines {
        let l = widgets::label(l, "nx2-dim");
        l.set_wrap(true);
        c.append(&l);
    }
    c
}

/// System › Updates: what's waiting and the four kinds beside the system's generations.
pub fn page(page: &Page) {
    let checked: Checked = Rc::default();

    // The picture: the generations, newest first, and the one an update would build.
    let pv = page.split();
    let column = pages::preview_column(&pv);
    let (head, _, _) = pages::preview_head(
        "Generations",
        "Each system update or system change makes one. Going back undoes the newest; your files stay.",
        &[],
    );
    column.append(&head);
    let timeline = widgets::vbox(0);
    timeline.add_css_class("nx2-upd-timeline");
    let next = widgets::vbox(0);
    timeline.append(&next);
    let list = super::generations();
    let current = list.iter().find(|g| g.current).map(|g| g.number);
    let previous = list.iter().find(|x| !x.current && list.iter().any(|c| c.current && c.number > x.number)).map(|x| x.number);
    let has_next = Rc::new(Cell::new(false));
    let mut first_rail: Option<gtk::DrawingArea> = None;
    const SHOWN: usize = 6;
    let more = list.len().saturating_sub(SHOWN);
    for (i, g) in list.iter().take(SHOWN).enumerate() {
        let mut pills = Vec::new();
        if g.current {
            pills.push(pages::pill("Running", "ok"));
        }
        let back = (Some(g.number) == previous).then(|| {
            let b = widgets::confirm_button("Go back to this", &format!("Go back to generation {}?", g.number), |b| {
                b.set_sensitive(false);
                super::rollback(|_| window::rebuild("updates"));
            });
            b.set_sensitive(!busy());
            b
        });
        let card = generation_card(&format!("Generation {}", g.number), &short_date(g.created), &pills, back.as_ref().map(|b| b.upcast_ref()), &[]);
        if g.current {
            card.add_css_class("nx2-upd-running");
        }
        let up = if i == 0 { has_next.clone() } else { Rc::new(Cell::new(true)) };
        let last = i + 1 == list.len().min(SHOWN) && more == 0;
        let (node, rail) = timeline_node(if g.current { Dot::Running } else { Dot::Earlier }, up, !last, &card);
        if i == 0 {
            first_rail = Some(rail);
        }
        timeline.append(&node);
    }
    if more > 0 {
        let l = widgets::label(
            &format!(
                "{more} earlier generation{} · in the boot menu until Disks › Clean up removes them",
                if more == 1 { "" } else { "s" }
            ),
            "nx2-dim",
        );
        l.set_wrap(true);
        l.add_css_class("nx2-upd-more");
        let (node, _) = timeline_node(Dot::More, Rc::new(Cell::new(true)), false, &l);
        timeline.append(&node);
    }
    if list.is_empty() {
        let c = widgets::vbox(4);
        c.add_css_class("preview-card");
        let l = widgets::label("No generations were found on this computer.", "nx2-dim");
        l.set_wrap(true);
        c.append(&l);
        timeline.append(&c);
    }
    column.append(&timeline);
    let foot = pv.foot("dialog-information-symbolic", "Your own apps have their own history.");
    pages::foot_end(&foot, &undo_button("updates"));

    // What's waiting, and Update Everything.
    let g = page.plain_group("");
    let ready = widgets::hbox(14);
    ready.add_css_class("preview-card");
    ready.add_css_class("nx2-upd-ready");
    let badge = gtk::Overlay::new();
    let tile = widgets::tile("software-update-available-symbolic");
    tile.add_css_class("nx2-upd-badge");
    badge.set_child(Some(&tile));
    let count = widgets::label("", "nx2-upd-count");
    count.set_halign(gtk::Align::End);
    count.set_valign(gtk::Align::Start);
    count.set_visible(false);
    badge.add_overlay(&count);
    badge.set_valign(gtk::Align::Center);
    ready.append(&badge);
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let title = widgets::label("Checking for updates…", "nx2-upd-title");
    title.set_wrap(true);
    text.append(&title);
    let sub = widgets::label(&super::version(), "nx2-dim");
    sub.set_wrap(true);
    text.append(&sub);
    ready.append(&text);
    let everything = gtk::Button::with_label("Update Everything");
    everything.add_css_class("suggested-action");
    everything.set_valign(gtk::Align::Center);
    everything.set_sensitive(!busy());
    everything.connect_clicked(|b| {
        b.set_sensitive(false);
        update(&[], "Updating everything. This can take a while.");
    });
    ready.append(&everything);
    let holder = widgets::stacked_row("", "", ready.upcast_ref());
    holder.add_css_class("bare");
    widgets::keywords("update upgrade nixos flake channel everything");
    g.add(&holder);
    let line = widgets::hbox(8);
    line.add_css_class("nx2-upd-checked");
    let mark = gtk::Image::from_icon_name("emblem-synchronizing-symbolic");
    line.append(&mark);
    let checked_text = widgets::label("Checking for updates…", "nx2-dim");
    checked_text.set_wrap(true);
    checked_text.set_hexpand(true);
    line.append(&checked_text);
    let check = pages::link("Check now");
    check.set_sensitive(false);
    check.connect_clicked(|b| {
        b.set_sensitive(false);
        window::rebuild("updates");
    });
    line.append(&check);
    g.add(&line);
    let notes = widgets::label("", "group-note");
    notes.set_wrap(true);
    notes.set_visible(false);
    g.add(&notes);

    // The four kinds, each a row with its update setting; each opens with everything in it.
    let g = pages::head_group(page, "Four kinds", &[pages::head_note("When there's an update").upcast_ref()], true);
    let mut kinds: Vec<(&'static str, gtk::Label)> = Vec::new();
    for (kind, title, what) in KINDS {
        let current = text_setting(policy_key(kind)).unwrap_or_else(|| default_policy(kind).to_string());
        let choices = policies(kind);
        let dd = widgets::dropdown(&choices, &current);
        dd.connect_selected_notify(move |d| {
            if let Some((id, _)) = choices.get(d.selected() as usize) {
                set_text_setting(policy_key(kind), id.clone());
            }
        });
        let end = widgets::hbox(8);
        end.append(&dd);
        let chevron = gtk::Image::from_icon_name("go-next-symbolic");
        chevron.add_css_class("chevron");
        end.append(&chevron);
        let r = widgets::icon_row(kind_icon(kind), title, "Checking for updates…", Some(end.upcast_ref()));
        r.add_css_class("nav-row");
        r.set_cursor_from_name(Some("pointer"));
        let click = gtk::GestureClick::new();
        let c = checked.clone();
        click.connect_released(move |_, _, _, _| kind_page(kind, title, what, c.clone()));
        r.add_controller(click);
        widgets::keywords("automatic updates policy notify check install");
        let summary = pages::description_of(r.upcast_ref()).unwrap_or_else(|| gtk::Label::new(None));
        g.add(&r);
        kinds.push((kind, summary));
    }

    // The update settings and the history, each a level down.
    let g = page.group("");
    pages::gap(&g);
    let mut said = vec![
        if setting("check", true) { "Daily check" } else { "No daily check" }.to_string(),
        if setting("notify", true) { "notify" } else { "no notifications" }.to_string(),
    ];
    said.push(match text_setting("install-at") {
        Some(t) => format!("install at {t}"),
        None => "install when found".into(),
    });
    let (r, _) = widgets::nav_row(Some("preferences-system-symbolic"), "Update settings", &said.join(" · "), "", || {
        widgets::subpage("Update settings", 600, settings);
    });
    widgets::keywords("automatic updates check notify metered firmware schedule time night");
    g.add(&r);
    let recent = read_history(6);
    let desc = match recent.first() {
        None => "Nothing has been updated yet".to_string(),
        Some(e) => {
            let failed = recent.iter().filter(|e| e.result == "failed").count();
            let failed = if failed > 0 { format!(" · {failed} of the last {} failed", recent.len()) } else { String::new() };
            format!("Last: {}{failed}", short_date(e.time))
        }
    };
    let (r, _) = widgets::nav_row(Some("document-open-recent-symbolic"), "Update history", &desc, "", || {
        widgets::subpage("Update history", 620, history);
    });
    widgets::keywords("update history log rollback undo previous generation revert recovery");
    g.add(&r);

    cmd::run_async(&["nyx-system", "update", "status", "--json"], move |r| {
        check.set_sensitive(!busy());
        let Ok(s) = r.map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Status>(&t).map_err(|e| e.to_string()))
        else {
            checked_text.set_text("Updates couldn't be checked. Try again later, or run nyx update in a terminal.");
            mark.set_icon_name(Some("dialog-warning-symbolic"));
            title.set_text("Not checked");
            for (_, summary) in &kinds {
                summary.set_text("Not checked");
            }
            return;
        };
        let when = if s.last_checked > 0 { format!("Checked {}", short_date(s.last_checked)) } else { "Checked just now".into() };
        checked_text.set_text(&if s.offline { format!("{when}. There's no network connection, so this is that check's.") } else { when });
        mark.set_icon_name(Some(if s.offline { "dialog-warning-symbolic" } else { "emblem-ok-symbolic" }));
        let mut waiting = 0;
        for (kind, summary) in &kinds {
            let (text, has) = kind_summary(&s, kind);
            summary.set_text(&text);
            if has {
                summary.add_css_class("nx2-upd-has");
            }
            if !s.policies.get(*kind).is_some_and(|p| p == "off") {
                waiting += s.targets.iter().filter(|t| t.category == *kind && t.available.is_some()).count();
            }
        }
        let restart = s.targets.iter().any(|t| t.available.is_some() && t.restart == "computer");
        title.set_text(&match waiting {
            0 => "Everything is up to date".to_string(),
            1 => "1 update is ready".to_string(),
            n => format!("{n} updates are ready"),
        });
        if waiting > 0 {
            count.set_text(&waiting.to_string());
            count.set_visible(true);
            sub.set_text(&format!("{}{}", super::version(), if restart { " · a restart after" } else { "" }));
        }
        let other: Vec<&String> = s.status.notes.iter().filter(|n| !n.starts_with("Firmware:")).collect();
        if !other.is_empty() {
            notes.set_text(&other.iter().map(|n| n.as_str()).collect::<Vec<_>>().join("\n"));
            notes.set_visible(true);
        }
        // The generation a system update would build.
        let system: Vec<String> = s
            .targets
            .iter()
            .filter(|t| t.available.is_some() && (t.category == "system" || t.needs_channel))
            .map(|t| format!("{} {}", t.name, t.available.as_deref().unwrap_or("")))
            .collect();
        if !system.is_empty() {
            let title = current.map(|n| format!("Generation {}", n + 1)).unwrap_or_else(|| "The next generation".into());
            let mut after = "It's switched to only when the build works".to_string();
            if restart {
                after.push_str(", then a restart finishes it");
            }
            after.push('.');
            if let Some(n) = current {
                after.push_str(&format!(" Generation {n} stays in the boot menu."));
            }
            let card = generation_card(&title, "After this update", &[pages::pill("Built first", "accent")], None, &[&system.join(" and "), &after]);
            card.add_css_class("nx2-upd-next");
            let (node, _) = timeline_node(Dot::Next, Rc::new(Cell::new(false)), !list.is_empty(), &card);
            next.append(&node);
            has_next.set(true);
            if let Some(r) = &first_rail {
                r.queue_draw();
            }
        }
        *checked.borrow_mut() = Some(Rc::new(s));
    });
}

fn default_policy(kind: &str) -> &'static str {
    match kind {
        "apps" | "tools" => "auto",
        _ => "notify",
    }
}

/// One thing that can be updated: its versions, how it's updated, and its own button.
fn target_row(t: &Target) -> gtk::Box {
    let state = match (&t.installed, &t.available) {
        (Some(a), Some(b)) => format!("{a} → {b}"),
        (None, Some(b)) => format!("→ {b}"),
        (Some(a), None) if t.checkable => format!("{a}, up to date"),
        (Some(a), None) => format!("{a}, checked when it updates"),
        (None, None) => "Checked when it updates".into(),
    };
    let desc = match how(t) {
        "" => state,
        h => format!("{state} · {h}"),
    };
    let control = (t.available.is_some() && !t.needs_channel && !busy()).then(|| {
        let id = t.id.clone();
        let name = t.name.clone();
        if t.category == "firmware" {
            widgets::confirm_button("Update", "Can't be undone. Update?", move |b| {
                b.set_sensitive(false);
                update(&[&id], &format!("Updating {name}. Keep the computer on and plugged in."));
            })
        } else {
            let b = gtk::Button::with_label("Update");
            b.connect_clicked(move |b| {
                b.set_sensitive(false);
                update(&[&id], &format!("Updating {name}…"));
            });
            b
        }
    });
    let r = widgets::row(&t.name, &desc, control.as_ref().map(|b| b.upcast_ref()));
    if t.needs_channel && t.available.is_some() {
        widgets::tag_row(&r, "With the system update");
    } else if t.available.is_some() && t.restart == "computer" {
        widgets::tag_row(&r, "Restart");
    }
    r
}

fn short_date(secs: i64) -> String {
    gtk::glib::DateTime::from_unix_local(secs)
        .and_then(|d| d.format("%e %b, %H:%M"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn date(secs: i64) -> String {
    gtk::glib::DateTime::from_unix_local(secs)
        .and_then(|d| d.format("%e %b %Y, %H:%M"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// One update in the history, as a row.
fn history_row(e: &HistoryEntry) -> gtk::Box {
    let what = match e.result.as_str() {
        "failed" => format!("Failed: {}", e.error.as_deref().unwrap_or("see nyx update history")),
        "current" => "Already up to date".to_string(),
        "rolled-back" => "Went back a generation".to_string(),
        _ if e.changes.is_empty() => "Updated".to_string(),
        _ => format!("Updated, {} package{} changed", e.changes.len(), if e.changes.len() == 1 { "" } else { "s" }),
    };
    let restart = if e.reboot { " Needed a restart." } else { "" };
    let duration = e.seconds.get("total").map(|s| format!(" · {}", took(*s))).unwrap_or_default();
    let r = widgets::row(&e.request, &format!("{}{duration} · {what}.{restart}", date(e.time)), None);
    if e.result == "failed" {
        widgets::tag_row(&r, "Failed");
    }
    r
}

fn read_history(n: usize) -> Vec<HistoryEntry> {
    cmd::read(&["nyx-system", "update", "history", &n.to_string(), "--json"])
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// The last few updates, the system's and this user's, and all of them in a dialog.
pub fn history(page: &Page) {
    let list = read_history(6);
    let g = page.group("Update History");
    if list.is_empty() {
        g.note("Nothing has been updated yet.");
        return;
    }
    for e in &list {
        g.add(&history_row(e));
    }
    let (r, _) = widgets::button_row("All updates", "The last 100, newest first.", "View All", |_| {
        let (dialog, card) = widgets::dialog("Update History", 620);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_min_content_height(420);
        scroll.set_propagate_natural_height(true);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("settings-card");
        for e in read_history(100) {
            list.append(&history_row(&e));
        }
        scroll.set_child(Some(&list));
        card.append(&scroll);
        dialog.present();
    });
    g.add(&r);
    widgets::keywords("update history log");
}

/// `nyx-system config get updates KEY`, or `default` when it isn't set.
fn setting(key: &str, default: bool) -> bool {
    match cmd::read(&["nyx-system", "config", "get", "updates", key]) {
        Ok(v) => v.trim() == "true",
        Err(_) => default,
    }
}

fn set_setting(key: &'static str, on: bool) {
    let value = if on { "true" } else { "false" };
    cmd::run_async(&["nyx-system", "config", "set", "updates", key, value], |r| {
        if let Err(e) = r {
            window::toast(&format!("Couldn't save: {e:#}"));
        }
    });
}

/// The daily check and what it does.
pub fn settings(page: &Page) {
    let g = page.group("Update Settings");
    let rows: [(&'static str, &str, &str, bool); 4] = [
        ("check", "Check daily", "Look for updates once a day, in the background. Off turns every kind off.", true),
        ("notify", "Notify", "Notifications for updates, automatic installs and failures, once per update.", true),
        (
            "check-on-metered",
            "Check on metered connections",
            "Otherwise checks and automatic installs wait for a connection that isn't metered.",
            false,
        ),
        ("firmware", "Include firmware", "Checks ask fwupd too. Firmware is only ever updated when you choose to.", true),
    ];
    for (key, title, desc, default) in rows {
        let (r, _) = widgets::switch_row(title, desc, setting(key, default), move |on| set_setting(key, on));
        g.add(&r);
    }
    let mut times = vec![(String::new(), "When they're found".to_string())];
    times.extend((0..24).map(|h| (format!("{h:02}:00"), format!("At {h:02}:00"))));
    let now = text_setting("install-at").unwrap_or_default();
    let (r, _) = widgets::choice_row(
        "Install automatic updates",
        "For kinds set to install automatically. Running apps keep going and use the new version when reopened.",
        times,
        &now,
        |t| {
            if t.is_empty() {
                cmd::run_async(&["nyx-system", "config", "reset", "updates", "install-at"], |_| {});
            } else {
                set_text_setting("install-at", t);
            }
        },
    );
    g.add(&r);
    widgets::keywords("automatic updates check notify metered firmware schedule time night");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_read_as_one_line() {
        let o: Outcome = serde_json::from_str(
            r#"{"steps": [{"step": {"kind": "system", "channel": true, "tools": []}, "ok": true, "message": "Updated NyxOS (3 packages changed)"}], "reboot": true, "restart": []}"#,
        )
        .unwrap();
        assert_eq!(describe(&o), "Updated. Restart the computer to finish.");
        let o: Outcome = serde_json::from_str(
            r#"{"steps": [{"step": {"kind": "system"}, "ok": true, "message": "Updated Claude Code 2.1.311"},
                {"step": {"kind": "flatpak"}, "ok": false, "message": "flatpak failed: no network"}], "reboot": false, "restart": []}"#,
        )
        .unwrap();
        assert_eq!(describe(&o), "Partly updated. flatpak failed: no network");
        assert_eq!(describe(&Outcome::default()), "Everything is up to date");
    }

    #[test]
    fn status_json_reads() {
        let s: Status = serde_json::from_str(
            r#"{"status": {"available": [], "notes": ["x"]},
                "targets": [{"id": "claude-code", "name": "Claude Code", "category": "tools", "provider": "tool-pin",
                  "installed": "2.1.152", "available": "2.1.311", "aliases": [], "admin": true, "restart": "app",
                  "rollback": true, "needsChannel": false, "key": "claude-code"}],
                "firmware": {"state": "unavailable", "reason": "fwupd isn't running"}}"#,
        )
        .unwrap();
        assert_eq!(s.targets[0].available.as_deref(), Some("2.1.311"));
        assert_eq!(how(&s.targets[0]), "A NyxOS tool, for everyone");
        assert!(matches!(s.firmware, Firmware::Unavailable { .. }));
    }
}
