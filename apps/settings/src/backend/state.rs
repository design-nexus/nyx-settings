//! Everything Settings has changed, persisted in `~/.config/nyx-settings/state.json`.
//! The managed Lua file is always regenerated from this, never parsed back.
//!
//! Versioned by `schema`: 2 keeps displays by their identity (see [`Monitor`]). A file
//! from before is copied to `state.json.v1.bak` first; its displays kept by port move
//! over as each is seen. A file that can't be read is kept as `state.json.broken-<time>`
//! rather than lost.

use super::gestures::Gestures;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bind {
    pub keys: String,
    pub description: String,
    pub command: String,
}

/// A window rule (matching `class` and/or `title`) or a layer rule (matching
/// `class` as the namespace), with one effect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Rule {
    pub class: String,
    pub title: String,
    pub effect: String,
    pub value: String,
}

/// A display's settings. Kept by the display's own identity (make, model and serial,
/// `nyx_desktop::Monitor::stable_id`) with `desc` naming it to Hyprland, so they follow it
/// to another port and come back when it's plugged in again; or by connector (`DP-1`)
/// for screens that report no identity, two identical ones without a serial, and entries
/// from before Settings knew displays by identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Monitor {
    pub mode: Option<String>,
    pub position: Option<String>,
    pub scale: Option<f64>,
    pub transform: Option<i64>,
    pub disabled: bool,
    /// Hyprland's `desc:` selector for it, when kept by identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    /// What to call it while it's not connected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl Monitor {
    /// The `output` of its Hyprland rule.
    pub fn output<'a>(&'a self, key: &'a str) -> &'a str {
        self.desc.as_deref().unwrap_or(key)
    }
}

/// What Settings sets for one workspace (1–10). Unset fields leave Hyprland's
/// and Omarchy's own behaviour alone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct WorkspaceRule {
    pub workspace: u32,
    /// The monitor it lives on.
    pub monitor: Option<String>,
    /// Its own name, shown instead of the number.
    pub name: Option<String>,
    /// Its own layout: dwindle, master, scrolling or floating (Omarchy's
    /// `o.workspace_mode`).
    pub mode: Option<String>,
    /// Its own gaps, inner and outer.
    pub gaps: Option<(i64, i64)>,
    pub no_border: bool,
    pub no_rounding: bool,
    pub no_shadow: bool,
}

impl WorkspaceRule {
    /// Whether it changes anything.
    pub fn is_empty(&self) -> bool {
        self.monitor.is_none()
            && self.name.is_none()
            && self.mode.is_none()
            && self.gaps.is_none()
            && !self.no_border
            && !self.no_rounding
            && !self.no_shadow
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct State {
    /// The file's format: [`SCHEMA`] when written by this version.
    pub schema: u32,
    /// Hyprland options by dotted path, e.g. `input.touchpad.natural_scroll`.
    pub options: BTreeMap<String, Value>,
    /// Per-device overrides (`hl.device`), keyed by device name.
    pub devices: BTreeMap<String, BTreeMap<String, Value>>,
    pub gestures: Gestures,
    /// Bindings added here.
    pub binds: Vec<Bind>,
    /// Existing bindings turned off here.
    pub unbinds: Vec<String>,
    pub monitors: BTreeMap<String, Monitor>,
    pub env: BTreeMap<String, String>,
    /// Loudest the volume keys may go, in percent. 100 leaves Omarchy's keys alone.
    pub max_volume: Option<u32>,
    /// Programs started with the session if they aren't already running
    /// (process name -> command).
    pub autostart: BTreeMap<String, String>,
    pub window_rules: Vec<Rule>,
    /// Window rules from the active workspace profile (its apps' workspaces); replaced
    /// whenever a profile is switched to.
    pub profile_rules: Vec<Rule>,
    /// Apps that draw their own titlebar, so Omarchy's floating titlebar is left off
    /// (`None`: the built-in list; see `backend::titlebars`).
    pub own_titlebar_apps: Option<Vec<String>>,
    /// On floating workspaces, the Nyx Dock's hover buttons instead of Omarchy's
    /// titlebar (mirrored to `window_controls` in the dock's `dock.toml`).
    pub hover_window_controls: bool,
    pub layer_rules: Vec<Rule>,
    /// Workspaces 1..=N always exist, even when empty (0: Hyprland's default).
    pub persistent_workspaces: u32,
    pub workspaces: Vec<WorkspaceRule>,
}

/// The suite's programs by the names they had before it was called Nixe (and
/// then Nyx), and Tasks by its name before Activity. Any other `nixe-<name>` is
/// now `nyx-<name>`.
const OLD_PROGRAMS: &[(&str, &str)] = &[
    ("archive", "nyx-archive"),
    ("chat", "nyx-chat"),
    ("fonts", "nyx-fonts"),
    ("media-player", "nyx-media-player"),
    ("music", "nyx-music"),
    ("notepad", "nyx-notepad"),
    ("pdf", "nyx-pdf"),
    ("settings", "nyx-settings"),
    ("tasks", "nyx-activity"),
    ("nixe-tasks", "nyx-activity"),
    ("nyx-tasks", "nyx-activity"),
    ("torrents", "nyx-torrents"),
    ("nexus-photos", "nyx-photos"),
    ("npv", "nyx-viewer"),
    ("nexus-office", "nyx-office"),
    ("nexus-write", "nyx-write"),
    ("nexus-sheets", "nyx-sheets"),
    ("nexus-present", "nyx-present"),
];

/// A command line that starts one of the suite's programs by its old name,
/// with the new name.
fn renamed_command(command: &str) -> Option<String> {
    let trimmed = command.trim_start();
    let (word, rest) = trimmed.split_once(char::is_whitespace).map_or((trimmed, ""), |(w, r)| (w, r));
    let (dir, program) = word.rsplit_once('/').map_or(("", word), |(d, p)| (d, p));
    let nixe = program.strip_prefix("nixe-").map(|name| format!("nyx-{name}"));
    let new = OLD_PROGRAMS.iter().find(|(old, _)| *old == program).map(|(_, new)| new.to_string()).or(nixe)?;
    // A path must be the installer's, not some other program of the same name.
    if !dir.is_empty() && !dir.ends_with("/.local/bin") {
        return None;
    }
    let dir = if dir.is_empty() { String::new() } else { format!("{dir}/") };
    Some(if rest.is_empty() { format!("{dir}{new}") } else { format!("{dir}{new} {rest}") })
}

/// A window class of the suite from before the rename, with the new one.
fn renamed_class(class: &str) -> String {
    let class =
        class.replace("design_nexus.nixe.", "design_nexus.nyx.").replace("design_nexus\\.nixe\\.", "design_nexus\\.nyx\\.");
    let class = class.replace("io.github.nexus.photos.viewer", "io.github.design_nexus.nyx.PhotoViewer");
    let class = class.replace("io.github.nexus.photos", "io.github.design_nexus.nyx.Photos");
    let class = class.replace("io\\.github\\.nexus\\.photos", "io\\.github\\.design_nexus\\.nyx\\.Photos");
    let class = if class.contains("design_nexus.nyx") || class.contains("design_nexus\\.nyx") {
        class
    } else {
        class.replace("design_nexus.", "design_nexus.nyx.").replace("design_nexus\\.", "design_nexus\\.nyx\\.")
    };
    class
        .replace("design_nexus.nyx.Tasks", "design_nexus.nyx.Activity")
        .replace("design_nexus\\.nyx\\.Tasks", "design_nexus\\.nyx\\.Activity")
}

pub const SCHEMA: u32 = 2;

impl State {
    pub fn load(path: &std::path::Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else { return State { schema: SCHEMA, ..Default::default() } };
        let mut state: Self = match serde_json::from_str(&text) {
            Ok(s) => s,
            Err(e) => {
                let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                let kept = path.with_file_name(format!("state.json.broken-{secs}"));
                let _ = std::fs::rename(path, &kept);
                eprintln!("nyx-settings: {} couldn't be read ({e}); kept as {}", path.display(), kept.display());
                return State { schema: SCHEMA, ..Default::default() };
            }
        };
        state.rename_old_programs();
        if state.schema < SCHEMA {
            let backup = path.with_file_name(format!("state.json.v{}.bak", state.schema.max(1)));
            if !backup.exists() {
                let _ = std::fs::copy(path, &backup);
            }
            state.schema = SCHEMA;
        }
        state
    }

    /// Point bindings, autostart entries and window rules at the suite's programs
    /// by their current names.
    fn rename_old_programs(&mut self) {
        for b in &mut self.binds {
            if let Some(c) = renamed_command(&b.command) {
                b.command = c;
            }
        }
        for c in self.autostart.values_mut() {
            if let Some(n) = renamed_command(c) {
                *c = n;
            }
        }
        for r in &mut self.window_rules {
            r.class = renamed_class(&r.class);
        }
    }
}

#[cfg(test)]
mod rename_tests {
    use super::*;

    #[test]
    fn renames_the_suite_programs() {
        assert_eq!(renamed_command("tasks").as_deref(), Some("nyx-activity"));
        assert_eq!(renamed_command("nyx-tasks --toggle").as_deref(), Some("nyx-activity --toggle"));
        assert_eq!(renamed_command("settings --section audio").as_deref(), Some("nyx-settings --section audio"));
        assert_eq!(
            renamed_command("/home/u/.local/bin/notepad --toggle").as_deref(),
            Some("/home/u/.local/bin/nyx-notepad --toggle")
        );
        assert_eq!(renamed_command("/usr/bin/music"), None);
        assert_eq!(renamed_command("taskset -c 1 foo"), None);
        assert_eq!(renamed_command("nyx-notepad"), None);
        assert_eq!(renamed_command("nixe-tasks").as_deref(), Some("nyx-activity"));
        assert_eq!(renamed_command("/home/u/.local/bin/nixe-files /tmp").as_deref(), Some("/home/u/.local/bin/nyx-files /tmp"));
        assert_eq!(
            renamed_class("^io\\.github\\.design_nexus\\.nixe\\.Notepad$"),
            "^io\\.github\\.design_nexus\\.nyx\\.Notepad$"
        );
        assert_eq!(renamed_class("io.github.design_nexus.nixe.Music"), "io.github.design_nexus.nyx.Music");
        assert_eq!(renamed_class("^io\\.github\\.design_nexus\\.Notepad$"), "^io\\.github\\.design_nexus\\.nyx\\.Notepad$");
        assert_eq!(renamed_class("io.github.design_nexus.nyx.Music"), "io.github.design_nexus.nyx.Music");
        assert_eq!(renamed_class("firefox"), "firefox");
        assert_eq!(renamed_class("^io\\.github\\.design_nexus\\.nyx\\.Tasks$"), "^io\\.github\\.design_nexus\\.nyx\\.Activity$");
        assert_eq!(renamed_class("io.github.design_nexus.Tasks"), "io.github.design_nexus.nyx.Activity");
    }
}

#[cfg(test)]
mod schema_tests {
    use super::*;

    /// The shape of the reference laptop's state.json (Handoff 09's capture), before schema 2.
    const KENS: &str = r#"{
  "options": { "input.repeat_delay": 350, "misc.vrr": 1 },
  "devices": {},
  "gestures": { "enabled": true, "loop_workspaces": true, "loop_count": 5 },
  "binds": [
    { "keys": "SUPER + CTRL + Escape", "description": "Task Manager", "command": "nyx-activity" },
    { "keys": "SUPER + ALT + Escape", "description": "Settings", "command": "nyx-settings" }
  ],
  "unbinds": ["SUPER + CTRL + Escape", "SUPER + ALT + Escape"],
  "monitors": { "eDP-2": { "mode": "2560x1600@60.00", "position": "0x0", "scale": 1.6, "transform": 0, "disabled": false } },
  "persistent_workspaces": 5
}"#;

    #[test]
    fn a_file_from_before_is_kept_and_upgraded() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("state.json");
        std::fs::write(&path, KENS).unwrap();
        let s = State::load(&path);
        assert_eq!(s.schema, SCHEMA);
        assert_eq!(s.options["input.repeat_delay"], serde_json::json!(350));
        assert_eq!(s.monitors["eDP-2"].scale, Some(1.6), "kept by port until the display is seen");
        assert_eq!(s.binds.len(), 2);
        assert_eq!(std::fs::read_to_string(d.path().join("state.json.v1.bak")).unwrap(), KENS);
    }

    #[test]
    fn a_broken_file_is_kept_aside() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("state.json");
        std::fs::write(&path, "{ not json").unwrap();
        let s = State::load(&path);
        assert!(s.options.is_empty());
        assert!(!path.exists());
        assert!(std::fs::read_dir(d.path()).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("state.json.broken-")));
    }
}
