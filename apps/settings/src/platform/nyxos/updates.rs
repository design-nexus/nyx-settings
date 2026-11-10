//! System › Updates on NyxOS: what has updates, by kind (the system, applications,
//! developer tools, firmware) and one by one, updating any of them, each kind's update
//! setting, the history and going back. It's the engine `nyx update` uses, asked through
//! `nyx-system update … --json`, so Settings and the command line plan and update the
//! same way; nothing here updates anything itself.

use super::{busy, set_busy};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use serde::Deserialize;
use std::collections::BTreeMap;

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
    ("system", "System", "NixOS, NyxOS and the Nyx apps built into the desktop, as one new generation."),
    ("apps", "Applications", "Your apps (Nyx apps, apps from the catalog and Flatpaks), each updated on its own."),
    ("tools", "Developer Tools", "Coding agents and your mise and uv tools, managed by NyxOS rather than their own updaters."),
    ("firmware", "Firmware", "From fwupd and LVFS, for devices that support it. Firmware can't be undone."),
];

/// The update settings a kind can have.
fn policies(kind: &str) -> Vec<(String, String)> {
    let mut v = vec![
        ("off".to_string(), "Don't check".to_string()),
        ("check".to_string(), "Check, without notifying".to_string()),
        ("notify".to_string(), "Notify me; I install".to_string()),
    ];
    // The system and firmware are never installed unattended.
    if matches!(kind, "apps" | "tools") {
        v.push(("auto".to_string(), "Install automatically".to_string()));
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

/// One kind's group, filled in once the check is back.
struct Kind {
    group: widgets::Group,
    summary: gtk::Label,
    button: gtk::Button,
}

/// System › Updates.
pub fn page(page: &Page) {
    let g = page.group("Updates");
    let (r, b) = widgets::button_row(
        &super::version(),
        "The system, your apps and tools, each the way it's installed. The system part is a new generation, switched to only when it builds. Firmware is only updated when you choose.",
        "Update Everything",
        |b| {
            b.set_sensitive(false);
            update(&[], "Updating everything. This can take a while.");
        },
    );
    b.add_css_class("suggested-action");
    b.set_sensitive(!busy());
    widgets::keywords("update upgrade nixos flake channel everything");
    g.add(&r);
    let (checked_row, check) = widgets::button_row("Last checked", "Checking for updates…", "Check Now", |b| {
        b.set_sensitive(false);
        window::rebuild("updates");
    });
    check.set_sensitive(false);
    let checked = checked_row.first_child().and_then(|t| t.last_child()).and_then(|l| l.downcast::<gtk::Label>().ok());
    g.add(&checked_row);
    let notes = gtk::Label::new(None);
    notes.add_css_class("group-note");
    notes.set_xalign(0.0);
    notes.set_wrap(true);
    notes.set_visible(false);
    g.top(&notes);

    let mut kinds: Vec<(&'static str, Kind)> = Vec::new();
    for (kind, title, what) in KINDS {
        let group = page.group(title);
        group.note(what);
        let current = text_setting(policy_key(kind)).unwrap_or_else(|| default_policy(kind).to_string());
        let (r, _) = widgets::choice_row("When there's an update", "", policies(kind), &current, move |p| {
            set_text_setting(policy_key(kind), p);
        });
        widgets::keywords("automatic updates policy notify check install");
        group.add(&r);
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
        button.set_visible(false);
        let row = widgets::row(&format!("All {}", title.to_lowercase()), "Checking for updates…", Some(button.upcast_ref()));
        let summary = row
            .first_child()
            .and_then(|t| t.last_child())
            .and_then(|l| l.downcast::<gtk::Label>().ok())
            .unwrap_or_else(|| gtk::Label::new(None));
        group.add(&row);
        kinds.push((kind, Kind { group, summary, button }));
    }

    cmd::run_async(&["nyx-system", "update", "status", "--json"], move |r| {
        check.set_sensitive(!busy());
        let Ok(s) = r.map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Status>(&t).map_err(|e| e.to_string()))
        else {
            if let Some(l) = &checked {
                l.set_text("Updates couldn't be checked. Try again later, or run nyx update in a terminal.");
            }
            return;
        };
        if let Some(l) = &checked {
            let when = if s.last_checked > 0 { date(s.last_checked) } else { "just now".into() };
            l.set_text(&if s.offline {
                format!("{when}. There's no network connection, so this is that check's.")
            } else {
                when
            });
        }
        // The channel (and so the tools' newest pins) couldn't be asked: not "up to date".
        let unchecked = s.status.notes.iter().any(|n| n.starts_with("The system channel couldn't be checked"));
        for (kind, k) in &kinds {
            if s.policies.get(*kind).is_some_and(|p| p == "off") {
                k.summary.set_text("Not checked (its update setting is Don't check).");
                continue;
            }
            let list: Vec<&Target> = s.targets.iter().filter(|t| t.category == *kind).collect();
            let news: Vec<&&Target> = list.iter().filter(|t| t.available.is_some()).collect();
            let text = if !news.is_empty() {
                let restart = news.iter().any(|t| t.restart == "computer");
                format!("{} with updates{}", news.len(), if restart { ". A restart of the computer may be needed." } else { "." })
            } else if *kind == "firmware" {
                match &s.firmware {
                    Firmware::Unavailable { reason } => reason.clone(),
                    Firmware::NotChecked => "Not checked.".into(),
                    Firmware::Devices { devices } if devices.is_empty() => "No device here gets firmware through fwupd.".into(),
                    _ => "Up to date.".into(),
                }
            } else if list.is_empty() {
                "Nothing here yet.".into()
            } else if unchecked && *kind != "apps" {
                "Couldn't be checked right now.".into()
            } else {
                "Up to date.".into()
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
            k.summary.set_text(&text);
            k.button.set_visible(!news.is_empty() && !busy());
            for t in list.iter().filter(|t| t.provider != "channel" || t.category == "system") {
                k.group.add(&target_row(t));
            }
            if *kind == "firmware"
                && let Firmware::Devices { devices } = &s.firmware
            {
                for d in devices.iter().filter(|d| d.update.is_none()) {
                    k.group.add(&widgets::row(&d.name, &format!("{}, up to date", d.version), None));
                }
            }
        }
        let other: Vec<&String> = s.status.notes.iter().filter(|n| !n.starts_with("Firmware:")).collect();
        if !other.is_empty() {
            notes.set_text(&other.iter().map(|n| n.as_str()).collect::<Vec<_>>().join("\n"));
            notes.set_visible(true);
        }
    });

    settings(page);
    history(page);
    going_back(page);
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
        (Some(a), None) => format!("{a}, up to date"),
        (None, None) => "Checked as it updates".into(),
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

/// Going back: the system's generations, and undoing the last change to your own apps.
fn going_back(page: &Page) {
    let g = page.group("Going back");
    let list = super::generations();
    g.note("A system update makes a new generation; going back undoes the last one, its software and system settings with it, and your files stay. Earlier ones are also in the boot menu. Your own apps have their own history. Firmware can't be undone.");
    let previous = list.iter().find(|x| !x.current && list.iter().any(|c| c.current && c.number > x.number)).map(|x| x.number);
    for g8 in list.iter().take(8) {
        let r = widgets::row(&format!("Generation {}", g8.number), &date(g8.created), None);
        if g8.current {
            widgets::tag_row(&r, "Current");
        }
        g.add(&r);
    }
    if let Some(n) = previous {
        let back = widgets::confirm_button("Go Back", &format!("Go back to generation {n}?"), |b| {
            b.set_sensitive(false);
            super::rollback(|_| window::rebuild("updates"));
        });
        back.set_sensitive(!busy());
        g.add(&widgets::row(
            &format!("Go back to generation {n}"),
            "Switches to it now; it stays until the next change.",
            Some(back.upcast_ref()),
        ));
    }
    let undo = widgets::confirm_button("Undo", "Undo your last app change?", |b| {
        b.set_sensitive(false);
        cmd::run_async(&["nix", "profile", "rollback"], |r| {
            window::toast(match r {
                Ok(_) => "Your apps are back as they were before the last change.",
                Err(_) => "Your apps couldn't be put back. Run nyx rollback in a terminal to see why.",
            });
            window::rebuild("updates");
        });
    });
    g.add(&widgets::row(
        "Your apps and tools",
        "Undo the last install, removal or update of your own apps (your Nix profile).",
        Some(undo.upcast_ref()),
    ));
    widgets::keywords("rollback undo previous generation revert recovery");
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
