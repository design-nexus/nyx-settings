//! Bar (Nyx Bar): its size, the status icons as one pill or separate, the clock (date
//! style, 12/24-hour, seconds, the calendar's layout) and the workspaces' look, beside
//! the bar drawn at its real size from its real layout. Read through every layer (NyxOS's
//! defaults, then the user's file); written into `~/.config/nyx/bar.toml` with
//! `toml_edit`, keeping the layout and comments the bar wrote there. The bar follows the
//! file as it changes.

use super::barlayout::{Glyph, glyph, now};
use super::nyxbarlayout::{DIVIDER, Entry, OTHERS};
use crate::sketch::{self, mix, rounded, set};
use crate::widgets::{self, Page};
use crate::window;
use gtk::prelude::*;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use toml_edit::{Array, DocumentMut, Item, Value};

fn path() -> PathBuf {
    gtk::glib::user_config_dir().join("nyx/bar.toml")
}

pub(crate) fn read_user() -> DocumentMut {
    std::fs::read_to_string(path()).ok().and_then(|t| t.parse().ok()).unwrap_or_default()
}

pub(crate) fn write_user(doc: &DocumentMut) {
    let p = path();
    let r = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(p.parent().unwrap_or(&p))?;
        let tmp = p.with_extension("toml.new");
        std::fs::write(&tmp, doc.to_string())?;
        std::fs::rename(&tmp, &p)
    })();
    if let Err(e) = r {
        window::toast(&format!("Couldn't save: {e}"));
    }
}

/// Size: one of the presets; an exact `height` would win over it, so it goes (its
/// comment stays, above the size).
pub fn set_size(doc: &mut DocumentMut, size: &str) {
    let comment = doc.as_table().key("height").and_then(|k| k.leaf_decor().prefix()).cloned();
    doc.remove("height");
    let had = doc.contains_key("size");
    doc["size"] = toml_edit::value(size);
    if !had
        && let (Some(c), Some(mut k)) = (comment, doc.as_table_mut().key_mut("size"))
    {
        k.leaf_decor_mut().set_prefix(c);
    }
}

/// One plugin's setting, in `[plugins."<id>"]`.
pub fn set_plugin(doc: &mut DocumentMut, plugin: &str, key: &str, value: impl Into<Value>) {
    if !doc.contains_table("plugins") {
        let mut t = toml_edit::Table::new();
        t.set_implicit(true);
        doc["plugins"] = Item::Table(t);
    }
    let plugins = doc["plugins"].as_table_mut().expect("plugins is a table");
    if !plugins.contains_table(plugin) {
        plugins.insert(plugin, Item::Table(toml_edit::Table::new()));
    }
    plugins[plugin][key] = toml_edit::value(value);
}

/// Where the status items sit in the right section: grouped (`["nyx.system"]`) or not,
/// or `None` when the layout doesn't place them there.
pub fn status_grouped(doc: &DocumentMut) -> Option<bool> {
    let right = doc.get("right")?.as_array()?;
    right.iter().find_map(|v| match v {
        Value::String(s) if s.value() == "nyx.system" => Some(false),
        Value::Array(a) if a.iter().any(|x| x.as_str() == Some("nyx.system")) => Some(true),
        _ => None,
    })
}

/// The status items as one pill or as separate items, in place in the right section.
pub fn set_status_grouped(doc: &mut DocumentMut, grouped: bool) -> bool {
    let Some(right) = doc.get_mut("right").and_then(|r| r.as_array_mut()) else { return false };
    let Some(i) = right.iter().position(|v| match v {
        Value::String(s) => s.value() == "nyx.system",
        Value::Array(a) => a.len() == 1 && a.iter().any(|x| x.as_str() == Some("nyx.system")),
        _ => false,
    }) else {
        return false;
    };
    let decor = right.get(i).map(|v| v.decor().clone()).unwrap_or_default();
    let mut new: Value = if grouped {
        let mut a = Array::new();
        a.push("nyx.system");
        Value::Array(a)
    } else {
        Value::from("nyx.system")
    };
    *new.decor_mut() = decor;
    right.replace(i, new);
    true
}

// ----- What the preview draws -----

/// The settings the bar in the picture follows.
#[derive(Clone)]
struct Look {
    size: String,
    date: String,
    hours: String,
    seconds: bool,
    calendar: String,
    workspaces: String,
}

fn size_px(size: &str, height: Option<i64>) -> i32 {
    match size {
        "compact" => 28,
        "cozy" => 36,
        "spacious" => 40,
        "custom" => height.unwrap_or(32) as i32,
        _ => 32,
    }
}

/// The date as the bar shows it, for today.
fn date_text(style: &str) -> String {
    now(match style {
        "long" => "%A, %B %-d",
        "numeric" => "%-m/%-d",
        "iso" => "%Y-%m-%d",
        "none" => "",
        _ => "%a %b %-d",
    })
}

fn time_text(hours: &str, seconds: bool) -> String {
    // Automatic follows the region: a 12-hour clock where the locale writes "AM"/"PM".
    let twelve = match hours {
        "12" => true,
        "24" => false,
        _ => !now("%p").is_empty(),
    };
    now(match (twelve, seconds) {
        (true, true) => "%-I:%M:%S %p",
        (true, false) => "%-I:%M %p",
        (false, true) => "%H:%M:%S",
        (false, false) => "%H:%M",
    })
}

/// The workspaces in one of their five looks.
fn draw_workspaces(cr: &gtk::cairo::Context, p: &sketch::Pal, look: &str, x: f64, cy: f64, h: f64) -> f64 {
    let apps = [p.accent, mix(p.accent, (0.6, 0.85, 0.4), 0.6), mix(p.accent, p.danger, 0.5)];
    let num = |cr: &gtk::cairo::Context, n: &str, x: f64, on: bool| {
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
        cr.set_font_size(h * 0.42);
        if let Ok(e) = cr.text_extents(n) {
            set(cr, if on { p.text } else { p.dim }, 1.0);
            cr.move_to(x - e.width() / 2.0 - e.x_bearing(), cy - e.height() / 2.0 - e.y_bearing());
            let _ = cr.show_text(n);
        }
    };
    let s = h * 0.32;
    match look {
        "numbers" | "underline" => {
            for i in 0..4 {
                let cx = x + s + i as f64 * h * 0.62;
                let on = i == 1;
                if on && look == "numbers" {
                    set(cr, p.accent, 1.0);
                    cr.arc(cx, cy, h * 0.27, 0.0, std::f64::consts::TAU);
                    let _ = cr.fill();
                }
                num(cr, &(i + 1).to_string(), cx, on);
                if on && look == "underline" {
                    set(cr, p.accent, 1.0);
                    rounded(cr, cx - s * 0.7, cy + h * 0.26, s * 1.4, 2.0, 1.0);
                    let _ = cr.fill();
                }
            }
            x + s + 4.0 * h * 0.62
        }
        "dots" => {
            let mut cx = x + 4.0;
            for i in 0..4 {
                let w = if i == 1 { s * 2.2 } else { s * 0.7 };
                set(cr, if i == 1 { p.accent } else { p.dim }, if i == 1 { 1.0 } else { 0.7 });
                rounded(cr, cx, cy - s * 0.35, w, s * 0.7, s * 0.35);
                let _ = cr.fill();
                cx += w + s * 0.6;
            }
            cx
        }
        "icons" => {
            let mut cx = x + 2.0;
            for (i, n) in [1usize, 2, 1].iter().enumerate() {
                let w = *n as f64 * (s + 3.0) + 6.0;
                if i == 1 {
                    set(cr, p.accent, 0.25);
                    rounded(cr, cx, cy - h * 0.3, w, h * 0.6, h * 0.15);
                    let _ = cr.fill();
                }
                for j in 0..*n {
                    set(cr, apps[(i + j) % 3], 1.0);
                    rounded(cr, cx + 3.0 + j as f64 * (s + 3.0), cy - s / 2.0, s, s, 2.5);
                    let _ = cr.fill();
                }
                cx += w + 4.0;
            }
            cx
        }
        _ => {
            // Pills: the current one holds its windows' icons.
            let mut cx = x;
            for i in 0..4 {
                let w = if i == 1 { 3.0 * (s + 3.0) + 8.0 } else { h * 0.5 };
                set(cr, if i == 1 { p.accent } else { mix(p.surface, p.text, 0.08) }, if i == 1 { 0.3 } else { 1.0 });
                rounded(cr, cx, cy - h * 0.3, w, h * 0.6, h * 0.3);
                let _ = cr.fill();
                if i == 1 {
                    for (j, c) in apps.iter().enumerate() {
                        set(cr, *c, 1.0);
                        rounded(cr, cx + 4.0 + j as f64 * (s + 3.0), cy - s / 2.0, s, s, 2.5);
                        let _ = cr.fill();
                    }
                } else {
                    num(cr, &(i + 1).to_string(), cx + w / 2.0, false);
                }
                cx += w + 3.0;
            }
            cx
        }
    }
}

/// One of the workspaces' looks, drawn small, to pick.
fn workspace_cards(current: &str, on_change: impl Fn(String) + 'static) -> gtk::Box {
    let row = widgets::hbox(6);
    row.set_homogeneous(true);
    row.add_css_class("nb-looks");
    let on_change = Rc::new(on_change);
    let mut first: Option<gtk::ToggleButton> = None;
    for (id, name) in [("pills", "Pills"), ("numbers", "Numbers"), ("icons", "App icons"), ("dots", "Dots"), ("underline", "Underline")] {
        let b = gtk::ToggleButton::new();
        b.add_css_class("nb-look");
        let v = widgets::vbox(4);
        let d = gtk::DrawingArea::new();
        d.set_content_height(24);
        d.set_hexpand(true);
        d.add_css_class("nb-look-art");
        d.set_draw_func(move |_, cr, w, h| {
            let p = sketch::pal();
            // Measure, then draw centred, smaller when the card is narrow.
            let (w, h) = (w as f64, h as f64);
            let end = {
                let rec = gtk::cairo::RecordingSurface::create(gtk::cairo::Content::ColorAlpha, None).ok();
                rec.and_then(|r| gtk::cairo::Context::new(&r).ok()).map(|c| draw_workspaces(&c, &p, id, 0.0, h / 2.0, h)).unwrap_or(w)
            };
            let k = ((w - 6.0) / end).min(1.0);
            cr.save().ok();
            cr.translate((w - end * k) / 2.0, h / 2.0 * (1.0 - k));
            cr.scale(k, k);
            draw_workspaces(cr, &p, id, 0.0, h / 2.0, h);
            cr.restore().ok();
        });
        v.append(&d);
        v.append(&widgets::label(name, "nb-look-name").tap_centre());
        b.set_child(Some(&v));
        b.set_tooltip_text(Some(name));
        match &first {
            Some(g) => b.set_group(Some(g)),
            None => first = Some(b.clone()),
        }
        b.set_active(id == current);
        let f = on_change.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                f(id.to_string());
            }
        });
        row.append(&b);
    }
    row
}

trait Centre {
    fn tap_centre(self) -> Self;
}
impl Centre for gtk::Label {
    fn tap_centre(self) -> Self {
        self.set_xalign(0.5);
        self
    }
}

/// An item of the bar as it shows, from what Settings knows of it.
fn item_widget(id: &str, name: &str, look: &Look, grouped_status: bool) -> Option<gtk::Widget> {
    let tail = id.rsplit(['.', '/']).next().unwrap_or(id);
    let w: gtk::Widget = match tail {
        // Shown only now and then (while recording, while the microphone is in use).
        "privacy" | "recording" => return None,
        "workspaces" => {
            let look = look.clone();
            let d = gtk::DrawingArea::new();
            d.set_content_width(96);
            d.set_draw_func(move |a, cr, _, h| {
                let end = draw_workspaces(cr, &sketch::pal(), &look.workspaces, 2.0, h as f64 / 2.0, h as f64);
                let want = (end + 4.0).ceil() as i32;
                if a.content_width() != want {
                    let a = a.clone();
                    gtk::glib::idle_add_local_once(move || a.set_content_width(want));
                }
            });
            d.upcast()
        }
        "clock" | "time" => {
            let b = widgets::hbox(6);
            let date = date_text(&look.date);
            if !date.is_empty() {
                b.append(&widgets::label(&date, "nb-date"));
            }
            b.append(&widgets::label(&time_text(&look.hours, look.seconds), "nb-time"));
            b.upcast()
        }
        "system" => {
            let b = widgets::hbox(if grouped_status { 7 } else { 12 });
            for icon in ["network-wireless-symbolic", "bluetooth-active-symbolic", "audio-volume-high-symbolic", "battery-good-symbolic"] {
                let i = gtk::Image::from_icon_name(icon);
                i.set_pixel_size(14);
                b.append(&i);
            }
            b.upcast()
        }
        _ => super::barlayout::glyph_widget(glyph(id).or(Some(Glyph::Icon("application-x-addon-symbolic"))), name, "", false),
    };
    w.set_tooltip_text(Some(name));
    Some(w)
}

/// The bar from its layout: pills, dividers and items, in three sections.
fn fill_bar(bar: &gtk::Box, info: &super::nyxbarlayout::Info, look: &Look, grouped_status: bool) {
    while let Some(c) = bar.first_child() {
        bar.remove(&c);
    }
    for (i, section) in info.layout().iter().enumerate() {
        let part = widgets::hbox(6);
        part.add_css_class("nb-part");
        for e in section {
            if !info.is_on(e) {
                continue;
            }
            match e {
                Entry::One(x) if x == DIVIDER => {
                    let d = gtk::Separator::new(gtk::Orientation::Vertical);
                    d.add_css_class("nb-divider");
                    part.append(&d);
                }
                Entry::One(x) if x == OTHERS => {}
                Entry::One(x) => {
                    if let Some(w) = item_widget(x, &info.name(x), look, false) {
                        w.add_css_class("nb-item");
                        part.append(&w);
                    }
                }
                Entry::Group(g) => {
                    let pill = widgets::hbox(8);
                    pill.add_css_class("nb-pill");
                    if g.iter().any(|x| x.contains("clock")) {
                        pill.add_css_class("clock");
                    }
                    for x in g {
                        if let Some(w) = item_widget(x, &info.name(x), look, grouped_status) {
                            pill.append(&w);
                        }
                    }
                    if pill.first_child().is_some() {
                        part.append(&pill);
                    }
                }
            }
        }
        match i {
            0 => part.set_halign(gtk::Align::Start),
            1 => {
                part.set_hexpand(true);
                part.set_halign(gtk::Align::Center);
            }
            _ => part.set_halign(gtk::Align::End),
        }
        bar.append(&part);
    }
}

/// The month under the clock, as a click on it opens it; with the agenda's place beside it.
fn draw_calendar(cr: &gtk::cairo::Context, p: &sketch::Pal, w: f64, h: f64, wide: bool) {
    let card_w = if wide { w.min(330.0) } else { w.min(210.0) };
    let x0 = (w - card_w) / 2.0;
    rounded(cr, x0, 0.5, card_w, h - 1.0, 10.0);
    set(cr, mix(p.surface, p.text, 0.04), 0.97);
    let _ = cr.fill_preserve();
    set(cr, p.dim, 0.3);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    let Ok(today) = gtk::glib::DateTime::now_local() else { return };
    let month_w = 200.0_f64.min(card_w - 10.0);
    let (mx, cell) = (x0 + 12.0, (month_w - 24.0) / 7.0);
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
    cr.set_font_size(11.0);
    set(cr, p.text, 0.95);
    cr.move_to(mx, 20.0);
    let _ = cr.show_text(&now("%B %Y"));
    cr.set_font_size(9.0);
    for (i, d) in ["M", "T", "W", "T", "F", "S", "S"].iter().enumerate() {
        set(cr, p.dim, 0.8);
        cr.move_to(mx + i as f64 * cell + cell / 2.0 - 3.0, 38.0);
        let _ = cr.show_text(d);
    }
    let first = gtk::glib::DateTime::from_local(today.year(), today.month(), 1, 0, 0, 0.0).ok();
    let offset = first.map(|f| f.day_of_week() - 1).unwrap_or(0);
    let days = (28..=31)
        .rev()
        .find(|d| gtk::glib::DateTime::from_local(today.year(), today.month(), *d, 0, 0, 0.0).is_ok_and(|t| t.month() == today.month()))
        .unwrap_or(30);
    cr.set_font_size(10.0);
    for d in 1..=days {
        let i = offset + d - 1;
        let (col, row) = (i % 7, i / 7);
        let (cx, cy) = (mx + col as f64 * cell + cell / 2.0, 54.0 + row as f64 * 18.0);
        if cy > h - 6.0 {
            break;
        }
        if d == today.day_of_month() {
            set(cr, p.accent, 1.0);
            cr.arc(cx, cy - 3.5, 8.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
        let s = d.to_string();
        if let Ok(e) = cr.text_extents(&s) {
            set(cr, if d == today.day_of_month() { p.bg } else { p.text }, 0.9);
            cr.move_to(cx - e.width() / 2.0 - e.x_bearing(), cy);
            let _ = cr.show_text(&s);
        }
    }
    if wide {
        let ax = x0 + month_w + 4.0;
        set(cr, p.dim, 0.25);
        cr.rectangle(ax, 10.0, 1.0, h - 20.0);
        let _ = cr.fill();
        cr.set_font_size(9.0);
        set(cr, p.dim, 0.9);
        cr.move_to(ax + 10.0, 22.0);
        let _ = cr.show_text("TODAY");
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
        cr.set_font_size(9.5);
        set(cr, p.dim, 0.9);
        for (i, line) in ["Your events", "and reminders", "show here."].iter().enumerate() {
            cr.move_to(ax + 10.0, 40.0 + i as f64 * 13.0);
            let _ = cr.show_text(line);
        }
    }
}

pub fn build(page: &Page) {
    let merged = nyx_config::Layers::standard().merged("bar").unwrap_or_default();
    let plugin = |id: &str, key: &str, d: &str| {
        merged.get("plugins").and_then(|p| p.get(id)).and_then(|p| p.get(key)).and_then(|v| v.as_str()).unwrap_or(d).to_string()
    };
    let opts = |o: &[(&str, &str)]| o.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
    let height = merged.get("height").and_then(|v| v.as_integer());
    let size = if height.is_some() { "custom".to_string() } else { merged.get("size").and_then(|v| v.as_str()).unwrap_or("default").to_string() };
    let look = Rc::new(RefCell::new(Look {
        size: size.clone(),
        date: plugin("nyx.clock", "date", "short"),
        hours: plugin("nyx.clock", "hours", "auto"),
        seconds: merged.get("plugins").and_then(|p| p.get("nyx.clock")).and_then(|p| p.get("seconds")).and_then(|v| v.as_bool()).unwrap_or(false),
        calendar: plugin("nyx.clock", "layout", "wide"),
        workspaces: plugin("nyx.workspaces", "look", "pills"),
    }));

    // The layout in use, through every layer: the user's file may set none of it.
    let info = super::nyxbarlayout::read_info();
    let in_use = || {
        let mut doc = read_user();
        if status_grouped(&doc).is_none()
            && let Some(info) = super::nyxbarlayout::read_info()
        {
            super::nyxbarlayout::write_sections(&mut doc, &info.layout());
        }
        doc
    };
    let mut shown = read_user();
    if status_grouped(&shown).is_none()
        && let Some(info) = &info
    {
        super::nyxbarlayout::write_sections(&mut shown, &info.layout());
    }
    let grouped = Rc::new(std::cell::Cell::new(status_grouped(&shown).unwrap_or(true)));

    // ----- The preview: the bar at its real size over the wallpaper -----
    let pv = page.split();
    let head = pv.head("Your bar, at full size", "Hover a part to see what it is.", &[]);
    if let Some(title) = head.first_child().and_then(|t| t.first_child()).and_downcast::<gtk::Label>() {
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    }
    let bar = widgets::hbox(10);
    bar.add_css_class("nb-bar");
    bar.set_valign(gtk::Align::Start);
    let bar_css = gtk::CssProvider::new();
    #[allow(deprecated)]
    bar.style_context().add_provider(&bar_css, gtk::STYLE_PROVIDER_PRIORITY_USER);
    let wall = gtk::DrawingArea::new();
    wall.set_hexpand(true);
    wall.set_draw_func(|_, cr, w, h| {
        let p = sketch::pal();
        let (w, h) = (w as f64, h as f64);
        rounded(cr, 0.5, 0.5, w - 1.0, h - 1.0, 10.0);
        cr.clip();
        let g = gtk::cairo::LinearGradient::new(0.0, 0.0, w, h);
        let (a, b) = (mix(p.bg, p.accent, 0.14), mix(p.bg, (0.0, 0.0, 0.0), 0.15));
        g.add_color_stop_rgb(0.0, a.0, a.1, a.2);
        g.add_color_stop_rgb(1.0, b.0, b.1, b.2);
        let _ = cr.set_source(&g);
        let _ = cr.paint();
        for (cx, cy, r, c) in [(0.85, 0.7, 0.32, p.danger), (0.2, 1.0, 0.22, mix(p.accent, (0.3, 0.8, 0.6), 0.6))] {
            let rg = gtk::cairo::RadialGradient::new(w * cx, h * cy, 0.0, w * cx, h * cy, w * r);
            rg.add_color_stop_rgba(0.0, c.0, c.1, c.2, 0.6);
            rg.add_color_stop_rgba(0.75, c.0, c.1, c.2, 0.35);
            rg.add_color_stop_rgba(1.0, c.0, c.1, c.2, 0.0);
            let _ = cr.set_source(&rg);
            let _ = cr.paint();
        }
    });
    super::theme::fit_height(&wall, 1.25, 380);
    let calendar = gtk::DrawingArea::new();
    calendar.set_content_height(170);
    calendar.set_valign(gtk::Align::Start);
    calendar.set_halign(gtk::Align::Fill);
    {
        let look = look.clone();
        calendar.set_draw_func(move |_, cr, w, h| draw_calendar(cr, &sketch::pal(), w as f64, h as f64, look.borrow().calendar == "wide"));
    }
    // A bar wider than the preview scrolls sideways, fading at the edge.
    let bar_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .child(&bar)
        .valign(gtk::Align::Start)
        .build();
    bar_scroll.add_css_class("nb-bar-scroll");
    let fade = gtk::DrawingArea::new();
    fade.set_content_width(36);
    fade.set_halign(gtk::Align::End);
    fade.set_valign(gtk::Align::Start);
    fade.set_can_target(false);
    {
        let (bar, scroll) = (bar.clone(), bar_scroll.clone());
        fade.set_draw_func(move |_, cr, w, h| {
            let adj = scroll.hadjustment();
            if adj.upper() - adj.page_size() - adj.value() < 2.0 {
                return;
            }
            let p = sketch::pal();
            let g = gtk::cairo::LinearGradient::new(0.0, 0.0, w as f64, 0.0);
            g.add_color_stop_rgba(0.0, p.bg.0, p.bg.1, p.bg.2, 0.0);
            g.add_color_stop_rgba(1.0, p.bg.0, p.bg.1, p.bg.2, 0.95);
            cr.rectangle(0.0, 0.0, w as f64, (bar.height() + 8) as f64);
            let _ = cr.set_source(&g);
            let _ = cr.fill();
            let _ = h;
        });
        let f = fade.clone();
        bar_scroll.hadjustment().connect_changed(move |_| f.queue_draw());
        let f = fade.clone();
        bar_scroll.hadjustment().connect_value_changed(move |_| f.queue_draw());
    }
    let stage = gtk::Overlay::new();
    stage.add_css_class("nb-stage");
    stage.set_child(Some(&wall));
    stage.add_overlay(&bar_scroll);
    stage.add_overlay(&fade);
    stage.add_overlay(&calendar);
    stage.set_overflow(gtk::Overflow::Hidden);
    pv.add(&stage);

    // The four sizes side by side.
    let sizes = widgets::vbox(6);
    sizes.add_css_class("nb-sizes");
    sizes.append(&widgets::label("Sizes", "nb-sizes-label"));
    let blocks = widgets::hbox(8);
    blocks.set_halign(gtk::Align::Start);
    let size_marks: Rc<RefCell<Vec<(&'static str, gtk::Box)>>> = Rc::default();
    for (id, px) in [("compact", 28), ("default", 32), ("cozy", 36), ("spacious", 40)] {
        let col = widgets::vbox(3);
        col.set_valign(gtk::Align::End);
        let block = widgets::hbox(0);
        block.add_css_class("nb-size");
        block.set_size_request(28, (px as f64 * 0.75) as i32);
        block.set_halign(gtk::Align::Center);
        col.append(&block);
        col.append(&widgets::label(&px.to_string(), "nb-size-px").tap_centre());
        blocks.append(&col);
        size_marks.borrow_mut().push((id, block));
    }
    let line = widgets::hbox(14);
    line.append(&blocks);
    let tip = widgets::label("32 px fits most screens; 36 or 40 suits a big screen seen from further away.", "nb-sizes-tip");
    tip.set_wrap(true);
    tip.set_hexpand(true);
    tip.set_valign(gtk::Align::Center);
    tip.set_width_chars(1);
    line.append(&tip);
    sizes.append(&line);
    pv.add(&sizes);

    let redraw: Rc<dyn Fn()> = {
        let (bar, calendar, look, grouped, info, size_marks, fade) =
            (bar.clone(), calendar.clone(), look.clone(), grouped.clone(), info.clone(), size_marks.clone(), fade.clone());
        Rc::new(move || {
            let l = look.borrow().clone();
            let px = size_px(&l.size, height);
            bar_css.load_from_string(&format!(".nb-bar {{ min-height: {px}px; font-size: {}px; }}", (px as f64 * 0.4).round()));
            fade.set_content_height(px + 8);
            for (id, b) in size_marks.borrow().iter() {
                if *id == l.size {
                    b.add_css_class("on");
                } else {
                    b.remove_css_class("on");
                }
            }
            calendar.set_margin_top(px + 6);
            calendar.queue_draw();
            if let Some(info) = &info {
                fill_bar(&bar, info, &l, grouped.get());
            }
        })
    };
    redraw();

    // ----- The controls -----
    let g = page.group("Bar");
    let seg_opts = opts(&[("compact", "28"), ("default", "32"), ("cozy", "36"), ("spacious", "40 px")]);
    let seg = {
        let (look, redraw) = (look.clone(), redraw.clone());
        widgets::segmented(&seg_opts, &size, move |v| {
            look.borrow_mut().size = v.clone();
            redraw();
            let mut doc = read_user();
            set_size(&mut doc, &v);
            write_user(&doc);
        })
    };
    let r = widgets::row("Size", "Height, text and icons", Some(seg.upcast_ref()));
    widgets::keywords("bar top panel height size compact default cozy spacious");
    g.add(&r);
    if let Some(h) = height {
        g.note(&format!("Your own height, {h} px, is in use; picking a size replaces it."));
    }
    // A size block in the preview picks that size too.
    for (i, (_, block)) in size_marks.borrow().iter().enumerate() {
        let seg = seg.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            let mut c = seg.first_child();
            let mut n = 0;
            while let Some(child) = c {
                if n == i
                    && let Some(t) = child.downcast_ref::<gtk::ToggleButton>()
                {
                    t.set_active(true);
                }
                n += 1;
                c = child.next_sibling();
            }
        });
        block.add_controller(click);
        block.set_cursor_from_name(Some("pointer"));
    }

    match status_grouped(&shown) {
        Some(on) => {
            let (grouped, redraw) = (grouped.clone(), redraw.clone());
            let (r, _) = widgets::switch_row("Status icons in one pill", "Opens Control Center", on, move |on| {
                let mut doc = in_use();
                if set_status_grouped(&mut doc, on) {
                    write_user(&doc);
                    grouped.set(on);
                    redraw();
                }
            });
            widgets::keywords("network bluetooth sound battery together each on its own");
            g.add(&r);
        }
        None => g.note("The status icons aren't in the bar's right section, so they can't be grouped here."),
    }

    let g = page.group("Clock");
    let date_opts: Vec<(String, String)> = ["short", "long", "numeric", "iso", "none"]
        .iter()
        .map(|id| (id.to_string(), if *id == "none" { "No date".to_string() } else { date_text(id) }))
        .collect();
    let (r, _) = {
        let (look, redraw) = (look.clone(), redraw.clone());
        widgets::choice_row("Date", "", date_opts, &plugin("nyx.clock", "date", "short"), move |v| {
            look.borrow_mut().date = v.clone();
            redraw();
            let mut doc = read_user();
            set_plugin(&mut doc, "nyx.clock", "date", v.as_str());
            write_user(&doc);
        })
    };
    widgets::keywords("clock calendar date beside the time");
    g.add(&r);
    let seg = {
        let (look, redraw) = (look.clone(), redraw.clone());
        widgets::segmented(&opts(&[("auto", "Automatic"), ("12", "12-hour"), ("24", "24-hour")]), &plugin("nyx.clock", "hours", "auto"), move |v| {
            look.borrow_mut().hours = v.clone();
            redraw();
            let mut doc = read_user();
            set_plugin(&mut doc, "nyx.clock", "hours", v.as_str());
            write_user(&doc);
        })
    };
    g.add(&widgets::row("Time", "", Some(seg.upcast_ref())));
    widgets::keywords("12-hour 24-hour military am pm automatic region");
    let seconds = look.borrow().seconds;
    let (r, _) = {
        let (look, redraw) = (look.clone(), redraw.clone());
        widgets::switch_row("Seconds", "", seconds, move |on| {
            look.borrow_mut().seconds = on;
            redraw();
            let mut doc = read_user();
            set_plugin(&mut doc, "nyx.clock", "seconds", on);
            write_user(&doc);
        })
    };
    g.add(&r);
    let own = plugin("nyx.clock", "format", "");
    if !own.trim().is_empty() {
        g.note(&format!(
            "Your own clock format, <tt>{}</tt>, is in use in place of these; take it out of the clock's settings to use them.",
            gtk::glib::markup_escape_text(&own)
        ));
    }
    let (r, _) = {
        let (look, redraw) = (look.clone(), redraw.clone());
        widgets::choice_row(
            "A click opens",
            "",
            opts(&[("wide", "Month and agenda"), ("compact", "Month only")]),
            &plugin("nyx.clock", "layout", "wide"),
            move |v| {
                look.borrow_mut().calendar = v.clone();
                redraw();
                let mut doc = read_user();
                set_plugin(&mut doc, "nyx.clock", "layout", v.as_str());
                write_user(&doc);
            },
        )
    };
    widgets::keywords("calendar month agenda events reminders");
    g.add(&r);

    let g = page.plain_group("Workspaces");
    let cards = {
        let (look, redraw) = (look.clone(), redraw.clone());
        workspace_cards(&plugin("nyx.workspaces", "look", "pills"), move |v| {
            look.borrow_mut().workspaces = v.clone();
            redraw();
            let mut doc = read_user();
            set_plugin(&mut doc, "nyx.workspaces", "look", v.as_str());
            write_user(&doc);
        })
    };
    let r = widgets::stacked_row("", "", cards.upcast_ref());
    r.add_css_class("bare");
    widgets::keywords("workspaces look pills numbers app icons dots underline");
    g.add(&r);

    let g = page.group("");
    let count = info.as_ref().map(|i| i.layout().iter().flatten().filter(|e| e.is_item()).count()).unwrap_or(0);
    let user_sets = info.as_ref().is_some_and(|i| i.user_sets_layout);
    let (r, _) = widgets::nav_row(
        Some("view-grid-symbolic"),
        "Arrangement",
        "What sits where, pills and dividers",
        &if count == 0 { String::new() } else { format!("{count} items") },
        move || {
            let d = widgets::subpage("Arrangement", 640, move |p| arrangement(p, user_sets));
            widgets::forget_rows(&d);
        },
    );
    widgets::keywords("arrange order move left center right section divider pill other plugins reset items");
    g.add(&r);
}

/// The arrangement editor, one level down.
fn arrangement(page: &Page, user_sets_layout: bool) {
    let g = page.plain_group("Arrangement");
    g.note("Drag an item to reorder it or move it to another section, or use the arrows and the ⋯ menu. A pill is items sharing one background; Other plugins is where plugins the layout doesn't place go.");
    g.add(&super::nyxbarlayout::editor());
    g.note("Bar Plugins turns items on and off.");
    if user_sets_layout {
        // Two clicks: this throws away the user's arrangement.
        let b = widgets::confirm_button("Reset", "Click again to reset", |_| {
            let mut doc = read_user();
            super::nyxbarlayout::reset(&mut doc);
            write_user(&doc);
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(300), || window::rebuild("nyx-bar"));
        });
        b.add_css_class("destructive-action");
        let g = page.group("");
        g.add(&widgets::row(
            "Reset to NyxOS's arrangement",
            "Puts every item back where NyxOS places it. Size, clock and workspaces stay as they are.",
            Some(b.upcast_ref()),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUT: &str = r#"# Nyx Bar layout.
left = [["nyx.launcher", "nyx.workspaces"], "nyx.window"]
right = [
    "nyx.tray",
    ["nyx.system"],
    ["nyx.panels/notifications"],
]

# Size: compact (28 px), default (32), cozy (36) or spacious (40).
height = 30
"#;

    #[test]
    fn edits_keep_the_layout_and_comments() {
        let mut doc: DocumentMut = LAYOUT.parse().unwrap();
        assert_eq!(status_grouped(&doc), Some(true));
        assert!(set_status_grouped(&mut doc, false));
        assert_eq!(status_grouped(&doc), Some(false));
        set_size(&mut doc, "cozy");
        set_plugin(&mut doc, "nyx.clock", "layout", "compact");
        let text = doc.to_string();
        assert!(text.contains("# Nyx Bar layout."));
        assert!(text.contains("# Size: compact"));
        assert!(text.contains("\n    \"nyx.system\",\n"), "{text}");
        assert!(text.contains("size = \"cozy\""));
        assert!(!text.contains("height"));
        assert!(text.contains("[plugins.\"nyx.clock\"]\nlayout = \"compact\""), "{text}");
        let mut doc: DocumentMut = text.parse().unwrap();
        assert!(set_status_grouped(&mut doc, true));
        assert_eq!(status_grouped(&doc), Some(true));
    }

    #[test]
    fn sizes_in_pixels() {
        assert_eq!(size_px("compact", None), 28);
        assert_eq!(size_px("default", None), 32);
        assert_eq!(size_px("custom", Some(30)), 30);
    }
}
