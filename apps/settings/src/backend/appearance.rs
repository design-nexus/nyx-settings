//! Desktop-wide look outside the colour palette: the icon theme override and the
//! interface font. Both are GNOME settings that GTK uses directly and Qt picks
//! up through its gtk3 platform theme.

use crate::{cmd, paths};
use gtk::gio;
use gtk::gio::prelude::*;
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

/// The interface settings through GIO, not the `gsettings` command (NyxOS doesn't ship
/// it). None when the schema isn't installed, where `Settings::new` would abort.
fn interface() -> Option<gio::Settings> {
    gio::SettingsSchemaSource::default()?.lookup(IFACE, true)?;
    Some(gio::Settings::new(IFACE))
}

fn gsettings_get(key: &str) -> Option<String> {
    Some(interface()?.string(key).to_string())
}

fn gsettings_set(key: &str, value: &str) -> anyhow::Result<()> {
    let settings = interface().ok_or_else(|| anyhow::anyhow!("the {IFACE} schema isn't installed"))?;
    settings.set_string(key, value)?;
    gio::Settings::sync();
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

// ----- Umbra -----
//
// Umbra comes as three icon themes, one per look (Umbra, Umbra-Solid, Umbra-Line), drawn
// in the theme's colours by `umbra-icons`. Settings shows it as one choice with a Look.

/// The id Settings stores for Umbra, whichever look.
pub const UMBRA: &str = "Umbra";
/// (look, label, icon theme directory)
pub const UMBRA_LOOKS: [(&str, &str, &str); 3] =
    [("tile", "Tile", "Umbra"), ("solid", "Solid", "Umbra-Solid"), ("line", "Line", "Umbra-Line")];

pub fn umbra_installed() -> bool {
    cmd::present("umbra-icons")
}

/// The Umbra look in use, if Umbra is.
pub fn umbra_look() -> Option<String> {
    let text = std::fs::read_to_string(paths::config_home().join("umbra/settings.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("look")?.as_str().filter(|l| UMBRA_LOOKS.iter().any(|(id, _, _)| id == l)).map(str::to_string)
}

/// Whether an icon theme directory is one of Umbra's looks.
pub fn is_umbra(theme: &str) -> bool {
    UMBRA_LOOKS.iter().any(|(_, _, dir)| *dir == theme)
}

fn umbra_dir(look: &str) -> &'static str {
    UMBRA_LOOKS.iter().find(|(id, _, _)| *id == look).map_or("Umbra", |(_, _, dir)| dir)
}

/// The icon theme Settings shows as chosen: the override (any Umbra look as Umbra), or
/// Umbra when it's in use, or None to follow the Omarchy theme.
pub fn chosen(c: &Config) -> Option<String> {
    match &c.icon_theme {
        Some(t) if is_umbra(t) => Some(UMBRA.into()),
        Some(t) => Some(t.clone()),
        None => umbra_look().map(|_| UMBRA.into()),
    }
}

/// Use an Umbra look: `umbra-icons` builds it if needed and switches to it.
pub fn use_umbra(look: &str) -> anyhow::Result<()> {
    cmd::run(&["umbra-icons", "look", look])?;
    Ok(())
}

/// Another icon theme was chosen: Umbra stops putting itself back after theme changes.
pub fn leave_umbra() {
    if umbra_look().is_some() {
        let _ = cmd::run(&["umbra-icons", "look", "off"]);
    }
}

/// Apply what the config says: the override, or the Omarchy theme's icons.
pub fn apply_icons(c: &Config) -> anyhow::Result<()> {
    let umbra = umbra_look().map(|l| umbra_dir(&l).to_string());
    let want = match &c.icon_theme {
        // Umbra's look lives with Umbra.
        Some(t) if is_umbra(t) => umbra.or_else(|| Some(t.clone())),
        Some(t) => Some(t.clone()),
        None => umbra.or_else(omarchy_icon_theme),
    };
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

/// Installed icon themes, sorted, without duplicates. Umbra's looks are one entry.
pub fn icon_themes() -> Vec<String> {
    let mut out: Vec<String> = all_icon_themes().into_iter().filter(|t| !is_umbra(t) || t == UMBRA).collect();
    if !out.iter().any(|t| t == UMBRA) && all_icon_themes().iter().any(|t| is_umbra(t)) {
        out.push(UMBRA.into());
        out.sort_by_key(|n| n.to_lowercase());
    }
    out
}

fn all_icon_themes() -> Vec<String> {
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
    gsettings_get("font-name").unwrap_or_else(|| "Inter 11".into())
}

pub fn set_interface_font(desc: &str) -> anyhow::Result<()> {
    gsettings_set("font-name", desc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn umbra_is_one_choice() {
        assert!(is_umbra("Umbra") && is_umbra("Umbra-Solid") && is_umbra("Umbra-Line"));
        assert!(!is_umbra("Umbra-Other") && !is_umbra("Yaru"));
        assert_eq!(umbra_dir("line"), "Umbra-Line");
        assert_eq!(umbra_dir("nonsense"), "Umbra");
        let solid = Config { icon_theme: Some("Umbra-Solid".into()) };
        assert_eq!(chosen(&solid).as_deref(), Some(UMBRA));
        let yaru = Config { icon_theme: Some("Yaru".into()) };
        assert_eq!(chosen(&yaru).as_deref(), Some("Yaru"));
    }

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
