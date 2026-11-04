//! NyxOS: Settings on the NixOS-based NyxOS, where the Nyx System Manager makes system
//! changes. They're declarative: a change is built into a new system generation and
//! switched to only when it builds, so pages collect what changed and apply it together.
//! Settings asks the manager through `nyx-system`, never runs privileged commands itself.

pub mod accounts;
pub mod layout;
pub mod pages;
pub mod printers;
pub mod users;

use crate::sections::home::Hero;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use nyx_system_proto::{Change, SystemSettings};
use std::cell::Cell;
use std::path::Path;

const PROFILES: &str = "/nix/var/nix/profiles";

/// The System Manager's settings (empty when it can't be reached).
pub fn settings() -> SystemSettings {
    cmd::output(&["nyx-system", "settings"]).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

thread_local! {
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

pub fn busy() -> bool {
    BUSY.with(Cell::get)
}

/// Run `nyx-system ARGS`: a toast while it builds, another when it's done, then `then`
/// with whether it worked.
fn run(args: Vec<String>, working: &str, done: &'static str, then: impl FnOnce(bool) + 'static) {
    if busy() {
        window::toast("Another system change is still running.");
        return;
    }
    BUSY.with(|b| b.set(true));
    window::toast(working);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    cmd::run_async(&refs, move |r| {
        BUSY.with(|b| b.set(false));
        match &r {
            Ok(_) => window::toast(done),
            Err(e) => {
                let text = format!("{e:#}");
                let last = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(&text);
                window::toast(last.trim().trim_start_matches("nyx-system: "));
            }
        }
        then(r.is_ok());
    });
}

/// Apply system changes in one rebuild.
pub fn apply(changes: Vec<Change>, done: &'static str, then: impl FnOnce(bool) + 'static) {
    if changes.is_empty() {
        return;
    }
    let json = serde_json::to_string(&changes).unwrap_or_default();
    run(
        vec!["nyx-system".into(), "apply".into(), json],
        "Building the new system. It's switched to when the build works.",
        done,
        then,
    );
}

pub fn update(then: impl FnOnce(bool) + 'static) {
    run(vec!["nyx-system".into(), "update".into()], "Updating NyxOS. This can take a while.", "NyxOS is up to date", then);
}

pub fn rollback(then: impl FnOnce(bool) + 'static) {
    run(
        vec!["nyx-system".into(), "rollback".into()],
        "Going back to the previous generation…",
        "Back to the previous generation",
        then,
    );
}

/// Remove generations older than `days` and what nothing needs any more.
pub fn clean(days: u32, then: impl FnOnce(bool) + 'static) {
    run(
        vec!["nyx-system".into(), "clean".into(), days.to_string()],
        "Removing old generations and unused packages…",
        "Cleaned up",
        then,
    );
}

/// Shrink the system log to `megabytes`.
pub fn clean_logs(megabytes: u32, then: impl FnOnce(bool) + 'static) {
    run(vec!["nyx-system".into(), "clean-logs".into(), megabytes.to_string()], "Removing old logs…", "Old logs removed", then);
}

/// One system generation.
pub struct Generation {
    pub number: u32,
    /// Seconds since the epoch.
    pub created: i64,
    pub current: bool,
}

/// The system's generations, newest first.
pub fn generations() -> Vec<Generation> {
    let current: Option<u32> = std::fs::read_link(Path::new(PROFILES).join("system"))
        .ok()
        .and_then(|l| l.to_string_lossy().strip_prefix("system-")?.strip_suffix("-link")?.parse().ok());
    let mut list: Vec<Generation> = std::fs::read_dir(PROFILES)
        .map(|d| {
            d.flatten()
                .filter_map(|e| {
                    let number: u32 =
                        e.file_name().to_string_lossy().strip_prefix("system-")?.strip_suffix("-link")?.parse().ok()?;
                    let created =
                        e.path().symlink_metadata().ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs()
                            as i64;
                    Some(Generation { number, created, current: Some(number) == current })
                })
                .collect()
        })
        .unwrap_or_default();
    list.sort_by_key(|g| std::cmp::Reverse(g.number));
    list
}

/// `NAME VERSION` from os-release.
pub fn version() -> String {
    let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let field = |k: &str| {
        text.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).map(|v| v.trim_matches('"').to_string()).unwrap_or_default()
    };
    let name = field("NAME");
    let version = field("VERSION");
    format!("{} {}", if name.is_empty() { "NyxOS".into() } else { name }, version).trim().to_string()
}

/// Turn one of the System Manager's services on or off.
pub fn set_service(name: &str, on: bool, done: &'static str, section: &'static str) {
    apply(vec![Change::Service { name: name.into(), enabled: Some(on) }], done, move |_| window::rebuild(section));
}

pub fn set_power(key: &str, action: &str, section: &'static str) {
    let change = Change::Power { key: key.into(), action: Some(action.into()) };
    apply(vec![change], "Saved", move |_| window::rebuild(section));
}

pub fn set_hibernate_delay(secs: &str, section: &'static str) {
    let Ok(seconds) = secs.parse::<u32>() else { return };
    apply(vec![Change::HibernateDelay { seconds: Some(seconds) }], "Saved", move |_| window::rebuild(section));
}

/// Whether the kernel can hibernate and there's swap to hibernate to.
pub fn hibernation_available() -> bool {
    let disk = std::fs::read_to_string("/sys/power/state").is_ok_and(|s| s.split_whitespace().any(|w| w == "disk"));
    let swap = std::fs::read_to_string("/proc/swaps").is_ok_and(|s| s.lines().count() > 1);
    disk && swap
}

/// Mark a row as a system setting, applied with a rebuild.
pub fn tag_system(row: &gtk::Box) {
    widgets::tag_row(row, "System");
}

/// Home: the generation in the badge; updates are on Software & Updates.
pub fn home_updates(page: &Page, hero: &Hero) {
    let current = generations().into_iter().find(|g| g.current).map(|g| g.number);
    hero.count.set_text(&current.map(|n| n.to_string()).unwrap_or_default());
    hero.caption.set_text("Generation");
    hero.badge.set_tooltip_text(Some("Software & Updates"));
    hero.badge.connect_clicked(|_| window::navigate("software"));
    let g = page.group("Updates");
    let (r, _) = widgets::button_row(
        &version(),
        "NyxOS, its apps and the software for everyone update together. Software & Updates has the update and earlier generations.",
        "Open",
        |_| window::navigate("software"),
    );
    g.add(&r);
}

/// About: NyxOS's version and generation; updates and going back are on Software & Updates.
pub fn about(page: &Page) {
    let g = page.group("NyxOS");
    let (r, _) = widgets::info_row("Version", &version());
    g.add(&r);
    if let Some(current) = generations().into_iter().find(|x| x.current) {
        let (r, _) = widgets::info_row("Generation", &current.number.to_string());
        g.add(&r);
    }
    let (r, _) = widgets::button_row(
        "Updates and generations",
        "Update NyxOS, or go back to an earlier generation.",
        "Software & Updates",
        |_| window::navigate("software"),
    );
    widgets::keywords("update upgrade rollback generation version nixos");
    g.add(&r);
}
