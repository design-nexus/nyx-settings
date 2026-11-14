//! Aura: the glow at the edge of the screen while the assistant is busy, beside a
//! small screen that glows as the settings say. Values live in Assistant's validated
//! configuration.

use super::{client, icon_slider, icon_switch, more, save, seg_pick, seg_row, swatch};
use crate::sketch::{self, R};
use crate::widgets::{self, Page};
use crate::{cmd, paths};
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The states Aura shows: (id, name, what it means, theme swatch).
const STATES: [(&str, &str, &str, &str); 4] = [
    ("listening", "Listening", "Your turn to talk", "blue"),
    ("thinking", "Thinking", "Working out an answer", "violet"),
    ("speaking", "Speaking", "Pulses with its voice", "teal"),
    ("waiting", "Waiting for you", "Needs a yes or no", "amber"),
];

/// What the picture shows, shared with the controls that change it.
struct Look {
    name: String,
    on: Cell<bool>,
    edge: Cell<bool>,
    state: Cell<usize>,
    thickness: Cell<f64>,
    intensity: Cell<f64>,
    speed: Cell<f64>,
    length: Cell<f64>,
    persona: Cell<bool>,
    voice: Cell<bool>,
    still: Cell<bool>,
    /// The persona's tint, as Aura blends it in.
    tint: sketch::Rgb,
    started: std::time::Instant,
}

impl Look {
    /// A state's colour as Aura draws it: the theme's swatch, with the persona's tint
    /// blended in when that's on.
    fn colour(&self, state: usize) -> sketch::Rgb {
        let p = sketch::pal();
        let c = swatch(STATES[state].3, p.accent);
        if self.persona.get() { sketch::mix(c, self.tint, 0.18) } else { c }
    }

    fn time(&self) -> f64 {
        if self.still.get() { 0.0 } else { self.started.elapsed().as_secs_f64() }
    }
}

/// The persona's Aura tint (`voices/<persona>.toml`), from the theme's colours.
fn persona_tint(cfg: &Value) -> sketch::Rgb {
    let id = client::get_str(cfg, "assistant.persona_id");
    let tint = std::fs::read_to_string(paths::config_home().join(format!("nyx-assistant/voices/{id}.toml")))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("aura_tint").and_then(|v| v.as_str()).map(str::to_string))
        .unwrap_or_else(|| "accent".into());
    let p = crate::theme::current_palette();
    let hex = match tint.as_str() {
        "highlight" => p.highlight,
        "danger" => p.danger,
        "border" => p.border,
        _ => p.accent,
    };
    crate::fangraph::rgb(&hex)
}

/// Whether Aura holds still: its own setting, or the desktop's.
fn reduced(mode: &str) -> bool {
    match mode {
        "on" => true,
        "off" => false,
        _ => gtk::Settings::default().is_some_and(|s| !s.is_gtk_enable_animations()),
    }
}

/// The glow round the edge of `r`, `reach` deep.
fn full_aura(cr: &gtk::cairo::Context, r: R, c: sketch::Rgb, reach: f64, alpha: f64, radius: f64) {
    cr.save().ok();
    sketch::rounded(cr, r.x, r.y, r.w, r.h, radius);
    cr.clip();
    let steps = (reach / 1.5).ceil().max(2.0) as usize;
    for i in 0..steps {
        let t = i as f64 / steps as f64;
        let d = t * reach;
        sketch::set(cr, c, alpha * (1.0 - t).powf(1.8) * 0.9);
        cr.set_line_width(reach / steps as f64 * 1.6);
        sketch::rounded(cr, r.x + d, r.y + d, r.w - 2.0 * d, r.h - 2.0 * d, (radius - d).max(2.0));
        let _ = cr.stroke();
    }
    sketch::set(cr, sketch::mix(c, (1.0, 1.0, 1.0), 0.25), alpha.min(1.0));
    cr.set_line_width(2.0);
    sketch::rounded(cr, r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h - 2.0, radius);
    let _ = cr.stroke();
    cr.restore().ok();
}

/// Edge Flow: a stretch of light travelling down the left side, along the bottom and
/// up the right side. `at` is where its middle is (0–1 along that path).
fn edge_flow(cr: &gtk::cairo::Context, r: R, c: sketch::Rgb, width: f64, alpha: f64, length: f64, at: f64) {
    let inset = width / 2.0 + 1.0;
    let (l, rt, b, top) = (r.x + inset, r.x + r.w - inset, r.y + r.h - inset, r.y + 2.0);
    let total = 2.0 * (b - top) + (rt - l);
    let point = |s: f64| -> (f64, f64) {
        let s = s.clamp(0.0, total);
        if s < b - top {
            (l, top + s)
        } else if s < b - top + (rt - l) {
            (l + s - (b - top), b)
        } else {
            (rt, b - (s - (b - top) - (rt - l)))
        }
    };
    let len = length * total;
    let start = at * (total + len) - len;
    let n = 60;
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    for (w, a) in [(width * 4.0 + 6.0, 0.18), (width * 2.0 + 2.0, 0.35), (width.max(1.5), 1.0)] {
        cr.new_path();
        let mut first = true;
        for i in 0..=n {
            let s = start + len * i as f64 / n as f64;
            if s < 0.0 || s > total {
                continue;
            }
            let (x, y) = point(s);
            if first {
                cr.move_to(x, y);
                first = false;
            } else {
                cr.line_to(x, y);
            }
        }
        sketch::set(cr, c, alpha * a);
        cr.set_line_width(w);
        let _ = cr.stroke();
    }
    cr.set_line_cap(gtk::cairo::LineCap::Butt);
}

/// How bright the glow is right now: breathing, and following the voice while it speaks.
fn strength(look: &Look) -> f64 {
    let t = look.time() * look.speed.get();
    let breathe = 0.82 + 0.18 * (t * 2.2).sin();
    let voice = if look.voice.get() && look.state.get() == 2 && !look.still.get() {
        0.8 + 0.2 * ((t * 9.0).sin() * (t * 5.3).cos()).abs()
    } else {
        1.0
    };
    (0.4 + 0.6 * look.intensity.get()) * breathe * voice
}

fn draw_screen(cr: &gtk::cairo::Context, p: &sketch::Pal, w: f64, h: f64, look: &Look) {
    use super::{text_at, text_width};
    let r = R::new(1.0, 1.0, w - 2.0, h - 2.0);
    sketch::desktop(cr, p, r);
    let state = look.state.get();
    let c = look.colour(state);
    let radius = (r.w * 0.012).clamp(4.0, 10.0);
    if look.on.get() {
        let thick = look.thickness.get();
        if look.edge.get() {
            let at = (look.time() * 0.12 * look.speed.get()).fract();
            let at = if look.still.get() { 0.45 } else { at };
            edge_flow(cr, r, c, thick * 1.2, strength(look).min(1.0), look.length.get(), at);
        } else {
            full_aura(cr, r, c, 8.0 + thick * 4.0, strength(look).min(1.0), radius);
        }
    }
    // What the assistant is doing, as its pill says.
    let text = match state {
        0 => format!("{} is listening…", look.name),
        1 => format!("{} is thinking…", look.name),
        2 => format!("{} is answering…", look.name),
        _ => format!("{} is waiting for you", look.name),
    };
    let tw = text_width(cr, &text, 12.0, false) + 34.0;
    let (px, py) = (r.cx() - tw / 2.0, r.y + r.h - 46.0);
    sketch::set(cr, (0.0, 0.0, 0.0), 0.5);
    sketch::rounded(cr, px, py, tw, 26.0, 13.0);
    let _ = cr.fill();
    sketch::set(cr, c, 1.0);
    cr.arc(px + 14.0, py + 13.0, 4.0, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
    sketch::set(cr, (1.0, 1.0, 1.0), 0.92);
    text_at(cr, &text, px + 24.0, py + 13.0, 12.0, false, false);
    if !look.on.get() {
        let off = "Aura is off";
        let ow = text_width(cr, off, 12.0, true) + 24.0;
        sketch::set(cr, (0.0, 0.0, 0.0), 0.5);
        sketch::rounded(cr, r.cx() - ow / 2.0, r.y + 22.0, ow, 26.0, 13.0);
        let _ = cr.fill();
        sketch::set(cr, (1.0, 1.0, 1.0), 0.85);
        text_at(cr, off, r.cx() - ow / 2.0 + 12.0, r.y + 35.0, 12.0, true, false);
    }
}

/// A small screen in a style card: the glow alone, in Speaking's colour.
fn draw_style(cr: &gtk::cairo::Context, p: &sketch::Pal, w: f64, h: f64, look: &Look, edge: bool) {
    let r = R::new(1.0, 1.0, w - 2.0, h - 2.0);
    sketch::rounded(cr, r.x, r.y, r.w, r.h, 6.0);
    sketch::set(cr, sketch::mix(p.bg, p.text, 0.05), 1.0);
    let _ = cr.fill();
    let c = look.colour(2);
    if edge {
        edge_flow(cr, r, c, 2.5, 0.95, 0.3, 0.3);
    } else {
        full_aura(cr, r, c, 14.0, 1.0, 6.0);
    }
}

/// The two styles as picture cards.
fn style_cards(look: &Rc<Look>, redraw: Rc<dyn Fn()>, on_style: Rc<dyn Fn(bool)>) -> (gtk::Box, Vec<gtk::DrawingArea>) {
    let row = widgets::hbox(10);
    row.set_homogeneous(true);
    row.add_css_class("asst-styles");
    let mut first: Option<gtk::ToggleButton> = None;
    let mut first_radio: Option<gtk::CheckButton> = None;
    let mut minis = Vec::new();
    for (edge, id, name, desc) in [(false, "full", "Full Aura", "A soft glow all round"), (true, "edge-flow", "Edge Flow", "A light that travels")] {
        let b = gtk::ToggleButton::new();
        b.add_css_class("asst-style");
        let col = widgets::vbox(8);
        let mini = {
            let look = look.clone();
            widgets::drawing(64, move |cr, p, _, w, h| draw_style(cr, p, w, h, &look, edge))
        };
        minis.push(mini.clone());
        col.append(&mini);
        let line = widgets::hbox(8);
        let radio = gtk::CheckButton::new();
        radio.set_can_target(false);
        radio.set_focusable(false);
        match &first_radio {
            Some(f) => radio.set_group(Some(f)),
            None => first_radio = Some(radio.clone()),
        }
        line.append(&radio);
        let text = widgets::vbox(1);
        text.append(&widgets::label(name, "asst-style-name"));
        text.append(&widgets::label(desc, "asst-style-desc"));
        line.append(&text);
        col.append(&line);
        b.set_child(Some(&col));
        match &first {
            Some(f) => b.set_group(Some(f)),
            None => first = Some(b.clone()),
        }
        b.set_active(look.edge.get() == edge);
        radio.set_active(look.edge.get() == edge);
        let (look, redraw, on_style) = (look.clone(), redraw.clone(), on_style.clone());
        b.connect_toggled(move |b| {
            radio.set_active(b.is_active());
            if b.is_active() {
                look.edge.set(edge);
                save("ui.aura_style", client::quoted(id));
                on_style(edge);
                redraw();
            }
        });
        row.append(&b);
    }
    (row, minis)
}

/// Glow in words.
fn glow_word(v: f64) -> &'static str {
    match v {
        v if v < 0.2 => "Faint",
        v if v < 0.5 => "Soft",
        v if v < 0.8 => "Bright",
        _ => "Strong",
    }
}

fn speed_word(v: f64) -> &'static str {
    match v {
        v if v < 0.6 => "Slow",
        v if v < 0.85 => "Calm",
        v if v <= 1.25 => "Normal",
        v if v <= 2.0 => "Quick",
        _ => "Fast",
    }
}

fn length_word(v: f64) -> &'static str {
    match v {
        v if v < 0.1 => "Short",
        v if v < 0.22 => "Medium",
        _ => "Long",
    }
}

const DISPLAYS: &[(&str, &str)] = &[("active", "The screen in use"), ("primary", "The main screen"), ("all", "Every screen")];

/// Connected screens: (connector, "name · connector").
fn monitors() -> Vec<(String, String)> {
    let Ok(text) = cmd::run(&["hyprctl", "monitors", "-j"]) else { return Vec::new() };
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let name = m["name"].as_str()?.to_string();
            if name.starts_with("HEADLESS") {
                return None;
            }
            let desc = m["description"].as_str().unwrap_or("").trim().to_string();
            let label = if desc.is_empty() { name.clone() } else { format!("{desc} · {name}") };
            Some((name, label))
        })
        .collect()
}

fn screens_page(page: &Page, look: &Rc<Look>, redraw: &Rc<dyn Fn()>, value: &Rc<RefCell<Option<gtk::Label>>>) {
    let cfg = client::config();
    let g = page.group("Screens");
    let value = value.clone();
    let (r, _) = widgets::choice_row(
        "Where it shows",
        "Headless displays are skipped.",
        widgets::opts(DISPLAYS),
        &client::get_str(&cfg, "ui.aura_monitor_mode"),
        move |id| {
            save("ui.aura_monitor_mode", client::quoted(&id));
            if let (Some(l), Some((_, name))) = (value.borrow().as_ref(), DISPLAYS.iter().find(|(d, _)| *d == id)) {
                l.set_text(name);
            }
        },
    );
    g.add(&r);
    let mut options = vec![(String::new(), "The one at the top left of the layout".to_string())];
    options.extend(monitors());
    let current = client::get_str(&cfg, "ui.aura_primary_monitor");
    if !options.iter().any(|(id, _)| *id == current) {
        options.push((current.clone(), current.clone()));
    }
    let (r, _) = widgets::choice_row("Main screen", "Used when it shows on the main screen.", options, &current, |id| {
        save("ui.aura_primary_monitor", client::quoted(&id));
    });
    g.add(&r);

    let g = page.group("Colour and motion");
    let (look2, redraw2) = (look.clone(), redraw.clone());
    let (r, _) = widgets::switch_row(
        &format!("{}'s own colour", look.name),
        "Blend the persona's tint into each state's colour.",
        look.persona.get(),
        move |on| {
            save("ui.aura_persona_colors", on.to_string());
            look2.persona.set(on);
            redraw2();
        },
    );
    g.add(&r);
    let current = client::get_str(&cfg, "ui.aura_reduce_motion");
    let (look2, redraw2) = (look.clone(), redraw.clone());
    let (r, _) = widgets::choice_row(
        "Less motion",
        "Holds still instead of breathing and travelling.",
        widgets::opts(&[("system", "Like the system"), ("on", "On"), ("off", "Off")]),
        &current,
        move |id| {
            save("ui.aura_reduce_motion", client::quoted(&id));
            look2.still.set(reduced(&id));
            redraw2();
        },
    );
    g.add(&r);
}

pub fn build(page: &Page) {
    let Some(cfg) = super::config_or_banner(page) else { return };
    let num = |k: &str, d: f64| client::get(&cfg, k).as_f64().unwrap_or(d);
    let flag = |k: &str| client::get(&cfg, k).as_bool().unwrap_or(false);
    let look = Rc::new(Look {
        name: client::get_str(&cfg, "assistant.name"),
        on: Cell::new(flag("ui.aura_enabled")),
        edge: Cell::new(client::get_str(&cfg, "ui.aura_style") == "edge-flow"),
        state: Cell::new(2),
        thickness: Cell::new(num("ui.aura_thickness", 3.0)),
        intensity: Cell::new(num("ui.aura_intensity", 0.4)),
        speed: Cell::new(num("ui.aura_speed", 1.0)),
        length: Cell::new(num("ui.aura_segment_length", 0.14)),
        persona: Cell::new(flag("ui.aura_persona_colors")),
        voice: Cell::new(flag("ui.aura_audio_reactive")),
        still: Cell::new(reduced(&client::get_str(&cfg, "ui.aura_reduce_motion"))),
        tint: persona_tint(&cfg),
        started: std::time::Instant::now(),
    });

    // The picture: a screen glowing as the settings say, and the four states.
    let pv = page.split();
    let pickers: Rc<RefCell<(Option<gtk::Box>, Vec<gtk::ToggleButton>)>> = Rc::default();
    let screen_slot: Rc<RefCell<Option<gtk::DrawingArea>>> = Rc::default();
    let swatches: Rc<RefCell<Vec<gtk::DrawingArea>>> = Rc::default();
    let pick_state: Rc<dyn Fn(usize)> = {
        let (look, pickers, screen_slot) = (look.clone(), pickers.clone(), screen_slot.clone());
        Rc::new(move |i: usize| {
            if look.state.get() == i {
                return;
            }
            look.state.set(i);
            let (seg, cards) = pickers.borrow().clone();
            if let Some(seg) = seg {
                seg_pick(&seg, i);
            }
            if let Some(b) = cards.get(i) {
                b.set_active(true);
            }
            if let Some(a) = screen_slot.borrow().as_ref() {
                a.queue_draw();
            }
        })
    };
    let seg = {
        let pick = pick_state.clone();
        let opts: Vec<(&str, &str)> = vec![("0", "Listening"), ("1", "Thinking"), ("2", "Speaking"), ("3", "Waiting")];
        widgets::segmented(&widgets::opts(&opts), "2", move |id| pick(id.parse().unwrap_or(2)))
    };
    pickers.borrow_mut().0 = Some(seg.clone());
    pv.head("On your screen", "Click a state to see how it looks.", &[seg.upcast_ref()]);
    let screen = {
        let look = look.clone();
        pv.drawing(320, move |cr, p, _, w, h| draw_screen(cr, p, w, h, &look))
    };
    super::screen_shaped(&screen, 0.6);
    *screen_slot.borrow_mut() = Some(screen.clone());
    // It breathes and travels while it's on screen.
    {
        let weak = screen.downgrade();
        let look = look.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            let Some(a) = weak.upgrade() else { return glib::ControlFlow::Break };
            if a.is_mapped() && look.on.get() && !look.still.get() {
                a.queue_draw();
            }
            glib::ControlFlow::Continue
        });
    }
    let legend = widgets::hbox(8);
    legend.set_homogeneous(true);
    legend.add_css_class("asst-legend");
    let mut first: Option<gtk::ToggleButton> = None;
    let mut cards = Vec::new();
    for (i, (_, name, desc, _)) in STATES.iter().enumerate() {
        let b = gtk::ToggleButton::new();
        b.add_css_class("asst-state");
        let line = widgets::hbox(8);
        let dot = {
            let look = look.clone();
            widgets::drawing(14, move |cr, _, _, w, h| {
                let c = look.colour(i);
                sketch::set(cr, c, 1.0);
                cr.arc(w / 2.0, h / 2.0, w.min(h) / 2.0 - 0.5, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            })
        };
        dot.set_content_width(14);
        dot.set_hexpand(false);
        dot.set_valign(gtk::Align::Center);
        swatches.borrow_mut().push(dot.clone());
        line.append(&dot);
        let text = widgets::vbox(0);
        let n = widgets::label(name, "asst-state-name");
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&n);
        let d = widgets::label(desc, "asst-state-desc");
        d.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&d);
        text.set_hexpand(true);
        line.append(&text);
        b.set_child(Some(&line));
        b.set_tooltip_text(Some(&format!("{name}: {desc}")));
        match &first {
            Some(f) => b.set_group(Some(f)),
            None => first = Some(b.clone()),
        }
        b.set_active(i == 2);
        let pick = pick_state.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                pick(i);
            }
        });
        legend.append(&b);
        cards.push(b);
    }
    pickers.borrow_mut().1 = cards;
    pv.add(&legend);
    let foot = pv.foot(
        "preferences-color-symbolic",
        if look.persona.get() { "Colours come from your theme, tinted with Echo's own." } else { "Colours come from your theme." },
    );
    let minis: Rc<RefCell<Vec<gtk::DrawingArea>>> = Rc::default();
    let redraw: Rc<dyn Fn()> = {
        let (screen, swatches, minis, look, foot) = (screen.clone(), swatches.clone(), minis.clone(), look.clone(), foot.clone());
        Rc::new(move || {
            screen.queue_draw();
            for a in swatches.borrow().iter().chain(minis.borrow().iter()) {
                a.queue_draw();
            }
            foot.set_text(&if look.persona.get() {
                format!("Colours come from your theme, tinted with {}'s own.", look.name)
            } else {
                "Colours come from your theme.".to_string()
            });
        })
    };
    redraw();

    // The controls: on or off, the style, how it looks, and when and where.
    let g = page.group("");
    let (r, _) = {
        let (look, redraw) = (look.clone(), redraw.clone());
        icon_switch(
            &cfg,
            "ui.aura_enabled",
            "focus-windows-symbolic",
            "Aura",
            &format!("Shows that {} is listening, thinking or talking.", look.name),
            move |on| {
                look.on.set(on);
                redraw();
            },
        )
    };
    r.add_css_class("asst-lead");
    g.add(&r);
    widgets::keywords("assistant appearance aura border glow");

    let length_row: Rc<RefCell<Option<gtk::Box>>> = Rc::default();
    let speed_title: Rc<RefCell<Option<gtk::Label>>> = Rc::default();
    let on_style: Rc<dyn Fn(bool)> = {
        let (length_row, speed_title) = (length_row.clone(), speed_title.clone());
        Rc::new(move |edge: bool| {
            if let Some(r) = length_row.borrow().as_ref() {
                r.set_visible(edge);
            }
            if let Some(t) = speed_title.borrow().as_ref() {
                t.set_text(if edge { "Travel speed" } else { "Breathing speed" });
            }
        })
    };
    let g = page.plain_group("Style");
    let (cards, style_minis) = style_cards(&look, redraw.clone(), on_style.clone());
    minis.borrow_mut().extend(style_minis);
    g.add(&cards);
    widgets::keywords("style full aura edge flow");

    let g = page.group("Look");
    let r = {
        let (look, redraw) = (look.clone(), redraw.clone());
        icon_slider(
            &cfg,
            "ui.aura_thickness",
            "format-justify-fill-symbolic",
            "Thickness",
            "",
            (1.0, 10.0, 1.0),
            0,
            " px",
            move |v, _| {
                look.thickness.set(v);
                redraw();
            },
        )
    };
    g.add(&r);
    widgets::keywords("thickness width reach line");
    let r = {
        let (look, redraw) = (look.clone(), redraw.clone());
        icon_slider(&cfg, "ui.aura_intensity", "display-brightness-symbolic", "Glow", "", (0.0, 1.0, 0.05), 2, "", move |v, l| {
            l.set_text(glow_word(v));
            l.set_tooltip_text(Some(&format!("{v:.2}")));
            look.intensity.set(v);
            redraw();
        })
    };
    g.add(&r);
    widgets::keywords("glow intensity brightness subtle");
    let r = {
        let (look, redraw) = (look.clone(), redraw.clone());
        icon_slider(&cfg, "ui.aura_speed", "view-refresh-symbolic", "Breathing speed", "", (0.25, 3.0, 0.05), 2, "×", move |v, l| {
            l.set_text(speed_word(v));
            l.set_tooltip_text(Some(&format!("{v:.2}×")));
            look.speed.set(v);
            redraw();
        })
    };
    *speed_title.borrow_mut() = super::row_title(&r);
    g.add(&r);
    widgets::keywords("animation speed breathing travel");
    let r = {
        let (look, redraw) = (look.clone(), redraw.clone());
        icon_slider(
            &cfg,
            "ui.aura_segment_length",
            "view-continuous-symbolic",
            "Length",
            "How much of the edge is lit.",
            (0.04, 0.4, 0.01),
            2,
            "",
            move |v, l| {
                l.set_text(length_word(v));
                l.set_tooltip_text(Some(&format!("{v:.2}")));
                look.length.set(v);
                redraw();
            },
        )
    };
    *length_row.borrow_mut() = Some(r.clone());
    g.add(&r);
    widgets::keywords("flow length segment");
    on_style(look.edge.get());

    let g = page.group("When and where");
    let visibility = client::get_str(&cfg, "ui.aura_visibility");
    let (r, _) = seg_row(
        "alarm-symbolic",
        "Between replies",
        "",
        &[("persistent", "Stay faint"), ("activity", "Hide")],
        &visibility,
        |id| save("ui.aura_visibility", client::quoted(&id)),
    );
    g.add(&r);
    widgets::keywords("visibility persistent activity conversation faint hide");
    let (r, _) = {
        let (look, redraw) = (look.clone(), redraw.clone());
        icon_switch(&cfg, "ui.aura_audio_reactive", "audio-volume-high-symbolic", "Move with its voice", "", move |on| {
            look.voice.set(on);
            redraw();
        })
    };
    g.add(&r);
    widgets::keywords("react voice audio reactive microphone");
    let mode = client::get_str(&cfg, "ui.aura_monitor_mode");
    let mode = DISPLAYS.iter().find(|(id, _)| *id == mode).map(|(_, l)| *l).unwrap_or("The screen in use");
    let value: Rc<RefCell<Option<gtk::Label>>> = Rc::default();
    let (look2, redraw2, value2) = (look.clone(), redraw.clone(), value.clone());
    let (r, label) = widgets::nav_row(Some("video-display-symbolic"), "Screens and motion", "", mode, move || {
        let (look, redraw, value) = (look2.clone(), redraw2.clone(), value2.clone());
        more("Screens and motion", move |page, _| screens_page(page, &look, &redraw, &value));
    });
    *value.borrow_mut() = Some(label);
    widgets::keywords("display monitor primary output all screens motion reduce persona colours colors tint");
    g.add(&r);
}
