use crate::backend::shell;
use crate::sketch;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use serde_json::json;
use std::cell::Cell;
use std::rc::Rc;

pub fn minutes_label(secs: i64) -> String {
    if secs <= 0 { "Never".into() } else { format!("{} min", secs / 60) }
}

/// Seconds until the screensaver and the lock, as the timeline shows them.
type Times = Rc<Cell<(i64, i64)>>;

/// How far the timeline runs, in minutes: past the last thing that happens.
fn span(saver: i64, lock: i64) -> f64 {
    let last = saver.max(lock) as f64 / 60.0;
    (last * 1.5).clamp(16.0, 120.0)
}

const TRACK_X0: f64 = 22.0;
const TRACK_PAD_R: f64 = 22.0;
const TRACK_Y: f64 = 22.0;

fn track_x(min: f64, w: f64, end: f64) -> f64 {
    TRACK_X0 + (w - TRACK_X0 - TRACK_PAD_R) * (min / end).clamp(0.0, 1.0)
}

/// The track: a dot where you stop, one for the screensaver and one for the lock, and
/// the minutes under it.
fn draw_track(cr: &gtk::cairo::Context, p: &sketch::Pal, accent: sketch::Rgb, w: f64, (saver, lock): (i64, i64), saver_shown: bool, held: Option<bool>) {
    let end = span(saver, lock);
    let x1 = w - TRACK_PAD_R;
    sketch::set(cr, p.dim, 0.25);
    sketch::rounded(cr, TRACK_X0, TRACK_Y - 3.0, x1 - TRACK_X0, 6.0, 3.0);
    let _ = cr.fill();
    let last = [saver, lock].into_iter().filter(|s| *s > 0).max().unwrap_or(0);
    if last > 0 {
        let g = gtk::cairo::LinearGradient::new(TRACK_X0, 0.0, track_x(last as f64 / 60.0, w, end), 0.0);
        g.add_color_stop_rgba(0.0, accent.0, accent.1, accent.2, 0.25);
        g.add_color_stop_rgba(1.0, accent.0, accent.1, accent.2, 0.95);
        sketch::rounded(cr, TRACK_X0, TRACK_Y - 3.0, track_x(last as f64 / 60.0, w, end) - TRACK_X0, 6.0, 3.0);
        let _ = cr.set_source(&g);
        let _ = cr.fill();
    }
    let text = |cr: &gtk::cairo::Context, s: &str, x: f64, y: f64, c: sketch::Rgb, bold: bool, size: f64| {
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, if bold { gtk::cairo::FontWeight::Bold } else { gtk::cairo::FontWeight::Normal });
        cr.set_font_size(size);
        if let Ok(e) = cr.text_extents(s) {
            let x = (x - e.x_advance() / 2.0).min(w - e.x_advance() - 2.0).max(2.0);
            sketch::set(cr, c, 1.0);
            cr.move_to(x, y);
            let _ = cr.show_text(s);
        }
    };
    // Where you stop.
    sketch::set(cr, p.text, 0.9);
    cr.arc(TRACK_X0, TRACK_Y, 5.0, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
    text(cr, "You stop", TRACK_X0 + 14.0, TRACK_Y + 26.0, p.text, true, 11.5);
    // The two that happen, the later one's label lower when they're close.
    let mut marks: Vec<(bool, i64)> = vec![(false, saver), (true, lock)].into_iter().filter(|(l, s)| *s > 0 && (*l || saver_shown)).collect();
    marks.sort_by_key(|(_, s)| *s);
    let mut prev_x = f64::NEG_INFINITY;
    for (is_lock, secs) in marks {
        let x = track_x(secs as f64 / 60.0, w, end);
        let r = if held == Some(is_lock) { 11.0 } else { 9.0 };
        cr.new_path();
        cr.arc(x, TRACK_Y, r, 0.0, std::f64::consts::TAU);
        sketch::set(cr, sketch::mix(p.text, (1.0, 1.0, 1.0), 0.6), 1.0);
        let _ = cr.fill_preserve();
        sketch::set(cr, accent, 1.0);
        cr.set_line_width(3.0);
        let _ = cr.stroke();
        let low = if x - prev_x < 90.0 { 26.0 } else { 0.0 };
        text(cr, if is_lock { "Lock" } else { "Screensaver" }, x, TRACK_Y + 28.0 + low, p.text, true, 11.5);
        text(cr, &minutes_label(secs), x, TRACK_Y + 42.0 + low, p.dim, false, 10.5);
        prev_x = x;
    }
    // Minutes along the foot.
    let step = if end <= 20.0 { 2.0 } else if end <= 40.0 { 5.0 } else { 15.0 };
    let mut m = 0.0;
    while m <= end + 0.01 {
        text(cr, &format!("{}", m as i64), track_x(m, w, end), TRACK_Y + 76.0, p.dim, false, 10.0);
        m += step;
    }
}

/// The three moments: your desktop, the screensaver, the lock screen, drawn small with
/// what happens when under each.
fn draw_stages(cr: &gtk::cairo::Context, p: &sketch::Pal, accent: sketch::Rgb, w: f64, h: f64, (saver, lock): (i64, i64), saver_shown: bool) {
    let gap = 22.0;
    let pw = ((w - 2.0 * gap) / 3.0).max(40.0);
    let ph = (pw * 0.62).min(h - 44.0);
    let y = ((h - 44.0 - ph) / 2.0).max(0.0);
    let saver_on = saver_shown && saver > 0;
    // Which happens last is outlined.
    let stages: [(&str, String, &str, bool); 3] = [
        ("desk", "You stop".into(), "Your desktop, as you left it", true),
        ("saver", if saver_on { minutes_label(saver).replace("min", "minutes") } else { "Off".into() }, "The screensaver starts", saver_on),
        ("lock", if lock > 0 { minutes_label(lock).replace("min", "minutes") } else { "Never".into() }, "The screen locks", lock > 0),
    ];
    let last = if lock > 0 { 2 } else if saver_on { 1 } else { 0 };
    for (i, (kind, when, what, on)) in stages.iter().enumerate() {
        let x = i as f64 * (pw + gap);
        let r = sketch::R::new(x + 1.5, y + 1.5, pw - 3.0, ph - 3.0);
        cr.save().ok();
        if !on {
            cr.push_group();
        }
        match *kind {
            "desk" => {
                sketch::desktop(cr, p, r);
            }
            "saver" => {
                sketch::rounded(cr, r.x, r.y, r.w, r.h, 8.0);
                sketch::set(cr, sketch::mix(p.bg, (0.0, 0.0, 0.0), 0.6), 1.0);
                let _ = cr.fill();
                cr.select_font_face("monospace", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
                let name = gtk::glib::host_name().to_uppercase();
                let mut size = r.h * 0.22;
                cr.set_font_size(size);
                if let Ok(e) = cr.text_extents(&name)
                    && e.x_advance() > r.w * 0.8
                {
                    size *= r.w * 0.8 / e.x_advance();
                    cr.set_font_size(size);
                }
                if let Ok(e) = cr.text_extents(&name) {
                    let g = gtk::cairo::LinearGradient::new(r.x, 0.0, r.x + r.w, 0.0);
                    g.add_color_stop_rgb(0.0, accent.0, accent.1, accent.2);
                    let b = sketch::mix(accent, p.text, 0.6);
                    g.add_color_stop_rgb(1.0, b.0, b.1, b.2);
                    let _ = cr.set_source(&g);
                    cr.move_to(r.cx() - e.x_advance() / 2.0, r.cy() + e.height() / 2.0);
                    let _ = cr.show_text(&name);
                }
            }
            _ => {
                sketch::desktop(cr, p, r);
                sketch::rounded(cr, r.x, r.y, r.w, r.h, 8.0);
                sketch::set(cr, sketch::mix(p.bg, (0.0, 0.0, 0.0), 0.3), 0.82);
                let _ = cr.fill();
                let now = gtk::glib::DateTime::now_local().ok();
                let clock = now.as_ref().and_then(|t| t.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default();
                let date = now.as_ref().and_then(|t| t.format("%A, %-d %B").ok()).map(|s| s.to_string()).unwrap_or_default();
                cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
                cr.set_font_size(r.h * 0.2);
                sketch::set(cr, accent, 1.0);
                if let Ok(e) = cr.text_extents(&clock) {
                    cr.move_to(r.cx() - e.x_advance() / 2.0, r.y + r.h * 0.34);
                    let _ = cr.show_text(&clock);
                }
                cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
                cr.set_font_size((r.h * 0.075).max(7.0));
                sketch::set(cr, p.text, 0.85);
                if let Ok(e) = cr.text_extents(&date) {
                    cr.move_to(r.cx() - e.x_advance() / 2.0, r.y + r.h * 0.48);
                    let _ = cr.show_text(&date);
                }
                // You, by your initial, over the password field.
                let who = gtk::glib::real_name().to_string_lossy().to_string();
                let who = if who.trim().is_empty() || who == "Unknown" { gtk::glib::user_name().to_string_lossy().to_string() } else { who };
                let rad = r.h * 0.09;
                cr.new_path();
                cr.arc(r.cx(), r.y + r.h * 0.66, rad, 0.0, std::f64::consts::TAU);
                sketch::set(cr, accent, 0.9);
                let _ = cr.fill();
                let initial: String = who.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
                cr.set_font_size(rad * 1.1);
                sketch::set(cr, p.bg, 1.0);
                if let Ok(e) = cr.text_extents(&initial) {
                    cr.move_to(r.cx() - e.x_advance() / 2.0, r.y + r.h * 0.66 + e.height() / 2.0);
                    let _ = cr.show_text(&initial);
                }
                cr.set_font_size((r.h * 0.07).max(7.0));
                sketch::set(cr, p.text, 0.9);
                if let Ok(e) = cr.text_extents(&who) {
                    cr.move_to(r.cx() - e.x_advance() / 2.0, r.y + r.h * 0.86);
                    let _ = cr.show_text(&who);
                }
            }
        }
        if !on {
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(0.3);
        }
        cr.restore().ok();
        sketch::rounded(cr, r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0, 9.0);
        if i == last {
            sketch::set(cr, accent, 1.0);
            cr.set_line_width(2.5);
        } else {
            sketch::set(cr, p.dim, 0.35);
            cr.set_line_width(1.0);
        }
        let _ = cr.stroke();
        // The arrow to the next.
        if i < 2 {
            let ax = x + pw + gap / 2.0;
            sketch::set(cr, p.dim, 0.7);
            cr.set_line_width(1.6);
            cr.move_to(ax - 3.0, y + ph / 2.0 - 5.0);
            cr.line_to(ax + 2.0, y + ph / 2.0);
            cr.line_to(ax - 3.0, y + ph / 2.0 + 5.0);
            let _ = cr.stroke();
        }
        // When, and what, kept to the picture's width.
        cr.save().ok();
        cr.rectangle(x, y + ph, pw + gap / 2.0, 44.0);
        cr.clip();
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
        cr.set_font_size(12.5);
        sketch::set(cr, if i == last { accent } else { p.text }, if *on { 1.0 } else { 0.5 });
        cr.move_to(x, y + ph + 18.0);
        let _ = cr.show_text(when);
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
        cr.set_font_size(11.0);
        sketch::set(cr, p.dim, if *on { 1.0 } else { 0.5 });
        cr.move_to(x, y + ph + 34.0);
        let _ = cr.show_text(what);
        cr.restore().ok();
    }
}

pub fn build(page: &Page) {
    let saver = shell::get(&["idle", "screensaver"]).and_then(|v| v.as_i64()).unwrap_or(150);
    let lock = shell::get(&["idle", "lock"]).and_then(|v| v.as_i64()).unwrap_or(300);
    let times: Times = Rc::new(Cell::new((saver, lock)));
    let saver_shown = Rc::new(Cell::new(!shell::toggle_on("screensaver-off")));

    // The picture: what happens after you stop, and a track to drag.
    let pv = page.split();
    pv.head("When you step away", "Drag a dot to change when it happens.", &[]);
    let middle = widgets::vbox(14);
    middle.set_vexpand(true);
    middle.set_valign(gtk::Align::Center);
    pv.add(&middle);
    let stages = {
        let (t, s) = (times.clone(), saver_shown.clone());
        widgets::drawing(180, move |cr, p, accent, w, h| draw_stages(cr, p, accent, w, h, t.get(), s.get()))
    };
    middle.append(&stages);
    let held: Rc<Cell<Option<bool>>> = Rc::default();
    let track = {
        let (t, s, held) = (times.clone(), saver_shown.clone(), held.clone());
        widgets::drawing(110, move |cr, p, accent, w, _h| draw_track(cr, p, accent, w, t.get(), s.get(), held.get()))
    };
    let track_card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    track_card.add_css_class("preview-card");
    track_card.add_css_class("idl-track");
    track_card.append(&track);
    middle.append(&track_card);
    pv.foot("dialog-information-symbolic", "Moving the mouse or pressing a key starts the count again from the beginning.");

    // ----- When you step away -----
    let g = page.group("When you step away");
    let redraw = {
        let (stages, track) = (stages.clone(), track.clone());
        move || {
            stages.queue_draw();
            track.queue_draw();
        }
    };
    let (saver_row, saver_scale) = idle_slider("screensaver", "Screensaver after", "preferences-desktop-screensaver-symbolic", saver, times.clone(), redraw.clone());
    g.add(&saver_row);
    let (lock_row, lock_scale) = idle_slider("lock", "Lock after", "system-lock-screen-symbolic", lock, times.clone(), redraw.clone());
    g.add(&lock_row);
    widgets::keywords("timeout minutes idle lock screen never");
    let sw = gtk::Switch::new();
    sw.set_active(saver_shown.get());
    {
        let (shown, redraw) = (saver_shown.clone(), redraw.clone());
        sw.connect_active_notify(move |s| {
            shown.set(s.is_active());
            redraw();
            cmd::spawn(&["omarchy-toggle-screensaver"]);
        });
    }
    g.add(&widgets::icon_row("view-reveal-symbolic", "Show the screensaver", "Off goes straight to the lock screen.", Some(sw.upcast_ref())));
    widgets::keywords("screensaver omarchy show before locking");

    // Drag the dots on the track: it moves the sliders, which save.
    let drag = gtk::GestureDrag::new();
    {
        let (t, held, track2, shown) = (times.clone(), held.clone(), track.clone(), saver_shown.clone());
        drag.connect_drag_begin(move |_, x, y| {
            let w = track2.width() as f64;
            let (saver, lock) = t.get();
            let end = span(saver, lock);
            let near = |secs: i64| (track_x(secs as f64 / 60.0, w, end) - x).abs();
            let mut pick = None;
            if lock > 0 && near(lock) < 16.0 {
                pick = Some(true);
            }
            if shown.get() && saver > 0 && near(saver) < 16.0 && pick.is_none_or(|_| near(saver) < near(lock)) {
                pick = Some(false);
            }
            if (y - TRACK_Y).abs() > 24.0 {
                pick = None;
            }
            held.set(pick);
            track2.queue_draw();
        });
    }
    {
        let (held, track2, ss, ls) = (held.clone(), track.clone(), saver_scale.clone(), lock_scale.clone());
        // The span is kept while dragging, so the track doesn't rescale under the pointer.
        let frozen: Rc<Cell<f64>> = Rc::default();
        let f2 = frozen.clone();
        let t2 = times.clone();
        drag.connect_drag_begin(move |_, _, _| {
            let (a, b) = t2.get();
            f2.set(span(a, b));
        });
        drag.connect_drag_update(move |d, dx, _| {
            let (Some(is_lock), Some((sx, _))) = (held.get(), d.start_point()) else { return };
            let w = track2.width() as f64;
            let end = frozen.get();
            let min = ((sx + dx - TRACK_X0) / (w - TRACK_X0 - TRACK_PAD_R) * end).clamp(1.0, 60.0).round();
            if is_lock { ls.set_value(min) } else { ss.set_value(min) }
        });
    }
    {
        let (held, track2) = (held.clone(), track.clone());
        drag.connect_drag_end(move |_, _, _| {
            held.set(None);
            track2.queue_draw();
        });
    }
    track.add_controller(drag);
    track.set_cursor_from_name(Some("ew-resize"));

    // ----- Right now -----
    let g = page.group("Right now");
    let awake = cmd::output(&["omarchy-toggle-idle", "status"])
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("enabled").and_then(|b| b.as_bool()))
        .unwrap_or(false);
    let sw = gtk::Switch::new();
    sw.set_active(awake);
    sw.connect_active_notify(|s| cmd::spawn(&["omarchy-toggle-idle", if s.is_active() { "stay-awake" } else { "allow-idle" }]));
    g.add(&widgets::icon_row(
        "display-brightness-symbolic",
        "Stay awake",
        "Until you turn it off: for talks and downloads.",
        Some(sw.upcast_ref()),
    ));
    widgets::keywords("caffeine inhibit presentation");
    let lock_btn = gtk::Button::new();
    let inner = widgets::hbox(6);
    inner.append(&gtk::Image::from_icon_name("system-lock-screen-symbolic"));
    inner.append(&gtk::Label::new(Some("Lock")));
    lock_btn.set_child(Some(&inner));
    lock_btn.connect_clicked(|_| cmd::spawn(&["omarchy-system-lock"]));
    g.add(&widgets::icon_row("system-lock-screen-symbolic", "Lock the screen now", "", Some(lock_btn.upcast_ref())));
    widgets::keywords("lock now");

    // ----- System menu -----
    let g = page.group("System menu");
    let sw = gtk::Switch::new();
    sw.set_active(!shell::toggle_on("suspend-off"));
    sw.connect_active_notify(|_| cmd::spawn(&["omarchy-toggle-suspend"]));
    g.add(&widgets::icon_row(
        "weather-clear-night-symbolic",
        "Show Suspend",
        "In the menu with Lock, Restart and Shut down.",
        Some(sw.upcast_ref()),
    ));
    widgets::keywords("suspend in the system menu sleep");
    let (r, _) = widgets::nav_row(Some("battery-level-80-symbolic"), "Lid, power button and sleep", "On Power & Battery", "", || {
        window::navigate("power")
    });
    widgets::keywords("lid power button hibernate");
    g.add(&r);
}

/// A slider in minutes for the screensaver or the lock, "Never" at zero.
fn idle_slider(key: &'static str, title: &str, icon: &str, secs: i64, times: Times, redraw: impl Fn() + 'static) -> (gtk::Box, gtk::Scale) {
    let lock = key == "lock";
    let (r, s) = widgets::slider_row(title, "", (0.0, 60.0, 1.0), (secs / 60) as f64, 0, " min", move |v| {
        let secs = (v.round() as i64) * 60;
        let (a, b) = times.get();
        times.set(if lock { (a, secs) } else { (secs, b) });
        redraw();
        if let Err(e) = shell::set(&["idle", key], json!(secs)) {
            window::toast(&format!("Couldn't save: {e}"));
        }
    });
    let readout = s.readout.clone();
    readout.set_text(&minutes_label(secs));
    s.scale.connect_value_changed(move |sc| readout.set_text(&minutes_label(sc.value().round() as i64 * 60)));
    s.scale.set_tooltip_text(Some("All the way left is never"));
    s.scale.set_width_request(130);
    r.prepend(&widgets::tile(icon));
    r.add_css_class("with-tile");
    (r, s.scale)
}
