//! Background services (systemd units), for you and for the whole system.
//! System changes go through the root helper; your own services need no password.

use crate::backend::accounts as acc;
use crate::sections::accounts::require_helper;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

/// Where the list's own redraw function is kept, weakly so it can refer to itself.
type Slot = Rc<std::cell::RefCell<Option<Weak<dyn Fn()>>>>;

#[derive(Debug, PartialEq, Clone)]
pub struct Unit {
    pub name: String,
    pub active: String,
    pub sub: String,
    pub description: String,
    /// `enabled`, `disabled`, `static`… when the unit has an install state.
    pub enabled: String,
}

/// Join `systemctl list-units` (UNIT LOAD ACTIVE SUB DESCRIPTION…) with
/// `list-unit-files` (UNIT STATE). Only real services are kept.
pub fn parse_units(units: &str, files: &str) -> Vec<Unit> {
    let states: HashMap<&str, &str> = files
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some((f.next()?, f.next()?))
        })
        .collect();
    let mut out: Vec<Unit> = units
        .lines()
        .filter_map(|l| {
            let l = l.trim_start_matches(['●', '○', '×', '*', ' ']);
            let mut f = l.split_whitespace();
            let name = f.next()?;
            let load = f.next()?;
            let active = f.next()?;
            let sub = f.next()?;
            if !name.ends_with(".service") || load == "not-found" || name.contains("@.") {
                return None;
            }
            let description = f.collect::<Vec<_>>().join(" ");
            let enabled = states.get(name).copied().unwrap_or("").to_string();
            Some(Unit { name: name.to_string(), active: active.to_string(), sub: sub.to_string(), description, enabled })
        })
        .collect();
    // Running first, then by name.
    out.sort_by(|a, b| (b.active == "active").cmp(&(a.active == "active")).then(a.name.cmp(&b.name)));
    out
}

const UNITS: &[&str] = &["list-units", "--type=service", "--all", "--plain", "--no-legend", "--no-pager"];
const UNIT_FILES: &[&str] = &["list-unit-files", "--type=service", "--plain", "--no-legend", "--no-pager"];

/// What the page asks systemctl while being built, for [`cmd::fetch_ahead`].
pub fn reads() -> Vec<Vec<&'static str>> {
    [&["systemctl", "--user"][..], &["systemctl"][..]]
        .iter()
        .flat_map(|base| [UNITS, UNIT_FILES].map(|extra| [*base, extra].concat()))
        .collect()
}

fn list(user: bool) -> Vec<Unit> {
    let base: &[&str] = if user { &["systemctl", "--user"] } else { &["systemctl"] };
    let run = |extra: &[&str]| cmd::read(&[base, extra].concat()).unwrap_or_default();
    parse_units(&run(UNITS), &run(UNIT_FILES))
}

pub fn build(page: &Page) {
    let can_edit = require_helper(page, "services", "starting and stopping system services");
    let mine = list(true);
    let system = list(false);

    let failed: Vec<(bool, Unit)> = mine
        .iter()
        .filter(|u| u.active == "failed")
        .map(|u| (true, u.clone()))
        .chain(system.iter().filter(|u| u.active == "failed").map(|u| (false, u.clone())))
        .collect();
    if !failed.is_empty() {
        let g = page.group("Needs attention");
        g.note("These stopped because of an error.");
        for (user, u) in &failed {
            let refill: Rc<dyn Fn()> = Rc::new(|| window::rebuild("services"));
            g.add(&unit_row(u, *user, can_edit, refill));
        }
    }
    section(page, "Your services", "Run as you, started when you sign in.", true, mine, true);
    section(page, "System services", "Run for the whole computer.", false, system, can_edit);
}

/// NyxOS: your own services, and the system's to look at (they're turned on and off as
/// system settings).
pub fn user_and_system_lists(page: &Page) {
    section(page, "Your services", "Run as you, started when you sign in.", true, list(true), true);
    section(page, "Running on this computer", "The whole computer's services, for reference.", false, list(false), false);
}

fn section(page: &Page, title: &str, note: &str, user: bool, units: Vec<Unit>, can_edit: bool) {
    // The system list is long and rarely needed: it folds away.
    let g = if user { page.group(title) } else { page.collapsible(title, false) };
    g.note(note);
    let results = widgets::vbox(6);
    let units = Rc::new(std::cell::RefCell::new(units));
    let query = Rc::new(std::cell::RefCell::new(String::new()));
    let refill: Slot = Rc::default();

    let fill: Rc<dyn Fn()> = {
        let (results, units, query, refill) = (results.clone(), units.clone(), query.clone(), refill.clone());
        Rc::new(move || {
            // The list is redrawn after the page was built: drop the old rows from
            // search and file the new ones under this page.
            widgets::forget_rows(&results);
            widgets::begin_section("services");
            while let Some(c) = results.first_child() {
                results.remove(&c);
            }
            let q = query.borrow().to_lowercase();
            let shown: Vec<Unit> = units
                .borrow()
                .iter()
                .filter(|u| q.is_empty() || u.name.to_lowercase().contains(&q) || u.description.to_lowercase().contains(&q))
                .take(20)
                .cloned()
                .collect();
            if shown.is_empty() {
                results.append(&widgets::label("No services match.", "dim"));
            }
            let again: Rc<dyn Fn()> = {
                let (units, refill) = (units.clone(), refill.clone());
                Rc::new(move || {
                    // Read the units again off the main thread, then redraw the list.
                    let (units, refill) = (units.clone(), refill.clone());
                    cmd::background(
                        move || list(user),
                        move |fresh| {
                            *units.borrow_mut() = fresh;
                            if let Some(f) = refill.borrow().as_ref().and_then(Weak::upgrade) {
                                f();
                            }
                        },
                    );
                })
            };
            for u in shown {
                results.append(&unit_row(&u, user, can_edit, again.clone()));
            }
        })
    };
    *refill.borrow_mut() = Some(Rc::downgrade(&fill));

    let (r, _) = widgets::entry_row("Find a service", "Part of a name or description. Press Enter.", "", "e.g. bluetooth", {
        let (query, fill) = (query.clone(), fill.clone());
        move |q| {
            *query.borrow_mut() = q.trim().to_string();
            fill();
        }
    });
    widgets::keywords("service daemon systemd unit start stop restart enable disable");
    g.add(&r);
    g.add(&results);
    fill();
}

/// One service: its state, and buttons that fit it.
fn unit_row(u: &Unit, user: bool, can_edit: bool, refill: Rc<dyn Fn()>) -> gtk::Box {
    let running = u.active == "active";
    let controls = widgets::hbox(6);
    if can_edit {
        let act = |label: &str, action: &'static str, unit: String, refill: Rc<dyn Fn()>| {
            let b = gtk::Button::with_label(label);
            b.connect_clicked(move |b| {
                b.set_sensitive(false);
                let (unit, refill) = (unit.clone(), refill.clone());
                cmd::background(
                    move || {
                        if user {
                            cmd::run(&["systemctl", "--user", action, &unit]).map(|_| ()).map_err(|e| e.to_string())
                        } else {
                            acc::helper(&["service", action, &unit], None).map(|_| ()).map_err(|e| e.to_string())
                        }
                    },
                    move |r| {
                        if let Err(e) = r {
                            window::toast(&e);
                        }
                        refill();
                    },
                );
            });
            b
        };
        controls.append(&act(
            if running { "Stop" } else { "Start" },
            if running { "stop" } else { "start" },
            u.name.clone(),
            refill.clone(),
        ));
        if running {
            controls.append(&act("Restart", "restart", u.name.clone(), refill.clone()));
        }
        match u.enabled.as_str() {
            "enabled" => controls.append(&act("Disable", "disable", u.name.clone(), refill.clone())),
            "disabled" => controls.append(&act("Enable", "enable", u.name.clone(), refill.clone())),
            _ => {}
        }
    }
    let state = if u.active == "failed" {
        "failed".to_string()
    } else if running {
        "running".to_string()
    } else {
        "stopped".to_string()
    };
    let boot = match u.enabled.as_str() {
        "enabled" => " · starts at boot",
        _ => "",
    };
    let desc = format!("{}{}", gtk::glib::markup_escape_text(&u.description), boot);
    let r = widgets::row(u.name.trim_end_matches(".service"), &desc, Some(controls.upcast_ref()));
    widgets::tag_row(&r, &state);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNITS: &str = "alsa-restore.service loaded active exited Save/Restore Sound Card State\nbluetooth.service loaded active running Bluetooth service\nfoo.service not-found inactive dead foo.service\n● bad.service loaded failed failed A broken one\ntemplate@.service loaded inactive dead x\nsome.socket loaded active running A socket\n";
    const FILES: &str = "alsa-restore.service static -\nbluetooth.service enabled disabled\nbad.service disabled -\n";

    #[test]
    fn parses_and_sorts() {
        let u = parse_units(UNITS, FILES);
        let names: Vec<&str> = u.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["alsa-restore.service", "bluetooth.service", "bad.service"]);
        assert_eq!(u[1].enabled, "enabled");
        assert_eq!(u[1].description, "Bluetooth service");
        assert_eq!(u[2].active, "failed");
        assert_eq!(u[0].enabled, "static");
    }
}
