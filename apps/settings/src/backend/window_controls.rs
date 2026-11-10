//! Adaptive window controls: `[window_controls]` in the `shell` settings, and whether
//! NyxOS's decorations (the nyx-decor Hyprland plugin) run.
//!
//! Nyx apps read `[window_controls]` from the settings layers themselves, also where
//! Shell Core doesn't run (Omarchy). Where it runs it owns the settings and Settings
//! goes through it; elsewhere Settings writes the user's layer, which is all Shell Core
//! would do.

use super::shell_core;
use serde_json::{Value, json};

/// The plugin as Hyprland lists it.
const PLUGIN: &str = "nyx-decor";

/// Whether nyx-decor is loaded in this Hyprland (asked once per run).
pub fn decorations() -> bool {
    thread_local! {
        static LOADED: std::cell::OnceCell<bool> = const { std::cell::OnceCell::new() };
    }
    LOADED.with(|l| *l.get_or_init(|| super::hypr::json(&["plugin", "list"]).is_some_and(|v| lists(&v, PLUGIN))))
}

fn lists(plugins: &Value, name: &str) -> bool {
    plugins.as_array().is_some_and(|a| a.iter().any(|p| p.get("name").and_then(Value::as_str) == Some(name)))
}

/// The merged `[window_controls]` (empty when nothing is set).
pub fn settings() -> Value {
    if shell_core::available()
        && let Ok((config, _)) = shell_core::config()
    {
        return config.get("window_controls").cloned().unwrap_or_else(|| json!({}));
    }
    nyx_config::Layers::standard()
        .merged("shell")
        .ok()
        .and_then(|t| t.get("window_controls").cloned())
        .and_then(|v| serde_json::to_value(v).ok())
        .unwrap_or_else(|| json!({}))
}

pub fn enabled(s: &Value) -> bool {
    s.get("enabled").and_then(Value::as_bool).unwrap_or(true)
}

pub fn hover_delay_ms(s: &Value) -> u64 {
    s.get("hover_delay_ms").and_then(Value::as_u64).unwrap_or(200)
}

/// Per app: `{ controls?, float? }`, by app id or class.
pub fn apps(s: &Value) -> serde_json::Map<String, Value> {
    s.get("apps").and_then(Value::as_object).cloned().unwrap_or_default()
}

/// Set `window_controls.KEY`.
pub fn set(key: &str, value: Value) -> Result<(), String> {
    let full = format!("window_controls.{key}");
    if shell_core::available() {
        return shell_core::set(&full, value);
    }
    let value = toml::Value::try_from(value).map_err(|e| e.to_string())?;
    nyx_config::Layers::standard().set(nyx_config::Layer::User, "shell", &full, value).map_err(|e| format!("{e:#}"))
}

/// Set the per-app rules, then bring Hyprland's copy of them up to date.
pub fn set_apps(apps: serde_json::Map<String, Value>) -> Result<(), String> {
    set("apps", Value::Object(apps))?;
    apply_rules();
    Ok(())
}

/// `nyx-shellctl window-controls --apply`: the per-app rules as Hyprland window rules
/// (it reloads Hyprland only when they changed).
pub fn apply_rules() {
    if crate::capability::program("nyx-shellctl") {
        crate::cmd::spawn(&["nyx-shellctl", "window-controls", "--apply"]);
    }
}

/// The key a desktop app's windows go by: its `StartupWMClass`, else its desktop id
/// (Wayland apps name themselves after it). Rules match it whatever its case.
pub fn app_key(desktop_id: &str) -> String {
    let file = gtk::glib::KeyFile::new();
    file.load_from_data_dirs(format!("applications/{desktop_id}.desktop"), gtk::glib::KeyFileFlags::NONE)
        .ok()
        .and_then(|_| file.string("Desktop Entry", "StartupWMClass").ok())
        .map(|c| c.to_string())
        .filter(|c| valid_app_id(c))
        .unwrap_or_else(|| desktop_id.to_string())
}

/// What Shell Core accepts as an app key: 1 to 128 of `[A-Za-z0-9._+-]`.
pub fn valid_app_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_list() {
        let v = json!([{ "name": "nyx-dock-swipe" }, { "name": "nyx-decor", "version": "0.1" }]);
        assert!(lists(&v, "nyx-decor"));
        assert!(!lists(&json!([]), "nyx-decor"));
        assert!(!lists(&json!("no plugins loaded"), "nyx-decor"));
    }

    #[test]
    fn defaults_and_rules() {
        let empty = json!({});
        assert!(enabled(&empty));
        assert_eq!(hover_delay_ms(&empty), 200);
        assert!(apps(&empty).is_empty());
        let s = json!({ "enabled": false, "hover_delay_ms": 350, "apps": { "foot": { "controls": "custom-headerbar" } } });
        assert!(!enabled(&s));
        assert_eq!(hover_delay_ms(&s), 350);
        assert_eq!(apps(&s)["foot"]["controls"], "custom-headerbar");
        assert!(valid_app_id("org.kde.dolphin") && !valid_app_id("Visual Studio Code"));
    }
}
