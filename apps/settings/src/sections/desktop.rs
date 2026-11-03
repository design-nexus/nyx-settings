//! Desktop: Nyx Dock, Control Center and Notification Center. Each page changes its
//! program the way the program itself does (its config file, its own commands, its
//! service), and only shows when the program is installed.

use crate::backend::{notifyd, shell};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

fn config(name: &str) -> PathBuf {
    gtk::glib::user_config_dir().join("nyx").join(name)
}

/// Run a command in the background; say so if it fails.
fn run(args: &[&str], done: impl FnOnce() + 'static) {
    let label = args[0].to_string();
    cmd::run_async(args, move |r| match r {
        Ok(_) => done(),
        Err(e) => window::toast(&format!("{label}: {e}")),
    });
}

fn service_enabled(unit: &str) -> bool {
    cmd::output(&["systemctl", "--user", "is-enabled", unit]).is_some_and(|s| s.trim() == "enabled")
}

/// A bar widget is on the bar (in any section of the layout in shell.json).
fn on_bar(id: &str) -> bool {
    let v = shell::read();
    v["bar"]["layout"]
        .as_object()
        .is_some_and(|sections| sections.values().filter_map(|l| l.as_array()).flatten().any(|e| e["id"] == id))
}

/// A switch for one of our bar widgets: on adds it to the right of the bar.
fn bar_switch(id: &'static str, title: &str, desc: &str) -> gtk::Box {
    let (r, _) = widgets::switch_row(title, desc, on_bar(id), move |on| {
        if on {
            run(&["omarchy-plugin-enable", id, "right"], || {});
        } else {
            run(&["omarchy-plugin-disable", id], || {});
        }
    });
    r
}

// ---------------------------------------------------------------- Dock

pub fn dock_installed() -> bool {
    cmd::present("nyx-dock")
}

/// Change one key in `dock.toml`, keeping the rest (the dock reloads it).
pub fn set_dock(key: &str, value: toml::Value) {
    let path = config("dock.toml");
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let mut t: toml::Table = match toml::from_str(&text) {
        Ok(t) => t,
        Err(e) => {
            window::toast(&format!("dock.toml can't be read: {e}"));
            return;
        }
    };
    t.insert(key.into(), value);
    match toml::to_string(&t) {
        Ok(s) => {
            if let Err(e) = cmd::atomic_write(&path, s) {
                window::toast(&format!("Couldn't save: {e}"));
            }
        }
        Err(e) => window::toast(&format!("Couldn't save: {e}")),
    }
}

pub fn build_dock(page: &Page) {
    let g = page.group("Dock");
    let on = service_enabled("nyx-dock.service");
    let (r, _) = widgets::switch_row(
        "Use Nyx Dock",
        "Show the Nyx Dock instead of Omarchy's, now and at login. Off gives Omarchy's dock back.",
        on,
        |on| {
            run(&["nyx-dock", if on { "--enable" } else { "--disable" }], || {
                // The dock writes its settings on first start; show them once it has.
                gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(2), || window::rebuild("dock"));
            });
        },
    );
    g.add(&r);
    widgets::keywords("dock launcher taskbar omarchy replace");

    let file: Option<toml::Table> = std::fs::read_to_string(config("dock.toml")).ok().and_then(|t| toml::from_str(&t).ok());
    let g = page.group("Look and behaviour");
    let Some(t) = file else {
        g.note("These appear once the Nyx Dock has started for the first time.");
        return;
    };
    let s = |k: &str, d: &str| t.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string();
    let b = |k: &str, d: bool| t.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
    let opts = |o: &[(&str, &str)]| o.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();

    let (r, _) = widgets::choice_row(
        "Position",
        "The edge of the screen it sits on.",
        opts(&[("bottom", "Bottom"), ("left", "Left"), ("right", "Right")]),
        &s("position", "bottom"),
        |v| set_dock("position", toml::Value::String(v)),
    );
    g.add(&r);
    let size = t.get("icon_size").and_then(|v| v.as_integer()).unwrap_or(48) as f64;
    let (r, _) = widgets::slider_row("Icon size", "", (32.0, 80.0, 4.0), size, 0, " px", |v| {
        set_dock("icon_size", toml::Value::Integer(v.round() as i64))
    });
    g.add(&r);
    let (r, _) = widgets::choice_row(
        "Hide",
        "Smart: hide while a window covers it. Automatically: until the pointer reaches the edge.",
        opts(&[("smart", "Smart"), ("always", "Automatically"), ("never", "Never")]),
        &s("hide", "smart"),
        |v| set_dock("hide", toml::Value::String(v)),
    );
    g.add(&r);
    let (r, _) = widgets::choice_row(
        "Running apps",
        "With more than one screen: every running app on each, or only those with a window there.",
        opts(&[("all", "On every screen"), ("monitor", "On their own screen")]),
        &s("running", "all"),
        |v| set_dock("running", toml::Value::String(v)),
    );
    g.add(&r);
    let (r, _) = widgets::switch_row(
        "Window previews",
        "Rest the pointer on a running app to see its windows.",
        b("previews", true),
        |on| set_dock("previews", toml::Value::Boolean(on)),
    );
    g.add(&r);
    let (r, _) = widgets::switch_row("Trash", "Keep the Trash at the end of the dock.", b("trash", true), |on| {
        set_dock("trash", toml::Value::Boolean(on))
    });
    g.add(&r);
    widgets::keywords("position size hide autohide previews thumbnails trash running monitor screen");
    g.note("Pin apps and add folders from the dock itself: right-click an app or a folder.");
}

// ---------------------------------------------------------------- Control Center

pub fn control_center_installed() -> bool {
    cmd::present("nyx-control-center")
}

pub fn build_control_center(page: &Page) {
    let g = page.group("Control Center");
    g.add(&widgets::row(
        "Open it",
        "<tt>Super D</tt>, or the bar button. Change the tiles with the pencil at the top of the panel.",
        None,
    ));
    let path = config("control-center.toml");
    let file: toml::Table = std::fs::read_to_string(&path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default();
    let remember = file.get("remember_page").and_then(|v| v.as_bool()).unwrap_or(true);
    let p2 = path.clone();
    let (r, _) = widgets::switch_row("Open on the last page", "Off: it always opens on the first page.", remember, move |on| {
        let mut t: toml::Table = std::fs::read_to_string(&p2).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default();
        t.insert("remember_page".into(), toml::Value::Boolean(on));
        match toml::to_string(&t).map_err(anyhow::Error::from).and_then(|s| cmd::atomic_write(&p2, s)) {
            // It reads its layout when it starts; it's hidden, so a restart is unseen.
            Ok(()) => run(&["systemctl", "--user", "try-restart", "nyx-control-center.service"], || {}),
            Err(e) => window::toast(&format!("Couldn't save: {e}")),
        }
    });
    g.add(&r);
    let reset = widgets::confirm_button("Restore", "Click again to restore", move |_| {
        let mut t: toml::Table = std::fs::read_to_string(&path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default();
        t.remove("pages");
        match toml::to_string(&t).map_err(anyhow::Error::from).and_then(|s| cmd::atomic_write(&path, s)) {
            Ok(()) => run(&["systemctl", "--user", "try-restart", "nyx-control-center.service"], || {
                window::toast("The Control Center has its standard tiles again")
            }),
            Err(e) => window::toast(&format!("Couldn't restore: {e}")),
        }
    });
    g.add(&widgets::row("Standard layout", "Put back the standard tiles and pages.", Some(reset.upcast_ref())));
    widgets::keywords("control center quick settings tiles layout pages reset super d");

    let g = page.group("In the bar");
    g.add(&bar_switch(
        "design-nexus.status",
        "Nyx Status",
        "The weather, CPU and memory, and the network, Bluetooth, sound and battery at a glance.",
    ));
    g.add(&bar_switch("design-nexus.control-center", "Control Center button", "One icon that opens and closes the panel."));
    g.note("What Nyx Status shows is set under Bar Plugins.");
    widgets::keywords("bar status widget weather cpu memory battery button");
}

// ---------------------------------------------------------------- Notifications

pub fn notification_center_installed() -> bool {
    cmd::present("nyx-notification-center")
}

pub fn build_notifications(page: &Page) {
    let g = page.group("Notification Center");
    g.add(&widgets::row(
        "Open it",
        "<tt>Super N</tt>, or the bell in the bar. It keeps an Inbox, pinned notifications and the history.",
        None,
    ));
    let Ok(settings) = notifyd::settings() else {
        g.note(
            "The notification service (nyx-notifyd) isn't running. Start it with <tt>systemctl --user start nyx-notifyd</tt>.",
        );
        return;
    };
    let nyx = notifyd::mode().as_deref() == Some("nyx");
    let (r, _) = widgets::choice_row(
        "Pop-ups",
        "Who receives notifications and shows them as they arrive. History is kept either way.",
        vec![("omarchy".into(), "Omarchy".into()), ("nyx".into(), "Nyx Notification Center".into())],
        if nyx { "nyx" } else { "omarchy" },
        |v| {
            let arg = if v == "nyx" { "--use-nyx-popups" } else { "--use-omarchy-popups" };
            window::toast("Switching who shows notifications…");
            run(&["nyx-notification-center", arg], || {
                window::toast("Done");
                window::rebuild("notifications");
            });
        },
    );
    g.add(&r);
    g.add(&bar_switch(
        "design-nexus.notification-center",
        "Bell in the bar",
        "Shows unread notifications; right-click for Do not disturb.",
    ));
    widgets::keywords("notification center popups toasts omarchy inbox bell super n");

    let state = Rc::new(RefCell::new(settings));
    let save = {
        let state = state.clone();
        Rc::new(move || {
            let s = state.borrow().clone();
            cmd::background(
                move || notifyd::set_settings(&s),
                |r| {
                    if let Err(e) = r {
                        window::toast(&format!("Couldn't save: {e}"));
                    }
                },
            );
        })
    };

    let g = page.group("Do not disturb on a schedule");
    let sched_on = state.borrow()["schedule"]["enabled"].as_bool().unwrap_or(false);
    let times = widgets::vbox(8);
    times.set_sensitive(sched_on);
    let (s2, sv, t2) = (state.clone(), save.clone(), times.clone());
    let (r, _) = widgets::switch_row(
        "Turn on automatically",
        "Quiet every night; critical notifications and sign-in codes still show. Switching it by hand in between sticks.",
        sched_on,
        move |on| {
            s2.borrow_mut()["schedule"]["enabled"] = on.into();
            t2.set_sensitive(on);
            sv();
        },
    );
    g.add(&r);
    for (label, key, default) in [("Starts at", "from", 22 * 60), ("Ends at", "to", 7 * 60)] {
        let current = notifyd::hhmm(state.borrow()["schedule"][key].as_u64().unwrap_or(default));
        let (s2, sv) = (state.clone(), save.clone());
        let (r, e) =
            widgets::entry_row(label, "24-hour time, like 22:30.", &current, "HH:MM", move |t| match notifyd::minutes(&t) {
                Some(m) => {
                    s2.borrow_mut()["schedule"][key] = m.into();
                    sv();
                }
                None => window::toast("Use a 24-hour time like 22:30"),
            });
        e.set_width_chars(8);
        e.add_css_class("mono");
        times.append(&r);
    }
    g.add(&times);
    widgets::keywords("dnd quiet hours schedule night silence");

    let g = page.group("History");
    let days = state.borrow()["retention_days"].as_u64().unwrap_or(30).to_string();
    let (s2, sv) = (state.clone(), save.clone());
    let (r, _) = widgets::choice_row(
        "Keep for",
        "Pinned notifications are always kept.",
        vec![("7".into(), "7 days".into()), ("30".into(), "30 days".into()), ("90".into(), "90 days".into())],
        &days,
        move |v| {
            s2.borrow_mut()["retention_days"] = v.parse::<u64>().unwrap_or(30).into();
            sv();
        },
    );
    g.add(&r);
    let del = widgets::confirm_button("Delete", "Click again to delete", |_| {
        cmd::background(notifyd::delete_history, |r| match r {
            Ok(()) => window::toast("History deleted; pinned notifications kept"),
            Err(e) => window::toast(&format!("Couldn't delete: {e}")),
        });
    });
    g.add(&widgets::row("Delete history", "Everything except pinned notifications.", Some(del.upcast_ref())));
    widgets::keywords("history retention days delete clear pinned");

    if nyx {
        let g = page.group("Pop-ups");
        let shown = !state.borrow()["hide_popup_text"].as_bool().unwrap_or(false);
        let (s2, sv) = (state.clone(), save.clone());
        let (r, _) = widgets::switch_row("Show text in pop-ups", "Off: only the app and the title show.", shown, move |on| {
            s2.borrow_mut()["hide_popup_text"] = (!on).into();
            sv();
        });
        g.add(&r);
        widgets::keywords("privacy preview text popups content");
    }

    let apps = notifyd::apps();
    if !apps.is_empty() {
        let g = page.collapsible("Apps", false);
        g.note(if nyx {
            "Turn off pop-ups to have an app's notifications arrive quietly in the Inbox; turn off history to keep none of them."
        } else {
            "Turn off history to keep none of an app's notifications."
        });
        for a in apps {
            let app = a["app"].as_str().unwrap_or_default().to_string();
            let name = notifyd::app_name(&app);
            let switches = widgets::hbox(12);
            if nyx {
                let pop = gtk::Switch::new();
                pop.set_valign(gtk::Align::Center);
                pop.set_tooltip_text(Some("Pop-ups"));
                let quiet = state.borrow()["no_popups"].as_array().is_some_and(|l| l.iter().any(|x| x == app.as_str()));
                pop.set_active(!quiet);
                let (s2, sv, app2) = (state.clone(), save.clone(), app.clone());
                pop.connect_state_set(move |_, on| {
                    edit_list(&mut s2.borrow_mut(), "no_popups", &app2, !on);
                    sv();
                    gtk::glib::Propagation::Proceed
                });
                switches.append(&gtk::Label::new(Some("Pop-ups")));
                switches.append(&pop);
            }
            let hist = gtk::Switch::new();
            hist.set_valign(gtk::Align::Center);
            hist.set_tooltip_text(Some("History"));
            hist.set_active(!a["excluded"].as_bool().unwrap_or(false));
            let (s2, sv, app2) = (state.clone(), save.clone(), app.clone());
            hist.connect_state_set(move |_, on| {
                edit_list(&mut s2.borrow_mut(), "excluded", &app2, !on);
                sv();
                gtk::glib::Propagation::Proceed
            });
            switches.append(&gtk::Label::new(Some("History")));
            switches.append(&hist);
            let count = a["count"].as_i64().unwrap_or(0);
            let desc = if count == 1 { "1 notification".to_string() } else { format!("{count} notifications") };
            g.add(&widgets::row(&name, &desc, Some(switches.upcast_ref())));
        }
        widgets::keywords("apps per app mute quiet exclude history popups");
    }
}

/// Add `app` to (or take it out of) the settings list `key`.
fn edit_list(s: &mut serde_json::Value, key: &str, app: &str, present: bool) {
    let mut list: Vec<String> =
        s[key].as_array().map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    list.retain(|x| x != app);
    if present {
        list.push(app.to_string());
    }
    s[key] = list.into();
}

#[cfg(test)]
mod tests {
    #[test]
    fn lists() {
        let mut s = serde_json::json!({ "excluded": ["A"] });
        super::edit_list(&mut s, "excluded", "B", true);
        super::edit_list(&mut s, "excluded", "A", false);
        super::edit_list(&mut s, "no_popups", "C", true);
        assert_eq!(s, serde_json::json!({ "excluded": ["B"], "no_popups": ["C"] }));
    }
}
