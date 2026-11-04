//! NyxOS: Settings on the NixOS-based NyxOS, where the Nyx System Manager makes system
//! changes. They're declarative: a change is built into a new system generation and
//! switched to only when it builds, so pages collect what changed and apply it together.
//! Settings asks the manager through `nyx-system`, never runs privileged commands itself.

use crate::{cmd, window};
use nyx_system_proto::{Change, SystemSettings};
use std::cell::Cell;
use std::path::Path;

const PROFILES: &str = "/nix/var/nix/profiles";

/// Whether this is NyxOS (NixOS with the Nyx System Manager).
pub fn active() -> bool {
    Path::new("/etc/NIXOS").exists() && Path::new("/etc/nyxos/system.toml").exists()
}

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

/// Mark a row as a system setting, applied with a rebuild.
pub fn tag_system(row: &gtk::Box) {
    crate::widgets::tag_row(row, "System");
}
