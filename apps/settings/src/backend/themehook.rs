//! The Omarchy theme-set hook. Omarchy resets the icon theme (and things like
//! keyboard lighting) on every theme change; this hook runs `settings --theme-sync`
//! right after, which publishes the Nyx theme engine's snapshot of the new theme,
//! puts back the icon theme chosen in Settings and tells the extensions that
//! asked, so they can do the same.

use super::appearance;
use crate::{cmd, paths};
use std::path::PathBuf;

pub fn hook_file() -> PathBuf {
    paths::omarchy_config().join("hooks/theme-set.d/50-settings-theme")
}

/// The hook earlier versions installed (keyboard lighting only).
fn old_hook_file() -> PathBuf {
    paths::omarchy_config().join("hooks/theme-set.d/50-settings-aura")
}

pub fn hook_script(self_cmd: &str) -> String {
    format!(
        "#!/bin/bash\n# Installed by Settings: re-apply the icon theme and extension settings you chose,\n\
         # after Omarchy resets them for a new theme. Settings puts this back when it starts.\n\
         exec {self_cmd} --theme-sync\n"
    )
}

fn wanted() -> bool {
    appearance::load().icon_theme.is_some()
        || crate::ext::any_wants("theme-changed")
        || nyx_theme::engine::suite().mode == nyx_theme::ThemeMode::Omarchy
}

/// Install, refresh or remove the hook to match what needs it.
pub fn ensure() {
    let _ = std::fs::remove_file(old_hook_file());
    let path = hook_file();
    if !wanted() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let want = hook_script(&super::hypr::self_command());
    if std::fs::read_to_string(&path).is_ok_and(|t| t == want) {
        return;
    }
    if cmd::atomic_write(&path, &want).is_ok() {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    }
}

/// `settings --theme-sync` (and the older `--aura-sync`).
pub fn sync() -> anyhow::Result<()> {
    // The Nyx system theme follows Omarchy's: publish it and tell every client.
    nyx_theme::announce("system", "omarchy-theme-set");
    let icons = appearance::apply_icons(&appearance::load());
    let extensions = crate::ext::theme_changed();
    icons.and(extensions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_runs_theme_sync() {
        let h = hook_script("/home/u/.local/bin/settings");
        assert!(h.starts_with("#!/bin/bash\n"));
        assert!(h.ends_with("exec /home/u/.local/bin/settings --theme-sync\n"));
    }
}
