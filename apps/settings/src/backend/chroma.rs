//! App-wide theming through hyprchroma (the engine behind Omarchroma).
//!
//! hyprchroma themes GTK 3/4 and libadwaita (`gtk.css` + `hyprchroma.css`),
//! KDE/Qt (`kdeglobals`), and optionally Dark Reader and Pear Desktop, and keeps
//! them in step with Omarchy's theme through its own service and hooks. Settings
//! only drives its command line; it never writes those files itself.

use crate::{cmd, paths};
use anyhow::Result;
use serde_json::Value;

pub const SERVICE: &str = "hyprchromad.service";
pub const PLUGIN: &str = "io.github.nobledoodle.omarchroma";

/// One thing hyprchroma can theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    /// `--target=` name on the command line.
    pub cli: &'static str,
    /// Key in its settings.json / status.json.
    pub key: &'static str,
    pub title: &'static str,
    pub desc: &'static str,
}

pub const TARGETS: &[Target] = &[
    Target { cli: "gtk", key: "gtk", title: "GTK & GNOME apps", desc: "GTK 3 and 4 and libadwaita apps, and file choosers." },
    Target {
        cli: "qt-kde",
        key: "qtKde",
        title: "Qt & KDE apps",
        desc: "KDE apps and Qt apps using KDE color schemes. Other Qt apps follow GTK already.",
    },
    Target {
        cli: "dark-reader",
        key: "darkReader",
        title: "Web pages (Dark Reader)",
        desc: "Recolors websites in the theme. Needs the Dark Reader browser extension.",
    },
    Target { cli: "pear", key: "pear", title: "YouTube Music (Pear Desktop)", desc: "Themes the Pear Desktop app." },
];

pub fn target(key: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|t| t.key == key || t.cli == key)
}

pub fn installed() -> bool {
    cmd::present("hyprchroma")
}

pub fn service_active() -> bool {
    cmd::output(&["systemctl", "--user", "is-active", SERVICE]).is_some_and(|s| s.trim() == "active")
}

pub fn plugin_installed() -> bool {
    paths::omarchy_config().join("plugins").join(PLUGIN).is_dir()
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    /// (key, enabled) for frameworks that haven't been removed, in [`TARGETS`] order.
    pub frameworks: Vec<(String, bool)>,
    /// Keys of frameworks hidden with `framework remove`.
    pub removed: Vec<String>,
    pub theme: String,
    /// RFC 3339 time of the last sync.
    pub last_sync: String,
    /// key -> status word, e.g. "synchronized", "not-installed".
    pub status: Vec<(String, String)>,
}

/// Parse hyprchroma's settings.json and status.json.
pub fn parse_state(settings: &str, status: &str) -> State {
    let s: Value = serde_json::from_str(settings).unwrap_or(Value::Null);
    let st: Value = serde_json::from_str(status).unwrap_or(Value::Null);
    let removed: Vec<String> = s
        .get("removed")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let frameworks = TARGETS
        .iter()
        .filter(|t| !removed.iter().any(|r| r == t.key))
        .map(|t| {
            (
                t.key.to_string(),
                s.get("frameworks")
                    .and_then(|f| f.get(t.key))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(t.key == "gtk" || t.key == "qtKde"),
            )
        })
        .collect();
    // status.json calls Pear "pearDesktop".
    let status = TARGETS
        .iter()
        .filter_map(|t| {
            let key = if t.key == "pear" { "pearDesktop" } else { t.key };
            st.get(key).and_then(|v| v.as_str()).map(|v| (t.key.to_string(), v.to_string()))
        })
        .collect();
    State {
        frameworks,
        removed,
        theme: st.get("theme").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        last_sync: st.get("lastSync").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        status,
    }
}

pub fn state() -> State {
    let dir = paths::state_home().join("hyprchroma");
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap_or_default();
    parse_state(&read("settings.json"), &read("status.json"))
}

/// "2 min ago" for an RFC 3339 timestamp, using `date` to parse it.
pub fn ago(rfc3339: &str) -> String {
    let Some(then) = cmd::output(&["date", "-d", rfc3339, "+%s"]).and_then(|s| s.trim().parse::<i64>().ok()) else {
        return String::new();
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(then);
    crate::units::ago(now - then)
}

fn run(args: &[&str]) -> Result<String> {
    let mut full = vec!["hyprchroma"];
    full.extend_from_slice(args);
    cmd::run(&full)
}

pub fn set_enabled(t: &Target, on: bool) -> Result<String> {
    run(&[&format!("--target={}", t.cli), &format!("--set-enabled={on}")])
}

pub fn sync_now() -> Result<String> {
    run(&["--force", "--quiet"])
}

pub fn restore_framework(t: &Target) -> Result<String> {
    run(&["framework", "restore", t.cli])
}

pub fn restore_stock() -> Result<String> {
    run(&["restore", "--stock"])
}

/// Apps still showing the previous theme.
pub fn stale_apps() -> Vec<String> {
    run(&["stale-apps"])
        .map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

pub fn palette_source() -> String {
    run(&["palette", "--source"]).map(|s| s.trim().to_string()).unwrap_or_default()
}

pub fn set_service(on: bool) -> Result<String> {
    cmd::run(&["systemctl", "--user", if on { "enable" } else { "disable" }, "--now", SERVICE])
}

#[cfg(test)]
mod tests {
    use super::*;

    // As found on the development machine.
    const SETTINGS: &str = r#"{ "frameworks": { "gtk": true, "qtKde": true, "darkReader": false, "pear": false }, "removed": ["darkReader", "pear"] }"#;
    const STATUS: &str = r#"{ "theme": "Catppuccin", "fingerprint": "0e", "lastSync": "2026-09-29T03:38:56.870585+00:00",
        "gtk": "synchronized", "qtKde": "synchronized", "pearDesktop": "not-installed", "darkReader": "synchronized" }"#;

    #[test]
    fn parses_state() {
        let s = parse_state(SETTINGS, STATUS);
        assert_eq!(s.frameworks, vec![("gtk".to_string(), true), ("qtKde".to_string(), true)]);
        assert_eq!(s.removed, ["darkReader", "pear"]);
        assert_eq!(s.theme, "Catppuccin");
        assert!(s.last_sync.starts_with("2026-09-29"));
        assert!(s.status.contains(&("pear".to_string(), "not-installed".to_string())));
        assert!(s.status.contains(&("qtKde".to_string(), "synchronized".to_string())));
    }

    #[test]
    fn missing_files_mean_defaults() {
        let s = parse_state("", "");
        assert_eq!(s.frameworks.len(), 4);
        assert!(s.frameworks.iter().any(|(k, on)| k == "gtk" && *on));
        assert!(s.frameworks.iter().any(|(k, on)| k == "darkReader" && !*on));
        assert!(s.removed.is_empty() && s.theme.is_empty());
    }

    #[test]
    fn maps_keys_and_cli_names() {
        assert_eq!(target("qtKde").unwrap().cli, "qt-kde");
        assert_eq!(target("dark-reader").unwrap().key, "darkReader");
        assert!(target("nope").is_none());
    }
}
