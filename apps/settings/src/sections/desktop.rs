//! Desktop: Nyx Dock, Control Center and Notification Center. Each page changes its
//! program the way the program itself does (its config file, its own commands, its
//! service), and only shows when the program is installed.
//!
//! The drawings the Desktop pages share (a small desktop with its bar, a window, a
//! label on a pill) are here too; Switcher and Tiling & Workspaces use them.

use crate::backend::{notifyd, shell};
use crate::sketch::{Pal, R, Rgb, mix, rounded, set};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::cairo::Context;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::f64::consts::TAU;
use std::path::PathBuf;
use std::rc::Rc;

fn config(name: &str) -> PathBuf {
    glib::user_config_dir().join("nyx").join(name)
}

/// The shell components' settings are NyxOS configuration domains: NyxOS's defaults, an
/// administrator's, then the user's file with only what they changed. Each component
/// owns its domain's format and schema; Settings reads them merged and sets or resets
/// one key at a time in the user's file.
fn layers() -> nyx_config::Layers {
    nyx_config::Layers::standard()
}

fn merged(domain: &str) -> toml::Table {
    layers().merged(domain).unwrap_or_default()
}

fn set_key(domain: &str, key: &str, value: toml::Value) -> Result<(), String> {
    layers().set(nyx_config::Layer::User, domain, key, value).map_err(|e| format!("{e:#}"))
}

fn reset_key(domain: &str, key: &str) -> Result<(), String> {
    layers().reset(nyx_config::Layer::User, domain, key).map_err(|e| format!("{e:#}"))
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

/// A switch for one of our bar widgets, with an icon: on adds it to the right of the bar.
fn bar_switch(id: &'static str, icon: &str, title: &str, desc: &str, also: impl Fn(bool) + 'static) -> gtk::Box {
    let (r, _) = widgets::switch_row(title, desc, on_bar(id), move |on| {
        if on {
            run(&["omarchy-plugin-enable", id, "right"], || {});
        } else {
            run(&["omarchy-plugin-disable", id], || {});
        }
        also(on);
    });
    r.prepend(&widgets::tile(icon));
    r.add_css_class("with-tile");
    r
}

// ---------------------------------------------------------------- Shared parts

/// Call `f` with whether the page is narrow (its preview above the controls), now and
/// whenever that changes.
pub(super) fn on_narrow(page: &Page, f: impl Fn(bool) + 'static) {
    f(page.body.has_css_class("narrow"));
    page.body.connect_notify_local(Some("css-classes"), move |b, _| f(b.has_css_class("narrow")));
}

/// The narrowest a page's main picture gets beside the controls. Narrower than that and
/// the page puts the picture above the controls instead.
pub(super) const PICTURE_MIN: i32 = 460;

/// Give `w` the section's colour as its `color` (`.section-<key>`, from the page it's on),
/// for outlines drawn with `currentColor`.
pub(super) fn tint(w: &impl IsA<gtk::Widget>) {
    w.upcast_ref::<gtk::Widget>().connect_realize(|w| {
        let key = std::iter::successors(w.parent(), |w| w.parent())
            .find_map(|w| w.css_classes().iter().find_map(|c| c.strip_prefix("colour-").map(str::to_string)));
        if let Some(k) = key {
            w.add_css_class(&format!("section-{k}"));
        }
    });
}

/// Make `w` (a tile or a card that isn't a row) found by search, in `group`.
pub(super) fn searchable(w: &impl IsA<gtk::Widget>, text: &str, group: &widgets::Group) {
    widgets::SEARCH.with(|s| {
        let mut s = s.borrow_mut();
        let section = s.last().map(|i| i.section.clone()).unwrap_or_default();
        s.push(widgets::SearchItem {
            section,
            text: crate::search::normalise(text),
            row: w.clone().upcast(),
            group: Some(group.wrapper.clone().upcast()),
        });
    });
}

/// The description line of a row, to change it later.
pub(super) fn row_desc(row: &gtk::Box) -> Option<gtk::Label> {
    fn find(w: &gtk::Widget) -> Option<gtk::Label> {
        if let Some(l) = w.downcast_ref::<gtk::Label>()
            && l.has_css_class("settings-option-description")
        {
            return Some(l.clone());
        }
        let mut c = w.first_child();
        while let Some(ch) = c {
            if let Some(l) = find(&ch) {
                return Some(l);
            }
            c = ch.next_sibling();
        }
        None
    }
    find(row.upcast_ref())
}

/// A line of text; returns how far it advances.
pub(super) fn text(cr: &Context, s: &str, x: f64, y: f64, size: f64, bold: bool, c: Rgb, a: f64) -> f64 {
    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        if bold { gtk::cairo::FontWeight::Bold } else { gtk::cairo::FontWeight::Normal },
    );
    cr.set_font_size(size);
    set(cr, c, a);
    cr.move_to(x, y);
    let _ = cr.show_text(s);
    cr.text_extents(s).map(|e| e.x_advance()).unwrap_or(0.0)
}

pub(super) fn text_width(cr: &Context, s: &str, size: f64, bold: bool) -> f64 {
    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        if bold { gtk::cairo::FontWeight::Bold } else { gtk::cairo::FontWeight::Normal },
    );
    cr.set_font_size(size);
    cr.text_extents(s).map(|e| e.x_advance()).unwrap_or(0.0)
}

/// A label on a pill, its left edge at `x` and its middle at `y`. Returns its width.
pub(super) fn pill(cr: &Context, s: &str, x: f64, y: f64, size: f64, bg: Rgb, bg_alpha: f64, fg: Rgb) -> f64 {
    let w = text_width(cr, s, size, true) + size * 1.4;
    let h = size + 8.0;
    rounded(cr, x, y - h / 2.0, w, h, h / 2.0);
    set(cr, bg, bg_alpha);
    let _ = cr.fill();
    text(cr, s, x + size * 0.7, y + size * 0.36, size, true, fg, 1.0);
    w
}

/// A rounded bar, for lines of text in drawn windows.
pub(super) fn bar(cr: &Context, x: f64, y: f64, w: f64, h: f64, c: Rgb, a: f64) {
    rounded(cr, x, y, w.max(0.0), h, h / 2.0);
    set(cr, c, a);
    let _ = cr.fill();
}

/// The time as the bar shows it ("Fri 14:20").
pub(super) fn clock() -> String {
    glib::DateTime::now_local().ok().and_then(|d| d.format("%a %H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()
}

/// A wallpaper in the theme's and the section's colours.
pub(super) fn wallpaper(cr: &Context, p: &Pal, sec: Rgb, r: R) {
    let g = gtk::cairo::LinearGradient::new(r.x, r.y, r.x + r.w, r.y + r.h);
    let a = mix(p.bg, sec, 0.5);
    let m = mix(p.bg, sec, 0.28);
    let b = mix(mix(p.bg, sec, 0.3), (0.9, 0.5, 0.32), 0.38);
    g.add_color_stop_rgb(0.0, a.0, a.1, a.2);
    g.add_color_stop_rgb(0.55, m.0, m.1, m.2);
    g.add_color_stop_rgb(1.0, b.0, b.1, b.2);
    let _ = cr.set_source(&g);
    cr.rectangle(r.x, r.y, r.w, r.h);
    let _ = cr.fill();
}

/// A small desktop: the wallpaper and the bar (workspaces, the clock, the tray).
/// Returns the area under the bar.
pub(super) fn desk(cr: &Context, p: &Pal, sec: Rgb, r: R) -> R {
    let radius = 9.0;
    cr.save().ok();
    rounded(cr, r.x, r.y, r.w, r.h, radius);
    cr.clip();
    wallpaper(cr, p, sec, r);
    let bh = 17.0;
    set(cr, p.bg, 0.92);
    cr.rectangle(r.x, r.y, r.w, bh);
    let _ = cr.fill();
    let y = r.y + bh / 2.0;
    bar(cr, r.x + 9.0, y - 2.5, 13.0, 5.0, p.accent, 1.0);
    for i in 0..3 {
        cr.arc(r.x + 29.0 + i as f64 * 8.0, y, 2.4, 0.0, TAU);
        set(cr, p.dim, 0.6);
        let _ = cr.fill();
    }
    let c = clock();
    let tw = text_width(cr, &c, 9.5, true);
    text(cr, &c, r.cx() - tw / 2.0, y + 3.4, 9.5, true, p.text, 0.9);
    // The tray: the battery, sound and Wi-Fi.
    let x = r.x + r.w - 12.0;
    cr.set_line_width(1.0);
    set(cr, p.text, 0.7);
    rounded(cr, x - 9.0, y - 3.0, 9.0, 6.0, 1.5);
    let _ = cr.stroke();
    cr.rectangle(x, y - 1.2, 1.2, 2.4);
    let _ = cr.fill();
    cr.move_to(x - 23.0, y - 1.5);
    cr.line_to(x - 20.5, y - 1.5);
    cr.line_to(x - 18.0, y - 4.0);
    cr.line_to(x - 18.0, y + 4.0);
    cr.line_to(x - 20.5, y + 1.5);
    cr.line_to(x - 23.0, y + 1.5);
    cr.close_path();
    let _ = cr.fill();
    for (i, rad) in [1.5, 3.8, 6.0].iter().enumerate() {
        if i == 0 {
            cr.arc(x - 35.0, y + 3.0, *rad, 0.0, TAU);
            let _ = cr.fill();
        } else {
            cr.arc(x - 35.0, y + 3.0, *rad, -2.36, -0.78);
            let _ = cr.stroke();
        }
    }
    cr.restore().ok();
    rounded(cr, r.x, r.y, r.w, r.h, radius);
    set(cr, p.text, 0.12);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    R::new(r.x, r.y + bh, r.w, r.h - bh)
}

/// What a drawn window shows.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Files,
    Term,
    Doc,
    Web,
}

/// A window on a drawn desktop: its header with three dots and a title, and lines
/// that suggest what's in it. A focused one has the accent's outline.
pub(super) fn window(cr: &Context, p: &Pal, r: R, kind: Kind, focused: bool) {
    if r.w < 10.0 || r.h < 10.0 {
        return;
    }
    let rad = (r.w.min(r.h) * 0.05).clamp(3.0, 7.0);
    rounded(cr, r.x, r.y + 2.0, r.w, r.h, rad);
    set(cr, (0.0, 0.0, 0.0), 0.22);
    let _ = cr.fill();
    let face = mix(p.bg, p.text, 0.035);
    rounded(cr, r.x, r.y, r.w, r.h, rad);
    set(cr, face, 1.0);
    let _ = cr.fill();
    let hh = (r.h * 0.08).clamp(8.0, 13.0);
    cr.save().ok();
    rounded(cr, r.x, r.y, r.w, r.h, rad);
    cr.clip();
    set(cr, mix(p.bg, p.text, 0.075), 1.0);
    cr.rectangle(r.x, r.y, r.w, hh);
    let _ = cr.fill();
    cr.restore().ok();
    let dot = (hh * 0.16).max(1.2);
    for i in 0..3 {
        cr.arc(r.x + hh * 0.6 + i as f64 * dot * 3.2, r.y + hh / 2.0, dot, 0.0, TAU);
        set(cr, p.dim, 0.55);
        let _ = cr.fill();
    }
    bar(cr, r.x + hh * 0.6 + dot * 10.0 + 4.0, r.y + hh / 2.0 - 1.0, r.w * 0.16, 2.0, p.dim, 0.55);

    let b = R::new(r.x + 7.0, r.y + hh + 7.0, r.w - 14.0, r.h - hh - 13.0);
    if b.w > 4.0 && b.h > 4.0 {
        cr.save().ok();
        cr.rectangle(b.x, b.y, b.w, b.h);
        cr.clip();
        let lh = (b.h / 22.0).clamp(1.6, 3.2);
        let step = (lh * 3.4).max(6.0);
        let rows = ((b.h / step) as usize).max(1);
        match kind {
            Kind::Files => {
                let (side, main) = b.hsplit(0.27, 7.0);
                for i in 0..4 {
                    let first = i == 0;
                    bar(cr, side.x, side.y + i as f64 * step, side.w * if first { 0.9 } else { 0.72 }, lh, if first { p.accent } else { p.dim }, if first { 0.85 } else { 0.4 });
                }
                bar(cr, main.x, main.y, main.w * 0.5, lh, p.text, 0.45);
                let rh = (step * 1.15).max(5.0);
                for i in 0..5 {
                    let y = main.y + step * 1.4 + i as f64 * (rh + lh * 1.6);
                    rounded(cr, main.x, y, main.w, rh, 2.0);
                    set(cr, if i == 0 { mix(face, p.accent, 0.28) } else { mix(face, p.text, 0.05) }, 1.0);
                    let _ = cr.fill();
                }
            }
            Kind::Term => {
                const C: [Rgb; 3] = [(0.42, 0.82, 0.45), (0.66, 0.52, 0.96), (0.95, 0.72, 0.3)];
                const W: [f64; 4] = [0.45, 0.6, 0.3, 0.5];
                for i in 0..rows.min(9) {
                    let y = b.y + i as f64 * step;
                    bar(cr, b.x, y, b.w * 0.18, lh, C[i % 3], 0.9);
                    bar(cr, b.x + b.w * 0.22, y, b.w * W[i % 4], lh, p.dim, 0.5);
                }
            }
            Kind::Doc => {
                const W: [f64; 5] = [0.9, 0.8, 0.95, 0.6, 0.85];
                bar(cr, b.x, b.y, b.w * 0.45, lh * 1.2, p.text, 0.5);
                for i in 1..rows.min(10) {
                    bar(cr, b.x, b.y + i as f64 * step, b.w * W[i % 5], lh, p.dim, 0.4);
                }
            }
            Kind::Web => {
                let ph = b.h * 0.42;
                let g = gtk::cairo::LinearGradient::new(b.x, b.y, b.x + b.w, b.y + ph);
                let a = mix(p.bg, p.accent, 0.45);
                let c = mix(p.bg, (0.95, 0.6, 0.35), 0.45);
                g.add_color_stop_rgb(0.0, a.0, a.1, a.2);
                g.add_color_stop_rgb(1.0, c.0, c.1, c.2);
                rounded(cr, b.x, b.y, b.w, ph, 3.0);
                let _ = cr.set_source(&g);
                let _ = cr.fill();
                bar(cr, b.x, b.y + ph + step * 0.8, b.w * 0.6, lh * 1.2, p.text, 0.5);
                for i in 0..4 {
                    bar(cr, b.x, b.y + ph + step * (1.8 + i as f64), b.w * [0.9, 0.75, 0.85, 0.5][i], lh, p.dim, 0.4);
                }
            }
        }
        cr.restore().ok();
    }
    rounded(cr, r.x, r.y, r.w, r.h, rad);
    if focused {
        set(cr, p.accent, 1.0);
        cr.set_line_width(1.5);
    } else {
        set(cr, p.text, 0.12);
        cr.set_line_width(1.0);
    }
    let _ = cr.stroke();
}

/// Every installed app's icon by its desktop id (without `.desktop`).
pub(super) fn app_icons() -> std::collections::HashMap<String, gio::Icon> {
    gio::AppInfo::all()
        .into_iter()
        .filter_map(|a| Some((a.id()?.trim_end_matches(".desktop").to_string(), a.icon()?)))
        .collect()
}

/// An app's icon by its desktop id, from [`app_icons`] (or a generic one).
pub(super) fn app_icon(icons: &std::collections::HashMap<String, gio::Icon>, id: &str) -> gio::Icon {
    icons
        .get(id)
        .or_else(|| icons.iter().find(|(k, _)| k.eq_ignore_ascii_case(id)).map(|(_, v)| v))
        .cloned()
        .unwrap_or_else(|| gio::ThemedIcon::new("application-x-executable").upcast())
}

/// Pick a toggle button by an id, without its signal doing anything.
fn toggles_pick(buttons: &[(String, gtk::ToggleButton)], guard: &Cell<bool>, id: &str) {
    guard.set(true);
    for (b_id, b) in buttons {
        if b_id == id {
            b.set_active(true);
        }
    }
    guard.set(false);
}

// ---------------------------------------------------------------- Dock

pub fn dock_installed() -> bool {
    crate::capability::program("nyx-dock")
}

/// Change one of the dock's settings (it follows its layers). Not before the dock's first
/// start, which picks the first pins when it finds no file of the user's.
pub fn set_dock(key: &str, value: toml::Value) {
    if !config("dock.toml").exists() {
        return;
    }
    if let Err(e) = set_key("dock", key, value) {
        window::toast(&format!("Couldn't save: {e}"));
    }
}

/// The dock's look and behaviour, which "Back to the defaults" hands back (pins and
/// folders stay).
const DOCK_LOOK: &[&str] =
    &["position", "icon_size", "hide", "hide_delay_ms", "running", "previews", "preview_delay_ms", "trash", "folder_counts", "apps", "app_names", "apps_search_bar", "click_animation"];

const ANIMS: &[(&str, &str)] =
    &[("squish", "Squish"), ("bounce", "Bounce"), ("ripple", "Ripple"), ("pop", "Pop"), ("glow", "Glow"), ("depress", "Depress"), ("none", "None")];

/// What the dock picture shows: the settings, and whether a window is drawn over it.
struct DockLook {
    edge: String,
    size: i64,
    hide: String,
    apps: bool,
    trash: bool,
    pinned: Vec<gio::Icon>,
    folders: usize,
    covered: bool,
}

impl DockLook {
    fn shown(&self) -> bool {
        match self.hide.as_str() {
            "never" => true,
            "always" => false,
            _ => !self.covered,
        }
    }

    /// The icons in the picture, scaled from the real ones.
    fn icon_px(&self) -> i32 {
        ((self.size as f64) * 0.42).round().clamp(12.0, 34.0) as i32
    }

    fn hide_tag(&self) -> &'static str {
        match (self.hide.as_str(), self.covered) {
            ("never", false) => "Never: always on screen",
            ("never", true) => "Never: windows stop above it",
            ("always", _) => "Auto: hidden until the pointer reaches the edge",
            (_, false) => "Smart: shown until a window covers it",
            (_, true) => "Smart: hidden while a window covers it",
        }
    }
}

fn edge_name(edge: &str) -> &'static str {
    match edge {
        "left" => "Left",
        "right" => "Right",
        _ => "Bottom",
    }
}

fn hide_desc(hide: &str) -> &'static str {
    match hide {
        "always" => "Auto: hidden until the pointer reaches the edge.",
        "never" => "Never: always on screen.",
        _ => "Smart: only while a window covers it.",
    }
}

/// The desktop under the dock: three windows clear of its edge, or one over it.
fn draw_dock_desk(cr: &Context, p: &Pal, sec: Rgb, w: f64, h: f64, l: &DockLook) {
    let area = desk(cr, p, sec, R::new(0.5, 0.5, w - 1.0, h - 1.0));
    let thick = l.icon_px() as f64 + 30.0;
    let free = match l.edge.as_str() {
        "left" => R::new(area.x + thick, area.y, area.w - thick, area.h),
        "right" => R::new(area.x, area.y, area.w - thick, area.h),
        _ => R::new(area.x, area.y, area.w, area.h - thick),
    };
    if l.covered {
        let r = if l.hide == "never" { free.inset(8.0) } else { area.inset(8.0) };
        window(cr, p, r, Kind::Web, true);
    } else {
        let mut room = free.inset(10.0);
        if l.edge == "bottom" {
            room.h -= 22.0;
        } else {
            room.h -= 26.0;
        }
        let (left, right) = room.hsplit(0.56, 9.0);
        let (top, bottom) = right.vsplit(0.62, 9.0);
        window(cr, p, left, Kind::Files, false);
        window(cr, p, top, Kind::Term, true);
        window(cr, p, bottom, Kind::Doc, false);
    }
    if !l.shown() {
        // Where it waits, out of sight.
        let (x, y, bw, bh) = match l.edge.as_str() {
            "left" => (area.x + 3.0, area.cy() - 30.0, 3.0, 60.0),
            "right" => (area.x + area.w - 6.0, area.cy() - 30.0, 3.0, 60.0),
            _ => (area.cx() - 30.0, area.y + area.h - 6.0, 60.0, 3.0),
        };
        rounded(cr, x, y, bw, bh, 1.5);
        set(cr, p.text, 0.55);
        let _ = cr.fill();
    }
}

/// A picture of a screen with the dock on `edge`, for the edge picker.
fn edge_button(edge: &'static str) -> (gtk::ToggleButton, gtk::DrawingArea) {
    let b = gtk::ToggleButton::new();
    b.add_css_class("dk-edge");
    b.set_tooltip_text(Some(edge_name(edge)));
    let weak = b.downgrade();
    let area = widgets::drawing(24, move |cr, p, sec, w, h| {
        let on = weak.upgrade().is_some_and(|b| b.is_active());
        rounded(cr, 0.5, 0.5, w - 1.0, h - 1.0, 3.0);
        set(cr, p.text, 0.06);
        let _ = cr.fill();
        let (x, y, bw, bh) = match edge {
            "left" => (3.0, h * 0.25, 3.0, h * 0.5),
            "right" => (w - 6.0, h * 0.25, 3.0, h * 0.5),
            _ => (w * 0.3, h - 6.0, w * 0.4, 3.0),
        };
        rounded(cr, x, y, bw, bh, 1.5);
        set(cr, if on { sec } else { p.dim }, if on { 1.0 } else { 0.7 });
        let _ = cr.fill();
    });
    area.set_content_width(36);
    area.set_hexpand(false);
    area.remove_css_class("preview-drawing");
    b.set_child(Some(&area));
    (b, area)
}

/// A tile in "In the dock": an icon and a switch over a name and a line.
fn dock_tile(icon: &str, title: &str, desc: &str, on: bool, change: impl Fn(bool) + 'static) -> (gtk::Box, gtk::Switch) {
    let t = widgets::vbox(2);
    t.add_css_class("dk-tile");
    t.set_hexpand(true);
    let top = widgets::hbox(6);
    let i = gtk::Image::from_icon_name(icon);
    i.add_css_class("dk-tile-icon");
    i.set_hexpand(true);
    i.set_halign(gtk::Align::Start);
    top.append(&i);
    let sw = gtk::Switch::new();
    sw.set_active(on);
    sw.set_valign(gtk::Align::Center);
    sw.connect_active_notify(move |s| change(s.is_active()));
    top.append(&sw);
    t.append(&top);
    let name = widgets::label(title, "dk-tile-name");
    name.set_xalign(0.0);
    name.set_margin_top(4);
    t.append(&name);
    let d = widgets::label(desc, "dk-tile-desc");
    d.set_xalign(0.0);
    d.set_ellipsize(gtk::pango::EllipsizeMode::End);
    t.append(&d);
    (t, sw)
}

pub fn build_dock(page: &Page) {
    // The GTK dock or its Quickshell interface (`nyx-dock --ui qml`, run by its backend).
    let on = service_enabled("nyx-dock.service") || service_enabled("nyx-dock-backend.service");
    let has_file = config("dock.toml").exists();
    let pv = has_file.then(|| page.split());

    let g = page.group("");
    let sw = gtk::Switch::new();
    sw.set_active(on);
    sw.connect_active_notify(|s| {
        run(&["nyx-dock", if s.is_active() { "--enable" } else { "--disable" }], || {
            // The dock writes its settings on first start; show them once it has.
            glib::timeout_add_local_once(std::time::Duration::from_secs(2), || window::rebuild("dock"));
        });
    });
    let r = widgets::icon_row(
        "user-desktop-symbolic",
        "Use Nyx Dock",
        "Instead of Omarchy's, now and at sign-in.",
        Some(sw.upcast_ref()),
    );
    r.add_css_class("dk-lead");
    g.add(&r);
    widgets::keywords("dock launcher taskbar omarchy replace");

    let Some(pv) = pv else {
        let g = page.group("Look and behaviour");
        g.note("These appear once the Nyx Dock has started for the first time.");
        return;
    };

    let t = merged("dock");
    let s = |k: &str, d: &str| t.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string();
    let b = |k: &str, d: bool| t.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
    let opts = |o: &[(&str, &str)]| o.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
    let icons = app_icons();
    let pinned: Vec<gio::Icon> = t
        .get("pinned")
        .and_then(|v| v.as_array())
        .map(|l| l.iter().filter_map(|v| v.as_str()).map(|id| app_icon(&icons, id)).collect())
        .unwrap_or_default();
    let folders = t.get("folders").and_then(|v| v.as_array()).map(|l| l.len()).unwrap_or(0);
    let look = Rc::new(RefCell::new(DockLook {
        edge: s("position", "bottom"),
        size: t.get("icon_size").and_then(|v| v.as_integer()).unwrap_or(48),
        hide: s("hide", "smart"),
        apps: b("apps", true),
        trash: b("trash", true),
        pinned,
        folders,
        covered: false,
    }));

    // ---- The picture: the dock on a desktop, at its edge and size.
    let line = widgets::label("", "preview-line");
    line.set_wrap(true);
    line.set_xalign(0.0);
    let scene = widgets::segmented(&opts(&[("empty", "Empty desk"), ("covered", "Window over it")]), "empty", |_| {});
    let head = pv.head("On your screen", "", &[scene.upcast_ref()]);
    if let Some(text) = head.first_child().and_downcast::<gtk::Box>() {
        text.append(&line);
    }
    let stage = gtk::Overlay::new();
    stage.add_css_class("dk-stage");
    stage.set_size_request(PICTURE_MIN, -1);
    stage.set_overflow(gtk::Overflow::Hidden);
    let desk_area = {
        let look = look.clone();
        widgets::drawing(300, move |cr, p, sec, w, h| draw_dock_desk(cr, p, sec, w, h, &look.borrow()))
    };
    desk_area.set_vexpand(true);
    stage.set_child(Some(&desk_area));
    let dock = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    dock.add_css_class("dk-dock");
    tint(&dock);
    let measure = widgets::hbox(0);
    measure.add_css_class("dk-measure");
    tint(&measure);
    let measure_label = widgets::label("", "dk-measure-text");
    measure.append(&measure_label);
    // The pill beside the dock, balanced by an empty box on its other side so the dock stays centred.
    let holder = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let balance = widgets::hbox(0);
    holder.append(&balance);
    holder.append(&dock);
    let measure_slot = widgets::hbox(0);
    measure_slot.append(&measure);
    holder.append(&measure_slot);
    for b in [&balance, &measure_slot] {
        b.set_size_request(52, 22);
    }
    stage.add_overlay(&holder);
    let tag = widgets::hbox(6);
    tag.add_css_class("dk-tag");
    tag.append(&gtk::Image::from_icon_name("view-reveal-symbolic"));
    let tag_label = widgets::label("", "dk-tag-text");
    tag.append(&tag_label);
    stage.add_overlay(&tag);
    pv.add(&stage);

    let refresh: Rc<dyn Fn()> = {
        let (look, dock, holder, measure, measure_label, tag, tag_label, desk_area, line) =
            (look.clone(), dock.clone(), holder.clone(), measure.clone(), measure_label.clone(), tag.clone(), tag_label.clone(), desk_area.clone(), line.clone());
        let (balance, measure_slot) = (balance.clone(), measure_slot.clone());
        Rc::new(move || {
            let l = look.borrow();
            let px = l.icon_px();
            let vertical = l.edge != "bottom";
            while let Some(c) = dock.first_child() {
                dock.remove(&c);
            }
            dock.set_orientation(if vertical { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal });
            let sep_orient = if vertical { gtk::Orientation::Horizontal } else { gtk::Orientation::Vertical };
            let add = |icon: &gio::Icon| {
                let i = gtk::Image::from_gicon(icon);
                i.set_pixel_size(px);
                dock.append(&i);
            };
            if l.apps {
                add(&gio::ThemedIcon::new("view-app-grid-symbolic").upcast());
                dock.append(&gtk::Separator::new(sep_orient));
            }
            for icon in l.pinned.iter().take(10) {
                add(icon);
            }
            if l.folders > 0 || l.trash {
                dock.append(&gtk::Separator::new(sep_orient));
            }
            for _ in 0..l.folders.min(4) {
                add(&gio::ThemedIcon::new("folder").upcast());
            }
            if l.trash {
                add(&gio::ThemedIcon::new("user-trash").upcast());
            }
            measure_label.set_text(&format!("{} px", l.size));
            for b in [&balance, &measure_slot] {
                if vertical {
                    b.set_size_request(-1, 22);
                } else {
                    b.set_size_request(52, -1);
                }
            }
            holder.set_orientation(if vertical { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal });
            holder.set_visible(l.shown());
            measure.set_halign(if vertical { gtk::Align::Center } else { gtk::Align::Start });
            measure.set_valign(if vertical { gtk::Align::Start } else { gtk::Align::Center });
            let (ha, va) = match l.edge.as_str() {
                "left" => (gtk::Align::Start, gtk::Align::Center),
                "right" => (gtk::Align::End, gtk::Align::Center),
                _ => (gtk::Align::Center, gtk::Align::End),
            };
            holder.set_halign(ha);
            holder.set_valign(va);
            holder.set_margin_bottom(if vertical { 8 } else { 9 });
            holder.set_margin_top(if vertical { 26 } else { 0 });
            holder.set_margin_start(9);
            holder.set_margin_end(9);
            tag_label.set_text(l.hide_tag());
            tag.set_valign(gtk::Align::End);
            tag.set_halign(if l.edge == "left" { gtk::Align::End } else { gtk::Align::Start });
            tag.set_margin_start(12);
            tag.set_margin_end(12);
            tag.set_margin_bottom(if l.edge == "bottom" { px + 28 } else { 12 });
            line.set_text(&format!("{} px, on the {} edge.", l.size, edge_name(&l.edge).to_lowercase()));
            desk_area.queue_draw();
        })
    };
    refresh();
    {
        let (look, refresh) = (look.clone(), refresh.clone());
        let mut c = scene.first_child();
        while let Some(child) = c {
            if let Some(b) = child.downcast_ref::<gtk::ToggleButton>() {
                let covered = b.label().is_some_and(|l| l.starts_with("Window"));
                let (look, refresh) = (look.clone(), refresh.clone());
                b.connect_toggled(move |b| {
                    if b.is_active() {
                        look.borrow_mut().covered = covered;
                        refresh();
                    }
                });
            }
            c = child.next_sibling();
        }
    }

    // The click animations, as frames to pick from.
    let anim_now = s("click_animation", "squish");
    let anim_guard = Rc::new(Cell::new(false));
    let anim_buttons: Rc<RefCell<Vec<(String, gtk::ToggleButton)>>> = Rc::default();
    let anim_dd: Rc<RefCell<Option<gtk::DropDown>>> = Rc::default();
    let card = pv.card();
    card.add_css_class("dk-anims");
    let ah = widgets::hbox(8);
    ah.append(&widgets::label("Click animation", "dk-anims-title"));
    let sub = widgets::label("How an icon answers a click.", "dk-anims-sub");
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    ah.append(&sub);
    card.append(&ah);
    let frames = widgets::hbox(6);
    frames.set_homogeneous(true);
    frames.add_css_class("dk-frames");
    let sample = look.borrow().pinned.first().cloned().unwrap_or_else(|| gio::ThemedIcon::new("audio-x-generic").upcast());
    let mut first: Option<gtk::ToggleButton> = None;
    for (id, name) in ANIMS {
        let b = gtk::ToggleButton::new();
        b.add_css_class("dk-anim");
        tint(&b);
        let col = widgets::vbox(5);
        let fx = widgets::hbox(0);
        fx.add_css_class("dk-fx");
        fx.add_css_class(&format!("dk-fx-{id}"));
        fx.set_halign(gtk::Align::Center);
        let i = gtk::Image::from_gicon(&sample);
        i.set_pixel_size(24);
        i.set_halign(gtk::Align::Center);
        i.set_valign(gtk::Align::Center);
        i.set_hexpand(true);
        fx.append(&i);
        col.append(&fx);
        let l = widgets::label(name, "dk-anim-name");
        l.set_xalign(0.5);
        col.append(&l);
        b.set_child(Some(&col));
        match &first {
            Some(f) => b.set_group(Some(f)),
            None => first = Some(b.clone()),
        }
        b.set_active(*id == anim_now);
        let (guard, dd, id2) = (anim_guard.clone(), anim_dd.clone(), id.to_string());
        b.connect_toggled(move |b| {
            if !b.is_active() || guard.get() {
                return;
            }
            set_dock("click_animation", toml::Value::String(id2.clone()));
            if let Some(dd) = dd.borrow().as_ref() {
                guard.set(true);
                dd.set_selected(ANIMS.iter().position(|(a, _)| *a == id2).unwrap_or(0) as u32);
                guard.set(false);
            }
        });
        frames.append(&b);
        anim_buttons.borrow_mut().push((id.to_string(), b));
    }
    card.append(&frames);
    let foot = pv.foot(
        "dialog-information-symbolic",
        "Nothing moves when Reduce motion is on. Right-click the dock for these settings too, a folder for how it opens, and an app for its card.",
    );

    // ---- The controls.
    let g = page.group("Where it sits");
    let edge_row_holder = widgets::hbox(6);
    let edge_row = widgets::row("Edge", edge_name(&look.borrow().edge), Some(edge_row_holder.upcast_ref()));
    let edge_desc = row_desc(&edge_row);
    let mut first: Option<gtk::ToggleButton> = None;
    let mut areas = Vec::new();
    for edge in ["bottom", "left", "right"] {
        let (eb, area) = edge_button(edge);
        tint(&eb);
        match &first {
            Some(f) => eb.set_group(Some(f)),
            None => first = Some(eb.clone()),
        }
        eb.set_active(look.borrow().edge == edge);
        areas.push(area);
        edge_row_holder.append(&eb);
        let (look, refresh, desc) = (look.clone(), refresh.clone(), edge_desc.clone());
        eb.connect_toggled(move |b| {
            if !b.is_active() {
                return;
            }
            set_dock("position", toml::Value::String(edge.into()));
            look.borrow_mut().edge = edge.into();
            if let Some(d) = &desc {
                d.set_text(edge_name(edge));
            }
            refresh();
        });
    }
    for eb in std::iter::successors(edge_row_holder.first_child(), |w| w.next_sibling()) {
        let areas = areas.clone();
        if let Some(t) = eb.downcast_ref::<gtk::ToggleButton>() {
            t.connect_toggled(move |_| areas.iter().for_each(|a| a.queue_draw()));
        }
    }
    g.add(&edge_row);
    widgets::keywords("position bottom left right edge side");
    let save_size = widgets::Debounce::default();
    let size = look.borrow().size as f64;
    let (r, slider) = {
        let (look, refresh) = (look.clone(), refresh.clone());
        widgets::slider_row("Icon size", "", (32.0, 80.0, 4.0), size, 0, " px", move |v| {
            let n = v.round() as i64;
            look.borrow_mut().size = n;
            refresh();
            save_size.call(300, move || set_dock("icon_size", toml::Value::Integer(n)));
        })
    };
    slider.scale.set_width_request(130);
    g.add(&r);
    let (r, _) = widgets::choice_row(
        "With two screens",
        "",
        opts(&[("all", "Apps on every screen"), ("monitor", "Apps on their own screen")]),
        &s("running", "all"),
        |v| set_dock("running", toml::Value::String(v)),
    );
    g.add(&r);
    widgets::keywords("running apps monitor screen every each");

    let g = page.group("Hiding");
    let hide_row_holder = widgets::hbox(0);
    let hide_row = widgets::row("Hide", hide_desc(&look.borrow().hide), Some(hide_row_holder.upcast_ref()));
    let hide_desc_label = row_desc(&hide_row);
    let hide_now = look.borrow().hide.clone();
    let seg = {
        let (look, refresh) = (look.clone(), refresh.clone());
        widgets::segmented(&opts(&[("smart", "Smart"), ("always", "Auto"), ("never", "Never")]), &hide_now, move |v| {
            set_dock("hide", toml::Value::String(v.clone()));
            if let Some(d) = &hide_desc_label {
                d.set_text(hide_desc(&v));
            }
            look.borrow_mut().hide = v;
            refresh();
        })
    };
    hide_row_holder.append(&seg);
    g.add(&hide_row);
    widgets::keywords("autohide hide smart automatically never");
    let (r, _) = widgets::switch_row(
        "Window previews",
        "Rest the pointer on an app to see its windows.",
        b("previews", true),
        |on| set_dock("previews", toml::Value::Boolean(on)),
    );
    g.add(&r);
    widgets::keywords("previews thumbnails hover");

    let g = page.plain_group("In the dock");
    let grid = gtk::Grid::new();
    grid.set_column_homogeneous(true);
    grid.set_column_spacing(8);
    grid.set_row_spacing(8);
    grid.add_css_class("dk-tiles");
    let search_switch: Rc<RefCell<Option<gtk::Switch>>> = Rc::default();
    let (apps_tile, _) = {
        let (look, refresh, ss) = (look.clone(), refresh.clone(), search_switch.clone());
        dock_tile("view-app-grid-symbolic", "Apps button", "First in line", b("apps", true), move |on| {
            set_dock("apps", toml::Value::Boolean(on));
            look.borrow_mut().apps = on;
            if let Some(s) = ss.borrow().as_ref()
                && let Some(t) = s.parent().and_then(|p| p.parent())
            {
                t.set_sensitive(on);
            }
            refresh();
        })
    };
    let (search_tile, ssw) = dock_tile("system-search-symbolic", "Search bar", "Always in Apps", b("apps_search_bar", false), |on| {
        set_dock("apps_search_bar", toml::Value::Boolean(on))
    });
    search_tile.set_sensitive(b("apps", true));
    *search_switch.borrow_mut() = Some(ssw);
    let (badge_tile, _) = dock_tile("folder-download-symbolic", "Badges", "Counts on folders", b("folder_counts", true), |on| {
        set_dock("folder_counts", toml::Value::Boolean(on))
    });
    let (trash_tile, _) = {
        let (look, refresh) = (look.clone(), refresh.clone());
        dock_tile("user-trash-symbolic", "Trash", "At the end", b("trash", true), move |on| {
            set_dock("trash", toml::Value::Boolean(on));
            look.borrow_mut().trash = on;
            refresh();
        })
    };
    let tiles = [
        (&apps_tile, "Apps button First in the dock: every app in a grid, with search. launcher"),
        (&search_tile, "Apps search bar Always show the search bar in the Apps panel. Off: it appears when you start typing."),
        (&badge_tile, "Folder item counts badges A badge with how many items each folder holds. Right-click a folder to set it for that folder."),
        (&trash_tile, "Trash Keep the Trash at the end of the dock."),
    ];
    for (tile, words) in tiles.iter() {
        searchable(*tile, words, &g);
    }
    let tiles: Vec<gtk::Box> = tiles.iter().map(|(t, _)| (*t).clone()).collect();
    g.add(&grid);

    // In a narrow window the animations leave the picture and come here.
    let g = page.group("");
    let (r, dd) = widgets::choice_row(
        "Click animation",
        "How an icon answers a click.",
        ANIMS.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        &anim_now,
        {
            let (guard, buttons) = (anim_guard.clone(), anim_buttons.clone());
            move |v| {
                if guard.get() {
                    return;
                }
                set_dock("click_animation", toml::Value::String(v.clone()));
                toggles_pick(&buttons.borrow(), &guard, &v);
            }
        },
    );
    *anim_dd.borrow_mut() = Some(dd);
    g.add(&r);
    widgets::forget_rows(&g.wrapper);
    let narrow_group = g.wrapper.clone();

    let g = page.group("");
    let reset = widgets::confirm_button("Reset", "Click again to reset", |_| {
        let failed: Vec<String> = DOCK_LOOK.iter().filter_map(|k| reset_key("dock", k).err()).collect();
        match failed.first() {
            None => window::toast("The dock looks as it does by default again"),
            Some(e) => window::toast(&format!("Couldn't reset: {e}")),
        }
        glib::timeout_add_local_once(std::time::Duration::from_millis(300), || window::rebuild("dock"));
    });
    g.add(&widgets::icon_row("view-refresh-symbolic", "Back to the defaults", "Pinned apps and folders stay.", Some(reset.upcast_ref())));
    widgets::keywords(
        "reset defaults position size hide autohide previews thumbnails trash apps launcher search running monitor screen folder count badge click animation bounce squish ripple pop glow depress",
    );

    let foot_box = foot.parent();
    on_narrow(page, move |narrow| {
        card.set_visible(!narrow);
        if let Some(f) = &foot_box {
            f.set_visible(!narrow);
        }
        narrow_group.set_visible(narrow);
        // Four across when the controls have the window's width, two by two beside the picture.
        let per_row = if narrow { 4 } else { 2 };
        for (i, t) in tiles.iter().enumerate() {
            if t.parent().is_some() {
                grid.remove(t);
            }
            grid.attach(t, (i % per_row) as i32, (i / per_row) as i32, 1, 1);
        }
    });
}

// ---------------------------------------------------------------- Control Center

/// The Control Center's tiles: (id, name, icon, the sizes it comes in), as the panel
/// names them. The panel draws a size a tile doesn't come in at its first.
const CC_TILES: &[(&str, &str, &str, &[(u32, u32)])] = &[
    ("backup", "Backup", "drive-harddisk-symbolic", &[(2, 3), (4, 3)]),
    ("wifi", "Wi-Fi", "network-wireless-symbolic", &[(2, 1), (1, 1)]),
    ("bluetooth", "Bluetooth", "bluetooth-symbolic", &[(2, 1), (1, 1)]),
    ("devices", "Devices", "phone-symbolic", &[(2, 1), (1, 1)]),
    ("tailscale", "Tailscale", "network-vpn-symbolic", &[(2, 1), (1, 1)]),
    ("vpn", "VPN", "security-high-symbolic", &[(2, 1), (1, 1)]),
    ("dnd", "Do not disturb", "notifications-disabled-symbolic", &[(1, 1)]),
    ("mode", "Workspace mode", "view-grid-symbolic", &[(1, 1)]),
    ("sound", "Sound & brightness", "audio-volume-high-symbolic", &[(4, 2), (2, 2)]),
    ("home", "Home", "go-home-symbolic", &[(2, 3), (4, 3)]),
    ("upcoming", "Up next", "x-office-calendar-symbolic", &[(4, 3), (2, 3)]),
    ("media", "Now playing", "audio-x-generic-symbolic", &[(4, 2)]),
    ("power", "Power mode", "power-profile-balanced-symbolic", &[(2, 1), (4, 1)]),
    ("battery", "Battery", "battery-good-symbolic", &[(1, 1), (2, 1)]),
    ("microphone", "Microphone", "audio-input-microphone-symbolic", &[(1, 1)]),
    ("night-light", "Night light", "night-light-symbolic", &[(1, 1)]),
    ("keep-awake", "Keep awake", "preferences-desktop-screensaver-symbolic", &[(1, 1)]),
    ("keyboard-light", "Keyboard light", "keyboard-brightness-symbolic", &[(1, 1)]),
    ("capture", "Screenshot", "camera-photo-symbolic", &[(1, 1)]),
];

/// The panel's own first-start layout, for when no layer sets one.
const CC_DEFAULT: &[&[(&str, (u32, u32))]] = &[
    &[
        ("wifi", (2, 1)),
        ("bluetooth", (2, 1)),
        ("vpn", (2, 1)),
        ("tailscale", (2, 1)),
        ("dnd", (1, 1)),
        ("mode", (1, 1)),
        ("night-light", (1, 1)),
        ("keep-awake", (1, 1)),
        ("sound", (4, 2)),
        ("media", (4, 2)),
        ("power", (2, 1)),
        ("keyboard-light", (1, 1)),
        ("capture", (1, 1)),
    ],
    &[("upcoming", (4, 3)), ("microphone", (1, 1)), ("battery", (1, 1)), ("home", (4, 3))],
];

#[derive(Clone)]
struct CcTile {
    widget: String,
    cols: u32,
    rows: u32,
}

fn cc_name_icon(id: &str) -> (String, &'static str) {
    if let Some(rest) = id.strip_prefix("plugin:") {
        let name = rest.split(':').next().unwrap_or(rest);
        return (name.to_string(), "application-x-addon-symbolic");
    }
    CC_TILES
        .iter()
        .find(|(i, ..)| *i == id)
        .map(|(_, n, ic, _)| (n.to_string(), *ic))
        .unwrap_or_else(|| (id.to_string(), "application-x-addon-symbolic"))
}

/// The size the panel draws a tile at.
fn cc_size(t: &CcTile) -> (u32, u32) {
    let size = (t.cols.min(4), t.rows);
    match CC_TILES.iter().find(|(i, ..)| *i == t.widget) {
        Some((.., sizes)) if !sizes.contains(&size) => sizes[0],
        _ => size,
    }
}

/// The pages through every layer, or the panel's own layout.
fn cc_pages(file: &toml::Table) -> Vec<Vec<CcTile>> {
    let parse = |s: &str| -> Option<(u32, u32)> {
        let (c, r) = s.split_once('x')?;
        let (c, r) = (c.trim().parse::<u32>().ok()?, r.trim().parse::<u32>().ok()?);
        (1..=4).contains(&c).then_some(())?;
        (1..=4).contains(&r).then_some((c, r))
    };
    let pages: Vec<Vec<CcTile>> = file
        .get("pages")
        .and_then(|v| v.as_array())
        .map(|pages| {
            pages
                .iter()
                .map(|p| {
                    p.get("tiles")
                        .and_then(|t| t.as_array())
                        .map(|tiles| {
                            tiles
                                .iter()
                                .filter_map(|t| {
                                    let widget = t.get("widget")?.as_str()?.to_string();
                                    let (cols, rows) = parse(t.get("size")?.as_str()?)?;
                                    Some(CcTile { widget, cols, rows })
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    if !pages.is_empty() {
        return pages;
    }
    CC_DEFAULT
        .iter()
        .map(|p| p.iter().map(|(w, (c, r))| CcTile { widget: w.to_string(), cols: *c, rows: *r }).collect())
        .collect()
}

/// Place tiles in order, each in the first spot where it fits, as the panel does.
fn cc_pack(tiles: &[CcTile]) -> Vec<(u32, u32)> {
    let mut taken: Vec<[bool; 4]> = Vec::new();
    let mut out = Vec::with_capacity(tiles.len());
    for t in tiles {
        let (cols, rows) = cc_size(t);
        let mut row = 0u32;
        let spot = loop {
            while taken.len() < (row + rows) as usize {
                taken.push([false; 4]);
            }
            let free = |c: u32| (row..row + rows).all(|r| (c..c + cols).all(|cc| !taken[r as usize][cc as usize]));
            if let Some(col) = (0..=4 - cols).find(|&c| free(c)) {
                break (col, row);
            }
            row += 1;
        };
        for r in spot.1..spot.1 + rows {
            for c in spot.0..spot.0 + cols {
                taken[r as usize][c as usize] = true;
            }
        }
        out.push(spot);
    }
    out
}

/// An icon centred in a box: the box takes no spare room, the icon fills the box.
fn cc_icon_box(icon: &str, class: &str) -> gtk::Box {
    let b = widgets::hbox(0);
    b.add_css_class(class);
    b.set_valign(gtk::Align::Center);
    b.set_halign(gtk::Align::Start);
    let i = gtk::Image::from_icon_name(icon);
    i.set_hexpand(true);
    i.set_vexpand(true);
    b.append(&i);
    b.set_hexpand(false);
    b.set_vexpand(false);
    b
}

/// A tile's rounded square with its icon; in the tile's colour when on.
fn cc_chip(icon: &str, on: bool) -> gtk::Box {
    let c = cc_icon_box(icon, "ccp-chip");
    if on {
        c.add_css_class("on");
    }
    c
}

fn cc_frame(class: &str, swatch: &str) -> gtk::Box {
    let b = widgets::vbox(0);
    b.add_css_class("ccp-tile");
    b.add_css_class(class);
    if !swatch.is_empty() {
        b.add_css_class(&format!("ccp-{swatch}"));
    }
    b
}

fn cc_text(title: &str, status: &str) -> gtk::Box {
    let t = widgets::vbox(0);
    t.set_valign(gtk::Align::Center);
    t.set_hexpand(true);
    for (s, class) in [(title, "ccp-title"), (status, "ccp-sub")] {
        if s.is_empty() {
            continue;
        }
        let l = widgets::label(s, class);
        l.set_xalign(0.0);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        // The panel's width is set; long text ellipsizes instead of widening it.
        l.set_max_width_chars(6);
        t.append(&l);
    }
    t
}

/// A bar as the panel's sliders draw it, at `value` (0 to 1); only to look at.
fn cc_bar(value: f64) -> gtk::Scale {
    let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
    s.set_value(value);
    s.set_draw_value(false);
    s.set_hexpand(true);
    s.set_valign(gtk::Align::Center);
    s.set_can_target(false);
    s.set_focusable(false);
    s
}

/// What a toggle shows in the picture: on or off, its colour and its status line,
/// made up but typical.
fn cc_state(id: &str) -> (bool, &'static str, &'static str) {
    match id {
        "wifi" => (true, "blue", "Home"),
        "bluetooth" => (false, "teal", "Off"),
        "devices" => (true, "blue", "Pixel 8 · 64%"),
        "vpn" => (false, "green", "Direct"),
        "tailscale" => (true, "violet", "4 online"),
        "dnd" => (false, "violet", "Off"),
        "mode" => (true, "green", "Tiling"),
        "microphone" => (false, "coral", "On"),
        "night-light" => (false, "coral", "Off"),
        "keep-awake" => (false, "amber", "Off"),
        "capture" => (false, "pink", "Screenshot"),
        "keyboard-light" => (true, "teal", "66%"),
        _ => (false, "", ""),
    }
}

/// One tile as the panel draws it, at its size, with made-up but typical contents.
fn cc_tile(t: &CcTile, (cols, _): (u32, u32)) -> gtk::Box {
    let (name, icon) = cc_name_icon(&t.widget);
    let (on, swatch, status) = cc_state(&t.widget);
    let tile = match t.widget.as_str() {
        "wifi" | "bluetooth" | "devices" | "vpn" | "tailscale" if cols >= 2 => {
            let b = widgets::hbox(0);
            b.add_css_class("ccp-tile");
            b.add_css_class("ccp-split");
            b.add_css_class(&format!("ccp-{swatch}"));
            if on {
                b.add_css_class("on");
            }
            let main = widgets::hbox(8);
            main.add_css_class("ccp-split-main");
            main.set_hexpand(true);
            main.append(&cc_chip(icon, on));
            main.append(&cc_text(&name, status));
            b.append(&main);
            let more = cc_icon_box("go-next-symbolic", "ccp-more");
            more.set_valign(gtk::Align::Fill);
            b.append(&more);
            b
        }
        "sound" => {
            let b = cc_frame("ccp-sliders", "");
            b.set_homogeneous(true);
            for (icon, value, swatch, pct, end) in [
                ("audio-volume-medium-symbolic", 0.45, "blue", "45%", "audio-speakers-symbolic"),
                ("display-brightness-symbolic", 0.7, "amber", "70%", "video-display-symbolic"),
            ] {
                let r = widgets::hbox(6);
                r.add_css_class("ccp-slider");
                r.add_css_class(&format!("ccp-{swatch}"));
                r.append(&cc_icon_box(icon, "ccp-ib"));
                r.append(&cc_bar(value));
                let v = widgets::label(pct, "ccp-value");
                v.set_width_chars(4);
                v.set_xalign(1.0);
                r.append(&v);
                if cols >= 4 {
                    r.append(&cc_icon_box(end, "ccp-ib"));
                }
                b.append(&r);
            }
            b
        }
        "media" => {
            let b = cc_frame("ccp-media", "pink");
            let row = widgets::hbox(10);
            row.set_vexpand(true);
            let art = cc_icon_box("audio-x-generic-symbolic", "ccp-art");
            row.append(&art);
            let text = cc_text("Midnight City", "M83 · Spotify");
            let prog = widgets::hbox(5);
            prog.add_css_class("ccp-progress");
            prog.append(&widgets::label("1:42", "ccp-time"));
            prog.append(&cc_bar(0.42));
            prog.append(&widgets::label("4:03", "ccp-time"));
            text.append(&prog);
            row.append(&text);
            let buttons = widgets::hbox(2);
            buttons.set_valign(gtk::Align::Center);
            buttons.append(&cc_icon_box("media-skip-backward-symbolic", "ccp-mb"));
            buttons.append(&cc_icon_box("media-playback-pause-symbolic", "ccp-play"));
            buttons.append(&cc_icon_box("media-skip-forward-symbolic", "ccp-mb"));
            row.append(&buttons);
            b.append(&row);
            b
        }
        "power" => {
            let b = cc_frame("ccp-power", "");
            let seg = widgets::hbox(2);
            seg.add_css_class("ccp-seg");
            seg.set_homogeneous(true);
            seg.set_vexpand(true);
            for (icon, swatch, picked) in [
                ("power-profile-power-saver-symbolic", "green", false),
                ("power-profile-balanced-symbolic", "blue", true),
                ("power-profile-performance-symbolic", "coral", false),
            ] {
                let s = cc_icon_box(icon, "ccp-segb");
                s.set_halign(gtk::Align::Fill);
                s.set_valign(gtk::Align::Fill);
                s.add_css_class(&format!("ccp-{swatch}"));
                if picked {
                    s.add_css_class("on");
                }
                seg.append(&s);
            }
            b.append(&seg);
            b
        }
        "keyboard-light" => {
            let b = cc_frame("ccp-toggle", swatch);
            let centre = gtk::CenterBox::new();
            centre.set_vexpand(true);
            let chip = cc_chip(icon, on);
            chip.set_halign(gtk::Align::Center);
            centre.set_center_widget(Some(&chip));
            let pips = widgets::vbox(4);
            pips.set_valign(gtk::Align::Center);
            for i in (1..=3).rev() {
                let p = widgets::hbox(0);
                p.add_css_class("ccp-pip");
                if i <= 2 {
                    p.add_css_class("lit");
                }
                pips.append(&p);
            }
            centre.set_end_widget(Some(&pips));
            b.append(&centre);
            b
        }
        "battery" => {
            let b = cc_frame("ccp-battery", "green");
            let r = widgets::hbox(6);
            r.set_halign(gtk::Align::Center);
            r.set_valign(gtk::Align::Center);
            r.set_vexpand(true);
            r.append(&gtk::Image::from_icon_name("battery-good-symbolic"));
            r.append(&widgets::label("78%", "ccp-title"));
            b.append(&r);
            b
        }
        "upcoming" | "home" | "backup" => {
            let (class, swatch, list): (&str, &str, &[(&str, &str, &str)]) = match t.widget.as_str() {
                "upcoming" => (
                    "ccp-list",
                    "red",
                    &[
                        ("Design review", "in 25 min", "blue"),
                        ("Lunch with Sam", "12:30", "green"),
                        ("Dentist", "Tomorrow 09:00", "amber"),
                        ("Rent due", "Monday", "red"),
                    ],
                ),
                "home" => (
                    "ccp-list",
                    "amber",
                    &[("Living room", "Lights on", ""), ("Thermostat", "21°", ""), ("Front door", "Locked", "")],
                ),
                _ => ("ccp-list", "blue", &[("Last backup", "2 h ago", ""), ("Next", "Tonight 02:00", ""), ("Size", "48 GB", "")]),
            };
            let b = cc_frame(class, swatch);
            let head = widgets::hbox(6);
            head.add_css_class("ccp-list-head");
            head.append(&gtk::Image::from_icon_name(icon));
            head.append(&widgets::label(&name, "ccp-title"));
            b.append(&head);
            let rows_shown = if cols >= 4 { 4 } else { 3 };
            for (title, when, swatch) in list.iter().take(rows_shown) {
                let e = widgets::hbox(8);
                e.add_css_class("ccp-event");
                if !swatch.is_empty() {
                    e.add_css_class(&format!("ccp-ev-{swatch}"));
                }
                let l = widgets::label(title, "ccp-event-title");
                l.set_xalign(0.0);
                l.set_hexpand(true);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                l.set_max_width_chars(6);
                e.append(&l);
                e.append(&widgets::label(when, "ccp-event-when"));
                b.append(&e);
            }
            b
        }
        _ => {
            let b = cc_frame("ccp-toggle", swatch);
            if cols == 1 {
                let chip = cc_chip(icon, on);
                chip.set_halign(gtk::Align::Center);
                chip.set_vexpand(true);
                b.append(&chip);
            } else {
                let r = widgets::hbox(10);
                r.set_vexpand(true);
                r.append(&cc_chip(icon, on));
                let status = if t.widget.starts_with("plugin:") { "Plugin" } else { status };
                r.append(&cc_text(&name, status));
                b.append(&r);
            }
            b
        }
    };
    tile.set_tooltip_text(Some(&name));
    tile
}

/// What the notes beside the panel point at: the widget, a bold part and the rest.
type Notes = Rc<RefCell<Vec<(gtk::Widget, String, String)>>>;

/// The panel as it opens, showing page `index`. Fills `notes` with its parts.
fn cc_panel(pages: &[Vec<CcTile>], index: usize, remember: bool, notes: &Notes) -> gtk::Box {
    let panel = widgets::vbox(10);
    panel.add_css_class("cc-panel");
    panel.set_valign(gtk::Align::Start);
    let head = widgets::hbox(8);
    head.add_css_class("ccp-head");
    let real = glib::real_name().to_string_lossy().to_string();
    let name = if real.is_empty() || real == "Unknown" { glib::user_name().to_string_lossy().to_string() } else { real };
    let avatar = widgets::label(&name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(), "ccp-avatar");
    avatar.set_valign(gtk::Align::Center);
    avatar.set_xalign(0.5);
    head.append(&avatar);
    head.append(&cc_text(&name, "Battery 78% · 3 h 20 min left"));
    let pencil = cc_icon_box("document-edit-symbolic", "ccp-hb");
    head.append(&pencil);
    head.append(&cc_icon_box("emblem-system-symbolic", "ccp-hb"));
    head.append(&cc_icon_box("system-shutdown-symbolic", "ccp-hb"));
    panel.append(&head);
    let grid = gtk::Grid::new();
    grid.set_column_homogeneous(true);
    grid.set_row_homogeneous(true);
    grid.set_column_spacing(6);
    grid.set_row_spacing(6);
    grid.add_css_class("cc-grid");
    let tiles = pages.get(index).cloned().unwrap_or_default();
    let mut n = notes.borrow_mut();
    n.clear();
    n.push((pencil.clone().upcast(), "Pencil".into(), ": change the tiles".into()));
    for (t, (col, row)) in tiles.iter().zip(cc_pack(&tiles)) {
        let size = cc_size(t);
        let w = cc_tile(t, size);
        grid.attach(&w, col as i32, row as i32, size.0 as i32, size.1 as i32);
        match t.widget.as_str() {
            "dnd" => n.push((w.clone().upcast(), "Do not disturb".into(), ", as in Notifications".into())),
            "sound" => n.push((w.clone().upcast(), String::new(), "Volume and brightness".into())),
            "media" => n.push((w.clone().upcast(), "Now playing".into(), ", while something plays".into())),
            _ => {}
        }
    }
    if tiles.is_empty() {
        let l = widgets::label("No tiles on this page", "cc-empty");
        grid.attach(&l, 0, 0, 4, 1);
    }
    panel.append(&grid);
    if pages.len() > 1 {
        let dots = widgets::hbox(4);
        dots.set_halign(gtk::Align::Center);
        for i in 0..pages.len() {
            let d = widgets::hbox(0);
            d.add_css_class("cc-dot");
            if i == index {
                d.add_css_class("on");
            }
            dots.append(&d);
        }
        panel.append(&dots);
        let (bold, rest) = if remember { ("Opens on the last page", " you left") } else { ("Always opens", " on the first page") };
        n.push((dots.upcast(), bold.into(), rest.into()));
    }
    panel
}

/// Notes beside the panel, each with a line to the part it's about.
fn cc_notes(panel_slot: &gtk::Box, notes: &Notes) -> gtk::DrawingArea {
    let (slot, notes) = (panel_slot.clone(), notes.clone());
    let area = widgets::drawing(10, move |cr, p, sec, w, _h| {
        let Some(panel) = slot.first_child() else { return };
        let mut last = f64::MIN;
        for (target, bold, rest) in notes.borrow().iter() {
            let pt = gtk::graphene::Point::new(0.0, target.height() as f32 / 2.0);
            let Some(at) = target.compute_point(&panel, &pt) else { continue };
            let y = at.y() as f64;
            if y - last < 20.0 {
                continue;
            }
            last = y;
            cr.arc(5.0, y, 3.5, 0.0, TAU);
            set(cr, sec, 1.0);
            let _ = cr.fill();
            set(cr, p.dim, 0.6);
            cr.rectangle(12.0, y - 0.5, 16.0, 1.0);
            let _ = cr.fill();
            let mut x = 34.0;
            if !bold.is_empty() {
                x += text(cr, bold, x, y + 4.0, 11.5, true, p.text, 1.0);
            }
            let room = w - x - 4.0;
            let mut shown = rest.clone();
            while !shown.is_empty() && text_width(cr, &shown, 11.5, false) > room {
                shown.pop();
            }
            text(cr, &shown, x, y + 4.0, 11.5, false, p.dim, 1.0);
        }
    });
    area.remove_css_class("preview-drawing");
    area.set_content_width(220);
    area.set_hexpand(true);
    area.set_vexpand(true);
    area
}

/// A strip of the bar with Nyx Status and the Control Center button outlined.
fn cc_bar_strip(status: bool, button: bool) -> (gtk::Box, gtk::Box, gtk::Box) {
    let strip = widgets::hbox(8);
    strip.add_css_class("cc-barstrip");
    let ws = widgets::hbox(4);
    ws.set_valign(gtk::Align::Center);
    for i in 0..3 {
        let d = widgets::hbox(0);
        d.add_css_class("cc-ws");
        if i == 0 {
            d.add_css_class("on");
        }
        ws.append(&d);
    }
    strip.append(&ws);
    let c = widgets::label(&clock(), "cc-clock");
    c.set_hexpand(true);
    strip.append(&c);
    let st = widgets::hbox(8);
    st.add_css_class("cc-status");
    tint(&st);
    for icon in ["weather-few-clouds-symbolic", "utilities-system-monitor-symbolic", "network-wireless-symbolic", "audio-volume-high-symbolic", "battery-good-symbolic"] {
        st.append(&gtk::Image::from_icon_name(icon));
    }
    strip_mark(&st, status);
    strip.append(&st);
    let btn = widgets::hbox(0);
    btn.add_css_class("cc-status");
    btn.add_css_class("cc-button");
    tint(&btn);
    btn.append(&gtk::Image::from_icon_name("view-grid-symbolic"));
    strip_mark(&btn, button);
    strip.append(&btn);
    (strip, st, btn)
}

/// An item in the bar strip: outlined while it's on the bar, faint while it isn't.
fn strip_mark(w: &gtk::Box, on: bool) {
    if on {
        w.remove_css_class("off");
    } else {
        w.add_css_class("off");
    }
    w.set_tooltip_text(Some(if on { "On the bar" } else { "Not on the bar" }));
}

pub fn build_control_center(page: &Page) {
    let file = merged("control-center");
    let remember = file.get("remember_page").and_then(|v| v.as_bool()).unwrap_or(true);
    let pages = Rc::new(cc_pages(&file));
    let shown = Rc::new(Cell::new(0usize));
    let remember_now = Rc::new(Cell::new(remember));

    // ---- The picture: the panel as it opens, with notes on its parts.
    let pv = page.split();
    let n_pages = pages.len();
    let line = widgets::label("", "preview-line");
    line.set_wrap(true);
    line.set_xalign(0.0);
    let page_opts: Vec<(String, String)> = (0..n_pages).map(|i| (i.to_string(), format!("Page {}", i + 1))).collect();
    let page_seg = widgets::segmented(&page_opts, "0", |_| {});
    page_seg.set_visible(n_pages > 1);
    let head = pv.head("Control Center", "", &[page_seg.upcast_ref()]);
    if let Some(text) = head.first_child().and_downcast::<gtk::Box>() {
        text.append(&line);
    }
    let stage = widgets::hbox(0);
    stage.add_css_class("cc-stage");
    stage.set_vexpand(true);
    let slot = widgets::hbox(0);
    slot.set_valign(gtk::Align::Start);
    slot.set_hexpand(false);
    let notes: Notes = Rc::default();
    let notes_area = cc_notes(&slot, &notes);
    stage.append(&slot);
    stage.append(&notes_area);
    pv.add(&stage);
    let show: Rc<dyn Fn()> = {
        let (pages, shown, remember_now, slot, notes, notes_area, line) =
            (pages.clone(), shown.clone(), remember_now.clone(), slot.clone(), notes.clone(), notes_area.clone(), line.clone());
        Rc::new(move || {
            while let Some(c) = slot.first_child() {
                slot.remove(&c);
            }
            slot.append(&cc_panel(&pages, shown.get(), remember_now.get(), &notes));
            line.set_text(&if pages.len() > 1 {
                format!("Page {} of {}, as it opens from the bar.", shown.get() + 1, pages.len())
            } else {
                "As it opens from the bar.".to_string()
            });
            let a = notes_area.clone();
            glib::idle_add_local_once(move || a.queue_draw());
        })
    };
    show();
    {
        let mut c = page_seg.first_child();
        let mut i = 0usize;
        while let Some(child) = c {
            if let Some(b) = child.downcast_ref::<gtk::ToggleButton>() {
                let (shown, show) = (shown.clone(), show.clone());
                b.connect_toggled(move |b| {
                    if b.is_active() {
                        shown.set(i);
                        show();
                    }
                });
            }
            i += 1;
            c = child.next_sibling();
        }
    }
    {
        let a = notes_area.clone();
        slot.connect_notify_local(Some("height-request"), move |_, _| a.queue_draw());
    }

    // ---- The controls.
    let g = page.group("Opening it");
    let caps = widgets::key_caps(&["Super", "D"]);
    g.add(&widgets::icon_row("input-keyboard-symbolic", "Keys", "Or the button in the bar.", Some(caps.upcast_ref())));
    widgets::keywords("open it super d shortcut");
    let sw = gtk::Switch::new();
    sw.set_active(remember);
    {
        let (remember_now, show) = (remember_now.clone(), show.clone());
        sw.connect_active_notify(move |s| {
            let on = s.is_active();
            match set_key("control-center", "remember_page", toml::Value::Boolean(on)) {
                // It reads its layout when it starts; it's hidden, so a restart is unseen.
                Ok(()) => run(&["systemctl", "--user", "try-restart", "nyx-control-center.service"], || {}),
                Err(e) => window::toast(&format!("Couldn't save: {e}")),
            }
            remember_now.set(on);
            show();
        });
    }
    g.add(&widgets::icon_row("view-paged-symbolic", "Open on the last page", "Off: it always opens on the first.", Some(sw.upcast_ref())));

    // Omarchy's bar takes these as its plugins; Nyx Bar's items (NyxOS) are its own
    // plugins, under Bar Plugins.
    if crate::platform::PLATFORM == crate::platform::Platform::Omarchy {
        let g = page.group("In the bar");
        let status_on = on_bar("design-nexus.status");
        let button_on = on_bar("design-nexus.control-center");
        let (strip, st, btn) = cc_bar_strip(status_on, button_on);
        g.add(&strip);
        g.add(&bar_switch(
            "design-nexus.status",
            "weather-few-clouds-symbolic",
            "Nyx Status",
            "Weather, CPU, memory, network, sound and battery. What it shows is set under Bar Plugins.",
            move |on| strip_mark(&st, on),
        ));
        g.add(&bar_switch(
            "design-nexus.control-center",
            "view-grid-symbolic",
            "Control Center button",
            "One icon that opens and closes it.",
            move |on| strip_mark(&btn, on),
        ));
        widgets::keywords("bar status widget weather cpu memory battery button");
    }

    let g = page.group("Tiles");
    let count = if n_pages == 1 { "1 page".to_string() } else { format!("{n_pages} pages") };
    let pages2 = pages.clone();
    let (r, _) = widgets::nav_row(Some("view-grid-symbolic"), "Change the tiles", "Add, take off and move them, page by page.", &count, move || {
        let pages = pages2.clone();
        widgets::subpage("The tiles", 520, move |sp| {
            let g = sp.group("");
            g.note("The tiles are changed in the panel itself: open it and press the pencil at its top. Drag a tile to move it; × takes it off.");
            let (r, _) = widgets::button_row("Open the Control Center", "", "Open", |_| cmd::spawn(&["nyx-control-center", "--show"]));
            g.add(&r);
            for (i, tiles) in pages.iter().enumerate() {
                let g = sp.group(&format!("Page {}", i + 1));
                for t in tiles {
                    let (name, icon) = cc_name_icon(&t.widget);
                    g.add(&widgets::icon_row(icon, &name, &format!("{} × {}", t.cols, t.rows), None));
                }
                if tiles.is_empty() {
                    g.add(&widgets::row("No tiles", "", None));
                }
            }
        });
    });
    g.add(&r);
    widgets::keywords("tiles layout pages edit pencil change add remove move");
    // The user's own layout goes; NyxOS's (or the Control Center's own) shows through.
    let reset = widgets::confirm_button("Restore", "Click again to restore", move |_| match reset_key("control-center", "pages") {
        Ok(()) => run(&["systemctl", "--user", "try-restart", "nyx-control-center.service"], || {
            window::toast("The Control Center has its standard tiles again");
            window::rebuild("control-center");
        }),
        Err(e) => window::toast(&format!("Couldn't restore: {e}")),
    });
    g.add(&widgets::icon_row("view-refresh-symbolic", "Standard layout", "Put back the standard tiles and pages.", Some(reset.upcast_ref())));
    widgets::keywords("control center quick settings tiles layout pages reset restore standard");

    on_narrow(page, move |narrow| {
        notes_area.set_visible(!narrow);
        stage.set_halign(if narrow { gtk::Align::Center } else { gtk::Align::Fill });
        stage.set_margin_start(if narrow { 0 } else { 20 });
    });
}

// ---------------------------------------------------------------- Notifications

/// An app's icon as the notification service names it: a file, a theme icon or the app.
fn notify_icon(icons: &std::collections::HashMap<String, gio::Icon>, app: &serde_json::Value) -> gio::Icon {
    let icon = app["app_icon"].as_str().unwrap_or_default();
    if icon.starts_with('/') && std::path::Path::new(icon).exists() {
        return gio::FileIcon::new(&gio::File::for_path(icon)).upcast();
    }
    if !icon.is_empty() && gtk::IconTheme::default().has_icon(icon) {
        return gio::ThemedIcon::new(icon).upcast();
    }
    let id = app["app"].as_str().unwrap_or_default();
    if let Some(i) = icons.get(id).or_else(|| icons.iter().find(|(k, _)| k.eq_ignore_ascii_case(id)).map(|(_, v)| v)) {
        return i.clone();
    }
    let lower = id.to_lowercase();
    if gtk::IconTheme::default().has_icon(&lower) {
        return gio::ThemedIcon::new(&lower).upcast();
    }
    gio::ThemedIcon::new("preferences-system-notifications-symbolic").upcast()
}

/// A pop-up as it arrives: the app, then its title and text as lines.
fn notify_popup(name: &str, icon: &gio::Icon, when: &str, with_text: bool) -> gtk::Box {
    let b = widgets::vbox(6);
    b.add_css_class("nt-pop");
    let h = widgets::hbox(6);
    let i = gtk::Image::from_gicon(icon);
    i.set_pixel_size(16);
    h.append(&i);
    let n = widgets::label(name, "nt-app");
    n.set_xalign(0.0);
    n.set_hexpand(true);
    n.set_ellipsize(gtk::pango::EllipsizeMode::End);
    h.append(&n);
    h.append(&widgets::label(when, "nt-when"));
    b.append(&h);
    let line = |w: i32, class: &str| {
        let l = widgets::hbox(0);
        l.add_css_class("nt-line");
        l.add_css_class(class);
        l.set_size_request(w, -1);
        l.set_halign(gtk::Align::Start);
        l
    };
    b.append(&line(120, "nt-title"));
    if with_text {
        b.append(&line(196, "nt-body"));
        b.append(&line(150, "nt-body"));
    }
    b
}

/// The top right of the screen: the bar with the bell, and a window under the pop-ups.
fn draw_corner(cr: &Context, p: &Pal, sec: Rgb, w: f64, h: f64, bell: bool) {
    let r = R::new(0.5, 0.5, w - 1.0, h - 1.0);
    cr.save().ok();
    rounded(cr, r.x, r.y, r.w, r.h, 9.0);
    cr.clip();
    wallpaper(cr, p, sec, r);
    let bh = 28.0;
    set(cr, p.bg, 0.94);
    cr.rectangle(r.x, r.y, r.w, bh);
    let _ = cr.fill();
    text(cr, &clock(), r.x + 14.0, r.y + 18.0, 11.0, true, p.text, 0.95);
    // The tray, and the bell at the right.
    let y = r.y + bh / 2.0;
    let bx = r.x + r.w - 26.0;
    set(cr, p.text, 0.75);
    cr.set_line_width(1.2);
    rounded(cr, bx - 44.0, y - 3.5, 11.0, 7.0, 1.5);
    let _ = cr.stroke();
    for (i, rad) in [1.6, 4.2, 6.8].iter().enumerate() {
        if i == 0 {
            cr.arc(bx - 62.0, y + 4.0, *rad, 0.0, TAU);
            let _ = cr.fill();
        } else {
            cr.arc(bx - 62.0, y + 4.0, *rad, -2.36, -0.78);
            let _ = cr.stroke();
        }
    }
    // The bell.
    cr.move_to(bx - 5.0, y + 3.0);
    cr.curve_to(bx - 5.0, y - 6.0, bx + 5.0, y - 6.0, bx + 5.0, y + 3.0);
    cr.line_to(bx + 6.5, y + 4.5);
    cr.line_to(bx - 6.5, y + 4.5);
    cr.close_path();
    let _ = cr.stroke();
    cr.arc(bx, y + 6.5, 1.6, 0.0, TAU);
    let _ = cr.fill();
    if bell {
        rounded(cr, bx - 12.0, y - 10.0, 24.0, 20.0, 6.0);
        set(cr, sec, 1.0);
        cr.set_line_width(1.5);
        let _ = cr.stroke();
    }
    // A window behind the pop-ups.
    let wr = R::new(r.x + r.w * 0.06, r.y + bh + 16.0, r.w * 0.6, r.h);
    rounded(cr, wr.x, wr.y, wr.w, wr.h, 7.0);
    set(cr, mix(p.bg, p.text, 0.04), 1.0);
    let _ = cr.fill();
    bar(cr, wr.x + 14.0, wr.y + 16.0, wr.w * 0.36, 4.0, p.text, 0.45);
    for (i, f) in [0.92, 0.8, 0.92, 0.6, 0.8].iter().enumerate() {
        bar(cr, wr.x + 14.0, wr.y + 32.0 + i as f64 * 15.0, (wr.w - 28.0) * f, 3.0, p.dim, 0.4);
    }
    cr.restore().ok();
    rounded(cr, r.x, r.y, r.w, r.h, 9.0);
    set(cr, p.text, 0.12);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
}

/// Tonight across the day: the quiet hours, their ends and now.
fn draw_night(cr: &Context, p: &Pal, sec: Rgb, w: f64, _h: f64, from: u64, to: u64, on: bool) {
    let (x0, tw) = (1.0, w - 2.0);
    let (ty, th) = (16.0, 24.0);
    let at = |m: u64| x0 + tw * (m as f64 / 1440.0);
    rounded(cr, x0, ty, tw, th, 6.0);
    set(cr, p.text, 0.07);
    let _ = cr.fill();
    let spans: Vec<(u64, u64)> = if from == to {
        vec![]
    } else if from < to {
        vec![(from, to)]
    } else {
        vec![(0, to), (from, 1440)]
    };
    let alpha = if on { 0.42 } else { 0.16 };
    cr.save().ok();
    rounded(cr, x0, ty, tw, th, 6.0);
    cr.clip();
    for (a, b) in &spans {
        cr.rectangle(at(*a), ty, at(*b) - at(*a), th);
        set(cr, sec, alpha);
        let _ = cr.fill();
    }
    cr.restore().ok();
    if let Some((a, b)) = spans.first() {
        let label = format!("until {}", notifyd::hhmm(to));
        if at(*b) - at(*a) > text_width(cr, &label, 10.5, true) + 16.0 {
            text(cr, &label, at(*a) + 10.0, ty + th / 2.0 + 4.0, 10.5, true, p.text, if on { 0.95 } else { 0.5 });
        }
    }
    for m in [from, to] {
        if from != to {
            rounded(cr, at(m) - 1.5, ty - 2.0, 3.0, th + 4.0, 1.5);
            set(cr, sec, if on { 1.0 } else { 0.5 });
            let _ = cr.fill();
        }
    }
    // Now.
    if let Ok(d) = glib::DateTime::now_local() {
        let x = at(d.hour() as u64 * 60 + d.minute() as u64);
        set(cr, p.text, 0.95);
        cr.rectangle(x - 1.0, ty - 4.0, 2.0, th + 8.0);
        let _ = cr.fill();
        let tw2 = text_width(cr, "Now", 10.0, true);
        text(cr, "Now", (x - tw2 / 2.0).min(w - tw2).max(0.0), ty - 7.0, 10.0, true, p.text, 0.95);
    }
    for (i, l) in ["00:00", "06:00", "12:00", "18:00", "24:00"].iter().enumerate() {
        let x = at(i as u64 * 360);
        let lw = text_width(cr, l, 10.0, false);
        text(cr, l, (x - lw / 2.0).min(w - lw).max(0.0), ty + th + 16.0, 10.0, false, p.dim, 1.0);
    }
}

/// A 24-hour time field that applies when Enter is pressed or it loses focus.
fn time_field(value: &str, on_apply: impl Fn(String) + 'static) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.set_text(value);
    e.set_placeholder_text(Some("HH:MM"));
    e.set_width_chars(5);
    e.set_max_width_chars(5);
    e.add_css_class("mono");
    let last = Rc::new(RefCell::new(value.to_string()));
    let apply = Rc::new(on_apply);
    let fire = move |e: &gtk::Entry| {
        let t = e.text().to_string();
        if *last.borrow() != t {
            *last.borrow_mut() = t.clone();
            apply(t);
        }
    };
    let f1 = fire.clone();
    e.connect_activate(move |e| f1(e));
    let focus = gtk::EventControllerFocus::new();
    let e2 = e.clone();
    focus.connect_leave(move |_| fire(&e2));
    e.add_controller(focus);
    e
}

/// Each app's pop-ups and history, one level down.
fn notify_apps(sp: &Page, apps: &[serde_json::Value], nyx: bool, state: &Rc<RefCell<serde_json::Value>>, save: &Rc<dyn Fn()>, changed: &Rc<dyn Fn()>) {
    let g = sp.group("");
    g.note(if nyx {
        "Turn off pop-ups to have an app's notifications arrive quietly in the Inbox; turn off history to keep none of them."
    } else {
        "Turn off history to keep none of an app's notifications."
    });
    let icons = app_icons();
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
            let (s2, sv, ch, app2) = (state.clone(), save.clone(), changed.clone(), app.clone());
            pop.connect_state_set(move |_, on| {
                edit_list(&mut s2.borrow_mut(), "no_popups", &app2, !on);
                sv();
                ch();
                glib::Propagation::Proceed
            });
            switches.append(&gtk::Label::new(Some("Pop-ups")));
            switches.append(&pop);
        }
        let hist = gtk::Switch::new();
        hist.set_valign(gtk::Align::Center);
        hist.set_tooltip_text(Some("History"));
        let kept = !state.borrow()["excluded"].as_array().is_some_and(|l| l.iter().any(|x| x == app.as_str()))
            && !a["excluded"].as_bool().unwrap_or(false);
        hist.set_active(kept);
        let (s2, sv, ch, app2) = (state.clone(), save.clone(), changed.clone(), app.clone());
        hist.connect_state_set(move |_, on| {
            edit_list(&mut s2.borrow_mut(), "excluded", &app2, !on);
            sv();
            ch();
            glib::Propagation::Proceed
        });
        switches.append(&gtk::Label::new(Some("History")));
        switches.append(&hist);
        let count = a["count"].as_i64().unwrap_or(0);
        let desc = if count == 1 { "1 notification".to_string() } else { format!("{count} notifications") };
        let r = widgets::row(&name, &desc, Some(switches.upcast_ref()));
        let i = gtk::Image::from_gicon(&notify_icon(&icons, a));
        i.set_pixel_size(24);
        i.set_valign(gtk::Align::Center);
        r.prepend(&i);
        g.add(&r);
    }
}

/// How many apps are quieted or keep no history.
fn apps_changed(state: &serde_json::Value) -> usize {
    let mut names: Vec<&str> = Vec::new();
    for key in ["no_popups", "excluded"] {
        for v in state[key].as_array().into_iter().flatten() {
            if let Some(s) = v.as_str()
                && !names.contains(&s)
            {
                names.push(s);
            }
        }
    }
    names.len()
}

pub fn build_notifications(page: &Page) {
    let Ok(settings) = notifyd::settings() else {
        let g = page.group("Notification Center");
        g.add(&widgets::row("Open it", "<tt>Super N</tt>, or the bell in the bar.", None));
        g.note(
            "The notification service (nyx-notifyd) isn't running. Start it with <tt>systemctl --user start nyx-notifyd</tt>.",
        );
        return;
    };
    let nyx = notifyd::mode().as_deref() == Some("nyx");
    // Only Omarchy has another notification service to hand pop-ups to; NyxOS's are always
    // the Notification Center's, and its bar's bell is a Nyx Bar plugin.
    let omarchy = crate::platform::PLATFORM == crate::platform::Platform::Omarchy;
    let apps = notifyd::apps();
    let state = Rc::new(RefCell::new(settings));
    let save: Rc<dyn Fn()> = {
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
    let with_text = Rc::new(Cell::new(!state.borrow()["hide_popup_text"].as_bool().unwrap_or(false)));
    let bell_on = Rc::new(Cell::new(omarchy && on_bar("design-nexus.notification-center")));

    // ---- The picture: pop-ups at the top right, and tonight's quiet hours.
    let pv = page.split();
    let text_seg = widgets::segmented(
        &[("text".to_string(), "With text".to_string()), ("title".to_string(), "Title only".to_string())],
        if with_text.get() { "text" } else { "title" },
        |_| {},
    );
    pv.head("As they arrive", "At the top right of the screen, under the bell.", &[text_seg.upcast_ref()]);
    let stage = gtk::Overlay::new();
    stage.add_css_class("nt-stage");
    stage.set_size_request(PICTURE_MIN, -1);
    stage.set_overflow(gtk::Overflow::Hidden);
    let corner = {
        let bell_on = bell_on.clone();
        widgets::drawing(250, move |cr, p, sec, w, h| draw_corner(cr, p, sec, w, h, bell_on.get()))
    };
    stage.set_child(Some(&corner));
    let pops = widgets::vbox(8);
    pops.set_halign(gtk::Align::End);
    pops.set_valign(gtk::Align::Start);
    pops.set_margin_top(38);
    pops.set_margin_end(14);
    pops.set_size_request(250, -1);
    stage.add_overlay(&pops);
    pv.add(&stage);
    // The apps that send the most, as their pop-ups would look.
    let mut senders: Vec<&serde_json::Value> = apps.iter().collect();
    senders.sort_by_key(|a| std::cmp::Reverse(a["count"].as_i64().unwrap_or(0)));
    let icons = app_icons();
    let senders: Vec<(String, gio::Icon)> =
        senders.iter().take(2).map(|a| (notifyd::app_name(a["app"].as_str().unwrap_or_default()), notify_icon(&icons, a))).collect();
    let narrow_now = Rc::new(Cell::new(false));
    let fill_pops: Rc<dyn Fn()> = {
        let (pops, with_text, narrow_now) = (pops.clone(), with_text.clone(), narrow_now.clone());
        Rc::new(move || {
            while let Some(c) = pops.first_child() {
                pops.remove(&c);
            }
            let list: Vec<(String, gio::Icon)> = if senders.is_empty() {
                vec![("Notification".to_string(), gio::ThemedIcon::new("preferences-system-notifications-symbolic").upcast())]
            } else {
                senders.clone()
            };
            let n = if narrow_now.get() { 1 } else { 2 };
            for (i, (name, icon)) in list.iter().take(n).enumerate() {
                pops.append(&notify_popup(name, icon, if i == 0 { "now" } else { "earlier" }, with_text.get()));
            }
        })
    };
    fill_pops();
    {
        let mut c = text_seg.first_child();
        while let Some(child) = c {
            if let Some(b) = child.downcast_ref::<gtk::ToggleButton>() {
                let text = b.label().is_some_and(|l| l.starts_with("With"));
                let (with_text, fill_pops) = (with_text.clone(), fill_pops.clone());
                b.connect_toggled(move |b| {
                    if b.is_active() {
                        with_text.set(text);
                        fill_pops();
                    }
                });
            }
            c = child.next_sibling();
        }
    }

    let night = pv.card();
    night.add_css_class("nt-night");
    let nh = widgets::hbox(8);
    nh.append(&widgets::label("Tonight", "nt-night-title"));
    let night_line = widgets::label("", "nt-night-line");
    night_line.set_hexpand(true);
    night_line.set_xalign(0.0);
    night_line.set_ellipsize(gtk::pango::EllipsizeMode::End);
    nh.append(&night_line);
    let chip = widgets::hbox(4);
    chip.add_css_class("nt-chip");
    tint(&chip);
    chip.append(&gtk::Image::from_icon_name("weather-clear-night-symbolic"));
    let chip_label = widgets::label("", "nt-chip-text");
    chip.append(&chip_label);
    nh.append(&chip);
    night.append(&nh);
    let strip = {
        let state = state.clone();
        widgets::drawing(64, move |cr, p, sec, w, h| {
            let s = state.borrow();
            let from = s["schedule"]["from"].as_u64().unwrap_or(22 * 60);
            let to = s["schedule"]["to"].as_u64().unwrap_or(7 * 60);
            let on = s["schedule"]["enabled"].as_bool().unwrap_or(false);
            draw_night(cr, p, sec, w, h, from, to, on);
        })
    };
    strip.remove_css_class("preview-drawing");
    night.append(&strip);
    let keep = widgets::hbox(8);
    keep.add_css_class("nt-keep");
    let ok = gtk::Image::from_icon_name("emblem-ok-symbolic");
    ok.set_valign(gtk::Align::Start);
    keep.append(&ok);
    let kl = widgets::label(
        "Critical alerts and sign-in codes still show. Turning it on or off by hand sticks until the next start or end.",
        "nt-keep-text",
    );
    kl.set_wrap(true);
    kl.set_xalign(0.0);
    kl.set_hexpand(true);
    keep.append(&kl);
    night.append(&keep);
    let describe_night: Rc<dyn Fn()> = {
        let (state, night_line, chip, chip_label, strip) = (state.clone(), night_line.clone(), chip.clone(), chip_label.clone(), strip.clone());
        Rc::new(move || {
            let s = state.borrow();
            let on = s["schedule"]["enabled"].as_bool().unwrap_or(false);
            let from = notifyd::hhmm(s["schedule"]["from"].as_u64().unwrap_or(22 * 60));
            let to = notifyd::hhmm(s["schedule"]["to"].as_u64().unwrap_or(7 * 60));
            night_line.set_text(&if on { format!("Quiet from {from} to {to}") } else { "Not on a schedule".to_string() });
            chip_label.set_text(if on { "On" } else { "Off" });
            if on {
                chip.add_css_class("on");
            } else {
                chip.remove_css_class("on");
            }
            strip.queue_draw();
        })
    };
    describe_night();
    let foot = pv.foot("dialog-information-symbolic", "Super N or the bell opens the Notification Center. It keeps an Inbox, pinned notifications and the history.");

    // ---- The controls.
    let g = page.group(if omarchy { "Pop-ups shown by" } else { "Pop-ups" });
    if omarchy {
        let choices = [
            ("nyx", "Nyx Notification Center", "With an Inbox, pinned notifications and the history"),
            ("omarchy", "Omarchy", "History is still kept"),
        ];
        let mut first: Option<gtk::CheckButton> = None;
        for (id, title, desc) in choices {
            let check = gtk::CheckButton::new();
            match &first {
                Some(f) => check.set_group(Some(f)),
                None => first = Some(check.clone()),
            }
            check.set_active((id == "nyx") == nyx);
            check.set_valign(gtk::Align::Center);
            check.connect_toggled(move |c| {
                if !c.is_active() {
                    return;
                }
                let arg = if id == "nyx" { "--use-nyx-popups" } else { "--use-omarchy-popups" };
                window::toast("Switching who shows notifications…");
                run(&["nyx-notification-center", arg], || {
                    window::toast("Done");
                    window::rebuild("notifications");
                });
            });
            let r = widgets::row(title, desc, None);
            r.prepend(&check);
            r.add_css_class("nt-choice");
            if (id == "nyx") == nyx {
                r.add_css_class("picked");
            }
            let click = gtk::GestureClick::new();
            let c2 = check.clone();
            click.connect_released(move |_, _, _, _| c2.set_active(true));
            r.add_controller(click);
            r.set_cursor_from_name(Some("pointer"));
            g.add(&r);
        }
        widgets::keywords("popups toasts omarchy nyx notification center who shows receives");
    }
    if nyx {
        let (s2, sv, wt, fp, seg) = (state.clone(), save.clone(), with_text.clone(), fill_pops.clone(), text_seg.clone());
        let (r, _) = widgets::switch_row("Show text in pop-ups", "Off: only the app and the title.", with_text.get(), move |on| {
            s2.borrow_mut()["hide_popup_text"] = (!on).into();
            sv();
            wt.set(on);
            fp();
            let want = if on { "With" } else { "Title" };
            let mut c = seg.first_child();
            while let Some(child) = c {
                if let Some(b) = child.downcast_ref::<gtk::ToggleButton>()
                    && b.label().is_some_and(|l| l.starts_with(want))
                {
                    b.set_active(true);
                }
                c = child.next_sibling();
            }
        });
        g.add(&r);
        widgets::keywords("privacy preview text popups content");
    }
    if omarchy {
        let (bell_on, corner) = (bell_on.clone(), corner.clone());
        let (r, _) = widgets::switch_row(
            "Bell in the bar",
            "Unread count; right-click for Do not disturb.",
            bell_on.get(),
            move |on| {
                if on {
                    run(&["omarchy-plugin-enable", "design-nexus.notification-center", "right"], || {});
                } else {
                    run(&["omarchy-plugin-disable", "design-nexus.notification-center"], || {});
                }
                bell_on.set(on);
                corner.queue_draw();
            },
        );
        g.add(&r);
        widgets::keywords("bell bar unread");
    }
    if !omarchy && !nyx {
        g.wrapper.set_visible(false);
    }
    widgets::keywords("notification center popups toasts omarchy inbox bell super n");

    let g = page.group("Do not disturb");
    let sched_on = state.borrow()["schedule"]["enabled"].as_bool().unwrap_or(false);
    let times = widgets::hbox(6);
    times.set_sensitive(sched_on);
    let (s2, sv, t2, dn) = (state.clone(), save.clone(), times.clone(), describe_night.clone());
    let (r, _) = widgets::switch_row("Every night", "Critical alerts and sign-in codes still show.", sched_on, move |on| {
        s2.borrow_mut()["schedule"]["enabled"] = on.into();
        t2.set_sensitive(on);
        sv();
        dn();
    });
    g.add(&r);
    widgets::keywords("dnd do not disturb schedule automatically night quiet silence");
    for (i, (key, default)) in [("from", 22 * 60), ("to", 7 * 60)].into_iter().enumerate() {
        times.append(&widgets::label(if i == 0 { "From" } else { "to" }, "nt-time-word"));
        let current = notifyd::hhmm(state.borrow()["schedule"][key].as_u64().unwrap_or(default));
        let (s2, sv, dn) = (state.clone(), save.clone(), describe_night.clone());
        let e = time_field(&current, move |t| match notifyd::minutes(&t) {
            Some(m) => {
                s2.borrow_mut()["schedule"][key] = m.into();
                sv();
                dn();
            }
            None => window::toast("Use a 24-hour time like 22:30"),
        });
        e.set_tooltip_text(Some(if i == 0 { "Starts at, as a 24-hour time like 22:30" } else { "Ends at, as a 24-hour time like 07:00" }));
        times.append(&e);
    }
    g.add(&widgets::row("Quiet hours", "", Some(times.upcast_ref())));
    widgets::keywords("starts ends at time hours schedule 24-hour");

    let g = page.group("History");
    let days = state.borrow()["retention_days"].as_u64().unwrap_or(30).to_string();
    let (s2, sv) = (state.clone(), save.clone());
    let keep_for = widgets::segmented(
        &[("7".into(), "7 days".into()), ("30".into(), "30 days".into()), ("90".into(), "90 days".into())],
        &days,
        move |v| {
            s2.borrow_mut()["retention_days"] = v.parse::<u64>().unwrap_or(30).into();
            sv();
        },
    );
    g.add(&widgets::row("Keep for", "", Some(keep_for.upcast_ref())));
    widgets::keywords("history retention days keep");
    let del = widgets::confirm_button("Delete", "Click again to delete", |_| {
        cmd::background(notifyd::delete_history, |r| match r {
            Ok(()) => window::toast("History deleted; pinned notifications kept"),
            Err(e) => window::toast(&format!("Couldn't delete: {e}")),
        });
    });
    del.add_css_class("destructive-action");
    g.add(&widgets::row("Delete history", "Pinned ones are always kept.", Some(del.upcast_ref())));
    widgets::keywords("history delete clear pinned");

    if !apps.is_empty() {
        let g = page.group("");
        let value = Rc::new(RefCell::new(None::<gtk::Label>));
        let changed: Rc<dyn Fn()> = {
            let (state, value) = (state.clone(), value.clone());
            Rc::new(move || {
                let n = apps_changed(&state.borrow());
                if let Some(l) = value.borrow().as_ref() {
                    l.set_text(&if n == 0 { String::new() } else { format!("{n} changed") });
                    l.set_visible(n > 0);
                }
            })
        };
        let (apps2, state2, save2, changed2) = (apps.clone(), state.clone(), save.clone(), changed.clone());
        let n = apps_changed(&state.borrow());
        let (r, v) = widgets::nav_row(
            Some("view-app-grid-symbolic"),
            "Apps",
            "Quiet an app, or keep no history",
            &if n == 0 { String::new() } else { format!("{n} changed") },
            move || {
                let (apps, state, save, changed) = (apps2.clone(), state2.clone(), save2.clone(), changed2.clone());
                widgets::subpage("Apps", 600, move |sp| notify_apps(sp, &apps, nyx, &state, &save, &changed));
            },
        );
        *value.borrow_mut() = Some(v);
        g.add(&r);
        widgets::keywords("apps per app mute quiet exclude history popups");
    }

    let foot_box = foot.parent();
    on_narrow(page, move |narrow| {
        night.set_visible(!narrow);
        if let Some(f) = &foot_box {
            f.set_visible(!narrow);
        }
        if narrow_now.get() != narrow {
            narrow_now.set(narrow);
            fill_pops();
        }
    });
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

    #[test]
    fn packs_tiles_first_fit() {
        let t = |c, r| super::CcTile { widget: String::new(), cols: c, rows: r };
        let spots = super::cc_pack(&[t(2, 1), t(1, 1), t(4, 1), t(1, 1)]);
        assert_eq!(spots, vec![(0, 0), (2, 0), (0, 1), (3, 0)]);
    }
}
