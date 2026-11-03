//! Desktop-wide look outside the colour palette: the icon theme override and the
//! interface font. Both are GNOME settings that GTK uses directly and Qt picks
//! up through its gtk3 platform theme.

use crate::{cmd, paths};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const IFACE: &str = "org.gnome.desktop.interface";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// A fixed icon theme; None follows the Omarchy theme's own choice.
    pub icon_theme: Option<String>,
}

pub fn config_file() -> PathBuf {
    paths::app_dir().join("appearance.toml")
}

pub fn load() -> Config {
    std::fs::read_to_string(config_file()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(c: &Config) -> anyhow::Result<()> {
    cmd::atomic_write(&config_file(), &toml::to_string_pretty(c)?)
}

fn gsettings_get(key: &str) -> Option<String> {
    cmd::output(&["gsettings", "get", IFACE, key]).map(|s| s.trim().trim_matches('\'').to_string())
}

fn gsettings_set(key: &str, value: &str) -> anyhow::Result<()> {
    cmd::run(&["gsettings", "set", IFACE, key, value])?;
    Ok(())
}

pub fn current_icon_theme() -> String {
    gsettings_get("icon-theme").unwrap_or_default()
}

/// The icon theme the current Omarchy theme asks for.
pub fn omarchy_icon_theme() -> Option<String> {
    std::fs::read_to_string(paths::omarchy_theme_dir().join("icons.theme"))
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Apply what the config says: the override, or the Omarchy theme's icons.
pub fn apply_icons(c: &Config) -> anyhow::Result<()> {
    let want = c.icon_theme.clone().or_else(omarchy_icon_theme);
    if let Some(t) = want
        && current_icon_theme() != t
    {
        gsettings_set("icon-theme", &t)?;
    }
    Ok(())
}

/// Whether a directory is an icon theme (not just cursors or a fallback shell).
pub fn is_icon_theme(dir: &Path) -> bool {
    let Ok(index) = std::fs::read_to_string(dir.join("index.theme")) else { return false };
    let has_dirs = index
        .lines()
        .any(|l| l.trim_start().starts_with("Directories=") && l.split('=').nth(1).is_some_and(|v| !v.trim().is_empty()));
    let hidden = index.lines().any(|l| l.trim() == "Hidden=true");
    has_dirs && !hidden
}

/// Installed icon themes, sorted, without duplicates.
pub fn icon_themes() -> Vec<String> {
    let mut out: Vec<String> =
        [PathBuf::from("/usr/share/icons"), paths::home().join(".local/share/icons"), paths::home().join(".icons")]
            .iter()
            .filter_map(|d| std::fs::read_dir(d).ok())
            .flat_map(|rd| rd.flatten())
            .filter(|e| is_icon_theme(&e.path()))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n != "default" && n != "hicolor" && n != "locolor")
            .collect();
    out.sort_by_key(|n| n.to_lowercase());
    out.dedup();
    out
}

pub fn interface_font() -> String {
    gsettings_get("font-name").unwrap_or_else(|| "Adwaita Sans 11".into())
}

pub fn set_interface_font(desc: &str) -> anyhow::Result<()> {
    gsettings_set("font-name", desc)
}

/// Where GTK apps that draw their own header read their window buttons from.
const WM: &str = "org.gnome.desktop.wm.preferences";
const ALL_BUTTONS: &str = "appmenu:minimize,maximize,close";

/// Whether GTK apps show maximize (and minimize, where it works) beside close.
pub fn gtk_window_buttons() -> bool {
    cmd::output(&["gsettings", "get", WM, "button-layout"]).is_some_and(|s| s.contains("maximize"))
}

/// All three buttons, or GNOME's default of close alone (what Omarchy leaves).
pub fn set_gtk_window_buttons(on: bool) -> anyhow::Result<()> {
    if on {
        cmd::run(&["gsettings", "set", WM, "button-layout", ALL_BUTTONS])?;
    } else {
        cmd::run(&["gsettings", "reset", WM, "button-layout"])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme_dir(name: &str, index: Option<&str>) -> PathBuf {
        let d = std::env::temp_dir().join(format!("settings-icons-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        if let Some(i) = index {
            std::fs::write(d.join("index.theme"), i).unwrap();
        }
        d
    }

    #[test]
    fn recognises_icon_themes() {
        let icons = theme_dir("icons", Some("[Icon Theme]\nName=X\nDirectories=16x16/apps,scalable/apps\n"));
        let cursors = theme_dir("cursors", Some("[Icon Theme]\nName=Y\nInherits=Adwaita\n"));
        let hidden = theme_dir("hidden", Some("[Icon Theme]\nDirectories=a\nHidden=true\n"));
        let none = theme_dir("none", None);
        assert!(is_icon_theme(&icons));
        assert!(!is_icon_theme(&cursors));
        assert!(!is_icon_theme(&hidden));
        assert!(!is_icon_theme(&none));
        for d in [icons, cursors, hidden, none] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn config_round_trips() {
        let c = Config { icon_theme: Some("Yaru-blue".into()) };
        assert_eq!(toml::from_str::<Config>(&toml::to_string_pretty(&c).unwrap()).unwrap(), c);
        assert_eq!(toml::from_str::<Config>("").unwrap(), Config::default());
    }
}
