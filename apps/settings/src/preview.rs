//! Live previews at the top of Window Style and Tiling & Workspaces: a small
//! desktop drawn with the values on the page, redrawn as they change.

use crate::backend::{hypr, store};
use crate::sketch::{self, R, Win, mix, rounded, set};
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Pixels in the preview per pixel on a screen: windows on a real screen are
/// big, so gaps and borders are drawn a little larger than true scale.
const SCALE: f64 = 0.6;

type Vals = Rc<RefCell<HashMap<&'static str, Value>>>;

fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        // Gaps can be "5" or "5 10 5 10": the first is enough here.
        Value::String(s) => s.split_whitespace().next()?.parse().ok(),
        Value::Object(o) => o.values().find_map(|x| number(Some(x))),
        _ => None,
    }
}

fn current(key: &str) -> Option<Value> {
    store::option(key).or_else(|| hypr::get_option(key))
}

/// Read `keys` now, and keep them up to date while `area` lives.
fn watch(area: &gtk::DrawingArea, keys: &'static [&'static str]) -> Vals {
    let vals: Vals = Rc::default();
    for k in keys {
        if let Some(v) = current(k) {
            vals.borrow_mut().insert(k, v);
        }
    }
    for k in keys {
        let (vals, a) = (vals.clone(), area.downgrade());
        store::subscribe(area, k, move |key, value| {
            let Some(&k) = keys_match(keys, key) else { return };
            match value {
                Some(v) => {
                    vals.borrow_mut().insert(k, v.clone());
                    if let Some(a) = a.upgrade() {
                        a.queue_draw();
                    }
                }
                // Reset: the value comes back from the user's config after a reload.
                None => {
                    let (vals, a) = (vals.clone(), a.clone());
                    glib::timeout_add_local_once(std::time::Duration::from_millis(750), move || {
                        match hypr::get_option(k) {
                            Some(v) => vals.borrow_mut().insert(k, v),
                            None => vals.borrow_mut().remove(k),
                        };
                        if let Some(a) = a.upgrade() {
                            a.queue_draw();
                        }
                    });
                }
            }
        });
    }
    vals
}

fn keys_match<'a>(keys: &'a [&'static str], key: &str) -> Option<&'a &'static str> {
    keys.iter().find(|k| **k == key)
}

fn get(vals: &Vals, key: &str, default: f64) -> f64 {
    number(vals.borrow().get(key)).unwrap_or(default)
}

fn card(area: &gtk::DrawingArea, caption: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.add_css_class("preview-card");
    area.set_hexpand(true);
    b.append(area);
    let l = gtk::Label::new(Some(caption));
    l.add_css_class("preview-caption");
    l.set_xalign(0.0);
    b.append(&l);
    b
}

/// The wallpaper and the bar, and the area left for windows.
fn desktop(cr: &gtk::cairo::Context, p: &sketch::Pal, w: f64, h: f64) -> R {
    rounded(cr, 0.5, 0.5, w - 1.0, h - 1.0, 8.0);
    let grad = gtk::cairo::LinearGradient::new(0.0, 0.0, w, h);
    let a = mix(p.bg, p.accent, 0.35);
    let b = mix(p.bg, p.text, 0.08);
    grad.add_color_stop_rgb(0.0, a.0, a.1, a.2);
    grad.add_color_stop_rgb(1.0, b.0, b.1, b.2);
    let _ = cr.set_source(&grad);
    let _ = cr.fill();
    // The bar.
    set(cr, p.bg, 0.85);
    cr.rectangle(1.0, 1.0, w - 2.0, 14.0);
    let _ = cr.fill();
    for i in 0..5 {
        set(cr, if i == 0 { p.accent } else { p.dim }, 0.9);
        cr.arc(12.0 + i as f64 * 9.0, 8.0, 2.2, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
    R::new(1.0, 15.0, w - 2.0, h - 16.0)
}

pub const STYLE_KEYS: &[&str] = &[
    "general.gaps_in",
    "general.gaps_out",
    "general.border_size",
    "decoration.rounding",
    "decoration.active_opacity",
    "decoration.inactive_opacity",
    "decoration.dim_inactive",
    "decoration.dim_strength",
    "decoration.blur.enabled",
    "decoration.shadow.enabled",
    "decoration.shadow.range",
];

/// Pixels in the Window Style preview per pixel on a screen.
const STAGE_SCALE: f64 = 0.75;

fn text(cr: &gtk::cairo::Context, s: &str, size: f64) -> Option<gtk::cairo::TextExtents> {
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
    cr.set_font_size(size);
    cr.text_extents(s).ok()
}

/// A number on the drawing where it shows: a pill in the section's colour, centred on (x, y).
fn tag(cr: &gtk::cairo::Context, colour: sketch::Rgb, s: &str, x: f64, y: f64, bounds: R) {
    let Some(e) = text(cr, s, 10.5) else { return };
    let (w, h) = (e.width() + 14.0, 19.0);
    // A drawing narrower than the tag pins it to the left (or top) edge.
    let px = (x - w / 2.0).min(bounds.x + bounds.w - w - 3.0).max(bounds.x + 3.0);
    let py = (y - h / 2.0).min(bounds.y + bounds.h - h - 3.0).max(bounds.y + 3.0);
    rounded(cr, px, py, w, h, h / 2.0);
    set(cr, mix(colour, (0.0, 0.0, 0.0), 0.25), 0.95);
    let _ = cr.fill_preserve();
    set(cr, mix(colour, (1.0, 1.0, 1.0), 0.35), 1.0);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    set(cr, (1.0, 1.0, 1.0), 1.0);
    cr.move_to(px + 7.0 - e.x_bearing(), py + h / 2.0 - e.height() / 2.0 - e.y_bearing());
    let _ = cr.show_text(s);
}

fn fmt(v: f64) -> String {
    format!("{}", v.round() as i64)
}

/// Window Style: a large desktop drawn with the page's gaps, borders, rounding,
/// transparency, blur and shadows, each number labelled where it shows.
pub fn window_style_stage() -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_height(320);
    area.set_hexpand(true);
    area.add_css_class("preview-drawing");
    let vals = watch(&area, STYLE_KEYS);
    let section = crate::widgets::section_rgb();
    area.set_draw_func(move |_, cr, w, h| {
        let p = sketch::pal();
        let (w, h) = (w as f64, h as f64);
        let screen = R::new(0.5, 0.5, w - 1.0, h - 1.0);
        cr.save().ok();
        rounded(cr, screen.x, screen.y, screen.w, screen.h, 10.0);
        cr.clip();
        // The wallpaper: soft shapes, so see-through windows show something behind them.
        let grad = gtk::cairo::LinearGradient::new(0.0, 0.0, w, h);
        let (a, b) = (mix(p.bg, p.accent, 0.18), mix(p.bg, (0.0, 0.0, 0.0), 0.2));
        grad.add_color_stop_rgb(0.0, a.0, a.1, a.2);
        grad.add_color_stop_rgb(1.0, b.0, b.1, b.2);
        let _ = cr.set_source(&grad);
        let _ = cr.paint();
        for (cx, cy, r, c) in [(0.78, 0.32, 0.3, p.danger), (0.25, 0.95, 0.24, mix(p.accent, (0.2, 0.8, 0.7), 0.6))] {
            let g = gtk::cairo::RadialGradient::new(w * cx, h * cy, 0.0, w * cx, h * cy, w * r);
            g.add_color_stop_rgba(0.0, c.0, c.1, c.2, 0.75);
            g.add_color_stop_rgba(0.7, c.0, c.1, c.2, 0.45);
            g.add_color_stop_rgba(1.0, c.0, c.1, c.2, 0.0);
            let _ = cr.set_source(&g);
            let _ = cr.paint();
        }
        // The bar.
        let bar_h = 20.0;
        set(cr, p.bg, 0.88);
        cr.rectangle(0.0, 0.0, w, bar_h);
        let _ = cr.fill();
        for i in 0..4 {
            set(cr, if i == 0 { p.accent } else { p.dim }, if i == 0 { 1.0 } else { 0.6 });
            rounded(cr, 10.0 + if i == 0 { 0.0 } else { 12.0 + i as f64 * 9.0 }, 7.0, if i == 0 { 16.0 } else { 6.0 }, 6.0, 2.0);
            let _ = cr.fill();
        }
        if let Some(e) = text(cr, "Fri 14:17", 10.0) {
            set(cr, p.text, 0.9);
            cr.move_to(w / 2.0 - e.width() / 2.0 - e.x_bearing(), bar_h / 2.0 - e.height() / 2.0 - e.y_bearing());
            let _ = cr.show_text("Fri 14:17");
        }
        for i in 0..3 {
            set(cr, p.text, 0.6);
            cr.arc(w - 12.0 - i as f64 * 11.0, bar_h / 2.0, 2.4, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
        let room = R::new(0.0, bar_h, w, h - bar_h);

        let raw = |key: &str, d: f64| get(&vals, key, d).max(0.0);
        let (gin_v, gout_v) = (raw("general.gaps_in", 5.0), raw("general.gaps_out", 10.0));
        let (border_v, radius_v) = (raw("general.border_size", 2.0), raw("decoration.rounding", 0.0));
        let range_v = raw("decoration.shadow.range", 4.0);
        let (gin, gout, radius, range) = (gin_v * STAGE_SCALE, gout_v * STAGE_SCALE, radius_v * STAGE_SCALE, range_v * STAGE_SCALE);
        let on = get(&vals, "decoration.active_opacity", 1.0);
        let off = get(&vals, "decoration.inactive_opacity", 1.0);
        let dim = if get(&vals, "decoration.dim_inactive", 0.0) > 0.5 { get(&vals, "decoration.dim_strength", 0.5) } else { 0.0 };
        let blur = get(&vals, "decoration.blur.enabled", 1.0) > 0.5;
        let shadow = get(&vals, "decoration.shadow.enabled", 1.0) > 0.5;

        let inner = room.inset(gout);
        let (left, right) = inner.hsplit(0.55, gin * 2.0);
        let (top, bottom) = right.vsplit(0.5, gin * 2.0);
        let wins = [(left, true), (top, false), (bottom, false)];
        if shadow {
            for (r, _) in wins {
                let steps = 8;
                for i in (1..=steps).rev() {
                    let d = range * i as f64 / steps as f64;
                    rounded(cr, r.x - d, r.y - d + range / 3.0, r.w + 2.0 * d, r.h + 2.0 * d, radius + d);
                    set(cr, (0.0, 0.0, 0.0), 0.55 / steps as f64);
                    let _ = cr.fill();
                }
            }
        }
        for (r, focused) in wins {
            let alpha = if focused { on } else { off };
            // Blur frosts what shows through a see-through window.
            if blur && alpha < 1.0 {
                rounded(cr, r.x, r.y, r.w, r.h, radius);
                set(cr, mix(p.surface, p.text, 0.12), 0.5 * (1.0 - alpha) + 0.1);
                let _ = cr.fill();
            }
            sketch::win(
                cr,
                &p,
                r,
                Win {
                    focused,
                    alpha,
                    radius: radius.max(0.01),
                    border: if border_v > 0.0 { (border_v * STAGE_SCALE).max(1.0) } else { 0.01 },
                    lines: 4,
                    ..Win::default()
                },
            );
            if !focused && dim > 0.0 {
                rounded(cr, r.x, r.y, r.w, r.h, radius);
                set(cr, (0.0, 0.0, 0.0), dim);
                let _ = cr.fill();
            }
        }
        cr.restore().ok();

        let b = screen;
        tag(cr, section, &format!("Gap {}", fmt(gin_v)), left.x + left.w + gin, left.y + left.h * 0.45, b);
        tag(cr, section, &format!("Edge {}", fmt(gout_v)), left.x, left.y + left.h * 0.8, b);
        tag(cr, section, &format!("Border {}", fmt(border_v)), left.cx(), left.y + left.h, b);
        tag(cr, section, &format!("Corners {}", fmt(radius_v)), top.x + 40.0, top.y, b);
        if off < 1.0 {
            let s = if blur { "See-through, blurred" } else { "See-through" };
            tag(cr, section, s, top.x + top.w - 60.0, top.y + top.h - 6.0, b);
        }
        let s = if shadow { format!("Shadow {}", fmt(range_v)) } else { "No shadow".to_string() };
        tag(cr, section, &s, bottom.x + bottom.w - 40.0, bottom.y + bottom.h, b);
    });
    area
}

const TILING_KEYS: &[&str] =
    &["general.layout", "general.gaps_in", "general.gaps_out", "master.mfact", "master.new_status", "scrolling.column_width"];

/// Tiling: the layout, master width, column width, and where a new window goes.
pub fn tiling() -> gtk::Box {
    let area = gtk::DrawingArea::new();
    area.set_content_height(200);
    let vals = watch(&area, TILING_KEYS);
    area.set_draw_func(move |_, cr, w, h| {
        let p = sketch::pal();
        let (w, h) = (w as f64, h as f64);
        let room = desktop(cr, &p, w, h);
        let gin = (get(&vals, "general.gaps_in", 5.0) * SCALE).max(2.0);
        let gout = (get(&vals, "general.gaps_out", 10.0) * SCALE).max(2.0);
        let inner = room.inset(gout);
        let g = gin * 2.0;
        let layout =
            vals.borrow().get("general.layout").and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "dwindle".into());
        let plain = Win { lines: 3, ..Win::default() };
        let focused = Win { focused: true, ..plain };
        let ghost = Win { ghost: true, ..plain };
        match layout.as_str() {
            "master" => {
                let mfact = get(&vals, "master.mfact", 0.55).clamp(0.1, 0.9);
                let status = vals
                    .borrow()
                    .get("master.new_status")
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "slave".into());
                let (m, stack) = inner.hsplit(mfact, g);
                let (a, rest) = stack.vsplit(1.0 / 3.0, g);
                let (b, c) = rest.vsplit(0.5, g);
                match status.as_str() {
                    "master" => {
                        sketch::win(cr, &p, m, ghost);
                        sketch::win(cr, &p, a, focused);
                        sketch::win(cr, &p, b, plain);
                        sketch::win(cr, &p, c, plain);
                    }
                    _ => {
                        sketch::win(cr, &p, m, focused);
                        sketch::win(cr, &p, a, plain);
                        sketch::win(cr, &p, b, plain);
                        sketch::win(cr, &p, c, ghost);
                    }
                }
                caption_at(cr, &p, m, &format!("{:.0}%", mfact * 100.0));
            }
            "scrolling" => {
                let cw = get(&vals, "scrolling.column_width", 0.5).clamp(0.1, 1.0);
                let col = inner.w * cw;
                cr.save().ok();
                cr.rectangle(room.x, room.y, room.w, room.h);
                cr.clip();
                let x0 = inner.cx() - col / 2.0;
                for i in -2i32..=2 {
                    let x = x0 + i as f64 * (col + g);
                    let r = R::new(x, inner.y, col, inner.h);
                    sketch::win(
                        cr,
                        &p,
                        r,
                        if i == 0 {
                            focused
                        } else if i == 1 {
                            ghost
                        } else {
                            plain
                        },
                    );
                }
                cr.restore().ok();
                caption_at(cr, &p, R::new(x0, inner.y, col, inner.h), &format!("{:.0}%", cw * 100.0));
            }
            _ => {
                let (a, rest) = inner.hsplit(0.5, g);
                let (b, rest) = rest.vsplit(0.5, g);
                let (c, rest) = rest.hsplit(0.5, g);
                let (d, e) = rest.vsplit(0.5, g);
                sketch::win(cr, &p, a, plain);
                sketch::win(cr, &p, b, plain);
                sketch::win(cr, &p, c, plain);
                sketch::win(cr, &p, d, focused);
                sketch::win(cr, &p, e, ghost);
            }
        }
    });
    card(&area, "Preview: the dashed outline is where the next window opens.")
}

fn caption_at(cr: &gtk::cairo::Context, p: &sketch::Pal, r: R, text: &str) {
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
    cr.set_font_size(11.0);
    if let Ok(e) = cr.text_extents(text) {
        let (x, y) = (r.cx() - e.width() / 2.0, r.y + r.h - 10.0);
        rounded(cr, x - 5.0, y - e.height() - 3.0, e.width() + 10.0, e.height() + 7.0, 4.0);
        set(cr, p.bg, 0.8);
        let _ = cr.fill();
        set(cr, p.text, 1.0);
        cr.move_to(x - e.x_bearing(), y);
        let _ = cr.show_text(text);
    }
}

#[cfg(test)]
mod tests {
    use super::number;
    use serde_json::json;

    #[test]
    fn reads_numbers_from_hyprland_values() {
        assert_eq!(number(Some(&json!(5))), Some(5.0));
        assert_eq!(number(Some(&json!("5 10 5 10"))), Some(5.0));
        assert_eq!(number(Some(&json!(true))), Some(1.0));
        assert_eq!(number(Some(&json!({"custom": "3 3 3 3"}))), Some(3.0));
        assert_eq!(number(None), None);
    }
}
