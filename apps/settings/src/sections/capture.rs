//! Screenshots, screen recording, reminders, weather, dictation and small tools.
//! Mostly buttons for Omarchy's own commands, with the choices that go with them.
//!
//! Screenshot and Recording are the two halves of the page: the mode and where it
//! goes on the left, a small screen showing it in action on the right. The helpers
//! are a More tools card, each opening its own page.

use super::keyboard::{fit, hold_controls, more_card, more_page, tame_head, tiled};
use crate::backend::hypr;
use crate::sections::accounts::terminal;
use crate::sketch::{self, Pal, R, Rgb};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::cairo::Context;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::process::{Command, Stdio};
use std::rc::Rc;

thread_local! {
    /// The half shown, kept across a rebuild of the page.
    static TAB: RefCell<String> = RefCell::new("shot".into());
}

/// Run a command after a short pause, so Settings can get out of the picture first.
fn later(args: Vec<String>) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(700));
        if let Some((program, rest)) = args.split_first() {
            let _ = Command::new(program).args(rest).stdin(Stdio::null()).status();
        }
    });
}

/// Ask the compositor to hide this window for a moment, then run `args`.
fn hide_and_run(args: Vec<String>) {
    if let Some(win) = window::window() {
        win.minimize();
    }
    later(args);
}

/// The keys of the shortcut Hyprland describes as `description`, the shortest if
/// there are several.
fn bind_for(description: &str) -> Option<String> {
    let serde_json::Value::Array(items) = hypr::json(&["binds"])? else { return None };
    items
        .iter()
        .filter(|b| b.get("description").and_then(|d| d.as_str()).is_some_and(|d| d.eq_ignore_ascii_case(description)))
        .filter_map(|b| {
            let key = b.get("key")?.as_str().filter(|k| !k.is_empty())?;
            let mask = b.get("modmask")?.as_u64()?;
            Some((mask.count_ones(), super::keybindings::keys_string(mask, key)))
        })
        .min_by_key(|(n, _)| *n)
        .map(|(_, k)| super::keybindings::pretty_keys(&k))
}

fn caps(keys: &str) -> gtk::Box {
    let b = widgets::hbox(4);
    for part in keys.split(" + ") {
        b.append(&widgets::label(part, "key-cap"));
    }
    b.set_valign(gtk::Align::Center);
    b
}

// ----- Screenshots -----

/// The screenshot modes: Omarchy's name, the tile's label and icon, and what happens.
const MODES: [(&str, &str, &str, &str); 5] = [
    ("smart", "Smart", "starred-symbolic", "Settings steps aside, then you click a window or drag a region."),
    ("region", "Region", "edit-select-all-symbolic", "Settings steps aside, then you drag over what you want."),
    ("windows", "Window", "window-restore-symbolic", "Settings steps aside, then you click the window you want."),
    ("fullscreen", "Screen", "video-display-symbolic", "Settings steps aside, and the whole screen is captured."),
    ("scroll", "Scrolling", "go-down-symbolic", "Scroll through a page longer than the screen; it's put together as one picture."),
];

/// Nyx Capture's `flag` when it is installed, else Omarchy's `script`.
fn capture_tool(flag: &str, script: &str) -> Option<Vec<String>> {
    if cmd::present("nyx-capture") {
        Some(vec!["nyx-capture".into(), flag.into()])
    } else if cmd::present(script) {
        Some(vec![script.into()])
    } else {
        None
    }
}

/// The three windows `sketch::desktop` draws in `area`: left, top right, bottom right.
fn desktop_windows(r: R, area: R) -> (R, R, R) {
    let (left, right) = area.hsplit(0.52, (r.w * 0.01).clamp(3.0, 8.0));
    let (top, bottom) = right.vsplit(0.48, (r.w * 0.01).clamp(3.0, 8.0));
    (left, top, bottom)
}

/// A dark pill with white words, centred on `x`.
fn pill(cr: &Context, text: &str, x: f64, y: f64, dot: Option<Rgb>) {
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
    cr.set_font_size(11.0);
    let Ok(ext) = cr.text_extents(text) else { return };
    let extra = if dot.is_some() { 14.0 } else { 0.0 };
    let (pw, ph) = (ext.x_advance() + 18.0 + extra, 24.0);
    let px = x - pw / 2.0;
    sketch::set(cr, (0.0, 0.0, 0.0), 0.62);
    sketch::rounded(cr, px, y, pw, ph, 7.0);
    let _ = cr.fill();
    if let Some(c) = dot {
        sketch::set(cr, c, 1.0);
        cr.arc(px + 13.0, y + ph / 2.0, 4.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
    sketch::set(cr, (1.0, 1.0, 1.0), 0.95);
    cr.move_to(px + 9.0 + extra - ext.x_bearing(), y + ph / 2.0 - ext.height() / 2.0 - ext.y_bearing());
    let _ = cr.show_text(text);
}

/// What's picked: a dimmed screen around it, a dashed outline and corner handles.
fn selection(cr: &Context, r: R, sel: R, sec: Rgb, handles: bool) {
    cr.save().ok();
    cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
    sketch::rounded(cr, r.x, r.y, r.w, r.h, 8.0);
    cr.rectangle(sel.x, sel.y, sel.w, sel.h);
    sketch::set(cr, (0.0, 0.0, 0.0), 0.32);
    let _ = cr.fill();
    cr.restore().ok();
    cr.set_dash(&[5.0, 4.0], 0.0);
    cr.set_line_width(1.6);
    sketch::set(cr, sec, 1.0);
    cr.rectangle(sel.x, sel.y, sel.w, sel.h);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    if handles {
        for (x, y) in [(sel.x, sel.y), (sel.x + sel.w, sel.y), (sel.x, sel.y + sel.h), (sel.x + sel.w, sel.y + sel.h)] {
            sketch::set(cr, (1.0, 1.0, 1.0), 1.0);
            cr.rectangle(x - 4.0, y - 4.0, 8.0, 8.0);
            let _ = cr.fill_preserve();
            sketch::set(cr, sec, 1.0);
            cr.set_line_width(1.2);
            let _ = cr.stroke();
        }
    }
}

fn draw_shot(cr: &Context, p: &Pal, sec: Rgb, w: f64, h: f64, mode: &str) {
    let r = R::new(1.0, 1.0, w - 2.0, h - 2.0);
    let area = sketch::desktop(cr, p, r);
    let (left, top, _) = desktop_windows(r, area);
    match mode {
        "region" => {
            let sel = R::new(area.x + area.w * 0.22, area.y + area.h * 0.18, area.w * 0.5, area.h * 0.55);
            selection(cr, r, sel, sec, true);
            pill(cr, "Drag over what you want", sel.cx(), sel.y + sel.h + 10.0, None);
        }
        "windows" => {
            selection(cr, r, left.inset(-3.0), sec, false);
            pill(cr, "Click the window you want", left.cx(), left.cy() - 12.0, None);
        }
        "fullscreen" => {
            sketch::set(cr, sec, 1.0);
            cr.set_line_width(3.0);
            sketch::rounded(cr, r.x + 1.5, r.y + 1.5, r.w - 3.0, r.h - 3.0, 8.0);
            let _ = cr.stroke();
            pill(cr, "The whole screen", r.cx(), r.cy() - 12.0, None);
        }
        "scroll" => {
            // The page goes on past the bottom of the window.
            let sel = R::new(top.x, top.y, top.w, area.y + area.h - top.y);
            selection(cr, r, sel.inset(-3.0), sec, false);
            sketch::set(cr, sec, 1.0);
            cr.set_line_width(2.0);
            let (x, y0, y1) = (sel.cx(), sel.cy() - 10.0, sel.cy() + 18.0);
            cr.move_to(x, y0);
            cr.line_to(x, y1);
            cr.move_to(x - 6.0, y1 - 6.0);
            cr.line_to(x, y1);
            cr.line_to(x + 6.0, y1 - 6.0);
            let _ = cr.stroke();
            pill(cr, "Scroll down the page", sel.cx(), sel.cy() + 26.0, None);
        }
        _ => {
            selection(cr, r, left.inset(-4.0), sec, true);
            pill(cr, "Click a window, or drag a region", left.x + left.w * 0.95, left.y + left.h * 0.78, None);
        }
    }
}

struct Shot {
    mode: Rc<RefCell<String>>,
    save: Cell<bool>,
    title: gtk::Label,
    line: gtk::Label,
    area: gtk::DrawingArea,
    tiles: Vec<(String, gtk::ToggleButton)>,
    hud: Vec<(String, gtk::ToggleButton)>,
    target: gtk::DropDown,
    radios: (gtk::CheckButton, gtk::CheckButton),
    syncing: Cell<bool>,
}

fn set_mode(s: &Shot, mode: &str) {
    if s.syncing.get() {
        return;
    }
    s.syncing.set(true);
    *s.mode.borrow_mut() = mode.to_string();
    for (m, b) in s.tiles.iter().chain(s.hud.iter()) {
        b.set_active(m == mode);
    }
    if let Some((_, name, _, line)) = MODES.iter().find(|(m, ..)| *m == mode) {
        s.title.set_text(&format!("{name} screenshot"));
        s.line.set_text(line);
    }
    s.area.queue_draw();
    s.syncing.set(false);
}

fn set_save(s: &Shot, save: bool) {
    if s.syncing.get() {
        return;
    }
    s.syncing.set(true);
    s.save.set(save);
    s.target.set_selected(save as u32);
    if save {
        s.radios.1.set_active(true);
    } else {
        s.radios.0.set_active(true);
    }
    s.syncing.set(false);
}

/// A tile for a mode: its icon over its name.
fn mode_tile(icon: &str, name: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.add_css_class("cp-mode");
    let col = widgets::vbox(4);
    let i = gtk::Image::from_icon_name(icon);
    i.set_pixel_size(18);
    col.append(&i);
    let l = widgets::label(name, "cp-mode-name");
    l.set_xalign(0.5);
    col.append(&l);
    b.set_child(Some(&col));
    b.set_hexpand(true);
    b
}

/// A card for a small screen tool: its icon, name and what to do.
fn tool_card(icon: &str, title: &str, desc: &str, args: Vec<String>) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("cp-tool");
    let row = widgets::hbox(10);
    row.append(&widgets::tile(icon));
    let text = widgets::vbox(2);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(title, "cp-tool-title");
    t.set_xalign(0.0);
    text.append(&t);
    let d = widgets::label(desc, "cp-tool-desc");
    d.set_wrap(true);
    d.set_xalign(0.0);
    d.set_max_width_chars(22);
    text.append(&d);
    row.append(&text);
    b.set_child(Some(&row));
    b.set_hexpand(true);
    b.connect_clicked(move |_| hide_and_run(args.clone()));
    b
}

/// The screenshot half: returns its groups (left) and its picture (right).
fn screenshots(page: &Page, pv_box: &gtk::Box) -> Vec<gtk::Widget> {
    let mut groups = Vec::new();
    let g = page.group("What to capture");
    groups.push(g.wrapper.clone().upcast());
    let tiles_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    tiles_box.set_homogeneous(true);
    tiles_box.add_css_class("cp-modes");
    let mut tiles = Vec::new();
    for (m, name, icon, _) in MODES {
        let b = mode_tile(icon, name);
        if let Some((_, f)) = tiles.first() {
            b.set_group(Some(f));
        }
        tiles_box.append(&b);
        tiles.push((m.to_string(), b));
    }
    g.add(&tiles_box);
    widgets::keywords("screenshot screen capture print region window scroll smart full screen");

    let g = page.group("Where it goes");
    groups.push(g.wrapper.clone().upcast());
    let copy = gtk::CheckButton::new();
    let save = gtk::CheckButton::new();
    save.set_group(Some(&copy));
    for (radio, title, value) in [(&copy, "Copy to the clipboard", "Paste it anywhere"), (&save, "Save to a file", "In Pictures")] {
        radio.set_valign(gtk::Align::Center);
        let r = widgets::row(title, "", Some(widgets::label(value, "row-value").upcast_ref()));
        r.prepend(radio);
        let click = gtk::GestureClick::new();
        let rb = radio.clone();
        click.connect_released(move |_, _, _, _| rb.set_active(true));
        r.add_controller(click);
        g.add(&r);
    }
    widgets::keywords("clipboard save file pictures folder");

    // The picture: the mode in action, then the bar Settings will show.
    let shot_key = bind_for("Screenshot");
    let head = widgets::hbox(10);
    head.add_css_class("preview-head");
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    let title = widgets::label("Smart screenshot", "preview-title");
    title.set_xalign(0.0);
    let line = widgets::label(MODES[0].3, "cp-line");
    line.set_wrap(true);
    line.set_xalign(0.0);
    text.append(&title);
    text.append(&line);
    head.append(&text);
    if let Some(k) = &shot_key {
        let or = widgets::hbox(6);
        or.append(&widgets::label("or press", "cp-or"));
        or.append(&caps(k));
        or.set_valign(gtk::Align::Center);
        head.append(&fit(&or));
    }
    tame_head(&head, 40);
    pv_box.append(&head);

    let mode = Rc::new(RefCell::new("smart".to_string()));
    let area = {
        let mode = mode.clone();
        widgets::drawing(300, move |cr, p, sec, w, h| draw_shot(cr, p, sec, w, h, &mode.borrow()))
    };
    area.add_css_class("cp-screen");
    area.set_vexpand(false);
    pv_box.append(&area);

    let hud = widgets::hbox(4);
    hud.add_css_class("cp-hud");
    hud.set_halign(gtk::Align::Center);
    let mut hud_modes = Vec::new();
    for (m, name, icon, _) in MODES {
        let b = gtk::ToggleButton::new();
        b.set_icon_name(icon);
        b.set_tooltip_text(Some(name));
        b.add_css_class("flat");
        b.add_css_class("cp-hud-mode");
        if let Some((_, f)) = hud_modes.first() {
            b.set_group(Some(f));
        }
        hud.append(&b);
        hud_modes.push((m.to_string(), b));
    }
    let sep = gtk::Separator::new(gtk::Orientation::Vertical);
    sep.add_css_class("cp-hud-sep");
    hud.append(&sep);
    let target = widgets::dropdown(&widgets::opts(&[("copy", "Copy"), ("save", "Save")]), "copy");
    target.set_valign(gtk::Align::Center);
    target.set_hexpand(false);
    target.add_css_class("cp-target");
    hud.append(&target);
    let go = gtk::Button::new();
    let gc = widgets::hbox(6);
    gc.append(&gtk::Image::from_icon_name("camera-photo-symbolic"));
    gc.append(&gtk::Label::new(Some("Capture")));
    go.set_child(Some(&gc));
    go.add_css_class("suggested-action");
    hud.append(&go);
    pv_box.append(&fit(&hud));

    let st = Rc::new(Shot {
        mode,
        save: Cell::new(false),
        title,
        line,
        area,
        tiles,
        hud: hud_modes,
        target: target.clone(),
        radios: (copy.clone(), save.clone()),
        syncing: Cell::new(false),
    });
    for (m, b) in st.tiles.iter().chain(st.hud.iter()) {
        let (st2, m) = (st.clone(), m.clone());
        b.connect_toggled(move |b| {
            if b.is_active() {
                set_mode(&st2, &m);
            }
        });
    }
    for (radio, is_save) in [(&copy, false), (&save, true)] {
        let st2 = st.clone();
        radio.connect_toggled(move |r| {
            if r.is_active() {
                set_save(&st2, is_save);
            }
        });
    }
    {
        let st2 = st.clone();
        target.connect_selected_notify(move |d| set_save(&st2, d.selected() == 1));
    }
    {
        let st2 = st.clone();
        go.connect_clicked(move |_| {
            let args = vec![
                "omarchy-capture-screenshot".to_string(),
                st2.mode.borrow().clone(),
                if st2.save.get() { "save".into() } else { "copy".into() },
            ];
            hide_and_run(args);
        });
    }
    set_mode(&st, "smart");
    set_save(&st, false);
    widgets::keywords("take a screenshot capture button");

    // The small screen tools.
    let tools = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    tools.set_homogeneous(true);
    tools.add_css_class("cp-tools");
    if let Some(args) = capture_tool("--text", "omarchy-capture-text") {
        tools.append(&tool_card("insert-text-symbolic", "Copy text", "Drag over words in a picture or video.", args));
    }
    if let Some(args) = capture_tool("--qr", "omarchy-capture-qr") {
        tools.append(&tool_card("view-grid-symbolic", "Read a QR code", "Drag over the code.", args));
    }
    let colour = if cmd::present("nyx-capture") {
        Some(vec!["nyx-capture".to_string(), "--colour".into()])
    } else if cmd::present("hyprpicker") {
        Some(vec!["hyprpicker".to_string(), "-a".into()])
    } else {
        None
    };
    if let Some(args) = colour {
        tools.append(&tool_card("color-select-symbolic", "Pick a colour", "Click any pixel to copy its colour.", args));
    }
    if tools.first_child().is_some() {
        pv_box.append(&tools);
    }
    // Search finds the tools by these words (they're cards, not rows).
    widgets::keywords("ocr text recognition extract qr code scan colour color picker eyedropper hex");
    groups
}

// ----- Recording -----

#[derive(Default)]
struct Rec {
    desktop: bool,
    mic: bool,
    webcam: bool,
    full: bool,
}

/// The recorder's arguments for the chosen options.
fn recording_args(r: &Rec) -> Vec<String> {
    let mut a = vec!["omarchy-capture-screenrecording".to_string()];
    if r.full {
        a.push("--fullscreen".into());
    }
    if r.desktop {
        a.push("--with-desktop-audio".into());
    }
    if r.mic {
        a.push("--with-microphone-audio".into());
    }
    if r.webcam {
        a.push("--with-webcam".into());
    }
    a
}

/// What a recording with these options takes in, as a sentence.
fn recording_line(r: &Rec) -> String {
    let what = if r.full { "The whole screen" } else { "A region or a window, chosen when you start" };
    let mut with = Vec::new();
    if r.desktop {
        with.push("sound");
    }
    if r.mic {
        with.push("your voice");
    }
    if r.webcam {
        with.push("the webcam");
    }
    match with.as_slice() {
        [] => format!("{what}, without sound."),
        [one] => format!("{what}, with {one}."),
        [rest @ .., last] => format!("{what}, with {} and {last}.", rest.join(", ")),
    }
}

fn draw_rec(cr: &Context, p: &Pal, _sec: Rgb, w: f64, h: f64, r: &Rec, live: bool) {
    let red = (0.92, 0.33, 0.30);
    let s = R::new(1.0, 1.0, w - 2.0, h - 2.0);
    let area = sketch::desktop(cr, p, s);
    let (left, _, _) = desktop_windows(s, area);
    if r.full {
        sketch::set(cr, red, 1.0);
        cr.set_line_width(3.0);
        sketch::rounded(cr, s.x + 1.5, s.y + 1.5, s.w - 3.0, s.h - 3.0, 8.0);
        let _ = cr.stroke();
    } else {
        selection(cr, s, left.inset(-4.0), red, false);
    }
    if r.webcam {
        let rad = (s.h * 0.13).clamp(18.0, 40.0);
        let (cx, cy) = (s.x + s.w - rad - 14.0, s.y + s.h - rad - 14.0);
        cr.arc(cx, cy, rad, 0.0, std::f64::consts::TAU);
        sketch::set(cr, sketch::mix(p.surface, p.text, 0.2), 1.0);
        let _ = cr.fill_preserve();
        sketch::set(cr, (1.0, 1.0, 1.0), 0.9);
        cr.set_line_width(2.5);
        let _ = cr.stroke();
        // A head and shoulders.
        sketch::set(cr, p.dim, 0.9);
        cr.arc(cx, cy - rad * 0.2, rad * 0.3, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        cr.save().ok();
        cr.arc(cx, cy, rad - 1.5, 0.0, std::f64::consts::TAU);
        cr.clip();
        cr.arc(cx, cy + rad * 0.75, rad * 0.55, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        cr.restore().ok();
    }
    let label = if live { "Recording" } else { "Starts when you press Start" };
    pill(cr, label, s.cx(), s.y + s.h * 0.08 + 18.0, Some(red));
}

/// The recording half: returns its groups (left), and fills its picture (right).
fn recording(page: &Page, pv_box: &gtk::Box) -> Vec<gtk::Widget> {
    let mut groups = Vec::new();
    let rec = Rc::new(RefCell::new(Rec::default()));
    let live = Rc::new(Cell::new(false));

    let head = widgets::hbox(10);
    head.add_css_class("preview-head");
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    let title = widgets::label("Recording", "preview-title");
    title.set_xalign(0.0);
    let line = widgets::label(&recording_line(&rec.borrow()), "cp-line");
    line.set_wrap(true);
    line.set_xalign(0.0);
    text.append(&title);
    text.append(&line);
    head.append(&text);
    let start = gtk::Button::new();
    let sc = widgets::hbox(6);
    let si = gtk::Image::from_icon_name("media-record-symbolic");
    let sl = gtk::Label::new(Some("Start"));
    sc.append(&si);
    sc.append(&sl);
    start.set_child(Some(&sc));
    start.add_css_class("suggested-action");
    start.set_valign(gtk::Align::Center);
    head.append(&start);
    tame_head(&head, 40);
    pv_box.append(&head);
    let area = {
        let (rec, live) = (rec.clone(), live.clone());
        widgets::drawing(340, move |cr, p, sec, w, h| draw_rec(cr, p, sec, w, h, &rec.borrow(), live.get()))
    };
    area.add_css_class("cp-screen");
    area.set_vexpand(false);
    pv_box.append(&area);
    let show_live = {
        let (start, si, sl, live, area) = (start.clone(), si.clone(), sl.clone(), live.clone(), area.clone());
        move |on: bool| {
            live.set(on);
            sl.set_text(if on { "Stop" } else { "Start" });
            si.set_icon_name(Some(if on { "media-playback-stop-symbolic" } else { "media-record-symbolic" }));
            if on {
                start.remove_css_class("suggested-action");
                start.add_css_class("destructive-action");
            } else {
                start.remove_css_class("destructive-action");
                start.add_css_class("suggested-action");
            }
            area.queue_draw();
        }
    };
    {
        let show_live = show_live.clone();
        cmd::run_async(&["omarchy-capture-screenrecording", "--status"], move |r| show_live(r.is_ok()));
    }
    {
        let (rec, live) = (rec.clone(), live.clone());
        start.connect_clicked(move |_| {
            if live.get() {
                cmd::spawn(&["omarchy-capture-screenrecording", "--stop-recording"]);
                show_live(false);
            } else {
                hide_and_run(recording_args(&rec.borrow()));
            }
        });
    }
    let refresh = {
        let (rec, line, area) = (rec.clone(), line.clone(), area.clone());
        move || {
            line.set_text(&recording_line(&rec.borrow()));
            area.queue_draw();
        }
    };

    let g = page.group("What to record");
    groups.push(g.wrapper.clone().upcast());
    let part = gtk::CheckButton::new();
    let whole = gtk::CheckButton::new();
    whole.set_group(Some(&part));
    part.set_active(true);
    for (radio, title, desc, full) in [
        (&part, "A region or a window", "You choose when you press Start.", false),
        (&whole, "The whole screen", "", true),
    ] {
        radio.set_valign(gtk::Align::Center);
        let r = widgets::row(title, desc, None);
        r.prepend(radio);
        let click = gtk::GestureClick::new();
        let rb = radio.clone();
        click.connect_released(move |_, _, _, _| rb.set_active(true));
        r.add_controller(click);
        let (rec, refresh) = (rec.clone(), refresh.clone());
        radio.connect_toggled(move |b| {
            if b.is_active() {
                rec.borrow_mut().full = full;
                refresh();
            }
        });
        g.add(&r);
    }
    widgets::keywords("record video screencast whole screen region window");

    let g = page.group("Include");
    groups.push(g.wrapper.clone().upcast());
    for (title, desc, icon, set) in [
        ("Sound from the computer", "Music, calls and videos.", "audio-speakers-symbolic", (|r: &mut Rec, v| r.desktop = v) as fn(&mut Rec, bool)),
        ("Your microphone", "Your voice.", "audio-input-microphone-symbolic", |r, v| r.mic = v),
        ("The webcam", "A small picture of you in a corner.", "camera-web-symbolic", |r, v| r.webcam = v),
    ] {
        let (rec, refresh) = (rec.clone(), refresh.clone());
        let (r, _) = widgets::switch_row(title, desc, false, move |on| {
            set(&mut rec.borrow_mut(), on);
            refresh();
        });
        g.add(&tiled(r, icon));
        widgets::keywords("record video screencast sound audio microphone webcam camera");
    }

    let g = page.group("");
    g.wrapper.add_css_class("cp-gap");
    groups.push(g.wrapper.clone().upcast());
    if let Some(k) = bind_for("Screenrecording") {
        g.add(&widgets::icon_row("input-keyboard-symbolic", "Start or stop", "From anywhere", Some(caps(&k).upcast_ref())));
    }
    let folder = gtk::glib::user_special_dir(gtk::glib::UserDirectory::Videos)
        .unwrap_or_else(|| std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Videos"));
    let folder = std::env::var_os("OMARCHY_SCREENRECORD_DIR").map(std::path::PathBuf::from).unwrap_or(folder);
    let name = folder.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Videos".into());
    let open = gtk::Button::new();
    let oc = widgets::hbox(6);
    oc.append(&gtk::Image::from_icon_name("folder-symbolic"));
    oc.append(&gtk::Label::new(Some("Open folder")));
    open.set_child(Some(&oc));
    {
        let folder = folder.clone();
        open.connect_clicked(move |_| {
            let uri = gtk::gio::File::for_path(&folder).uri();
            let _ = gtk::gio::AppInfo::launch_default_for_uri(&uri, None::<&gtk::gio::AppLaunchContext>);
        });
    }
    g.add(&widgets::icon_row("folder-videos-symbolic", &format!("Recordings go to {name}"), "", Some(open.upcast_ref())));
    widgets::keywords("record video screencast start stop folder videos saved");
    groups
}

// ----- Reminders -----

/// `minutes` and an optional message for `omarchy-reminder`.
fn reminder_args(minutes: &str, message: &str) -> Option<Vec<String>> {
    let m: u32 = minutes.trim().parse().ok().filter(|m| (1..=10080).contains(m))?;
    let mut a = vec!["omarchy-reminder".to_string(), m.to_string()];
    // A leading - would be read as an option.
    let msg = message.trim().trim_start_matches('-').trim_start();
    if !msg.is_empty() {
        a.push(msg.to_string());
    }
    Some(a)
}

/// "Check the oven in 12m (14:05)" for each waiting reminder in `omarchy-reminder show --json`.
fn reminder_labels(json: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    v.get("reminders")
        .and_then(|r| r.as_array())
        .map(|list| {
            list.iter()
                .map(|r| {
                    let text = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                    format!("{} in {} ({})", text("label"), text("remaining"), text("atTime"))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Reminders a level down; returns how many are waiting.
fn reminders(content: &gtk::Box) -> usize {
    let card = more_card(content, "");
    let minutes = gtk::Entry::new();
    minutes.set_placeholder_text(Some("Minutes"));
    minutes.set_width_chars(8);
    let message = gtk::Entry::new();
    message.set_placeholder_text(Some("Message (optional)"));
    message.set_hexpand(true);
    let set = gtk::Button::with_label("Remind me");
    set.add_css_class("suggested-action");
    let form = widgets::hbox(8);
    form.append(&minutes);
    form.append(&message);
    form.append(&set);
    set.connect_clicked(move |_| match reminder_args(&minutes.text(), &message.text()) {
        Some(args) => {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            cmd::run_async(&refs, |r| {
                match r {
                    Ok(_) => window::toast("Reminder set"),
                    Err(e) => window::toast(&format!("{e}")),
                }
                window::rebuild("capture");
            });
        }
        None => window::toast("Enter the minutes from now, 1 to 10080"),
    });
    card.append(&tiled(widgets::stacked_row("Remind me in", "A notification after that many minutes.", form.upcast_ref()), "alarm-symbolic"));
    widgets::keywords("reminder timer alarm notify later");
    // `show` pops a notification; the JSON form only prints.
    let shown = reminder_labels(&cmd::output(&["omarchy-reminder", "show", "--json"]).unwrap_or_default());
    if !shown.is_empty() {
        let card = more_card(content, "Waiting");
        for label in &shown {
            card.append(&tiled(widgets::row(label, "", None), "alarm-symbolic"));
        }
        let (r, _) = widgets::button_row("Cancel reminders", "", "Clear all", |_| {
            cmd::run_async(&["omarchy-reminder", "clear"], |_| window::rebuild("capture"));
        });
        card.append(&r);
    }
    shown.len()
}

// ----- Weather -----

fn weather(content: &gtk::Box) {
    let card = more_card(content, "");
    let (r, _) = widgets::entry_row(
        "Weather location",
        "A town or city name, used by the weather panel and bar. Empty finds it automatically. Press Enter.",
        "",
        "e.g. Austin",
        |place| {
            let place = place.trim().to_string();
            if place.starts_with('-') {
                return;
            }
            let args: Vec<&str> = if place.is_empty() {
                vec!["omarchy-weather-location", "--clear"]
            } else {
                vec!["omarchy-weather-location", "--set", &place]
            };
            cmd::run_async(&args, |r| match r {
                Ok(_) => window::toast("Weather location saved"),
                Err(e) => window::toast(&format!("{e}")),
            });
        },
    );
    widgets::keywords("weather location city forecast");
    card.append(&tiled(r, "weather-few-clouds-symbolic"));
    let controls = widgets::hbox(8);
    for (script, label) in [
        ("omarchy-notification-time", "Time"),
        ("omarchy-notification-battery", "Battery"),
        ("omarchy-notification-weather", "Weather"),
    ] {
        if cmd::present(script) {
            let b = gtk::Button::with_label(label);
            b.connect_clicked(move |_| cmd::spawn(&[script]));
            controls.append(&b);
        }
    }
    card.append(&tiled(
        widgets::row("Show now", "Pop up the time, battery or weather panel.", Some(controls.upcast_ref())),
        "dialog-information-symbolic",
    ));
    widgets::keywords("notification panel time battery weather popup");
}

// ----- Dictation -----

/// Dictation a level down; returns what to show beside its row ("On", "Not installed").
fn dictation(content: &gtk::Box) -> Option<String> {
    if crate::platform::PLATFORM == crate::platform::Platform::NyxOS {
        return voice_dictation(content);
    }
    // Omarchy's dictation (VoxType); the install script's name changed in Omarchy 4.5.
    let install = ["omarchy-install-dictation-voxtype", "omarchy-voxtype-install"].into_iter().find(|s| cmd::present(s))?;
    let card = more_card(content, "");
    let on = cmd::present("voxtype");
    let controls = widgets::hbox(8);
    if on {
        for (script, label) in [("omarchy-voxtype-config", "Settings"), ("omarchy-voxtype-model", "Voice model")] {
            if cmd::present(script) {
                let b = gtk::Button::with_label(label);
                b.connect_clicked(move |_| cmd::spawn(&[script]));
                controls.append(&b);
            }
        }
        if cmd::present("omarchy-voxtype-remove") {
            let b = gtk::Button::with_label("Remove…");
            b.connect_clicked(|_| terminal("omarchy-voxtype-remove"));
            controls.append(&b);
        }
    } else {
        let b = gtk::Button::with_label("Install…");
        b.connect_clicked(move |_| terminal(install));
        controls.append(&b);
    }
    card.append(&tiled(
        widgets::row(
            "Speech to text",
            if on { "Installed. Type by talking, in any app." } else { "Type by talking, in any app. Runs on this computer." },
            Some(controls.upcast_ref()),
        ),
        "audio-input-microphone-symbolic",
    ));
    widgets::keywords("voxtype dictation voice speech whisper microphone typing");
    Some(if on { "On".into() } else { "Not installed".into() })
}

/// `~/.config/nyx-voice/config.toml`, Nyx Voice's settings.
fn voice_config_path() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"))
        .join("nyx-voice/config.toml")
}

fn voice_dictation_value(key: &str) -> Option<toml::Value> {
    let text = std::fs::read_to_string(voice_config_path()).ok()?;
    let t: toml::Table = toml::from_str(&text).ok()?;
    t.get("dictation")?.get(key).cloned()
}

/// Change one `[dictation]` key, keeping the rest of the file. Nyx Voice reads it when it
/// next starts (it starts on the next dictation once idle).
fn set_voice_dictation(key: &str, value: toml::Value) {
    let path = voice_config_path();
    let mut t: toml::Table = std::fs::read_to_string(&path).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default();
    let section = t.entry("dictation").or_insert_with(|| toml::Value::Table(Default::default()));
    if let Some(table) = section.as_table_mut() {
        table.insert(key.into(), value);
    }
    let saved = toml::to_string(&t).map_err(|e| e.to_string()).and_then(|text| {
        std::fs::create_dir_all(path.parent().unwrap_or(&path)).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())
    });
    match saved {
        Ok(()) => {
            let _ = Command::new("systemctl").args(["--user", "try-restart", "nyx-voiced.service"]).status();
        }
        Err(e) => window::toast(&format!("Couldn't save dictation settings: {e}")),
    }
}

/// NyxOS: dictation is Nyx Voice's.
fn voice_dictation(content: &gtk::Box) -> Option<String> {
    if !cmd::present("nyx-voice") {
        return None;
    }
    let card = more_card(content, "");
    card.append(&widgets::info_row("Talk to type", "Hold F9 or Right Alt and speak, or press Super+Ctrl+X to start and stop. Runs on this computer.").0);
    let output = voice_dictation_value("output").and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "type".into());
    let (r, _) = widgets::choice_row(
        "Put the words",
        "Typed where you're writing, or on the clipboard to paste.",
        widgets::opts(&[("type", "Typed into the window"), ("clipboard", "On the clipboard")]),
        &output,
        |v| set_voice_dictation("output", toml::Value::String(v)),
    );
    card.append(&r);
    let pause = voice_dictation_value("pause_media").and_then(|v| v.as_bool()).unwrap_or(true);
    let (r, _) = widgets::switch_row("Pause music and videos while you talk", "They carry on when you stop.", pause, |on| {
        set_voice_dictation("pause_media", toml::Value::Boolean(on))
    });
    card.append(&r);
    let max = voice_dictation_value("max_seconds").and_then(|v| v.as_integer()).unwrap_or(60).to_string();
    let (r, _) = widgets::choice_row(
        "Longest dictation",
        "It stops by itself after this long.",
        widgets::opts(&[("30", "30 seconds"), ("60", "1 minute"), ("120", "2 minutes"), ("300", "5 minutes")]),
        &max,
        |v| set_voice_dictation("max_seconds", toml::Value::Integer(v.parse().unwrap_or(60))),
    );
    card.append(&r);
    widgets::keywords("dictation voice speech microphone typing nyx voice f9");
    Some(if output == "clipboard" { "On the clipboard".into() } else { "Typed".into() })
}

// ----- Tools -----

/// Coding agents Omarchy knows, as (id, name).
const AGENTS: &[(&str, &str)] = &[
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("opencode", "OpenCode"),
    ("pi", "Pi"),
    ("omp", "Oh My Pi"),
    ("ori", "Ori"),
    ("grok", "Grok"),
    ("agy", "Antigravity"),
    ("copilot", "GitHub Copilot"),
    ("crush", "Crush"),
    ("cursor-agent", "Cursor CLI"),
    ("muse", "Muse Code"),
    ("hermes", "Hermes"),
    ("openclaw", "OpenClaw"),
];

/// Helpers and fixes a level down; returns whether there was anything to show.
fn tools(content: &gtk::Box) -> bool {
    let card = more_card(content, "");
    if cmd::present("omarchy-default-agent") {
        // The choice is a one-line file; asking the script to read it would be the same.
        let current = std::fs::read_to_string(crate::paths::config_home().join("omarchy/defaults/agent"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let mut options = widgets::opts(AGENTS);
        if current.is_empty() {
            options.insert(0, (String::new(), "None chosen".into()));
        } else if !AGENTS.iter().any(|(id, _)| *id == current) {
            options.push((current.clone(), current.clone()));
        }
        let (r, _) = widgets::choice_row(
            "Default coding agent",
            "The assistant the agent shortcut opens. Choosing one installs it if needed, in a terminal, and opens it.",
            options,
            &current,
            {
                let current = current.clone();
                move |id| {
                    if !id.is_empty() && id != current && AGENTS.iter().any(|(a, _)| *a == id) {
                        cmd::spawn(&["omarchy-default-agent", &id]);
                    }
                }
            },
        );
        widgets::keywords("agent ai assistant claude codex opencode cursor copilot coding llm");
        card.append(&tiled(r, "utilities-terminal-symbolic"));
    }
    if cmd::present("omarchy-toggle-crash-capture") {
        let off = cmd::run(&["omarchy-toggle-enabled", "crash-capture-off"]).is_ok();
        let (r, _) =
            widgets::switch_row("Tell me when a program crashes", "A notification with a way to look into it.", !off, |_| {
                cmd::run_async(&["omarchy-toggle-crash-capture"], |r| {
                    if let Err(e) = r {
                        window::toast(&format!("{e}"));
                    }
                });
            });
        widgets::keywords("crash coredump notification report debug");
        card.append(&tiled(r, "dialog-warning-symbolic"));
    }
    if cmd::present("omarchy-disk-speedtest") {
        let (r, _) = widgets::button_row("Disk speed test", "Measures how fast the drive reads and writes.", "Run…", |_| {
            terminal("omarchy-disk-speedtest")
        });
        widgets::keywords("ssd nvme benchmark storage");
        card.append(&tiled(r, "drive-harddisk-symbolic"));
    }
    let controls = widgets::hbox(8);
    for (script, label) in [("omarchy-restart-audio", "Sound"), ("omarchy-restart-shell", "Bar and menus")] {
        if cmd::present(script) {
            let b = gtk::Button::with_label(label);
            b.connect_clicked(move |b| {
                b.set_sensitive(false);
                let b = b.clone();
                cmd::run_async(&[script], move |r| {
                    b.set_sensitive(true);
                    match r {
                        Ok(_) => window::toast("Restarted"),
                        Err(e) => window::toast(&format!("{e}")),
                    }
                });
            });
            controls.append(&b);
        }
    }
    if controls.first_child().is_some() {
        card.append(&tiled(
            widgets::row("Restart", "Try this when sound stops or the bar misbehaves.", Some(controls.upcast_ref())),
            "view-refresh-symbolic",
        ));
        widgets::keywords("fix reset pipewire audio shell bar");
    }
    card.first_child().is_some()
}

fn more_tools(page: &Page) {
    let g = page.group("More tools");
    let mut any = false;
    if cmd::present("omarchy-reminder") {
        let (content, open) = more_page("Reminders", 520);
        let waiting = reminders(&content);
        let value = match waiting {
            0 => String::new(),
            n => format!("{n} waiting"),
        };
        let (r, _) = widgets::nav_row(Some("alarm-symbolic"), "Reminders", "A notification after a while", &value, move || open());
        g.add(&r);
        any = true;
    }
    let (content, open) = more_page("Dictation", 520);
    if let Some(value) = dictation(&content) {
        let (r, _) = widgets::nav_row(Some("audio-input-microphone-symbolic"), "Dictation", "Type by talking, in any app", &value, move || open());
        g.add(&r);
        any = true;
    }
    if cmd::present("omarchy-weather-location") {
        let (content, open) = more_page("Weather and status", 520);
        weather(&content);
        let (r, _) = widgets::nav_row(
            Some("weather-few-clouds-symbolic"),
            "Weather and status",
            "Location, and time or battery pop-ups",
            "",
            move || open(),
        );
        g.add(&r);
        any = true;
    }
    let (content, open) = more_page("Helpers and fixes", 560);
    if tools(&content) {
        let (r, _) = widgets::nav_row(
            Some("applications-engineering-symbolic"),
            "Helpers and fixes",
            "Coding agent, crash alerts, restarts",
            "",
            move || open(),
        );
        g.add(&r);
        any = true;
    }
    g.wrapper.set_visible(any);
}

pub fn build(page: &Page) {
    let shots = cmd::present("omarchy-capture-screenshot");
    let recs = cmd::present("omarchy-capture-screenrecording");
    if !shots && !recs {
        page.banner("Screenshots and screen recording use Omarchy's capture tools, which aren't installed.", true);
        more_tools(page);
        return;
    }
    let tab = TAB.with(|t| t.borrow().clone());
    let tab = if (tab == "rec" && recs) || !shots { "rec" } else { "shot" };

    let pv = page.split();
    let shot_box = widgets::vbox(14);
    let rec_box = widgets::vbox(14);
    pv.add(&shot_box);
    pv.add(&rec_box);
    let foot = pv.foot("dialog-information-symbolic", "");

    let tabs_group = page.plain_group("");
    let shot_groups = if shots { screenshots(page, &shot_box) } else { Vec::new() };
    let rec_groups = if recs { recording(page, &rec_box) } else { Vec::new() };
    let shot_key = bind_for("Screenshot");
    let rec_key = bind_for("Screenrecording");
    let show = Rc::new(move |t: &str| {
        TAB.with(|x| *x.borrow_mut() = t.to_string());
        let shot = t == "shot";
        shot_box.set_visible(shot);
        rec_box.set_visible(!shot);
        for g in &shot_groups {
            g.set_visible(shot);
        }
        for g in &rec_groups {
            g.set_visible(!shot);
        }
        foot.set_text(&match (shot, &shot_key, &rec_key) {
            (true, Some(k), _) => format!("Press {k} for a screenshot from anywhere; Settings needn't be open."),
            (true, None, _) => "Settings steps aside while you capture, then comes back.".to_string(),
            (false, _, Some(k)) => format!("Stop from the red dot in the bar, or with {k}."),
            (false, _, None) => "Stop from the red dot in the bar.".to_string(),
        });
    });
    if shots && recs {
        let s2 = show.clone();
        let seg = widgets::segmented(&widgets::opts(&[("shot", "Screenshot"), ("rec", "Recording")]), tab, move |id| s2(&id));
        seg.add_css_class("cp-tabs");
        seg.set_hexpand(true);
        let mut c = seg.first_child();
        while let Some(b) = c {
            b.set_hexpand(true);
            c = b.next_sibling();
        }
        tabs_group.add(&seg);
    }
    show(tab);

    more_tools(page);
    hold_controls(page, 40);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_arguments() {
        assert_eq!(recording_args(&Rec::default()), ["omarchy-capture-screenrecording"]);
        let r = Rec { desktop: true, mic: true, webcam: false, full: true };
        assert_eq!(
            recording_args(&r),
            ["omarchy-capture-screenrecording", "--fullscreen", "--with-desktop-audio", "--with-microphone-audio"]
        );
    }

    #[test]
    fn says_what_it_records() {
        let r = Rec { desktop: true, mic: true, webcam: true, full: true };
        assert_eq!(recording_line(&r), "The whole screen, with sound, your voice and the webcam.");
        assert_eq!(recording_line(&Rec::default()), "A region or a window, chosen when you start, without sound.");
    }

    #[test]
    fn waiting_reminders() {
        assert!(reminder_labels(r#"{"count":0,"reminders":[]}"#).is_empty());
        assert!(reminder_labels("not json").is_empty());
        let j = r#"{"count":1,"reminders":[{"label":"Check the oven","remaining":"12m","atTime":"14:05"}]}"#;
        assert_eq!(reminder_labels(j), ["Check the oven in 12m (14:05)"]);
    }

    #[test]
    fn reminders() {
        assert_eq!(reminder_args("5", ""), Some(vec!["omarchy-reminder".into(), "5".into()]));
        assert_eq!(
            reminder_args(" 30 ", " Check the oven "),
            Some(vec!["omarchy-reminder".into(), "30".into(), "Check the oven".into()])
        );
        assert_eq!(reminder_args("5", "--help"), Some(vec!["omarchy-reminder".into(), "5".into(), "help".into()]));
        assert_eq!(reminder_args("0", ""), None);
        assert_eq!(reminder_args("abc", ""), None);
        assert_eq!(reminder_args("99999", ""), None);
    }
}
