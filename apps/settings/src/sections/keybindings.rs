use crate::backend::state::Bind;
use crate::backend::{hypr, store};
use crate::widgets::{self, Page};
use crate::window;
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    /// The custom shortcut being edited in the form at the bottom of the list.
    static EDITING: Cell<Option<usize>> = const { Cell::new(None) };
}

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

fn caps(keys: &str) -> gtk::Box {
    let b = widgets::hbox(4);
    for (i, part) in keys.split(" + ").enumerate() {
        if i > 0 {
            b.append(&widgets::label("+", "dim"));
        }
        let pretty = match part {
            "SUPER" => "Super".to_string(),
            "CTRL" => "Ctrl".to_string(),
            "ALT" => "Alt".to_string(),
            "SHIFT" => "Shift".to_string(),
            k if k.starts_with("code:") => {
                // Number-row keycodes 10–19 are 1–0.
                match k[5..].parse::<u32>() {
                    Ok(n @ 10..=19) => ((n - 9) % 10).to_string(),
                    _ => k.to_string(),
                }
            }
            k => k.to_string(),
        };
        let l = gtk::Label::new(Some(&pretty));
        l.add_css_class("key-cap");
        b.append(&l);
    }
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

/// A button that records the next key combination pressed.
fn recorder(entry: &gtk::Entry) -> gtk::Button {
    let b = gtk::Button::with_label("Record");
    let entry = entry.clone();
    b.connect_clicked(move |btn| {
        btn.set_label("Press keys…");
        let Some(win) = window::window() else { return };
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

struct Existing {
    keys: String,
    description: String,
    submap: String,
}

fn existing_binds() -> Vec<Existing> {
    let Some(serde_json::Value::Array(items)) = hypr::json(&["binds"]) else { return vec![] };
    let custom: Vec<String> = store::read(|s| s.binds.iter().map(|b| normalise(&b.keys)).collect());
    let mut out: Vec<Existing> = items
        .iter()
        .filter(|b| !b.get("mouse").and_then(|m| m.as_bool()).unwrap_or(false))
        .filter_map(|b| {
            let mut key = b.get("key")?.as_str()?.to_string();
            // Number-row binds are made by keycode, with no key name.
            if key.is_empty() {
                let code = b.get("keycode").and_then(|c| c.as_u64()).unwrap_or(0);
                if code == 0 {
                    return None;
                }
                key = format!("code:{code}");
            }
            let mask = b.get("modmask")?.as_u64()?;
            let description = b.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string();
            let submap = b.get("submap").and_then(|d| d.as_str()).unwrap_or("").to_string();
            Some(Existing { keys: keys_string(mask, &key), description, submap })
        })
        .filter(|e| e.submap.is_empty() && !custom.contains(&e.keys))
        .collect();
    out.dedup_by(|a, b| a.keys == b.keys);
    out
}

pub fn build(page: &Page) {
    // ----- Your shortcuts -----
    let g = page.group("Your shortcuts");
    let custom = store::read(|s| s.binds.clone());
    if custom.is_empty() {
        g.note("Shortcuts you add here run a command. They replace any existing shortcut on the same keys.");
    }
    for (i, b) in custom.iter().enumerate() {
        let content = widgets::hbox(10);
        content.append(&caps(&normalise(&b.keys)));
        let edit = gtk::Button::from_icon_name("document-edit-symbolic");
        edit.add_css_class("flat");
        edit.set_tooltip_text(Some("Edit"));
        edit.connect_clicked(move |_| {
            EDITING.with(|e| e.set(Some(i)));
            window::rebuild("keybindings");
        });
        content.append(&edit);
        let del = gtk::Button::from_icon_name("user-trash-symbolic");
        del.add_css_class("flat");
        del.set_tooltip_text(Some("Remove"));
        let removed = b.clone();
        del.connect_clicked(move |_| {
            EDITING.with(|e| e.set(None));
            store::update(true, |s| {
                if i < s.binds.len() {
                    s.binds.remove(i);
                }
            });
            store::flush();
            window::rebuild("keybindings");
            let back = removed.clone();
            window::toast_action("Shortcut removed", "Undo", move || {
                let back = back.clone();
                store::update(true, move |s| s.binds.insert(i.min(s.binds.len()), back));
                store::flush();
                window::rebuild("keybindings");
            });
        });
        content.append(&del);
        let desc = format!("<tt>{}</tt>", glib::markup_escape_text(&b.command));
        g.add(&widgets::row(
            if b.description.is_empty() { &b.command } else { &b.description },
            &desc,
            Some(content.upcast_ref()),
        ));
    }

    // Add / edit form.
    let editing = EDITING.with(|e| e.get()).filter(|i| *i < custom.len());
    let current = editing.map(|i| custom[i].clone());
    let form = widgets::vbox(8);
    let keys_line = widgets::hbox(8);
    let keys = gtk::Entry::new();
    keys.set_placeholder_text(Some("SUPER + ALT + K"));
    keys.add_css_class("mono");
    keys.set_hexpand(true);
    if let Some(b) = &current {
        keys.set_text(&b.keys);
    }
    keys_line.append(&keys);
    keys_line.append(&recorder(&keys));
    form.append(&keys_line);
    // Say when the keys already do something.
    let clash = widgets::label("", "clash-warning");
    clash.set_wrap(true);
    clash.set_visible(false);
    form.append(&clash);
    {
        let taken: Vec<(String, String)> = existing_binds()
            .into_iter()
            .map(|e| (e.keys, e.description))
            .chain(custom.iter().enumerate().filter(|(i, _)| Some(*i) != editing).map(|(_, b)| {
                (normalise(&b.keys), if b.description.is_empty() { b.command.clone() } else { b.description.clone() })
            }))
            .collect();
        let clash = clash.clone();
        keys.connect_changed(move |e| {
            let k = normalise(&e.text());
            match taken.iter().find(|(keys, _)| !k.is_empty() && keys.eq_ignore_ascii_case(&k)) {
                Some((_, what)) => {
                    let what = if what.is_empty() { "another shortcut".to_string() } else { format!("“{what}”") };
                    clash.set_text(&format!("These keys already do {what}. Yours will replace it."));
                    clash.set_visible(true);
                }
                None => clash.set_visible(false),
            }
        });
    }
    let desc = gtk::Entry::new();
    desc.set_placeholder_text(Some("What it does (shown in the keybindings menu)"));
    if let Some(b) = &current {
        desc.set_text(&b.description);
    }
    form.append(&desc);
    let command = gtk::Entry::new();
    command.set_placeholder_text(Some("Command, e.g. kitty -e btop"));
    command.add_css_class("mono");
    if let Some(b) = &current {
        command.set_text(&b.command);
    }
    form.append(&command);
    let add = gtk::Button::with_label(if editing.is_some() { "Save changes" } else { "Add shortcut" });
    add.add_css_class("suggested-action");
    {
        let (keys, desc, command) = (keys.clone(), desc.clone(), command.clone());
        add.connect_clicked(move |_| {
            let k = normalise(&keys.text());
            let c = command.text().trim().to_string();
            if k.is_empty() || c.is_empty() {
                window::toast("Enter the keys and a command");
                return;
            }
            let d = desc.text().trim().to_string();
            let editing = EDITING.with(|e| e.take());
            store::update(true, |s| {
                // Editing replaces the shortcut in place.
                if let Some(i) = editing.filter(|i| *i < s.binds.len()) {
                    s.binds.remove(i);
                }
                s.binds.retain(|b| normalise(&b.keys) != k);
                s.unbinds.retain(|u| normalise(u) != k);
                s.binds.push(Bind { keys: k.clone(), description: d, command: c });
            });
            store::flush();
            window::toast(&if editing.is_some() { format!("Saved {k}") } else { format!("Added {k}") });
            glib::timeout_add_local_once(std::time::Duration::from_millis(600), || window::rebuild("keybindings"));
        });
    }
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    if editing.is_some() {
        let cancel = gtk::Button::with_label("Cancel");
        cancel.connect_clicked(|_| {
            EDITING.with(|e| e.set(None));
            window::rebuild("keybindings");
        });
        buttons.append(&cancel);
    }
    buttons.append(&add);
    form.append(&buttons);
    let add_row = widgets::form_disclosure(if editing.is_some() { "Edit a shortcut" } else { "Add a shortcut…" }, "", &form);
    if editing.is_some() {
        widgets::open_disclosure(&add_row);
    }
    g.add(&add_row);

    // ----- Turned off -----
    let off = store::read(|s| s.unbinds.clone());
    if !off.is_empty() {
        let g = page.collapsible("Turned off", false);
        for key in off {
            let k2 = key.clone();
            let b = gtk::Button::with_label("Turn back on");
            b.connect_clicked(move |_| {
                let k = k2.clone();
                store::update(true, |s| s.unbinds.retain(|u| *u != k));
                store::flush();
                glib::timeout_add_local_once(std::time::Duration::from_millis(600), || window::rebuild("keybindings"));
            });
            let content = widgets::hbox(10);
            content.append(&caps(&key));
            content.append(&b);
            g.add(&widgets::row("Off", "", Some(content.upcast_ref())));
        }
    }

    // ----- Everything else -----
    let existing = existing_binds();
    let filter = gtk::SearchEntry::new();
    filter.set_placeholder_text(Some(&format!("Filter {} shortcuts", existing.len())));
    filter.add_css_class("shortcut-filter");
    let g = page.plain_group("All shortcuts");
    g.note("Switch one off to free its keys for something else.");
    g.add(&filter);
    let mut rows: Vec<(gtk::Box, String)> = Vec::new();
    let mut cards: Vec<(gtk::Box, Vec<usize>)> = Vec::new();
    for cat in CATEGORIES {
        let in_cat: Vec<&Existing> = existing.iter().filter(|e| category(&e.description, &e.keys) == *cat).collect();
        if in_cat.is_empty() {
            continue;
        }
        let g = page.group(cat);
        let mut mine = Vec::new();
        for e in in_cat {
            let content = widgets::hbox(12);
            content.append(&caps(&e.keys));
            let sw = gtk::Switch::new();
            sw.set_active(true);
            sw.set_valign(gtk::Align::Center);
            sw.set_tooltip_text(Some("Off frees these keys"));
            let k = e.keys.clone();
            sw.connect_active_notify(move |sw| {
                let on = sw.is_active();
                let k = k.clone();
                store::update(true, |s| {
                    s.unbinds.retain(|u| *u != k);
                    if !on {
                        s.unbinds.push(k);
                    }
                });
            });
            content.append(&sw);
            let title = if e.description.is_empty() { e.keys.clone() } else { e.description.clone() };
            let row = widgets::row(&title, "", Some(content.upcast_ref()));
            g.add(&row);
            widgets::keywords(&e.keys);
            mine.push(rows.len());
            rows.push((row, crate::search::normalise(&format!("{title} {} {cat}", e.keys))));
        }
        cards.push((g.wrapper.clone(), mine));
    }
    filter.connect_search_changed(move |f| {
        let terms = crate::search::terms(&f.text());
        for (row, text) in &rows {
            row.set_visible(terms.is_empty() || crate::search::matches(text, &terms));
        }
        for (card, idx) in &cards {
            card.set_visible(idx.iter().any(|i| rows[*i].0.is_visible()));
            widgets::mark_first_rows(card.upcast_ref());
        }
    });
}

/// The groups the full list is split into, in order.
const CATEGORIES: &[&str] = &["Apps & menus", "Windows", "Workspaces & monitors", "Screenshots & recording", "Media & hardware"];

/// Which group a shortcut goes in, by what it says it does and its key.
fn category(description: &str, keys: &str) -> &'static str {
    let d = description.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| d.contains(w));
    if keys.contains("XF86")
        || has(&["volume", "brightness", "mute", "media", "play", "pause", "track", "backlight", "microphone"])
    {
        "Media & hardware"
    } else if has(&["screenshot", "screen record", "recording", "color picker", "colour picker", "ocr", "qr code", "capture"]) {
        "Screenshots & recording"
    } else if has(&["workspace", "monitor"]) {
        "Workspaces & monitors"
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
        "Windows"
    } else {
        "Apps & menus"
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
        assert_eq!(category("Mute", "XF86AudioMute"), "Media & hardware");
        assert_eq!(category("Full screen", "SUPER + F"), "Windows");
        assert_eq!(category("Move window to workspace 1", "SUPER + SHIFT + code:10"), "Workspaces & monitors");
        assert_eq!(category("Color picker", "SUPER + PRINT"), "Screenshots & recording");
        assert_eq!(category("Browser (private)", "SUPER + ALT + SHIFT + B"), "Apps & menus");
    }

    #[test]
    fn normalises_typed_keys() {
        assert_eq!(normalise("shift+super + k"), "SUPER + SHIFT + k");
        assert_eq!(normalise("SUPER + ALT + K"), "SUPER + ALT + K");
        assert_eq!(normalise(""), "");
    }
}
