//! Well-known locations. Every path honours the XDG overrides so the whole app
//! can be pointed at a scratch copy of `~/.config` for testing.

pub use nyx_core::paths::*;

use std::path::PathBuf;

/// `~/.local/share/nyx-settings/extensions` — one folder per installed extension.
pub fn ext_dir() -> PathBuf {
    data_dir().join("extensions")
}

/// Packaged extensions: `nyx-settings/extensions` in each of `$XDG_DATA_DIRS`, first
/// wins. Folders that really live under `$HOME` are left out (anything there can be
/// written without a package manager; a Nix profile resolves into the read-only store).
pub fn system_ext_dirs() -> Vec<PathBuf> {
    let dirs =
        std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let home = std::env::var_os("HOME").map(PathBuf::from).filter(|h| h != std::path::Path::new("/"));
    dirs.split(':')
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join("nyx-settings/extensions"))
        .filter(|d| {
            let Some(home) = &home else { return true };
            !d.canonicalize().unwrap_or_else(|_| d.clone()).starts_with(home)
        })
        .collect()
}

/// What extensions said they offer last time, so the sidebar fills in without waiting.
pub fn ext_cache_dir() -> PathBuf {
    cache_dir().join("extensions")
}

/// `~/.config/autostart` — user autostart desktop entries.
pub fn autostart_dir() -> PathBuf {
    config_home().join("autostart")
}

pub fn state_file() -> PathBuf {
    app_dir().join("state.json")
}

pub fn hypr_dir() -> PathBuf {
    config_home().join("hypr")
}

/// The single Hyprland file this app writes.
pub fn managed_lua() -> PathBuf {
    hypr_dir().join("settings.lua")
}

pub fn hyprland_lua() -> PathBuf {
    hypr_dir().join("hyprland.lua")
}

pub fn omarchy_config() -> PathBuf {
    config_home().join("omarchy")
}

pub fn shell_json() -> PathBuf {
    omarchy_config().join("shell.json")
}
