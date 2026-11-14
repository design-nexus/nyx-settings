//! Displays: the desk, with the monitors drawn to scale as little desktops, clicked to
//! choose one and dragged into place; and Identify, which shows each monitor's name on
//! the monitor itself.

use crate::sketch::{self, R};
use crate::widgets;
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A monitor in layout coordinates (logical pixels: scaled, and rotated).
#[derive(Clone, Debug, PartialEq)]
pub struct Rect {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Its size in the layout: resolution over scale, sideways when turned 90° or 270°.
pub fn logical_size(width: f64, height: f64, scale: f64, transform: i64) -> (f64, f64) {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let (w, h) = (width / scale, height / scale);
    if transform % 2 == 1 { (h, w) } else { (w, h) }
}

/// Where a dropped monitor lands: against the nearest edge of another one, so
/// screens touch and don't overlap. Returns whole-pixel coordinates.
pub fn snap(moving: &Rect, others: &[Rect]) -> (i64, i64) {
    if others.is_empty() {
        return (0, 0);
    }
    let mut best: Option<(f64, f64, f64)> = None;
    for o in others {
        // Beside it (left or right), lined up top, bottom or as dropped; and above or below.
        let ys = [o.y, o.y + o.h - moving.h, moving.y.clamp(o.y - moving.h + 1.0, o.y + o.h - 1.0)];
        let xs = [o.x, o.x + o.w - moving.w, moving.x.clamp(o.x - moving.w + 1.0, o.x + o.w - 1.0)];
        let mut candidates: Vec<(f64, f64)> = Vec::new();
        for y in ys {
            candidates.push((o.x - moving.w, y));
            candidates.push((o.x + o.w, y));
        }
        for x in xs {
            candidates.push((x, o.y - moving.h));
            candidates.push((x, o.y + o.h));
        }
        for (x, y) in candidates {
            let overlaps = others.iter().any(|p| x < p.x + p.w && x + moving.w > p.x && y < p.y + p.h && y + moving.h > p.y);
            if overlaps {
                continue;
            }
            let d = (x - moving.x).powi(2) + (y - moving.y).powi(2);
            if best.is_none_or(|(_, _, bd)| d < bd) {
                best = Some((x, y, d));
            }
        }
    }
    let (x, y, _) = best.unwrap_or((moving.x, moving.y, 0.0));
    (x.round() as i64, y.round() as i64)
}

/// Shift a layout so its top-left corner is at 0,0 (Hyprland's convention).
pub fn normalise(rects: &mut [Rect]) {
    let min_x = rects.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
    let min_y = rects.iter().map(|r| r.y).fold(f64::INFINITY, f64::min);
    if min_x.is_finite() {
        for r in rects.iter_mut() {
            r.x -= min_x;
            r.y -= min_y;
        }
    }
}

/// How a screen is labelled under it on the desk.
#[derive(Clone, Debug, Default)]
pub struct Caption {
    pub title: String,
    pub detail: String,
}

/// The desk: the monitors drawn as little desktops, the chosen one outlined.
pub struct Desk {
    pub area: gtk::DrawingArea,
    selected: Rc<RefCell<String>>,
}

impl Desk {
    /// Outline the monitor called `name`.
    pub fn select(&self, name: &str) {
        *self.selected.borrow_mut() = name.to_string();
        self.area.queue_draw();
    }
}

/// Room under the screens for their captions and the hint.
const FOOT: f64 = 64.0;
const PAD: f64 = 22.0;

/// The desk. `on_select` gets a monitor's name when it's clicked; `on_moved` every
/// monitor's new position once one is dropped somewhere new.
pub fn canvas(
    rects: Vec<Rect>,
    captions: Vec<Caption>,
    selected: &str,
    on_select: impl Fn(String) + 'static,
    on_moved: impl Fn(Vec<(String, i64, i64)>) + 'static,
) -> Desk {
    let rects = Rc::new(RefCell::new(rects));
    let selected = Rc::new(RefCell::new(selected.to_string()));
    // Layout → screen: scale and offset, set while drawing.
    let view: Rc<Cell<(f64, f64, f64)>> = Rc::new(Cell::new((1.0, 0.0, 0.0)));
    let area = {
        let (rects, selected, view) = (rects.clone(), selected.clone(), view.clone());
        widgets::drawing(200, move |cr, p, accent, w, h| {
            draw(cr, p, accent, w, h, &rects.borrow(), &captions, &selected.borrow(), &view);
        })
    };
    area.add_css_class("dsp-desk");
    area.set_vexpand(true);

    // (index, offset from the monitor's corner to the pointer, in layout pixels, moved yet)
    let drag: Rc<RefCell<Option<(usize, f64, f64, bool)>>> = Rc::default();
    let gesture = gtk::GestureDrag::new();
    {
        let (rects, view, drag) = (rects.clone(), view.clone(), drag.clone());
        gesture.connect_drag_begin(move |_, px, py| {
            let (s, ox, oy) = view.get();
            let (lx, ly) = ((px - ox) / s, (py - oy) / s);
            let rs = rects.borrow();
            // The last drawn is on top.
            let hit = rs.iter().rposition(|r| lx >= r.x && lx <= r.x + r.w && ly >= r.y && ly <= r.y + r.h);
            *drag.borrow_mut() = hit.map(|i| (i, lx - rs[i].x, ly - rs[i].y, false));
        });
    }
    {
        let (rects, view, drag, area2) = (rects.clone(), view.clone(), drag.clone(), area.clone());
        gesture.connect_drag_update(move |g, dx, dy| {
            let Some((i, offx, offy, moved)) = *drag.borrow() else { return };
            if !moved && dx.hypot(dy) < 4.0 {
                return;
            }
            if rects.borrow().len() < 2 {
                return;
            }
            let Some((sx, sy)) = g.start_point() else { return };
            let (s, ox, oy) = view.get();
            let (lx, ly) = ((sx + dx - ox) / s, (sy + dy - oy) / s);
            if let Some(r) = rects.borrow_mut().get_mut(i) {
                r.x = lx - offx;
                r.y = ly - offy;
            }
            *drag.borrow_mut() = Some((i, offx, offy, true));
            area2.queue_draw();
        });
    }
    {
        let (rects, drag, area2, selected) = (rects.clone(), drag.clone(), area.clone(), selected.clone());
        gesture.connect_drag_end(move |_, _, _| {
            let Some((i, _, _, moved)) = drag.borrow_mut().take() else { return };
            let name = rects.borrow()[i].name.clone();
            if *selected.borrow() != name {
                *selected.borrow_mut() = name.clone();
                on_select(name);
            }
            if !moved {
                area2.queue_draw();
                return;
            }
            let mut rs = rects.borrow_mut();
            let others: Vec<Rect> = rs.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, r)| r.clone()).collect();
            let (x, y) = snap(&rs[i], &others);
            rs[i].x = x as f64;
            rs[i].y = y as f64;
            normalise(&mut rs);
            let positions = rs.iter().map(|r| (r.name.clone(), r.x.round() as i64, r.y.round() as i64)).collect();
            drop(rs);
            area2.queue_draw();
            on_moved(positions);
        });
    }
    area.add_controller(gesture);
    area.set_cursor_from_name(Some("grab"));
    Desk { area, selected }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    cr: &gtk::cairo::Context,
    p: &sketch::Pal,
    accent: sketch::Rgb,
    w: f64,
    h: f64,
    rs: &[Rect],
    captions: &[Caption],
    selected: &str,
    view: &Cell<(f64, f64, f64)>,
) {
    // The desk: a faint panel with a dot grid.
    sketch::rounded(cr, 0.5, 0.5, w - 1.0, h - 1.0, 12.0);
    sketch::set(cr, sketch::mix(p.bg, p.text, 0.025), 1.0);
    let _ = cr.fill_preserve();
    sketch::set(cr, p.dim, 0.18);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    sketch::set(cr, p.dim, 0.22);
    let step = 16.0;
    let mut y = step;
    while y < h - 4.0 {
        let mut x = step;
        while x < w - 4.0 {
            cr.rectangle(x, y, 1.2, 1.2);
            x += step;
        }
        y += step;
    }
    let _ = cr.fill();

    // The hint along the foot.
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
    cr.set_font_size(11.0);
    sketch::set(cr, p.dim, 0.9);
    let hint = if rs.len() > 1 { "Drag a screen to move it; the edges snap together." } else { "Plug in another screen to arrange them side by side." };
    cr.move_to(14.0, h - 12.0);
    let _ = cr.show_text(hint);

    if rs.is_empty() {
        return;
    }
    let min_x = rs.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
    let min_y = rs.iter().map(|r| r.y).fold(f64::INFINITY, f64::min);
    let max_x = rs.iter().map(|r| r.x + r.w).fold(f64::NEG_INFINITY, f64::max);
    let max_y = rs.iter().map(|r| r.y + r.h).fold(f64::NEG_INFINITY, f64::max);
    let (lw, lh) = ((max_x - min_x).max(1.0), (max_y - min_y).max(1.0));
    let (aw, ah) = ((w - 2.0 * PAD).max(1.0), (h - PAD - FOOT).max(1.0));
    // Leave room to drag one past the others; a screen on its own sits smaller.
    let fit = (aw / lw).min(ah / lh);
    let scale = if rs.len() > 1 { fit / 1.12 } else { fit * 0.72 };
    let ox = PAD + (aw - lw * scale) / 2.0 - min_x * scale;
    let oy = PAD + (ah - lh * scale) / 2.0 - min_y * scale;
    view.set((scale, ox, oy));

    let screen_rect = |r: &Rect| R::new(ox + r.x * scale + 2.0, oy + r.y * scale + 2.0, (r.w * scale - 4.0).max(6.0), (r.h * scale - 4.0).max(6.0));
    for r in rs {
        let sr = screen_rect(r);
        let chosen = r.name == selected;
        // The bezel, then the desktop on it.
        sketch::rounded(cr, sr.x - 2.0, sr.y - 2.0, sr.w + 4.0, sr.h + 4.0, 7.0);
        sketch::set(cr, sketch::mix(p.bg, (0.0, 0.0, 0.0), 0.35), 1.0);
        let _ = cr.fill();
        if sr.w > 40.0 && sr.h > 30.0 {
            sketch::desktop(cr, p, sr);
        } else {
            sketch::rounded(cr, sr.x, sr.y, sr.w, sr.h, 3.0);
            sketch::set(cr, sketch::mix(p.bg, p.accent, 0.3), 1.0);
            let _ = cr.fill();
        }
        sketch::rounded(cr, sr.x - 2.0, sr.y - 2.0, sr.w + 4.0, sr.h + 4.0, 7.0);
        if chosen {
            sketch::set(cr, accent, 1.0);
            cr.set_line_width(2.5);
        } else {
            sketch::set(cr, p.dim, 0.35);
            cr.set_line_width(1.0);
        }
        let _ = cr.stroke();
    }

    // Each one's name and mode under it: the chosen one's first, then the others that
    // don't run into a caption already there.
    let mut order: Vec<usize> = (0..rs.len()).collect();
    order.sort_by_key(|i| rs[*i].name != selected);
    let mut taken: Vec<(f64, f64, f64, f64)> = Vec::new();
    for i in order {
        let r = &rs[i];
        let sr = screen_rect(r);
        let c = captions.get(i).cloned().unwrap_or_else(|| Caption { title: r.name.clone(), detail: String::new() });
        let (x0, x1, y0, y1) = caption(cr, p, &c, sr.cx(), sr.y + sr.h + 17.0, r.name == selected, sr.w + 40.0, false);
        if taken.iter().any(|(a0, a1, b0, b1)| x0 < *a1 && x1 > *a0 && y0 < *b1 && y1 > *b0) {
            continue;
        }
        caption(cr, p, &c, sr.cx(), sr.y + sr.h + 17.0, r.name == selected, sr.w + 40.0, true);
        taken.push((x0 - 6.0, x1 + 6.0, y0, y1));
    }
}

/// A screen's name (bold) and detail (dim) on one line, centred under it; or the name
/// over the detail when the line is wider than `room`. Returns the box it takes, and
/// draws it if `paint`.
#[allow(clippy::too_many_arguments)]
fn caption(cr: &gtk::cairo::Context, p: &sketch::Pal, c: &Caption, cx: f64, y: f64, chosen: bool, room: f64, paint: bool) -> (f64, f64, f64, f64) {
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
    cr.set_font_size(12.0);
    let tw = cr.text_extents(&c.title).map(|e| e.x_advance()).unwrap_or(0.0);
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
    cr.set_font_size(11.0);
    let detail = format!("  {}", c.detail);
    let dw = if c.detail.is_empty() { 0.0 } else { cr.text_extents(&detail).map(|e| e.x_advance()).unwrap_or(0.0) };
    // The detail goes under the name when both don't fit beside each other.
    let stacked = tw + dw > room;
    let x = if stacked { cx - tw / 2.0 } else { cx - (tw + dw) / 2.0 };
    let width = if stacked { tw.max(dw) } else { tw + dw };
    let area = (cx - width / 2.0, cx + width / 2.0, y - 12.0, y + if stacked { 18.0 } else { 4.0 });
    if !paint {
        return area;
    }
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
    cr.set_font_size(12.0);
    sketch::set(cr, p.text, if chosen { 1.0 } else { 0.85 });
    cr.move_to(x, y);
    let _ = cr.show_text(&c.title);
    if dw > 0.0 {
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
        cr.set_font_size(11.0);
        sketch::set(cr, p.dim, 1.0);
        if stacked {
            let d = c.detail.as_str();
            let w = cr.text_extents(d).map(|e| e.x_advance()).unwrap_or(0.0);
            cr.move_to(cx - w / 2.0, y + 14.0);
            let _ = cr.show_text(d);
        } else {
            cr.move_to(x + tw, y);
            let _ = cr.show_text(&detail);
        }
    }
    area
}

/// Show each monitor's name, large, on that monitor for a moment.
pub fn identify() {
    let Some(display) = gdk::Display::default() else { return };
    let monitors = display.monitors();
    for i in 0..monitors.n_items() {
        let Some(mon) = monitors.item(i).and_downcast::<gdk::Monitor>() else { continue };
        let name = mon.connector().map(|c| c.to_string()).unwrap_or_default();
        let win = gtk::Window::builder().title("Settings: identify display").decorated(false).build();
        win.add_css_class("identify-window");
        let label = gtk::Label::new(Some(&name));
        label.add_css_class("identify-name");
        let model = mon.model().map(|m| m.to_string()).unwrap_or_default();
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 8);
        bx.set_halign(gtk::Align::Center);
        bx.set_valign(gtk::Align::Center);
        bx.append(&label);
        if !model.is_empty() {
            let m = gtk::Label::new(Some(&model));
            m.add_css_class("identify-model");
            bx.append(&m);
        }
        win.set_child(Some(&bx));
        win.fullscreen_on_monitor(&mon);
        win.present();
        // Any click closes it early.
        let click = gtk::GestureClick::new();
        let w2 = win.clone();
        click.connect_pressed(move |_, _, _, _| w2.close());
        win.add_controller(click);
        glib::timeout_add_local_once(std::time::Duration::from_millis(2500), move || win.close());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(name: &str, x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { name: name.into(), x, y, w, h }
    }

    #[test]
    fn sizes() {
        assert_eq!(logical_size(2560.0, 1600.0, 1.6, 0), (1600.0, 1000.0));
        assert_eq!(logical_size(1920.0, 1080.0, 1.0, 1), (1080.0, 1920.0));
    }

    #[test]
    fn snaps_to_the_nearest_edge() {
        let laptop = r("eDP-1", 0.0, 0.0, 1600.0, 1000.0);
        // Dropped a little right of the laptop and lower: lands touching its right edge.
        let ext = r("DP-1", 1650.0, 40.0, 1920.0, 1080.0);
        assert_eq!(snap(&ext, std::slice::from_ref(&laptop)), (1600, 40));
        // Dropped mostly above: sits on top.
        let ext = r("DP-1", 100.0, -1000.0, 1920.0, 1080.0);
        assert_eq!(snap(&ext, &[laptop]), (100, -1080));
    }

    #[test]
    fn never_overlaps() {
        let a = r("A", 0.0, 0.0, 1000.0, 1000.0);
        let b = r("B", 500.0, 500.0, 1000.0, 1000.0);
        let (x, y) = snap(&b, std::slice::from_ref(&a));
        assert!(x >= 1000 || y >= 1000 || x <= -1000 || y <= -1000);
    }

    #[test]
    fn layout_starts_at_zero() {
        let mut v = vec![r("A", 0.0, 0.0, 10.0, 10.0), r("B", -20.0, -5.0, 20.0, 10.0)];
        normalise(&mut v);
        assert_eq!((v[0].x, v[0].y, v[1].x, v[1].y), (20.0, 5.0, 0.0, 0.0));
    }
}
