//! NyxOS's Lock Screen: when the screen locks, turns off and sleeps (the idle settings,
//! which hypridle reads when it starts), how Nyx Locker looks (`appearance`'s
//! `lock-style`), and staying awake for a while. All of it is the user's own, through
//! `nyx-system config`; no password is asked for.

use crate::sections::idle::{minutes_label, timeline};
use crate::sketch::Sketch;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// (key, title, description, default seconds, longest minutes, timeline name)
const TIMES: [(&str, &str, &str, i64, f64, &str); 3] = [
    ("lock", "Lock the screen after", "0 never locks on its own.", 300, 60.0, "Lock"),
    ("screen-off", "Turn the screen off after", "0 leaves it on.", 330, 60.0, "Screen off"),
    ("suspend", "Sleep after", "0 never sleeps on its own; closing the lid still can.", 0, 120.0, "Sleep"),
];

const STYLES: [(&str, &str); 4] =
    [("circular", "Circular"), ("cyberpunk", "Cyberpunk"), ("minimal", "Minimal"), ("edge", "Edge glow")];

/// The transient unit that holds an idle inhibitor while Stay awake is on.
const AWAKE_UNIT: &str = "nyx-stay-awake";

fn get(domain: &str, key: &str) -> Option<String> {
    cmd::output(&["nyx-system", "config", "get", domain, key]).map(|s| s.trim().to_string())
}

fn set(domain: &str, key: &str, value: &str) -> anyhow::Result<()> {
    cmd::run(&["nyx-system", "config", "set", domain, key, value]).map(|_| ())
}

/// Save one time and restart hypridle to read it; held back while a slider moves.
fn save_later(pending: &Rc<RefCell<Option<glib::SourceId>>>, key: &'static str, secs: i64) {
    if let Some(id) = pending.borrow_mut().take() {
        id.remove();
    }
    let p = pending.clone();
    let id = glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
        p.borrow_mut().take();
        match set("idle", key, &secs.to_string()) {
            Ok(()) => cmd::spawn(&["systemctl", "--user", "try-restart", "hypridle.service"]),
            Err(e) => window::toast(&format!("Couldn't save: {e}")),
        }
    });
    *pending.borrow_mut() = Some(id);
}

fn awake() -> bool {
    cmd::output(&["systemctl", "--user", "is-active", &format!("{AWAKE_UNIT}.service")]).is_some_and(|s| s.trim() == "active")
}

fn set_awake(on: bool) {
    if on {
        cmd::spawn(&[
            "systemd-run",
            "--user",
            "--collect",
            &format!("--unit={AWAKE_UNIT}"),
            "--description=Stay awake (Nyx Settings)",
            "systemd-inhibit",
            "--what=idle",
            "--who=Nyx Settings",
            "--why=Stay awake is on",
            "sleep",
            "infinity",
        ]);
    } else {
        cmd::spawn(&["systemctl", "--user", "stop", &format!("{AWAKE_UNIT}.service")]);
    }
}

/// The ways in besides the password, from Nyx Locker's configuration.
fn ways_in() -> Vec<String> {
    let text = std::fs::read_to_string("/etc/nyx-locker/config.toml").unwrap_or_default();
    let methods: Vec<String> = text
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("methods")?.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    methods
        .iter()
        .filter_map(|m| match m.as_str() {
            "fingerprint" => Some("a fingerprint".to_string()),
            "face" => Some("your face".to_string()),
            "key" => Some("a security key".to_string()),
            _ => None,
        })
        .collect()
}

pub fn build(page: &Page) {
    let g = page.group("When you're away");
    let secs: Rc<Vec<Cell<i64>>> = Rc::new(
        TIMES
            .iter()
            .map(|(key, _, _, default, _, _)| {
                Cell::new(get("idle", key).and_then(|v| v.parse().ok()).filter(|v: &i64| *v >= 0).unwrap_or(*default))
            })
            .collect(),
    );
    let s = secs.clone();
    let line = timeline(move || TIMES.iter().zip(s.iter()).map(|(t, v)| (v.get(), t.5)).collect());
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("preview-card");
    card.append(&line);
    g.add(&card);
    for (i, (key, title, desc, _, most, _)) in TIMES.iter().enumerate() {
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        let (s, l) = (secs.clone(), line.clone());
        let start = secs[i].get();
        let (r, slider) = widgets::slider_row(title, desc, (0.0, *most, 1.0), (start / 60) as f64, 0, " min", move |v| {
            let v = (v.round() as i64) * 60;
            s[i].set(v);
            l.queue_draw();
            save_later(&pending, key, v);
        });
        let readout = slider.readout.clone();
        readout.set_text(&minutes_label(start));
        slider.scale.connect_value_changed(move |sc| readout.set_text(&minutes_label(sc.value().round() as i64 * 60)));
        g.add(&r);
    }
    widgets::keywords("idle timeout minutes screen off dpms sleep suspend lock");

    let g = page.group("Look");
    let current =
        get("appearance", "lock-style").filter(|s| STYLES.iter().any(|(id, _)| id == s)).unwrap_or_else(|| "circular".into());
    let chosen = Rc::new(RefCell::new(current.clone()));
    let c = chosen.clone();
    let (r, _, _) = widgets::visual_choice(
        "Sign-in animation",
        "What the lock screen shows while it checks your password, fingerprint or face.",
        STYLES.iter().map(|(id, name)| (id.to_string(), name.to_string(), Sketch::LockStyle(id))).collect(),
        &current,
        move |id| match set("appearance", "lock-style", &id) {
            Ok(()) => *c.borrow_mut() = id,
            Err(e) => window::toast(&format!("Couldn't save: {e}")),
        },
    );
    g.add(&r);
    let c = chosen.clone();
    let (r, _) = widgets::button_row(
        "Preview",
        "Opens the lock screen in a window, without locking; the password there is “nyx”.",
        "Preview",
        move |_| cmd::spawn(&["nyx-locker", "--demo", "--style", &c.borrow()]),
    );
    g.add(&r);
    let mut ways = ways_in();
    let desc = match ways.pop() {
        None => "Your password. Sign-in sets up a fingerprint, your face or a security key.".to_string(),
        Some(last) => {
            ways.insert(0, "Your password".into());
            format!("{} or {last}, whichever you use first.", ways.join(", "))
        }
    };
    let (r, _) = widgets::button_row("Ways to unlock", &desc, "Sign-in", |_| window::navigate("sign-in"));
    widgets::keywords("lock style animation fingerprint face biometrics");
    g.add(&r);

    let g = page.group("Right now");
    let (r, _) = widgets::switch_row(
        "Stay awake",
        "Keep the screen on and unlocked until you turn this off or sign out — for presentations and long downloads.",
        awake(),
        set_awake,
    );
    widgets::keywords("caffeine inhibit presentation");
    g.add(&r);
    let (r, _) = widgets::button_row("Lock now", "", "Lock", |_| cmd::spawn(&["loginctl", "lock-session"]));
    g.add(&r);
}
