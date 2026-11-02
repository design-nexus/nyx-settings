//! The boot screen, login screen, hibernation and snapshots. Omarchy's own
//! scripts do the work (they ask for sudo themselves), so they run in a terminal.

use crate::sections::accounts::terminal;
use crate::widgets::{self, Page};
use crate::{cmd, window};

pub fn build(page: &Page) {
    screens(page);
    sleep(page);
    recovery(page);
}

/// Names a script printed, one per line.
fn lines(program: &str) -> Vec<String> {
    cmd::output(&[program]).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

fn screens(page: &Page) {
    if !cmd::present("omarchy-plymouth-set-by-theme") {
        return;
    }
    let g = page.group("Boot and login screens");
    g.note("Styles the screen shown while the computer starts and the one you sign in on, using one of your themes. A terminal opens and asks for your password.");
    let themes = lines("omarchy-plymouth-list");
    if !themes.is_empty() {
        let current = cmd::output(&["omarchy-plymouth-current"]).unwrap_or_default();
        let mut options: Vec<(String, String)> = themes.iter().map(|t| (t.clone(), t.replace(['-', '_'], " "))).collect();
        if current == "default" || !themes.contains(&current) {
            options.insert(0, ("default".into(), "Omarchy default".into()));
        }
        let (r, _) = widgets::choice_row(
            "Style from a theme",
            "Colors and logo for the boot and login screens.",
            options,
            if themes.contains(&current) { &current } else { "default" },
            {
                let (themes, current) = (themes.clone(), current.clone());
                move |t| {
                    if t == current {
                        return;
                    }
                    if t == "default" {
                        terminal("omarchy-plymouth-reset");
                    } else if themes.contains(&t) {
                        terminal(&format!("omarchy-plymouth-set-by-theme {}", cmd::shell_quote(&t)));
                    }
                    // Show what's really set once the terminal is likely done, even if it was cancelled.
                    gtk::glib::timeout_add_seconds_local_once(30, || window::rebuild_if_built("boot"));
                }
            },
        );
        widgets::keywords("plymouth sddm splash boot screen login greeter");
        g.add(&r);
    }
    if cmd::present("omarchy-plymouth-reset") {
        let (r, _) =
            widgets::button_row("Back to default", "Restore Omarchy's own boot and login screens.", "Restore…", |_| {
                terminal("omarchy-plymouth-reset")
            });
        widgets::keywords("plymouth sddm reset original");
        g.add(&r);
    }
    if cmd::present("omarchy-refresh-sddm") {
        let (r, _) =
            widgets::button_row("Repair login screen", "Reinstall the login screen if it looks broken.", "Repair…", |_| {
                terminal("omarchy-refresh-sddm")
            });
        widgets::keywords("sddm refresh fix login");
        g.add(&r);
    }
}

fn sleep(page: &Page) {
    let g = page.group("Hibernation");
    if !cmd::present("omarchy-hibernation-setup") {
        g.note("Not available on this system.");
        return;
    }
    let on = cmd::present("omarchy-hibernation-available") && cmd::run(&["omarchy-hibernation-available"]).is_ok();
    let (r, _) = widgets::button_row(
        "Hibernation",
        if on {
            "On. The computer can save everything to disk and switch off completely."
        } else {
            "Off. Setting it up makes a swap file on disk and changes the boot settings."
        },
        if on { "Remove…" } else { "Set up…" },
        move |_| terminal(if on { "omarchy-hibernation-remove" } else { "omarchy-hibernation-setup" }),
    );
    widgets::keywords("hibernate swap resume suspend sleep disk");
    g.add(&r);
}

fn recovery(page: &Page) {
    let any = ["omarchy-snapshot", "omarchy-setup-direct-boot", "omarchy-refresh-limine"].iter().any(|c| cmd::present(c));
    if !any {
        return;
    }
    let g = page.group("Recovery and boot loader");
    if cmd::present("omarchy-snapshot") {
        let (r, _) = widgets::button_row(
            "Restore a snapshot",
            "Go back to a saved state of the system. Make one first from About.",
            "Choose…",
            |_| terminal("omarchy-snapshot restore"),
        );
        widgets::keywords("snapper rollback undo btrfs backup");
        g.add(&r);
    }
    if cmd::present("omarchy-setup-direct-boot") {
        let (r, _) = widgets::button_row(
            "Boot directly",
            "Add or remove a firmware boot entry so the computer starts Omarchy without the boot menu.",
            "Open…",
            |_| terminal("omarchy-setup-direct-boot"),
        );
        widgets::keywords("uki efi limine bootloader menu skip");
        g.add(&r);
    }
    if cmd::present("omarchy-refresh-limine") {
        let (r, _) = widgets::button_row("Repair the boot menu", "Rebuild the boot menu's settings.", "Repair…", |_| {
            terminal("omarchy-refresh-limine")
        });
        widgets::keywords("limine bootloader refresh fix grub");
        g.add(&r);
    }
}
