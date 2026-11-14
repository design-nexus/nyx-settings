//! NyxOS's Lock Screen: when the screen locks, turns off and sleeps (the idle settings,
//! which hypridle reads when it starts), how Nyx Locker looks (`appearance`'s
//! `lock-style`), and staying awake for a while. All of it is the user's own, through
//! `nyx-system config`; no password is asked for.

use super::pages;
use crate::sections::idle::minutes_label;
use crate::sketch::{self, R, Rgb, Sketch};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::cairo::Context;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::f64::consts::{PI, TAU};
use std::rc::Rc;

/// (key, title, description, default seconds, longest minutes, name on the axis, icon)
const TIMES: [(&str, &str, &str, i64, f64, &str, &str); 3] = [
    ("lock", "Lock the screen after", "0 never locks on its own", 300, 60.0, "Lock", "system-lock-screen-symbolic"),
    ("screen-off", "Turn the screen off after", "0 leaves it on", 360, 60.0, "Off", "video-display-symbolic"),
    ("suspend", "Sleep after", "0 never; closing the lid still sleeps", 0, 120.0, "Sleep", "weather-clear-night-symbolic"),
];

const STYLES: [(&str, &str); 4] =
    [("circular", "Circular"), ("cyberpunk", "Cyberpunk"), ("minimal", "Minimal"), ("edge", "Edge glow")];

/// The transient unit that holds an idle inhibitor while Stay awake is on.
const AWAKE_UNIT: &str = "nyx-stay-awake";

fn get(domain: &str, key: &str) -> Option<String> {
    cmd::output(&["nyx-system", "config", "get", domain, key]).map(|s| s.trim().to_string())
}

fn set(domain: &str, key: &str, value: &str) -> anyhow::Result<()> {
    cmd::run(&["nyx-system", "config", "set", domain, key, value]).map(|_| ())
}

/// Save one time and restart hypridle to read it; held back while a slider moves.
fn save_later(pending: &Rc<RefCell<Option<glib::SourceId>>>, key: &'static str, secs: i64) {
    if let Some(id) = pending.borrow_mut().take() {
        id.remove();
    }
    let p = pending.clone();
    let id = glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
        p.borrow_mut().take();
        match set("idle", key, &secs.to_string()) {
            Ok(()) => cmd::spawn(&["systemctl", "--user", "try-restart", "hypridle.service"]),
            Err(e) => window::toast(&format!("Couldn't save: {e}")),
        }
    });
    *pending.borrow_mut() = Some(id);
}

fn awake() -> bool {
    cmd::output(&["systemctl", "--user", "is-active", &format!("{AWAKE_UNIT}.service")]).is_some_and(|s| s.trim() == "active")
}

fn set_awake(on: bool) {
    if on {
        cmd::spawn(&[
            "systemd-run",
            "--user",
            "--collect",
            &format!("--unit={AWAKE_UNIT}"),
            "--description=Stay awake (Nyx Settings)",
            "systemd-inhibit",
            "--what=idle",
            "--who=Nyx Settings",
            "--why=Stay awake is on",
            "sleep",
            "infinity",
        ]);
    } else {
        cmd::spawn(&["systemctl", "--user", "stop", &format!("{AWAKE_UNIT}.service")]);
    }
}

/// The ways in besides the password, from Nyx Locker's configuration.
fn ways_in() -> Vec<String> {
    let text = std::fs::read_to_string("/etc/nyx-locker/config.toml").unwrap_or_default();
    let methods: Vec<String> = text
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("methods")?.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    methods
        .iter()
        .filter_map(|m| match m.as_str() {
            "fingerprint" => Some("a fingerprint".to_string()),
            "face" => Some("your face".to_string()),
            "key" => Some("a security key".to_string()),
            _ => None,
        })
        .collect()
}

/// "after 5 min", or that it doesn't happen on its own.
fn after(secs: i64) -> String {
    if secs <= 0 { "Never on its own".into() } else { format!("after {}", minutes_label(secs)) }
}

/// The axis: the first ten minutes take most of it, so 5 and 6 minutes don't run
/// together; `end` minutes is its far end.
fn axis_x(min: f64, end: f64, x0: f64, x1: f64) -> f64 {
    const FIRST: f64 = 0.6;
    let t = if min <= 10.0 { min / 10.0 * FIRST } else { FIRST + (min - 10.0) / (end - 10.0).max(1.0) * (1.0 - FIRST) };
    x0 + (x1 - x0) * t.clamp(0.0, 1.0)
}

fn axis_minutes(x: f64, end: f64, x0: f64, x1: f64) -> f64 {
    const FIRST: f64 = 0.6;
    let t = ((x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0);
    if t <= FIRST { t / FIRST * 10.0 } else { 10.0 + (t - FIRST) / (1.0 - FIRST) * (end - 10.0) }
}

/// Where the axis ends for the longest time set.
fn axis_end(longest_min: f64) -> f64 {
    if longest_min <= 30.0 {
        40.0
    } else if longest_min <= 60.0 {
        70.0
    } else {
        130.0
    }
}

/// A small picture of what's on the screen at each step.
fn draw_stage(cr: &Context, p: &sketch::Pal, accent: Rgb, w: f64, h: f64, step: usize, style: &'static str, picked: bool) {
    let r = R::new(1.0, 1.0, w - 2.0, h - 2.0);
    match step {
        0 => {
            sketch::desktop(cr, p, r);
        }
        1 => {
            // Nyx Locker in the style picked, drawn the way its picture card is.
            cr.save().ok();
            sketch::rounded(cr, r.x, r.y, r.w, r.h, 8.0);
            cr.clip();
            sketch::draw(cr, Sketch::LockStyle(style), w, h, true);
            cr.restore().ok();
        }
        2 => {
            sketch::rounded(cr, r.x, r.y, r.w, r.h, 8.0);
            sketch::set(cr, (0.0, 0.0, 0.0), 1.0);
            let _ = cr.fill();
        }
        _ => {
            sketch::rounded(cr, r.x, r.y, r.w, r.h, 8.0);
            sketch::set(cr, (0.0, 0.0, 0.0), 1.0);
            let _ = cr.fill();
            let (cx, cy, rad) = (r.cx(), r.cy(), r.h * 0.11);
            sketch::set(cr, p.accent, 0.9);
            cr.arc(cx, cy, rad, 0.0, TAU);
            cr.arc_negative(cx + rad * 0.55, cy - rad * 0.35, rad * 0.85, TAU, 0.0);
            cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
            let _ = cr.fill();
            cr.set_fill_rule(gtk::cairo::FillRule::Winding);
        }
    }
    sketch::rounded(cr, r.x, r.y, r.w, r.h, 8.0);
    if picked {
        sketch::set(cr, accent, 1.0);
        cr.set_line_width(2.0);
    } else {
        sketch::set(cr, p.text, 0.12);
        cr.set_line_width(1.0);
    }
    let _ = cr.stroke();
}

/// A handle's sign: a padlock, a screen or a moon.
fn glyph(cr: &Context, which: usize, x: f64, y: f64, c: Rgb) {
    sketch::set(cr, c, 1.0);
    cr.set_line_width(1.4);
    match which {
        0 => {
            sketch::rounded(cr, x - 3.5, y - 1.0, 7.0, 5.0, 1.0);
            let _ = cr.fill();
            cr.arc(x, y - 1.5, 2.3, PI, 0.0);
            let _ = cr.stroke();
        }
        1 => {
            sketch::rounded(cr, x - 4.0, y - 3.5, 8.0, 5.5, 1.0);
            let _ = cr.stroke();
            cr.move_to(x - 2.0, y + 4.0);
            cr.line_to(x + 2.0, y + 4.0);
            let _ = cr.stroke();
        }
        _ => {
            cr.arc(x, y, 4.0, 0.0, TAU);
            cr.arc_negative(x + 2.2, y - 1.4, 3.4, TAU, 0.0);
            cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
            let _ = cr.fill();
            cr.set_fill_rule(gtk::cairo::FillRule::Winding);
        }
    }
}

/// Where the axis's line runs in a drawing `w` wide.
fn axis_span(w: f64) -> (f64, f64) {
    (14.0, w - 22.0)
}

fn draw_axis(cr: &Context, p: &sketch::Pal, accent: Rgb, w: f64, secs: &[i64; 3], end: f64, hot: Option<usize>) {
    let (x0, x1) = axis_span(w);
    let y = 44.0;
    let at = |s: i64| axis_x(s as f64 / 60.0, end, x0, x1);
    // What the screen is doing, from your last touch: working, then each step in turn.
    let mut steps: Vec<(i64, usize)> = secs.iter().enumerate().filter(|(_, s)| **s > 0).map(|(i, s)| (*s, i)).collect();
    steps.sort();
    let colours = [accent, sketch::mix(p.dim, p.bg, 0.3), sketch::mix(p.accent, p.bg, 0.45)];
    let mut from = (x0, sketch::mix(p.accent, p.bg, 0.25));
    for (s, i) in &steps {
        let x = at(*s);
        sketch::rounded(cr, from.0, y - 4.0, (x - from.0).max(0.0), 8.0, 2.0);
        sketch::set(cr, from.1, 1.0);
        let _ = cr.fill();
        from = (x, colours[*i]);
    }
    sketch::rounded(cr, from.0, y - 4.0, (x1 - from.0).max(0.0), 8.0, 2.0);
    sketch::set(cr, from.1, 1.0);
    let _ = cr.fill();
    // The minutes under it.
    let mut ticks: Vec<f64> = vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0];
    ticks.extend([20.0, 30.0, 45.0, 60.0, 90.0, 120.0].into_iter().filter(|m| *m < end));
    let mut last = -100.0;
    for (i, m) in ticks.iter().chain(std::iter::once(&end)).enumerate() {
        let x = axis_x(*m, end, x0, x1);
        if i < ticks.len() && (x - last < 24.0 || x > x1 - 44.0) {
            continue;
        }
        let s = if *m == end { format!("{} min", *m as i64) } else { format!("{}", *m as i64) };
        pages::text(cr, &s, x, y + 38.0, 10.5, false, p.dim, 0.9, if *m == end { 1.0 } else if i == 0 { 0.0 } else { 0.5 });
        last = x;
    }
    // A handle for each step, its name above or, when it'd crowd the one before, under.
    let mut previous: Option<f64> = None;
    for (s, i) in &steps {
        let x = at(*s);
        let r = if Some(*i) == hot { 11.0 } else { 10.0 };
        cr.arc(x, y, r, 0.0, TAU);
        sketch::set(cr, (1.0, 1.0, 1.0), 1.0);
        let _ = cr.fill_preserve();
        sketch::set(cr, if *i == 0 { accent } else { p.bg }, if *i == 0 { 1.0 } else { 0.4 });
        cr.set_line_width(if *i == 0 { 3.0 } else { 1.0 });
        let _ = cr.stroke();
        glyph(cr, *i, x, y, (0.12, 0.12, 0.14));
        let below = previous.is_some_and(|px| x - px < 92.0);
        let label = format!("{} · {}", TIMES[*i].5, minutes_label(*s));
        let align = if x < x0 + 40.0 { 0.0 } else if x > x1 - 40.0 { 1.0 } else { 0.5 };
        pages::text(cr, &label, x, if below { y + 24.0 } else { y - 18.0 }, 11.0, true, p.text, 0.9, align);
        previous = if below { None } else { Some(x) };
    }
}

pub fn build(page: &Page) {
    let secs: Rc<Vec<Cell<i64>>> = Rc::new(
        TIMES
            .iter()
            .map(|(key, _, _, default, _, _, _)| {
                Cell::new(get("idle", key).and_then(|v| v.parse().ok()).filter(|v: &i64| *v >= 0).unwrap_or(*default))
            })
            .collect(),
    );
    let current =
        get("appearance", "lock-style").filter(|s| STYLES.iter().any(|(id, _)| id == s)).unwrap_or_else(|| "circular".into());
    let style: Rc<Cell<&'static str>> =
        Rc::new(Cell::new(STYLES.iter().find(|(id, _)| *id == current).map(|(id, _)| *id).unwrap_or("circular")));

    // The picture: what happens when you walk away, as four screens and a time axis.
    let pv = page.split();
    let lock = gtk::Button::new();
    let inner = widgets::hbox(6);
    inner.append(&gtk::Image::from_icon_name("system-lock-screen-symbolic"));
    inner.append(&gtk::Label::new(Some("Lock now")));
    lock.set_child(Some(&inner));
    lock.connect_clicked(|_| cmd::spawn(&["loginctl", "lock-session"]));
    let column = pages::preview_column(&pv);
    let (head, _, _) =
        pages::preview_head("After your last touch", "What this computer does when you walk away, as you set it.", &[lock.upcast_ref()]);
    column.append(&head);

    let strip = widgets::hbox(6);
    strip.add_css_class("nx2-lks-strip");
    let names = ["You're working", "Locked", "Screen off", "Asleep"];
    let mut whens: Vec<gtk::Label> = Vec::new();
    let mut pictures: Vec<gtk::DrawingArea> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            let arrow = gtk::Image::from_icon_name("go-next-symbolic");
            arrow.add_css_class("nx2-lks-arrow");
            arrow.set_valign(gtk::Align::Start);
            arrow.set_margin_top(28);
            strip.append(&arrow);
        }
        let col = widgets::vbox(4);
        col.set_hexpand(true);
        let style = style.clone();
        let area = widgets::drawing(64, move |cr, p, accent, w, h| draw_stage(cr, p, accent, w, h, i, style.get(), i == 1));
        col.append(&area);
        let t = widgets::label(name, "nx2-lks-name");
        t.set_xalign(0.5);
        t.set_wrap(true);
        t.set_max_width_chars(10);
        t.set_justify(gtk::Justification::Center);
        col.append(&t);
        let when = widgets::label(if i == 0 { "Any key or touch starts this again" } else { "" }, "nx2-lks-when");
        when.set_xalign(0.5);
        when.set_wrap(true);
        when.set_max_width_chars(12);
        when.set_justify(gtk::Justification::Center);
        if i == 1 {
            when.add_css_class("hot");
        }
        col.append(&when);
        strip.append(&col);
        pictures.push(area);
        if i > 0 {
            whens.push(when);
        }
    }
    column.append(&strip);
    let describe = {
        let (secs, whens) = (secs.clone(), whens.clone());
        move || {
            for (i, l) in whens.iter().enumerate() {
                l.set_text(&after(secs[i].get()));
            }
        }
    };
    describe();

    let axis_card = pv.card();
    axis_card.add_css_class("nx2-lks-axis");
    let dragging: Rc<Cell<Option<(usize, f64)>>> = Rc::default();
    let axis = {
        let (secs, dragging) = (secs.clone(), dragging.clone());
        widgets::drawing(92, move |cr, p, accent, w, _| {
            let s = [secs[0].get(), secs[1].get(), secs[2].get()];
            let (end, hot) = match dragging.get() {
                Some((i, end)) => (end, Some(i)),
                None => (axis_end(s.iter().copied().max().unwrap_or(0) as f64 / 60.0), None),
            };
            draw_axis(cr, p, accent, w, &s, end, hot);
        })
    };
    axis.set_cursor_from_name(Some("grab"));
    axis_card.append(&axis);

    let note = widgets::hbox(10);
    note.add_css_class("preview-card");
    note.add_css_class("nx2-lks-note");
    let i = gtk::Image::from_icon_name("dialog-information-symbolic");
    i.set_valign(gtk::Align::Start);
    note.append(&i);
    let words = widgets::vbox(2);
    words.append(&widgets::label("On battery it's the same", "nx2-sub"));
    let l = widgets::label(
        "These times hold whether the computer is plugged in or not; Power has what the lid and the power button do.",
        "nx2-dim",
    );
    l.set_wrap(true);
    words.append(&l);
    note.append(&words);
    pv.add(&note);
    pv.foot("emblem-ok-symbolic", "Saved as you change it. No password, no rebuild.");

    // The controls: the three times, the look, the ways in, and staying awake.
    let g = page.group("When you're away");
    let scales: Rc<RefCell<Vec<gtk::Scale>>> = Rc::default();
    for (i, (key, title, desc, _, most, _, icon)) in TIMES.iter().enumerate() {
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        let (s, axis, pictures, describe) = (secs.clone(), axis.clone(), pictures.clone(), describe.clone());
        let start = secs[i].get();
        let (r, slider) = widgets::slider_row(title, desc, (0.0, *most, 1.0), (start / 60) as f64, 0, " min", move |v| {
            let v = (v.round() as i64) * 60;
            if s[i].get() == v {
                return;
            }
            s[i].set(v);
            axis.queue_draw();
            for p in &pictures {
                p.queue_draw();
            }
            describe();
            save_later(&pending, key, v);
        });
        slider.scale.set_width_request(84);
        let readout = slider.readout.clone();
        readout.set_text(&minutes_label(start));
        readout.set_width_chars(5);
        slider.scale.connect_value_changed(move |sc| readout.set_text(&minutes_label(sc.value().round() as i64 * 60)));
        r.prepend(&widgets::tile(icon));
        r.add_css_class("with-tile");
        g.add(&r);
        scales.borrow_mut().push(slider.scale.clone());
    }
    widgets::keywords("idle timeout minutes screen off dpms sleep suspend lock");

    // Dragging a handle on the axis moves its slider.
    let drag = gtk::GestureDrag::new();
    {
        let (secs, scales, axis2) = (secs.clone(), scales.clone(), axis.clone());
        let start_x = Rc::new(Cell::new(0.0));
        let (sx, dragging1) = (start_x.clone(), dragging.clone());
        drag.connect_drag_begin(move |_, x, _| {
            let w = axis2.width() as f64;
            let (x0, x1) = axis_span(w);
            let s = [secs[0].get(), secs[1].get(), secs[2].get()];
            let end = axis_end(s.iter().copied().max().unwrap_or(0) as f64 / 60.0);
            let near = (0..3)
                .filter(|i| s[*i] > 0)
                .map(|i| (i, (axis_x(s[i] as f64 / 60.0, end, x0, x1) - x).abs()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .filter(|(_, d)| *d < 18.0);
            dragging1.set(near.map(|(i, _)| (i, end)));
            sx.set(x);
            axis2.queue_draw();
        });
        let (dragging2, axis3) = (dragging.clone(), axis.clone());
        drag.connect_drag_update(move |_, dx, _| {
            let Some((i, end)) = dragging2.get() else { return };
            let (x0, x1) = axis_span(axis3.width() as f64);
            let most = TIMES[i].4;
            let m = axis_minutes(start_x.get() + dx, end, x0, x1).round().clamp(1.0, most);
            if let Some(sc) = scales.borrow().get(i) {
                sc.set_value(m);
            }
        });
        let axis4 = axis.clone();
        drag.connect_drag_end(move |_, _, _| {
            dragging.set(None);
            axis4.queue_draw();
        });
    }
    axis.add_controller(drag);

    let g = page.group("Look");
    let dd = widgets::dropdown(&STYLES.iter().map(|(id, name)| (id.to_string(), name.to_string())).collect::<Vec<_>>(), &current);
    {
        let (style, pictures) = (style.clone(), pictures.clone());
        dd.connect_selected_notify(move |d| {
            let Some((id, _)) = STYLES.get(d.selected() as usize) else { return };
            match set("appearance", "lock-style", id) {
                Ok(()) => {
                    style.set(id);
                    for p in &pictures {
                        p.queue_draw();
                    }
                }
                Err(e) => window::toast(&format!("Couldn't save: {e}")),
            }
        });
    }
    g.add(&widgets::icon_row(
        "view-reveal-symbolic",
        "Sign-in animation",
        "While it checks your password, fingerprint or face",
        Some(dd.upcast_ref()),
    ));
    let (r, _) = widgets::button_row(
        "Preview",
        "Opens the lock screen in a window, without locking; the password there is “nyx”.",
        "Preview",
        {
            let style = style.clone();
            move |_| cmd::spawn(&["nyx-locker", "--demo", "--style", style.get()])
        },
    );
    r.prepend(&widgets::tile("focus-windows-symbolic"));
    r.add_css_class("with-tile");
    widgets::keywords("lock style animation fingerprint face biometrics");
    g.add(&r);

    let g = page.group("");
    pages::gap(&g);
    let mut ways = ways_in();
    let desc = match ways.pop() {
        None => "Your password. Sign-in sets up a fingerprint, your face or a security key.".to_string(),
        Some(last) => {
            ways.insert(0, "Your password".into());
            format!("{} or {last}, whichever comes first.", ways.join(", "))
        }
    };
    let (r, v) = widgets::nav_row(Some("security-high-symbolic"), "Ways to unlock", &desc, "Sign-in", || window::navigate("sign-in"));
    v.set_ellipsize(gtk::pango::EllipsizeMode::None);
    widgets::keywords("unlock password fingerprint face security key");
    g.add(&r);

    let g = page.group("Right now");
    let sw = gtk::Switch::new();
    sw.set_active(awake());
    sw.connect_active_notify(|s| set_awake(s.is_active()));
    g.add(&widgets::icon_row(
        "weather-clear-symbolic",
        "Stay awake",
        "Keeps the screen on and unlocked until you turn it off or sign out — for presentations and long downloads.",
        Some(sw.upcast_ref()),
    ));
    widgets::keywords("caffeine inhibit presentation");
}
