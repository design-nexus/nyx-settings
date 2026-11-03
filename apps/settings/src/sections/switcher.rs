//! Switcher: Nyx Switcher's `~/.config/nyx/switcher.toml`. The switcher reads it each
//! time it opens; a changed key is bound in Hyprland by `nyx-switcher --integrate`,
//! which refuses a key another binding uses (its message is shown).

use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub fn installed() -> bool {
    cmd::present("nyx-switcher")
}

fn path() -> PathBuf {
    gtk::glib::user_config_dir().join("nyx/switcher.toml")
}

fn read() -> toml::Table {
    std::fs::read_to_string(path()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

/// Change one key (in `[keys]` when `table` is given), keeping the rest.
fn set(table: Option<&str>, key: &str, value: toml::Value) -> bool {
    let path = path();
    let mut t: toml::Table = match std::fs::read_to_string(&path) {
        Ok(text) => match toml::from_str(&text) {
            Ok(t) => t,
            Err(e) => {
                window::toast(&format!("switcher.toml can't be read: {e}"));
                return false;
            }
        },
        Err(_) => toml::Table::new(),
    };
    match table {
        Some(name) => {
            let sub = t.entry(name).or_insert_with(|| toml::Value::Table(toml::Table::new()));
            if let Some(sub) = sub.as_table_mut() {
                sub.insert(key.into(), value);
            }
        }
        None => {
            t.insert(key.into(), value);
        }
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match toml::to_string(&t).map_err(anyhow::Error::from).and_then(|s| cmd::atomic_write(&path, s)) {
        Ok(()) => true,
        Err(e) => {
            window::toast(&format!("Couldn't save: {e}"));
            false
        }
    }
}

const KEYS: &[(&str, &str, &str)] = &[
    ("windows", "Switch windows", "ALT + TAB"),
    ("windows_reverse", "Switch windows backwards", "ALT + SHIFT + TAB"),
    ("apps", "Switch apps", "SUPER + TAB"),
    ("apps_reverse", "Switch apps backwards", "SUPER + SHIFT + TAB"),
    ("same_app", "Windows of this app", "ALT + grave"),
    ("same_app_reverse", "Windows of this app backwards", "ALT + SHIFT + grave"),
    ("overview", "Show all windows", "SUPER + ALT + W"),
];

pub fn build(page: &Page) {
    let t = read();
    let s = |k: &str, d: &str| t.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string();
    let b = |k: &str, d: bool| t.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
    let opts = |o: &[(&str, &str)]| o.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();

    let g = page.group("Switching");
    let (r, _) = widgets::choice_row(
        "Windows from",
        "Which windows the switcher lists. The overview always shows them all.",
        opts(&[("all", "Every workspace"), ("monitor", "This monitor"), ("workspace", "This workspace")]),
        &s("scope", "all"),
        |v| {
            set(None, "scope", toml::Value::String(v));
        },
    );
    g.add(&r);
    let (r, _) = widgets::switch_row(
        "Window thumbnails",
        "Show each window as a thumbnail. Off: app icons only.",
        b("thumbnails", true),
        |on| {
            set(None, "thumbnails", toml::Value::Boolean(on));
        },
    );
    g.add(&r);
    let (r, _) = widgets::choice_row(
        "Thumbnail size",
        "",
        opts(&[("small", "Small"), ("medium", "Medium"), ("large", "Large")]),
        &s("thumbnail_size", "medium"),
        |v| {
            set(None, "thumbnail_size", toml::Value::String(v));
        },
    );
    g.add(&r);
    let (r, _) = widgets::switch_row(
        "One entry per group",
        "Show each group of tabbed windows once when switching windows; ↓ lists its tabs. The overview shows every tab.",
        b("collapse_groups", true),
        |on| {
            set(None, "collapse_groups", toml::Value::Boolean(on));
        },
    );
    g.add(&r);
    let delay = t.get("show_delay_ms").and_then(|v| v.as_integer()).unwrap_or(80) as f64;
    let (r, _) = widgets::slider_row(
        "Show after",
        "How long the keys are held before the switcher appears. A quicker tap switches without showing it.",
        (0.0, 400.0, 10.0),
        delay,
        0,
        " ms",
        |v| {
            set(None, "show_delay_ms", toml::Value::Integer(v.round() as i64));
        },
    );
    g.add(&r);
    let (r, _) = widgets::switch_row("Animation", "Fade the switcher in and the selection along.", b("animation", true), |on| {
        set(None, "animation", toml::Value::Boolean(on));
    });
    g.add(&r);
    let (r, _) = widgets::switch_row(
        "Agent workspaces",
        "Also list windows on workspaces that assistants work in (named nyx-agent…).",
        b("show_agent_workspaces", false),
        |on| {
            set(None, "show_agent_workspaces", toml::Value::Boolean(on));
        },
    );
    g.add(&r);
    widgets::keywords("switcher alt tab window app scope workspace monitor thumbnails previews size delay animation agent group tabs");

    let g = page.group("Touchpad");
    let current = s("gesture", "");
    let prev = Rc::new(RefCell::new(current.clone()));
    let (r, _) = widgets::choice_row(
        "Show all windows",
        "A touchpad gesture that opens the overview. One another setting already uses isn't taken.",
        opts(&[
            ("", "No gesture"),
            ("4 pinchout", "Spread four fingers"),
            ("5 up", "Five fingers up"),
            ("5 down", "Five fingers down"),
        ]),
        &current,
        move |v| {
            let old = prev.replace(v.clone());
            if set(None, "gesture", toml::Value::String(v)) {
                let prev = prev.clone();
                bind(move || {
                    set(None, "gesture", toml::Value::String(old.clone()));
                    prev.replace(old.clone());
                    window::rebuild("switcher");
                });
            }
        },
    );
    g.add(&r);
    widgets::keywords("gesture swipe touchpad trackpad fingers overview");

    let g = page.group("Keys");
    let keys = t.get("keys").and_then(|v| v.as_table()).cloned().unwrap_or_default();
    for (key, title, default) in KEYS {
        let current = keys.get(*key).and_then(|v| v.as_str()).unwrap_or(default).to_string();
        let k = *key;
        let bound = Rc::new(RefCell::new(current.clone()));
        let field: Rc<RefCell<Option<glib::WeakRef<gtk::Entry>>>> = Rc::default();
        let f = field.clone();
        let (r, entry) = widgets::entry_row(title, "", &current, "Not bound", move |v| {
            let v = v.trim().to_string();
            let old = bound.replace(v.clone());
            if !set(Some("keys"), k, toml::Value::String(v)) {
                return;
            }
            let (bound, f) = (bound.clone(), f.clone());
            bind(move || {
                // Refused: the file and the field go back to the key that's bound.
                set(Some("keys"), k, toml::Value::String(old.clone()));
                bound.replace(old.clone());
                if let Some(e) = f.borrow().as_ref().and_then(|w| w.upgrade()) {
                    e.set_text(&old);
                }
            });
        });
        *field.borrow_mut() = Some(entry.downgrade());
        g.add(&r);
    }
    g.note("Keys as Hyprland writes them, like ALT + TAB; leave one empty to unbind it. A key another binding uses isn't taken. Omarchy's Alt+Tab and Super+Tab come back when the switcher is removed.");
    widgets::keywords("keys shortcut keybinding super tab alt grave overview conflict");
}

/// Bind the keys from the file; show why not when Hyprland or a conflict says no,
/// and run `undo`.
fn bind(undo: impl FnOnce() + 'static) {
    cmd::background(
        || std::process::Command::new("nyx-switcher").arg("--integrate").output(),
        |r| match r {
            Ok(out) if out.status.success() => window::toast("Keys updated"),
            Ok(out) => {
                let msg = String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("nyx-switcher: ").to_string();
                window::toast(&format!("Keys not changed: {msg}"));
                undo();
            }
            Err(e) => {
                window::toast(&format!("nyx-switcher: {e}"));
                undo();
            }
        },
    );
}
