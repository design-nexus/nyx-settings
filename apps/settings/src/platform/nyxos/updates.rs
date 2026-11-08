//! Software & Updates on NyxOS: what has updates, by kind (the system, applications,
//! tools, firmware), updating any of them, the update history and the update settings.
//! It's the engine `nyx update` uses, asked through `nyx-system update … --json`, so
//! Settings and the command line plan and update the same way.

use super::{busy, set_busy};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
struct Status {
    #[serde(default)]
    status: Summary,
    #[serde(default)]
    targets: Vec<Target>,
    #[serde(default)]
    firmware: Firmware,
}

#[derive(Debug, Default, Deserialize)]
struct Summary {
    #[serde(default)]
    notes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    name: String,
    category: String,
    provider: String,
    installed: Option<String>,
    available: Option<String>,
    #[serde(default)]
    needs_channel: bool,
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
        window::rebuild("software");
    });
}

/// A row whose description is filled in once the check is back.
struct Live {
    row: gtk::Box,
    desc: Option<gtk::Label>,
    button: gtk::Button,
}

impl Live {
    fn new(title: &str, button: gtk::Button) -> Live {
        button.set_visible(false);
        let row = widgets::row(title, "Checking for updates…", Some(button.upcast_ref()));
        let desc = row.first_child().and_then(|text| text.last_child()).and_then(|l| l.downcast::<gtk::Label>().ok());
        Live { row, desc, button }
    }

    fn show(&self, text: &str, can_update: bool) {
        if let Some(d) = &self.desc {
            d.set_text(text);
        }
        self.button.set_visible(can_update && !busy());
        self.row.set_visible(true);
    }
}

fn change(t: &Target) -> String {
    match (&t.installed, &t.available) {
        (Some(a), Some(b)) => format!("{} {a} → {b}", t.name),
        (None, Some(b)) => format!("{} {b}", t.name),
        _ => t.name.clone(),
    }
}

/// The Updates group: everything at once, then each kind with what it has.
pub fn group(page: &Page) {
    let g = page.group("Updates");
    let (r, b) = widgets::button_row(
        &super::version(),
        "The system, its Nyx apps, coding agents, your Flatpaks and your Nix profile, each the way it's installed. The system part is a new generation, switched to only when it builds. Firmware is updated on its own.",
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

    let system = Live::new("System", gtk::Button::with_label("Update"));
    system.button.connect_clicked(|b| {
        b.set_sensitive(false);
        update(&["system"], "Updating NyxOS. This can take a while.");
    });
    let apps = Live::new("Applications", gtk::Button::with_label("Update"));
    apps.button.connect_clicked(|b| {
        b.set_sensitive(false);
        update(&["apps"], "Updating applications…");
    });
    let tools = Live::new("Tools", gtk::Button::with_label("Update"));
    tools.button.connect_clicked(|b| {
        b.set_sensitive(false);
        update(&["tools"], "Updating the coding agents and tools…");
    });
    let firmware = Live::new(
        "Firmware",
        widgets::confirm_button("Update", "Can't be undone. Update?", |b| {
            b.set_sensitive(false);
            update(&["firmware"], "Updating firmware. Keep the computer on and plugged in.");
        }),
    );
    widgets::keywords("firmware fwupd bios tools coding agents claude codex flatpak");
    for l in [&system, &apps, &tools, &firmware] {
        g.add(&l.row);
    }
    for l in [&apps, &tools, &firmware] {
        l.row.set_visible(false);
    }
    let notes = gtk::Label::new(None);
    notes.add_css_class("group-note");
    notes.set_xalign(0.0);
    notes.set_wrap(true);
    notes.set_visible(false);
    g.top(&notes);

    cmd::run_async(&["nyx-system", "update", "status", "--json"], move |r| {
        let Ok(s) = r.map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Status>(&t).map_err(|e| e.to_string()))
        else {
            system.show("Updates couldn't be checked. Try again later, or run nyx update in a terminal.", true);
            return;
        };
        let of = |category: &str| s.targets.iter().filter(|t| t.category == category).collect::<Vec<_>>();
        // The channel (and so the tools' newest pins) couldn't be asked: not "up to date".
        let unchecked = s.status.notes.iter().any(|n| n.starts_with("The system channel couldn't be checked"));
        let current = if unchecked { "Couldn't be checked right now." } else { "Up to date." };

        let sys = of("system");
        match sys.first() {
            Some(t) if t.available.is_some() => {
                system.show(&format!("{}: NixOS, Nyx apps and apps from nixpkgs.", change(t)), true)
            }
            Some(t) if !unchecked => {
                system.show(&format!("Up to date{}.", t.installed.as_ref().map(|v| format!(" ({v})")).unwrap_or_default()), false)
            }
            _ => system.show(current, false),
        }

        let user_apps: Vec<&Target> =
            of("apps").into_iter().filter(|t| t.provider == "flatpak" || t.provider == "profile").collect();
        if !user_apps.is_empty() {
            let news: Vec<String> = user_apps.iter().filter(|t| t.available.is_some()).map(|t| change(t)).collect();
            let profile = user_apps.iter().any(|t| t.provider == "profile");
            let text = if news.is_empty() {
                if profile {
                    "Flatpaks are up to date; your Nix profile is checked as it updates.".to_string()
                } else {
                    "Up to date.".into()
                }
            } else {
                news.join(", ")
            };
            apps.show(&text, !news.is_empty() || profile);
        }

        let tool_list = of("tools");
        if !tool_list.is_empty() {
            let news: Vec<String> = tool_list
                .iter()
                .filter(|t| t.available.is_some())
                .map(|t| if t.needs_channel { format!("{} (with the system update)", change(t)) } else { change(t) })
                .collect();
            let alone = tool_list.iter().any(|t| t.available.is_some() && !t.needs_channel || t.provider == "profile");
            tools.show(&if news.is_empty() { current.to_string() } else { news.join(", ") }, alone);
        }

        match &s.firmware {
            Firmware::Devices { devices } if !devices.is_empty() => {
                let news: Vec<String> =
                    devices.iter().filter_map(|d| d.update.as_ref().map(|u| format!("{} {} → {u}", d.name, d.version))).collect();
                if news.is_empty() {
                    firmware.show("Up to date.", false);
                } else {
                    firmware.show(&format!("{}. Firmware can't be undone.", news.join(", ")), true);
                }
            }
            Firmware::Unavailable { reason } => firmware.show(reason, false),
            _ => {}
        }

        let other: Vec<&String> = s.status.notes.iter().filter(|n| !n.starts_with("Firmware:")).collect();
        if !other.is_empty() {
            notes.set_text(&other.iter().map(|n| n.as_str()).collect::<Vec<_>>().join("\n"));
            notes.set_visible(true);
        }
    });
}

fn date(secs: i64) -> String {
    gtk::glib::DateTime::from_unix_local(secs)
        .and_then(|d| d.format("%e %b %Y, %H:%M"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// The last few updates, the system's and this user's.
pub fn history(page: &Page) {
    let list: Vec<HistoryEntry> = cmd::read(&["nyx-system", "update", "history", "6", "--json"])
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    if list.is_empty() {
        return;
    }
    let g = page.group("Update history");
    for e in &list {
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
        g.add(&r);
    }
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
    let g = page.group("Checking for updates");
    let rows: [(&'static str, &str, &str, bool); 4] = [
        ("check", "Check daily", "Look for updates once a day, in the background.", true),
        ("notify", "Notify", "A notification when something new is available, once per update.", true),
        (
            "check-on-metered",
            "Check on metered connections",
            "Otherwise the check waits for a connection that isn't metered.",
            false,
        ),
        ("firmware", "Include firmware", "Checks ask fwupd too. Firmware is only ever updated when you choose to.", true),
    ];
    for (key, title, desc, default) in rows {
        let (r, _) = widgets::switch_row(title, desc, setting(key, default), move |on| set_setting(key, on));
        g.add(&r);
    }
    widgets::keywords("automatic updates check notify metered firmware");
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
        assert_eq!(change(&s.targets[0]), "Claude Code 2.1.152 → 2.1.311");
        assert!(matches!(s.firmware, Firmware::Unavailable { .. }));
    }
}
