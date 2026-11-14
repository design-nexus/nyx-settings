//! Nyx apps: the look every app in the suite shares, and installing, updating
//! and removing the apps themselves.

use crate::sketch::{self, Pal, Rgb};
use crate::widgets::{self, Page};
use crate::{cmd, paths, window};
use gtk::cairo::Context;
use gtk::prelude::*;
use nyx_theme::ThemeMode;
use std::cell::RefCell;
use std::rc::Rc;

/// Each app: what `nyx` calls it, its name, and the program that shows it's installed.
const APPS: &[(&str, &str, &str, &str)] = &[
    ("settings", "Settings", "nyx-settings", "A control panel for Omarchy"),
    ("contacts", "Contacts", "nyx-contacts", "Contacts, favorites and lists"),
    ("git", "Git", "nyx-git", "Git repositories and working changes"),
    ("passwords", "Passwords", "nyx-passwords", "Your Vaultwarden vault"),
    ("store", "Store", "nyx-store", "Discover, install and update apps"),
    ("activity", "Activity", "nyx-activity", "Processes, usage and startup apps"),
    ("torrents", "Torrents", "nyx-torrents", "A BitTorrent client"),
    ("pdf", "PDF", "nyx-pdf", "Read, fill in and mark up PDFs"),
    ("notepad", "Notepad", "nyx-notepad", "Plain text, with tabs"),
    ("music", "Music", "nyx-music", "Your music library, radio and podcasts"),
    ("media-player", "Media Player", "nyx-media-player", "Movies and TV shows"),
    ("chat", "Chat", "nyx-chat", "Chat with models on Ollama"),
    ("fonts", "Fonts", "nyx-fonts", "Google Fonts and Nerd Fonts"),
    ("archive", "Archive", "nyx-archive", "Open and make archives"),
    ("office", "Office", "nyx-office", "Write, Sheets and Present"),
    ("photos", "Photos", "nyx-photos", "A photo gallery and editor"),
    ("weather", "Weather", "nyx-weather", "Forecasts, hourly conditions and alerts"),
];

pub fn installed(program: &str) -> bool {
    paths::home().join(".local/bin").join(program).is_file()
}

/// Each app: what `nyx` calls it, its name, its program (also its id for theme
/// choices) and what it is.
pub fn apps() -> impl Iterator<Item = &'static (&'static str, &'static str, &'static str, &'static str)> {
    APPS.iter()
}

/// Run `nyx …` in a terminal: installing can ask for a password and takes a while.
fn in_terminal(args: &str) {
    let script = format!("nyx {args}; echo; read -rp 'Done. Press Enter to close.'");
    cmd::spawn(&["xdg-terminal-exec", "bash", "-lc", &script]);
}

/// An app's icon from its desktop entry (`io.github.design_nexus.nyx.MediaPlayer`), or a tile.
fn app_icon(name: &str) -> gtk::Widget {
    let id = match name {
        "PDF" => "Pdf".to_string(),
        n => n.replace(' ', ""),
    };
    let icon = format!("io.github.design_nexus.nyx.{id}");
    if gtk::gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(&icon)) {
        let img = gtk::Image::from_icon_name(&icon);
        img.set_pixel_size(30);
        img.set_valign(gtk::Align::Center);
        img.add_css_class("nx-app-icon");
        img.upcast()
    } else {
        widgets::tile("application-x-executable-symbolic").upcast()
    }
}

fn apps_page(page: &Page) {
    let (here, more): (Vec<_>, Vec<_>) = APPS.iter().partition(|a| installed(a.2));
    let g = page.group("Installed");
    g.note("Updating rebuilds the installed apps from the latest source; a terminal shows the progress.");
    let (r, _) = widgets::button_row("Update every app", "Rebuild the installed apps from the latest source.", "Update", |_| {
        in_terminal("update")
    });
    r.prepend(&widgets::tile("software-update-available-symbolic"));
    r.add_css_class("with-tile");
    g.add(&r);
    for &&(id, name, _, desc) in &here {
        let remove = widgets::confirm_button("Remove", "Remove?", move |_| in_terminal(&format!("uninstall {id}")));
        remove.add_css_class("flat");
        let r = widgets::row(name, desc, Some(remove.upcast_ref()));
        r.prepend(&app_icon(name));
        r.add_css_class("with-tile");
        g.add(&r);
    }
    if !more.is_empty() {
        let g = page.group("More to install");
        g.note("Installing builds the app from source, which takes a few minutes the first time.");
        for &&(id, name, _, desc) in &more {
            let add = gtk::Button::with_label("Install");
            add.connect_clicked(move |_| in_terminal(&format!("install {id}")));
            let r = widgets::row(name, desc, Some(add.upcast_ref()));
            r.prepend(&app_icon(name));
            r.add_css_class("with-tile");
            g.add(&r);
        }
    }
}

// ---------- The picture: a Nyx app drawn with the look ----------

#[derive(Clone, Copy, PartialEq)]
enum Sample {
    Contacts,
    Music,
    Notepad,
}

impl Sample {
    fn name(self) -> &'static str {
        match self {
            Sample::Contacts => "Contacts",
            Sample::Music => "Music",
            Sample::Notepad => "Notepad",
        }
    }

    /// What its list holds, for "7 … fit".
    fn things(self) -> &'static str {
        match self {
            Sample::Contacts => "people",
            Sample::Music => "albums",
            Sample::Notepad => "lines",
        }
    }
}

struct Look {
    compact: bool,
    scale: f64,
    sample: Sample,
}

fn text(cr: &Context, c: Rgb, a: f64, s: &str, x: f64, y: f64, size: f64, bold: bool) -> f64 {
    let weight = if bold { gtk::cairo::FontWeight::Bold } else { gtk::cairo::FontWeight::Normal };
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, weight);
    cr.set_font_size(size);
    sketch::set(cr, c, a);
    let ext = cr.text_extents(s).ok();
    cr.move_to(x, y + size * 0.36);
    let _ = cr.show_text(s);
    ext.map(|e| e.x_advance()).unwrap_or(0.0)
}

fn bar(cr: &Context, c: Rgb, a: f64, x: f64, y: f64, w: f64, h: f64) {
    sketch::set(cr, c, a);
    sketch::rounded(cr, x, y - h / 2.0, w.max(0.0), h, h / 2.0);
    let _ = cr.fill();
}

/// The theme's swatch colours, for avatars and album covers.
fn swatches() -> Vec<Rgb> {
    let s: Vec<Rgb> = nyx_theme::swatches().into_iter().map(|(_, hex)| crate::fangraph::rgb(&hex)).collect();
    if s.is_empty() { vec![sketch::pal().accent] } else { s }
}

/// A row's height in the list at a density and text size.
fn row_h(sample: Sample, compact: bool, scale: f64) -> f64 {
    let base = match (sample, compact) {
        (Sample::Contacts, false) => 46.0,
        (Sample::Contacts, true) => 34.0,
        (Sample::Music, false) => 118.0,
        (Sample::Music, true) => 98.0,
        (Sample::Notepad, false) => 22.0,
        (Sample::Notepad, true) => 18.0,
    };
    base * scale
}

fn head_h(scale: f64) -> f64 {
    34.0 + 6.0 * scale
}

fn side_w(body_w: f64) -> f64 {
    (body_w * 0.24).clamp(110.0, 170.0)
}

fn contacts_detail_w(body_w: f64) -> f64 {
    if body_w > 560.0 { (body_w * 0.32).min(240.0) } else { 0.0 }
}

fn music_cover(compact: bool, scale: f64) -> (f64, f64) {
    let rh = row_h(Sample::Music, compact, scale);
    let cover = rh * 0.74;
    (cover, (rh * 0.82 - cover).max(8.0))
}

/// How many fit in a `w`×`h` picture: whole rows of the list, or covers for Music.
fn fits(sample: Sample, compact: bool, scale: f64, w: f64, h: f64) -> usize {
    let body_h = h - head_h(scale) - 1.0;
    let rh = row_h(sample, compact, scale);
    let rows = |space: f64| (space / rh).floor().max(0.0) as usize;
    match sample {
        Sample::Contacts => rows(body_h - 6.0),
        Sample::Music => {
            let grid_w = w - side_w(w) - 1.0;
            let (cover, gap) = music_cover(compact, scale);
            let cols = ((grid_w - 16.0) / (cover + gap)).floor().max(1.0) as usize;
            rows(body_h - 40.0 * scale - 12.0) * cols
        }
        Sample::Notepad => rows(body_h - if compact { 26.0 } else { 32.0 } * scale - 10.0),
    }
}

fn draw_app(cr: &Context, p: &Pal, w: f64, h: f64, look: &Look) {
    let s = look.scale;
    let f = 11.0 * s;
    let frame = sketch::R::new(0.5, 0.5, w - 1.0, h - 1.0);
    sketch::rounded(cr, frame.x, frame.y, frame.w, frame.h, 10.0);
    sketch::set(cr, sketch::mix(p.bg, p.text, 0.035), 1.0);
    let _ = cr.fill_preserve();
    sketch::set(cr, p.dim, 0.22);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    cr.save().ok();
    sketch::rounded(cr, frame.x, frame.y, frame.w, frame.h, 10.0);
    cr.clip();
    // The header: the app's tile and name, and a search field.
    let head_h = head_h(s);
    sketch::set(cr, p.dim, 0.18);
    cr.rectangle(0.0, head_h, w, 1.0);
    let _ = cr.fill();
    sketch::set(cr, p.accent, 0.9);
    sketch::rounded(cr, 12.0, head_h / 2.0 - 9.0, 18.0, 18.0, 5.0);
    let _ = cr.fill();
    text(cr, p.text, 0.95, look.sample.name(), 38.0, head_h / 2.0, f * 1.08, true);
    let sw = (w * 0.26).clamp(90.0, 170.0);
    sketch::set(cr, sketch::mix(p.bg, p.text, 0.08), 1.0);
    sketch::rounded(cr, w - sw - 12.0, head_h / 2.0 - 11.0, sw, 22.0, 7.0);
    let _ = cr.fill();
    text(cr, p.dim, 0.8, "Search", w - sw + 4.0, head_h / 2.0, f * 0.92, false);
    let body = sketch::R::new(0.0, head_h + 1.0, w, h - head_h - 1.0);
    let colours = swatches();
    match look.sample {
        Sample::Contacts => contacts(cr, p, body, look, &colours),
        Sample::Music => music(cr, p, body, look, &colours),
        Sample::Notepad => notepad(cr, p, body, look),
    }
    cr.restore().ok();
}

fn sidebar(cr: &Context, p: &Pal, body: sketch::R, look: &Look, items: &[&str]) -> f64 {
    let s = look.scale;
    let side_w = side_w(body.w);
    sketch::set(cr, p.dim, 0.16);
    cr.rectangle(body.x + side_w, body.y, 1.0, body.h);
    let _ = cr.fill();
    let item_h = if look.compact { 26.0 } else { 32.0 } * s;
    for (i, name) in items.iter().enumerate() {
        let y = body.y + 10.0 + i as f64 * item_h + item_h / 2.0;
        if i == 0 {
            sketch::set(cr, p.accent, 0.16);
            sketch::rounded(cr, body.x + 8.0, y - item_h / 2.0 + 2.0, side_w - 16.0, item_h - 4.0, 6.0);
            let _ = cr.fill();
        }
        sketch::set(cr, if i == 0 { p.accent } else { p.dim }, 0.9);
        cr.arc(body.x + 22.0, y, 4.0 * s, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        text(cr, p.text, if i == 0 { 1.0 } else { 0.75 }, name, body.x + 34.0, y, 11.0 * s, i == 0);
    }
    side_w
}

fn contacts(cr: &Context, p: &Pal, body: sketch::R, look: &Look, colours: &[Rgb]) {
    let s = look.scale;
    let side_w = sidebar(cr, p, body, look, &["All", "Favorites", "Birthdays"]);
    let detail_w = contacts_detail_w(body.w);
    let list = sketch::R::new(body.x + side_w + 1.0, body.y, body.w - side_w - 1.0 - detail_w, body.h);
    let rh = row_h(Sample::Contacts, look.compact, s);
    let avatar = if look.compact { 11.0 } else { 14.0 } * s;
    let mut y = list.y + 6.0;
    let mut i = 0;
    while y + rh <= list.y + list.h {
        let cy = y + rh / 2.0;
        if i == 0 {
            sketch::set(cr, p.accent, 0.14);
            sketch::rounded(cr, list.x + 6.0, y + 1.0, list.w - 12.0, rh - 2.0, 7.0);
            let _ = cr.fill();
        }
        sketch::set(cr, colours[i % colours.len()], 0.85);
        cr.arc(list.x + 18.0 + avatar, cy, avatar, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        let tx = list.x + 28.0 + avatar * 2.0;
        let tw = list.w - (tx - list.x) - 16.0;
        let name_w = tw * [0.55, 0.42, 0.6, 0.48, 0.38, 0.52][i % 6];
        if look.compact {
            bar(cr, p.text, 0.7, tx, cy, name_w, 6.0 * s);
        } else {
            bar(cr, p.text, 0.7, tx, cy - 6.0 * s, name_w, 6.0 * s);
            bar(cr, p.dim, 0.45, tx, cy + 7.0 * s, name_w * 0.7, 5.0 * s);
        }
        y += rh;
        i += 1;
    }
    if detail_w > 0.0 {
        let d = sketch::R::new(body.x + body.w - detail_w, body.y, detail_w, body.h);
        sketch::set(cr, p.dim, 0.16);
        cr.rectangle(d.x, d.y, 1.0, d.h);
        let _ = cr.fill();
        let r = 28.0 * s;
        sketch::set(cr, colours[0], 0.9);
        cr.arc(d.cx(), d.y + 24.0 + r, r, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        let ty = d.y + 40.0 + r * 2.0;
        bar(cr, p.text, 0.8, d.cx() - d.w * 0.25, ty, d.w * 0.5, 8.0 * s);
        bar(cr, p.dim, 0.5, d.cx() - d.w * 0.18, ty + 16.0 * s, d.w * 0.36, 5.0 * s);
        let card = sketch::R::new(d.x + 14.0, ty + 32.0 * s, d.w - 28.0, (3.0 * 26.0 + 14.0) * s);
        sketch::set(cr, sketch::mix(p.bg, p.text, 0.07), 1.0);
        sketch::rounded(cr, card.x, card.y, card.w, card.h, 8.0);
        let _ = cr.fill();
        for (j, label) in ["Phone", "Email", "Birthday"].iter().enumerate() {
            let fy = card.y + 7.0 * s + 13.0 * s + j as f64 * 26.0 * s;
            text(cr, p.dim, 0.85, label, card.x + 10.0, fy, 9.5 * s, false);
            bar(cr, p.text, 0.6, card.x + 10.0 + 56.0 * s, fy, card.w * 0.42, 5.0 * s);
        }
    }
}

fn music(cr: &Context, p: &Pal, body: sketch::R, look: &Look, colours: &[Rgb]) {
    let s = look.scale;
    let side_w = sidebar(cr, p, body, look, &["Library", "Albums", "Radio", "Podcasts"]);
    let player_h = 40.0 * s;
    let grid = sketch::R::new(body.x + side_w + 1.0, body.y, body.w - side_w - 1.0, body.h - player_h);
    let rh = row_h(Sample::Music, look.compact, s);
    let (cover, gap) = music_cover(look.compact, s);
    let cols = ((grid.w - 16.0) / (cover + gap)).floor().max(1.0) as usize;
    let mut i = 0;
    let mut y = grid.y + 12.0;
    while y + rh <= grid.y + grid.h {
        for c in 0..cols {
            let x = grid.x + 14.0 + c as f64 * (cover + gap);
            let col = colours[(i * 3 + 1) % colours.len()];
            let g = gtk::cairo::LinearGradient::new(x, y, x + cover, y + cover);
            let a = sketch::mix(col, p.bg, 0.15);
            let b = sketch::mix(col, p.bg, 0.55);
            g.add_color_stop_rgb(0.0, a.0, a.1, a.2);
            g.add_color_stop_rgb(1.0, b.0, b.1, b.2);
            sketch::rounded(cr, x, y, cover, cover, 8.0);
            let _ = cr.set_source(&g);
            let _ = cr.fill();
            bar(cr, p.text, 0.7, x, y + cover + 10.0 * s, cover * 0.7, 5.5 * s);
            if !look.compact {
                bar(cr, p.dim, 0.45, x, y + cover + 21.0 * s, cover * 0.5, 4.5 * s);
            }
            i += 1;
        }
        y += rh;
    }
    // The player along the foot.
    let py = body.y + body.h - player_h;
    sketch::set(cr, sketch::mix(p.bg, p.text, 0.06), 1.0);
    cr.rectangle(body.x, py, body.w, player_h);
    let _ = cr.fill();
    let cy = py + player_h / 2.0;
    sketch::set(cr, colours[0], 0.85);
    sketch::rounded(cr, body.x + 12.0, cy - 12.0 * s, 24.0 * s, 24.0 * s, 5.0);
    let _ = cr.fill();
    bar(cr, p.text, 0.7, body.x + 22.0 + 24.0 * s, cy - 5.0 * s, 90.0 * s, 5.5 * s);
    bar(cr, p.dim, 0.45, body.x + 22.0 + 24.0 * s, cy + 6.0 * s, 60.0 * s, 4.5 * s);
    sketch::set(cr, p.accent, 1.0);
    cr.arc(body.cx(), cy, 11.0 * s, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
    sketch::set(cr, p.bg, 1.0);
    cr.move_to(body.cx() - 3.0 * s, cy - 5.0 * s);
    cr.line_to(body.cx() + 5.0 * s, cy);
    cr.line_to(body.cx() - 3.0 * s, cy + 5.0 * s);
    cr.close_path();
    let _ = cr.fill();
    let track = sketch::R::new(body.x + body.w * 0.62, cy - 2.0, body.w * 0.3, 4.0);
    bar(cr, p.dim, 0.3, track.x, cy, track.w, 4.0);
    bar(cr, p.accent, 0.9, track.x, cy, track.w * 0.38, 4.0);
}

fn notepad(cr: &Context, p: &Pal, body: sketch::R, look: &Look) {
    let s = look.scale;
    // Tabs.
    let tab_h = if look.compact { 26.0 } else { 32.0 } * s;
    sketch::set(cr, sketch::mix(p.bg, p.text, 0.05), 1.0);
    cr.rectangle(body.x, body.y, body.w, tab_h);
    let _ = cr.fill();
    for i in 0..3 {
        let x = body.x + 10.0 + i as f64 * 120.0 * s;
        if i == 0 {
            sketch::set(cr, sketch::mix(p.bg, p.text, 0.035), 1.0);
            sketch::rounded(cr, x, body.y + 4.0, 112.0 * s, tab_h - 4.0, 6.0);
            let _ = cr.fill();
            sketch::set(cr, p.accent, 1.0);
            cr.rectangle(x + 6.0, body.y + tab_h - 2.0, 100.0 * s, 2.0);
            let _ = cr.fill();
        }
        bar(cr, p.text, if i == 0 { 0.75 } else { 0.4 }, x + 12.0, body.y + tab_h / 2.0 + 2.0, 70.0 * s, 5.5 * s);
    }
    let area = sketch::R::new(body.x, body.y + tab_h, body.w, body.h - tab_h);
    let rh = row_h(Sample::Notepad, look.compact, s);
    let gutter = 34.0 * s;
    let widths = [0.62, 0.48, 0.0, 0.71, 0.55, 0.66, 0.3, 0.0, 0.58, 0.75, 0.42, 0.63, 0.5, 0.0, 0.69, 0.36];
    let mut y = area.y + 10.0;
    let mut i = 0;
    while y + rh <= area.y + area.h {
        let cy = y + rh / 2.0;
        text(cr, p.dim, 0.6, &(i + 1).to_string(), area.x + 12.0, cy, 9.5 * s, false);
        let lw = widths[i % widths.len()];
        if lw > 0.0 {
            bar(cr, p.text, 0.55, area.x + gutter + 10.0, cy, (area.w - gutter - 40.0) * lw, 5.5 * s);
        }
        if i == 4 {
            sketch::set(cr, p.accent, 1.0);
            let x = area.x + gutter + 12.0 + (area.w - gutter - 40.0) * lw;
            cr.rectangle(x, cy - 7.0 * s, 1.5, 14.0 * s);
            let _ = cr.fill();
        }
        y += rh;
        i += 1;
    }
}

// ---------- The page ----------

fn theme_name(id: &str) -> String {
    nyx_theme::engine::list_themes().into_iter().find(|m| m.id == id).map(|m| m.name).unwrap_or_else(|| id.to_string())
}

/// "Contacts, with the Omarchy theme, comfortable, at 100%".
fn describe(look: &Look) -> String {
    let suite = nyx_theme::read_suite().unwrap_or_default();
    let theme = if suite.mode == ThemeMode::Omarchy && nyx_theme::omarchy_available() {
        "the Omarchy theme".to_string()
    } else {
        format!("the {} theme", theme_name(&suite.theme))
    };
    format!(
        "{}, with {theme}, {}, at {}%",
        look.sample.name(),
        if look.compact { "compact" } else { "comfortable" },
        (look.scale * 100.0).round()
    )
}

/// A label that wraps instead of holding the preview wider than the window.
fn wrapping(l: gtk::Label) -> gtk::Label {
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l
}

/// A row from `widgets`, with an icon on a tile before it.
fn with_tile(r: gtk::Box, icon: &str) -> gtk::Box {
    r.prepend(&widgets::tile(icon));
    r.add_css_class("with-tile");
    r
}

pub fn build(page: &Page) {
    let suite = nyx_theme::read_suite().unwrap_or_default();
    let look = Rc::new(RefCell::new(Look {
        compact: suite.density == "compact",
        scale: suite.text_scale,
        sample: Sample::Contacts,
    }));

    // The picture: a Nyx app drawn with these settings.
    let pv = page.split();
    let pick = nyx_ui::widgets::segmented(
        &widgets::opts(&[("contacts", "Contacts"), ("music", "Music"), ("notepad", "Notepad")]),
        "contacts",
        |_| {},
    );
    let head = pv.head("How Nyx apps look", "", &[pick.upcast_ref()]);
    let line = widgets::label("", "preview-line");
    line.set_wrap(true);
    if let Some(t) = head.first_child().and_downcast::<gtk::Box>() {
        if let Some(title) = t.first_child().and_downcast::<gtk::Label>() {
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        }
        t.append(&line);
    }
    let picture = {
        let look = look.clone();
        pv.drawing(280, move |cr, p, _, w, h| draw_app(cr, p, w, h, &look.borrow()))
    };
    picture.set_vexpand(true);
    picture.add_css_class("nx-picture");

    // How many fit at each density, and where one app gets its own theme.
    let strip = widgets::hbox(10);
    strip.set_homogeneous(true);
    strip.add_css_class("nx-compare");
    let density_cards: Rc<Vec<(bool, gtk::ToggleButton, gtk::Label)>> = Rc::new(
        [(false, "Comfortable"), (true, "Compact")]
            .into_iter()
            .map(|(compact, name)| {
                let b = gtk::ToggleButton::new();
                b.add_css_class("nx-card");
                let col = widgets::vbox(2);
                col.append(&wrapping(widgets::label(name, "nx-card-title")));
                let fit = wrapping(widgets::label("", "nx-card-line"));
                col.append(&fit);
                b.set_child(Some(&col));
                strip.append(&b);
                (compact, b, fit)
            })
            .collect(),
    );
    let own = gtk::Button::new();
    own.add_css_class("nx-card");
    let col = widgets::vbox(2);
    col.append(&wrapping(widgets::label("Your own theme for one app", "nx-card-title")));
    let link = widgets::hbox(4);
    link.append(&wrapping(widgets::label("Appearance › App themes", "nx-card-link")));
    link.append(&gtk::Image::from_icon_name("go-next-symbolic"));
    col.append(&link);
    own.set_child(Some(&col));
    own.connect_clicked(|_| window::navigate("look"));
    strip.append(&own);
    pv.add(&strip);

    // Keep the line, the picture and the counts in step with the look.
    let refresh: Rc<dyn Fn()> = {
        let (look, line, picture, cards) = (look.clone(), line.clone(), picture.clone(), density_cards.clone());
        Rc::new(move || {
            let l = look.borrow();
            line.set_text(&describe(&l));
            picture.queue_draw();
            let (w, h) = (picture.width() as f64, picture.height() as f64);
            for (compact, b, fit) in cards.iter() {
                b.set_active(*compact == l.compact);
                if h > 0.0 {
                    let n = fits(l.sample, *compact, l.scale, w, h);
                    fit.set_text(&format!("{n} {} fit in this window", l.sample.things()));
                }
            }
        })
    };
    {
        // The counts need the picture's size.
        let refresh = refresh.clone();
        picture.connect_resize(move |_, _, _| {
            let r = refresh.clone();
            gtk::glib::idle_add_local_once(move || r());
        });
    }
    {
        let (look, refresh) = (look.clone(), refresh.clone());
        let mut child = pick.first_child();
        for sample in [Sample::Contacts, Sample::Music, Sample::Notepad] {
            let Some(b) = child.and_downcast::<gtk::ToggleButton>() else { break };
            let (look, refresh) = (look.clone(), refresh.clone());
            b.connect_toggled(move |b| {
                if b.is_active() {
                    look.borrow_mut().sample = sample;
                    refresh();
                }
            });
            child = b.next_sibling();
        }
    }
    {
        let r = refresh.clone();
        crate::theme::subscribe(&picture, move || r());
    }

    // The controls.
    let g = page.group("Look");
    g.note("Every Nyx app follows these at once");
    let follow = suite.mode == ThemeMode::Omarchy;
    let themes: Vec<(String, String)> = nyx_theme::engine::list_themes().into_iter().map(|m| (m.id, m.name)).collect();
    let (theme_row, theme_dd) = widgets::choice_row("Theme when not following", "", themes, &suite.theme, {
        let refresh = refresh.clone();
        move |id| {
            if let Err(e) = nyx_theme::choose_system_theme(&id) {
                crate::window::toast(&e);
            }
            refresh();
        }
    });
    theme_dd.set_sensitive(!follow || !nyx_theme::omarchy_available());
    widgets::keywords("system theme nyx apps shell");
    if nyx_theme::omarchy_available() {
        let dd = theme_dd.clone();
        let refresh = refresh.clone();
        let (r, _) = widgets::switch_row("Follow the Omarchy theme", "Its colours, and change when it does.", follow, move |on| {
            dd.set_sensitive(!on);
            let id = if on { "omarchy".to_string() } else { nyx_theme::engine::suite().theme };
            if let Err(e) = nyx_theme::choose_system_theme(&id) {
                crate::window::toast(&e);
            }
            refresh();
        });
        g.add(&with_tile(r, "applications-graphics-symbolic"));
    }
    g.add(&with_tile(theme_row, "weather-clear-night-symbolic"));

    let density = nyx_ui::widgets::segmented(
        &[("comfortable".into(), "Comfortable".into()), ("compact".into(), "Compact".into())],
        &suite.density,
        {
            let (look, refresh) = (look.clone(), refresh.clone());
            move |d| {
                look.borrow_mut().compact = d == "compact";
                refresh();
                nyx_theme::update_suite(|s| s.density = d);
            }
        },
    );
    g.add(&with_tile(widgets::row("Density", "", Some(density.upcast_ref())), "view-list-symbolic"));
    widgets::keywords("compact comfortable fits more rows on screen");
    // The cards under the picture pick the density too.
    for (compact, b, _) in density_cards.iter() {
        let (compact, seg) = (*compact, density.clone());
        b.connect_clicked(move |b| {
            let want = if compact { 1 } else { 0 };
            let mut child = seg.first_child();
            for _ in 0..want {
                child = child.and_then(|c| c.next_sibling());
            }
            match child.and_downcast::<gtk::ToggleButton>() {
                Some(t) if !t.is_active() => t.set_active(true),
                // Already that density: a toggle button would turn itself off.
                _ => b.set_active(true),
            }
        });
    }

    let (r, text_slider) = widgets::slider_row(
        "Text size",
        "",
        (90.0, 125.0, 5.0),
        (suite.text_scale * 100.0).round(),
        0,
        "%",
        {
            let (look, refresh) = (look.clone(), refresh.clone());
            move |v| {
                let scale = (v / 5.0).round() * 5.0 / 100.0;
                look.borrow_mut().scale = scale;
                refresh();
                nyx_theme::update_suite(|s| s.text_scale = scale);
            }
        },
    );
    text_slider.scale.set_width_request(110);
    g.add(&with_tile(r, "font-x-generic-symbolic"));
    widgets::keywords("larger smaller text in every nyx app theme density compact text size font scale suite");

    let start = nyx_ui::widgets::segmented(
        &widgets::opts(&[("home", "Start page"), ("last", "Last page")]),
        if nyx_theme::reopens_last_page() { "last" } else { "home" },
        |v| nyx_theme::update_suite(|s| s.start = v),
    );
    g.add(&with_tile(widgets::row("Open on", "", Some(start.upcast_ref())), "window-new-symbolic"));
    widgets::keywords("where every nyx app opens start page last page home remember");

    // The apps, and the ones with a theme of their own.
    let g = page.group("");
    let here = APPS.iter().filter(|a| installed(a.2)).count();
    let more = APPS.len() - here;
    let desc = match more {
        0 => format!("All {here} installed"),
        m => format!("{here} installed, {m} more to install"),
    };
    let (r, _) = widgets::nav_row(Some("view-app-grid-symbolic"), "Apps", &desc, "", || {
        widgets::subpage("Nyx apps", 580, |p| {
            apps_page(p);
            widgets::forget_rows(&p.body);
        });
    });
    widgets::keywords("install update remove uninstall rebuild every app music pdf chat contacts weather");
    g.add(&r);
    let own: Vec<&str> = APPS
        .iter()
        .filter(|(_, _, program, _)| suite.applications.get(*program).is_some_and(|c| c.mode == nyx_theme::engine::AppMode::Override))
        .map(|(_, name, _, _)| *name)
        .collect();
    let value = match own.len() {
        0 => "None".to_string(),
        1..=2 => own.join(", "),
        n => format!("{n} apps"),
    };
    let (r, _) = widgets::nav_row(
        Some("preferences-color-symbolic"),
        "Apps with their own theme",
        "Set on Appearance › App themes",
        &value,
        || window::navigate("look"),
    );
    g.add(&r);

    refresh();
}
