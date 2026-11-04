//! Small drawings that show what a setting does: a tiling layout, where focus
//! goes, a bar at the top. They're drawn in the theme's colours on a little
//! desktop, so they read as "your screen" at a glance. Picture cards
//! ([`crate::widgets::visual_choice`]) and row thumbnails
//! ([`crate::widgets::sketched`]) use them.

use crate::fangraph::rgb;
use crate::theme;
use gtk::cairo::Context;
use gtk::prelude::*;
use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;

pub type Rgb = (f64, f64, f64);

/// What a drawing shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sketch {
    /// A tiling layout: "dwindle", "master" or "scrolling".
    Layout(&'static str),
    /// Where a new window goes in Master: "slave", "master" or "inherit".
    NewWindow(&'static str),
    /// `input.follow_mouse`: 0–3.
    Focus(u8),
    Swallow,
    BackAndForth,
    Scratchpad,
    SplitTowardCursor,
    PreserveSplit,
    FocusOnActivate,
    NoWarps,
    MiddlePaste,
    Dim,
    Blur,
    Shadow,
    Animations,
    ResizeOnBorder,
    /// The bar at the "top", "bottom", "left" or "right".
    Bar(&'static str),
    /// A lock screen's sign-in indicator: "circular", "cyberpunk", "minimal" or "edge".
    LockStyle(&'static str),
}

/// The theme's colours, as Cairo wants them.
#[derive(Clone, Copy)]
pub struct Pal {
    pub bg: Rgb,
    pub surface: Rgb,
    pub text: Rgb,
    pub dim: Rgb,
    pub accent: Rgb,
    pub danger: Rgb,
}

pub fn pal() -> Pal {
    let p = theme::current_palette();
    Pal {
        bg: rgb(&p.bg),
        surface: rgb(&p.surface),
        text: rgb(&p.text),
        dim: rgb(&p.dim_text),
        accent: rgb(&p.accent),
        danger: rgb(&p.danger),
    }
}

pub fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t)
}

pub fn set(cr: &Context, c: Rgb, a: f64) {
    cr.set_source_rgba(c.0, c.1, c.2, a);
}

pub fn rounded(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.max(0.0).min(h / 2.0).min(w / 2.0);
    if r <= 0.01 {
        cr.rectangle(x, y, w, h);
        return;
    }
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

#[derive(Clone, Copy)]
pub struct R {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl R {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> R {
        R { x, y, w, h }
    }
    pub fn inset(self, d: f64) -> R {
        R::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn cx(self) -> f64 {
        self.x + self.w / 2.0
    }
    pub fn cy(self) -> f64 {
        self.y + self.h / 2.0
    }
    /// Split across: the left part takes `t` of the width.
    pub fn hsplit(self, t: f64, gap: f64) -> (R, R) {
        let lw = (self.w - gap) * t;
        (R::new(self.x, self.y, lw, self.h), R::new(self.x + lw + gap, self.y, self.w - gap - lw, self.h))
    }
    pub fn vsplit(self, t: f64, gap: f64) -> (R, R) {
        let th = (self.h - gap) * t;
        (R::new(self.x, self.y, self.w, th), R::new(self.x, self.y + th + gap, self.w, self.h - gap - th))
    }
}

/// The screen: a rounded panel a little lighter than the window behind it.
pub fn screen(cr: &Context, p: &Pal, r: R) {
    rounded(cr, r.x, r.y, r.w, r.h, 5.0);
    set(cr, mix(p.bg, p.text, 0.06), 1.0);
    let _ = cr.fill_preserve();
    set(cr, p.dim, 0.25);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
}

/// How a drawn window looks.
#[derive(Clone, Copy, Default)]
pub struct Win {
    pub focused: bool,
    /// New, or where something will appear: a dashed accent outline.
    pub ghost: bool,
    pub alpha: f64,
    pub radius: f64,
    pub border: f64,
    /// 0 shows no title lines.
    pub lines: u8,
}

pub fn win(cr: &Context, p: &Pal, r: R, s: Win) {
    let alpha = if s.alpha > 0.0 { s.alpha } else { 1.0 };
    let radius = if s.radius > 0.0 { s.radius } else { 2.5 };
    if s.ghost {
        rounded(cr, r.x, r.y, r.w, r.h, radius);
        set(cr, p.accent, 0.12);
        let _ = cr.fill_preserve();
        cr.set_dash(&[3.0, 2.5], 0.0);
        cr.set_line_width(1.2);
        set(cr, p.accent, 0.9);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        return;
    }
    rounded(cr, r.x, r.y, r.w, r.h, radius);
    set(cr, mix(p.surface, p.text, if s.focused { 0.10 } else { 0.05 }), alpha);
    let _ = cr.fill_preserve();
    let border = if s.border > 0.0 { s.border } else { 1.0 };
    cr.set_line_width(border);
    if s.focused {
        set(cr, p.accent, alpha);
    } else {
        set(cr, p.dim, 0.45 * alpha);
    }
    let _ = cr.stroke();
    // A couple of text lines suggest content.
    let lines = if s.lines == 0 && r.w > 14.0 && r.h > 12.0 { 2 } else { s.lines };
    for i in 0..lines {
        let y = r.y + 5.0 + i as f64 * 4.0;
        if y > r.y + r.h - 3.0 {
            break;
        }
        let w = (r.w - 8.0) * if i == 0 { 0.6 } else { 0.85 };
        set(cr, p.dim, 0.5 * alpha);
        cr.rectangle(r.x + 4.0, y, w.max(0.0), 1.5);
        let _ = cr.fill();
    }
}

/// The mouse pointer, its tip at (x, y).
pub fn pointer(cr: &Context, p: &Pal, x: f64, y: f64) {
    cr.move_to(x, y);
    cr.line_to(x, y + 10.0);
    cr.line_to(x + 2.6, y + 7.6);
    cr.line_to(x + 4.6, y + 11.6);
    cr.line_to(x + 6.0, y + 10.9);
    cr.line_to(x + 4.0, y + 7.0);
    cr.line_to(x + 7.2, y + 7.0);
    cr.close_path();
    set(cr, p.text, 1.0);
    let _ = cr.fill_preserve();
    set(cr, p.bg, 1.0);
    cr.set_line_width(0.8);
    let _ = cr.stroke();
}

/// Rings around a click at (x, y).
fn click(cr: &Context, p: &Pal, x: f64, y: f64) {
    cr.set_line_width(1.0);
    for (r, a) in [(4.0, 0.9), (7.0, 0.45)] {
        set(cr, p.accent, a);
        cr.arc(x, y, r, 0.0, 2.0 * PI);
        let _ = cr.stroke();
    }
}

fn arrow(cr: &Context, c: Rgb, a: f64, (x1, y1): (f64, f64), (x2, y2): (f64, f64)) {
    set(cr, c, a);
    cr.set_line_width(1.4);
    cr.move_to(x1, y1);
    cr.line_to(x2, y2);
    let _ = cr.stroke();
    let ang = (y2 - y1).atan2(x2 - x1);
    let l = 4.0;
    cr.move_to(x2, y2);
    cr.line_to(x2 - l * (ang - 0.5).cos(), y2 - l * (ang - 0.5).sin());
    cr.line_to(x2 - l * (ang + 0.5).cos(), y2 - l * (ang + 0.5).sin());
    cr.close_path();
    let _ = cr.fill();
}

fn label(cr: &Context, c: Rgb, a: f64, text: &str, x: f64, y: f64, size: f64) {
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
    cr.set_font_size(size);
    if let Ok(ext) = cr.text_extents(text) {
        set(cr, c, a);
        cr.move_to(x - ext.width() / 2.0 - ext.x_bearing(), y - ext.height() / 2.0 - ext.y_bearing());
        let _ = cr.show_text(text);
    }
}

/// Draw `s` into a `w`×`h` box. `on` is the switch's state, for thumbnails.
pub fn draw(cr: &Context, s: Sketch, w: f64, h: f64, on: bool) {
    let p = pal();
    let full = R::new(0.5, 0.5, w - 1.0, h - 1.0);
    screen(cr, &p, full);
    let area = full.inset(4.0);
    let g = 3.0;
    let plain = Win::default();
    let focused = Win { focused: true, ..Win::default() };
    let ghost = Win { ghost: true, ..Win::default() };
    match s {
        Sketch::Layout(kind) => match kind {
            "master" => {
                let (m, stack) = area.hsplit(0.58, g);
                win(cr, &p, m, focused);
                let (a, rest) = stack.vsplit(1.0 / 3.0, g);
                let (b, c) = rest.vsplit(0.5, g);
                for r in [a, b, c] {
                    win(cr, &p, r, plain);
                }
            }
            "scrolling" => {
                cr.save().ok();
                rounded(cr, area.x, area.y, area.w, area.h, 2.0);
                cr.clip();
                let cw = area.w * 0.42;
                for i in 0..3 {
                    let x = area.x + 4.0 + i as f64 * (cw + g);
                    win(cr, &p, R::new(x, area.y, cw, area.h - 7.0), if i == 1 { focused } else { plain });
                }
                cr.restore().ok();
                // The strip scrolls sideways.
                set(cr, p.dim, 0.8);
                let y = area.y + area.h - 2.5;
                arrow(cr, p.dim, 0.8, (area.cx() - 4.0, y), (area.cx() - 12.0, y));
                arrow(cr, p.dim, 0.8, (area.cx() + 4.0, y), (area.cx() + 12.0, y));
            }
            _ => {
                let (a, rest) = area.hsplit(0.5, g);
                win(cr, &p, a, focused);
                let (b, rest) = rest.vsplit(0.5, g);
                win(cr, &p, b, plain);
                let (c, d) = rest.hsplit(0.5, g);
                win(cr, &p, c, plain);
                win(cr, &p, d, plain);
            }
        },
        Sketch::NewWindow(kind) => {
            let (m, stack) = area.hsplit(0.58, g);
            match kind {
                "master" => {
                    win(cr, &p, m, ghost);
                    let (a, b) = stack.vsplit(0.5, g);
                    win(cr, &p, a, plain);
                    win(cr, &p, b, plain);
                }
                "inherit" => {
                    win(cr, &p, m, plain);
                    let (a, rest) = stack.vsplit(1.0 / 3.0, g);
                    let (b, c) = rest.vsplit(0.5, g);
                    win(cr, &p, a, focused);
                    win(cr, &p, b, ghost);
                    win(cr, &p, c, plain);
                }
                _ => {
                    win(cr, &p, m, focused);
                    let (a, rest) = stack.vsplit(1.0 / 3.0, g);
                    let (b, c) = rest.vsplit(0.5, g);
                    win(cr, &p, a, plain);
                    win(cr, &p, b, plain);
                    win(cr, &p, c, ghost);
                }
            }
        }
        Sketch::Focus(mode) => {
            let (a, b) = area.hsplit(0.5, g);
            let (px, py) = (b.cx() + 2.0, b.cy() - 2.0);
            match mode {
                // Always: the window under the pointer has focus.
                1 => {
                    win(cr, &p, a, plain);
                    win(cr, &p, b, focused);
                }
                // Clicking moves focus; keyboard follows the click.
                0 => {
                    win(cr, &p, a, plain);
                    win(cr, &p, b, focused);
                    click(cr, &p, px, py);
                }
                // The pointer's window gets mouse focus (dashed); keys stay put.
                2 => {
                    win(cr, &p, a, focused);
                    win(cr, &p, b, plain);
                    rounded(cr, b.x - 1.5, b.y - 1.5, b.w + 3.0, b.h + 3.0, 3.5);
                    cr.set_dash(&[2.0, 2.0], 0.0);
                    set(cr, p.accent, 0.7);
                    cr.set_line_width(1.0);
                    let _ = cr.stroke();
                    cr.set_dash(&[], 0.0);
                }
                // Fully separate: keyboard focus never follows the mouse.
                _ => {
                    win(cr, &p, a, focused);
                    win(cr, &p, b, plain);
                    label(cr, p.accent, 0.9, "⌨", a.cx(), a.y + a.h - 6.0, 7.0);
                }
            }
            pointer(cr, &p, px, py);
        }
        Sketch::Swallow => {
            let (a, b) = area.hsplit(0.5, g);
            if on {
                win(cr, &p, a, Win { lines: 1, ..focused });
                label(cr, p.accent, 1.0, "▶", a.cx(), a.cy() + 2.0, 9.0);
                win(cr, &p, b, plain);
            } else {
                let (t, app) = (a, b);
                win(cr, &p, t, Win { lines: 1, ..plain });
                label(cr, p.dim, 1.0, ">_", t.cx(), t.cy() + 2.0, 7.0);
                win(cr, &p, app, focused);
                label(cr, p.accent, 1.0, "▶", app.cx(), app.cy() + 2.0, 9.0);
            }
        }
        Sketch::BackAndForth => {
            let y = area.cy();
            for (i, n) in ["1", "2"].iter().enumerate() {
                let x = area.x + area.w * (0.28 + 0.44 * i as f64);
                rounded(cr, x - 8.0, y - 7.0, 16.0, 14.0, 4.0);
                set(cr, if i == 1 { p.accent } else { mix(p.surface, p.text, 0.12) }, 1.0);
                let _ = cr.fill();
                label(cr, if i == 1 { p.bg } else { p.text }, 1.0, n, x, y, 8.0);
            }
            let (l, r) = (area.x + area.w * 0.28 + 9.0, area.x + area.w * 0.72 - 9.0);
            arrow(cr, p.dim, 0.9, (l, y - 3.0), (r, y - 3.0));
            if on {
                arrow(cr, p.accent, 1.0, (r, y + 3.0), (l, y + 3.0));
            }
        }
        Sketch::Scratchpad => {
            let (a, b) = area.hsplit(0.5, g);
            win(cr, &p, a, Win { alpha: 0.5, ..plain });
            win(cr, &p, b, Win { alpha: 0.5, ..plain });
            let f = R::new(area.x + area.w * 0.2, area.y + area.h * 0.18, area.w * 0.6, area.h * 0.64);
            if on {
                win(cr, &p, f, focused);
            } else {
                win(cr, &p, f, ghost);
            }
        }
        Sketch::SplitTowardCursor => {
            let (a, b) = area.hsplit(0.5, g);
            win(cr, &p, a, plain);
            let (top, bottom) = b.vsplit(0.5, g);
            if on {
                win(cr, &p, top, plain);
                win(cr, &p, bottom, ghost);
                pointer(cr, &p, bottom.cx(), bottom.cy() - 2.0);
            } else {
                win(cr, &p, top, plain);
                win(cr, &p, bottom, ghost);
                pointer(cr, &p, top.cx(), top.cy() - 4.0);
            }
        }
        Sketch::PreserveSplit => {
            let (top, bottom) = area.vsplit(0.5, g);
            win(cr, &p, top, focused);
            win(cr, &p, bottom, plain);
            if on {
                label(cr, p.accent, 1.0, "⇅", area.x + area.w - 7.0, area.cy(), 9.0);
            } else {
                label(cr, p.dim, 0.9, "⇄", area.x + area.w - 7.0, area.cy(), 9.0);
            }
        }
        Sketch::FocusOnActivate => {
            let (a, b) = area.hsplit(0.5, g);
            win(cr, &p, a, if on { plain } else { focused });
            win(cr, &p, b, if on { focused } else { plain });
            set(cr, p.danger, 1.0);
            cr.arc(b.x + b.w - 4.0, b.y + 4.0, 2.6, 0.0, 2.0 * PI);
            let _ = cr.fill();
        }
        Sketch::NoWarps => {
            let (a, b) = area.hsplit(0.5, g);
            win(cr, &p, a, plain);
            win(cr, &p, b, focused);
            if on {
                pointer(cr, &p, a.cx(), a.cy() - 2.0);
            } else {
                cr.set_dash(&[2.0, 2.0], 0.0);
                arrow(cr, p.dim, 0.8, (a.cx(), a.cy()), (b.cx() - 3.0, b.cy()));
                cr.set_dash(&[], 0.0);
                pointer(cr, &p, b.cx(), b.cy() - 2.0);
            }
        }
        Sketch::MiddlePaste => {
            let (cx, cy) = (area.cx(), area.cy());
            rounded(cr, cx - 9.0, cy - 13.0, 18.0, 26.0, 9.0);
            set(cr, mix(p.surface, p.text, 0.1), 1.0);
            let _ = cr.fill_preserve();
            set(cr, p.dim, 0.7);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            rounded(cr, cx - 1.6, cy - 10.0, 3.2, 7.0, 1.6);
            set(cr, if on { p.accent } else { p.dim }, 1.0);
            let _ = cr.fill();
        }
        Sketch::Dim => {
            let (a, b) = area.hsplit(0.5, g);
            win(cr, &p, a, focused);
            win(cr, &p, b, plain);
            if on {
                rounded(cr, b.x, b.y, b.w, b.h, 2.5);
                set(cr, (0.0, 0.0, 0.0), 0.45);
                let _ = cr.fill();
            }
        }
        Sketch::Blur => {
            for i in 0..6 {
                let x = area.x + i as f64 * area.w / 6.0;
                set(cr, if i % 2 == 0 { p.accent } else { mix(p.accent, p.text, 0.5) }, 0.6);
                cr.rectangle(x, area.y, area.w / 12.0, area.h);
                let _ = cr.fill();
            }
            let f = area.inset(area.h * 0.22);
            rounded(cr, f.x, f.y, f.w, f.h, 3.0);
            if on {
                set(cr, mix(p.surface, p.accent, 0.35), 0.92);
            } else {
                set(cr, p.surface, 0.55);
            }
            let _ = cr.fill_preserve();
            set(cr, p.dim, 0.6);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        }
        Sketch::Shadow => {
            let f = area.inset(area.h * 0.2);
            if on {
                for i in (1..6).rev() {
                    let d = i as f64 * 1.3;
                    rounded(cr, f.x - d + 2.0, f.y - d + 3.0, f.w + 2.0 * d, f.h + 2.0 * d, 3.0 + d);
                    set(cr, (0.0, 0.0, 0.0), 0.09);
                    let _ = cr.fill();
                }
            }
            win(cr, &p, f, focused);
        }
        Sketch::Animations => {
            let f = R::new(area.x + area.w * 0.45, area.y + area.h * 0.2, area.w * 0.45, area.h * 0.6);
            if on {
                for i in 1..4 {
                    let d = i as f64 * area.w * 0.1;
                    win(cr, &p, R::new(f.x - d, f.y, f.w, f.h), Win { alpha: 0.35 / i as f64, lines: 1, ..plain });
                }
            }
            win(cr, &p, f, focused);
        }
        Sketch::ResizeOnBorder => {
            let f = area.inset(area.h * 0.18);
            win(cr, &p, R::new(f.x, f.y, f.w * 0.7, f.h), focused);
            let x = f.x + f.w * 0.7;
            if on {
                arrow(cr, p.accent, 1.0, (x - 2.0, f.cy()), (x + 10.0, f.cy()));
            }
        }
        Sketch::Bar(where_) => {
            let t = 4.0;
            let (bar, body) = match where_ {
                "bottom" => (R::new(area.x, area.y + area.h - t, area.w, t), R::new(area.x, area.y, area.w, area.h - t - g)),
                "left" => (R::new(area.x, area.y, t, area.h), R::new(area.x + t + g, area.y, area.w - t - g, area.h)),
                "right" => (R::new(area.x + area.w - t, area.y, t, area.h), R::new(area.x, area.y, area.w - t - g, area.h)),
                _ => (R::new(area.x, area.y, area.w, t), R::new(area.x, area.y + t + g, area.w, area.h - t - g)),
            };
            rounded(cr, bar.x, bar.y, bar.w, bar.h, 1.5);
            set(cr, p.accent, 0.9);
            let _ = cr.fill();
            let (a, b) = body.hsplit(0.5, g);
            win(cr, &p, a, focused);
            win(cr, &p, b, plain);
        }
        Sketch::LockStyle(style) => {
            rounded(cr, area.x, area.y, area.w, area.h, 2.5);
            set(cr, mix(p.bg, (0.0, 0.0, 0.0), 0.4), 1.0);
            let _ = cr.fill();
            // The clock, and the card with the password under the indicator.
            set(cr, p.text, 0.85);
            rounded(cr, area.cx() - 14.0, area.y + 6.0, 28.0, 6.0, 2.0);
            let _ = cr.fill();
            let card = R::new(area.cx() - 26.0, area.y + 17.0, 52.0, area.h - 22.0);
            rounded(cr, card.x, card.y, card.w, card.h, 4.0);
            set(cr, p.surface, 0.95);
            let _ = cr.fill();
            set(cr, p.dim, 0.6);
            rounded(cr, card.x + 6.0, card.y + card.h - 10.0, card.w - 12.0, 5.0, 2.0);
            cr.set_line_width(0.8);
            let _ = cr.stroke();
            let (cx, cy) = (card.cx(), card.y + (card.h - 12.0) / 2.0);
            match style {
                "cyberpunk" => {
                    let r = 11.0;
                    cr.set_line_width(1.4);
                    set(cr, p.accent, 1.0);
                    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                        cr.move_to(cx + sx * r, cy + sy * (r - 4.0));
                        cr.line_to(cx + sx * r, cy + sy * r);
                        cr.line_to(cx + sx * (r - 4.0), cy + sy * r);
                    }
                    let _ = cr.stroke();
                    set(cr, p.accent, 0.8);
                    cr.rectangle(cx - r + 2.0, cy - 2.0, 2.0 * r - 4.0, 1.2);
                    let _ = cr.fill();
                }
                "minimal" => {
                    set(cr, p.accent, 0.9);
                    cr.arc(cx, cy, 4.0, 0.0, 2.0 * PI);
                    let _ = cr.fill();
                    set(cr, p.accent, 0.3);
                    cr.arc(cx, cy, 7.0, 0.0, 2.0 * PI);
                    cr.set_line_width(1.0);
                    let _ = cr.stroke();
                }
                "edge" => {
                    for i in 0..4 {
                        let d = i as f64 * 1.5;
                        rounded(cr, area.x + d, area.y + d, area.w - 2.0 * d, area.h - 2.0 * d, 2.5);
                        set(cr, p.accent, 0.7 - i as f64 * 0.17);
                        cr.set_line_width(1.5);
                        let _ = cr.stroke();
                    }
                }
                _ => {
                    set(cr, p.dim, 0.35);
                    cr.arc(cx, cy, 10.0, 0.0, 2.0 * PI);
                    cr.set_line_width(2.0);
                    let _ = cr.stroke();
                    set(cr, p.accent, 1.0);
                    cr.arc(cx, cy, 10.0, -PI / 2.0, PI / 2.0);
                    let _ = cr.stroke();
                }
            }
        }
    }
}

/// A drawing `w`×`h`; `on` is read on every draw (for a switch's state).
pub fn area(s: Sketch, w: i32, h: i32, on: Rc<Cell<bool>>) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(w);
    a.set_content_height(h);
    a.add_css_class("sketch");
    a.set_draw_func(move |_, cr, w, h| draw(cr, s, w as f64, h as f64, on.get()));
    a
}
