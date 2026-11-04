use crate::backend::shell;
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

fn idle_row(key: &'static str, title: &str, desc: &str, default: i64, times: Times, timeline: gtk::DrawingArea) -> gtk::Box {
    let secs = shell::get(&["idle", key]).and_then(|v| v.as_i64()).unwrap_or(default);
    let lock = key == "lock";
    let (r, s) = widgets::slider_row(title, desc, (0.0, 60.0, 1.0), (secs / 60) as f64, 0, " min", move |v| {
        let secs = (v.round() as i64) * 60;
        let (a, b) = times.get();
        times.set(if lock { (a, secs) } else { (secs, b) });
        timeline.queue_draw();
        if let Err(e) = shell::set(&["idle", key], json!(secs)) {
            window::toast(&format!("Couldn't save: {e}"));
        }
    });
    // Show "Never" at zero.
    let readout = s.readout.clone();
    readout.set_text(&minutes_label(secs));
    s.scale.connect_value_changed(move |sc| readout.set_text(&minutes_label(sc.value().round() as i64 * 60)));
    r
}

/// A line from "you stop" to what happens when, at their minutes: `marks` gives
/// (seconds, name) on every draw, 0 seconds for never.
pub fn timeline(marks: impl Fn() -> Vec<(i64, &'static str)> + 'static) -> gtk::DrawingArea {
    use crate::sketch::{self, mix, rounded, set};
    let area = gtk::DrawingArea::new();
    area.set_content_height(74);
    area.set_hexpand(true);
    area.set_draw_func(move |_, cr, w, _h| {
        let p = sketch::pal();
        let w = w as f64;
        let mut marks: Vec<(i64, &str)> = marks().into_iter().filter(|m| m.0 > 0).collect();
        let last = marks.iter().map(|m| m.0).max().unwrap_or(0);
        let end = (last as f64 / 60.0 + 5.0).clamp(15.0, 125.0);
        let (x0, x1, y) = (16.0, w - 16.0, 30.0);
        let at = |secs: i64| x0 + (x1 - x0) * (secs as f64 / 60.0 / end);
        set(cr, p.dim, 0.35);
        rounded(cr, x0, y - 2.0, x1 - x0, 4.0, 2.0);
        let _ = cr.fill();
        let text = |cr: &gtk::cairo::Context, s: &str, x: f64, y: f64, c: sketch::Rgb, bold: bool| {
            cr.select_font_face(
                "sans-serif",
                gtk::cairo::FontSlant::Normal,
                if bold { gtk::cairo::FontWeight::Bold } else { gtk::cairo::FontWeight::Normal },
            );
            cr.set_font_size(11.0);
            if let Ok(e) = cr.text_extents(s) {
                let x = (x - e.width() / 2.0).clamp(2.0, w - e.width() - 2.0);
                set(cr, c, 1.0);
                cr.move_to(x - e.x_bearing(), y);
                let _ = cr.show_text(s);
            }
        };
        set(cr, p.text, 1.0);
        cr.arc(x0, y, 5.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        text(cr, "You stop", x0 + 20.0, y - 12.0, p.dim, false);
        marks.sort_by_key(|m| m.0);
        for (i, (secs, name)) in marks.iter().enumerate() {
            let x = at(*secs);
            set(cr, mix(p.accent, p.text, 0.0), 1.0);
            cr.arc(x, y, 6.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            let below = i % 2 == 1 && (x - at(marks[i - 1].0)) < 110.0;
            let ly = if below { y + 32.0 } else { y + 22.0 };
            text(cr, &format!("{name} · {} min", secs / 60), x, ly, p.text, true);
        }
        if marks.is_empty() {
            text(cr, "Nothing happens on its own", (x0 + x1) / 2.0, y + 22.0, p.dim, false);
        }
    });
    area
}

pub fn build(page: &Page) {
    let g = page.group("When idle");
    let saver = shell::get(&["idle", "screensaver"]).and_then(|v| v.as_i64()).unwrap_or(150);
    let lock = shell::get(&["idle", "lock"]).and_then(|v| v.as_i64()).unwrap_or(300);
    let times: Times = Rc::new(Cell::new((saver, lock)));
    let t = times.clone();
    let line = timeline(move || {
        let (saver, lock) = t.get();
        vec![(saver, "Screensaver"), (lock, "Lock")]
    });
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("preview-card");
    card.append(&line);
    g.add(&card);
    g.add(&idle_row(
        "screensaver",
        "Start the screensaver after",
        "0 turns the screensaver off.",
        150,
        times.clone(),
        line.clone(),
    ));
    g.add(&idle_row("lock", "Lock the screen after", "0 never locks on its own.", 300, times, line));
    widgets::keywords("timeout minutes");

    let g = page.group("Right now");
    let awake = cmd::output(&["omarchy-toggle-idle", "status"])
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("enabled").and_then(|b| b.as_bool()))
        .unwrap_or(false);
    let (r, _) = widgets::switch_row(
        "Stay awake",
        "Keep the screen on and unlocked until you turn this off — for presentations and long downloads.",
        awake,
        |on| cmd::spawn(&["omarchy-toggle-idle", if on { "stay-awake" } else { "allow-idle" }]),
    );
    widgets::keywords("caffeine inhibit presentation");
    g.add(&r);

    let g = page.group("Lock & sleep");
    let (r, _) = widgets::switch_row(
        "Screensaver",
        "Show Omarchy's screensaver before locking.",
        !shell::toggle_on("screensaver-off"),
        |_| cmd::spawn(&["omarchy-toggle-screensaver"]),
    );
    g.add(&r);
    let (r, _) = widgets::switch_row("Suspend in the system menu", "", !shell::toggle_on("suspend-off"), |_| {
        cmd::spawn(&["omarchy-toggle-suspend"])
    });
    g.add(&r);
    let (r, _) = widgets::button_row("Lock now", "", "Lock", |_| cmd::spawn(&["omarchy-system-lock"]));
    g.add(&r);
}
