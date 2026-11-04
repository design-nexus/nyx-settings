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
