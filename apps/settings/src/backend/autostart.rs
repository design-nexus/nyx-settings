//! Autostart desktop applications (`~/.config/autostart` and `/etc/xdg/autostart`).

use crate::paths;
use gtk::gio;
use gtk::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

const GROUP: &str = "Desktop Entry";

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub comment: String,
    pub exec: String,
    pub icon: String,
    pub enabled: bool,
    pub is_system: bool,
}

#[derive(Default, Debug)]
struct RawDesktop {
    name: Option<String>,
    exec: Option<String>,
    comment: Option<String>,
    icon: Option<String>,
    hidden: Option<bool>,
    no_display: Option<bool>,
    autostart_enabled: Option<bool>,
    only_show_in: Option<Vec<String>>,
    not_show_in: Option<Vec<String>>,
    try_exec: Option<String>,
}

fn read_desktop_file(path: &Path) -> Option<RawDesktop> {
    let kf = gtk::glib::KeyFile::new();
    if kf.load_from_file(path, gtk::glib::KeyFileFlags::NONE).is_err() {
        return None;
    }
    let name = kf.string(GROUP, "Name").ok().map(|s| s.to_string());
    let exec = kf.string(GROUP, "Exec").ok().map(|s| s.to_string());
    let comment = kf.string(GROUP, "Comment").ok().map(|s| s.to_string());
    let icon = kf.string(GROUP, "Icon").ok().map(|s| s.to_string());
    let hidden = kf.boolean(GROUP, "Hidden").ok();
    let no_display = kf.boolean(GROUP, "NoDisplay").ok();
    let autostart_enabled = kf.boolean(GROUP, "X-GNOME-Autostart-enabled").ok();
    let list = |key: &str| kf.string_list(GROUP, key).ok().map(|l| l.iter().map(|s| s.to_string()).collect::<Vec<String>>());
    let only_show_in = list("OnlyShowIn");
    let not_show_in = list("NotShowIn");
    let try_exec = kf.string(GROUP, "TryExec").ok().map(|s| s.to_string());

    Some(RawDesktop { name, exec, comment, icon, hidden, no_display, autostart_enabled, only_show_in, not_show_in, try_exec })
}

/// Desktops named in `XDG_CURRENT_DESKTOP` (colon separated).
fn current_desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().split(':').filter(|d| !d.is_empty()).map(str::to_string).collect()
}

/// Whether `OnlyShowIn` / `NotShowIn` let the entry start in these desktops.
fn shown_in(only: Option<&[String]>, not: Option<&[String]>, desktops: &[String]) -> bool {
    let named = |list: &[String]| list.iter().any(|d| desktops.iter().any(|c| c.eq_ignore_ascii_case(d)));
    if let Some(only) = only
        && !only.is_empty()
        && !named(only)
    {
        return false;
    }
    !not.is_some_and(named)
}

/// Whether the `TryExec` program exists and can be run.
fn try_exec_ok(try_exec: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let runnable = |p: &Path| p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if try_exec.contains('/') {
        return runnable(Path::new(try_exec));
    }
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| runnable(&d.join(try_exec))))
}

/// Drop the desktop-entry field codes (`%U`, `%f`, …) that only mean something to a launcher.
pub fn strip_field_codes(exec: &str) -> String {
    let mut out = String::new();
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        if chars.next() == Some('%') {
            out.push('%');
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn system_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(val) = std::env::var_os("XDG_CONFIG_DIRS") {
        for p in std::env::split_paths(&val) {
            dirs.push(p.join("autostart"));
        }
    }
    if dirs.is_empty() {
        dirs.push(PathBuf::from("/etc/xdg/autostart"));
    }
    dirs
}

/// Read all autostart items combining user and system autostart directories.
pub fn list_from(user_dir: &Path, sys_dirs: &[PathBuf]) -> Vec<Item> {
    let mut system_entries: BTreeMap<String, RawDesktop> = BTreeMap::new();
    for dir in sys_dirs {
        if !dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                let Some(id) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
                if system_entries.contains_key(&id) {
                    continue;
                }
                if let Some(raw) = read_desktop_file(&path) {
                    system_entries.insert(id, raw);
                }
            }
        }
    }

    let mut user_entries: BTreeMap<String, RawDesktop> = BTreeMap::new();
    if user_dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(user_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Some(id) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            if let Some(raw) = read_desktop_file(&path) {
                user_entries.insert(id, raw);
            }
        }
    }

    let desktops = current_desktops();
    let all_ids: HashSet<String> = system_entries.keys().chain(user_entries.keys()).cloned().collect();
    let mut items = Vec::new();

    for id in all_ids {
        let sys = system_entries.get(&id);
        let usr = user_entries.get(&id);

        let no_display = usr.and_then(|u| u.no_display).or_else(|| sys.and_then(|s| s.no_display)).unwrap_or(false);
        if no_display {
            continue;
        }

        let only = usr.and_then(|u| u.only_show_in.as_deref()).or_else(|| sys.and_then(|s| s.only_show_in.as_deref()));
        let not = usr.and_then(|u| u.not_show_in.as_deref()).or_else(|| sys.and_then(|s| s.not_show_in.as_deref()));
        if !shown_in(only, not, &desktops) {
            continue;
        }
        let try_exec = usr.and_then(|u| u.try_exec.as_deref()).or_else(|| sys.and_then(|s| s.try_exec.as_deref()));
        if try_exec.is_some_and(|t| !try_exec_ok(t)) {
            continue;
        }

        let name = usr
            .and_then(|u| u.name.clone())
            .or_else(|| sys.and_then(|s| s.name.clone()))
            .unwrap_or_else(|| id.trim_end_matches(".desktop").to_string());

        let exec = usr.and_then(|u| u.exec.clone()).or_else(|| sys.and_then(|s| s.exec.clone())).unwrap_or_default();

        let comment = usr.and_then(|u| u.comment.clone()).or_else(|| sys.and_then(|s| s.comment.clone())).unwrap_or_default();

        let icon = usr.and_then(|u| u.icon.clone()).or_else(|| sys.and_then(|s| s.icon.clone())).unwrap_or_default();

        // An orphan file with no name and no exec is invalid
        if exec.is_empty() && name == id.trim_end_matches(".desktop") {
            continue;
        }

        let is_system = sys.is_some();

        let enabled = if let Some(u) = usr {
            !(u.hidden == Some(true) || u.autostart_enabled == Some(false))
        } else if let Some(s) = sys {
            !(s.hidden == Some(true) || s.autostart_enabled == Some(false))
        } else {
            false
        };

        items.push(Item { id, name, comment, exec, icon, enabled, is_system });
    }

    // Active/enabled first, then alphabetical by name
    items.sort_by(|a, b| b.enabled.cmp(&a.enabled).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));

    items
}

/// List all startup items on the running system.
pub fn list() -> Vec<Item> {
    list_from(&paths::autostart_dir(), &system_dirs())
}

/// Set enabled/disabled state for an autostart item.
pub fn set_enabled_in(user_dir: &Path, id: &str, is_system: bool, enabled: bool) -> Result<(), String> {
    std::fs::create_dir_all(user_dir).map_err(|e| format!("Couldn't create autostart directory: {e}"))?;
    let path = user_dir.join(id);

    let kf = gtk::glib::KeyFile::new();
    let loaded = path.exists() && kf.load_from_file(&path, gtk::glib::KeyFileFlags::NONE).is_ok();
    let has_exec = loaded && kf.string(GROUP, "Exec").is_ok();

    if enabled {
        // A user file with no command only exists to mask the system entry. It would replace
        // that entry whole (command and all) if kept, so removing it is what turns the entry back on.
        if is_system && !has_exec {
            if path.exists() {
                std::fs::remove_file(&path).map_err(|e| format!("Couldn't enable {id}: {e}"))?;
            }
            return Ok(());
        }
        kf.set_boolean(GROUP, "Hidden", false);
        kf.set_boolean(GROUP, "X-GNOME-Autostart-enabled", true);
    } else {
        if !loaded && !is_system {
            kf.set_string(GROUP, "Type", "Application");
        }
        kf.set_boolean(GROUP, "Hidden", true);
        if has_exec {
            kf.set_boolean(GROUP, "X-GNOME-Autostart-enabled", false);
        }
    }
    kf.save_to_file(&path).map_err(|e| format!("Couldn't save {id}: {e}"))
}

pub fn set_enabled(id: &str, is_system: bool, enabled: bool) -> Result<(), String> {
    set_enabled_in(&paths::autostart_dir(), id, is_system, enabled)
}

/// Remove an autostart item. User items are deleted; system items are disabled via Hidden=true.
pub fn remove_in(user_dir: &Path, id: &str, is_system: bool) -> Result<(), String> {
    if is_system {
        set_enabled_in(user_dir, id, is_system, false)
    } else {
        let path = user_dir.join(id);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("Couldn't remove {id}: {e}"))?;
        }
        Ok(())
    }
}

pub fn remove(id: &str, is_system: bool) -> Result<(), String> {
    remove_in(&paths::autostart_dir(), id, is_system)
}

/// Add an installed desktop application to the autostart directory.
pub fn add_app_in(user_dir: &Path, app: &gio::AppInfo) -> Result<String, String> {
    std::fs::create_dir_all(user_dir).map_err(|e| format!("Couldn't create autostart directory: {e}"))?;

    let name = app.name().to_string();
    let id = app.id().map(|i| i.to_string()).unwrap_or_else(|| format!("{}.desktop", name.to_lowercase().replace(' ', "-")));
    let id = if id.ends_with(".desktop") { id } else { format!("{id}.desktop") };

    let target = user_dir.join(&id);

    // Look for existing desktop file in applications directories to copy
    let app_dirs = [
        paths::data_home().join("applications"),
        PathBuf::from("/usr/local/share/applications"),
        PathBuf::from("/usr/share/applications"),
    ];

    let mut copied = false;
    for dir in &app_dirs {
        let src = dir.join(&id);
        if src.exists() {
            let kf = gtk::glib::KeyFile::new();
            if kf.load_from_file(&src, gtk::glib::KeyFileFlags::NONE).is_ok() {
                if let Ok(exec) = kf.string(GROUP, "Exec") {
                    kf.set_string(GROUP, "Exec", &strip_field_codes(&exec));
                }
                let _ = kf.remove_key(GROUP, "DBusActivatable");
                kf.set_boolean(GROUP, "Hidden", false);
                kf.set_boolean(GROUP, "X-GNOME-Autostart-enabled", true);
                if kf.save_to_file(&target).is_ok() {
                    copied = true;
                    break;
                }
            }
        }
    }

    if !copied {
        let exec = app.commandline().map(|c| strip_field_codes(&c.to_string_lossy())).unwrap_or_default();
        let icon = app.icon().and_then(|i| i.to_string()).map(|s| s.to_string()).unwrap_or_default();
        let kf = gtk::glib::KeyFile::new();
        kf.set_string(GROUP, "Type", "Application");
        kf.set_string(GROUP, "Name", &name);
        kf.set_string(GROUP, "Exec", &exec);
        if !icon.is_empty() {
            kf.set_string(GROUP, "Icon", &icon);
        }
        kf.set_boolean(GROUP, "Hidden", false);
        kf.set_boolean(GROUP, "X-GNOME-Autostart-enabled", true);
        kf.save_to_file(&target).map_err(|e| format!("Couldn't create autostart desktop file: {e}"))?;
    }

    Ok(name)
}

pub fn add_app(app: &gio::AppInfo) -> Result<String, String> {
    add_app_in(&paths::autostart_dir(), app)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("settings-autostart-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn parses_and_sorts_items() {
        let temp = TempDir::new("parse");
        let user_dir = temp.path().join("user_autostart");
        let sys_dir = temp.path().join("sys_autostart");
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::create_dir_all(&sys_dir).unwrap();

        // 1. User app: Enabled
        std::fs::write(user_dir.join("dropbox.desktop"), "[Desktop Entry]\nName=Dropbox\nExec=dropbox\nType=Application\n")
            .unwrap();

        // 2. System app: Enabled
        std::fs::write(
            sys_dir.join("system-helper.desktop"),
            "[Desktop Entry]\nName=System Helper\nExec=syshelper\nType=Application\n",
        )
        .unwrap();

        // 3. System app with NoDisplay: should be ignored
        std::fs::write(sys_dir.join("daemon.desktop"), "[Desktop Entry]\nName=Daemon\nExec=daemon\nNoDisplay=true\n").unwrap();

        // 4. System app masked by user with Hidden=true: should be disabled
        std::fs::write(sys_dir.join("print-applet.desktop"), "[Desktop Entry]\nName=Print Applet\nExec=printapp\n").unwrap();
        std::fs::write(user_dir.join("print-applet.desktop"), "[Desktop Entry]\nHidden=true\n").unwrap();

        let items = list_from(&user_dir, &[sys_dir]);
        assert_eq!(items.len(), 3);

        // Enabled items come first
        assert!(items[0].enabled);
        assert!(items[1].enabled);
        assert!(!items[2].enabled);

        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"Dropbox"));
        assert!(names.contains(&"System Helper"));
        assert!(names.contains(&"Print Applet"));
    }

    #[test]
    fn toggles_and_removes() {
        let temp = TempDir::new("toggle");
        let user_dir = temp.path().join("user_autostart");
        let sys_dir = temp.path().join("sys_autostart");
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::create_dir_all(&sys_dir).unwrap();

        std::fs::write(user_dir.join("app.desktop"), "[Desktop Entry]\nName=App\nExec=app\nType=Application\n").unwrap();

        // Disable
        set_enabled_in(&user_dir, "app.desktop", false, false).unwrap();
        let items = list_from(&user_dir, std::slice::from_ref(&sys_dir));
        assert_eq!(items.len(), 1);
        assert!(!items[0].enabled);

        // Re-enable
        set_enabled_in(&user_dir, "app.desktop", false, true).unwrap();
        let items = list_from(&user_dir, std::slice::from_ref(&sys_dir));
        assert_eq!(items.len(), 1);
        assert!(items[0].enabled);

        // Remove
        remove_in(&user_dir, "app.desktop", false).unwrap();
        let items = list_from(&user_dir, &[sys_dir]);
        assert!(items.is_empty());
    }

    #[test]
    fn reenabling_a_system_item_restores_it() {
        let temp = TempDir::new("system");
        let user_dir = temp.path().join("user_autostart");
        let sys_dir = temp.path().join("sys_autostart");
        std::fs::create_dir_all(&sys_dir).unwrap();
        std::fs::write(sys_dir.join("helper.desktop"), "[Desktop Entry]\nName=Helper\nExec=helper\n").unwrap();

        set_enabled_in(&user_dir, "helper.desktop", true, false).unwrap();
        let items = list_from(&user_dir, std::slice::from_ref(&sys_dir));
        assert!(!items[0].enabled);

        set_enabled_in(&user_dir, "helper.desktop", true, true).unwrap();
        assert!(!user_dir.join("helper.desktop").exists(), "the mask must go so the system entry applies whole");
        let items = list_from(&user_dir, &[sys_dir]);
        assert!(items[0].enabled);
    }

    #[test]
    fn skips_entries_for_other_desktops_and_missing_programs() {
        let temp = TempDir::new("desktops");
        let sys_dir = temp.path().join("sys_autostart");
        std::fs::create_dir_all(&sys_dir).unwrap();
        std::fs::write(sys_dir.join("gnome-only.desktop"), "[Desktop Entry]\nName=G\nExec=g\nOnlyShowIn=GNOME;\n").unwrap();
        std::fs::write(sys_dir.join("missing.desktop"), "[Desktop Entry]\nName=M\nExec=m\nTryExec=/nonexistent/program\n")
            .unwrap();
        std::fs::write(sys_dir.join("fine.desktop"), "[Desktop Entry]\nName=Fine\nExec=f\n").unwrap();

        let names: Vec<String> = list_from(&temp.path().join("none"), &[sys_dir]).into_iter().map(|i| i.name).collect();
        assert_eq!(names, ["Fine"]);
    }

    #[test]
    fn matches_desktops() {
        let hypr = vec!["Hyprland".to_string()];
        let gnome = vec!["GNOME".to_string()];
        assert!(shown_in(None, None, &hypr));
        assert!(!shown_in(Some(&gnome), None, &hypr));
        assert!(shown_in(Some(&["hyprland".to_string()]), None, &hypr));
        assert!(!shown_in(None, Some(&hypr), &hypr));
    }

    #[test]
    fn strips_field_codes() {
        assert_eq!(strip_field_codes("app --flag %U"), "app --flag");
        assert_eq!(strip_field_codes("app %f %i 100%%"), "app 100%");
    }
}
