use crate::backend::updates;
use crate::widgets::{self, Page};
use crate::{cmd, prefs, units, window};
use gtk::prelude::*;
use std::cell::RefCell;

thread_local! {
    /// The version just installed, until Settings restarts into it.
    static INSTALLED: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn checked_text(s: &updates::Status) -> String {
    if s.checked == 0 {
        return "Not checked yet.".into();
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("Checked {}.", units::ago(now.saturating_sub(s.checked) as i64))
}

fn check_now(b: &gtk::Button) {
    b.set_sensitive(false);
    b.set_label("Checking…");
    cmd::background(
        || updates::check(true),
        |s| {
            if !s.settings_update() && s.extensions.is_empty() {
                window::toast("Everything is up to date");
            }
            window::after_update_check(s);
        },
    );
}

/// Settings' own version and updates, and extension updates.
fn settings_group(page: &Page) {
    let g = page.group("Settings");
    let s = updates::cached();
    let current = env!("CARGO_PKG_VERSION");
    if let Some(v) = INSTALLED.with(|i| i.borrow().clone()) {
        let (r, _) = widgets::button_row(
            &format!("Settings {v} is installed"),
            "Restart Settings to start using it.",
            "Restart Settings",
            |_| window::restart("about"),
        );
        g.add(&r);
    } else if s.settings_update() {
        let line = widgets::hbox(8);
        if !s.url.is_empty() {
            let notes = gtk::Button::with_label("What's new");
            let url = s.url.clone();
            notes.connect_clicked(move |_| cmd::spawn(&["xdg-open", &url]));
            line.append(&notes);
        }
        let update = gtk::Button::with_label("Update");
        update.add_css_class("suggested-action");
        let can = updates::self_updatable();
        update.set_sensitive(can);
        let latest = s.latest.clone();
        update.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_label("Updating…");
            let (b, latest) = (b.clone(), latest.clone());
            cmd::background(
                || updates::update_settings().map_err(|e| format!("{e:#}")),
                move |r| match r {
                    Ok(()) => {
                        INSTALLED.with(|i| *i.borrow_mut() = Some(latest.clone()));
                        window::toast(&format!("Settings {latest} is installed. Restart to use it."));
                        window::rebuild("about");
                    }
                    Err(e) => {
                        b.set_sensitive(true);
                        b.set_label("Update");
                        window::toast(&format!("Couldn't update: {e}"));
                    }
                },
            );
        });
        line.append(&update);
        let desc = if can {
            format!("You have {current}. Updating takes a few seconds; then restart Settings.")
        } else {
            format!("You have {current}, a development build: update it from its source.")
        };
        g.add(&widgets::row(&format!("Settings {} is available", s.latest), &desc, Some(line.upcast_ref())));
    } else {
        let (r, _) = widgets::button_row(
            &format!("Settings {current}"),
            &format!("Up to date. {}", checked_text(&s)),
            "Check now",
            check_now,
        );
        g.add(&r);
    }
    widgets::keywords("update upgrade version new release settings app check");

    // Only extensions still installed (the list may predate a removal).
    let names: Vec<String> = crate::ext::installed()
        .into_iter()
        .filter(|e| s.extensions.iter().any(|id| id == e.id()))
        .map(|e| e.manifest.name)
        .collect();
    if !names.is_empty() {
        let (r, _) = widgets::button_row(
            &format!("Extension updates ({})", names.len()),
            &glib_escape(&names.join(", ")),
            "Update all",
            |b| {
                b.set_sensitive(false);
                b.set_label("Updating…");
                let b = b.clone();
                cmd::background(
                    || updates::update_extensions().map_err(|e| format!("{e:#}")),
                    move |r| {
                        b.set_sensitive(true);
                        match r {
                            Ok(n) => window::toast(&format!("Updated {}", n.join(", "))),
                            Err(e) => window::toast(&format!("Couldn't update: {e}")),
                        }
                        window::reload_sections(false);
                        window::rebuild("about");
                        window::rebuild_if_built("extensions");
                    },
                );
            },
        );
        g.add(&r);
    }
    let (r, _) = widgets::switch_row(
        "Install extension updates automatically",
        "Settings looks for updates once a day when it opens. Settings itself always asks first.",
        prefs::get().auto_update_extensions,
        |on| prefs::update(|p| p.auto_update_extensions = on),
    );
    widgets::keywords("automatic auto update extensions");
    g.add(&r);
}

fn glib_escape(t: &str) -> String {
    gtk::glib::markup_escape_text(t).to_string()
}

/// Everything changed here, by page, with a way to see it and to undo it.
fn changes_group(page: &Page) {
    let g = page.collapsible("Your changes", false);
    let st = crate::backend::store::read(|s| s.clone());
    // Hyprland options, by the page their row is on.
    let mut by_page: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for key in st.options.keys() {
        let (title, section) = widgets::option_row(key).unwrap_or_else(|| (key.clone(), String::new()));
        match by_page.iter_mut().find(|(s, _)| *s == section) {
            Some((_, list)) => list.push((key.clone(), title)),
            None => by_page.push((section, vec![(key.clone(), title)])),
        }
    }
    for (section, list) in by_page {
        let page_title = crate::window::section_title(&section).unwrap_or("Other options").to_string();
        let names: Vec<String> = list.iter().map(|(_, t)| glib_escape(t)).collect();
        let controls = widgets::hbox(8);
        if !section.is_empty() {
            let show = gtk::Button::with_label("Show");
            let (sec, first) = (section.clone(), list[0].1.clone());
            show.connect_clicked(move |_| crate::window::show_option(&sec, &first));
            controls.append(&show);
        }
        let keys: Vec<String> = list.iter().map(|(k, _)| k.clone()).collect();
        let sec = section.clone();
        let reset = widgets::confirm_button("Reset", "Reset all?", move |_| {
            let keys = keys.clone();
            crate::backend::store::update(true, move |s| {
                for k in &keys {
                    s.options.remove(k);
                }
            });
            crate::backend::store::flush();
            window::toast("Back to your own config and Omarchy's defaults");
            // Its page shows the old values until redrawn.
            if let Some(id) = window::section_id(&sec) {
                window::rebuild_if_built(id);
            }
            window::rebuild("about");
        });
        reset.set_tooltip_text(Some("Reset every option on this page that was changed here"));
        controls.append(&reset);
        g.add(&widgets::row(&page_title, &names.join(", "), Some(controls.upcast_ref())));
        widgets::keywords("changed modified reset defaults");
    }
    // Other things Settings manages, each kept on its own page.
    let mut others: Vec<(&str, String)> = Vec::new();
    if !st.binds.is_empty() || !st.unbinds.is_empty() {
        others.push(("keybindings", format!("{} of your own, {} turned off", st.binds.len(), st.unbinds.len())));
    }
    if !st.monitors.is_empty() {
        others.push((
            "displays",
            format!("{} display{} set up here", st.monitors.len(), if st.monitors.len() == 1 { "" } else { "s" }),
        ));
    }
    let n_startup =
        st.autostart.len() + crate::backend::autostart::list().into_iter().filter(|i| i.enabled && !i.is_system).count();
    if !st.window_rules.is_empty() || !st.layer_rules.is_empty() || n_startup > 0 {
        others.push((
            "rules",
            format!(
                "{n_startup} startup program{}, {} rules",
                if n_startup == 1 { "" } else { "s" },
                st.window_rules.len() + st.layer_rules.len()
            ),
        ));
    }
    if !st.devices.is_empty() {
        others
            .push(("mouse", format!("Settings for {} device{}", st.devices.len(), if st.devices.len() == 1 { "" } else { "s" })));
    }
    if st.gestures != Default::default() {
        others.push(("trackpad", "Swipe and pinch gestures".to_string()));
    }
    if st.max_volume.is_some_and(|v| v != 100) {
        others.push(("audio", "Maximum volume".to_string()));
    }
    let total = st.options.len();
    g.note(&match (total, others.is_empty()) {
        (0, true) => "Nothing changed yet: everything uses your own config files and Omarchy's defaults.".to_string(),
        (0, false) => "What Settings manages for you, by page.".to_string(),
        (n, _) => format!(
            "{n} option{} changed here. Reset hands them back to your own config or Omarchy's defaults.",
            if n == 1 { "" } else { "s" }
        ),
    });
    for (id, desc) in others {
        let (r, _) = widgets::button_row(window::section_title(id).unwrap_or(id), &desc, "Show", move |_| window::navigate(id));
        g.add(&r);
    }
}

/// Export everything to a file, or bring a backup in.
fn backup_group(page: &Page) {
    use crate::backend::backup;
    let g = page.group("Move your settings");
    g.note("Everything Settings keeps — its own preferences, Hyprland options, shortcuts, displays, sound and lighting — in one file.");
    let (r, _) =
        widgets::button_row("Export", "Save a backup file to keep or to bring to another computer.", "Export…", |_| {
            let dialog =
                gtk::FileDialog::builder().title("Export settings").initial_name("omarchy-settings.json").modal(true).build();
            dialog.save(window::window().as_ref(), gtk::gio::Cancellable::NONE, |res| {
                let Ok(file) = res else { return };
                let Some(path) = file.path() else { return };
                match backup::export().and_then(|text| cmd::atomic_write(&path, &text)) {
                    Ok(()) => {
                        window::toast(&format!("Saved {}", path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()))
                    }
                    Err(e) => window::toast(&format!("Couldn't export: {e:#}")),
                }
            });
        });
    widgets::keywords("export backup save copy transfer migrate");
    g.add(&r);
    let (r, _) = widgets::button_row(
        "Import",
        "Replace these settings with a backup's. Settings restarts to use them.",
        "Import…",
        |_| {
            let dialog = gtk::FileDialog::builder().title("Import settings").modal(true).build();
            dialog.open(window::window().as_ref(), gtk::gio::Cancellable::NONE, |res| {
                let Ok(file) = res else { return };
                let Some(path) = file.path() else { return };
                let files =
                    match std::fs::read_to_string(&path).map_err(anyhow::Error::from).and_then(|t| backup::read_backup(&t)) {
                        Ok(f) => f,
                        Err(e) => return window::toast(&format!("Couldn't import: {e:#}")),
                    };
                let names: Vec<String> = files.iter().map(|(n, _)| n.clone()).collect();
                crate::dialog::ask(
                    "Import these settings?",
                    &format!("This replaces {} here. Settings restarts afterwards.", names.join(", ")),
                    vec![],
                    "Import",
                    None,
                    move |_, _| {
                        if let Err(e) = backup::import(&files) {
                            return Some(format!("{e:#}"));
                        }
                        restored();
                        None
                    },
                );
            });
        },
    );
    widgets::keywords("import restore backup load transfer migrate");
    g.add(&r);
}

/// Take a restored backup into use, then restart for the rest.
fn restored() {
    use crate::backend::{audio, state::State, store};
    let fresh = State::load(&crate::paths::state_file());
    store::update(true, move |s| *s = fresh);
    store::flush();
    crate::prefs::reload();
    crate::theme::apply();
    cmd::background(
        || {
            let eq = audio::load();
            if eq.enabled { audio::install_and_start(&eq).map_err(|e| format!("{e:#}")) } else { Ok(()) }
        },
        |r| {
            if let Err(e) = r {
                window::toast(&format!("Couldn't apply the sound settings: {e}"));
            }
            window::restart("about");
        },
    );
}

pub fn build(page: &Page) {
    settings_group(page);
    changes_group(page);
    backup_group(page);
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
