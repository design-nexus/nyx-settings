//! The devices an extension's preview can show, drawn in the theme's colours with the
//! device's lit parts in `accent`: laptop, keyboard, mouse, headset, monitor, gpu,
//! gamepad, tablet and memory. Each is drawn in its own design size and scaled to fit.

use crate::sketch::{Pal, Rgb, mix, rounded, set};
use gtk::cairo::Context;
use std::f64::consts::PI;

/// The design size of each picture: (width, height).
fn size(kind: &str) -> (f64, f64) {
    match kind {
        "laptop" => (260.0, 150.0),
        "keyboard" => (360.0, 110.0),
        "mouse" => (170.0, 230.0),
        "headset" => (220.0, 170.0),
        "monitor" => (260.0, 170.0),
        "gpu" => (300.0, 130.0),
        "gamepad" => (260.0, 160.0),
        "tablet" => (280.0, 160.0),
        "memory" => (300.0, 90.0),
        _ => (100.0, 100.0),
    }
}

struct Ink {
    body: Rgb,
    edge: Rgb,
    line: Rgb,
    key: Rgb,
    lit: Rgb,
}

fn fill_stroke(cr: &Context, fill: Rgb, fa: f64, edge: Rgb, ea: f64, width: f64) {
    set(cr, fill, fa);
    let _ = cr.fill_preserve();
    set(cr, edge, ea);
    cr.set_line_width(width);
    let _ = cr.stroke();
}

pub fn draw(cr: &Context, p: &Pal, accent: Rgb, kind: &str, w: f64, h: f64) {
    let (dw, dh) = size(kind);
    let scale = (w / dw).min(h / dh);
    let _ = cr.save();
    cr.translate((w - dw * scale) / 2.0, (h - dh * scale) / 2.0);
    cr.scale(scale, scale);
    let ink = Ink {
        body: mix(p.surface, p.text, 0.08),
        edge: mix(p.surface, p.text, 0.3),
        line: mix(p.surface, p.text, 0.35),
        key: mix(p.surface, p.text, 0.16),
        lit: accent,
    };
    match kind {
        "laptop" => laptop(cr, &ink),
        "keyboard" => keyboard(cr, &ink),
        "mouse" => mouse(cr, &ink),
        "headset" => headset(cr, &ink),
        "monitor" => monitor(cr, &ink),
        "gpu" => gpu(cr, &ink),
        "gamepad" => gamepad(cr, &ink),
        "tablet" => tablet(cr, &ink),
        "memory" => memory(cr, &ink),
        _ => {}
    }
    let _ = cr.restore();
}

fn laptop(cr: &Context, k: &Ink) {
    // The lid and its screen, lit faintly in the accent.
    rounded(cr, 36.0, 8.0, 188.0, 108.0, 8.0);
    fill_stroke(cr, k.body, 1.0, k.edge, 1.0, 1.5);
    rounded(cr, 46.0, 18.0, 168.0, 90.0, 3.0);
    set(cr, mix(k.body, k.lit, 0.25), 1.0);
    let _ = cr.fill();
    // The base.
    cr.move_to(14.0, 118.0);
    cr.line_to(246.0, 118.0);
    cr.line_to(236.0, 136.0);
    cr.curve_to(234.0, 139.0, 232.0, 140.0, 229.0, 140.0);
    cr.line_to(31.0, 140.0);
    cr.curve_to(28.0, 140.0, 26.0, 139.0, 24.0, 136.0);
    cr.close_path();
    fill_stroke(cr, k.body, 1.0, k.edge, 1.0, 1.5);
    for r in 0..4 {
        for i in 0..14 {
            rounded(cr, 42.0 + i as f64 * 13.0 + r as f64 * 2.0, 121.0 + r as f64 * 3.6, 10.0, 2.4, 0.6);
            set(cr, mix(k.key, k.lit, 0.55), 0.9);
            let _ = cr.fill();
        }
    }
}

fn keyboard(cr: &Context, k: &Ink) {
    // The glow under the keys.
    rounded(cr, 10.0, 10.0, 340.0, 92.0, 12.0);
    set(cr, k.lit, 0.12);
    let _ = cr.fill();
    rounded(cr, 2.0, 2.0, 356.0, 106.0, 10.0);
    fill_stroke(cr, k.body, 1.0, k.edge, 1.0, 1.5);
    for r in 0..5 {
        let n = if r == 4 { 9 } else { 15 };
        let mut x = 12.0;
        for i in 0..n {
            let wide = r == 4 && i == 4;
            let kw = if wide { 22.6 * 6.0 - 4.0 } else { 18.6 };
            rounded(cr, x, 12.0 + r as f64 * 18.5, kw, 14.5, 3.0);
            fill_stroke(cr, mix(k.key, k.lit, 0.18), 1.0, k.lit, 0.75, 1.0);
            x += if wide { 22.6 * 6.0 } else { 22.6 };
        }
    }
}

fn mouse(cr: &Context, k: &Ink) {
    cr.move_to(85.0, 6.0);
    cr.curve_to(44.0, 6.0, 18.0, 42.0, 18.0, 96.0);
    cr.line_to(18.0, 152.0);
    cr.curve_to(18.0, 196.0, 46.0, 226.0, 85.0, 226.0);
    cr.curve_to(124.0, 226.0, 152.0, 196.0, 152.0, 152.0);
    cr.line_to(152.0, 96.0);
    cr.curve_to(152.0, 42.0, 126.0, 6.0, 85.0, 6.0);
    cr.close_path();
    fill_stroke(cr, k.body, 1.0, k.edge, 1.0, 1.5);
    set(cr, k.line, 0.8);
    cr.set_line_width(1.5);
    cr.move_to(85.0, 8.0);
    cr.line_to(85.0, 92.0);
    cr.move_to(20.0, 92.0);
    cr.line_to(150.0, 92.0);
    let _ = cr.stroke();
    // The wheel, in the accent.
    rounded(cr, 76.0, 28.0, 18.0, 42.0, 9.0);
    fill_stroke(cr, mix(k.body, k.lit, 0.3), 1.0, k.lit, 1.0, 1.5);
    // Side buttons.
    for y in [122.0, 140.0] {
        rounded(cr, 2.0, y, 10.0, 13.0, 3.0);
        fill_stroke(cr, k.body, 1.0, k.edge, 1.0, 1.2);
    }
}

fn headset(cr: &Context, k: &Ink) {
    set(cr, k.edge, 0.7);
    cr.set_line_width(10.0);
    cr.arc(110.0, 82.0, 70.0, PI, 2.0 * PI);
    let _ = cr.stroke();
    for x in [22.0, 154.0] {
        rounded(cr, x, 92.0, 44.0, 64.0, 16.0);
        fill_stroke(cr, k.body, 1.0, k.edge, 1.0, 1.5);
        rounded(cr, x + 8.0, 104.0, 28.0, 40.0, 10.0);
        fill_stroke(cr, mix(k.body, k.lit, 0.3), 1.0, k.lit, 1.0, 1.5);
    }
    // The mic boom.
    set(cr, k.line, 0.9);
    cr.set_line_width(1.5);
    cr.move_to(30.0, 146.0);
    cr.curve_to(24.0, 156.0, 28.0, 164.0, 42.0, 164.0);
    cr.line_to(66.0, 164.0);
    let _ = cr.stroke();
    cr.arc(70.0, 164.0, 4.0, 0.0, 2.0 * PI);
    set(cr, k.lit, 1.0);
    let _ = cr.fill();
}

fn monitor(cr: &Context, k: &Ink) {
    rounded(cr, 16.0, 8.0, 228.0, 128.0, 8.0);
    fill_stroke(cr, k.body, 1.0, k.edge, 1.0, 1.5);
    rounded(cr, 26.0, 18.0, 208.0, 108.0, 4.0);
    set(cr, mix(k.body, k.lit, 0.35), 1.0);
    let _ = cr.fill();
    cr.move_to(112.0, 136.0);
    cr.line_to(148.0, 136.0);
    cr.line_to(154.0, 156.0);
    cr.line_to(106.0, 156.0);
    cr.close_path();
    fill_stroke(cr, k.body, 1.0, k.edge, 1.0, 1.5);
    rounded(cr, 84.0, 154.0, 92.0, 8.0, 4.0);
    fill_stroke(cr, k.body, 1.0, k.edge, 1.0, 1.5);
}

fn gpu(cr: &Context, k: &Ink) {
    rounded(cr, 8.0, 14.0, 284.0, 92.0, 10.0);
    fill_stroke(cr, k.body, 1.0, k.edge, 1.0, 1.5);
    for x in [84.0, 200.0] {
        cr.arc(x, 60.0, 34.0, 0.0, 2.0 * PI);
        fill_stroke(cr, mix(k.body, k.lit, 0.25), 1.0, k.lit, 1.0, 1.5);
        set(cr, k.lit, 0.6);
        cr.set_line_width(1.2);
        for i in 0..7 {
            let a = i as f64 * 2.0 * PI / 7.0;
            cr.move_to(x + 9.0 * a.cos(), 60.0 + 9.0 * a.sin());
            cr.line_to(x + 30.0 * (a + 0.5).cos(), 60.0 + 30.0 * (a + 0.5).sin());
        }
        let _ = cr.stroke();
        cr.arc(x, 60.0, 8.0, 0.0, 2.0 * PI);
        fill_stroke(cr, k.body, 1.0, k.edge, 1.0, 1.2);
    }
    rounded(cr, 30.0, 106.0, 150.0, 12.0, 2.0);
    fill_stroke(cr, mix(k.body, k.lit, 0.15), 1.0, k.edge, 1.0, 1.2);
}

fn gamepad(cr: &Context, k: &Ink) {
    cr.move_to(60.0, 30.0);
    cr.line_to(200.0, 30.0);
    cr.curve_to(228.0, 30.0, 244.0, 60.0, 250.0, 100.0);
    cr.curve_to(254.0, 130.0, 238.0, 148.0, 220.0, 140.0);
    cr.curve_to(206.0, 134.0, 196.0, 110.0, 180.0, 110.0);
    cr.line_to(80.0, 110.0);
    cr.curve_to(64.0, 110.0, 54.0, 134.0, 40.0, 140.0);
    cr.curve_to(22.0, 148.0, 6.0, 130.0, 10.0, 100.0);
    cr.curve_to(16.0, 60.0, 32.0, 30.0, 60.0, 30.0);
    cr.close_path();
    fill_stroke(cr, k.body, 1.0, k.edge, 1.0, 1.5);
    // The lightbar and touchpad.
    rounded(cr, 96.0, 26.0, 68.0, 8.0, 4.0);
    set(cr, k.lit, 0.9);
    let _ = cr.fill();
    rounded(cr, 98.0, 40.0, 64.0, 30.0, 6.0);
    fill_stroke(cr, mix(k.body, k.lit, 0.2), 1.0, k.lit, 0.8, 1.2);
    for x in [92.0, 168.0] {
        cr.arc(x, 96.0, 14.0, 0.0, 2.0 * PI);
        fill_stroke(cr, k.key, 1.0, k.edge, 1.0, 1.2);
    }
    set(cr, k.line, 0.9);
    cr.set_line_width(1.5);
    cr.move_to(52.0, 64.0);
    cr.line_to(52.0, 82.0);
    cr.move_to(43.0, 73.0);
    cr.line_to(61.0, 73.0);
    let _ = cr.stroke();
    for (x, y) in [(206.0, 60.0), (218.0, 72.0), (194.0, 72.0), (206.0, 84.0)] {
        cr.arc(x, y, 4.0, 0.0, 2.0 * PI);
        set(cr, k.line, 0.9);
        let _ = cr.stroke();
    }
}

fn tablet(cr: &Context, k: &Ink) {
    rounded(cr, 10.0, 14.0, 260.0, 136.0, 14.0);
    fill_stroke(cr, k.body, 1.0, k.edge, 1.0, 1.5);
    rounded(cr, 58.0, 30.0, 196.0, 104.0, 4.0);
    set(cr, mix(k.body, k.lit, 0.25), 1.0);
    let _ = cr.fill();
    cr.set_dash(&[4.0, 4.0], 0.0);
    rounded(cr, 80.0, 42.0, 152.0, 80.0, 2.0);
    set(cr, k.lit, 0.9);
    cr.set_line_width(1.5);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    for i in 0..4 {
        rounded(cr, 22.0, 36.0 + i as f64 * 24.0, 22.0, 16.0, 4.0);
        fill_stroke(cr, k.key, 1.0, k.edge, 1.0, 1.2);
    }
}

fn memory(cr: &Context, k: &Ink) {
    rounded(cr, 14.0, 6.0, 272.0, 12.0, 3.0);
    set(cr, k.lit, 0.9);
    let _ = cr.fill();
    rounded(cr, 10.0, 20.0, 280.0, 56.0, 4.0);
    fill_stroke(cr, k.body, 1.0, k.edge, 1.0, 1.5);
    for i in 0..8 {
        rounded(cr, 24.0 + i as f64 * 33.0, 34.0, 24.0, 26.0, 2.0);
        fill_stroke(cr, k.key, 1.0, k.edge, 1.0, 1.2);
    }
}
