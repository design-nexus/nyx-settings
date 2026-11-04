//! Omarchy: what Settings does its own way there. Most of Omarchy's specifics live in
//! the pages themselves, which were written for it.

use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;

/// About's Omarchy group: its version and channel, updates (on Home) and snapshots.
pub fn about(page: &Page) {
    let g = page.group("Omarchy");
    let (r, _) = widgets::info_row("Version", &cmd::output(&["omarchy-version"]).unwrap_or_default());
    g.add(&r);
    if let Some(ch) = cmd::output(&["omarchy-version-channel"]) {
        let (r, _) = widgets::info_row("Channel", &ch);
        g.add(&r);
    }
    // Home is the one place that lists and installs system updates.
    let cached = crate::backend::pkgupdates::cached();
    let waiting = (cached.checked > 0).then(|| cached.count());
    let (r, _) = widgets::button_row(
        "Updates",
        &match waiting {
            Some(0) => "Omarchy and every package are up to date. Home lists updates when there are some.".to_string(),
            Some(n) => format!("{n} waiting: Omarchy, packages, the AUR and firmware. Home lists them and installs them."),
            None => "Omarchy, package, AUR and firmware updates are listed and installed on Home.".to_string(),
        },
        "Open Home",
        |_| window::navigate("home"),
    );
    widgets::keywords("update upgrade pacman packages available list aur firmware");
    g.add(&r);
    let (r, _) = widgets::button_row(
        "Snapshot",
        "Save the system as it is now, so you can roll back after a bad update.",
        "Create snapshot",
        |b| {
            b.set_sensitive(false);
            let b = b.clone();
            cmd::run_async(&["omarchy-snapshot", "create"], move |r| {
                b.set_sensitive(true);
                match r {
                    Ok(_) => window::toast("Snapshot created"),
                    Err(e) => window::toast(&format!("{e}")),
                }
            });
        },
    );
    widgets::keywords("backup restore snapper rollback");
    g.add(&r);
}
