//! Input › Mouse: pointer speed, acceleration and scrolling, and mice with settings of
//! their own, beside a drawn mouse, the acceleration curve and a pad to try it on.

use super::keyboard::{fit, hold_controls, legend_item, slim_slider, tame_head, tiled};
use crate::backend::{devices, hypr, store};
use crate::sketch::{self, Pal};
use crate::widgets::{self, Page, hypr_slider, hypr_switch, opts};
use gtk::cairo::Context;
use gtk::glib;
use gtk::prelude::*;
use nyx_devices::{Category, DeviceId};
use serde_json::json;
use std::cell::RefCell;
use std::f64::consts::PI;
use std::rc::Rc;

/// What the picture shows: every mouse, or one with its own settings.
#[derive(Clone, Default)]
struct Shown {
    device: Option<String>,
}

/// The settings the picture draws, for every mouse or for one.
struct Feel {
    flat: bool,
    left_handed: bool,
    natural: bool,
    speed: f64,
}

fn global_bool(key: &str) -> bool {
    store::option(key).and_then(|v| v.as_bool()).or_else(|| hypr::get_bool(key)).unwrap_or(false)
}

fn global_accel() -> String {
    store::option("input.accel_profile").and_then(|v| v.as_str().map(String::from)).or_else(|| hypr::get_str("input.accel_profile")).unwrap_or_default()
}

fn feel(shown: &Shown) -> Feel {
    let mut f = Feel {
        flat: global_accel() == "flat",
        left_handed: global_bool("input.left_handed"),
        natural: global_bool("input.natural_scroll"),
        speed: store::option("input.sensitivity").and_then(|v| v.as_f64()).or_else(|| hypr::get_f64("input.sensitivity")).unwrap_or(0.0),
    };
    if let Some(name) = &shown.device {
        let fields = store::read(|s| s.devices.get(name).cloned().unwrap_or_default());
        if let Some(a) = fields.get("accel_profile").and_then(|v| v.as_str()) {
            f.flat = a == "flat";
        }
        if let Some(n) = fields.get("natural_scroll").and_then(|v| v.as_bool()) {
            f.natural = n;
        }
        if let Some(s) = fields.get("sensitivity").and_then(|v| v.as_f64()) {
            f.speed = s;
        }
    }
    f
}

/// Every pointing device Hyprland lists, trackpads too, by its name for them.
fn pointers() -> Vec<String> {
    hypr::json(&["devices"])
        .and_then(|d| d.get("mice").cloned())
        .and_then(|m| m.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect()
}

/// A mouse, seen from above, with what its buttons do labelled beside it.
fn draw_mouse(cr: &Context, p: &Pal, w: f64, h: f64, f: &Feel) {
    // Labels over the top corners and under it; the mouse in between.
    let (top, bottom) = (40.0, 40.0);
    let s = ((h - top - bottom) / 240.0).min((w - 20.0) / 160.0).max(0.1);
    let (ox, oy) = (w / 2.0 - 80.0 * s, top + (h - top - bottom - 240.0 * s) / 2.0);
    let pt = |x: f64, y: f64| (ox + x * s, oy + y * s);
    let body = |cr: &Context| {
        let (x, y) = pt(80.0, 6.0);
        cr.move_to(x, y);
        let c = |cr: &Context, a: (f64, f64), b: (f64, f64), e: (f64, f64)| {
            let (a, b, e) = (pt(a.0, a.1), pt(b.0, b.1), pt(e.0, e.1));
            cr.curve_to(a.0, a.1, b.0, b.1, e.0, e.1);
        };
        c(cr, (38.0, 6.0), (14.0, 40.0), (14.0, 92.0));
        let (x, y) = pt(14.0, 154.0);
        cr.line_to(x, y);
        c(cr, (14.0, 200.0), (42.0, 234.0), (80.0, 234.0));
        c(cr, (118.0, 234.0), (146.0, 200.0), (146.0, 154.0));
        let (x, y) = pt(146.0, 92.0);
        cr.line_to(x, y);
        c(cr, (146.0, 40.0), (122.0, 6.0), (80.0, 6.0));
        cr.close_path();
    };
    body(cr);
    sketch::set(cr, sketch::mix(p.bg, p.text, 0.12), 1.0);
    let _ = cr.fill_preserve();
    sketch::set(cr, sketch::mix(p.bg, p.text, 0.3), 1.0);
    cr.set_line_width(1.2);
    let _ = cr.stroke();
    // The two buttons, the main one tinted.
    for left in [true, false] {
        cr.save().ok();
        body(cr);
        cr.clip();
        let (x0, y0) = pt(if left { 0.0 } else { 80.0 }, 0.0);
        cr.rectangle(x0, y0, 80.0 * s, 100.0 * s);
        let main = left != f.left_handed;
        sketch::set(cr, if main { p.accent } else { sketch::mix(p.bg, p.text, 0.18) }, if main { 0.45 } else { 1.0 });
        let _ = cr.fill();
        cr.restore().ok();
    }
    sketch::set(cr, sketch::mix(p.bg, p.text, 0.3), 1.0);
    cr.set_line_width(1.2);
    let (a, b) = (pt(80.0, 8.0), pt(80.0, 100.0));
    cr.move_to(a.0, a.1);
    cr.line_to(b.0, b.1);
    let (a, b) = (pt(16.0, 100.0), pt(144.0, 100.0));
    cr.move_to(a.0, a.1);
    cr.line_to(b.0, b.1);
    let _ = cr.stroke();
    // The wheel, and which way it turns the page.
    let (wx, wy) = pt(71.0, 34.0);
    sketch::rounded(cr, wx, wy, 18.0 * s, 40.0 * s, 9.0 * s);
    sketch::set(cr, sketch::mix(p.bg, p.text, 0.25), 1.0);
    let _ = cr.fill_preserve();
    sketch::set(cr, p.accent, 1.0);
    let _ = cr.stroke();
    cr.set_line_width(2.0);
    for (from, to) in [((80.0, 28.0), (80.0, 14.0)), ((80.0, 80.0), (80.0, 94.0))] {
        let (a, b) = (pt(from.0, from.1), pt(to.0, to.1));
        cr.move_to(a.0, a.1);
        cr.line_to(b.0, b.1);
        let dir = if b.1 < a.1 { -1.0 } else { 1.0 };
        cr.move_to(b.0 - 5.0 * s, b.1 - dir * 5.0 * s);
        cr.line_to(b.0, b.1);
        cr.line_to(b.0 + 5.0 * s, b.1 - dir * 5.0 * s);
    }
    let _ = cr.stroke();
    // Labels: what the left and right buttons and the wheel do.
    let (main, menu) = if f.left_handed { ("Menu", "Click") } else { ("Click", "Menu") };
    caption(cr, p, main, "Left button", 4.0, 12.0, true, main == "Click");
    caption(cr, p, menu, "Right button", w - 4.0, 12.0, false, menu == "Click");
    let page = if f.natural { "The page moves down" } else { "The page moves up" };
    centred(cr, p, "Wheel down", w / 2.0, h - 20.0, true);
    centred(cr, p, page, w / 2.0, h - 5.0, false);
}

fn caption(cr: &Context, p: &Pal, title: &str, sub: &str, x: f64, y: f64, from_left: bool, hot: bool) {
    for (i, (text, bold)) in [(title, true), (sub, false)].into_iter().enumerate() {
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, if bold { gtk::cairo::FontWeight::Bold } else { gtk::cairo::FontWeight::Normal });
        cr.set_font_size(if bold { 11.5 } else { 10.0 });
        let Ok(ext) = cr.text_extents(text) else { continue };
        sketch::set(cr, if bold && hot { p.accent } else if bold { p.text } else { p.dim }, 1.0);
        cr.move_to(if from_left { x } else { x - ext.x_advance() }, y + i as f64 * 14.0);
        let _ = cr.show_text(text);
    }
}

fn centred(cr: &Context, p: &Pal, text: &str, x: f64, y: f64, bold: bool) {
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, if bold { gtk::cairo::FontWeight::Bold } else { gtk::cairo::FontWeight::Normal });
    cr.set_font_size(if bold { 11.5 } else { 10.0 });
    let Ok(ext) = cr.text_extents(text) else { return };
    sketch::set(cr, if bold { p.text } else { p.dim }, 1.0);
    cr.move_to(x - ext.x_advance() / 2.0, y);
    let _ = cr.show_text(text);
}

/// How far the pointer goes for how fast the hand moves: adaptive curves up, flat is a line.
fn draw_accel(cr: &Context, p: &Pal, w: f64, h: f64, f: &Feel) {
    let (top, bottom) = (6.0, h - 22.0);
    sketch::set(cr, p.dim, 0.18);
    cr.set_line_width(1.0);
    for i in 0..4 {
        let y = top + (bottom - top) * i as f64 / 3.0;
        cr.move_to(0.0, y.round() + 0.5);
        cr.line_to(w, y.round() + 0.5);
    }
    let _ = cr.stroke();
    // Speed lifts both lines a little.
    let lift = f.speed * 0.25;
    let y = |t: f64| bottom - (bottom - top) * (t + lift).clamp(0.0, 1.0);
    let flat = |cr: &Context| {
        cr.move_to(0.0, y(0.0));
        cr.line_to(w, y(0.82));
    };
    let adaptive = |cr: &Context| {
        cr.move_to(0.0, y(0.0));
        for i in 1..=40 {
            let t = i as f64 / 40.0;
            cr.line_to(w * t, y(t.powf(1.9) * 0.95));
        }
    };
    for (draw, on) in [(&flat as &dyn Fn(&Context), f.flat), (&adaptive, !f.flat)] {
        draw(cr);
        if on {
            sketch::set(cr, p.accent, 1.0);
            cr.set_line_width(2.4);
            cr.set_dash(&[], 0.0);
        } else {
            sketch::set(cr, p.dim, 0.7);
            cr.set_line_width(1.4);
            cr.set_dash(&[5.0, 4.0], 0.0);
        }
        let _ = cr.stroke();
    }
    cr.set_dash(&[], 0.0);
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
    cr.set_font_size(10.0);
    sketch::set(cr, p.dim, 1.0);
    for (text, at) in [("Slow hand", 0.0), ("Hand speed", 0.5), ("Fast hand", 1.0)] {
        let Ok(ext) = cr.text_extents(text) else { continue };
        cr.move_to(((w - ext.x_advance()) * at).max(0.0), h - 4.0);
        let _ = cr.show_text(text);
    }
}

/// The trail the pointer leaves on the pad: where it was over the last moment.
fn draw_trail(cr: &Context, p: &Pal, w: f64, h: f64, pts: &[(f64, f64, i64)]) {
    if pts.is_empty() {
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
        cr.set_font_size(12.0);
        let text = "Move the pointer here";
        if let Ok(ext) = cr.text_extents(text) {
            sketch::set(cr, p.dim, 0.8);
            cr.move_to((w - ext.x_advance()) / 2.0, h / 2.0);
            let _ = cr.show_text(text);
        }
        return;
    }
    let now = glib::monotonic_time();
    cr.set_line_width(2.0);
    cr.set_dash(&[2.0, 4.0], 0.0);
    for pair in pts.windows(2) {
        let age = ((now - pair[1].2) as f64 / 1_500_000.0).clamp(0.0, 1.0);
        sketch::set(cr, p.accent, 0.9 * (1.0 - age));
        cr.move_to(pair[0].0, pair[0].1);
        cr.line_to(pair[1].0, pair[1].1);
        let _ = cr.stroke();
    }
    cr.set_dash(&[], 0.0);
    for (i, (x, y, t)) in pts.iter().enumerate() {
        if i % 8 != 0 && i != pts.len() - 1 {
            continue;
        }
        let age = ((now - t) as f64 / 1_500_000.0).clamp(0.0, 1.0);
        sketch::set(cr, p.accent, 1.0 - age);
        cr.arc(*x, *y, 3.5, 0.0, 2.0 * PI);
        let _ = cr.fill();
    }
}

pub fn build(page: &Page) {
    let all = pointers();
    let mice: Vec<String> = all.iter().filter(|n| !n.contains("touchpad")).cloned().collect();
    let remembered = devices::remembered_mice(&mice);
    let shown = Rc::new(RefCell::new(Shown::default()));

    // ----- The picture -----
    let pv = page.split();
    let own: Vec<String> = mice
        .iter()
        .chain(remembered.iter().map(|(n, _)| n))
        .filter(|n| store::read(|s| s.devices.get(*n).is_some_and(|f| !f.is_empty())))
        .cloned()
        .collect();
    let chooser = if own.is_empty() {
        None
    } else {
        let mut choices = vec![(String::new(), "Every mouse".to_string())];
        choices.extend(own.iter().map(|n| (n.clone(), short_device(n))));
        Some(choices)
    };
    let slot = widgets::hbox(0);
    let head = pv.head("Every mouse", "How the pointer and the wheel behave, unless a mouse has its own settings.", &[fit(&slot).upcast_ref()]);
    if chooser.is_some() {
        tame_head(&head, 26);
    }
    let text = head.first_child().and_downcast::<gtk::Box>();
    let title = text.as_ref().and_then(|t| t.first_child()).and_downcast::<gtk::Label>();
    let line = text.as_ref().and_then(|t| t.last_child()).and_downcast::<gtk::Label>();

    let pair = widgets::hbox(12);
    pair.set_homogeneous(true);
    pv.add(&pair);
    let mouse_card = widgets::vbox(0);
    mouse_card.add_css_class("preview-card");
    mouse_card.add_css_class("ms-card");
    let mouse_pic = {
        let shown = shown.clone();
        widgets::drawing(250, move |cr, p, _, w, h| draw_mouse(cr, p, w, h, &feel(&shown.borrow())))
    };
    mouse_card.append(&mouse_pic);
    pair.append(&mouse_card);

    let chart_card = widgets::vbox(4);
    chart_card.add_css_class("preview-card");
    chart_card.add_css_class("ms-card");
    let chart_title = widgets::label("", "ms-chart-title");
    chart_title.set_wrap(true);
    chart_card.append(&chart_title);
    let chart_line = widgets::label("", "ms-chart-line");
    chart_line.set_wrap(true);
    chart_card.append(&chart_line);
    let chart = {
        let shown = shown.clone();
        widgets::drawing(150, move |cr, p, _, w, h| draw_accel(cr, p, w, h, &feel(&shown.borrow())))
    };
    chart.set_vexpand(true);
    chart_card.append(&chart);
    let key = widgets::vbox(4);
    key.append(&legend_item("Adaptive", "ms-dot-adaptive"));
    key.append(&legend_item("Flat: the same distance at any speed, best for games", "ms-dot-flat"));
    chart_card.append(&key);
    pair.append(&chart_card);

    let pad_card = pv.card();
    pad_card.add_css_class("ms-pad");
    let ph = widgets::hbox(8);
    ph.append(&widgets::label("Try it", "ms-pad-title"));
    let hint = widgets::label("Move across the pad: the trail shows how far the pointer goes.", "ms-pad-hint");
    hint.set_wrap(true);
    hint.set_hexpand(true);
    ph.append(&hint);
    pad_card.append(&ph);
    let trail: Rc<RefCell<Vec<(f64, f64, i64)>>> = Rc::default();
    let pad = {
        let trail = trail.clone();
        widgets::drawing(120, move |cr, p, _, w, h| draw_trail(cr, p, w, h, &trail.borrow()))
    };
    pad.set_vexpand(true);
    pad_card.append(&pad);
    {
        let motion = gtk::EventControllerMotion::new();
        let (moved, pad2) = (trail.clone(), pad.clone());
        motion.connect_motion(move |_, x, y| {
            let mut t = moved.borrow_mut();
            let now = glib::monotonic_time();
            t.retain(|(_, _, at)| now - at < 1_500_000);
            t.push((x, y, now));
            pad2.queue_draw();
        });
        pad.add_controller(motion);
        // Let the trail fade once the pointer stops.
        let pad2 = pad.clone();
        pad.add_tick_callback(move |_, _| {
            let now = glib::monotonic_time();
            let mut t = trail.borrow_mut();
            let before = t.len();
            t.retain(|(_, _, at)| now - at < 1_500_000);
            if before > 0 {
                pad2.queue_draw();
            }
            glib::ControlFlow::Continue
        });
    }

    let redraw: Rc<dyn Fn()> = {
        let (shown, mouse_pic, chart, chart_title, chart_line, title, line) =
            (shown.clone(), mouse_pic.clone(), chart.clone(), chart_title.clone(), chart_line.clone(), title.clone(), line.clone());
        Rc::new(move || {
            let s = shown.borrow();
            let f = feel(&s);
            chart_title.set_text(if f.flat { "Flat acceleration" } else { "Adaptive acceleration" });
            chart_line.set_text(if f.flat {
                "The pointer goes the same distance however fast you move."
            } else {
                "Quick flicks go further; slow moves stay precise."
            });
            if let (Some(t), Some(l)) = (&title, &line) {
                match &s.device {
                    None => {
                        t.set_text("Every mouse");
                        l.set_text("How the pointer and the wheel behave, unless a mouse has its own settings.");
                    }
                    Some(n) => {
                        t.set_text(&short_device(n));
                        l.set_text("Its own settings, used over the ones for every mouse.");
                    }
                }
            }
            mouse_pic.queue_draw();
            chart.queue_draw();
        })
    };
    if let Some(choices) = chooser {
        let (shown, redraw) = (shown.clone(), redraw.clone());
        slot.append(&widgets::segmented(&choices, "", move |id| {
            shown.borrow_mut().device = (!id.is_empty()).then_some(id);
            redraw();
        }));
    }
    redraw();
    {
        let redraw = redraw.clone();
        store::subscribe(&mouse_pic, "input.", move |_, _| redraw());
    }

    // ----- Pointer -----
    let g = page.group("Pointer");
    g.add(&tiled(
        slim_slider(hypr_slider("input.sensitivity", "Speed", "Mice and trackpads. 0 is the default.", (-1.0, 1.0, 0.05), 2, "", false), 110),
        "input-mouse-symbolic",
    ));
    widgets::keywords("pointer speed sensitivity");
    let accel = global_accel();
    let accel_id = if accel == "flat" { "flat" } else if accel.is_empty() || accel == "adaptive" { "adaptive" } else { accel.as_str() };
    let seg = widgets::segmented(&opts(&[("adaptive", "Adaptive"), ("flat", "Flat")]), accel_id, |id| {
        store::set_option("input.accel_profile", json!(id));
    });
    let r = widgets::row("Acceleration", "", Some(seg.upcast_ref()));
    g.add(&tiled(r, "power-profile-performance-symbolic"));
    widgets::keywords("acceleration accel flat adaptive gaming");
    g.add(&tiled(hypr_switch("input.left_handed", "Left-handed", "Swap the left and right buttons."), "object-flip-horizontal-symbolic"));

    // ----- Scrolling -----
    let g = page.group("Scrolling");
    g.add(&tiled(hypr_switch("input.natural_scroll", "Natural scrolling", "Content moves with the wheel."), "view-continuous-symbolic"));
    g.add(&tiled(slim_slider(hypr_slider("input.scroll_factor", "Scroll speed", "", (0.1, 3.0, 0.05), 2, "×", false), 120), "input-mouse-symbolic"));
    widgets::keywords("scroll wheel natural reverse speed mouse");

    // ----- Mice -----
    let g = page.group("Mice");
    g.note("Give one mouse settings of its own. They're used over the ones above.");
    if mice.is_empty() {
        g.add(&widgets::row("No mice found", "Connect a mouse to give it its own settings.", None));
    }
    for name in &mice {
        let has_own = store::read(|s| s.devices.get(name).is_some_and(|f| !f.is_empty()));
        let desc = if is_trackpad_buttons(name, &all) {
            "The trackpad's buttons".to_string()
        } else if has_own {
            "Its own speed and acceleration".to_string()
        } else {
            "Same as every mouse".to_string()
        };
        let n = name.clone();
        let redraw = redraw.clone();
        let (r, _) = widgets::nav_row(Some("input-mouse-symbolic"), &short_device(name), &desc, if has_own { "Own" } else { "" }, move || {
            open_device(&n, None, redraw.clone())
        });
        r.set_tooltip_text(Some(name));
        g.add(&r);
    }
    widgets::keywords("per device mouse own settings override");
    for (name, profile) in remembered {
        let n = name.clone();
        let seen = devices::last_seen(profile.as_ref());
        let redraw = redraw.clone();
        let (r, _) = widgets::nav_row(Some("document-open-recent-symbolic"), &short_device(&name), &format!("Not connected · {seen}"), "", move || {
            open_device(&n, Some(()), redraw.clone())
        });
        r.set_tooltip_text(Some(&name));
        g.add(&r);
        widgets::keywords("remembered disconnected mouse forget");
    }
    hold_controls(page, 24);
}

/// A mouse Hyprland lists that's really the buttons of a trackpad (`…-mouse` beside `…-touchpad`).
fn is_trackpad_buttons(name: &str, pointers: &[String]) -> bool {
    name.strip_suffix("-mouse").is_some_and(|base| pointers.iter().any(|n| *n == format!("{base}-touchpad")))
}

/// A mouse's name, a little tidier: "logitech-mx-master-3" becomes "Logitech mx master 3".
fn short_device(name: &str) -> String {
    let t = name.replace(['-', '_'], " ");
    let mut c = t.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// One mouse's own settings, a level down. `remembered`: it isn't connected, so it can be forgotten.
fn open_device(name: &str, remembered: Option<()>, redraw: Rc<dyn Fn()>) {
    let name = name.to_string();
    let title = short_device(&name);
    let win: Rc<RefCell<Option<gtk::Window>>> = Rc::default();
    let w2 = win.clone();
    let made = widgets::subpage(&title, 460, move |p| {
        let g = p.group("");
        g.note("Changed here, used whenever it's connected. Anything left as it is follows the settings for every mouse.");
        let fields = store::read(|s| s.devices.get(&name).cloned().unwrap_or_default());

        let sens = fields.get("sensitivity").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let n = name.clone();
        let (r, s) = widgets::slider_row("Speed", "", (-1.0, 1.0, 0.05), sens, 2, "", move |v| set(&n, "sensitivity", json!((v * 100.0).round() / 100.0)));
        s.scale.set_width_request(140);
        g.add(&tiled(r, "input-mouse-symbolic"));

        let accel = fields.get("accel_profile").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let n = name.clone();
        let (r, _) = widgets::choice_row("Acceleration", "", opts(&[("", "Same as every mouse"), ("adaptive", "Adaptive"), ("flat", "Flat")]), &accel, move |id| {
            if !id.is_empty() {
                set(&n, "accel_profile", json!(id));
            }
        });
        g.add(&tiled(r, "power-profile-performance-symbolic"));

        let natural = fields.get("natural_scroll").and_then(|v| v.as_bool()).unwrap_or(false);
        let n = name.clone();
        let (r, _) = widgets::switch_row("Natural scrolling", "", natural, move |on| set(&n, "natural_scroll", json!(on)));
        g.add(&tiled(r, "view-continuous-symbolic"));

        let g = p.group("");
        let n = name.clone();
        let w3 = w2.clone();
        let (r, _) = widgets::button_row("Use the settings for every mouse", "Removes this mouse's own settings.", "Reset", move |_| {
            store::reset_device(&n);
            crate::window::toast("Device overrides removed");
            if let Some(w) = w3.borrow().as_ref() {
                w.close();
            }
            crate::window::rebuild("mouse");
        });
        g.add(&r);
        if remembered.is_some() {
            let n = name.clone();
            let w3 = w2.clone();
            let (r, _) = widgets::button_row("Forget this mouse", "It's not connected now.", "Forget", move |_| {
                match devices::forget(&DeviceId::new(Category::Mouse, &n)) {
                    Ok(()) => crate::window::toast("Mouse forgotten"),
                    Err(e) => crate::window::toast(&format!("Couldn't forget all of it: {e}")),
                }
                if let Some(w) = w3.borrow().as_ref() {
                    w.close();
                }
                crate::window::rebuild("mouse");
            });
            g.add(&r);
        }
    });
    made.connect_close_request(move |_| {
        redraw();
        glib::Propagation::Proceed
    });
    *win.borrow_mut() = Some(made);
}

/// Set one of a mouse's own settings, and remember the mouse.
fn set(name: &str, key: &str, value: serde_json::Value) {
    store::set_device(name, key, value);
    devices::remember_mouse(name);
}
