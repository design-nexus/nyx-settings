//! The platform Settings is built for: Omarchy (the default) or NyxOS (the `nyxos`
//! feature). The pages are shared; what a platform does its own way (its sidebar, its
//! system pages, its updates and version) comes from here, so pages never ask which
//! platform they're on.

pub mod nyxos;
mod omarchy;

use crate::sections::Section;
use crate::sections::home::Hero;
use crate::widgets::Page;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Omarchy,
    NyxOS,
}

pub const PLATFORM: Platform = if cfg!(feature = "nyxos") { Platform::NyxOS } else { Platform::Omarchy };

/// The sidebar's pages: the platform's categories, names and own pages.
pub fn layout(list: &mut [Section]) {
    if PLATFORM == Platform::NyxOS {
        nyxos::layout::apply(list);
    }
}

/// Pages that open from another page instead of the sidebar.
pub fn off_sidebar(id: &str) -> bool {
    PLATFORM == Platform::NyxOS && nyxos::layout::off_sidebar(id)
}

/// The group whose colours a sidebar group takes.
pub fn colour_group(group: &str) -> &str {
    match PLATFORM {
        Platform::NyxOS => nyxos::layout::colour_group(group),
        Platform::Omarchy => group,
    }
}

/// Home's updates: Omarchy's packages, or NyxOS's generation.
/// System › Updates is NyxOS's (on Omarchy, Software has the updates).
pub fn has_updates() -> bool {
    PLATFORM == Platform::NyxOS
}

pub fn updates(page: &Page) {
    if PLATFORM == Platform::NyxOS {
        nyxos::updates::page(page);
    }
}

pub fn home_updates(page: &Page, hero: Rc<Hero>) {
    match PLATFORM {
        Platform::NyxOS => nyxos::home_updates(page, &hero),
        Platform::Omarchy => crate::sections::home::updates_group(page, hero),
    }
}

/// Whether system changes can be made from `section`, and a notice to show when they
/// can't yet (Omarchy's root helper isn't set up).
pub fn system_notice(section: &'static str, what: &str) -> (bool, Option<gtk::Box>) {
    match PLATFORM {
        Platform::NyxOS => (true, None),
        Platform::Omarchy => crate::sections::accounts::helper_notice(section, what),
    }
}

/// What logind does for a lid or button event (`key` is one of `[Login]`'s).
pub fn set_power(key: &str, action: &str, section: &'static str) {
    match PLATFORM {
        Platform::NyxOS => nyxos::set_power(key, action, section),
        Platform::Omarchy => crate::sections::accounts::run_admin(&["logind", key, action], None, "Saved", section),
    }
}

/// How long suspend-then-hibernate sleeps before hibernating, in seconds.
pub fn set_hibernate_delay(secs: &str, section: &'static str) {
    match PLATFORM {
        Platform::NyxOS => nyxos::set_hibernate_delay(secs, section),
        Platform::Omarchy => crate::sections::accounts::run_admin(&["sleep-delay", secs], None, "Saved", section),
    }
}

pub fn hibernation_available() -> bool {
    match PLATFORM {
        Platform::NyxOS => nyxos::hibernation_available(),
        Platform::Omarchy => crate::cmd::run(&["omarchy-hibernation-available"]).is_ok(),
    }
}

/// The Network page's firewall group.
pub fn firewall_group(page: &Page) {
    match PLATFORM {
        Platform::NyxOS => nyxos::pages::firewall(page),
        Platform::Omarchy => crate::sections::network::ufw_group(page),
    }
}

/// Whether the printing service (CUPS) is on.
pub fn printing_on() -> bool {
    let active = |unit: &str| crate::cmd::output(&["systemctl", "is-active", unit]).is_some_and(|s| s == "active");
    match PLATFORM {
        // NixOS starts CUPS from its socket when something prints.
        Platform::NyxOS => active("cups.socket") || active("cups.service"),
        Platform::Omarchy => active("cups.service"),
    }
}

/// Turn the printing service (CUPS) on or off.
pub fn set_printing(on: bool, section: &'static str) {
    match PLATFORM {
        Platform::NyxOS => nyxos::set_service("printing", on, if on { "Printing is on" } else { "Printing is off" }, section),
        Platform::Omarchy => crate::sections::accounts::run_admin(
            &["service", if on { "enable-now" } else { "disable-now" }, "cups.service"],
            None,
            if on { "Printing is on" } else { "Printing is off" },
            section,
        ),
    }
}

/// Add a driverless printer (`queue` is its CUPS name).
pub fn printer_add(queue: &str, uri: &str, section: &'static str) {
    match PLATFORM {
        Platform::NyxOS => nyxos::printers::add(queue, uri, section),
        Platform::Omarchy => crate::sections::accounts::run_admin(&["printer-add", queue, uri], None, "Printer added", section),
    }
}

pub fn printer_remove(queue: &str, section: &'static str) {
    match PLATFORM {
        Platform::NyxOS => nyxos::printers::remove(queue, section),
        Platform::Omarchy => crate::sections::accounts::run_admin(&["printer-remove", queue], None, "Printer removed", section),
    }
}

pub fn printer_set_uri(queue: &str, uri: &str, section: &'static str) {
    match PLATFORM {
        Platform::NyxOS => nyxos::printers::set_uri(queue, uri, section),
        Platform::Omarchy => {
            crate::sections::accounts::run_admin(&["printer-set-uri", queue, uri], None, "Printer updated", section)
        }
    }
}

/// `program` (from `package`) isn't installed: offer it, or say the system brings it.
/// Whether to go ahead anyway.
pub fn provide_tool(program: &str, package: &'static str) -> bool {
    match PLATFORM {
        // A share's tools come with the first mount of its kind.
        Platform::NyxOS if program.starts_with("mount.") => true,
        Platform::NyxOS => {
            crate::window::toast_action(&format!("This needs {package}, which isn't installed."), "Open Store", move || {
                let _ = std::process::Command::new("nyx-store").args(["--search", package]).spawn();
            });
            false
        }
        Platform::Omarchy => {
            crate::window::toast_action(&format!("This needs {package}, which isn't installed."), "Install", move || {
                crate::sections::accounts::terminal(&format!("sudo pacman -S --needed -- {package}"))
            });
            false
        }
    }
}

/// How adding a mount is asked for.
pub fn mounts_note() -> &'static str {
    match PLATFORM {
        Platform::NyxOS => {
            "Adding one asks for an administrator's password; it becomes part of the system configuration, which is rebuilt."
        }
        Platform::Omarchy => {
            "Adding one asks for your password once, then remembers it for a few minutes (Users can turn this off)."
        }
    }
}

/// Whether system changes can be made now (Omarchy needs its root helper installed).
pub fn system_ready() -> bool {
    match PLATFORM {
        Platform::NyxOS => true,
        Platform::Omarchy => crate::backend::accounts::helper_installed(),
    }
}

/// fstab, with the mounts Settings made named by their ids.
pub fn fstab() -> Vec<crate::backend::mounts::Mount> {
    match PLATFORM {
        Platform::NyxOS => nyxos::mounts::fstab(),
        Platform::Omarchy => crate::backend::mounts::fstab(),
    }
}

// Mounts: blocking, for a background thread.

pub fn mount_add_drive(id: &str, uuid: &str, dir: &str, fs: &str, options: &str) -> anyhow::Result<String> {
    match PLATFORM {
        Platform::NyxOS => nyxos::mounts::add_drive(id, uuid, dir, fs, options),
        Platform::Omarchy => crate::backend::accounts::helper(&["mount-add-drive", id, uuid, dir, fs, options], None),
    }
}

/// `login` is a share's user, password and domain on three lines (empty for a guest).
pub fn mount_add_network(id: &str, fstype: &str, source: &str, dir: &str, options: &str, login: &str) -> anyhow::Result<String> {
    match PLATFORM {
        Platform::NyxOS => nyxos::mounts::add_network(id, fstype, source, dir, options, login),
        Platform::Omarchy => crate::backend::accounts::helper(&["mount-add-net", id, fstype, source, dir, options], Some(login)),
    }
}

pub fn mount_remove(id: &str) -> anyhow::Result<String> {
    match PLATFORM {
        Platform::NyxOS => nyxos::mounts::remove(id),
        Platform::Omarchy => crate::backend::accounts::helper(&["mount-remove", id], None),
    }
}

/// Try a share before adding it.
pub fn mount_test(fstype: &str, source: &str, options: &str, login: &str) -> anyhow::Result<String> {
    match PLATFORM {
        Platform::NyxOS => {
            anyhow::bail!("Testing a share first isn't on NyxOS yet; add it, and the list shows whether it mounts")
        }
        Platform::Omarchy => crate::backend::accounts::helper(&["mount-test", fstype, source, options], Some(login)),
    }
}

/// The system's rows in Disks' Clean up group.
pub fn system_cleanup(g: &crate::widgets::Group) {
    match PLATFORM {
        Platform::NyxOS => nyxos::pages::cleanup_rows(g),
        Platform::Omarchy => crate::sections::cleanup::omarchy_rows(g),
    }
}

/// About's group for the system itself.
pub fn about(page: &Page) {
    match PLATFORM {
        Platform::NyxOS => nyxos::about(page),
        Platform::Omarchy => omarchy::about(page),
    }
}

/// The power profiles power-profiles-daemon offers, power saver first, from
/// `powerprofilesctl list`.
pub fn power_profiles() -> Vec<String> {
    parse_profiles(&crate::cmd::output(&["powerprofilesctl", "list"]).unwrap_or_default())
}

fn parse_profiles(list: &str) -> Vec<String> {
    let mut out: Vec<String> = list
        .lines()
        .filter_map(|l| {
            let name = l.trim().trim_start_matches('*').trim().strip_suffix(':')?;
            (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).then(|| name.to_string())
        })
        .collect();
    out.reverse();
    out
}

/// Where the profile for each power source (`ac`, `battery`) is kept.
pub fn power_profile_state_dir() -> std::path::PathBuf {
    match PLATFORM {
        Platform::NyxOS => nyxos::power::profile_state_dir(),
        Platform::Omarchy => std::env::var_os("OMARCHY_POWERPROFILES_STATE_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| crate::paths::state_home().join("omarchy/powerprofiles")),
    }
}

/// Save the profile for a power source, and switch to it when that source is in use.
pub fn set_power_profile(source: &str, profile: &str, in_use: bool) -> anyhow::Result<()> {
    match PLATFORM {
        Platform::Omarchy if in_use => crate::cmd::run(&["omarchy-powerprofiles-set", source, profile]).map(|_| ()),
        _ => {
            crate::cmd::atomic_write(&power_profile_state_dir().join(source), format!("{profile}\n"))?;
            if in_use {
                crate::cmd::run(&["powerprofilesctl", "set", profile])?;
            }
            Ok(())
        }
    }
}

pub fn battery_present() -> bool {
    match PLATFORM {
        Platform::NyxOS => nyxos::power::battery_present(),
        Platform::Omarchy => {
            crate::cmd::output(&["omarchy-battery-present"]).is_some()
                || std::path::Path::new("/sys/class/power_supply/BAT0").exists()
        }
    }
}

/// The battery as `key\tvalue` lines: state, time, percentage, rate, size (and threshold).
pub fn battery_shell() -> String {
    match PLATFORM {
        Platform::NyxOS => nyxos::power::battery_shell(),
        Platform::Omarchy => crate::cmd::output(&["omarchy-battery-status", "--shell"]).unwrap_or_default(),
    }
}

/// The screen's brightness in percent, when it can be set.
pub fn brightness() -> Option<f64> {
    match PLATFORM {
        Platform::NyxOS => nyxos::power::brightness(),
        Platform::Omarchy => {
            crate::cmd::output(&["omarchy-brightness-display", "--no-osd"]).and_then(|s| s.trim().parse::<f64>().ok())
        }
    }
}

pub fn set_brightness(percent: i64) {
    match PLATFORM {
        Platform::NyxOS => nyxos::power::set_brightness(percent),
        Platform::Omarchy => crate::cmd::spawn(&["omarchy-brightness-display", "--no-osd", &format!("{percent}%")]),
    }
}

/// The command that locks the screen now.
pub fn lock_command() -> &'static [&'static str] {
    match PLATFORM {
        Platform::NyxOS => &["loginctl", "lock-session"],
        Platform::Omarchy => &["omarchy-system-lock"],
    }
}

/// The session's end: `logout`, `reboot` or `shutdown`.
pub fn session_command(what: &str) -> &'static [&'static str] {
    match (PLATFORM, what) {
        (Platform::NyxOS, "logout") => &["uwsm", "stop"],
        (Platform::NyxOS, "reboot") => &["systemctl", "reboot"],
        (Platform::NyxOS, _) => &["systemctl", "poweroff"],
        (Platform::Omarchy, "logout") => &["omarchy-system-logout"],
        (Platform::Omarchy, "reboot") => &["omarchy-system-reboot"],
        (Platform::Omarchy, _) => &["omarchy-system-shutdown"],
    }
}

/// Run `command` (a shell line) in a terminal window: Omarchy's floating one, or the
/// default terminal (`xdg-terminal-exec`).
pub fn in_terminal(command: &str) {
    match PLATFORM {
        Platform::NyxOS => crate::cmd::spawn(&["xdg-terminal-exec", "sh", "-c", command]),
        Platform::Omarchy => crate::cmd::spawn(&["omarchy-launch-floating-terminal-with-presentation", command]),
    }
}

#[cfg(test)]
mod power_tests {
    #[test]
    fn profiles_from_powerprofilesctl() {
        let list = "  performance:\n    CpuDriver:\tintel_pstate\n\n* balanced:\n    CpuDriver:\tintel_pstate\n\n  power-saver:\n    PlatformDriver:\tplaceholder\n";
        assert_eq!(super::parse_profiles(list), vec!["power-saver", "balanced", "performance"]);
        assert!(super::parse_profiles("").is_empty());
    }
}
