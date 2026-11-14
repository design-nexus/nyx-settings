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

// ----- Icon sets: Umbra and Vellum -----
//
// An icon set comes as one icon theme per look (Umbra, Umbra-Solid…; Vellum,
// Vellum-Mono), drawn in the theme's colours by its own command (`umbra-icons`,
// `vellum-icons`). Settings shows each set as one choice with a Look row. `nyx icons`
// draws app icons in the style of the look in use.

/// A look: (id, label, icon theme directory).
pub type Look = (&'static str, &'static str, &'static str);

pub struct IconSet {
    /// What Settings stores for the set, whichever look: its first look's directory.
    pub id: &'static str,
    pub command: &'static str,
    /// Its folder in ~/.config, where `settings.json` has the look in use.
    pub config: &'static str,
    pub looks: &'static [Look],
    pub about: &'static str,
}

pub const UMBRA: &str = "Umbra";
pub const VELLUM: &str = "Vellum";
pub static SETS: [IconSet; 2] = [
    IconSet {
        id: UMBRA,
        command: "umbra-icons",
        config: "umbra",
        looks: &[
            ("tile", "Tile", "Umbra"),
            ("solid", "Solid", "Umbra-Solid"),
            ("line", "Line", "Umbra-Line"),
            ("tinted", "Tinted", "Umbra-Tinted"),
            ("frame", "Frame", "Umbra-Frame"),
            ("colour", "Colour", "Umbra-Colour"),
        ],
        about: "Umbra's folders and files: satin tiles, solid shapes, coloured lines, tinted tiles, outlined frames or colour tiles.",
    },
    IconSet {
        id: VELLUM,
        command: "vellum-icons",
        config: "vellum",
        looks: &[("paper", "Paper", "Vellum"), ("mono", "Mono", "Vellum-Mono")],
        about: "Vellum's folders and files: pastel paper, or the theme's greys with colour only on the mark.",
    },
];

impl IconSet {
    pub fn installed(&self) -> bool {
        cmd::present(self.command)
    }

    /// The look in use, if the set is.
    pub fn look(&self) -> Option<String> {
        let text = std::fs::read_to_string(paths::config_home().join(self.config).join("settings.json")).ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        v.get("look")?.as_str().filter(|l| self.looks.iter().any(|(id, _, _)| id == l)).map(str::to_string)
    }

    /// The look Settings shows: the one in use, else the first.
    pub fn look_or_default(&self) -> String {
        self.look().unwrap_or_else(|| self.looks[0].0.to_string())
    }

    pub fn owns(&self, theme: &str) -> bool {
        self.looks.iter().any(|(_, _, dir)| *dir == theme)
    }

    fn dir(&self, look: &str) -> &'static str {
        self.looks.iter().find(|(id, _, _)| *id == look).map_or(self.id, |(_, _, dir)| dir)
    }

    /// Use a look: the set's command builds it if needed and switches to it.
    pub fn use_look(&self, look: &str) -> anyhow::Result<()> {
        cmd::run(&[self.command, "look", look])?;
        Ok(())
    }

    /// Another icon theme was chosen: the set stops putting itself back after theme changes.
    pub fn leave(&self) {
        if self.look().is_some() {
            let _ = cmd::run(&[self.command, "look", "off"]);
        }
    }
}

/// The set an icon theme directory or stored id belongs to.
pub fn set_of(theme: &str) -> Option<&'static IconSet> {
    SETS.iter().find(|s| s.owns(theme))
}

/// Every set but `keep` stops putting itself back.
pub fn leave_sets(keep: Option<&str>) {
    for s in SETS.iter().filter(|s| Some(s.id) != keep) {
        s.leave();
    }
}

/// The icon theme Settings shows as chosen: the override (any look as its set), or the
/// set in use, or None to follow the Omarchy theme.
pub fn chosen(c: &Config) -> Option<String> {
    match &c.icon_theme {
        Some(t) => Some(set_of(t).map_or_else(|| t.clone(), |s| s.id.to_string())),
        None => SETS.iter().find(|s| s.look().is_some()).map(|s| s.id.to_string()),
    }
}

/// Apply what the config says: the override, or the Omarchy theme's icons.
pub fn apply_icons(c: &Config) -> anyhow::Result<()> {
    let in_use = |s: &IconSet| s.look().map(|l| s.dir(&l).to_string());
    let want = match &c.icon_theme {
        // A set's look lives with the set.
        Some(t) => match set_of(t) {
            Some(s) => in_use(s).or_else(|| Some(t.clone())),
            None => Some(t.clone()),
        },
        None => SETS.iter().find_map(in_use).or_else(omarchy_icon_theme),
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

/// Installed icon themes, sorted, without duplicates. Each set's looks are one entry.
pub fn icon_themes() -> Vec<String> {
    one_per_set(all_icon_themes())
}

fn one_per_set(all: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = all.iter().filter(|t| set_of(t).is_none_or(|s| s.id == *t)).cloned().collect();
    for s in &SETS {
        if !out.iter().any(|t| t == s.id) && all.iter().any(|t| s.owns(t)) {
            out.push(s.id.into());
        }
    }
    out.sort_by_key(|n| n.to_lowercase());
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

/// Nyx's interface and document fonts, which Restore puts back.
pub const DEFAULT_INTERFACE_FONT: &str = "Inter 11";
pub const DEFAULT_DOCUMENT_FONT: &str = "Inter 12";

pub fn interface_font() -> String {
    gsettings_get("font-name").unwrap_or_else(|| DEFAULT_INTERFACE_FONT.into())
}

pub fn set_interface_font(desc: &str) -> anyhow::Result<()> {
    gsettings_set("font-name", desc)
}

pub fn restore_fonts() -> anyhow::Result<()> {
    gsettings_set("font-name", DEFAULT_INTERFACE_FONT)?;
    gsettings_set("document-font-name", DEFAULT_DOCUMENT_FONT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_set_is_one_choice() {
        let umbra = set_of("Umbra-Colour").unwrap();
        assert_eq!(umbra.id, UMBRA);
        assert!(["Umbra", "Umbra-Solid", "Umbra-Line", "Umbra-Tinted", "Umbra-Frame"].iter().all(|t| umbra.owns(t)));
        assert_eq!(set_of("Vellum-Mono").map(|s| s.id), Some(VELLUM));
        assert!(set_of("Umbra-Other").is_none() && set_of("Yaru").is_none());
        assert_eq!(umbra.dir("line"), "Umbra-Line");
        assert_eq!(umbra.dir("nonsense"), "Umbra");
        let solid = Config { icon_theme: Some("Umbra-Solid".into()) };
        assert_eq!(chosen(&solid).as_deref(), Some(UMBRA));
        let mono = Config { icon_theme: Some("Vellum-Mono".into()) };
        assert_eq!(chosen(&mono).as_deref(), Some(VELLUM));
        let yaru = Config { icon_theme: Some("Yaru".into()) };
        assert_eq!(chosen(&yaru).as_deref(), Some("Yaru"));
    }

    #[test]
    fn icon_list_collapses_each_sets_looks() {
        let all = ["Adwaita", "Umbra-Frame", "Umbra", "Umbra-Solid", "Vellum-Mono", "Yaru"].map(String::from).to_vec();
        assert_eq!(one_per_set(all), ["Adwaita", "Umbra", "Vellum", "Yaru"]);
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
