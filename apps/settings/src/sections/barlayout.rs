//! Bar layout: the bar drawn from its real layout, and its left, center and right
//! sections as chips you drag to reorder or move between sections; a click on a chip
//! has the same moves as buttons, a spacer's width, and Remove.
//!
//! Moves and spacer sizes go through `omarchy-bar`, which asks the running
//! shell (it owns the bar config in memory). Adding or removing one spacer
//! among several has no shell command, so those edit shell.json the way
//! `omarchy-bar` itself does for whole-bar changes: an atomic write followed by
//! `omarchy-shell shell reloadConfig`.

use crate::backend::shell;
use crate::widgets;
use crate::{cmd, window};
use gtk::prelude::*;
use gtk::{gdk, glib};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

pub const SECTIONS: [&str; 3] = ["left", "center", "right"];

fn section_title(s: &str) -> &'static str {
    match s {
        "left" => "Left",
        "center" => "Center",
        _ => "Right",
    }
}

/// A widget's place on the bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub section: String,
    pub index: usize,
    pub id: String,
}

pub fn read_layout() -> Vec<Vec<String>> {
    read_entries().iter().map(|s| s.iter().filter_map(|e| entry_id(e).map(String::from)).collect()).collect()
}

pub const SPACER: &str = "omarchy.spacer";
pub const SPACER_DEFAULT: i64 = 12;

/// Each section's raw entries (so per-widget settings like a spacer's size are kept).
fn read_entries() -> Vec<Vec<Value>> {
    SECTIONS.iter().map(|s| shell::get(&["bar", "layout", s]).and_then(|v| v.as_array().cloned()).unwrap_or_default()).collect()
}

fn entry_size(e: &Value) -> i64 {
    e.get("size").and_then(|v| v.as_i64()).unwrap_or(SPACER_DEFAULT)
}

fn entry_id(e: &Value) -> Option<&str> {
    match e {
        Value::String(s) => Some(s),
        Value::Object(o) => o.get("id").and_then(|v| v.as_str()),
        _ => None,
    }
}

/// Add a spacer at the end of a section.
pub fn with_spacer_added(mut config: Value, section: &str, size: i64) -> Option<Value> {
    let list = config.get_mut("bar")?.get_mut("layout")?.get_mut(section)?.as_array_mut()?;
    list.push(serde_json::json!({ "id": SPACER, "size": size }));
    Some(config)
}

/// Remove the entry at `section[index]`, but only if it is `id` (so a stale
/// view can never remove the wrong widget).
pub fn with_entry_removed(mut config: Value, section: &str, index: usize, id: &str) -> Option<Value> {
    let list = config.get_mut("bar")?.get_mut("layout")?.get_mut(section)?.as_array_mut()?;
    if list.get(index).and_then(entry_id) != Some(id) {
        return None;
    }
    list.remove(index);
    Some(config)
}

/// Write shell.json and have the shell reload it, like `omarchy-bar` does.
fn commit_config(config: &Value) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(config)? + "\n";
    cmd::atomic_write(&crate::paths::shell_json(), &text)?;
    if cmd::run(&["omarchy-shell", "shell", "reloadConfig"]).is_err() {
        let _ = cmd::run(&["omarchy-shell", "-q", "shell", "rescanPlugins"]);
    }
    Ok(())
}

/// The `--index` to send so the widget ends up in front of `before` (or at the
/// end when `before` is None). The shell takes the widget out first, so a
/// later position in the same section moves up by one.
pub fn target_index(from: &Slot, to_section: &str, before: Option<usize>, len_to: usize) -> usize {
    let raw = before.unwrap_or(len_to);
    if from.section == to_section && from.index < raw { raw - 1 } else { raw }
}

fn move_args(from: &Slot, to_section: &str, index: usize) -> Vec<String> {
    vec![
        "omarchy-bar".into(),
        "move".into(),
        from.id.clone(),
        "--section".into(),
        to_section.into(),
        "--index".into(),
        index.to_string(),
        "--from-section".into(),
        from.section.clone(),
        "--from-index".into(),
        from.index.to_string(),
    ]
}

/// Widget names and which bar widgets aren't on the bar, from the plugin list.
fn plugins() -> (HashMap<String, String>, Vec<(String, String)>) {
    let list: Vec<Value> = cmd::output(&["omarchy-plugin-list", "--json"])
        .and_then(|t| serde_json::from_str(&t).ok())
        .and_then(|v: Value| v.as_array().cloned())
        .unwrap_or_default();
    let mut names = HashMap::new();
    let mut off_bar = Vec::new();
    for p in &list {
        let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = p.get("name").and_then(|v| v.as_str()).unwrap_or(&id).to_string();
        let is_widget = p.get("kinds").and_then(|k| k.as_array()).is_some_and(|k| k.iter().any(|x| x == "bar-widget"));
        if is_widget && id != SPACER && !p.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false) {
            off_bar.push((id.clone(), name.clone()));
        }
        names.insert(id, name);
    }
    names.entry("omarchy.spacer".into()).or_insert_with(|| "Spacer".into());
    off_bar.sort_by_key(|(_, n)| n.to_lowercase());
    (names, off_bar)
}

// ----- Chips that wrap at their own widths -----

mod flow {
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::glib;
    use std::cell::Cell;

    #[derive(Default)]
    pub struct Imp {
        pub spacing: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Imp {
        const NAME: &'static str = "NyxSettingsChipFlow";
        type Type = Flow;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for Imp {}

    fn children(w: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut v = Vec::new();
        let mut c = w.first_child();
        while let Some(child) = c {
            if child.should_layout() {
                v.push(child.clone());
            }
            c = child.next_sibling();
        }
        v
    }

    /// Each child's place, wrapping at `width`; and the height they take.
    fn lines(w: &gtk::Widget, width: i32, gap: i32) -> (Vec<(gtk::Widget, i32, i32, i32, i32)>, i32) {
        let mut out = Vec::new();
        let (mut x, mut y, mut line_h) = (0, 0, 0);
        for c in children(w) {
            let (_, cw, _, _) = c.measure(gtk::Orientation::Horizontal, -1);
            let cw = cw.min(width.max(1));
            let (_, ch, _, _) = c.measure(gtk::Orientation::Vertical, cw);
            if x > 0 && x + cw > width {
                x = 0;
                y += line_h + gap;
                line_h = 0;
            }
            out.push((c, x, y, cw, ch));
            x += cw + gap;
            line_h = line_h.max(ch);
        }
        (out, y + line_h)
    }

    /// The widest child, and a modest natural width: wrapping content grows taller instead.
    fn widths(w: &gtk::Widget, gap: i32) -> (i32, i32) {
        let kids = children(w);
        let min = kids.iter().map(|c| c.measure(gtk::Orientation::Horizontal, -1).0).max().unwrap_or(0);
        let nat: i32 = kids.iter().map(|c| c.measure(gtk::Orientation::Horizontal, -1).1).sum::<i32>() + gap * (kids.len() as i32 - 1).max(0);
        (min, nat.min(320).max(min))
    }

    impl LayoutManagerImpl for Imp {
        fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, widget: &gtk::Widget, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let (min_w, nat_w) = widths(widget, self.spacing.get());
            if orientation == gtk::Orientation::Horizontal {
                (min_w, nat_w, -1, -1)
            } else {
                let width = if for_size < 0 { nat_w } else { for_size.max(min_w) };
                let (_, h) = lines(widget, width, self.spacing.get());
                (h, h, -1, -1)
            }
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, _height: i32, _baseline: i32) {
            for (c, x, y, w, h) in lines(widget, width, self.spacing.get()).0 {
                c.size_allocate(&gtk::Allocation::new(x, y, w, h), -1);
            }
        }
    }

    glib::wrapper! {
        pub struct Flow(ObjectSubclass<Imp>) @extends gtk::LayoutManager;
    }

    impl Flow {
        pub fn new(spacing: i32) -> Self {
            let f: Self = glib::Object::new();
            f.imp().spacing.set(spacing);
            f
        }
    }
}

/// A box whose children wrap onto more lines at their own widths, like words.
pub fn wrap_box(spacing: i32) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.set_layout_manager(Some(flow::Flow::new(spacing)));
    b
}

// ----- How a widget looks in the bar -----

/// What a bar widget shows, as near as Settings can draw it.
pub enum Glyph {
    Icon(&'static str),
    Workspaces,
    Clock,
    Text(String),
    Spacer(i64),
}

/// The glyph for a bar widget (Omarchy's or Nyx Bar's) by its id; `None` for a
/// plugin Settings doesn't know, which shows its name.
pub fn glyph(id: &str) -> Option<Glyph> {
    let tail = id.rsplit(['.', '/']).next().unwrap_or(id);
    let icon = |name| Some(Glyph::Icon(name));
    match tail {
        "workspaces" => Some(Glyph::Workspaces),
        "clock" | "time" => Some(Glyph::Clock),
        "menu" | "launcher" => icon("start-here-symbolic"),
        "active-window" | "window" => Some(Glyph::Text("Window".into())),
        "tray" => icon("view-more-horizontal-symbolic"),
        "network" => icon("network-wireless-symbolic"),
        "bluetooth" => icon("bluetooth-active-symbolic"),
        "audio" | "volume" => icon("audio-volume-high-symbolic"),
        "power" | "battery" => icon("battery-good-symbolic"),
        "monitor" => icon("video-display-symbolic"),
        "tailscale" | "privacy-route" => icon("network-vpn-symbolic"),
        "dropbox" => icon("folder-remote-symbolic"),
        "indicators" | "recording" | "privacy" => icon("media-record-symbolic"),
        "system-update" | "updates" => icon("software-update-available-symbolic"),
        "keyboard-layout" | "keyboard" => icon("input-keyboard-symbolic"),
        "media" => icon("audio-x-generic-symbolic"),
        "weather" | "meteobar" | "now" => icon("weather-few-clouds-symbolic"),
        "notifications" | "notification-center" => icon("preferences-system-notifications-symbolic"),
        "control-center" => icon("view-grid-symbolic"),
        "keepawake" => icon("night-light-symbolic"),
        "stats" | "system-monitor" | "status" => icon("display-brightness-symbolic"),
        "system" => icon("network-wireless-symbolic"),
        _ => None,
    }
}

/// A Qt date format (Omarchy's clock) as a strftime one, for the clock in the picture.
fn qt_to_strftime(qt: &str) -> String {
    let twelve = qt.contains("AP") || qt.contains("ap");
    let tokens: [(&str, &str); 17] = [
        ("dddd", "%A"),
        ("ddd", "%a"),
        ("dd", "%d"),
        ("d", "%-d"),
        ("MMMM", "%B"),
        ("MMM", "%b"),
        ("MM", "%m"),
        ("M", "%-m"),
        ("yyyy", "%Y"),
        ("yy", "%y"),
        ("HH", "%H"),
        ("H", "%-H"),
        ("hh", if twelve { "%I" } else { "%H" }),
        ("h", if twelve { "%-I" } else { "%-H" }),
        ("mm", "%M"),
        ("ss", "%S"),
        ("AP", "%p"),
    ];
    let mut out = String::new();
    let mut rest = qt;
    'outer: while !rest.is_empty() {
        for (t, f) in tokens {
            if let Some(r) = rest.strip_prefix(t) {
                out.push_str(f);
                rest = r;
                continue 'outer;
            }
        }
        if let Some(r) = rest.strip_prefix("ap") {
            out.push_str("%P");
            rest = r;
            continue;
        }
        let c = rest.chars().next().unwrap_or(' ');
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

pub fn now(format: &str) -> String {
    glib::DateTime::now_local().ok().and_then(|d| d.format(format).ok()).map(|s| s.to_string()).unwrap_or_default()
}

/// The battery's charge, for the battery in the picture.
fn battery_percent() -> Option<String> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    dir.flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("BAT"))
        .find_map(|e| std::fs::read_to_string(e.path().join("capacity")).ok())
        .map(|c| format!("{}%", c.trim()))
}

/// One widget as the bar shows it: an icon, the workspaces, the clock, or its name.
pub fn glyph_widget(g: Option<Glyph>, name: &str, clock_format: &str, vertical: bool) -> gtk::Widget {
    match g {
        Some(Glyph::Icon(icon)) => {
            let i = gtk::Image::from_icon_name(icon);
            i.set_pixel_size(14);
            i.upcast()
        }
        Some(Glyph::Workspaces) => {
            let b = gtk::Box::new(if vertical { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal }, 2);
            for n in 1..=4 {
                let l = gtk::Label::new(Some(&n.to_string()));
                l.add_css_class("bar-ws");
                if n == 1 {
                    l.add_css_class("on");
                }
                b.append(&l);
            }
            b.upcast()
        }
        Some(Glyph::Clock) => {
            let l = gtk::Label::new(Some(&now(if vertical { "%H\n%M" } else { clock_format })));
            l.add_css_class("bar-clock");
            l.set_justify(gtk::Justification::Center);
            if !vertical {
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            }
            l.upcast()
        }
        Some(Glyph::Text(t)) => {
            let l = gtk::Label::new(Some(&t));
            l.add_css_class("bar-text");
            l.upcast()
        }
        Some(Glyph::Spacer(size)) => {
            let s = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let px = (size as f64 * 0.5).round() as i32;
            if vertical {
                s.set_size_request(-1, px);
            } else {
                s.set_size_request(px, -1);
            }
            s.upcast()
        }
        None => {
            let l = gtk::Label::new(Some(name));
            l.add_css_class("bar-text");
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_width_chars(3);
            l.set_max_width_chars(12);
            l.upcast()
        }
    }
}

// ----- The editor -----

struct Editor {
    /// The bar as drawn over the screen.
    strip: gtk::Box,
    /// The three sections as chips.
    zones: gtk::Box,
    /// Widgets that aren't on the bar, to add.
    add: gtk::Box,
    busy: Cell<bool>,
    position: RefCell<String>,
    /// Widget names, from the last plugin list.
    names: RefCell<HashMap<String, String>>,
    /// Called after the bar changes, to redraw the screen around it.
    after: RefCell<Option<Box<dyn Fn()>>>,
}

type Ed = Rc<Editor>;

fn run_then_refresh(ed: &Ed, args: Vec<String>) {
    if ed.busy.replace(true) {
        return;
    }
    let ed = ed.clone();
    let label = args[..3.min(args.len())].join(" ");
    cmd::background(
        move || {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            cmd::run(&refs)
        },
        move |r| {
            if let Err(e) = r {
                window::toast(&format!("{label}: {e}"));
            }
            // The shell writes its config a moment after answering.
            glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                ed.busy.set(false);
                refresh(&ed);
            });
        },
    );
}

/// Like [`run_then_refresh`], for a shell.json edit.
fn edit_then_refresh(ed: &Ed, change: impl FnOnce(Value) -> Option<Value> + Send + 'static) {
    if ed.busy.replace(true) {
        return;
    }
    let ed = ed.clone();
    cmd::background(
        move || {
            let next = change(shell::read()).ok_or_else(|| "the bar changed meanwhile; try again".to_string())?;
            commit_config(&next).map_err(|e| format!("{e:#}"))
        },
        move |r| {
            if let Err(e) = r {
                window::toast(&format!("Couldn't change the bar: {e}"));
            }
            glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                ed.busy.set(false);
                refresh(&ed);
            });
        },
    );
}

fn encode(s: &Slot) -> String {
    format!("{}\t{}\t{}", s.section, s.index, s.id)
}

fn decode(v: &str) -> Option<Slot> {
    let mut p = v.splitn(3, '\t');
    Some(Slot { section: p.next()?.to_string(), index: p.next()?.parse().ok()?, id: p.next()?.to_string() })
}

fn clear_drop_marks(w: &gtk::Widget) {
    w.remove_css_class("drop-before");
    w.remove_css_class("drop-after");
}

fn flat(label: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("flat");
    if let Some(l) = b.child().and_downcast::<gtk::Label>() {
        l.set_xalign(0.0);
    }
    b
}

/// What a click on a chip offers: its moves, a spacer's width, Remove.
fn chip_menu(ed: &Ed, slot: &Slot, name: &str, len: usize, anchor: bool, size: i64) -> gtk::Popover {
    let pop = gtk::Popover::new();
    let items = widgets::vbox(2);
    items.add_css_class("ob-menu");
    let head = widgets::label(name, "ob-menu-title");
    items.append(&head);
    let id = widgets::label(&slot.id, "ob-menu-id");
    id.add_css_class("mono");
    items.append(&id);
    if anchor {
        items.append(&widgets::label("The bar keeps this widget in the middle of the screen.", "ob-menu-id"));
    }
    if slot.id == SPACER {
        let spin = gtk::SpinButton::with_range(0.0, 400.0, 2.0);
        spin.set_value(size as f64);
        spin.set_width_chars(4);
        spin.set_tooltip_text(Some("Width of the gap, in pixels"));
        let debounce = widgets::Debounce::default();
        let slot2 = slot.clone();
        let ed2 = ed.clone();
        spin.connect_value_changed(move |s| {
            let (v, slot, ed) = (s.value().round() as i64, slot2.clone(), ed2.clone());
            debounce.call(400, move || {
                let args = [
                    "omarchy-bar".to_string(),
                    "set".into(),
                    SPACER.into(),
                    "size".into(),
                    v.to_string(),
                    "--json".into(),
                    "--from-section".into(),
                    slot.section.clone(),
                    "--from-index".into(),
                    slot.index.to_string(),
                ];
                cmd::background(
                    move || {
                        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                        cmd::run(&refs)
                    },
                    move |r| {
                        if let Err(e) = r {
                            window::toast(&format!("Couldn't resize the spacer: {e}"));
                        }
                        refresh_strip(&ed);
                    },
                );
            });
        });
        let line = widgets::hbox(8);
        line.append(&widgets::label("Width", ""));
        line.append(&spin);
        line.append(&widgets::label("px", "dim"));
        items.append(&line);
    }
    let add = |label: &str, args: Vec<String>| {
        let b = flat(label);
        let (ed, pop) = (ed.clone(), pop.clone());
        b.connect_clicked(move |_| {
            pop.popdown();
            run_then_refresh(&ed, args.clone());
        });
        items.append(&b);
    };
    if slot.index > 0 {
        add("Move earlier", move_args(slot, &slot.section, slot.index - 1));
    }
    if slot.index + 1 < len {
        add("Move later", move_args(slot, &slot.section, slot.index + 1));
    }
    let layout = read_layout();
    for (i, s) in SECTIONS.iter().enumerate().filter(|(_, s)| **s != slot.section) {
        add(&format!("Move to {}", section_title(s)), move_args(slot, s, layout.get(i).map_or(0, Vec::len)));
    }
    let remove = flat("Remove from bar");
    remove.add_css_class("ob-remove");
    {
        let (ed, slot, pop) = (ed.clone(), slot.clone(), pop.clone());
        remove.connect_clicked(move |_| {
            pop.popdown();
            if slot.id == SPACER {
                // Turning the spacer plugin off would take every spacer away; remove just this one.
                let slot = slot.clone();
                edit_then_refresh(&ed, move |c| with_entry_removed(c, &slot.section, slot.index, SPACER));
            } else {
                run_then_refresh(&ed, vec!["omarchy-plugin-disable".into(), slot.id.clone()]);
            }
        });
    }
    items.append(&remove);
    pop.set_child(Some(&items));
    pop
}

fn chip(ed: &Ed, slot: Slot, name: &str, len: usize, anchor: bool, size: i64, clock: &str) -> gtk::MenuButton {
    let b = gtk::MenuButton::new();
    b.add_css_class("ob-chip");
    let content = widgets::hbox(4);
    let g = if slot.id == SPACER { Some(Glyph::Text(format!("{size} px"))) } else { glyph(&slot.id) };
    if slot.id == SPACER {
        b.add_css_class("spacer");
    }
    content.append(&glyph_widget(g, name, clock, false));
    b.set_child(Some(&content));
    b.set_tooltip_text(Some(name));
    b.set_popover(Some(&chip_menu(ed, &slot, name, len, anchor, size)));

    let drag = gtk::DragSource::new();
    drag.set_actions(gdk::DragAction::MOVE);
    {
        let payload = encode(&slot);
        drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&payload.to_value())));
        let w = b.clone();
        drag.connect_drag_begin(move |src, _| {
            src.set_icon(Some(&gtk::WidgetPaintable::new(Some(&w))), 12, 12);
            w.add_css_class("dragging");
        });
        let w = b.clone();
        drag.connect_drag_end(move |_, _, _| w.remove_css_class("dragging"));
    }
    b.add_controller(drag);

    // Drop another chip on this one: on its left half goes before, on its right half after.
    let drop = gtk::DropTarget::new(glib::types::Type::STRING, gdk::DragAction::MOVE);
    {
        let w = b.clone();
        drop.connect_motion(move |_, x, _| {
            clear_drop_marks(w.upcast_ref());
            w.add_css_class(if x < w.width() as f64 / 2.0 { "drop-before" } else { "drop-after" });
            gdk::DragAction::MOVE
        });
        let w = b.clone();
        drop.connect_leave(move |_| clear_drop_marks(w.upcast_ref()));
        let (w, ed, slot) = (b.clone(), ed.clone(), slot.clone());
        drop.connect_drop(move |_, value, x, _| {
            clear_drop_marks(w.upcast_ref());
            let Some(from) = value.get::<String>().ok().as_deref().and_then(decode) else { return false };
            let before = if x < w.width() as f64 / 2.0 { slot.index } else { slot.index + 1 };
            if from.section == slot.section && (before == from.index || before == from.index + 1) {
                return true; // dropped where it already is
            }
            let len_to = read_layout()[SECTIONS.iter().position(|x| *x == slot.section).unwrap_or(0)].len();
            let idx = target_index(&from, &slot.section, Some(before), len_to);
            run_then_refresh(&ed, move_args(&from, &slot.section, idx));
            true
        });
    }
    b.add_controller(drop);
    b
}

fn zone(ed: &Ed, section: &str, ids: &[String], entries: &[Value], names: &HashMap<String, String>, anchor: &str, clock: &str) -> gtk::Box {
    let row = widgets::vbox(0);
    row.add_css_class("ob-zone");
    let flow = wrap_box(4);
    let head = widgets::hbox(4);
    head.add_css_class("ob-zone-head");
    head.set_valign(gtk::Align::Center);
    head.append(&widgets::label(section_title(section), "ob-zone-title"));
    head.append(&widgets::label(&ids.len().to_string(), "ob-zone-count"));
    flow.append(&head);
    for (i, id) in ids.iter().enumerate() {
        let name = names.get(id).cloned().unwrap_or_else(|| id.clone());
        let slot = Slot { section: section.to_string(), index: i, id: id.clone() };
        let size = entries.get(i).map(entry_size).unwrap_or(SPACER_DEFAULT);
        flow.append(&chip(ed, slot, &name, ids.len(), section == "center" && id == anchor, size, clock));
    }
    if ids.is_empty() {
        flow.append(&widgets::label("Drop widgets here", "ob-drop-end"));
    }
    row.append(&flow);

    // Dropped on the row but not on a chip: at the end of this section.
    let drop = gtk::DropTarget::new(glib::types::Type::STRING, gdk::DragAction::MOVE);
    {
        let r = row.clone();
        drop.connect_enter(move |_, _, _| {
            r.add_css_class("drop-end");
            gdk::DragAction::MOVE
        });
        let r = row.clone();
        drop.connect_leave(move |_| r.remove_css_class("drop-end"));
        let (r, ed, section, len) = (row.clone(), ed.clone(), section.to_string(), ids.len());
        drop.connect_drop(move |_, value, _, _| {
            r.remove_css_class("drop-end");
            let Some(from) = value.get::<String>().ok().as_deref().and_then(decode) else { return false };
            let idx = target_index(&from, &section, None, len);
            run_then_refresh(&ed, move_args(&from, &section, idx));
            true
        });
    }
    row.add_controller(drop);
    row
}

/// A chip that adds something to a section of your choice.
fn add_chip(label: &str, on_add: impl Fn(&'static str) + 'static) -> gtk::MenuButton {
    let b = gtk::MenuButton::new();
    b.add_css_class("ob-add-chip");
    let c = widgets::hbox(4);
    c.append(&gtk::Image::from_icon_name("list-add-symbolic"));
    let l = gtk::Label::new(Some(label));
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_width_chars(4);
    c.append(&l);
    b.set_child(Some(&c));
    let pop = gtk::Popover::new();
    let items = widgets::vbox(2);
    items.add_css_class("ob-menu");
    items.append(&widgets::label("Add to", "ob-menu-title"));
    let on_add = Rc::new(on_add);
    for s in SECTIONS {
        let item = flat(section_title(s));
        let (on_add, pop) = (on_add.clone(), pop.clone());
        item.connect_clicked(move |_| {
            pop.popdown();
            on_add(s);
        });
        items.append(&item);
    }
    pop.set_child(Some(&items));
    b.set_popover(Some(&pop));
    b
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn clock_format() -> String {
    let center = shell::get(&["bar", "layout", "center"]).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let all = read_entries().into_iter().flatten().chain(center);
    all.into_iter()
        .find(|e| entry_id(e) == Some("omarchy.clock"))
        .and_then(|e| e.get("format").and_then(|f| f.as_str()).map(qt_to_strftime))
        .unwrap_or_else(|| "%a %H:%M".into())
}

/// The bar over the screen: the three sections with every widget's glyph.
fn refresh_strip(ed: &Ed) {
    let strip = &ed.strip;
    clear(strip);
    let vertical = matches!(ed.position.borrow().as_str(), "left" | "right");
    strip.set_orientation(if vertical { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal });
    let entries = read_entries();
    let names = ed.names.borrow().clone();
    let clock = clock_format();
    for (i, section) in entries.iter().enumerate() {
        let part = gtk::Box::new(strip.orientation(), 7);
        part.add_css_class("ob-strip-part");
        for e in section {
            let Some(id) = entry_id(e) else { continue };
            let name = names.get(id).cloned().unwrap_or_else(|| id.to_string());
            let g = if id == SPACER { Some(Glyph::Spacer(entry_size(e))) } else { glyph(id) };
            // Plugins Settings doesn't know show as a plugin in the picture.
            let g = g.or(Some(Glyph::Icon("application-x-addon-symbolic")));
            let w = glyph_widget(g, &name, &clock, vertical);
            if matches!(id, "omarchy.power") && !vertical && e.get("showPercentage").and_then(Value::as_bool).unwrap_or(false) {
                let b = widgets::hbox(3);
                b.append(&w);
                if let Some(p) = battery_percent() {
                    b.append(&widgets::label(&p, "bar-text"));
                }
                b.set_tooltip_text(Some(&name));
                part.append(&b);
            } else {
                w.set_tooltip_text(Some(&name));
                part.append(&w);
            }
        }
        match i {
            0 => part.set_halign(gtk::Align::Start),
            1 => {
                part.set_hexpand(!vertical);
                part.set_vexpand(vertical);
                part.set_halign(gtk::Align::Center);
                part.set_valign(gtk::Align::Center);
            }
            _ => {
                part.set_halign(if vertical { gtk::Align::Center } else { gtk::Align::End });
                part.set_valign(if vertical { gtk::Align::End } else { gtk::Align::Center });
            }
        }
        if vertical {
            part.set_halign(gtk::Align::Center);
        }
        strip.append(&part);
    }
}

fn refresh(ed: &Ed) {
    let (names, off_bar) = plugins();
    *ed.names.borrow_mut() = names.clone();
    refresh_strip(ed);
    clear(&ed.zones);
    clear(&ed.add);
    let layout = read_layout();
    let entries = read_entries();
    let anchor = shell::get(&["bar", "centerAnchor"]).and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
    let clock = clock_format();
    for (i, s) in SECTIONS.iter().enumerate() {
        ed.zones.append(&zone(ed, s, &layout[i], &entries[i], &names, &anchor, &clock));
    }

    // Add a widget that isn't on the bar, or a spacer.
    let flow = wrap_box(6);
    let label = widgets::label("Add", "ob-add-label");
    label.set_valign(gtk::Align::Center);
    flow.append(&label);
    for (id, name) in off_bar {
        let ed2 = ed.clone();
        flow.append(&add_chip(&name, move |s| {
            run_then_refresh(&ed2, vec!["omarchy-bar".into(), "put".into(), id.clone(), "--section".into(), s.into()])
        }));
    }
    let ed2 = ed.clone();
    flow.append(&add_chip("Spacer", move |s| edit_then_refresh(&ed2, move |c| with_spacer_added(c, s, SPACER_DEFAULT))));
    ed.add.append(&flow);
    if let Some(f) = ed.after.borrow().as_ref() {
        f();
    }
}

/// The editor's parts, placed by the page: the bar strip (over a drawing of the
/// screen), the sections, and the widgets to add.
pub struct View {
    ed: Ed,
    pub strip: gtk::Box,
    pub zones: gtk::Box,
    pub add: gtk::Box,
}

impl View {
    /// Where the bar sits: top, bottom, left or right.
    pub fn set_position(&self, position: &str) {
        *self.ed.position.borrow_mut() = position.to_string();
        refresh_strip(&self.ed);
    }
    /// Called whenever the bar changes.
    pub fn on_change(&self, f: impl Fn() + 'static) {
        *self.ed.after.borrow_mut() = Some(Box::new(f));
    }
}

pub fn view(position: &str) -> View {
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    strip.add_css_class("ob-strip");
    let zones = widgets::vbox(0);
    zones.add_css_class("ob-zones");
    let add = widgets::vbox(0);
    add.add_css_class("ob-add");
    let ed: Ed = Rc::new(Editor {
        strip: strip.clone(),
        zones: zones.clone(),
        add: add.clone(),
        busy: Cell::new(false),
        position: RefCell::new(position.to_string()),
        names: RefCell::default(),
        after: RefCell::default(),
    });
    refresh(&ed);
    View { ed, strip, zones, add }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(section: &str, index: usize) -> Slot {
        Slot { section: section.into(), index, id: "x".into() }
    }

    #[test]
    fn moving_down_in_the_same_section_accounts_for_removal() {
        // [a b c d]: drop b (1) before d (3) -> ends at index 2.
        assert_eq!(target_index(&slot("left", 1), "left", Some(3), 4), 2);
        // Drop b at the end -> index 3 (last).
        assert_eq!(target_index(&slot("left", 1), "left", None, 4), 3);
    }

    #[test]
    fn moving_up_or_across_sections_uses_the_position_as_is() {
        assert_eq!(target_index(&slot("left", 3), "left", Some(0), 4), 0);
        assert_eq!(target_index(&slot("left", 1), "right", Some(2), 5), 2);
        assert_eq!(target_index(&slot("left", 1), "right", None, 5), 5);
    }

    #[test]
    fn move_targets_the_exact_entry() {
        let a = move_args(&Slot { section: "left".into(), index: 2, id: "omarchy.spacer".into() }, "right", 0);
        assert_eq!(
            a,
            [
                "omarchy-bar",
                "move",
                "omarchy.spacer",
                "--section",
                "right",
                "--index",
                "0",
                "--from-section",
                "left",
                "--from-index",
                "2"
            ]
        );
    }

    fn config() -> Value {
        serde_json::json!({
            "version": 1,
            "bar": { "position": "top", "layout": {
                "left": [{ "id": "omarchy.menu" }, { "id": "omarchy.spacer", "size": 20 }],
                "center": [{ "id": "omarchy.clock" }],
                "right": []
            }},
            "plugins": []
        })
    }

    #[test]
    fn adds_a_spacer_at_the_end_keeping_everything_else() {
        let c = with_spacer_added(config(), "right", 12).unwrap();
        assert_eq!(c["bar"]["layout"]["right"][0], serde_json::json!({ "id": "omarchy.spacer", "size": 12 }));
        assert_eq!(c["bar"]["layout"]["left"].as_array().unwrap().len(), 2);
        assert_eq!(c["bar"]["position"], "top");
        // A second spacer in the same section is fine.
        let c = with_spacer_added(c, "left", 8).unwrap();
        assert_eq!(c["bar"]["layout"]["left"][2]["size"], 8);
    }

    #[test]
    fn removes_only_the_expected_entry() {
        let c = with_entry_removed(config(), "left", 1, "omarchy.spacer").unwrap();
        assert_eq!(c["bar"]["layout"]["left"].as_array().unwrap().len(), 1);
        assert_eq!(c["bar"]["layout"]["left"][0]["id"], "omarchy.menu");
        // Wrong widget at that index (the layout changed meanwhile): nothing is removed.
        assert!(with_entry_removed(config(), "left", 0, "omarchy.spacer").is_none());
        assert!(with_entry_removed(config(), "left", 9, "omarchy.spacer").is_none());
    }

    #[test]
    fn spacer_size_defaults() {
        assert_eq!(entry_size(&serde_json::json!({ "id": "omarchy.spacer" })), 12);
        assert_eq!(entry_size(&serde_json::json!({ "id": "omarchy.spacer", "size": 40 })), 40);
    }

    #[test]
    fn drag_payload_round_trips() {
        let s = Slot { section: "center".into(), index: 4, id: "omarchy.clock".into() };
        assert_eq!(decode(&encode(&s)), Some(s));
        assert_eq!(decode("junk"), None);
    }

    #[test]
    fn reads_qt_clock_formats() {
        assert_eq!(qt_to_strftime("ddd d MMM h:mm AP"), "%a %-d %b %-I:%M %p");
        assert_eq!(qt_to_strftime("HH:mm"), "%H:%M");
    }
}
