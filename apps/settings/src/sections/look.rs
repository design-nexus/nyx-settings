//! Window Style: the everyday sliders beside one large preview that follows them,
//! presets to start from, and the finer details on pages of their own.

use crate::backend::{hypr, store};
use crate::sketch::Sketch;
use crate::widgets::{self, Page, hypr_slider, hypr_switch, sketched};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

/// The first `gtk::Scale` inside `w`.
fn find_scale(w: &gtk::Widget) -> Option<gtk::Scale> {
    if let Some(s) = w.downcast_ref::<gtk::Scale>() {
        return Some(s.clone());
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(s) = find_scale(&child) {
            return Some(s);
        }
        c = child.next_sibling();
    }
    None
}

fn find_switch(w: &gtk::Widget) -> Option<gtk::Switch> {
    if let Some(s) = w.downcast_ref::<gtk::Switch>() {
        return Some(s.clone());
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(s) = find_switch(&child) {
            return Some(s);
        }
        c = child.next_sibling();
    }
    None
}

/// A slider for a Hyprland option that also follows the option when a preset sets it.
fn slider(key: &'static str, title: &str, desc: &str, range: (f64, f64, f64), unit: &str, integer: bool) -> gtk::Box {
    let r = hypr_slider(key, title, desc, range, if integer { 0 } else { 2 }, unit, integer);
    if let Some(scale) = find_scale(r.upcast_ref()) {
        scale.set_width_request(100);
        store::subscribe(&r, key, move |k, v| {
            if k != key {
                return;
            }
            if let Some(n) = v.and_then(Value::as_f64)
                && (scale.value() - n).abs() > 0.001
            {
                scale.set_value(n);
            }
        });
    }
    r
}

/// Opacity as a percentage, on a slider that stores 0.5–1.
fn percent_slider(key: &'static str, title: &str, desc: &str) -> gtk::Box {
    let r = slider(key, title, desc, (0.5, 1.0, 0.01), "", false);
    if let Some(scale) = find_scale(r.upcast_ref())
        && let Some(readout) = scale.next_sibling().and_downcast::<gtk::Label>()
    {
        let show = move |s: &gtk::Scale| readout.set_text(&format!("{}%", (s.value() * 100.0).round() as i64));
        show(&scale);
        scale.connect_value_changed(show);
    }
    r
}

/// A switch for a Hyprland option that also follows it when a preset sets it.
fn switch(key: &'static str, title: &str, desc: &str) -> gtk::Box {
    let r = hypr_switch(key, title, desc);
    if let Some(sw) = find_switch(r.upcast_ref()) {
        store::subscribe(&r, key, move |k, v| {
            if k == key
                && let Some(on) = v.and_then(Value::as_bool)
                && sw.is_active() != on
            {
                sw.set_active(on);
            }
        });
    }
    r
}

/// [`switch`] with a drawing of what it does.
fn pictured(key: &'static str, title: &str, desc: &str, s: Sketch) -> gtk::Box {
    let r = switch(key, title, desc);
    sketched(&r, s);
    r
}

/// Starting points: gaps, border, rounding and shadow (and for Glass, see-through windows).
const PRESETS: &[(&str, &[(&str, f64)])] = &[
    ("Omarchy", &[("general.gaps_in", 5.0), ("general.gaps_out", 10.0), ("general.border_size", 2.0), ("decoration.rounding", 0.0), ("decoration.shadow.range", 0.0)]),
    ("Tight", &[("general.gaps_in", 2.0), ("general.gaps_out", 2.0), ("general.border_size", 1.0), ("decoration.rounding", 0.0), ("decoration.shadow.range", 0.0)]),
    ("Airy", &[("general.gaps_in", 16.0), ("general.gaps_out", 24.0), ("general.border_size", 2.0), ("decoration.rounding", 8.0), ("decoration.shadow.range", 14.0)]),
    ("Soft", &[("general.gaps_in", 10.0), ("general.gaps_out", 14.0), ("general.border_size", 2.0), ("decoration.rounding", 10.0), ("decoration.shadow.range", 18.0)]),
    (
        "Glass",
        &[
            ("general.gaps_in", 10.0),
            ("general.gaps_out", 14.0),
            ("general.border_size", 1.0),
            ("decoration.rounding", 12.0),
            ("decoration.shadow.range", 22.0),
            ("decoration.inactive_opacity", 0.85),
        ],
    ),
    ("Flat", &[("general.gaps_in", 8.0), ("general.gaps_out", 8.0), ("general.border_size", 0.0), ("decoration.rounding", 4.0), ("decoration.shadow.range", 0.0)]),
];

fn value_now(key: &str) -> Option<f64> {
    let v = store::option(key).or_else(|| hypr::get_option(key))?;
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if b { 1.0 } else { 0.0 }),
        Value::String(s) => s.split_whitespace().next()?.parse().ok(),
        _ => None,
    }
}

fn apply_preset(values: &[(&str, f64)]) {
    for (key, v) in values {
        if *key == "decoration.shadow.range" {
            store::set_option("decoration.shadow.enabled", json!(*v > 0.0));
            if *v <= 0.0 {
                continue;
            }
        }
        if *key == "decoration.inactive_opacity" {
            store::set_option("decoration.blur.enabled", json!(true));
            store::set_option(key, json!(v));
        } else {
            store::set_option(key, json!(v.round() as i64));
        }
    }
}

fn preset_matches(values: &[(&str, f64)]) -> bool {
    values.iter().all(|(key, v)| {
        if *key == "decoration.shadow.range" {
            let on = value_now("decoration.shadow.enabled").unwrap_or(0.0) > 0.5;
            return if *v <= 0.0 { !on } else { on && value_now(key).is_some_and(|n| (n - v).abs() < 0.5) };
        }
        value_now(key).is_some_and(|n| (n - v).abs() < 0.01)
    })
}

pub fn build(page: &Page) {
    let pv = page.split();

    // Undo puts back what was set when the page opened.
    let before: Vec<(&'static str, Option<Value>)> = crate::preview::STYLE_KEYS.iter().map(|k| (*k, store::option(k))).collect();
    let undo = gtk::Button::from_icon_name("edit-undo-symbolic");
    undo.add_css_class("flat");
    undo.set_sensitive(false);
    undo.set_tooltip_text(Some("Undo: put back the window style from when you opened this page"));
    undo.connect_clicked(move |b| {
        b.set_sensitive(false);
        let mut reload = false;
        for (key, old) in &before {
            match old {
                Some(v) if store::option(key).as_ref() != Some(v) => store::set_option(key, v.clone()),
                None if store::is_managed(key) => {
                    store::reset_option(key);
                    reload = true;
                }
                _ => {}
            }
        }
        if reload {
            // Hyprland reloads first, so the page reads the user's own values.
            glib::timeout_add_local_once(std::time::Duration::from_millis(800), || window::rebuild("look"));
        }
    });
    let head = pv.head("Live preview", "The focused window is on the left. It changes as you drag.", &[undo.upcast_ref()]);
    if let Some(title) = head.first_child().and_then(|t| t.first_child()).and_downcast::<gtk::Label>() {
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    }
    let stage = crate::preview::window_style_stage();
    stage.add_css_class("lk-stage");
    super::theme::fit_height(&stage, 1.15, 400);
    pv.add(&stage);

    let presets = widgets::hbox(6);
    presets.add_css_class("lk-presets");
    let chips: Rc<RefCell<Vec<(usize, gtk::Button)>>> = Rc::default();
    let mark = {
        let chips = chips.clone();
        move || {
            for (i, b) in chips.borrow().iter() {
                if preset_matches(PRESETS[*i].1) {
                    b.add_css_class("selected");
                } else {
                    b.remove_css_class("selected");
                }
            }
        }
    };
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_max_children_per_line(7);
    flow.set_min_children_per_line(1);
    flow.set_column_spacing(6);
    flow.set_row_spacing(6);
    flow.set_halign(gtk::Align::Start);
    flow.set_valign(gtk::Align::Center);
    let label = widgets::label("Start from", "lk-presets-label");
    label.set_valign(gtk::Align::Center);
    flow.insert(&label, -1);
    for (i, (name, values)) in PRESETS.iter().enumerate() {
        let b = gtk::Button::with_label(name);
        b.add_css_class("chip");
        b.connect_clicked(move |_| apply_preset(values));
        flow.insert(&b, -1);
        if let Some(c) = flow.last_child() {
            c.set_focusable(false);
        }
        chips.borrow_mut().push((i, b));
    }
    presets.append(&flow);
    pv.add(&presets);
    mark();
    for key in crate::preview::STYLE_KEYS {
        let (mark, undo) = (mark.clone(), undo.clone());
        // Listeners hear of a change just before it's stored: read the values after.
        store::subscribe(&presets, key, move |_, _| {
            undo.set_sensitive(true);
            let mark = mark.clone();
            glib::idle_add_local_once(move || mark());
        });
    }

    let g = page.group("Spacing");
    g.add(&slider("general.gaps_in", "Inner gaps", "Between windows", (0.0, 40.0, 1.0), " px", true));
    g.add(&slider("general.gaps_out", "Outer gaps", "At the screen's edge", (0.0, 60.0, 1.0), " px", true));
    widgets::keywords("margin padding space between neighboring windows screen edge");

    let g = page.group("Borders & corners");
    g.add(&slider("general.border_size", "Border width", "", (0.0, 10.0, 1.0), " px", true));
    widgets::keywords("outline drawn around every window");
    g.add(&slider("decoration.rounding", "Corner rounding", "", (0.0, 24.0, 1.0), " px", true));
    widgets::keywords("radius round corners");
    g.add(&pictured("general.resize_on_border", "Resize from borders", "Drag an edge to resize", Sketch::ResizeOnBorder));

    let g = page.group("Glass & shadow");
    g.add(&percent_slider("decoration.inactive_opacity", "Unfocused windows", "Opacity"));
    widgets::keywords("transparency transparent see-through opacity unfocused window");
    g.add(&switch("decoration.blur.enabled", "Blur behind windows", "Frosted, so text stays readable"));
    g.add(&switch("decoration.shadow.enabled", "Shadows", "Lift windows off the wallpaper"));
    widgets::keywords("window shadows drop shadow");

    let g = page.group("");
    let (r, _) = widgets::nav_row(Some("preferences-other-symbolic"), "Blur, shadow and cursor details", "", "", || {
        let d = widgets::subpage("Blur, shadow and cursor details", 620, details);
        widgets::forget_rows(&d);
    });
    widgets::keywords(
        "focused window opacity dim unfocused windows dim amount blur size passes vibrancy saturation shadow size falloff cursor size hide cursor after typing inactive timeout pointer",
    );
    g.add(&r);
    let animations = value_now("animations.enabled").is_none_or(|v| v > 0.5);
    let (r, value) = widgets::nav_row(Some("media-playback-start-symbolic"), "Animations", "", if animations { "On" } else { "Off" }, || {
        let d = widgets::subpage("Animations", 560, |p| {
            let g = p.group("Animations");
            g.add(&pictured("animations.enabled", "Animations", "Windows and workspaces slide and fade as they open, move and switch.", Sketch::Animations));
            g.add(&hypr_switch("misc.animate_manual_resizes", "Animate resizing", "Smooth out keyboard and mouse resizes."));
        });
        widgets::forget_rows(&d);
    });
    store::subscribe(&r, "animations.enabled", move |_, v| {
        if let Some(on) = v.and_then(Value::as_bool) {
            value.set_text(if on { "On" } else { "Off" });
        }
    });
    widgets::keywords("animations slide fade motion animate resizing");
    g.add(&r);
}

/// The finer settings: transparency, blur, shadows and the cursor.
fn details(page: &Page) {
    let g = page.group("Transparency");
    g.add(&percent_slider("decoration.active_opacity", "Focused window", ""));
    g.add(&percent_slider("decoration.inactive_opacity", "Unfocused windows", ""));
    g.add(&pictured("decoration.dim_inactive", "Dim unfocused windows", "So the one you're using stands out.", Sketch::Dim));
    g.add(&slider("decoration.dim_strength", "Dim amount", "", (0.0, 0.8, 0.01), "", false));

    let g = page.group("Blur");
    g.note("Blur shows behind see-through windows, the bar and menus. It costs some GPU time.");
    g.add(&pictured("decoration.blur.enabled", "Blur", "What's behind a see-through window looks frosted, so its text stays readable.", Sketch::Blur));
    g.add(&slider("decoration.blur.size", "Blur size", "", (1.0, 20.0, 1.0), "", true));
    g.add(&slider("decoration.blur.passes", "Blur passes", "More passes look smoother.", (1.0, 6.0, 1.0), "", true));
    g.add(&slider("decoration.blur.vibrancy", "Vibrancy", "How colourful the blur stays.", (0.0, 1.0, 0.01), "", false));

    let g = page.group("Shadows");
    g.add(&pictured("decoration.shadow.enabled", "Window shadows", "A soft shadow lifts each window off the wallpaper.", Sketch::Shadow));
    g.add(&slider("decoration.shadow.range", "Shadow size", "", (0.0, 50.0, 1.0), " px", true));
    g.add(&slider("decoration.shadow.render_power", "Shadow falloff", "Higher is a sharper edge.", (1.0, 4.0, 1.0), "", true));

    let g = page.group("Cursor");
    let size = store::read(|s| s.env.get("XCURSOR_SIZE").cloned())
        .or_else(|| std::env::var("XCURSOR_SIZE").ok())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(24.0);
    let (r, _) = widgets::slider_row("Cursor size", "", (16.0, 64.0, 4.0), size, 0, " px", |v| {
        let n = (v.round() as i64).to_string();
        store::set_env("XCURSOR_SIZE", Some(n.clone()));
        store::set_env("HYPRCURSOR_SIZE", Some(n.clone()));
        let theme = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "Adwaita".into());
        cmd::spawn(&["hyprctl", "setcursor", &theme, &n]);
    });
    g.add(&r);
    g.add(&hypr_slider("cursor.inactive_timeout", "Hide cursor after", "Seconds without movement. 0 never hides it.", (0.0, 30.0, 1.0), 0, " s", false));
    g.add(&hypr_switch("cursor.hide_on_key_press", "Hide cursor while typing", ""));
}
