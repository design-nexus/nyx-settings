use crate::backend::store;
use crate::sketch;
use crate::widgets::{self, Page};
use crate::{cmd, paths, window};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone)]
struct Schedule {
    on: bool,
    evening: String,
    morning: String,
    temperature: i64,
}

fn conf_path() -> std::path::PathBuf {
    paths::hypr_dir().join("hyprsunset.conf")
}

const MARK: &str = "# Managed by Settings";

fn read_schedule() -> Schedule {
    let text = std::fs::read_to_string(conf_path()).unwrap_or_default();
    let managed = text.starts_with(MARK);
    let mut times = Vec::new();
    let mut temperature = 4000;
    for line in text.lines().map(str::trim).filter(|l| !l.starts_with('#')) {
        if let Some(t) = line.strip_prefix("time =") {
            times.push(t.trim().to_string());
        }
        if let Some(t) = line.strip_prefix("temperature =") {
            temperature = t.trim().parse().unwrap_or(4000);
        }
    }
    Schedule {
        on: managed && store::read(|s| s.autostart.contains_key("hyprsunset")),
        morning: times.first().cloned().filter(|_| managed).unwrap_or("07:00".into()),
        evening: times.get(1).cloned().filter(|_| managed).unwrap_or("20:00".into()),
        temperature,
    }
}

fn write_schedule(s: &Schedule) -> anyhow::Result<()> {
    let path = conf_path();
    if let Ok(old) = std::fs::read_to_string(&path)
        && !old.starts_with(MARK)
    {
        std::fs::write(path.with_extension("conf.before-settings"), old)?;
    }
    let text = if s.on {
        format!(
            "{MARK}. Change it in Settings → Night Light.\n\nprofile {{\n    time = {}\n    identity = true\n}}\n\nprofile {{\n    time = {}\n    temperature = {}\n}}\n",
            s.morning, s.evening, s.temperature
        )
    } else {
        format!("{MARK}. Change it in Settings → Night Light.\n\nprofile {{\n    time = 07:00\n    identity = true\n}}\n")
    };
    cmd::atomic_write(&path, &text)?;
    store::update(true, |st| {
        if s.on {
            st.autostart.insert("hyprsunset".into(), "uwsm-app -- hyprsunset".into());
        } else {
            st.autostart.remove("hyprsunset");
        }
    });
    // Restart so the new schedule is read.
    if cmd::present("omarchy-restart-hyprsunset") {
        cmd::spawn(&["omarchy-restart-hyprsunset"]);
    }
    Ok(())
}

fn valid_time(t: &str) -> bool {
    let parts: Vec<&str> = t.split(':').collect();
    parts.len() == 2
        && parts[0].parse::<u32>().is_ok_and(|h| h < 24)
        && parts[1].len() == 2
        && parts[1].parse::<u32>().is_ok_and(|m| m < 60)
}

/// Omarchy's toggle where it's there; elsewhere hyprsunset itself.
fn omarchy_toggle() -> bool {
    crate::capability::program("omarchy-toggle-nightlight")
}

fn on_now() -> bool {
    if omarchy_toggle() {
        let status = cmd::output(&["omarchy-toggle-nightlight", "--status"])
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
        return status.as_ref().and_then(|v| v.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(false);
    }
    // hyprsunset answers with the temperature it's showing; 6000 K and over is neutral.
    cmd::output(&["hyprctl", "hyprsunset", "temperature"]).and_then(|t| t.trim().parse::<i64>().ok()).is_some_and(|k| k < 6000)
}

fn set_now(on: bool, temperature: i64) {
    if omarchy_toggle() {
        cmd::spawn(&["omarchy-toggle-nightlight"]);
    } else if !on {
        cmd::spawn(&["hyprctl", "hyprsunset", "identity"]);
    } else if cmd::run(&["pgrep", "-x", "hyprsunset"]).is_ok() {
        cmd::spawn(&["hyprctl", "hyprsunset", "temperature", &temperature.to_string()]);
    } else {
        cmd::spawn(&["uwsm", "app", "--", "hyprsunset", "-t", &temperature.to_string()]);
    }
}

/// The colour a white screen takes at `k` kelvin (from candlelight to daylight).
fn kelvin_rgb(k: f64) -> sketch::Rgb {
    const STOPS: [(f64, (f64, f64, f64)); 9] = [
        (2500.0, (255.0, 159.0, 70.0)),
        (3000.0, (255.0, 180.0, 107.0)),
        (3500.0, (255.0, 196.0, 137.0)),
        (4000.0, (255.0, 209.0, 163.0)),
        (4500.0, (255.0, 219.0, 186.0)),
        (5000.0, (255.0, 228.0, 206.0)),
        (5500.0, (255.0, 236.0, 224.0)),
        (6000.0, (255.0, 243.0, 239.0)),
        (6500.0, (255.0, 249.0, 253.0)),
    ];
    let k = k.clamp(2500.0, 6500.0);
    let i = STOPS.iter().position(|(s, _)| *s >= k).unwrap_or(8).max(1);
    let ((k0, a), (k1, b)) = (STOPS[i - 1], STOPS[i]);
    let t = if k1 > k0 { (k - k0) / (k1 - k0) } else { 0.0 };
    sketch::mix((a.0 / 255.0, a.1 / 255.0, a.2 / 255.0), (b.0 / 255.0, b.1 / 255.0, b.2 / 255.0), t)
}

/// Named warmths to pick from, beside the slider.
const PRESETS: [(&str, i64); 4] = [("Candle", 2700), ("Warm", 3400), ("Soft", 4000), ("Light touch", 5000)];

/// The desktop as it is on the left of the line at `split`, and warmed to `k` on its right.
fn draw_compare(cr: &gtk::cairo::Context, p: &sketch::Pal, w: f64, h: f64, k: f64, split: f64) {
    let r = sketch::R::new(1.0, 1.0, w - 2.0, h - 2.0);
    sketch::desktop(cr, p, r);
    let x = r.x + r.w * split;
    cr.save().ok();
    sketch::rounded(cr, r.x, r.y, r.w, r.h, 8.0);
    cr.clip();
    cr.rectangle(x, r.y, r.x + r.w - x, r.h);
    cr.clip();
    let c = kelvin_rgb(k);
    cr.set_operator(gtk::cairo::Operator::Multiply);
    sketch::set(cr, c, 1.0);
    let _ = cr.paint();
    cr.restore().ok();
    // The line to drag, with a handle in the middle.
    sketch::set(cr, (1.0, 1.0, 1.0), 0.9);
    cr.rectangle(x - 1.0, r.y, 2.0, r.h);
    let _ = cr.fill();
    cr.arc(x, r.cy(), 13.0, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
    sketch::set(cr, p.bg, 0.85);
    for dir in [-1.0, 1.0] {
        cr.move_to(x + dir * 3.0, r.cy() - 4.5);
        cr.line_to(x + dir * 7.5, r.cy());
        cr.line_to(x + dir * 3.0, r.cy() + 4.5);
    }
    cr.set_line_width(1.8);
    let _ = cr.stroke();
    for (text, left) in [("Normal".to_string(), true), (format!("{} K", k.round() as i64), false)] {
        tag(cr, p, &text, if left { r.x + 10.0 } else { r.x + r.w - 10.0 }, r.y + r.h - 14.0, left);
    }
}

/// A small label on a dark pill, anchored at its left or right end.
fn tag(cr: &gtk::cairo::Context, _p: &sketch::Pal, text: &str, x: f64, y: f64, from_left: bool) {
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
    cr.set_font_size(11.0);
    let Ok(ext) = cr.text_extents(text) else { return };
    let (pw, ph) = (ext.width() + 16.0, 20.0);
    let px = if from_left { x } else { x - pw };
    sketch::set(cr, (0.0, 0.0, 0.0), 0.55);
    sketch::rounded(cr, px, y - ph / 2.0, pw, ph, ph / 2.0);
    let _ = cr.fill();
    sketch::set(cr, (1.0, 1.0, 1.0), 0.95);
    cr.move_to(px + 8.0 - ext.x_bearing(), y - ext.height() / 2.0 - ext.y_bearing());
    let _ = cr.show_text(text);
}

/// The scale from candlelight to daylight, with `k` marked on it.
fn draw_scale(cr: &gtk::cairo::Context, p: &sketch::Pal, w: f64, h: f64, k: f64) {
    let (y, bh) = (4.0, 12.0);
    let g = gtk::cairo::LinearGradient::new(0.0, 0.0, w, 0.0);
    for i in 0..=8 {
        let kk = 2500.0 + i as f64 * 500.0;
        let c = kelvin_rgb(kk);
        g.add_color_stop_rgb(i as f64 / 8.0, c.0, c.1, c.2);
    }
    sketch::rounded(cr, 0.0, y, w, bh, bh / 2.0);
    let _ = cr.set_source(&g);
    let _ = cr.fill();
    let x = ((k - 2500.0) / 4000.0).clamp(0.0, 1.0) * w;
    sketch::set(cr, p.text, 1.0);
    sketch::rounded(cr, (x - 2.0).min(w - 4.0).max(0.0), y - 4.0, 4.0, bh + 8.0, 2.0);
    let _ = cr.fill();
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
    cr.set_font_size(11.0);
    sketch::set(cr, p.dim, 1.0);
    cr.move_to(0.0, h - 4.0);
    let _ = cr.show_text("2500 K · candlelight");
    if let Ok(ext) = cr.text_extents("6500 K · daylight") {
        cr.move_to(w - ext.width() - ext.x_bearing(), h - 4.0);
        let _ = cr.show_text("6500 K · daylight");
    }
}

/// A tile of a named warmth: its colour, name and kelvin.
fn preset_button(name: &str, k: i64) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.add_css_class("nl-preset");
    let col = widgets::vbox(2);
    let swatch = gtk::DrawingArea::new();
    swatch.set_content_width(28);
    swatch.set_content_height(14);
    swatch.set_halign(gtk::Align::Center);
    swatch.set_draw_func(move |_, cr, w, h| {
        sketch::set(cr, kelvin_rgb(k as f64), 1.0);
        sketch::rounded(cr, 0.0, 0.0, w as f64, h as f64, 7.0);
        let _ = cr.fill();
    });
    col.append(&swatch);
    for (text, class) in [(name.to_string(), "nl-preset-name"), (format!("{k} K"), "nl-preset-k")] {
        let l = widgets::label(&text, class);
        l.set_xalign(0.5);
        col.append(&l);
    }
    b.set_child(Some(&col));
    b.set_hexpand(true);
    b
}

pub fn build(page: &Page) {
    let sched = Rc::new(RefCell::new(read_schedule()));
    let kelvin = Rc::new(Cell::new(sched.borrow().temperature as f64));
    let split_at = Rc::new(Cell::new(0.5));

    // The picture: the desktop as it is, and warmed, either side of a line.
    let pv = page.split();
    let head = pv.head("Before and after", "", &[]);
    let line = widgets::label("", "preview-line");
    line.set_wrap(true);
    if let Some(text) = head.first_child().and_downcast::<gtk::Box>() {
        text.append(&line);
    }
    let describe = {
        let line = line.clone();
        move |k: f64| line.set_text(&format!("Drag the line to compare. The right side is your screen at {} K.", k.round() as i64))
    };
    describe(kelvin.get());
    let compare = {
        let (k, s) = (kelvin.clone(), split_at.clone());
        pv.drawing(320, move |cr, p, _, w, h| draw_compare(cr, p, w, h, k.get(), s.get()))
    };
    compare.add_css_class("nl-compare");
    compare.set_vexpand(true);
    let drag = gtk::GestureDrag::new();
    {
        let (s, c) = (split_at.clone(), compare.clone());
        let start = Rc::new(Cell::new(0.0));
        let st = start.clone();
        drag.connect_drag_begin(move |_, x, _| {
            st.set(x);
            s.set((x / c.width().max(1) as f64).clamp(0.04, 0.96));
            c.queue_draw();
        });
        let (s, c) = (split_at.clone(), compare.clone());
        drag.connect_drag_update(move |_, dx, _| {
            s.set(((start.get() + dx) / c.width().max(1) as f64).clamp(0.04, 0.96));
            c.queue_draw();
        });
    }
    compare.add_controller(drag);
    compare.set_cursor_from_name(Some("col-resize"));
    let scale = {
        let k = kelvin.clone();
        pv.drawing(40, move |cr, p, _, w, h| draw_scale(cr, p, w, h, k.get()))
    };
    pv.foot(
        "weather-clear-night-symbolic",
        if omarchy_toggle() { "Also in the Omarchy menu." } else { "Lower is warmer; 6500 K is neutral." },
    );

    // The controls: right now, then the schedule.
    let g = page.group("Now");
    let warmth = sched.borrow().temperature;
    let sw = gtk::Switch::new();
    sw.set_active(on_now());
    sw.connect_active_notify(move |s| set_now(s.is_active(), warmth));
    g.add(&widgets::icon_row("weather-clear-night-symbolic", "Night Light", "Warm the screen colours right away.", Some(sw.upcast_ref())));

    let temp = sched.borrow().temperature as f64;
    let s2 = sched.clone();
    let pending: Rc<std::cell::Cell<Option<gtk::glib::SourceId>>> = Rc::default();
    let (slider_box, slider) = widgets::slider(2500.0, 6000.0, 100.0, temp, 0, " K");
    slider.scale.set_width_request(140);
    let presets: Rc<RefCell<Vec<(i64, gtk::ToggleButton)>>> = Rc::default();
    let mark_preset = {
        let presets = presets.clone();
        move |k: i64| {
            for (pk, b) in presets.borrow().iter() {
                b.set_active(*pk == k);
            }
        }
    };
    {
        let (kelvin, compare, scale, describe, mark_preset) = (kelvin.clone(), compare.clone(), scale.clone(), describe.clone(), mark_preset.clone());
        slider.scale.connect_value_changed(move |sc| {
            let v = sc.value();
            let k = v.round() as i64;
            kelvin.set(v);
            compare.queue_draw();
            scale.queue_draw();
            describe(v);
            mark_preset(k);
            s2.borrow_mut().temperature = k;
            // Preview immediately when hyprsunset is running.
            cmd::spawn(&["hyprctl", "hyprsunset", "temperature", &k.to_string()]);
            // Save the schedule once the slider settles (it restarts hyprsunset).
            if let Some(id) = pending.take() {
                id.remove();
            }
            let s3 = s2.clone();
            let p2 = pending.clone();
            pending.set(Some(gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
                p2.set(None);
                if s3.borrow().on {
                    let _ = write_schedule(&s3.borrow());
                }
            })));
        });
    }
    g.add(&widgets::icon_row("display-brightness-symbolic", "Warmth", "Lower is warmer.", Some(slider_box.upcast_ref())));
    widgets::keywords("temperature kelvin warm");

    // Named warmths, which set the slider.
    let row = widgets::hbox(8);
    row.add_css_class("nl-presets");
    for (name, k) in PRESETS {
        let b = preset_button(name, k);
        let sc = slider.scale.clone();
        b.connect_clicked(move |b| {
            if b.is_active() {
                sc.set_value(k as f64);
            }
        });
        row.append(&b);
        presets.borrow_mut().push((k, b));
    }
    mark_preset(warmth);
    g.add(&row);

    let g = page.group("Schedule");
    let times = widgets::vbox(0);
    times.set_sensitive(sched.borrow().on);
    let t2 = times.clone();
    let s3 = sched.clone();
    let auto = gtk::Switch::new();
    auto.set_active(sched.borrow().on);
    auto.connect_active_notify(move |sw| {
        let on = sw.is_active();
        s3.borrow_mut().on = on;
        t2.set_sensitive(on);
        if let Err(e) = write_schedule(&s3.borrow()) {
            window::toast(&format!("Couldn't save the schedule: {e}"));
        }
    });
    g.add(&widgets::icon_row(
        "alarm-symbolic",
        "Turn on automatically",
        "Every evening, and back to normal in the morning.",
        Some(auto.upcast_ref()),
    ));

    for (label, evening) in [("Turns on at", true), ("Turns off at", false)] {
        let current = if evening { sched.borrow().evening.clone() } else { sched.borrow().morning.clone() };
        let s = sched.clone();
        let (r, e) = widgets::entry_row(label, "24-hour time, like 20:30.", &current, "HH:MM", move |t| {
            let t = t.trim().to_string();
            if !valid_time(&t) {
                window::toast("Use a 24-hour time like 20:30");
                return;
            }
            if evening {
                s.borrow_mut().evening = t;
            } else {
                s.borrow_mut().morning = t;
            }
            if let Err(e) = write_schedule(&s.borrow()) {
                window::toast(&format!("Couldn't save the schedule: {e}"));
            }
        });
        e.set_width_chars(8);
        e.add_css_class("mono");
        r.prepend(&widgets::tile(if evening { "weather-clear-night-symbolic" } else { "weather-clear-symbolic" }));
        r.add_css_class("with-tile");
        times.append(&r);
    }
    g.add(&times);
}
