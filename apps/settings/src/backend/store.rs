//! The in-memory copy of [`State`], with debounced persistence.
//!
//! Changes apply live straight away (where Hyprland allows it) and are written to
//! `state.json` + `hypr/settings.lua` shortly after the last change, so dragging
//! a slider doesn't rewrite files on every tick.

use super::hypr;
use super::state::State;
use crate::{cmd, paths};
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type Listener = Rc<dyn Fn(&str, Option<&Value>)>;

thread_local! {
    /// Told about option changes under a prefix (live previews).
    static LISTENERS: RefCell<Vec<(glib::WeakRef<gtk::Widget>, String, Listener)>> = const { RefCell::new(Vec::new()) };
    static STATE: RefCell<State> = RefCell::new(State::load(&paths::state_file()));
    static PENDING: Cell<Option<glib::SourceId>> = const { Cell::new(None) };
    static RELOAD_AFTER: Cell<bool> = const { Cell::new(false) };
    /// The workspace profile this copy of the state belongs to.
    static PROFILE_SEEN: RefCell<Option<String>> = RefCell::new(super::profiles::store().active());
}

/// Take the workspaces (and profile rules) from disk, after a workspace profile was
/// switched to outside this window (`nyx-settings --workspace-profile`).
pub fn reload_workspaces() {
    let disk = State::load(&paths::state_file());
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.persistent_workspaces = disk.persistent_workspaces;
        s.workspaces = disk.workspaces;
        s.profile_rules = disk.profile_rules;
    });
    PROFILE_SEEN.with(|p| *p.borrow_mut() = super::profiles::store().active());
}

pub fn read<T>(f: impl FnOnce(&State) -> T) -> T {
    STATE.with(|s| f(&s.borrow()))
}

/// Change state; `reload` asks Hyprland to re-read its config after writing
/// (needed for gestures, binds and monitors, which can't be applied live).
pub fn update(reload: bool, f: impl FnOnce(&mut State)) {
    STATE.with(|s| f(&mut s.borrow_mut()));
    if reload {
        RELOAD_AFTER.with(|r| r.set(true));
    }
    schedule();
}

fn schedule() {
    PENDING.with(|p| {
        if let Some(id) = p.take() {
            id.remove();
        }
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(450), || {
            PENDING.with(|p| p.set(None));
            flush();
        });
        p.set(Some(id));
    });
}

/// Write everything now.
pub fn flush() {
    PENDING.with(|p| {
        if let Some(id) = p.take() {
            id.remove();
        }
    });
    // Another profile became active meanwhile: its workspaces win over this copy's.
    if PROFILE_SEEN.with(|p| *p.borrow() != super::profiles::store().active()) {
        reload_workspaces();
    }
    let state = read(State::clone);
    if let Ok(text) = serde_json::to_string_pretty(&state)
        && let Err(e) = cmd::atomic_write(&paths::state_file(), &text)
    {
        eprintln!("nyx-settings: {e:#}");
    }
    if let Err(e) = hypr::write(&state) {
        eprintln!("nyx-settings: {e:#}");
        crate::window::toast(&format!("Couldn't write Hyprland settings: {e}"));
    }
    super::profiles::sync_active(&state);
    if RELOAD_AFTER.with(|r| r.replace(false)) {
        hypr::reload();
    }
}

// ----- Hyprland options -----

pub fn option(key: &str) -> Option<Value> {
    read(|s| s.options.get(key).cloned())
}

pub fn is_managed(key: &str) -> bool {
    read(|s| s.options.contains_key(key))
}

/// While `owner` lives, call `f` with each option under `prefix` as it
/// changes: the new value, or `None` when it was reset (re-read it after a
/// reload).
pub fn subscribe(owner: &impl IsA<gtk::Widget>, prefix: &str, f: impl Fn(&str, Option<&Value>) + 'static) {
    let weak = owner.upcast_ref::<gtk::Widget>().downgrade();
    LISTENERS.with(|l| l.borrow_mut().push((weak, prefix.to_string(), Rc::new(f))));
}

fn notify(key: &str, value: Option<&Value>) {
    let listeners: Vec<Listener> = LISTENERS.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|(w, _, _)| w.upgrade().is_some());
        l.iter().filter(|(_, p, _)| key.starts_with(p.as_str())).map(|(_, _, f)| f.clone()).collect()
    });
    for f in listeners {
        f(key, value);
    }
}

/// Set a Hyprland option: apply it live, remember it, write soon.
pub fn set_option(key: &str, value: Value) {
    if let Err(e) = hypr::apply_option(key, &value) {
        eprintln!("nyx-settings: live apply of {key} failed: {e:#}");
    }
    notify(key, Some(&value));
    update(false, |s| {
        s.options.insert(key.to_string(), value);
    });
}

/// Stop managing an option. Hyprland is reloaded so the value falls back to
/// the user's own config or Omarchy's default.
pub fn reset_option(key: &str) {
    update(true, |s| {
        s.options.remove(key);
    });
    flush();
    notify(key, None);
}

pub fn set_device(device: &str, key: &str, value: Value) {
    let lua = format!(
        "hl.device({{ name = {}, {} = {} }})",
        super::lua::quote(device),
        super::lua::ident_or_key(key),
        super::lua::value(&value)
    );
    let _ = hypr::eval(&lua);
    update(false, |s| {
        s.devices.entry(device.to_string()).or_default().insert(key.to_string(), value);
    });
}

pub fn reset_device(device: &str) {
    update(true, |s| {
        s.devices.remove(device);
    });
    flush();
}

pub fn set_env(key: &str, value: Option<String>) {
    update(true, |s| match value {
        Some(v) => {
            s.env.insert(key.to_string(), v);
        }
        None => {
            s.env.remove(key);
        }
    });
}
