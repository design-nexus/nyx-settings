//! Desktop › Keyboard Shortcuts: the keyboard is the picture. Every key that does
//! something with Super held is coloured by what it does, free keys are blank and two
//! things on one key are outlined; a switch shows the Shift, Alt and Ctrl layers. The
//! list on the left follows the group picked above it.

use super::keyboard::{self as kb, Key, Look, fit, flow, flow_add, hold_controls, legend_item, tame_head};
use crate::backend::state::Bind;
use crate::backend::{hypr, store};
use crate::sketch::Rgb;
use crate::widgets::{self, Page};
use crate::window;
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

/// Hyprland modmask bits, in the order Omarchy writes them.
const MODS: &[(u64, &str)] = &[(64, "SUPER"), (4, "CTRL"), (8, "ALT"), (1, "SHIFT")];

pub fn keys_string(modmask: u64, key: &str) -> String {
    let mut parts: Vec<String> = MODS.iter().filter(|(bit, _)| modmask & bit != 0).map(|(_, n)| n.to_string()).collect();
    parts.push(key.to_string());
    parts.join(" + ")
}

/// Normalise "super+shift+ x" to "SUPER + SHIFT + x" so it can match unbind lists.
pub fn normalise(keys: &str) -> String {
    let parts: Vec<String> = keys.split('+').map(str::trim).filter(|p| !p.is_empty()).map(String::from).collect();
    let Some((key, mods)) = parts.split_last() else { return String::new() };
    let mods_upper: Vec<String> = mods.iter().map(|m| m.to_uppercase()).collect();
    let mut out: Vec<String> =
        MODS.iter().filter(|(_, n)| mods_upper.iter().any(|m| m == n)).map(|(_, n)| n.to_string()).collect();
    out.push(key.clone());
    out.join(" + ")
}

/// "SUPER + SHIFT + W" as its modmask and key.
fn parse_keys(keys: &str) -> (u64, String) {
    let n = normalise(keys);
    let mut parts: Vec<&str> = n.split(" + ").collect();
    let key = parts.pop().unwrap_or("").to_string();
    let mask = parts.iter().filter_map(|p| MODS.iter().find(|(_, m)| m == p).map(|(b, _)| *b)).fold(0, |a, b| a | b);
    (mask, key)
}

/// What a key is called on its keycap.
fn pretty_key(k: &str) -> String {
    if let Some(n) = k.strip_prefix("code:").and_then(|n| n.parse::<u32>().ok()) {
        return kb::ROWS.iter().flat_map(|r| r.iter()).find(|key| key.code == n).map(|key| key.us.to_string()).unwrap_or_else(|| k.to_string());
    }
    let s = match k.to_lowercase().as_str() {
        "super" => "Super",
        "ctrl" | "control" => "Ctrl",
        "alt" => "Alt",
        "shift" => "Shift",
        "left" => "←",
        "right" => "→",
        "up" => "↑",
        "down" => "↓",
        "return" | "enter" => "Return",
        "space" => "Space",
        "escape" => "Esc",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "minus" => "-",
        "equal" => "=",
        "comma" => ",",
        "period" => ".",
        "slash" => "/",
        "backslash" => "\\",
        "grave" => "`",
        "semicolon" => ";",
        "apostrophe" => "'",
        "bracketleft" => "[",
        "bracketright" => "]",
        "print" => "Print",
        _ if k.chars().count() == 1 => return k.to_uppercase(),
        _ => return k.strip_prefix("XF86").unwrap_or(k).to_string(),
    };
    s.to_string()
}

/// "SUPER + SHIFT + code:10" as people say it: "Super + Shift + 1".
pub fn pretty_keys(keys: &str) -> String {
    keys.split(" + ").map(pretty_key).collect::<Vec<_>>().join(" + ")
}

fn caps(keys: &str) -> gtk::Box {
    let b = widgets::hbox(4);
    for part in keys.split(" + ") {
        let l = gtk::Label::new(Some(&pretty_key(part)));
        l.add_css_class("key-cap");
        b.append(&l);
    }
    b.set_valign(gtk::Align::Center);
    b
}

fn gdk_key_name(key: gdk::Key) -> Option<String> {
    let name = key.to_lower().name()?.to_string();
    let modifier = [
        "Shift_L",
        "Shift_R",
        "Control_L",
        "Control_R",
        "Alt_L",
        "Alt_R",
        "Super_L",
        "Super_R",
        "Meta_L",
        "Meta_R",
        "ISO_Level3_Shift",
    ];
    if modifier.contains(&name.as_str()) {
        return None;
    }
    Some(if name.len() == 1 { name.to_uppercase() } else { name })
}

/// A button that records the next key combination pressed in the window `entry` is in.
fn recorder(entry: &gtk::Entry) -> gtk::Button {
    let b = gtk::Button::with_label("Record");
    let entry = entry.clone();
    b.connect_clicked(move |btn| {
        let Some(win) = entry.root().and_downcast::<gtk::Window>() else { return };
        btn.set_label("Press keys…");
        let ctl = gtk::EventControllerKey::new();
        ctl.set_propagation_phase(gtk::PropagationPhase::Capture);
        let entry = entry.clone();
        let btn = btn.clone();
        let win2 = win.clone();
        let slot: Rc<RefCell<Option<gtk::EventControllerKey>>> = Rc::new(RefCell::new(None));
        let slot2 = slot.clone();
        ctl.connect_key_pressed(move |_, key, _, state| {
            let Some(name) = gdk_key_name(key) else { return glib::Propagation::Stop };
            let mut mask = 0u64;
            if state.contains(gdk::ModifierType::SUPER_MASK) || state.contains(gdk::ModifierType::META_MASK) {
                mask |= 64;
            }
            if state.contains(gdk::ModifierType::CONTROL_MASK) {
                mask |= 4;
            }
            if state.contains(gdk::ModifierType::ALT_MASK) {
                mask |= 8;
            }
            if state.contains(gdk::ModifierType::SHIFT_MASK) {
                mask |= 1;
            }
            entry.set_text(&keys_string(mask, &name));
            btn.set_label("Record");
            if let Some(c) = slot2.borrow_mut().take() {
                win2.remove_controller(&c);
            }
            glib::Propagation::Stop
        });
        win.add_controller(ctl.clone());
        *slot.borrow_mut() = Some(ctl);
    });
    b
}

// ---------- The shortcuts ----------

/// The groups shortcuts are sorted into, with their colours on the map. The last is
/// the user's own.
const GROUPS: [(&str, Rgb); 6] = [
    ("Apps & menus", (0.33, 0.56, 0.97)),
    ("Windows", (0.58, 0.42, 0.96)),
    ("Workspaces & monitors", (0.85, 0.36, 0.85)),
    ("Screenshots & recording", (0.18, 0.74, 0.70)),
    ("Media & hardware", (0.96, 0.66, 0.20)),
    ("Your own", (0.30, 0.78, 0.40)),
];
const MINE: usize = 5;
/// The outline of a key that does two things.
const CLASH: Rgb = (0.98, 0.72, 0.18);

/// The layers the map shows: Super, and Super with one more modifier.
const LAYERS: [(u64, &str, &str); 4] =
    [(64, "Super", "Super alone"), (65, "+ Shift", "Super + Shift"), (72, "+ Alt", "Super + Alt"), (68, "+ Ctrl", "Super + Ctrl")];

/// Keys that only change others; they're never the key of a shortcut on the map.
const MODIFIER_KEYS: [&str; 8] = ["LFSH", "RTSH", "LCTL", "RCTL", "LALT", "RALT", "LWIN", "FN"];

#[derive(Clone)]
struct Shortcut {
    mask: u64,
    /// The key as Hyprland names it: "W", "Return", "code:10".
    key: String,
    keys: String,
    what: String,
    group: usize,
    /// What it runs, when it runs a command (so it can move to other keys).
    command: Option<String>,
    /// Its place in the user's own list.
    mine: Option<usize>,
}

/// Keys that do two things at once: the keys and what each does.
struct Clash {
    keys: String,
    whats: Vec<String>,
}

fn load() -> (Vec<Shortcut>, Vec<Clash>) {
    let custom = store::read(|s| s.binds.clone());
    let custom_keys: Vec<String> = custom.iter().map(|b| normalise(&b.keys)).collect();
    let mut all: Vec<Shortcut> = Vec::new();
    let mut actions: Vec<(String, String, bool)> = Vec::new();
    let mut clashes: Vec<Clash> = Vec::new();
    let by_code = keycodes_by_description();
    if let Some(serde_json::Value::Array(items)) = hypr::json(&["binds"]) {
        for b in &items {
            if b.get("mouse").and_then(|m| m.as_bool()).unwrap_or(false)
                || !b.get("submap").and_then(|d| d.as_str()).unwrap_or("").is_empty()
            {
                continue;
            }
            let Some(mut key) = b.get("key").and_then(|k| k.as_str()).map(String::from) else { continue };
            let Some(mask) = b.get("modmask").and_then(|m| m.as_u64()) else { continue };
            let description = b.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string();
            // Binds made by keycode (the number row, minus and equal) can come back with
            // neither a key nor a keycode; Omarchy's binding files say which key they are.
            if key.is_empty() {
                let code = match b.get("keycode").and_then(|c| c.as_u64()).unwrap_or(0) {
                    0 => match keycode_for(&by_code, mask, &description) {
                        Some(c) => c,
                        None => continue,
                    },
                    c => c as u32,
                };
                key = format!("code:{code}");
            }
            let keys = keys_string(mask, &key);
            if custom_keys.contains(&keys) {
                continue;
            }
            let release = b.get("release").and_then(|r| r.as_bool()).unwrap_or(false);
            let dispatcher = b.get("dispatcher").and_then(|d| d.as_str()).unwrap_or("").to_string();
            let arg = b.get("arg").and_then(|d| d.as_str()).unwrap_or("").trim().to_string();
            let what = if description.is_empty() { pretty_keys(&keys) } else { description };
            let action = format!("{dispatcher} {arg}");
            if let Some((_, first, first_release)) = actions.iter().find(|(k, _, _)| *k == keys) {
                // One bind on pressing and one on letting go (push-to-talk) is one shortcut.
                if *first != action && *first_release == release {
                    let first_what = all.iter().find(|s| s.keys == keys).map(|s| s.what.clone()).unwrap_or_default();
                    match clashes.iter_mut().find(|c| c.keys == keys) {
                        Some(c) => c.whats.push(what),
                        None => clashes.push(Clash { keys: keys.clone(), whats: vec![first_what, what] }),
                    }
                }
                continue;
            }
            actions.push((keys.clone(), action, release));
            let command = (dispatcher == "exec" && !arg.is_empty()).then_some(arg);
            all.push(Shortcut { mask, group: group_of(&what, &keys), key, keys, what, command, mine: None });
        }
    }
    for (i, b) in custom.iter().enumerate() {
        let (mask, key) = parse_keys(&b.keys);
        if key.is_empty() {
            continue;
        }
        all.push(Shortcut {
            mask,
            keys: keys_string(mask, &key),
            key,
            what: if b.description.is_empty() { b.command.clone() } else { b.description.clone() },
            group: MINE,
            command: Some(b.command.clone()),
            mine: Some(i),
        });
    }
    (all, clashes)
}

/// Omarchy's binds written with a keycode, as (modmask, description, keycode), read
/// from its binding files and the user's own.
fn keycodes_by_description() -> Vec<(u64, String, u32)> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("OMARCHY_PATH") {
        dirs.push(std::path::PathBuf::from(p).join("default/hypr/bindings"));
    }
    dirs.push("/usr/share/omarchy/default/hypr/bindings".into());
    dirs.push(crate::paths::hypr_dir());
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "lua") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            for line in text.lines() {
                // The string literals on the line: the keys, then the description.
                let strings: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
                for pair in strings.windows(2) {
                    let Some((_, code)) = pair[0].rsplit_once("code:") else { continue };
                    let Ok(code) = code.trim().parse::<u32>() else { continue };
                    out.push((parse_keys(pair[0]).0, pair[1].to_string(), code));
                }
            }
        }
    }
    out
}

/// The keycode of a bind Hyprland lists without its key: from the binding files, or
/// for "… workspace 3" and the like, the number row (Omarchy's loops bind N to N + 9).
fn keycode_for(by_code: &[(u64, String, u32)], mask: u64, description: &str) -> Option<u32> {
    if let Some((_, _, c)) = by_code.iter().find(|(m, d, _)| *m == mask && d == description) {
        return Some(*c);
    }
    let n: u32 = description.rsplit(' ').next()?.parse().ok()?;
    (1..=10).contains(&n).then_some(n + 9)
}

/// The keysym a drawn key types on a US keyboard, as Hyprland's binds name it.
fn sym_name(key: &Key) -> String {
    let s = match key.name {
        "ESC" => "escape",
        "PRSC" => "print",
        "TLDE" => "grave",
        "AE11" => "minus",
        "AE12" => "equal",
        "BKSP" => "backspace",
        "TAB" => "tab",
        "AD11" => "bracketleft",
        "AD12" => "bracketright",
        "BKSL" => "backslash",
        "CAPS" => "caps_lock",
        "AC10" => "semicolon",
        "AC11" => "apostrophe",
        "RTRN" => "return",
        "AB08" => "comma",
        "AB09" => "period",
        "AB10" => "slash",
        "SPCE" => "space",
        "LEFT" => "left",
        "RGHT" => "right",
        _ => return key.us.to_lowercase(),
    };
    s.to_string()
}

/// What to call a drawn key in a new bind.
fn bind_name(key: &Key) -> String {
    if key.us.chars().count() == 1 && key.us.chars().all(|c| c.is_ascii_alphanumeric()) {
        key.us.to_string()
    } else {
        sym_name(key)
    }
}

fn on_key(s: &Shortcut, key: &Key) -> bool {
    if key.code == 0 || MODIFIER_KEYS.contains(&key.name) {
        return false;
    }
    match s.key.strip_prefix("code:").and_then(|n| n.parse::<u32>().ok()) {
        Some(c) => c == key.code,
        None => s.key.to_lowercase() == sym_name(key) || (s.key.eq_ignore_ascii_case("enter") && key.name == "RTRN"),
    }
}

fn drawn_key(s: &Shortcut) -> Option<&'static Key> {
    kb::ROWS.iter().flat_map(|r| r.iter()).find(|k| on_key(s, k))
}

fn bindable() -> impl Iterator<Item = &'static Key> {
    kb::ROWS.iter().flat_map(|r| r.iter()).filter(|k| k.code != 0 && !MODIFIER_KEYS.contains(&k.name))
}

thread_local! {
    /// The layer, group and key picked, kept across a rebuild of the page.
    static PICK: Cell<(u64, usize, Option<&'static str>)> = const { Cell::new((64, 1, None)) };
}

/// What the map shows.
struct Sel {
    all: Vec<Shortcut>,
    clash_keys: Vec<String>,
    layer: Cell<u64>,
    group: Cell<usize>,
    /// A key on the map.
    key: Cell<Option<&'static str>>,
    /// A shortcut picked in the list whose keys aren't on the map.
    off_map: Cell<Option<usize>>,
}

impl Sel {
    fn on_layer(&self, key: &Key, layer: u64) -> Vec<usize> {
        self.all.iter().enumerate().filter(|(_, s)| s.mask == layer && on_key(s, key)).map(|(i, _)| i).collect()
    }

    fn look(&self, key: &Key) -> Look {
        let layer = self.layer.get();
        let mut l = Look {
            lit: key.name == "LWIN"
                || (layer & 1 != 0 && matches!(key.name, "LFSH" | "RTSH"))
                || (layer & 8 != 0 && matches!(key.name, "LALT" | "RALT"))
                || (layer & 4 != 0 && matches!(key.name, "LCTL" | "RCTL")),
            picked: self.key.get() == Some(key.name),
            ..Look::default()
        };
        let here = self.on_layer(key, layer);
        if let Some(&i) = here.first() {
            l.fill = Some(GROUPS[self.all[i].group].1);
            if here.len() > 1 || self.clash_keys.contains(&self.all[i].keys) {
                l.ring = Some(CLASH);
            }
        }
        l
    }

    fn save(&self) {
        PICK.with(|p| p.set((self.layer.get(), self.group.get(), self.key.get())));
    }
}

struct Map {
    sel: Rc<Sel>,
    board: gtk::DrawingArea,
    pop: gtk::Box,
    seg: gtk::Box,
    layer_cards: Vec<(u64, gtk::Button)>,
    foot: OnceCell<gtk::Label>,
    rows: Vec<(gtk::Box, usize)>,
}

fn refresh(m: &Rc<Map>) {
    let sel = &m.sel;
    sel.save();
    m.board.queue_draw();
    let layer = sel.layer.get();
    for (i, (mask, _, _)) in LAYERS.iter().enumerate() {
        if let Some(b) = nth_child(m.seg.upcast_ref(), i).and_downcast::<gtk::ToggleButton>()
            && *mask == layer
            && !b.is_active()
        {
            b.set_active(true);
        }
    }
    for (mask, card) in &m.layer_cards {
        if *mask == layer {
            card.add_css_class("ks-on");
        } else {
            card.remove_css_class("ks-on");
        }
    }
    let picked: Vec<usize> = match (sel.key.get(), sel.off_map.get()) {
        (_, Some(i)) => vec![i],
        (Some(name), None) => bindable().find(|k| k.name == name).map(|k| sel.on_layer(k, layer)).unwrap_or_default(),
        _ => Vec::new(),
    };
    for (row, i) in &m.rows {
        if picked.contains(i) {
            row.add_css_class("ks-picked");
        } else {
            row.remove_css_class("ks-picked");
        }
    }
    if let Some(foot) = m.foot.get() {
        let (_, _, name) = LAYERS.iter().find(|(mask, _, _)| *mask == layer).copied().unwrap_or(LAYERS[0]);
        let free: Vec<&str> = bindable()
            .filter(|k| k.us.chars().count() == 1 && k.us.chars().all(|c| c.is_ascii_alphanumeric()))
            .filter(|k| sel.on_layer(k, layer).is_empty())
            .map(|k| k.us)
            .collect();
        foot.set_text(&match free.len() {
            0 => format!("Every letter and number is taken with {name}."),
            1 => format!("Free with {name}: only {}.", free[0]),
            n if n <= 6 => format!("Free with {name}: {} and {}.", free[..n - 1].join(", "), free[n - 1]),
            _ => format!("Free with {name}: {} and more.", free[..6].join(", ")),
        });
    }
    fill_pop(m);
}

fn nth_child(w: &gtk::Widget, n: usize) -> Option<gtk::Widget> {
    std::iter::successors(w.first_child(), |c| c.next_sibling()).nth(n)
}

/// The strip under the keyboard: what the picked key does, and what can be done with it.
fn fill_pop(m: &Rc<Map>) {
    super::keyboard::clear(&m.pop);
    let sel = &m.sel;
    let layer = sel.layer.get();
    let (key, here): (Option<&'static Key>, Vec<usize>) = match (sel.off_map.get(), sel.key.get()) {
        (Some(i), _) => (None, vec![i]),
        (None, Some(name)) => match bindable().find(|k| k.name == name) {
            Some(k) => (Some(k), sel.on_layer(k, layer)),
            None => (None, Vec::new()),
        },
        _ => (None, Vec::new()),
    };
    let glyph_text;
    let glyph_class;
    let (title, line);
    let actions = widgets::hbox(6);
    actions.set_valign(gtk::Align::Center);
    if here.is_empty() {
        let Some(k) = key else {
            let hint = widgets::label("Click a key to see what it does, or a free one to give it a shortcut.", "ks-pop-hint");
            hint.set_wrap(true);
            hint.set_max_width_chars(80);
            hint.set_xalign(0.0);
            m.pop.append(&hint);
            return;
        };
        let keys = keys_string(layer, &bind_name(k));
        glyph_text = k.us.to_string();
        glyph_class = "ks-g-free".to_string();
        title = format!("{} is free", pretty_keys(&keys));
        line = others_line(sel, k, layer, "Nothing uses these keys yet.");
        let add = gtk::Button::with_label("Give it a shortcut");
        let m2 = m.clone();
        add.connect_clicked(move |_| editor(&m2, None, &keys));
        actions.append(&add);
    } else {
        let s = &sel.all[here[0]];
        glyph_text = key.map(|k| k.us.to_string()).unwrap_or_else(|| pretty_key(&s.key));
        glyph_class = format!("ks-g{}", s.group);
        let clash = sel.clash_keys.contains(&s.keys);
        if clash || here.len() > 1 {
            title = format!("{} does two things", pretty_keys(&s.keys));
            let mut whats: Vec<String> = here.iter().map(|i| sel.all[*i].what.clone()).collect();
            if let Some(c) = CLASHES.with(|c| c.borrow().iter().find(|c| c.keys == s.keys).map(|c| c.whats.clone())) {
                whats = c;
            }
            line = format!("{}. Both run when you press them.", join_and(&whats));
        } else {
            title = format!("{}: {}", pretty_keys(&s.keys), s.what);
            let group = GROUPS[s.group].0;
            line = match key {
                Some(k) => others_line(sel, k, layer, group),
                None => match &s.command {
                    Some(c) if s.mine.is_some() => format!("{group} · runs {c}"),
                    _ => group.to_string(),
                },
            };
        }
        if s.mine.is_some() || s.command.is_some() {
            let change = gtk::Button::with_label("Change");
            change.set_tooltip_text(Some(if s.mine.is_some() { "Change the keys, the name or what it runs" } else { "Move it to other keys" }));
            let (m2, s2) = (m.clone(), s.clone());
            change.connect_clicked(move |_| editor(&m2, Some(s2.clone()), &s2.keys));
            actions.append(&change);
        }
        let off = gtk::Button::with_label(if s.mine.is_some() { "Remove" } else { "Turn off" });
        off.add_css_class("flat");
        let s2 = s.clone();
        off.connect_clicked(move |_| match s2.mine {
            Some(i) => remove_mine(i),
            None => turn_off(&s2.keys),
        });
        actions.append(&off);
    }
    let glyph = widgets::label(&glyph_text, "ks-glyph");
    glyph.add_css_class(&glyph_class);
    glyph.set_xalign(0.5);
    glyph.set_valign(gtk::Align::Center);
    m.pop.append(&glyph);
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(&title, "ks-pop-title");
    t.set_wrap(true);
    t.set_xalign(0.0);
    text.append(&t);
    let l = widgets::label(&line, "ks-pop-line");
    l.set_wrap(true);
    l.set_xalign(0.0);
    text.append(&l);
    m.pop.append(&text);
    m.pop.append(&actions);
}

/// The group a key's shortcut is in, then what the same key does on the other layers.
fn others_line(sel: &Sel, key: &Key, layer: u64, first: &str) -> String {
    let mut parts = vec![first.to_string()];
    for (mask, _, _) in LAYERS.iter().filter(|(m, _, _)| *m != layer) {
        if let Some(&i) = sel.on_layer(key, *mask).first() {
            parts.push(format!("{} is {}", pretty_keys(&sel.all[i].keys), lower_first(&sel.all[i].what)));
        }
    }
    parts.truncate(3);
    parts.join(" · ")
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match (c.next(), c.clone().next()) {
        // Keep names like "OCR" or "AI" as they are.
        (Some(f), Some(n)) if f.is_uppercase() && !n.is_uppercase() => f.to_lowercase().chain(c).collect(),
        _ => s.to_string(),
    }
}

fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{}, and {}", rest.join(", "), lower_first(last)),
    }
}

thread_local! {
    static CLASHES: RefCell<Vec<Clash>> = const { RefCell::new(Vec::new()) };
}

fn rebuild_soon() {
    glib::timeout_add_local_once(std::time::Duration::from_millis(600), || window::rebuild("keybindings"));
}

fn turn_off(keys: &str) {
    let k = keys.to_string();
    store::update(true, |s| {
        s.unbinds.retain(|u| normalise(u) != normalise(&k));
        s.unbinds.push(k.clone());
    });
    store::flush();
    rebuild_soon();
    let back = keys.to_string();
    window::toast_action(&format!("Turned off {}", pretty_keys(keys)), "Undo", move || {
        let back = back.clone();
        store::update(true, move |s| s.unbinds.retain(|u| *u != back));
        store::flush();
        rebuild_soon();
    });
}

fn remove_mine(i: usize) {
    let removed = store::read(|s| s.binds.get(i).cloned());
    store::update(true, |s| {
        if i < s.binds.len() {
            s.binds.remove(i);
        }
    });
    store::flush();
    rebuild_soon();
    if let Some(back) = removed {
        window::toast_action("Shortcut removed", "Undo", move || {
            let back = back.clone();
            store::update(true, move |s| s.binds.insert(i.min(s.binds.len()), back));
            store::flush();
            rebuild_soon();
        });
    }
}

/// Add a shortcut, change one of the user's own, or move a built-in one that runs a
/// command to other keys (the old keys are turned off).
fn editor(m: &Rc<Map>, start: Option<Shortcut>, keys_prefill: &str) {
    let title = match &start {
        Some(s) if s.mine.is_some() => "Change a shortcut",
        Some(_) => "Move to other keys",
        None => "Add a shortcut",
    };
    let taken: Vec<(String, String)> = m
        .sel
        .all
        .iter()
        .filter(|s| start.as_ref().is_none_or(|st| st.keys != s.keys))
        .map(|s| (normalise(&s.keys), s.what.clone()))
        .collect();
    let win: Rc<OnceCell<gtk::Window>> = Rc::default();
    let w2 = win.clone();
    let keys_prefill = keys_prefill.to_string();
    let w = widgets::subpage(title, 480, move |p| {
        let g = p.group("");
        let keys = gtk::Entry::new();
        keys.set_placeholder_text(Some("SUPER + ALT + K"));
        keys.add_css_class("mono");
        keys.set_hexpand(true);
        keys.set_text(&keys_prefill);
        let line = widgets::hbox(8);
        line.append(&keys);
        line.append(&recorder(&keys));
        g.add(&widgets::stacked_row("Keys", "Type them, or press Record and then the keys.", line.upcast_ref()));
        let clash = widgets::label("", "clash-warning");
        clash.set_wrap(true);
        clash.set_xalign(0.0);
        clash.add_css_class("ks-form-clash");
        let check = {
            let clash = clash.clone();
            move |text: &str| {
                let k = normalise(text);
                match taken.iter().find(|(keys, _)| !k.is_empty() && keys.eq_ignore_ascii_case(&k)) {
                    Some((_, what)) => {
                        clash.set_text(&format!("These keys already do “{what}”. Yours will take their place."));
                        clash.set_visible(true);
                    }
                    None => clash.set_visible(false),
                }
            }
        };
        check(&keys_prefill);
        keys.connect_changed(move |e| check(&e.text()));
        g.add(&clash);
        let name = gtk::Entry::new();
        name.set_placeholder_text(Some("Like “Open the calculator”"));
        let command = gtk::Entry::new();
        command.set_placeholder_text(Some("A command, like kitty -e btop"));
        command.add_css_class("mono");
        if let Some(s) = &start {
            name.set_text(&s.what);
            command.set_text(s.command.as_deref().unwrap_or(""));
        }
        g.add(&widgets::stacked_row("Name", "Shown in the keybindings menu.", name.upcast_ref()));
        g.add(&widgets::stacked_row("Runs", "A command or an app.", command.upcast_ref()));
        let save = gtk::Button::with_label(match &start {
            Some(s) if s.mine.is_some() => "Save",
            Some(_) => "Move",
            None => "Add shortcut",
        });
        save.add_css_class("suggested-action");
        save.set_halign(gtk::Align::End);
        save.set_margin_top(12);
        let start = start.clone();
        let w3 = w2.clone();
        save.connect_clicked(move |_| {
            let k = normalise(&keys.text());
            let c = command.text().trim().to_string();
            if k.is_empty() || c.is_empty() {
                window::toast("Enter the keys and a command");
                return;
            }
            let d = name.text().trim().to_string();
            let (mine, old) = match &start {
                Some(s) => (s.mine, (s.mine.is_none() && normalise(&s.keys) != k).then(|| s.keys.clone())),
                None => (None, None),
            };
            store::update(true, |s| {
                if let Some(i) = mine.filter(|i| *i < s.binds.len()) {
                    s.binds.remove(i);
                }
                s.binds.retain(|b| normalise(&b.keys) != k);
                s.unbinds.retain(|u| normalise(u) != k);
                if let Some(old) = &old
                    && !s.unbinds.iter().any(|u| normalise(u) == normalise(old))
                {
                    s.unbinds.push(old.clone());
                }
                s.binds.push(Bind { keys: k.clone(), description: d, command: c });
            });
            store::flush();
            window::toast(&if mine.is_some() { format!("Saved {}", pretty_keys(&k)) } else { format!("Added {}", pretty_keys(&k)) });
            if let Some(w) = w3.get() {
                w.close();
            }
            rebuild_soon();
        });
        p.plain_group("").add(&save);
    });
    let _ = win.set(w);
}

pub fn build(page: &Page) {
    let (all, clashes) = load();
    let (layer, group, key) = PICK.with(|p| p.get());
    let sel = Rc::new(Sel {
        clash_keys: clashes.iter().map(|c| c.keys.clone()).collect(),
        all,
        layer: Cell::new(layer),
        group: Cell::new(group),
        key: Cell::new(key),
        off_map: Cell::new(None),
    });
    let slot: Rc<OnceCell<Rc<Map>>> = Rc::default();
    widgets::keywords("keybindings hotkeys shortcuts keys super bind");

    // ----- The picture: Super's keys, layer by layer -----
    let pv = page.split();
    let seg = {
        let (sel, slot) = (sel.clone(), slot.clone());
        let options: Vec<(String, String)> = LAYERS.iter().map(|(m, l, _)| (m.to_string(), l.to_string())).collect();
        widgets::segmented(&options, &layer.to_string(), move |id| {
            let mask = id.parse().unwrap_or(64);
            if sel.layer.get() == mask {
                return;
            }
            sel.layer.set(mask);
            sel.off_map.set(None);
            if let Some(m) = slot.get() {
                refresh(m);
            }
        })
    };
    let head = pv.head(
        "Super and…",
        "Keys that do something with Super held, coloured by what they do.",
        &[fit(&seg).upcast_ref()],
    );
    tame_head(&head, 34);

    let card = widgets::vbox(0);
    card.add_css_class("preview-card");
    card.add_css_class("ks-board");
    let board = {
        let sel = sel.clone();
        widgets::drawing(236, move |cr, p, _, w, h| kb::draw_board(cr, p, w, h, |k| sel.look(k)))
    };
    board.set_cursor_from_name(Some("pointer"));
    card.append(&board);
    let pop = widgets::hbox(12);
    pop.add_css_class("ks-pop");
    card.append(&pop);
    pv.add(&card);

    let legend = flow();
    legend.add_css_class("ks-legend");
    for (i, (name, _)) in GROUPS.iter().enumerate() {
        flow_add(&legend, &legend_item(name, &format!("ks-dot{i}")));
    }
    flow_add(&legend, &legend_item("Free", "ks-dot-free"));
    flow_add(&legend, &legend_item("Two things", "ks-dot-clash"));
    pv.add(&legend);

    let layers_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    layers_box.set_homogeneous(true);
    layers_box.add_css_class("ks-layers");
    let mut layer_cards = Vec::new();
    let total = bindable().count();
    for (mask, _, name) in LAYERS {
        let used = bindable().filter(|k| !sel.on_layer(k, mask).is_empty()).count();
        let b = gtk::Button::new();
        b.add_css_class("ks-layer");
        let col = widgets::vbox(2);
        col.append(&widgets::label(if mask == 64 { "Super" } else { name }, "ks-layer-name"));
        col.append(&widgets::label(&format!("{used} used · {} free", total - used), "ks-layer-count"));
        for c in std::iter::successors(col.first_child(), |c| c.next_sibling()) {
            if let Some(l) = c.downcast_ref::<gtk::Label>() {
                l.set_xalign(0.0);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            }
        }
        b.set_child(Some(&col));
        let (sel, slot) = (sel.clone(), slot.clone());
        b.connect_clicked(move |_| {
            sel.layer.set(mask);
            sel.off_map.set(None);
            if let Some(m) = slot.get() {
                refresh(m);
            }
        });
        layers_box.append(&b);
        layer_cards.push((mask, b));
    }
    pv.add(&fit(&layers_box));

    // ----- The controls: search, groups, the picked group's list -----
    let g = page.plain_group("");
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(&format!("Search {} shortcuts", sel.all.len())));
    search.add_css_class("ks-search");
    g.add(&search);
    // In lines that fit the column (a flow box would line them up in columns instead).
    let chips = widgets::vbox(6);
    chips.add_css_class("ks-chips");
    let lines: [gtk::Box; 3] = std::array::from_fn(|_| widgets::hbox(6));
    for l in &lines {
        chips.append(l);
    }
    let mut chip_buttons: Vec<gtk::ToggleButton> = Vec::new();
    for (i, (name, _)) in GROUPS.iter().enumerate() {
        let count = sel.all.iter().filter(|s| s.group == i).count();
        let b = gtk::ToggleButton::new();
        b.add_css_class("ks-chip");
        let c = widgets::hbox(6);
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("ks-chip-dot");
        dot.add_css_class(&format!("ks-dot{i}"));
        dot.set_valign(gtk::Align::Center);
        c.append(&dot);
        c.append(&gtk::Label::new(Some(name)));
        c.append(&widgets::label(&count.to_string(), "ks-chip-count"));
        b.set_child(Some(&c));
        if let Some(f) = chip_buttons.first() {
            b.set_group(Some(f));
        }
        b.set_active(i == group);
        b.set_focus_on_click(false);
        lines[match i {
            0 | 1 | MINE => 0,
            2 | 4 => 1,
            _ => 2,
        }]
        .append(&b);
        chip_buttons.push(b);
    }
    g.add(&chips);

    if let Some(c) = clashes.first() {
        let more = match clashes.len() {
            1 => String::new(),
            2 => " One more combination does two things too.".to_string(),
            n => format!(" {} more combinations do two things too.", n - 1),
        };
        let b = page.banner(
            &format!(
                "<b>{} does two things</b>\n{}.{more}",
                glib::markup_escape_text(&pretty_keys(&c.keys)),
                glib::markup_escape_text(&join_and(&c.whats))
            ),
            true,
        );
        b.add_css_class("ks-clash");
        let show = gtk::Button::with_label("Show");
        show.set_valign(gtk::Align::Center);
        let (sel, slot, keys) = (sel.clone(), slot.clone(), c.keys.clone());
        show.connect_clicked(move |_| {
            let Some(i) = sel.all.iter().position(|s| s.keys == keys) else { return };
            pick_shortcut(&sel, i);
            if let Some(m) = slot.get() {
                refresh(m);
            }
        });
        b.append(&show);
    }
    CLASHES.with(|c| *c.borrow_mut() = clashes);

    let g = page.plain_group("");
    g.wrapper.add_css_class("ks-list");
    let list_head = widgets::hbox(8);
    list_head.add_css_class("ks-list-head");
    let list_title = widgets::label(GROUPS[group].0, "ks-list-title");
    list_title.set_hexpand(true);
    list_title.set_xalign(0.0);
    list_head.append(&list_title);
    let add = gtk::Button::new();
    let ac = widgets::hbox(4);
    ac.append(&gtk::Image::from_icon_name("list-add-symbolic"));
    ac.append(&gtk::Label::new(Some("Add")));
    add.set_child(Some(&ac));
    add.add_css_class("ks-add");
    add.set_tooltip_text(Some("Add a shortcut of your own"));
    list_head.append(&add);
    g.add(&list_head);

    let mut rows: Vec<(gtk::Box, usize)> = Vec::new();
    let mut cards: Vec<gtk::Box> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    for (gi, (gname, _)) in GROUPS.iter().enumerate() {
        let card = widgets::vbox(0);
        card.add_css_class("settings-card");
        card.set_overflow(gtk::Overflow::Hidden);
        for (i, s) in sel.all.iter().enumerate().filter(|(_, s)| s.group == gi) {
            let desc = match (&s.command, s.mine) {
                (Some(c), Some(_)) => format!("<tt>{}</tt>", glib::markup_escape_text(c)),
                _ => String::new(),
            };
            let row = widgets::row(&s.what, &desc, Some(caps(&s.keys).upcast_ref()));
            row.add_css_class("ks-row");
            row.set_cursor_from_name(Some("pointer"));
            let click = gtk::GestureClick::new();
            let (sel, slot) = (sel.clone(), slot.clone());
            click.connect_released(move |_, _, _, _| {
                pick_shortcut(&sel, i);
                if let Some(m) = slot.get() {
                    refresh(m);
                }
            });
            row.add_controller(click);
            widgets::keywords(&s.keys);
            card.append(&row);
            texts.push(crate::search::normalise(&format!("{} {} {} {gname}", s.what, s.keys, pretty_keys(&s.keys))));
            rows.push((row, i));
        }
        if gi == MINE && card.first_child().is_none() {
            let r = widgets::row(
                "None of your own yet",
                "Add one to run a command or open an app with keys you pick. It replaces whatever the keys did before.",
                None,
            );
            card.append(&r);
        }
        card.set_visible(gi == group);
        g.add(&card);
        cards.push(card);
    }
    widgets::mark_first_rows(g.wrapper.upcast_ref());

    let off = store::read(|s| s.unbinds.clone());
    if !off.is_empty() {
        let g = page.group("");
        let n = off.len();
        let (r, _) = widgets::nav_row(Some("view-conceal-symbolic"), "Turned off", "Free keys you can give to something else", &n.to_string(), move || {
            let off = off.clone();
            let win: Rc<OnceCell<gtk::Window>> = Rc::default();
            let w2 = win.clone();
            let w = widgets::subpage("Turned off", 460, move |p| {
                let g = p.group("");
                g.note("These keys do nothing now. Turn one back on to get what it did before.");
                for key in off {
                    let b = gtk::Button::with_label("Turn back on");
                    b.set_valign(gtk::Align::Center);
                    let (k, w3) = (key.clone(), w2.clone());
                    b.connect_clicked(move |_| {
                        let k = k.clone();
                        store::update(true, |s| s.unbinds.retain(|u| *u != k));
                        store::flush();
                        if let Some(w) = w3.get() {
                            w.close();
                        }
                        rebuild_soon();
                    });
                    let content = widgets::hbox(10);
                    content.append(&caps(&normalise(&key)));
                    content.append(&b);
                    g.add(&widgets::row(&pretty_keys(&normalise(&key)), "", Some(content.upcast_ref())));
                }
            });
            let _ = win.set(w);
        });
        g.add(&r);
    }

    let map = Rc::new(Map { sel: sel.clone(), board: board.clone(), pop, seg, layer_cards, foot: OnceCell::new(), rows });
    let _ = slot.set(map.clone());
    let foot = pv.foot("dialog-information-symbolic", "");
    let _ = map.foot.set(foot);

    {
        let m = map.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |g, _, x, y| {
            let Some(w) = g.widget() else { return };
            let Some(k) = kb::key_at(w.width() as f64, w.height() as f64, x, y) else { return };
            if k.code == 0 || MODIFIER_KEYS.contains(&k.name) {
                return;
            }
            m.sel.key.set(Some(k.name));
            m.sel.off_map.set(None);
            refresh(&m);
        });
        board.add_controller(click);
    }
    {
        let m = map.clone();
        add.connect_clicked(move |_| editor(&m, None, ""));
    }

    // Picking a group shows its list; searching shows matches from every group.
    let show_group = {
        let (cards, list_title, sel) = (cards.clone(), list_title.clone(), sel.clone());
        let wrapper = g.wrapper.clone();
        move |gi: usize| {
            sel.group.set(gi);
            sel.save();
            list_title.set_text(GROUPS[gi].0);
            for (i, c) in cards.iter().enumerate() {
                c.set_visible(i == gi);
            }
            widgets::mark_first_rows(wrapper.upcast_ref());
        }
    };
    for (gi, b) in chip_buttons.iter().enumerate() {
        let (show_group, search) = (show_group.clone(), search.clone());
        b.connect_toggled(move |b| {
            if b.is_active() {
                search.set_text("");
                show_group(gi);
            }
        });
    }
    {
        let (cards, list_title, sel, map) = (cards.clone(), list_title.clone(), sel.clone(), map.clone());
        let wrapper = g.wrapper.clone();
        search.connect_search_changed(move |f| {
            let terms = crate::search::terms(&f.text());
            if terms.is_empty() {
                for (row, _) in &map.rows {
                    row.set_visible(true);
                }
                show_group(sel.group.get());
                return;
            }
            let mut found = 0;
            for ((row, _), text) in map.rows.iter().zip(&texts) {
                let hit = crate::search::matches(text, &terms);
                row.set_visible(hit);
                found += hit as usize;
            }
            for c in &cards {
                c.set_visible(std::iter::successors(c.first_child(), |r| r.next_sibling()).any(|r| r.is_visible() && r.has_css_class("ks-row")));
            }
            list_title.set_text(&match found {
                0 => "Nothing matches".to_string(),
                1 => "1 match".to_string(),
                n => format!("{n} matches"),
            });
            widgets::mark_first_rows(wrapper.upcast_ref());
        });
    }

    refresh(&map);
    hold_controls(page, 40);
}

/// Pick a shortcut from the list: its key on its layer, or by itself when it isn't on
/// the map.
fn pick_shortcut(sel: &Sel, i: usize) {
    let s = &sel.all[i];
    match drawn_key(s).filter(|_| LAYERS.iter().any(|(m, _, _)| *m == s.mask)) {
        Some(k) => {
            sel.layer.set(s.mask);
            sel.key.set(Some(k.name));
            sel.off_map.set(None);
        }
        None => {
            sel.key.set(None);
            sel.off_map.set(Some(i));
        }
    }
}

/// Which group a shortcut goes in, by what it says it does and its key.
fn group_of(description: &str, keys: &str) -> usize {
    let d = description.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| d.contains(w));
    if keys.contains("XF86")
        || has(&["volume", "brightness", "mute", "media", "play", "pause", "track", "backlight", "microphone"])
    {
        4
    } else if has(&["screenshot", "screen record", "recording", "color picker", "colour picker", "ocr", "qr code", "capture"]) {
        3
    } else if has(&["workspace", "monitor"]) {
        2
    } else if has(&[
        "window",
        "focus",
        "full screen",
        "fullscreen",
        "float",
        "group",
        "split",
        "resize",
        "grow",
        "shrink",
        "swap",
        "pin",
        "close",
        "kill",
        "pseudo",
        "tile",
        "move",
        "scratchpad",
        "transparen",
        "width",
    ]) {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_keys_from_modmask() {
        assert_eq!(keys_string(64 | 1, "W"), "SUPER + SHIFT + W");
        assert_eq!(keys_string(0, "XF86AudioMute"), "XF86AudioMute");
    }

    #[test]
    fn sorts_into_groups() {
        assert_eq!(GROUPS[group_of("Mute", "XF86AudioMute")].0, "Media & hardware");
        assert_eq!(GROUPS[group_of("Full screen", "SUPER + F")].0, "Windows");
        assert_eq!(GROUPS[group_of("Move window to workspace 1", "SUPER + SHIFT + code:10")].0, "Workspaces & monitors");
        assert_eq!(GROUPS[group_of("Color picker", "SUPER + PRINT")].0, "Screenshots & recording");
        assert_eq!(GROUPS[group_of("Browser (private)", "SUPER + ALT + SHIFT + B")].0, "Apps & menus");
    }

    #[test]
    fn normalises_typed_keys() {
        assert_eq!(normalise("shift+super + k"), "SUPER + SHIFT + k");
        assert_eq!(normalise("SUPER + ALT + K"), "SUPER + ALT + K");
        assert_eq!(normalise(""), "");
    }

    #[test]
    fn reads_keys_back() {
        assert_eq!(parse_keys("SUPER + SHIFT + W"), (65, "W".to_string()));
        assert_eq!(pretty_keys("SUPER + code:10"), "Super + 1");
    }
}
