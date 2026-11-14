//! A fan curve you edit by dragging its points: up and down for fan speed,
//! left and right for temperature. Temperatures are kept in whole °C and
//! labelled in the user's unit; speeds are percentages.

use crate::{theme, units};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const T_MAX: f64 = 110.0;
const PAD_L: f64 = 44.0;
const PAD_R: f64 = 16.0;
const PAD_T: f64 = 14.0;
const PAD_B: f64 = 30.0;
const HIT: f64 = 16.0;

/// (temperature °C, speed %)
pub type Points = Vec<(u32, u32)>;

pub struct FanGraph {
    pub area: gtk::DrawingArea,
    points: Rc<RefCell<Points>>,
}

impl FanGraph {
    /// Replace the curve being shown (no callback).
    pub fn set(&self, p: Points) {
        *self.points.borrow_mut() = p;
        self.area.queue_draw();
    }
}

pub(crate) fn rgb(hex: &str) -> (f64, f64, f64) {
    let c = gtk::gdk::RGBA::parse(hex).unwrap_or(gtk::gdk::RGBA::WHITE);
    (c.red() as f64, c.green() as f64, c.blue() as f64)
}

struct Geom {
    w: f64,
    h: f64,
}

impl Geom {
    fn x(&self, t: f64) -> f64 {
        PAD_L + t / T_MAX * (self.w - PAD_L - PAD_R)
    }
    fn y(&self, pct: f64) -> f64 {
        PAD_T + (1.0 - pct / 100.0) * (self.h - PAD_T - PAD_B)
    }
    fn t(&self, x: f64) -> f64 {
        (x - PAD_L) / (self.w - PAD_L - PAD_R) * T_MAX
    }
    fn pct(&self, y: f64) -> f64 {
        (1.0 - (y - PAD_T) / (self.h - PAD_T - PAD_B)) * 100.0
    }
}

/// Keep a moved point between its neighbours: temperatures strictly rising,
/// speeds never falling.
pub fn constrain(points: &Points, i: usize, t: f64, pct: f64) -> (u32, u32) {
    let lo_t = if i == 0 { 0 } else { points[i - 1].0 + 1 };
    let hi_t = if i + 1 < points.len() { points[i + 1].0.saturating_sub(1) } else { T_MAX as u32 };
    let t = (t.round().max(0.0) as u32).clamp(lo_t, hi_t.max(lo_t));
    let lo_p = if i == 0 { 0 } else { points[i - 1].1 };
    let hi_p = if i + 1 < points.len() { points[i + 1].1 } else { 100 };
    let p = (pct.round().clamp(0.0, 100.0) as u32).clamp(lo_p, hi_p.max(lo_p));
    (t, p)
}

pub fn fan_graph(points: Points, on_change: impl Fn(&Points) + 'static) -> FanGraph {
    let area = gtk::DrawingArea::new();
    area.add_css_class("fan-graph");
    area.set_content_height(230);
    area.set_hexpand(true);
    let points = Rc::new(RefCell::new(points));
    let hover: Rc<Cell<Option<usize>>> = Rc::default();
    let drag: Rc<Cell<Option<(usize, f64, f64)>>> = Rc::default();

    {
        let points = points.clone();
        let hover = hover.clone();
        let drag = drag.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let pal = theme::current_palette();
            let (ar, ag, ab) = rgb(&pal.accent);
            let (tr, tg, tb) = rgb(&pal.text);
            let (dr, dg, db) = rgb(&pal.dim_text);
            let (br, bg, bb) = rgb(&pal.border);
            let g = Geom { w: w as f64, h: h as f64 };
            let pts = points.borrow();

            // Grid and labels.
            cr.set_font_size(11.0);
            cr.set_line_width(1.0);
            for pct in [0.0, 25.0, 50.0, 75.0, 100.0] {
                let y = g.y(pct).round() + 0.5;
                cr.set_source_rgba(br, bg, bb, if pct == 0.0 { 0.9 } else { 0.45 });
                cr.move_to(PAD_L, y);
                cr.line_to(g.w - PAD_R, y);
                let _ = cr.stroke();
                cr.set_source_rgba(dr, dg, db, 1.0);
                let label = format!("{}%", pct as i64);
                let ext = cr.text_extents(&label).ok();
                let tw = ext.map(|e| e.width()).unwrap_or(20.0);
                cr.move_to(PAD_L - 8.0 - tw, y + 4.0);
                let _ = cr.show_text(&label);
            }
            let mut t = 0.0;
            while t <= T_MAX {
                let x = g.x(t).round() + 0.5;
                cr.set_source_rgba(br, bg, bb, 0.3);
                cr.move_to(x, PAD_T);
                cr.line_to(x, g.h - PAD_B);
                let _ = cr.stroke();
                cr.set_source_rgba(dr, dg, db, 1.0);
                let label = format!("{}{}", units::from_celsius(t).round() as i64, units::symbol());
                let tw = cr.text_extents(&label).map(|e| e.width()).unwrap_or(20.0);
                cr.move_to(x - tw / 2.0, g.h - PAD_B + 18.0);
                let _ = cr.show_text(&label);
                t += 20.0;
            }
            if pts.is_empty() {
                return;
            }

            // Area under the curve, then the curve.
            let xy: Vec<(f64, f64)> = pts.iter().map(|(t, p)| (g.x(*t as f64), g.y(*p as f64))).collect();
            cr.move_to(xy[0].0, g.y(0.0));
            for (x, y) in &xy {
                cr.line_to(*x, *y);
            }
            cr.line_to(g.x(T_MAX), xy.last().unwrap().1);
            cr.line_to(g.x(T_MAX), g.y(0.0));
            cr.close_path();
            cr.set_source_rgba(ar, ag, ab, 0.14);
            let _ = cr.fill();
            cr.set_source_rgba(ar, ag, ab, 1.0);
            cr.set_line_width(2.5);
            cr.move_to(xy[0].0, xy[0].1);
            for (x, y) in xy.iter().skip(1) {
                cr.line_to(*x, *y);
            }
            cr.line_to(g.x(T_MAX), xy.last().unwrap().1);
            let _ = cr.stroke();

            // Points.
            let active = drag.get().map(|d| d.0).or(hover.get());
            for (i, (x, y)) in xy.iter().enumerate() {
                let big = active == Some(i);
                let r = if big { 8.0 } else { 6.0 };
                cr.arc(*x, *y, r + 2.5, 0.0, std::f64::consts::TAU);
                cr.set_source_rgba(ar, ag, ab, if big { 0.35 } else { 0.18 });
                let _ = cr.fill();
                cr.arc(*x, *y, r, 0.0, std::f64::consts::TAU);
                cr.set_source_rgba(tr, tg, tb, 1.0);
                let _ = cr.fill_preserve();
                cr.set_source_rgba(ar, ag, ab, 1.0);
                cr.set_line_width(2.0);
                let _ = cr.stroke();
            }

            // Readout for the point under the pointer.
            if let Some(i) = active {
                let (t, p) = pts[i];
                let text = format!("{}{} · {}%", units::from_celsius(t as f64).round() as i64, units::symbol(), p);
                cr.set_font_size(12.0);
                let tw = cr.text_extents(&text).map(|e| e.width()).unwrap_or(60.0);
                let (x, y) = xy[i];
                let bx = (x - tw / 2.0 - 8.0).min(g.w - PAD_R - tw - 16.0).max(PAD_L);
                let by = if y - 34.0 < PAD_T { y + 14.0 } else { y - 34.0 };
                cr.set_source_rgba(tr, tg, tb, 0.95);
                cr.rectangle(bx, by, tw + 16.0, 22.0);
                let _ = cr.fill();
                let (bgr, bgg, bgb) = rgb(&pal.bg);
                cr.set_source_rgba(bgr, bgg, bgb, 1.0);
                cr.move_to(bx + 8.0, by + 15.0);
                let _ = cr.show_text(&text);
            }
        });
    }

    let nearest = {
        let points = points.clone();
        let area = area.clone();
        move |x: f64, y: f64| -> Option<usize> {
            let g = Geom { w: area.width() as f64, h: area.height() as f64 };
            points
                .borrow()
                .iter()
                .enumerate()
                .map(|(i, (t, p))| (i, (g.x(*t as f64) - x).hypot(g.y(*p as f64) - y)))
                .filter(|(_, d)| *d <= HIT)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        }
    };

    let motion = gtk::EventControllerMotion::new();
    {
        let hover = hover.clone();
        let area2 = area.clone();
        let nearest = nearest.clone();
        motion.connect_motion(move |_, x, y| {
            let n = nearest(x, y);
            if n != hover.get() {
                hover.set(n);
                area2.set_cursor_from_name(if n.is_some() { Some("grab") } else { None });
                area2.queue_draw();
            }
        });
    }
    {
        let hover = hover.clone();
        let area2 = area.clone();
        motion.connect_leave(move |_| {
            hover.set(None);
            area2.queue_draw();
        });
    }
    area.add_controller(motion);

    let on_change = Rc::new(on_change);
    let gesture = gtk::GestureDrag::new();
    {
        let drag = drag.clone();
        let area2 = area.clone();
        gesture.connect_drag_begin(move |_, x, y| {
            drag.set(nearest(x, y).map(|i| (i, x, y)));
            area2.set_cursor_from_name(Some("grabbing"));
            area2.queue_draw();
        });
    }
    {
        let drag = drag.clone();
        let points = points.clone();
        let area2 = area.clone();
        gesture.connect_drag_update(move |_, dx, dy| {
            let Some((i, sx, sy)) = drag.get() else { return };
            let g = Geom { w: area2.width() as f64, h: area2.height() as f64 };
            let snapshot = points.borrow().clone();
            let new = constrain(&snapshot, i, g.t(sx + dx), g.pct(sy + dy));
            points.borrow_mut()[i] = new;
            area2.queue_draw();
        });
    }
    {
        let drag = drag.clone();
        let points = points.clone();
        let area2 = area.clone();
        gesture.connect_drag_end(move |_, _, _| {
            if drag.take().is_some() {
                on_change(&points.borrow());
            }
            area2.set_cursor_from_name(None);
            area2.queue_draw();
        });
    }
    area.add_controller(gesture);

    // Redraw when the theme changes colour.
    {
        let area = area.downgrade();
        let last = RefCell::new(theme::current_palette());
        gtk::glib::timeout_add_seconds_local(1, move || {
            let Some(area) = area.upgrade() else { return gtk::glib::ControlFlow::Break };
            let now = theme::current_palette();
            if *last.borrow() != now {
                *last.borrow_mut() = now;
                area.queue_draw();
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    FanGraph { area, points }
}

/// Speeds (in %) for the presets, applied to the curve's existing temperatures.
pub fn preset(name: &str, n: usize) -> Vec<u32> {
    let shape: &[u32] = match name {
        "quiet" => &[0, 10, 15, 22, 30, 40, 52, 65],
        "balanced" => &[10, 18, 26, 35, 45, 56, 70, 82],
        _ => &[100, 100, 100, 100, 100, 100, 100, 100],
    };
    (0..n).map(|i| shape[(i * (shape.len() - 1)) / n.saturating_sub(1).max(1)].min(100)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve() -> Points {
        vec![(0, 10), (57, 43), (61, 61), (65, 84)]
    }

    #[test]
    fn temperatures_stay_between_neighbours() {
        let p = curve();
        assert_eq!(constrain(&p, 1, 90.0, 17.0).0, 60); // can't pass the next point (61)
        assert_eq!(constrain(&p, 1, -5.0, 17.0).0, 1); // can't pass the previous point (0)
        assert_eq!(constrain(&p, 3, 200.0, 50.0).0, 110); // capped at the top of the scale
    }

    #[test]
    fn speeds_never_fall() {
        let p = curve();
        assert_eq!(constrain(&p, 2, 61.0, 0.0).1, 43); // not below the previous point
        assert_eq!(constrain(&p, 2, 61.0, 100.0).1, 84); // not above the next point
    }

    #[test]
    fn presets_rise_and_fit() {
        for name in ["quiet", "balanced", "max"] {
            let v = preset(name, 8);
            assert_eq!(v.len(), 8);
            assert!(v.windows(2).all(|w| w[0] <= w[1]), "{name}: {v:?}");
            assert!(v.iter().all(|x| *x <= 100));
        }
        assert_eq!(preset("quiet", 8)[7], 65);
    }
}
