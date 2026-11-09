//! This app's own preferences (`~/.config/nyx-settings/settings.toml`), versioned by
//! `schema`. One that can't be read is kept as `settings.toml.broken-<time>`.

pub use nyx_theme::ThemeMode;

use crate::{cmd, paths};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum TempUnit {
    #[default]
    Celsius,
    Fahrenheit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// The file's format.
    pub schema: u32,
    pub mode: ThemeMode,
    pub theme: String,
    pub density: String,
    pub reduce_motion: bool,
    /// The sidebar shows only icons, whatever the window width.
    pub sidebar_collapsed: bool,
    pub glow: bool,
    /// How temperatures are shown (they are always stored in Celsius).
    pub temp_unit: TempUnit,
    pub last_section: String,
    /// Where Settings opened before the suite-wide "Open on" (`home`, `last`);
    /// `suite` once carried over to `~/.config/nyx/theme.toml`.
    pub open_on: String,
    /// Installed extensions that are turned off: kept, but their pages are hidden.
    pub disabled_extensions: Vec<String>,
    /// Install extension updates found by the daily check without asking.
    pub auto_update_extensions: bool,
    /// The newest Settings version already announced, so it's announced once.
    pub announced_version: String,
    /// Collapsible groups the user opened or closed, by `section/title`.
    pub open_groups: std::collections::BTreeMap<String, bool>,
    /// Disks, mounts and cleanup don't ask for a password (a polkit rule the
    /// helper wrote). Kept here because the rule's folder is root's to read.
    pub password_free: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            mode: ThemeMode::Omarchy,
            theme: nyx_theme::DEFAULT_THEME.into(),
            density: "comfortable".into(),
            reduce_motion: false,
            sidebar_collapsed: false,
            glow: true,
            temp_unit: TempUnit::Celsius,
            last_section: "home".into(),
            open_on: "home".into(),
            disabled_extensions: Vec::new(),
            auto_update_extensions: false,
            announced_version: String::new(),
            open_groups: Default::default(),
            password_free: false,
        }
    }
}

thread_local! {
    static PREFS: RefCell<Prefs> = RefCell::new(load());
}

/// Read from disk (not this thread's cached copy), for worker threads and headless commands.
pub fn read() -> Prefs {
    load()
}

const SCHEMA: u32 = 1;

fn load() -> Prefs {
    let path = paths::prefs_file();
    let Ok(text) = std::fs::read_to_string(&path) else { return Prefs::default() };
    match toml::from_str(&text) {
        Ok(p) => p,
        Err(_) => {
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let _ = std::fs::rename(&path, path.with_file_name(format!("settings.toml.broken-{secs}")));
            Prefs::default()
        }
    }
}

/// Take what's on disk now (after restoring a backup), replacing this thread's copy.
pub fn reload() {
    PREFS.with(|p| *p.borrow_mut() = load());
}

pub fn get() -> Prefs {
    PREFS.with(|p| p.borrow().clone())
}

pub fn update(change: impl FnOnce(&mut Prefs)) {
    PREFS.with(|p| {
        let mut prefs = p.borrow_mut();
        change(&mut prefs);
        if let Ok(text) = toml::to_string_pretty(&*prefs) {
            let _ = cmd::atomic_write(&paths::prefs_file(), &text);
        }
    });
}
