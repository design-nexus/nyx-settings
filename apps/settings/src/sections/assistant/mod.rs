//! Nyx Assistant: its settings, what it's allowed to do, what it's doing and its models.
//! Every value is read and written through `nyx-assistant config`, which checks it.

pub mod activity;
pub mod appearance;
pub mod client;
pub mod general;
pub mod models;
pub mod performance;
pub mod permissions;
pub mod voice;

use crate::widgets::{self, Debounce, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;

/// Save one setting off the main thread; say why if the assistant refused it.
pub fn save(key: &str, value: String) {
    let key = key.to_string();
    cmd::background(
        move || client::set(&key, &value),
        |r| {
            if let Err(e) = r {
                window::toast(&format!("Couldn't save: {e}"));
            }
        },
    );
}

/// The settings, or a banner saying why they couldn't be read (the page then stops).
pub fn config_or_banner(page: &Page) -> Option<Value> {
    let cfg = client::config();
    if cfg.is_object() {
        return Some(cfg);
    }
    page.banner("Couldn't read the assistant's settings. Update Nyx Assistant (./install.sh assistant) and try again.", true);
    None
}

pub fn switch(cfg: &Value, key: &'static str, title: &str, desc: &str) -> gtk::Box {
    let on = client::get(cfg, key).as_bool().unwrap_or(false);
    widgets::switch_row(title, desc, on, move |on| save(key, on.to_string())).0
}

pub fn choice(cfg: &Value, key: &'static str, title: &str, desc: &str, options: &[(&str, &str)]) -> gtk::Box {
    let options: Vec<(String, String)> = options.iter().map(|(id, l)| (id.to_string(), l.to_string())).collect();
    let current = client::get_str(cfg, key);
    widgets::choice_row(title, desc, options, &current, move |id| save(key, client::quoted(&id))).0
}

/// A number setting; saved once the slider rests.
pub fn slider(
    cfg: &Value,
    key: &'static str,
    title: &str,
    desc: &str,
    range: (f64, f64, f64),
    digits: u32,
    unit: &str,
) -> gtk::Box {
    let value = client::get(cfg, key).as_f64().unwrap_or(range.0);
    let wait = Debounce::default();
    widgets::slider_row(title, desc, range, value, digits, unit, move |v| {
        let text = if digits == 0 { format!("{}", v.round() as i64) } else { format!("{:.*}", digits as usize, v) };
        wait.call(400, move || save(key, text));
    })
    .0
}

/// "5.3 GB", "640 MB".
pub fn size(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", bytes as f64 / 1e6) }
}

/// "14:03:22" today, else "Oct 12 14:03".
pub fn when(ms: u64) -> String {
    let Some(t) = glib::DateTime::from_unix_local((ms / 1000) as i64).ok() else { return String::new() };
    let today = glib::DateTime::now_local().ok().map(|n| n.ymd()) == Some(t.ymd());
    t.format(if today { "%H:%M:%S" } else { "%b %e %H:%M" }).map(|s| s.to_string()).unwrap_or_default()
}

/// A dim one-line label.
pub fn dim(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("dim");
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l
}

// ---------- Parts the redesigned pages share ----------

/// [`switch`] with an icon tile; `then` hears each change too (for a preview).
pub fn icon_switch(
    cfg: &Value,
    key: &'static str,
    icon: &str,
    title: &str,
    desc: &str,
    then: impl Fn(bool) + 'static,
) -> (gtk::Box, gtk::Switch) {
    let sw = gtk::Switch::new();
    sw.set_active(client::get(cfg, key).as_bool().unwrap_or(false));
    sw.connect_active_notify(move |s| {
        save(key, s.is_active().to_string());
        then(s.is_active());
    });
    (widgets::icon_row(icon, title, desc, Some(sw.upcast_ref())), sw)
}

/// [`slider`] with an icon tile and a shorter track; `then` hears each move (for a
/// preview, or to put the readout in words).
#[allow(clippy::too_many_arguments)]
pub fn icon_slider(
    cfg: &Value,
    key: &'static str,
    icon: &str,
    title: &str,
    desc: &str,
    range: (f64, f64, f64),
    digits: u32,
    unit: &str,
    then: impl Fn(f64, &gtk::Label) + 'static,
) -> gtk::Box {
    let value = client::get(cfg, key).as_f64().unwrap_or(range.0);
    let (b, s) = widgets::slider(range.0, range.1, range.2, value, digits, unit);
    s.scale.set_width_request(120);
    s.readout.set_width_chars(6);
    then(value, &s.readout);
    let wait = Debounce::default();
    let readout = s.readout.clone();
    s.scale.connect_value_changed(move |sc| {
        let v = sc.value();
        then(v, &readout);
        let text = if digits == 0 { format!("{}", v.round() as i64) } else { format!("{:.*}", digits as usize, v) };
        wait.call(400, move || save(key, text));
    });
    widgets::icon_row(icon, title, desc, Some(b.upcast_ref()))
}

/// A row's text column (its title and description), to put more under them.
pub fn row_text(row: &gtk::Box) -> Option<gtk::Box> {
    let mut c = row.first_child();
    while let Some(w) = c {
        if w.has_css_class("row-text") {
            return w.downcast::<gtk::Box>().ok();
        }
        c = w.next_sibling();
    }
    None
}

/// A row's title label.
pub fn row_title(row: &gtk::Box) -> Option<gtk::Label> {
    row_text(row)?.first_child().and_downcast::<gtk::Label>()
}

/// Buttons joined into one control in a row with an icon tile.
pub fn seg_row(
    icon: &str,
    title: &str,
    desc: &str,
    options: &[(&str, &str)],
    current: &str,
    on_change: impl Fn(String) + 'static,
) -> (gtk::Box, gtk::Box) {
    let seg = widgets::segmented(&widgets::opts(options), current, on_change);
    (widgets::icon_row(icon, title, desc, Some(seg.upcast_ref())), seg)
}

/// Pick the `index`th button of a segmented control (its handler runs as for a click).
pub fn seg_pick(seg: &gtk::Box, index: usize) {
    let mut c = seg.first_child();
    let mut i = 0;
    while let Some(w) = c {
        if i == index
            && let Some(b) = w.downcast_ref::<gtk::ToggleButton>()
        {
            b.set_active(true);
        }
        i += 1;
        c = w.next_sibling();
    }
}

/// A small rounded label: a word it answers to, a key word, a state.
pub fn chip(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("asst-chip");
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l.set_valign(gtk::Align::Center);
    l
}

/// A row of chips that wraps.
pub fn chip_flow() -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_column_spacing(4);
    f.set_row_spacing(4);
    f.set_max_children_per_line(30);
    f.set_homogeneous(false);
    f.add_css_class("asst-chips");
    f
}

/// Add to a [`chip_flow`] without each child taking keyboard focus.
pub fn flow_add(f: &gtk::FlowBox, w: &impl IsA<gtk::Widget>) {
    f.append(w);
    if let Some(child) = f.last_child() {
        child.set_focusable(false);
    }
}

/// The assistant's glowing orb, in the section's colour; grey while it's stopped.
pub fn orb(size: i32, lit: bool) -> gtk::DrawingArea {
    let a = widgets::drawing(size, move |cr, p, c, w, h| draw_orb(cr, p, c, w / 2.0, h / 2.0, w.min(h) / 2.0 - 1.0, lit));
    a.set_content_width(size);
    a.set_hexpand(false);
    a.set_valign(gtk::Align::Center);
    a.add_css_class("asst-orb");
    a
}

pub fn draw_orb(cr: &gtk::cairo::Context, p: &crate::sketch::Pal, c: crate::sketch::Rgb, x: f64, y: f64, r: f64, lit: bool) {
    use crate::sketch::{mix, set};
    let c = if lit { c } else { mix(c, p.dim, 0.75) };
    if lit {
        let glow = gtk::cairo::RadialGradient::new(x, y, r * 0.6, x, y, r * 1.0);
        glow.add_color_stop_rgba(0.0, c.0, c.1, c.2, 0.35);
        glow.add_color_stop_rgba(1.0, c.0, c.1, c.2, 0.0);
        let _ = cr.set_source(&glow);
        cr.arc(x, y, r, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
    let rr = r * 0.82;
    let g = gtk::cairo::RadialGradient::new(x - rr * 0.35, y - rr * 0.4, rr * 0.05, x, y, rr);
    let hi = mix(c, (1.0, 1.0, 1.0), 0.65);
    let lo = mix(c, p.bg, 0.45);
    g.add_color_stop_rgb(0.0, hi.0, hi.1, hi.2);
    g.add_color_stop_rgb(0.55, c.0, c.1, c.2);
    g.add_color_stop_rgb(1.0, lo.0, lo.1, lo.2);
    let _ = cr.set_source(&g);
    cr.arc(x, y, rr, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
    set(cr, (1.0, 1.0, 1.0), 0.18);
    cr.set_line_width(1.0);
    cr.arc(x, y, rr - 0.5, 0.0, std::f64::consts::TAU);
    let _ = cr.stroke();
}

/// Keep a drawing as tall as `ratio` of its width, like a screen; flatter in a narrow
/// window, where the preview sits above the controls and shouldn't push them away.
pub fn screen_shaped(area: &gtk::DrawingArea, ratio: f64) {
    fn narrow(w: &gtk::Widget) -> bool {
        let mut p = w.parent();
        while let Some(x) = p {
            if x.has_css_class("narrow") {
                return true;
            }
            p = x.parent();
        }
        false
    }
    area.connect_resize(move |a, w, _| {
        let ratio = if narrow(a.upcast_ref()) { ratio * 0.6 } else { ratio };
        let want = ((w as f64) * ratio).clamp(200.0, 400.0) as i32;
        if a.content_height() != want {
            let a = a.clone();
            glib::idle_add_local_once(move || a.set_content_height(want));
        }
    });
}

/// Text in a drawing, `size` px, with its left edge at `x` and its middle at `y`
/// (or its right edge at `x` when `right`). Returns its width.
pub fn text_at(cr: &gtk::cairo::Context, text: &str, x: f64, y: f64, size: f64, bold: bool, right: bool) -> f64 {
    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        if bold { gtk::cairo::FontWeight::Bold } else { gtk::cairo::FontWeight::Normal },
    );
    cr.set_font_size(size);
    let Ok(ext) = cr.text_extents(text) else { return 0.0 };
    let left = if right { x - ext.x_advance() } else { x };
    cr.move_to(left, y - ext.height() / 2.0 - ext.y_bearing());
    let _ = cr.show_text(text);
    ext.x_advance()
}

/// How wide `text` is at `size` px.
pub fn text_width(cr: &gtk::cairo::Context, text: &str, size: f64, bold: bool) -> f64 {
    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        if bold { gtk::cairo::FontWeight::Bold } else { gtk::cairo::FontWeight::Normal },
    );
    cr.set_font_size(size);
    cr.text_extents(text).map(|e| e.x_advance()).unwrap_or(0.0)
}

/// A theme swatch ("green", "amber"…) as Cairo wants it, or `fallback`.
pub fn swatch(name: &str, fallback: crate::sketch::Rgb) -> crate::sketch::Rgb {
    crate::theme::swatches().into_iter().find(|(n, _)| n == name).map(|(_, c)| crate::fangraph::rgb(&c)).unwrap_or(fallback)
}

/// Run `fill` again after a change.
pub type Again = std::rc::Rc<dyn Fn()>;

/// A page one level down whose contents `fill` builds, and builds again (in place)
/// when it calls the [`Again`] it's given. Its rows stay out of search: the row that
/// opens it carries their words.
pub fn more(title: &str, fill: impl Fn(&Page, &Again) + 'static) {
    let fill: std::rc::Rc<dyn Fn(&Page, &Again)> = std::rc::Rc::new(fill);
    let win = widgets::subpage(title, 560, |page| {
        let holder = widgets::vbox(0);
        page.body.append(&holder);
        refill(&holder, fill.clone());
    });
    widgets::forget_rows(&win);
}

fn refill(holder: &gtk::Box, fill: std::rc::Rc<dyn Fn(&Page, &Again)>) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let weak = holder.downgrade();
    let f = fill.clone();
    let again: Again = std::rc::Rc::new(move || {
        if let Some(h) = weak.upgrade() {
            refill(&h, f.clone());
        }
    });
    let page = Page::new(gtk::ScrolledWindow::new(), holder.clone());
    fill(&page, &again);
    widgets::forget_rows(holder);
}

/// Run `nyx-assistant ARGS…` off the main thread, then `after` (whether it worked or not).
pub fn run_then(args: Vec<String>, failed: &'static str, after: impl Fn() + 'static) {
    cmd::background(
        move || client::run(&args.iter().map(String::as_str).collect::<Vec<_>>()),
        move |r| {
            if let Err(e) = r {
                window::toast(&format!("{failed}: {e}"));
            }
            cmd::forget_ahead();
            after();
        },
    );
}
