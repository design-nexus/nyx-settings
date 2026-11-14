//! Appearance: themes as swatch cards that preview on a small desktop before they're
//! applied, the wallpaper, fonts and icons; the theme library, apps that follow the
//! theme and this window's own look are rows that open their own pages.

use crate::backend::{appearance, chroma};
use crate::sketch::{self, R, Rgb, mix, rounded, set};
use crate::widgets::{self, Page};
use crate::{cmd, paths, prefs, theme, window};
use gtk::prelude::*;
use gtk::{gdk, gdk_pixbuf, glib, pango};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

// ----- Themes and their colours -----

/// Enough of a theme's colours to draw a desktop in it.
#[derive(Clone, Copy)]
pub(crate) struct Colours {
    pub bg: Rgb,
    pub panel: Rgb,
    pub text: Rgb,
    pub accent: Rgb,
    pub warm: Rgb,
    pub green: Rgb,
}

/// A theme to pick: its id (Omarchy's display name or a Nyx theme id), name and look.
#[derive(Clone)]
pub(crate) struct Choice {
    pub id: String,
    pub name: String,
    pub colours: Option<Colours>,
    /// The wallpaper it brings, for the preview.
    pub wallpaper: Option<PathBuf>,
    /// Its own picture, for a theme whose colours can't be read.
    pub picture: Option<PathBuf>,
}

pub(crate) fn hex(s: &str) -> Option<Rgb> {
    let c = gdk::RGBA::parse(s.trim()).ok()?;
    Some((c.red() as f64, c.green() as f64, c.blue() as f64))
}

fn read_toml(path: &Path) -> Option<toml::Table> {
    toml::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Omarchy theme directory for a display name ("Catppuccin Latte" -> catppuccin-latte).
fn theme_dir(name: &str) -> Option<PathBuf> {
    let slug = name.to_lowercase().replace(' ', "-");
    [paths::omarchy_config().join("themes").join(&slug), PathBuf::from("/usr/share/omarchy/themes").join(&slug)]
        .into_iter()
        .find(|p| p.exists())
}

/// An Omarchy theme's colours: `colors.toml`, or an older theme's `alacritty.toml`.
fn omarchy_colours(dir: &Path) -> Option<Colours> {
    if let Some(t) = read_toml(&dir.join("colors.toml")) {
        let get = |keys: &[&str]| keys.iter().find_map(|k| t.get(*k).and_then(|v| v.as_str()).and_then(hex));
        let bg = get(&["background"])?;
        let text = get(&["foreground"])?;
        let accent = get(&["accent", "blue", "color4"]).unwrap_or(text);
        return Some(Colours {
            bg,
            panel: get(&["lighter_background"]).unwrap_or(mix(bg, text, 0.07)),
            text,
            accent,
            warm: get(&["red", "color1", "orange"]).unwrap_or(accent),
            green: get(&["green", "color2"]).unwrap_or(accent),
        });
    }
    let t = read_toml(&dir.join("alacritty.toml"))?;
    let colors = t.get("colors")?;
    let get = |group: &str, key: &str| colors.get(group)?.get(key)?.as_str().and_then(hex);
    let bg = get("primary", "background")?;
    let text = get("primary", "foreground")?;
    let accent = get("normal", "blue").unwrap_or(text);
    Some(Colours {
        bg,
        panel: mix(bg, text, 0.07),
        text,
        accent,
        warm: get("normal", "red").unwrap_or(accent),
        green: get("normal", "green").unwrap_or(accent),
    })
}

fn is_image(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| ["png", "jpg", "jpeg", "webp", "bmp", "gif"].contains(&e.to_lowercase().as_str()))
}

fn images_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| is_image(p)).collect();
    v.sort();
    v
}

/// The picture that best shows a theme: its preview, else a screenshot or its first wallpaper.
fn theme_image(dir: &Path) -> Option<PathBuf> {
    ["preview.png", "theme.png", "preview.jpg", "theme.jpg"]
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.exists())
        .or_else(|| images_in(&dir.join("backgrounds")).into_iter().next())
}

/// Omarchy's themes and the one in use.
fn omarchy_choices() -> (Vec<Choice>, String) {
    let current = cmd::output(&["omarchy-theme-current"]).map(|s| s.trim().to_string()).unwrap_or_default();
    let names: Vec<String> = cmd::output(&["omarchy-theme-list"])
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    let choices = names
        .into_iter()
        .map(|name| {
            let dir = theme_dir(&name);
            let slug = name.to_lowercase().replace(' ', "-");
            let mut walls = images_in(&paths::omarchy_config().join("backgrounds").join(&slug));
            if let Some(d) = &dir {
                walls.extend(images_in(&d.join("backgrounds")));
            }
            Choice {
                colours: dir.as_deref().and_then(omarchy_colours),
                wallpaper: walls.into_iter().next(),
                picture: dir.as_deref().and_then(theme_image),
                id: name.clone(),
                name,
            }
        })
        .collect();
    (choices, current)
}

// ----- Omarchy's wallpaper -----

fn omarchy_state() -> PathBuf {
    paths::omarchy_theme_dir().parent().map(Path::to_path_buf).unwrap_or_default()
}

/// The wallpaper showing now.
fn current_background() -> Option<PathBuf> {
    std::fs::canonicalize(omarchy_state().join("background")).ok()
}

/// The current theme's wallpapers, the ones Next steps through (pictures only).
fn current_wallpapers() -> Vec<PathBuf> {
    let name = std::fs::read_to_string(omarchy_state().join("theme.name")).unwrap_or_default();
    let mut v = images_in(&paths::omarchy_config().join("backgrounds").join(name.trim()));
    v.extend(images_in(&paths::omarchy_theme_dir().join("backgrounds")));
    v.into_iter().map(|p| std::fs::canonicalize(&p).unwrap_or(p)).collect()
}

// ----- Pictures -----

thread_local! {
    /// Wallpapers loaded for the preview; `None` while loading or when it can't be read.
    static WALLS: RefCell<HashMap<PathBuf, Option<gdk_pixbuf::Pixbuf>>> = RefCell::default();
}

/// The wallpaper at `path`, or `None` until it's loaded (then `ready` is called).
fn wall_pixbuf(path: &Path, ready: impl Fn() + 'static) -> Option<gdk_pixbuf::Pixbuf> {
    if let Some(v) = WALLS.with(|w| w.borrow().get(path).cloned()) {
        return v;
    }
    WALLS.with(|w| w.borrow_mut().insert(path.to_path_buf(), None));
    let (p, key) = (path.to_path_buf(), path.to_path_buf());
    cmd::background(
        move || {
            let pb = gdk_pixbuf::Pixbuf::from_file_at_scale(&p, 1100, 700, true).ok()?;
            Some((pb.read_pixel_bytes(), pb.width(), pb.height(), pb.rowstride(), pb.has_alpha()))
        },
        move |r| {
            if let Some((bytes, w, h, stride, alpha)) = r {
                let pb = gdk_pixbuf::Pixbuf::from_bytes(&bytes, gdk_pixbuf::Colorspace::Rgb, alpha, 8, w, h, stride);
                WALLS.with(|m| m.borrow_mut().insert(key, Some(pb)));
                ready();
            }
        },
    );
    None
}

pub(crate) fn load_thumbnail(picture: &gtk::Picture, path: PathBuf) {
    let picture = picture.clone();
    cmd::background(
        move || {
            let pb = gdk_pixbuf::Pixbuf::from_file_at_scale(&path, 360, 202, true).ok()?;
            Some((pb.read_pixel_bytes(), pb.width(), pb.height(), pb.rowstride(), pb.has_alpha()))
        },
        move |result| {
            if let Some((bytes, w, h, stride, alpha)) = result {
                let format = if alpha { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
                let texture = gdk::MemoryTexture::new(w, h, format, &bytes, stride as usize);
                picture.set_paintable(Some(&texture));
            }
        },
    );
}

// ----- Drawings in a theme's colours -----

fn text(cr: &gtk::cairo::Context, s: &str, x: f64, y: f64, size: f64, bold: bool, centre: bool) {
    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        if bold { gtk::cairo::FontWeight::Bold } else { gtk::cairo::FontWeight::Normal },
    );
    cr.set_font_size(size);
    let Ok(e) = cr.text_extents(s) else { return };
    let x = if centre { x - e.width() / 2.0 - e.x_bearing() } else { x };
    cr.move_to(x, y - e.height() / 2.0 - e.y_bearing());
    let _ = cr.show_text(s);
}

/// The wallpaper filling `r`, cropped to fit, or a wash of the theme's colours.
fn paint_wall(cr: &gtk::cairo::Context, c: &Colours, wall: Option<&gdk_pixbuf::Pixbuf>, r: R) {
    use gdk::prelude::GdkCairoContextExt;
    match wall {
        Some(pb) => {
            let (pw, ph) = (pb.width() as f64, pb.height() as f64);
            let s = (r.w / pw).max(r.h / ph);
            cr.save().ok();
            cr.rectangle(r.x, r.y, r.w, r.h);
            cr.clip();
            cr.translate(r.x + (r.w - pw * s) / 2.0, r.y + (r.h - ph * s) / 2.0);
            cr.scale(s, s);
            cr.set_source_pixbuf(pb, 0.0, 0.0);
            let _ = cr.paint();
            cr.restore().ok();
        }
        None => {
            let g = gtk::cairo::LinearGradient::new(r.x, r.y, r.x + r.w, r.y + r.h);
            for (at, col) in [(0.0, mix(c.bg, c.green, 0.55)), (0.55, mix(c.bg, c.accent, 0.3)), (1.0, c.bg)] {
                g.add_color_stop_rgb(at, col.0, col.1, col.2);
            }
            cr.rectangle(r.x, r.y, r.w, r.h);
            let _ = cr.set_source(&g);
            let _ = cr.fill();
        }
    }
}

/// The largest 16:10 screen that fits in `w`×`h`, centred.
fn screen_rect(w: f64, h: f64) -> R {
    let sw = w.min(h * 1.6) - 2.0;
    let sh = sw / 1.6;
    R::new((w - sw) / 2.0, (h - sh) / 2.0, sw, sh)
}

fn now(format: &str) -> String {
    glib::DateTime::now_local().ok().and_then(|d| d.format(format).ok()).map(|s| s.to_string()).unwrap_or_default()
}

/// A window in the theme: its panel colour, a hairline and three dots in the title bar.
fn theme_window(cr: &gtk::cairo::Context, c: &Colours, r: R, focused: bool, radius: f64) -> R {
    rounded(cr, r.x, r.y, r.w, r.h, radius);
    set(cr, c.panel, 0.96);
    let _ = cr.fill_preserve();
    if focused {
        set(cr, c.accent, 1.0);
        cr.set_line_width(1.6);
    } else {
        set(cr, mix(c.panel, c.text, 0.18), 1.0);
        cr.set_line_width(1.0);
    }
    let _ = cr.stroke();
    let dot = (r.h * 0.025).clamp(1.6, 2.6);
    for i in 0..3 {
        set(cr, c.text, 0.3);
        cr.arc(r.x + 8.0 + i as f64 * dot * 3.0, r.y + 8.0, dot, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
    R::new(r.x + 8.0, r.y + 16.0, r.w - 16.0, r.h - 22.0)
}

fn bar_line(cr: &gtk::cairo::Context, col: Rgb, a: f64, x: f64, y: f64, w: f64, h: f64) {
    set(cr, col, a);
    rounded(cr, x, y, w.max(0.0), h, h / 2.0);
    let _ = cr.fill();
}

/// The desktop in a theme: its wallpaper, the bar, three windows and a notification.
fn draw_desktop(cr: &gtk::cairo::Context, c: &Colours, wall: Option<&gdk_pixbuf::Pixbuf>, r: R) {
    cr.save().ok();
    rounded(cr, r.x, r.y, r.w, r.h, 10.0);
    cr.clip();
    paint_wall(cr, c, wall, r);
    let bar_h = (r.h * 0.065).clamp(14.0, 24.0);
    set(cr, c.bg, 0.92);
    cr.rectangle(r.x, r.y, r.w, bar_h);
    let _ = cr.fill();
    let s = bar_h * 0.42;
    for i in 0..4 {
        let w = if i == 0 { s * 2.0 } else { s };
        let x = r.x + bar_h * 0.5 + if i == 0 { 0.0 } else { s * 2.4 + (i - 1) as f64 * s * 1.4 };
        set(cr, if i == 0 { c.accent } else { c.text }, if i == 0 { 1.0 } else { 0.35 });
        rounded(cr, x, r.y + (bar_h - s) / 2.0, w, s, s * 0.3);
        let _ = cr.fill();
    }
    set(cr, c.text, 0.92);
    text(cr, &now("%a %H:%M"), r.cx(), r.y + bar_h / 2.0, bar_h * 0.46, true, true);
    for i in 0..4 {
        set(cr, c.text, 0.65);
        cr.arc(r.x + r.w - bar_h * (0.7 + i as f64 * 0.8), r.y + bar_h / 2.0, bar_h * 0.15, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }

    let gap = (r.w * 0.012).clamp(4.0, 9.0);
    let area = R::new(r.x, r.y + bar_h, r.w, r.h - bar_h).inset(gap);
    let (left, right) = area.hsplit(0.5, gap);
    let (top, bottom) = right.vsplit(0.5, gap);
    let radius = (r.w * 0.01).clamp(3.0, 7.0);
    let lh = (r.h * 0.022).clamp(3.0, 6.0);

    // An app with a sidebar, a list and a button.
    let inside = theme_window(cr, c, left, true, radius);
    let side_w = inside.w * 0.28;
    set(cr, mix(c.panel, c.bg, 0.5), 1.0);
    cr.rectangle(inside.x - 2.0, inside.y, side_w, inside.h);
    let _ = cr.fill();
    for i in 0..4 {
        let y = inside.y + 6.0 + i as f64 * lh * 2.2;
        bar_line(cr, if i == 0 { c.accent } else { c.text }, if i == 0 { 0.9 } else { 0.25 }, inside.x + 2.0, y, side_w - 10.0, lh);
    }
    let main_x = inside.x + side_w + 6.0;
    let main_w = inside.w - side_w - 6.0;
    bar_line(cr, c.text, 0.7, main_x, inside.y + 6.0, main_w * 0.7, lh);
    bar_line(cr, c.text, 0.3, main_x, inside.y + 6.0 + lh * 2.0, main_w * 0.45, lh);
    for i in 0..3 {
        let y = inside.y + 6.0 + lh * 4.5 + i as f64 * lh * 3.4;
        set(cr, if i == 0 { mix(c.panel, c.green, 0.35) } else { mix(c.panel, c.text, 0.08) }, 1.0);
        rounded(cr, main_x, y, main_w, lh * 2.6, 2.0);
        let _ = cr.fill();
    }
    set(cr, c.green, 0.9);
    rounded(cr, main_x, inside.y + inside.h - lh * 3.0, main_w * 0.4, lh * 2.4, 2.0);
    let _ = cr.fill();

    // A terminal with the theme's colours in its text.
    let inside = theme_window(cr, c, top, false, radius);
    for (i, (col, w)) in [(c.green, 0.55), (c.warm, 0.65), (c.accent, 0.4), (c.green, 0.1)].iter().enumerate() {
        let y = inside.y + 4.0 + i as f64 * lh * 2.0;
        bar_line(cr, *col, 0.9, inside.x, y, inside.w * 0.12, lh);
        bar_line(cr, c.text, 0.45, inside.x + inside.w * 0.15, y, inside.w * w, lh);
    }

    // A note.
    let inside = theme_window(cr, c, bottom, false, radius);
    for (i, w) in [0.6, 0.9, 0.8, 0.4].iter().enumerate() {
        bar_line(cr, c.text, if i == 0 { 0.7 } else { 0.3 }, inside.x, inside.y + 4.0 + i as f64 * lh * 2.0, inside.w * w, lh);
    }

    // A notification in the corner.
    let tw = (r.w * 0.26).max(90.0);
    let th = bar_h * 1.9;
    let t = R::new(r.x + r.w - tw - gap * 1.6, r.y + bar_h + gap * 1.6, tw, th);
    rounded(cr, t.x, t.y, t.w, t.h, radius);
    set(cr, mix(c.panel, c.text, 0.07), 0.98);
    let _ = cr.fill_preserve();
    set(cr, mix(c.panel, c.text, 0.2), 1.0);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    let ic = th * 0.45;
    set(cr, c.accent, 0.85);
    rounded(cr, t.x + th * 0.28, t.cy() - ic / 2.0, ic, ic, 3.0);
    let _ = cr.fill();
    bar_line(cr, c.text, 0.75, t.x + th * 0.28 + ic + 6.0, t.cy() - lh * 1.2, t.w * 0.5, lh);
    bar_line(cr, c.text, 0.35, t.x + th * 0.28 + ic + 6.0, t.cy() + lh * 0.4, t.w * 0.35, lh);
    cr.restore().ok();
}

/// The lock screen in a theme: the wallpaper dimmed, the time and the password field.
fn draw_lock(cr: &gtk::cairo::Context, c: &Colours, wall: Option<&gdk_pixbuf::Pixbuf>, r: R) {
    cr.save().ok();
    rounded(cr, r.x, r.y, r.w, r.h, 10.0);
    cr.clip();
    paint_wall(cr, c, wall, r);
    set(cr, c.bg, 0.45);
    let _ = cr.paint();
    set(cr, c.text, 0.95);
    text(cr, &now("%H:%M"), r.cx(), r.y + r.h * 0.36, r.h * 0.17, true, true);
    set(cr, c.text, 0.75);
    text(cr, &now("%A %-d %B"), r.cx(), r.y + r.h * 0.48, r.h * 0.045, false, true);
    let (fw, fh) = (r.w * 0.34, r.h * 0.085);
    let f = R::new(r.cx() - fw / 2.0, r.y + r.h * 0.62, fw, fh);
    rounded(cr, f.x, f.y, f.w, f.h, fh / 2.0);
    set(cr, c.bg, 0.85);
    let _ = cr.fill_preserve();
    set(cr, c.accent, 1.0);
    cr.set_line_width(2.0);
    let _ = cr.stroke();
    for i in 0..6 {
        set(cr, c.text, 0.8);
        cr.arc(f.x + fh * 0.7 + i as f64 * fh * 0.42, f.cy(), fh * 0.1, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
    cr.restore().ok();
}

/// Other apps in a theme: a dialog and a terminal over its background.
fn draw_apps(cr: &gtk::cairo::Context, c: &Colours, r: R) {
    cr.save().ok();
    rounded(cr, r.x, r.y, r.w, r.h, 10.0);
    cr.clip();
    set(cr, mix(c.bg, c.text, 0.03), 1.0);
    let _ = cr.paint();
    let lh = (r.h * 0.022).clamp(3.0, 6.0);
    let gap = r.w * 0.04;

    let d = R::new(r.x + gap, r.y + r.h * 0.08, r.w * 0.52, r.h * 0.46);
    let inside = theme_window(cr, c, d, true, 7.0);
    set(cr, c.text, 0.9);
    text(cr, "Save a copy", inside.x, inside.y + 6.0, (r.h * 0.04).clamp(9.0, 13.0), true, false);
    set(cr, mix(c.panel, c.bg, 0.6), 1.0);
    rounded(cr, inside.x, inside.y + inside.h * 0.32, inside.w, inside.h * 0.22, 4.0);
    let _ = cr.fill_preserve();
    set(cr, c.accent, 0.8);
    cr.set_line_width(1.2);
    let _ = cr.stroke();
    bar_line(cr, c.text, 0.6, inside.x + 8.0, inside.y + inside.h * 0.43 - lh / 2.0, inside.w * 0.45, lh);
    let bw = inside.w * 0.26;
    let by = inside.y + inside.h - inside.h * 0.24;
    set(cr, mix(c.panel, c.text, 0.1), 1.0);
    rounded(cr, inside.x + inside.w - bw * 2.0 - 6.0, by, bw, inside.h * 0.22, 4.0);
    let _ = cr.fill();
    set(cr, c.accent, 1.0);
    rounded(cr, inside.x + inside.w - bw, by, bw, inside.h * 0.22, 4.0);
    let _ = cr.fill();

    let t = R::new(r.x + r.w * 0.4, r.y + r.h * 0.6, r.w * 0.55, r.h * 0.34);
    let inside = theme_window(cr, c, t, false, 7.0);
    for (i, (col, w)) in [(c.green, 0.5), (c.accent, 0.62), (c.warm, 0.35), (c.text, 0.55), (c.green, 0.08)].iter().enumerate() {
        let y = inside.y + 4.0 + i as f64 * lh * 2.0;
        bar_line(cr, *col, 0.9, inside.x, y, inside.w * 0.1, lh);
        bar_line(cr, c.text, 0.45, inside.x + inside.w * 0.13, y, inside.w * w, lh);
    }
    cr.restore().ok();
}

/// A theme's swatch: its panel, its background and three of its colours.
fn draw_swatch(cr: &gtk::cairo::Context, c: &Colours, w: f64, h: f64) {
    rounded(cr, 0.0, 0.0, w, h, 7.0);
    cr.clip();
    set(cr, c.bg, 1.0);
    let _ = cr.paint();
    set(cr, c.panel, 1.0);
    cr.rectangle(0.0, 0.0, w * 0.3, h);
    let _ = cr.fill();
    let r = (h * 0.12).clamp(3.0, 5.5);
    for (i, col) in [c.green, c.warm, c.accent].iter().enumerate() {
        set(cr, *col, 1.0);
        cr.arc(w - r * 1.8 - i as f64 * r * 2.6, h - r * 1.9, r, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
}

// ----- The page -----

#[derive(Clone, Copy, PartialEq)]
enum View {
    Desktop,
    Apps,
    Lock,
}

/// What the page shows: the themes, the one in use and the one being tried.
struct State {
    choices: Vec<Choice>,
    current: RefCell<String>,
    preview: RefCell<String>,
    view: Cell<View>,
    omarchy: bool,
    /// The wallpaper showing now, for the theme in use (Omarchy).
    background: RefCell<Option<PathBuf>>,
    listeners: RefCell<Vec<Box<dyn Fn(&State)>>>,
}

impl State {
    fn choice(&self, id: &str) -> Option<&Choice> {
        self.choices.iter().find(|c| c.id == id)
    }
    fn previewing(&self) -> Option<&Choice> {
        self.choice(&self.preview.borrow())
    }
    fn trying(&self) -> bool {
        *self.preview.borrow() != *self.current.borrow()
    }
    fn name(&self, id: &str) -> String {
        self.choice(id).map(|c| c.name.clone()).unwrap_or_else(|| id.to_string())
    }
    fn listen(&self, f: impl Fn(&State) + 'static) {
        self.listeners.borrow_mut().push(Box::new(f));
    }
    fn changed(&self) {
        for f in self.listeners.borrow().iter() {
            f(self);
        }
    }
    /// The colours to draw the preview in: the theme tried, else the app's own.
    fn colours(&self) -> Colours {
        self.previewing().and_then(|c| c.colours).unwrap_or_else(|| {
            let p = sketch::pal();
            Colours { bg: p.bg, panel: p.surface, text: p.text, accent: p.accent, warm: p.danger, green: p.accent }
        })
    }
    fn wallpaper(&self) -> Option<PathBuf> {
        if !self.trying() && self.background.borrow().is_some() {
            return self.background.borrow().clone();
        }
        self.previewing().and_then(|c| c.wallpaper.clone())
    }
}

pub fn build(page: &Page) {
    let omarchy = super::themes::omarchy_chooses();
    let (choices, current) = if omarchy { omarchy_choices() } else { super::themes::nyx_choices() };
    let st = Rc::new(State {
        choices,
        preview: RefCell::new(current.clone()),
        current: RefCell::new(current),
        view: Cell::new(View::Desktop),
        omarchy,
        background: RefCell::new(if omarchy { current_background() } else { None }),
        listeners: RefCell::default(),
    });

    preview(page, &st);
    themes_group(page, &st);
    if crate::platform::PLATFORM == crate::platform::Platform::Omarchy {
        wallpaper_group(page, &st);
    }
    fonts_group(page, &st);
    more_group(page);
    if !omarchy {
        // A theme chosen elsewhere (another app, the command line) moves the marks too.
        let st2 = st.clone();
        nyx_theme::subscribe(&page.body, move || {
            let active = nyx_theme::engine::get_active_theme();
            if *st2.current.borrow() != active {
                let was_trying = st2.trying();
                *st2.current.borrow_mut() = active.clone();
                if !was_trying {
                    *st2.preview.borrow_mut() = active;
                }
                st2.changed();
            }
        });
    }
    st.changed();
}

// ----- The preview: the theme tried on a desktop, Keep and Apply -----

fn preview(page: &Page, st: &Rc<State>) {
    let pv = page.split();
    let views = widgets::opts(&[("desktop", "Desktop"), ("apps", "Apps"), ("lock", "Lock screen")]);
    let stage_slot: Rc<RefCell<Option<gtk::DrawingArea>>> = Rc::default();
    let seg = {
        let (st, slot) = (st.clone(), stage_slot.clone());
        widgets::segmented(&views, "desktop", move |v| {
            st.view.set(match v.as_str() {
                "apps" => View::Apps,
                "lock" => View::Lock,
                _ => View::Desktop,
            });
            if let Some(a) = slot.borrow().as_ref() {
                a.queue_draw();
            }
        })
    };
    let head = pv.head("", "", &[seg.upcast_ref()]);
    let title = head.first_child().and_then(|t| t.first_child()).and_downcast::<gtk::Label>();
    let line = widgets::label("", "preview-line");
    line.set_wrap(true);
    if let Some(text) = head.first_child().and_downcast::<gtk::Box>() {
        text.append(&line);
    }
    st.listen(move |st| {
        let name = st.previewing().map(|c| c.name.clone()).unwrap_or_default();
        if let Some(t) = &title {
            t.set_text(&if st.trying() { format!("Previewing {name}") } else { name });
        }
        line.set_text(if st.trying() { "Nothing changes until you apply it." } else { "In use. Pick a theme to try it here first." });
    });

    let stage = pv.drawing(300, |_, _, _, _, _| {});
    {
        let (st, weak) = (st.clone(), stage.downgrade());
        stage.set_draw_func(move |_, cr, w, h| {
            let r = screen_rect(w as f64, h as f64);
            let c = st.colours();
            let weak = weak.clone();
            let redraw = move || {
                if let Some(a) = weak.upgrade() {
                    a.queue_draw();
                }
            };
            let wall = st.wallpaper().and_then(|p| wall_pixbuf(&p, redraw));
            match st.view.get() {
                View::Desktop => draw_desktop(cr, &c, wall.as_ref(), r),
                View::Lock => draw_lock(cr, &c, wall.as_ref(), r),
                View::Apps => draw_apps(cr, &c, r),
            }
        });
    }
    stage.add_css_class("ap-stage");
    fit_height(&stage, 1.6, 290);
    *stage_slot.borrow_mut() = Some(stage.clone());
    st.listen(move |_| stage.queue_draw());

    sample_card(page, &pv, st);
    apply_card(&pv, st);
}

/// Keep a drawing `ratio` times as wide as it is tall, as the preview's width changes,
/// and no taller than `max`; lower when the preview spans a narrow window, above the controls.
pub(crate) fn fit_height(area: &gtk::DrawingArea, ratio: f64, max: i32) {
    area.connect_resize(move |a, w, _| {
        let narrow = std::iter::successors(a.parent(), |p| p.parent()).any(|p| p.has_css_class("narrow"));
        let max = if narrow { max.min(280) } else { max };
        let h = ((w as f64 / ratio) as i32).min(max).max(160);
        if a.content_height() != h {
            let a = a.clone();
            glib::idle_add_local_once(move || a.set_content_height(h));
        }
    });
}

fn font_attrs(desc: &str, size: f64) -> pango::AttrList {
    let mut d = pango::FontDescription::from_string(desc);
    d.set_size((size * pango::SCALE as f64) as i32);
    let attrs = pango::AttrList::new();
    attrs.insert(pango::AttrFontDesc::new(&d));
    attrs
}

thread_local! {
    /// The sample card's fonts, set from the font rows.
    static SAMPLE: RefCell<Option<(gtk::Label, gtk::Label)>> = const { RefCell::new(None) };
}

fn set_sample_font(mono: bool, desc: &str) {
    SAMPLE.with(|s| {
        if let Some((ui, code)) = s.borrow().as_ref() {
            let family = pango::FontDescription::from_string(desc).family().map(|f| f.to_string()).unwrap_or_default();
            (if mono { code } else { ui }).set_attributes(Some(&font_attrs(&family, 11.0)));
        }
    });
}

fn css_hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", (c.0 * 255.0).round() as u8, (c.1 * 255.0).round() as u8, (c.2 * 255.0).round() as u8)
}

/// The fonts and icons in the tried theme's colours.
fn sample_card(page: &Page, pv: &widgets::Preview, st: &Rc<State>) {
    let card = widgets::hbox(18);
    card.add_css_class("ap-sample");
    let ui = widgets::label("The quick brown fox", "ap-sample-text");
    let code = widgets::label("fn main() { 42 }", "ap-sample-text");
    for (cap, l) in [("Interface", &ui), ("Monospace", &code)] {
        let col = widgets::vbox(2);
        col.append(&widgets::label(cap, "ap-sample-cap"));
        col.append(l);
        card.append(&col);
    }
    let icons = widgets::hbox(8);
    icons.set_hexpand(true);
    icons.set_halign(gtk::Align::End);
    icons.set_valign(gtk::Align::Center);
    for name in ["folder", "text-x-generic", "image-x-generic", "audio-x-generic"] {
        let i = gtk::Image::from_icon_name(name);
        i.set_pixel_size(28);
        icons.append(&i);
    }
    card.append(&icons);
    SAMPLE.with(|s| *s.borrow_mut() = Some((ui, code)));
    set_sample_font(false, &appearance::interface_font());
    let mono = if st.omarchy { cmd::output(&["omarchy-font-current"]).map(|s| s.trim().to_string()).unwrap_or_default() } else { String::new() };
    set_sample_font(true, if mono.is_empty() { "monospace" } else { &mono });

    let provider = gtk::CssProvider::new();
    #[allow(deprecated)]
    card.style_context().add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
    st.listen(move |st| {
        let c = st.colours();
        provider.load_from_string(&format!(".ap-sample {{ background: {}; color: {}; }}", css_hex(c.panel), css_hex(c.text)));
    });
    pv.add(&card);
    // At 800 px the preview sits above the controls: keep it short there.
    let body = page.body.clone();
    let c2 = card.clone();
    let fit = move |b: &gtk::Box| c2.set_visible(!b.has_css_class("narrow"));
    fit(&body);
    body.connect_css_classes_notify(fit);
}

/// The tried theme beside Keep and Apply; the theme in use when nothing is tried.
fn apply_card(pv: &widgets::Preview, st: &Rc<State>) {
    let card = widgets::hbox(10);
    card.add_css_class("ap-apply");
    let swatch = gtk::DrawingArea::new();
    swatch.set_content_width(44);
    swatch.set_content_height(28);
    swatch.set_valign(gtk::Align::Center);
    {
        let st = st.clone();
        swatch.set_draw_func(move |_, cr, w, h| draw_swatch(cr, &st.colours(), w as f64, h as f64));
    }
    card.append(&swatch);
    let text = widgets::vbox(1);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let name = widgets::label("", "ap-apply-name");
    name.set_ellipsize(pango::EllipsizeMode::End);
    let note = widgets::label("", "ap-apply-note");
    text.append(&name);
    text.append(&note);
    card.append(&text);
    let keep = gtk::Button::new();
    keep.add_css_class("flat");
    keep.set_valign(gtk::Align::Center);
    let apply = gtk::Button::new();
    apply.add_css_class("suggested-action");
    apply.set_valign(gtk::Align::Center);
    card.append(&keep);
    card.append(&apply);
    {
        let st = st.clone();
        keep.connect_clicked(move |_| {
            let current = st.current.borrow().clone();
            *st.preview.borrow_mut() = current;
            st.changed();
        });
    }
    {
        let st = st.clone();
        apply.connect_clicked(move |b| apply_theme(&st, b));
    }
    {
        let (swatch, keep, apply) = (swatch.clone(), keep.clone(), apply.clone());
        st.listen(move |st| {
            swatch.queue_draw();
            let tried = st.preview.borrow().clone();
            name.set_text(&st.name(&tried));
            note.set_text(if st.trying() { "Not applied yet" } else { "In use" });
            keep.set_label(&format!("Keep {}", st.name(&st.current.borrow())));
            apply.set_label(&format!("Apply {}", st.name(&tried)));
            keep.set_visible(st.trying());
            apply.set_visible(st.trying());
        });
    }
    pv.add(&card);
}

fn apply_theme(st: &Rc<State>, b: &gtk::Button) {
    let id = st.preview.borrow().clone();
    let name = st.name(&id);
    if st.omarchy {
        b.set_sensitive(false);
        b.set_label("Applying…");
        let (st, b, arg) = (st.clone(), b.clone(), id.clone());
        cmd::run_async(&["omarchy-theme-set", &arg], move |r| {
            b.set_sensitive(true);
            match r {
                Ok(_) => {
                    *st.current.borrow_mut() = id;
                    st.changed();
                    window::toast(&format!("Theme set to {name}"));
                    // The wallpapers are the new theme's now.
                    glib::timeout_add_local_once(std::time::Duration::from_millis(400), || window::rebuild("theme"));
                }
                Err(e) => {
                    st.changed();
                    window::toast(&format!("Couldn't set theme: {e}"));
                }
            }
        });
    } else {
        match nyx_theme::choose_system_theme(&id) {
            Ok(()) => {
                *st.current.borrow_mut() = id;
                st.changed();
                window::toast(&format!("Theme set to {name}"));
            }
            Err(e) => window::toast(&e),
        }
    }
}

// ----- Theme cards -----

/// How many cards show before "Show all".
const FIRST: usize = 8;

fn theme_card(choice: &Choice) -> (gtk::Button, gtk::Label) {
    let card = gtk::Button::new();
    card.add_css_class("ap-card");
    card.set_tooltip_text(Some(&choice.name));
    let v = widgets::vbox(3);
    let frame = widgets::hbox(0);
    frame.add_css_class("ap-sw");
    match choice.colours {
        Some(c) => {
            let d = gtk::DrawingArea::new();
            d.set_content_height(42);
            d.set_hexpand(true);
            d.set_draw_func(move |_, cr, w, h| draw_swatch(cr, &c, w as f64, h as f64));
            frame.append(&d);
        }
        None => {
            let pic = gtk::Picture::new();
            pic.set_content_fit(gtk::ContentFit::Cover);
            pic.set_size_request(-1, 42);
            pic.set_can_shrink(true);
            pic.set_hexpand(true);
            if let Some(p) = choice.picture.clone().or_else(|| choice.wallpaper.clone()) {
                load_thumbnail(&pic, p);
            }
            frame.append(&pic);
        }
    }
    frame.set_overflow(gtk::Overflow::Hidden);
    v.append(&frame);
    let name = widgets::label(&choice.name, "ap-card-name");
    name.set_ellipsize(pango::EllipsizeMode::End);
    v.append(&name);
    let state = widgets::label("", "ap-card-state");
    v.append(&state);
    card.set_child(Some(&v));
    (card, state)
}

/// A group's title with something at its right (a link, buttons).
fn heading(title: &str, end: &[&gtk::Widget]) -> gtk::Box {
    let h = widgets::hbox(6);
    h.add_css_class("ap-heading");
    let l = widgets::label(&title.to_uppercase(), "group-title");
    l.set_hexpand(true);
    h.append(&l);
    for w in end {
        w.set_valign(gtk::Align::Center);
        h.append(*w);
    }
    h
}

fn themes_group(page: &Page, st: &Rc<State>) {
    let g = page.plain_group("");
    g.wrapper.add_css_class("ap-group");
    let total = st.choices.len();
    let more = gtk::Button::new();
    more.add_css_class("flat");
    more.add_css_class("ap-link");
    let more_label = gtk::Label::new(Some(&format!("Show all {total}")));
    let more_box = widgets::hbox(4);
    more_box.append(&more_label);
    more_box.append(&gtk::Image::from_icon_name("go-next-symbolic"));
    more.set_child(Some(&more_box));
    more.set_visible(total > FIRST);
    g.add(&heading("Theme", &[more.upcast_ref()]));

    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_max_children_per_line(4);
    flow.set_min_children_per_line(4);
    flow.set_row_spacing(10);
    flow.set_column_spacing(8);
    flow.set_homogeneous(true);
    let cards: Rc<RefCell<Vec<(String, gtk::Button, gtk::Label, gtk::FlowBoxChild)>>> = Rc::default();
    for choice in &st.choices {
        let (card, state) = theme_card(choice);
        let id = choice.id.clone();
        let st2 = st.clone();
        card.connect_clicked(move |_| {
            *st2.preview.borrow_mut() = id.clone();
            st2.changed();
        });
        flow.insert(&card, -1);
        if let Some(child) = flow.last_child().and_downcast::<gtk::FlowBoxChild>() {
            child.set_focusable(false);
            cards.borrow_mut().push((choice.id.clone(), card, state, child));
        }
    }
    let show_all = Rc::new(Cell::new(false));
    let mark = {
        let (cards, show_all) = (cards.clone(), show_all.clone());
        move |st: &State| {
            let (current, tried) = (st.current.borrow().clone(), st.preview.borrow().clone());
            for (i, (id, card, state, child)) in cards.borrow().iter().enumerate() {
                let in_use = *id == current;
                let trying = *id == tried && st.trying();
                for (class, on) in [("in-use", in_use), ("trying", trying)] {
                    if on {
                        card.add_css_class(class);
                    } else {
                        card.remove_css_class(class);
                    }
                }
                state.set_text(if trying {
                    "Previewing"
                } else if in_use {
                    "In use"
                } else {
                    ""
                });
                child.set_visible(show_all.get() || i < FIRST || in_use || *id == tried);
            }
        }
    };
    st.listen(mark.clone());
    {
        let st = st.clone();
        more.connect_clicked(move |_| {
            show_all.set(!show_all.get());
            more_label.set_text(&if show_all.get() { "Show fewer".to_string() } else { format!("Show all {total}") });
            mark(&st);
        });
    }
    let row = widgets::stacked_row("", "", flow.upcast_ref());
    row.add_css_class("bare");
    widgets::keywords(&format!("theme colours colors try preview apply keep {}", st.choices.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(" ")));
    g.add(&row);
}

// ----- Wallpaper (Omarchy) -----

fn wallpaper_group(page: &Page, st: &Rc<State>) {
    let g = page.plain_group("");
    g.wrapper.add_css_class("ap-group");
    let next = gtk::Button::new();
    next.add_css_class("flat");
    let nb = widgets::hbox(6);
    nb.append(&gtk::Image::from_icon_name("view-refresh-symbolic"));
    nb.append(&gtk::Label::new(Some("Next")));
    next.set_child(Some(&nb));
    let choose = widgets::labeled_button("image-x-generic-symbolic", "Choose…");
    choose.connect_clicked(|_| cmd::spawn(&["omarchy-menu", "toggle", "background"]));
    g.add(&heading("Wallpaper", &[next.upcast_ref(), choose.upcast_ref()]));

    let walls = current_wallpapers();
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_max_children_per_line(5);
    flow.set_min_children_per_line(5);
    flow.set_row_spacing(8);
    flow.set_column_spacing(8);
    flow.set_homogeneous(true);
    let buttons: Rc<RefCell<Vec<(PathBuf, gtk::Button)>>> = Rc::default();
    for path in &walls {
        let b = gtk::Button::new();
        b.add_css_class("ap-wall");
        let pic = gtk::Picture::new();
        pic.set_content_fit(gtk::ContentFit::Cover);
        pic.set_size_request(-1, 40);
        pic.set_can_shrink(true);
        load_thumbnail(&pic, path.clone());
        b.set_child(Some(&pic));
        b.set_overflow(gtk::Overflow::Hidden);
        b.set_tooltip_text(path.file_stem().and_then(|s| s.to_str()));
        let (st, p) = (st.clone(), path.clone());
        b.connect_clicked(move |_| {
            let (st, p2) = (st.clone(), p.clone());
            cmd::run_async(&["omarchy-theme-bg-set", &p.to_string_lossy()], move |r| match r {
                Ok(_) => {
                    *st.background.borrow_mut() = Some(p2);
                    st.changed();
                }
                Err(e) => window::toast(&format!("Couldn't set the wallpaper: {e}")),
            });
        });
        flow.insert(&b, -1);
        if let Some(child) = flow.last_child() {
            child.set_focusable(false);
        }
        buttons.borrow_mut().push((path.clone(), b));
    }
    {
        let st = st.clone();
        next.connect_clicked(move |_| {
            let st = st.clone();
            cmd::run_async(&["omarchy-theme-bg-next"], move |_| {
                *st.background.borrow_mut() = current_background();
                st.changed();
            });
        });
    }
    st.listen(move |st| {
        let now = st.background.borrow().clone();
        for (p, b) in buttons.borrow().iter() {
            if Some(p) == now.as_ref() {
                b.add_css_class("selected");
            } else {
                b.remove_css_class("selected");
            }
        }
    });
    let content: gtk::Widget = if walls.is_empty() {
        widgets::label("This theme has no wallpapers of its own; Choose… picks any picture.", "dim").upcast()
    } else {
        flow.upcast()
    };
    let row = widgets::stacked_row("", "", &content);
    row.add_css_class("bare");
    widgets::keywords("wallpaper background picture next choose cycle");
    g.add(&row);
}

// ----- Fonts and icons -----

fn with_tile(r: &gtk::Box, icon: &str) {
    r.prepend(&widgets::tile(icon));
    r.add_css_class("with-tile");
}

fn fonts_group(page: &Page, st: &Rc<State>) {
    let g = page.group("Fonts & icons");
    g.add(&interface_font_row());
    if st.omarchy {
        let fonts: Vec<(String, String)> = cmd::output(&["omarchy-font-list"])
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|f| (f.to_string(), f.to_string()))
            .collect();
        let current_font = cmd::output(&["omarchy-font-current"]).unwrap_or_default();
        let (r, _) = widgets::choice_row("Monospace font", "", fonts, current_font.trim(), |font| {
            set_sample_font(true, &font);
            cmd::run_async(&["omarchy-font-set", &font], move |r| {
                if let Err(e) = r {
                    window::toast(&format!("Couldn't set font: {e}"));
                }
            });
        });
        with_tile(&r, "utilities-terminal-symbolic");
        widgets::keywords("monospace font terminal bar lock screen code");
        g.add(&r);
    }
    icons_rows(&g);
}

/// A row under the fonts that opens a page of its own.
fn more_group(page: &Page) {
    let g = page.group("");
    let installed = chroma::installed();
    let synced = installed && !chroma::state().theme.is_empty();
    let (r, _) = widgets::nav_row(
        Some("view-app-grid-symbolic"),
        "Apps follow the theme",
        "",
        if !installed {
            "Not set up"
        } else if synced {
            "Synced"
        } else {
            "Not synced"
        },
        || open(widgets::subpage("Apps follow the theme", 600, apps_group)),
    );
    widgets::keywords("gtk qt kde libadwaita apps windows dialogs popups consistent dark reader hyprchroma omarchroma sync automatic restore stock");
    g.add(&r);

    let added = super::themes::added_count();
    let (r, _) = widgets::nav_row(
        Some("applications-graphics-symbolic"),
        "Themes and app themes",
        "",
        &if added == 0 { String::new() } else { format!("{added} added") },
        || {
            open(widgets::subpage("Themes and app themes", 640, |p| {
                super::themes::library_group(p);
                super::themes::install_group(p);
                super::themes::apps_group(p);
            }))
        },
    );
    widgets::keywords(
        "theme library installed themes update remove details author licence install github url repository download add import app theme per app override follow system each app",
    );
    g.add(&r);

    let nyxos = crate::platform::PLATFORM == crate::platform::Platform::NyxOS;
    let (r, _) = widgets::nav_row(
        Some("emblem-system-symbolic"),
        "Settings window and defaults",
        if nyxos { "This window's theme, reset" } else { "Logo, this window's theme, reset" },
        "",
        move || {
            open(widgets::subpage("Settings window and defaults", 600, move |p| {
                // On NyxOS the logo is a system setting, under Boot & Sign-in.
                if !nyxos {
                    super::themes::logo_group(p);
                }
                settings_window_group(p);
                let g = p.group("Defaults");
                super::themes::reset_group(&g);
            }))
        },
    );
    widgets::keywords(
        "logo brand mark emblem settings theme accent colour color swatch highlight reduce motion animations temperature unit fahrenheit celsius degrees start startup launch first page home reset default appearance restore",
    );
    g.add(&r);
}

/// A page one level down: its rows aren't the main page's, so search leaves them out.
fn open(dialog: gtk::Window) {
    widgets::forget_rows(&dialog);
}

fn settings_window_group(page: &Page) {
    let g = page.group("Settings window");
    let p = prefs::get();
    let system = theme::engine::resolve_effective_theme(None, &theme::engine::suite(), &theme::A11y::default()).meta.name;
    let mut options = vec![(String::new(), format!("Follow system ({system})"))];
    options.extend(theme::engine::list_themes().into_iter().map(|m| (m.id, m.name)));
    let (theme_row, _) = widgets::choice_row(
        "Settings theme",
        "Settings follows the system theme unless you give it one of its own. Add your own themes in <tt>~/.config/nyx/themes</tt>.",
        options,
        &theme::app_override().unwrap_or_default(),
        |id| {
            if let Err(e) = theme::set_app_theme((!id.is_empty()).then_some(id.as_str())) {
                window::toast(&e);
            }
        },
    );
    g.add(&theme_row);

    g.add(&nyx_ui::widgets::accent_row("settings"));

    let (r, _) =
        widgets::switch_row("Reduce motion", "Turn off transitions and animations in this window.", p.reduce_motion, |on| {
            prefs::update(|p| p.reduce_motion = on);
            theme::apply();
        });
    g.add(&r);
    let unit = if prefs::get().temp_unit == prefs::TempUnit::Fahrenheit { "f" } else { "c" };
    let (r, _) = widgets::choice_row(
        "Temperature unit",
        "How temperatures are shown, such as fan curves and GPU limits.",
        widgets::opts(&[("c", "Celsius (°C)"), ("f", "Fahrenheit (°F)")]),
        unit,
        |v| {
            prefs::update(|p| p.temp_unit = if v == "f" { prefs::TempUnit::Fahrenheit } else { prefs::TempUnit::Celsius });
            window::rebuild_if_built("asus");
        },
    );
    g.add(&r);
    // Shared by every Nyx app, like the theme.
    g.add(&nyx_ui::widgets::start_row());
}

// ----- Apps follow the theme (hyprchroma) -----

const HYPRCHROMA_URL: &str = "https://github.com/NobleDoodle/hyprchroma";

/// "Synced for <i>Catppuccin</i> · 2 min ago"
fn status_text(st: &chroma::State) -> String {
    if st.theme.is_empty() {
        return "Not synced yet.".into();
    }
    let ago = chroma::ago(&st.last_sync);
    let name = glib::markup_escape_text(&st.theme);
    if ago.is_empty() { format!("Synced for <i>{name}</i>") } else { format!("Synced for <i>{name}</i> · {ago}") }
}

/// The description label of a `widgets::row`.
fn row_desc(row: &gtk::Box) -> Option<gtk::Label> {
    row.first_child()?.last_child()?.downcast::<gtk::Label>().ok()
}

fn apps_group(page: &Page) {
    let g = page.group("Apps follow the theme");
    if !chroma::installed() {
        g.add(&widgets::banner(
            "Install <b>hyprchroma</b> to give GTK, libadwaita, Qt and KDE apps the Omarchy theme's colours, \
             so every window and dialog matches.",
            false,
        ));
        let (r, _) = widgets::button_row("hyprchroma", "Not installed.", "Install instructions…", |_| {
            cmd::spawn(&["xdg-open", HYPRCHROMA_URL]);
        });
        g.add(&r);
        return;
    }
    g.note("Windows and dialogs in other apps use the Omarchy theme's colours, and change with it. Powered by hyprchroma.");

    let st = chroma::state();
    let status_row = widgets::row("Status", &status_text(&st), None);
    let refresh = {
        let status_row = status_row.clone();
        move || {
            if let Some(l) = row_desc(&status_row) {
                l.set_markup(&status_text(&chroma::state()));
            }
        }
    };
    let sync = gtk::Button::with_label("Sync now");
    sync.set_valign(gtk::Align::Center);
    {
        let refresh = refresh.clone();
        sync.connect_clicked(move |b| {
            b.set_sensitive(false);
            let (b, refresh) = (b.clone(), refresh.clone());
            cmd::background(chroma::sync_now, move |r| {
                b.set_sensitive(true);
                refresh();
                match r {
                    Ok(_) => window::toast("Apps re-themed"),
                    Err(e) => window::toast(&format!("Couldn't sync: {e}")),
                }
            });
        });
    }
    status_row.append(&sync);
    g.add(&status_row);

    for (key, on) in &st.frameworks {
        let Some(t) = chroma::target(key) else { continue };
        let refresh = refresh.clone();
        let (r, _) = widgets::switch_row(t.title, t.desc, *on, move |on| {
            let refresh = refresh.clone();
            cmd::background(
                move || chroma::set_enabled(t, on),
                move |r| {
                    refresh();
                    if let Err(e) = r {
                        window::toast(&format!("Couldn't change {}: {e}", t.title));
                    }
                },
            );
        });
        if st.status.iter().any(|(k, v)| k == key && v == "not-installed") {
            widgets::tag_row(&r, "App not installed");
        }
        g.add(&r);
    }

    let removed: Vec<&'static chroma::Target> = st.removed.iter().filter_map(|k| chroma::target(k)).collect();
    if !removed.is_empty() {
        let names = removed.iter().map(|t| t.title).collect::<Vec<_>>().join(", ");
        let (r, _) = widgets::button_row("Hidden", &format!("{names}: removed from hyprchroma."), "Restore", move |b| {
            b.set_sensitive(false);
            let removed = removed.clone();
            cmd::background(
                move || removed.iter().try_for_each(|t| chroma::restore_framework(t).map(|_| ())),
                |r| match r {
                    Ok(()) => window::rebuild("theme"),
                    Err(e) => window::toast(&format!("Couldn't restore: {e}")),
                },
            );
        });
        g.add(&r);
    }

    let (r, _) = widgets::switch_row(
        "Automatic sync",
        "Re-theme apps in the background whenever the Omarchy theme or font changes.",
        chroma::service_active(),
        |on| {
            cmd::background(
                move || chroma::set_service(on),
                |r| {
                    if let Err(e) = r {
                        window::toast(&format!("Couldn't change automatic sync: {e}"));
                    }
                },
            );
        },
    );
    g.add(&r);

    // Apps that need a restart; filled in once hyprchroma answers.
    let stale = widgets::vbox(6);
    stale.set_visible(false);
    g.add(&stale);
    {
        let stale = stale.clone();
        cmd::background(chroma::stale_apps, move |apps| {
            for app in &apps {
                stale.append(&widgets::row(app, "Still showing the old theme. Restart it to update.", None));
            }
            stale.set_visible(!apps.is_empty());
        });
    }

    if chroma::plugin_installed() {
        let b = widgets::confirm_button("Remove", "Click again to remove", |b| {
            b.set_sensitive(false);
            cmd::run_async(&["omarchy-plugin-remove", chroma::PLUGIN, "--yes"], |r| match r {
                Ok(_) => {
                    window::toast("Omarchroma removed");
                    window::rebuild("theme");
                }
                Err(e) => window::toast(&format!("Couldn't remove Omarchroma: {e}")),
            });
        });
        b.add_css_class("destructive-action");
        g.add(&widgets::row(
            "Omarchroma bar widget",
            "Settings covers everything it does. Removing it keeps hyprchroma and automatic sync.",
            Some(b.upcast_ref()),
        ));
    }

    let (adv, content) = widgets::disclosure("Advanced", "");
    let b = widgets::confirm_button("Restore", "Click again to restore", move |b| {
        b.set_sensitive(false);
        let (refresh, b) = (refresh.clone(), b.clone());
        cmd::background(chroma::restore_stock, move |r| {
            refresh();
            b.set_sensitive(true);
            match r {
                Ok(_) => window::toast("Stock look restored"),
                Err(e) => window::toast(&format!("Couldn't restore: {e}")),
            }
        });
    });
    b.add_css_class("destructive-action");
    content.append(&widgets::row(
        "Restore stock look",
        "Undo hyprchroma's changes to GTK and KDE files and put back the default styling. Sync again to re-theme.",
        Some(b.upcast_ref()),
    ));
    let source = widgets::row("Colours from", "", None);
    content.append(&source);
    cmd::background(chroma::palette_source, move |src| {
        if let Some(t) = source.first_child()
            && let Ok(t) = t.downcast::<gtk::Box>()
        {
            let d = widgets::label(if src.is_empty() { "Unknown" } else { &src }, "settings-option-description");
            t.append(&d);
        }
    });
    g.add(&adv);
}

// ----- Icons and interface font -----

const FOLLOW: &str = "";

fn icons_rows(g: &widgets::Group) {
    let cfg = appearance::load();
    let follow_label = match appearance::omarchy_icon_theme() {
        Some(t) => format!("Follow the theme ({t})"),
        None => "Follow the theme".into(),
    };
    let mut options = vec![(FOLLOW.to_string(), follow_label)];
    options.extend(appearance::icon_themes().into_iter().map(|t| (t.clone(), t)));
    let current = appearance::chosen(&cfg).unwrap_or_default();

    // Each icon set's Look, shown while that set is the icon theme. App icons take the
    // look's style (`nyx icons` follows the set's settings).
    let look_rows: Vec<(&'static str, gtk::Box)> = appearance::SETS
        .iter()
        .map(|set| {
            let looks: Vec<(String, String)> =
                set.looks.iter().map(|(id, label, _)| (id.to_string(), label.to_string())).collect();
            let (row, _) = widgets::choice_row("Icon look", set.about, looks, &set.look_or_default(), move |id| {
                cmd::background(
                    move || set.use_look(&id),
                    |r: anyhow::Result<()>| {
                        if let Err(e) = r {
                            window::toast(&format!("Couldn't change the look: {e}"));
                        }
                    },
                );
            });
            with_tile(&row, "preferences-desktop-appearance-symbolic");
            let ids: Vec<&str> = set.looks.iter().map(|(id, _, _)| *id).collect();
            widgets::keywords(&format!("{} icons icon theme look {}", set.id.to_lowercase(), ids.join(" ")));
            row.set_visible(current == set.id);
            (set.id, row)
        })
        .collect();

    let rows = look_rows.clone();
    let (r, _) = widgets::choice_row("Icons", "", options, &current, move |id| {
        for (set, row) in &rows {
            row.set_visible(*set == id);
        }
        let set = appearance::set_of(&id);
        let c = appearance::Config { icon_theme: (id != FOLLOW).then_some(id) };
        cmd::background(
            move || {
                appearance::save(&c)?;
                crate::backend::themehook::ensure();
                appearance::leave_sets(set.map(|s| s.id));
                match set {
                    Some(s) => s.use_look(&s.look_or_default()),
                    None => appearance::apply_icons(&c),
                }
            },
            |r: anyhow::Result<()>| {
                if let Err(e) = r {
                    window::toast(&format!("Couldn't set icons: {e}"));
                }
            },
        );
    });
    with_tile(&r, "folder-symbolic");
    widgets::keywords("icon theme icons yaru papirus adwaita umbra vellum look apps file choosers");
    g.add(&r);
    for (set, (_, row)) in appearance::SETS.iter().zip(look_rows) {
        if set.installed() || current == set.id {
            g.add(&row);
        }
    }
}

fn interface_font_row() -> gtk::Box {
    let button = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    button.set_font_desc(&pango::FontDescription::from_string(&appearance::interface_font()));
    button.set_use_font(true);
    button.set_use_size(true);
    // A long family name shortens, so the row's title keeps its line.
    let mut stack = vec![button.clone().upcast::<gtk::Widget>()];
    while let Some(w) = stack.pop() {
        if let Some(l) = w.downcast_ref::<gtk::Label>() {
            l.set_ellipsize(pango::EllipsizeMode::End);
            l.set_max_width_chars(16);
        }
        stack.extend(std::iter::successors(w.first_child(), |c| c.next_sibling()));
    }
    button.set_tooltip_text(Some(&appearance::interface_font()));
    let restore = gtk::Button::from_icon_name("edit-undo-symbolic");
    restore.add_css_class("flat");
    restore.set_valign(gtk::Align::Center);
    restore.set_tooltip_text(Some(&format!("Back to {}", appearance::DEFAULT_INTERFACE_FONT)));
    restore.set_visible(appearance::interface_font() != appearance::DEFAULT_INTERFACE_FONT);
    {
        let button = button.clone();
        restore.connect_clicked(move |r| {
            let (button, r) = (button.clone(), r.clone());
            cmd::background(appearance::restore_fonts, move |res| match res {
                Ok(()) => {
                    button.set_font_desc(&pango::FontDescription::from_string(appearance::DEFAULT_INTERFACE_FONT));
                    button.set_tooltip_text(Some(appearance::DEFAULT_INTERFACE_FONT));
                    set_sample_font(false, appearance::DEFAULT_INTERFACE_FONT);
                    r.set_visible(false);
                }
                Err(e) => window::toast(&format!("Couldn't set font: {e}")),
            });
        });
    }
    let r = restore.clone();
    button.connect_font_desc_notify(move |b| {
        let Some(desc) = b.font_desc() else { return };
        let desc = desc.to_string();
        if desc == appearance::interface_font() {
            return;
        }
        r.set_visible(desc != appearance::DEFAULT_INTERFACE_FONT);
        b.set_tooltip_text(Some(&desc));
        set_sample_font(false, &desc);
        cmd::background(
            move || appearance::set_interface_font(&desc),
            |r| {
                if let Err(e) = r {
                    window::toast(&format!("Couldn't set font: {e}"));
                }
            },
        );
    });
    let end = widgets::hbox(4);
    end.append(&restore);
    end.append(&button);
    let r = widgets::icon_row("font-x-generic-symbolic", "Interface font", "", Some(end.upcast_ref()));
    widgets::keywords("interface font ui text gtk qt size app windows menus dialogs restore reset default inter");
    r
}
