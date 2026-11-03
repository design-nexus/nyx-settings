//! Everything Settings has changed, persisted in `~/.config/settings/state.json`.
//! The managed Lua file is always regenerated from this, never parsed back.

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Monitor {
    pub mode: Option<String>,
    pub position: Option<String>,
    pub scale: Option<f64>,
    pub transform: Option<i64>,
    pub disabled: bool,
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

impl State {
    pub fn load(path: &std::path::Path) -> Self {
        let mut state: Self = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        state.rename_old_programs();
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
