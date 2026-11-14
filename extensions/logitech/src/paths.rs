//! Well-known locations, honouring the XDG overrides.

use std::path::PathBuf;

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(".config"))
}

pub fn state_home() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/state"))
}

/// Saved device settings, shared with the Logi app and re-applied whenever a
/// device (re)connects.
pub fn devices_file() -> PathBuf {
    match crate::devices::mock_state() {
        Some(dir) => dir.join("devices.toml"),
        None => config_home().join("logi/devices.toml"),
    }
}

pub fn udev_rule() -> PathBuf {
    PathBuf::from("/etc/udev/rules.d/42-logi.rules")
}

pub fn omarchy_colors() -> PathBuf {
    state_home().join("omarchy/current/theme/colors.toml")
}
