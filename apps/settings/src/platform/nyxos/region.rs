//! NyxOS's Time & Language: the time zone, language and formats, the keyboard layout
//! and this computer's name, applied together as one rebuild. Beside them, the picked
//! zone on a world map with the time there, the formats in the picked language, and
//! the name as the network sees it.

use super::pending::{Pending, RowMark};
use crate::platform::nyxos::{self, mark_system};
use crate::sketch;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use nyx_system_proto::{Change, valid_hostname};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

fn options(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

const LOCALES: &[(&str, &str)] = &[
    ("en_US.UTF-8", "English (US)"),
    ("en_GB.UTF-8", "English (UK)"),
    ("en_CA.UTF-8", "English (Canada)"),
    ("en_AU.UTF-8", "English (Australia)"),
    ("de_DE.UTF-8", "Deutsch (Deutschland)"),
    ("fr_FR.UTF-8", "Français (France)"),
    ("es_ES.UTF-8", "Español (España)"),
    ("es_MX.UTF-8", "Español (México)"),
    ("it_IT.UTF-8", "Italiano"),
    ("pt_PT.UTF-8", "Português (Portugal)"),
    ("pt_BR.UTF-8", "Português (Brasil)"),
    ("nl_NL.UTF-8", "Nederlands"),
    ("sv_SE.UTF-8", "Svenska"),
    ("nb_NO.UTF-8", "Norsk bokmål"),
    ("da_DK.UTF-8", "Dansk"),
    ("fi_FI.UTF-8", "Suomi"),
    ("pl_PL.UTF-8", "Polski"),
    ("cs_CZ.UTF-8", "Čeština"),
    ("tr_TR.UTF-8", "Türkçe"),
    ("ru_RU.UTF-8", "Русский"),
    ("uk_UA.UTF-8", "Українська"),
    ("ja_JP.UTF-8", "日本語"),
    ("ko_KR.UTF-8", "한국어"),
    ("zh_CN.UTF-8", "中文 (简体)"),
];

const KEYBOARDS: &[(&str, &str)] = &[
    ("us", "English (US)"),
    ("gb", "English (UK)"),
    ("ca", "Canadian"),
    ("de", "German"),
    ("ch", "Swiss"),
    ("fr", "French"),
    ("be", "Belgian"),
    ("es", "Spanish"),
    ("latam", "Spanish (Latin America)"),
    ("pt", "Portuguese"),
    ("br", "Portuguese (Brazil)"),
    ("it", "Italian"),
    ("nl", "Dutch"),
    ("se", "Swedish"),
    ("no", "Norwegian"),
    ("dk", "Danish"),
    ("fi", "Finnish"),
    ("pl", "Polish"),
    ("cz", "Czech"),
    ("tr", "Turkish"),
    ("ru", "Russian"),
    ("ua", "Ukrainian"),
    ("jp", "Japanese"),
    ("kr", "Korean"),
];

/// `options` with `current` in it, when it isn't one of them already.
fn with_current(mut list: Vec<(String, String)>, current: &str) -> Vec<(String, String)> {
    if !current.is_empty() && !list.iter().any(|(id, _)| id == current) {
        list.insert(0, (current.to_string(), current.to_string()));
    }
    list
}

fn label_of(list: &[(String, String)], id: &str) -> String {
    list.iter().find(|(i, _)| i == id).map(|(_, l)| l.clone()).unwrap_or_else(|| id.to_string())
}

// ---------- Time zones: where they are, and the time there ----------

/// Where a zone is: its main city's latitude and longitude, and its country.
#[derive(Clone)]
struct Place {
    lat: f64,
    lon: f64,
    country: String,
}

/// The tz database's own tables, from wherever this system keeps them.
fn tz_file(name: &str) -> String {
    let dirs = [std::env::var("TZDIR").unwrap_or_default(), "/usr/share/zoneinfo".into(), "/etc/zoneinfo".into()];
    dirs.iter().filter(|d| !d.is_empty()).find_map(|d| std::fs::read_to_string(format!("{d}/{name}")).ok()).unwrap_or_default()
}

/// `+3843-00908` or `+404251-0740023`: degrees, minutes and maybe seconds.
fn coordinates(text: &str) -> Option<(f64, f64)> {
    let split = text[1..].find(['+', '-'])? + 1;
    let part = |s: &str, deg: usize| -> Option<f64> {
        let sign = if s.starts_with('-') { -1.0 } else { 1.0 };
        let d = &s[1..];
        let degrees: f64 = d.get(..deg)?.parse().ok()?;
        let minutes: f64 = d.get(deg..deg + 2)?.parse().ok()?;
        let seconds: f64 = d.get(deg + 2..deg + 4).and_then(|x| x.parse().ok()).unwrap_or(0.0);
        Some(sign * (degrees + minutes / 60.0 + seconds / 3600.0))
    };
    Some((part(&text[..split], 2)?, part(&text[split..], 3)?))
}

thread_local! {
    static PLACES: RefCell<Option<Rc<HashMap<String, Place>>>> = const { RefCell::new(None) };
}

/// Every zone's place, links (`US/Central`) included, read once.
fn places() -> Rc<HashMap<String, Place>> {
    if let Some(p) = PLACES.with(|p| p.borrow().clone()) {
        return p;
    }
    let countries: HashMap<String, String> = tz_file("iso3166.tab")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t').map(|(c, n)| (c.to_string(), n.trim().to_string())))
        .collect();
    let mut map: HashMap<String, Place> = HashMap::new();
    for line in tz_file("zone.tab").lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 3 {
            continue;
        }
        if let Some((lat, lon)) = coordinates(f[1]) {
            let country = countries.get(f[0]).cloned().unwrap_or_default();
            map.insert(f[2].to_string(), Place { lat, lon, country });
        }
    }
    for line in tz_file("tzdata.zi").lines() {
        let mut f = line.split_whitespace();
        if let (Some("L"), Some(target), Some(link)) = (f.next(), f.next(), f.next())
            && !map.contains_key(link)
            && let Some(p) = map.get(target).cloned()
        {
            map.insert(link.to_string(), p);
        }
    }
    let map = Rc::new(map);
    PLACES.with(|p| *p.borrow_mut() = Some(map.clone()));
    map
}

/// `Europe/Lisbon` → `Lisbon`; `America/Argentina/Buenos_Aires` → `Buenos Aires`.
fn city(zone: &str) -> String {
    zone.rsplit('/').next().unwrap_or(zone).replace('_', " ")
}

/// The time in `zone` now.
fn now_in(zone: &str) -> Option<glib::DateTime> {
    // An unknown zone comes back as UTC.
    let tz = glib::TimeZone::new(Some(zone));
    if tz.identifier() != zone && zone != "UTC" {
        return None;
    }
    glib::DateTime::now(&tz).ok()
}

/// Seconds east of UTC in `zone` now.
fn offset(zone: &str) -> Option<i64> {
    now_in(zone).map(|d| d.utc_offset().as_seconds())
}

/// `UTC+0`, `UTC−5`, `UTC+5:30`.
fn utc_text(secs: i64) -> String {
    let sign = if secs < 0 { "−" } else { "+" };
    let (h, m) = (secs.abs() / 3600, secs.abs() % 3600 / 60);
    if m == 0 { format!("UTC{sign}{h}") } else { format!("UTC{sign}{h}:{m:02}") }
}

/// "5 hours ahead of New York", "30 minutes behind Paris", "" when they're the same.
fn ahead_of(secs: i64, of: &str) -> String {
    if secs == 0 {
        return format!("the same time as {of}");
    }
    let way = if secs > 0 { "ahead of" } else { "behind" };
    let (h, m) = (secs.abs() / 3600, secs.abs() % 3600 / 60);
    let amount = match (h, m) {
        (0, m) => format!("{m} minutes"),
        (1, 0) => "1 hour".into(),
        (h, 0) => format!("{h} hours"),
        (h, m) => format!("{h} h {m} min"),
    };
    format!("{amount} {way} {of}")
}

// ---------- The world map ----------

/// Land on a 60 × 24 grid (6° a cell, 84° N to 60° S), as inclusive column ranges per row.
const LAND: [&[(u8, u8)]; 24] = [
    &[(13, 17), (20, 27)],
    &[(10, 18), (19, 27), (33, 34), (45, 50)],
    &[(2, 6), (7, 17), (19, 27), (34, 37), (38, 59)],
    &[(1, 13), (16, 18), (20, 25), (26, 26), (32, 59)],
    &[(3, 6), (8, 13), (16, 20), (29, 29), (31, 33), (34, 55), (56, 57)],
    &[(8, 20), (29, 30), (30, 52), (53, 54)],
    &[(9, 19), (29, 48), (49, 51), (53, 54)],
    &[(9, 18), (28, 32), (33, 37), (38, 51), (52, 54)],
    &[(10, 17), (28, 35), (36, 52), (52, 53)],
    &[(11, 13), (15, 16), (27, 36), (36, 40), (40, 51)],
    &[(11, 15), (16, 17), (27, 36), (36, 40), (41, 44), (45, 50)],
    &[(13, 16), (27, 38), (42, 44), (45, 48), (50, 51)],
    &[(15, 22), (27, 38), (43, 43), (46, 47), (50, 51)],
    &[(16, 23), (31, 37), (45, 49), (50, 51)],
    &[(16, 24), (32, 37), (46, 50), (52, 55)],
    &[(16, 25), (32, 37), (47, 51), (53, 54)],
    &[(17, 24), (32, 37), (38, 39), (50, 55)],
    &[(17, 24), (32, 36), (38, 39), (48, 55)],
    &[(17, 22), (32, 36), (48, 55)],
    &[(17, 21), (33, 35), (49, 55), (58, 59)],
    &[(17, 20), (52, 54), (57, 59)],
    &[(17, 19), (52, 53), (57, 58)],
    &[(17, 19)],
    &[(18, 19)],
];

/// What the map shows: the picked zone, and the one the computer is on when they differ.
struct MapView {
    zone: String,
    was: Option<String>,
}

fn draw_map(cr: &gtk::cairo::Context, p: &sketch::Pal, accent: sketch::Rgb, w: f64, h: f64, v: &MapView) {
    let places = places();
    // The 600 × 240 grid, as large as fits.
    let s = (w / 600.0).min(h / 240.0);
    let (ox, oy) = ((w - 600.0 * s) / 2.0, (h - 240.0 * s) / 2.0);
    let px = |lon: f64| ox + (lon + 180.0) / 360.0 * 600.0 * s;
    let py = |lat: f64| oy + (84.0 - lat) / 144.0 * 240.0 * s;

    // Night, where the sun is more than 90° of longitude away.
    if let Ok(utc) = glib::DateTime::now_utc() {
        let hours = utc.hour() as f64 + utc.minute() as f64 / 60.0;
        let sun = (12.0 - hours) * 15.0;
        let strips = 120;
        for i in 0..strips {
            let lon = -180.0 + (i as f64 + 0.5) * 360.0 / strips as f64;
            let d = ((lon - sun).rem_euclid(360.0) - 180.0).abs();
            let dark = ((180.0 - d - 84.0) / 12.0).clamp(0.0, 1.0);
            if dark > 0.0 {
                sketch::set(cr, sketch::mix(p.surface, (0.0, 0.0, 0.0), 0.6), 0.55 * dark);
                let x0 = px(lon - 180.0 / strips as f64);
                cr.rectangle(x0, oy, 600.0 * s / strips as f64 + 0.5, 240.0 * s);
                let _ = cr.fill();
            }
        }
    }

    // The zone as a band an hour wide, around its offset.
    let band = offset(&v.zone).map(|o| {
        let c = o as f64 / 3600.0 * 15.0;
        (px(c - 7.5), px(c + 7.5))
    });
    if let Some((b0, b1)) = band {
        sketch::set(cr, accent, 0.14);
        cr.rectangle(b0, oy, b1 - b0, 240.0 * s);
        let _ = cr.fill();
    }

    let r = 2.7 * s;
    for (row, runs) in LAND.iter().enumerate() {
        for &(a, b) in runs.iter() {
            for c in a..=b {
                let x = ox + (c as f64 * 10.0 + 5.0) * s;
                let y = oy + (row as f64 * 10.0 + 5.0) * s;
                let inside = band.is_some_and(|(b0, b1)| x >= b0 && x <= b1);
                if inside {
                    sketch::set(cr, accent, 0.95);
                } else {
                    sketch::set(cr, p.dim, 0.5);
                }
                cr.arc(x, y, r, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
        }
    }

    let pin = |zone: &str, now: bool| {
        let Some(place) = places.get(zone) else { return };
        let (x, y) = (px(place.lon), py(place.lat));
        if now {
            sketch::set(cr, accent, 0.3);
            cr.arc(x, y, 10.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            sketch::set(cr, (1.0, 1.0, 1.0), 1.0);
            cr.arc(x, y, 5.5, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            sketch::set(cr, accent, 1.0);
            cr.arc(x, y, 3.8, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        } else {
            sketch::set(cr, p.text, 0.18);
            cr.arc(x, y, 10.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            sketch::set(cr, p.text, 0.9);
            cr.set_line_width(2.0);
            cr.arc(x, y, 5.0, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
        }
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
        cr.set_font_size(11.0);
        let name = city(zone);
        if let Ok(ext) = cr.text_extents(&name) {
            // The old place's name to its left, the new one's to its right, kept inside.
            let tx = if now { x + 10.0 } else { x - 10.0 - ext.width() };
            let tx = tx.clamp(2.0, (w - ext.width() - 2.0).max(2.0));
            let ty = (y - 12.0).max(ext.height() + 2.0);
            sketch::set(cr, p.bg, 0.7);
            sketch::rounded(cr, tx - 4.0, ty - ext.height() - 3.0, ext.width() + 8.0, ext.height() + 6.0, 4.0);
            let _ = cr.fill();
            sketch::set(cr, p.text, 0.95);
            cr.move_to(tx - ext.x_bearing(), ty);
            let _ = cr.show_text(&name);
        }
    };
    if let Some(was) = &v.was {
        pin(was, false);
    }
    pin(&v.zone, true);
}

/// The map's key: the new zone, where the computer is now, and night.
fn legend(changed: bool) -> gtk::Box {
    let b = widgets::hbox(16);
    b.add_css_class("nxr-legend");
    let items: &[(&str, &str)] =
        if changed { &[("zone", "Your new zone"), ("now", "Now"), ("night", "Night")] } else { &[("zone", "Your zone"), ("night", "Night")] };
    for (kind, text) in items {
        let item = widgets::hbox(6);
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("nxr-key");
        dot.add_css_class(kind);
        dot.set_valign(gtk::Align::Center);
        item.append(&dot);
        item.append(&widgets::label(text, "nxr-legend-text"));
        b.append(&item);
    }
    b
}

// ---------- Formats in a language ----------

/// How a language writes dates, times, numbers and money, from the system's own locale
/// data (only for languages installed on this computer).
#[derive(Clone, PartialEq)]
struct Formats {
    date: String,
    twelve_hour: bool,
    number: String,
    money: String,
    week: String,
}

/// `en_GB.UTF-8` and `en_GB.utf8` alike.
fn normal_locale(l: &str) -> String {
    l.to_lowercase().replace("utf-8", "utf8")
}

fn installed(locale: &str) -> bool {
    let want = normal_locale(locale);
    cmd::output(&["locale", "-a"]).is_some_and(|all| all.lines().any(|l| normal_locale(l.trim()) == want))
}

fn in_locale(locale: &str, zone: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(args[0])
        .args(&args[1..])
        .env("LC_ALL", locale)
        .env("TZ", zone)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// 1234567.89 written with `group` between thousands and `point` before the decimals.
fn grouped(int: u64, cents: u64, group: &str, point: &str) -> String {
    let digits = int.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push_str(group);
        }
        out.push(ch);
    }
    format!("{out}{point}{cents:02}")
}

fn formats(locale: &str, zone: &str) -> Option<Formats> {
    if !installed(locale) {
        return None;
    }
    let keys = in_locale(
        locale,
        zone,
        &[
            "locale",
            "-k",
            "thousands_sep",
            "decimal_point",
            "mon_thousands_sep",
            "mon_decimal_point",
            "currency_symbol",
            "p_cs_precedes",
            "p_sep_by_space",
            "first_weekday",
            "week-1stday",
            "t_fmt",
        ],
    )?;
    let k: HashMap<&str, String> = keys
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(a, b)| (a, b.trim_matches('"').to_string()))
        .collect();
    let get = |key: &str| k.get(key).cloned().unwrap_or_default();
    let date = in_locale(locale, zone, &["date", "+%x"])?;
    let t_fmt = get("t_fmt");
    let twelve_hour = t_fmt.contains("%r") || t_fmt.contains("%p") || t_fmt.contains("%I");
    let number = grouped(1_234_567, 89, &get("thousands_sep"), &get("decimal_point"));
    let amount = grouped(1234, 56, &get("mon_thousands_sep"), &get("mon_decimal_point"));
    let symbol = get("currency_symbol");
    let space = if get("p_sep_by_space") == "1" { " " } else { "" };
    let money = if get("p_cs_precedes") == "1" { format!("{symbol}{space}{amount}") } else { format!("{amount}{space}{symbol}") };
    // week-1stday is a day (19971130, a Sunday); first_weekday counts from it.
    let week = (|| {
        let day = get("week-1stday");
        let (y, m, d) = (day.get(..4)?.parse().ok()?, day.get(4..6)?.parse().ok()?, day.get(6..8)?.parse().ok()?);
        let base = glib::DateTime::from_utc(y, m, d, 12, 0, 0.0).ok()?;
        let first: i32 = get("first_weekday").parse().unwrap_or(1);
        let dow = (base.day_of_week() - 1 + first - 1).rem_euclid(7);
        Some(["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"][dow as usize].to_string())
    })()
    .unwrap_or_default();
    Some(Formats { date, twelve_hour, number, money, week })
}

/// The time in `zone`, the way a language writes it.
fn time_text(zone: &str, twelve_hour: bool) -> String {
    now_in(zone)
        .and_then(|d| d.format(if twelve_hour { "%l:%M %p" } else { "%H:%M" }).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// The samples card: each format in the picked language, the computer's own struck
/// through where they differ.
struct Samples {
    title: gtk::Label,
    after: gtk::Label,
    grid: gtk::Grid,
    note: gtk::Label,
}

const SAMPLE_ROWS: [(&str, &str); 5] = [
    ("x-office-calendar-symbolic", "Date"),
    ("preferences-system-time-symbolic", "Time"),
    ("accessories-calculator-symbolic", "Numbers"),
    ("emblem-documents-symbolic", "Money"),
    ("view-grid-symbolic", "Weeks start on"),
];

impl Samples {
    fn new() -> (gtk::Box, Samples) {
        let col = widgets::vbox(8);
        col.add_css_class("nxr-col");
        let head = widgets::hbox(8);
        let title = widgets::label("", "nxr-col-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_hexpand(true);
        head.append(&title);
        let after = widgets::label("After applying", "nxr-after");
        after.set_visible(false);
        head.append(&after);
        col.append(&head);
        let card = widgets::vbox(10);
        card.add_css_class("preview-card");
        card.add_css_class("nxr-fmt");
        card.set_vexpand(true);
        let grid = gtk::Grid::new();
        grid.set_column_spacing(10);
        grid.set_row_spacing(10);
        card.append(&grid);
        let note = widgets::label("", "nxr-note");
        note.set_wrap(true);
        note.set_visible(false);
        card.append(&note);
        col.append(&card);
        (col, Samples { title, after, grid, note })
    }

    fn show(&self, language: &str, language_now: &str, changed: bool, checked: bool, zone: &str, now: Option<&Formats>, picked: Option<&Formats>) {
        while let Some(c) = self.grid.first_child() {
            self.grid.remove(&c);
        }
        let shown = if changed { picked } else { now };
        // A language that isn't installed yet can't show its formats: the ones in use stay.
        let unseen = changed && picked.is_none();
        self.note.set_visible(unseen && checked);
        self.title.set_text(&format!("In {}", if unseen { language_now } else { language }));
        self.after.set_visible(changed && shown.is_some());
        self.note.set_text(&format!("{language} isn't installed on this computer yet; its formats show once it's applied."));
        let Some(f) = shown.or(now) else { return };
        let old = if changed { now.filter(|_| picked.is_some()) } else { None };
        let values = |f: &Formats| [f.date.clone(), time_text(zone, f.twelve_hour), f.number.clone(), f.money.clone(), f.week.clone()];
        let new = values(f);
        let was = old.map(values);
        for (i, (icon, name)) in SAMPLE_ROWS.iter().enumerate() {
            let i = i as i32;
            let img = gtk::Image::from_icon_name(icon);
            img.add_css_class("nxr-fmt-icon");
            self.grid.attach(&img, 0, i, 1, 1);
            let k = widgets::label(name, "nxr-fmt-key");
            self.grid.attach(&k, 1, i, 1, 1);
            let v = widgets::label(&new[i as usize], "nxr-fmt-value");
            v.set_hexpand(true);
            v.set_ellipsize(gtk::pango::EllipsizeMode::End);
            self.grid.attach(&v, 2, i, 1, 1);
            if let Some(was) = &was
                && was[i as usize] != new[i as usize]
            {
                let w = widgets::label(&was[i as usize], "nxr-fmt-was");
                w.set_xalign(1.0);
                self.grid.attach(&w, 3, i, 1, 1);
            }
        }
    }
}

// ---------- The page ----------

/// What's picked on the page, and what the computer has now.
struct State {
    zone_now: String,
    zone: String,
    locale_now: String,
    locale: String,
    formats_now: Option<Formats>,
    formats: Option<Formats>,
    /// Whether the picked language's formats have been looked up yet.
    checked: bool,
}

/// Everything in the picture that follows the controls.
struct Picture {
    title: gtk::Label,
    line: gtk::Label,
    clock: gtk::Label,
    map: gtk::DrawingArea,
    view: Rc<RefCell<MapView>>,
    legend_slot: gtk::Box,
    samples: Samples,
}

impl Picture {
    fn show(&self, st: &State) {
        let places = places();
        let zone = &st.zone;
        let place = places.get(zone);
        self.title.set_text(&match place.map(|p| p.country.as_str()).filter(|c| !c.is_empty()) {
            Some(country) => format!("{}, {country}", city(zone)),
            None => zone.replace('_', " "),
        });
        let mut line = vec![zone.replace('_', " ")];
        if let Some(o) = offset(zone) {
            line.push(utc_text(o));
            if zone != &st.zone_now
                && let Some(o0) = offset(&st.zone_now)
            {
                line.push(ahead_of(o - o0, &city(&st.zone_now)));
            }
        }
        self.line.set_text(&line.join(" · "));
        let twelve = st.formats.as_ref().or(st.formats_now.as_ref()).is_some_and(|f| f.twelve_hour);
        self.clock.set_text(&time_text(zone, twelve));
        {
            let mut v = self.view.borrow_mut();
            v.zone = zone.clone();
            v.was = (zone != &st.zone_now && places.contains_key(&st.zone_now)).then(|| st.zone_now.clone());
        }
        self.map.queue_draw();
        while let Some(c) = self.legend_slot.first_child() {
            self.legend_slot.remove(&c);
        }
        self.legend_slot.append(&legend(zone != &st.zone_now));
        let changed = st.locale != st.locale_now;
        let language = label_of(&with_current(options(LOCALES), &st.locale), &st.locale);
        let language_now = label_of(&with_current(options(LOCALES), &st.locale_now), &st.locale_now);
        self.samples.show(&language, &language_now, changed, st.checked, zone, st.formats_now.as_ref(), st.formats.as_ref());
    }
}

/// The name as other computers see it.
fn name_card(name: &str) -> (gtk::Box, gtk::Label, gtk::Label) {
    let col = widgets::vbox(8);
    col.add_css_class("nxr-col");
    col.append(&widgets::label("This computer", "nxr-col-title"));
    let card = widgets::vbox(4);
    card.add_css_class("preview-card");
    card.add_css_class("nxr-net");
    card.set_vexpand(true);
    let line = widgets::hbox(10);
    line.append(&widgets::tile("computer-symbolic"));
    let big = widgets::label(name, "nxr-name");
    big.set_ellipsize(gtk::pango::EllipsizeMode::End);
    big.set_hexpand(true);
    line.append(&big);
    card.append(&line);
    card.append(&widgets::label("On the network", "nxr-fmt-key"));
    // Its .local address, where this computer answers mDNS.
    let mdns = cmd::output(&["systemctl", "is-active", "avahi-daemon"]).is_some_and(|s| s.trim() == "active");
    let address = widgets::label(&format!("{name}.local"), "nxr-address");
    address.add_css_class("mono");
    address.set_visible(mdns);
    card.append(&address);
    let rule = widgets::label("Letters, digits and dashes, up to 63.", "nxr-note");
    rule.set_wrap(true);
    card.append(&rule);
    col.append(&card);
    (col, big, address)
}

/// A row's control (its last child) asking for no more than `max` pixels of width.
fn cap_control(row: &gtk::Box, max: i32) {
    if let Some(control) = row.last_child() {
        row.remove(&control);
        let c = nyxos::capped(&control, max);
        c.set_valign(gtk::Align::Center);
        row.append(&c);
    }
}

/// A row with an icon on a tile before its text, made from an ordinary one.
fn with_tile(row: &gtk::Box, icon: &str) {
    row.prepend(&widgets::tile(icon));
    row.add_css_class("with-tile");
}

pub fn region(page: &Page) {
    let s = nyxos::settings();
    let pending = Pending::new("region");

    let tz_now =
        cmd::output(&["timedatectl", "show", "-p", "Timezone", "--value"]).map(|t| t.trim().to_string()).unwrap_or_default();
    let lang = s.locale.clone().or_else(|| std::env::var("LANG").ok()).unwrap_or_else(|| "en_US.UTF-8".into());
    let state = Rc::new(RefCell::new(State {
        zone_now: tz_now.clone(),
        zone: tz_now.clone(),
        locale_now: lang.clone(),
        locale: lang.clone(),
        formats_now: None,
        formats: None,
        checked: false,
    }));

    // The picture: the zone on the map, the time there, the formats and the name.
    let pv = page.split();
    let clock = widgets::label("", "nxr-clock");
    let head = pv.head("", "", &[clock.upcast_ref()]);
    pv.pane.remove(&head);
    pv.add(&nyxos::shrinkable(&head));
    let text = head.first_child().and_downcast::<gtk::Box>();
    let title = text.as_ref().and_then(|t| t.first_child()).and_downcast::<gtk::Label>().unwrap_or_else(|| gtk::Label::new(None));
    let line = widgets::label("", "preview-line");
    line.set_wrap(true);
    if let Some(t) = &text {
        t.append(&line);
    }
    let view = Rc::new(RefCell::new(MapView { zone: tz_now.clone(), was: None }));
    let card = pv.card();
    card.add_css_class("nxr-map");
    let map = {
        let v = view.clone();
        widgets::drawing(220, move |cr, p, accent, w, h| draw_map(cr, p, accent, w, h, &v.borrow()))
    };
    card.append(&map);
    let legend_slot = widgets::hbox(0);
    card.append(&nyxos::shrinkable(&legend_slot));

    let cols = gtk::FlowBox::new();
    cols.add_css_class("nxr-cols");
    cols.set_selection_mode(gtk::SelectionMode::None);
    cols.set_homogeneous(true);
    cols.set_min_children_per_line(1);
    cols.set_max_children_per_line(2);
    cols.set_column_spacing(12);
    cols.set_row_spacing(12);
    let (samples_col, samples) = Samples::new();
    cols.insert(&samples_col, -1);
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname").map(|h| h.trim().to_string()).unwrap_or_default();
    let (name_col, name_label, address) = name_card(&host);
    cols.insert(&name_col, -1);
    let mut c = cols.first_child();
    while let Some(child) = c {
        child.set_focusable(false);
        c = child.next_sibling();
    }
    pv.add(&nyxos::shrinkable(&cols));

    let picture = Rc::new(Picture { title, line, clock, map, view, legend_slot, samples });
    picture.show(&state.borrow());
    {
        // The formats come from the system's locale data, asked once in the background.
        let (state, picture) = (state.clone(), picture.clone());
        let (l, z) = (lang.clone(), tz_now.clone());
        cmd::background(
            move || formats(&l, &z),
            move |f| {
                let mut st = state.borrow_mut();
                st.formats_now = f.clone();
                if st.locale == st.locale_now {
                    st.formats = f;
                    st.checked = true;
                }
                picture.show(&st);
            },
        );
    }
    {
        // The clock and the night move on while the page is open.
        let (weak, state, picture) = (picture.map.downgrade(), state.clone(), picture.clone());
        glib::timeout_add_seconds_local(20, move || {
            let Some(map) = weak.upgrade() else { return glib::ControlFlow::Break };
            if map.is_mapped() {
                picture.show(&state.borrow());
            }
            glib::ControlFlow::Continue
        });
    }

    // The controls: date and time, language and keyboard, this computer.
    let g = page.group("Date and time");
    let zones: Vec<(String, String)> = cmd::output(&["timedatectl", "list-timezones"])
        .unwrap_or_default()
        .lines()
        .map(|z| (z.to_string(), z.replace('_', " ")))
        .collect();
    let zones = with_current(zones, &tz_now);
    let here = {
        let mut parts = vec![match places().get(&tz_now).map(|p| p.country.clone()).filter(|c| !c.is_empty()) {
            Some(country) => format!("{}, {country}", city(&tz_now)),
            None => tz_now.replace('_', " "),
        }];
        parts.extend(offset(&tz_now).map(utc_text));
        if tz_now.is_empty() { String::new() } else { parts.join(" · ") }
    };
    let (r, dd) = widgets::choice_row("Time zone", &here, zones.clone(), &tz_now, |_| {});
    with_tile(&r, "web-browser-symbolic");
    mark_system(&r);
    let mark = RowMark::new(&r);
    {
        let (p, state, picture, now) = (pending.clone(), state.clone(), picture.clone(), tz_now.clone());
        dd.connect_selected_notify(move |d| {
            let Some((z, label)) = zones.get(d.selected() as usize).cloned() else { return };
            let changed = z != now;
            let before = now.replace('_', " ");
            p.set_shown("time-zone", changed.then(|| Change::TimeZone { value: Some(z.clone()) }), &before, &label);
            mark.set(changed.then_some(before.as_str()));
            state.borrow_mut().zone = z;
            picture.show(&state.borrow());
        });
    }
    dd.set_enable_search(true);
    dd.set_expression(Some(gtk::PropertyExpression::new(gtk::StringObject::static_type(), None::<gtk::Expression>, "string")));
    widgets::keywords("timezone zone clock city");
    cap_control(&r, 130);
    g.add(&r);
    let clock = cmd::output(&["date", "+%A %e %B, %H:%M"]).unwrap_or_default();
    let (r, _) = widgets::info_row("Now", clock.split_whitespace().collect::<Vec<_>>().join(" ").as_str());
    with_tile(&r, "preferences-system-time-symbolic");
    g.add(&r);

    let g = page.group("Language and keyboard");
    let locales = with_current(options(LOCALES), &lang);
    let normal = "Menus, dates, numbers and money.";
    let (r, _) = widgets::choice_row("Language and formats", normal, locales.clone(), &lang, |_| {});
    with_tile(&r, "preferences-desktop-locale-symbolic");
    mark_system(&r);
    let mark = RowMark::new(&r);
    if let Some(dd) = r.last_child().and_downcast::<gtk::DropDown>() {
        let (p, state, picture, now) = (pending.clone(), state.clone(), picture.clone(), lang.clone());
        let list = locales.clone();
        dd.connect_selected_notify(move |d| {
            let Some((l, label)) = list.get(d.selected() as usize).cloned() else { return };
            let changed = l != now;
            let before = label_of(&list, &now);
            p.set_shown("locale", changed.then(|| Change::Locale { value: Some(l.clone()) }), &before, &label);
            mark.set(changed.then_some(before.as_str()));
            {
                let mut st = state.borrow_mut();
                st.locale = l.clone();
                st.formats = if changed { None } else { st.formats_now.clone() };
                st.checked = !changed;
            }
            picture.show(&state.borrow());
            if changed {
                let (state, picture) = (state.clone(), picture.clone());
                let zone = state.borrow().zone.clone();
                cmd::background(
                    move || (formats(&l, &zone), l),
                    move |(f, l)| {
                        let mut st = state.borrow_mut();
                        if st.locale == l {
                            st.formats = f;
                            st.checked = true;
                            picture.show(&st);
                        }
                    },
                );
            }
        });
    }
    widgets::keywords("language locale format region translation");
    cap_control(&r, 130);
    g.add(&r);
    let kb = s.keyboard_layout.clone().unwrap_or_else(|| "us".into());
    let keyboards = with_current(options(KEYBOARDS), &kb);
    let (r, _) = widgets::choice_row("Keyboard layout", "On the desktop and the console.", keyboards.clone(), &kb, |_| {});
    with_tile(&r, "input-keyboard-symbolic");
    mark_system(&r);
    let mark = RowMark::new(&r);
    if let Some(dd) = r.last_child().and_downcast::<gtk::DropDown>() {
        let (p, now) = (pending.clone(), kb.clone());
        dd.connect_selected_notify(move |d| {
            let Some((k, label)) = keyboards.get(d.selected() as usize).cloned() else { return };
            let changed = k != now;
            let before = label_of(&keyboards, &now);
            p.set_shown("keyboard", changed.then(|| Change::KeyboardLayout { value: Some(k) }), &before, &label);
            mark.set(changed.then_some(before.as_str()));
        });
    }
    widgets::keywords("keyboard layout xkb keymap");
    cap_control(&r, 130);
    g.add(&r);
    let (r, _) = widgets::nav_row(
        Some("preferences-desktop-keyboard-symbolic"),
        "More layouts",
        "To switch between while you type.",
        "Keyboard",
        || window::navigate("keyboard"),
    );
    widgets::keywords("layouts switch input sources");
    g.add(&r);

    let g = page.group("This computer");
    let (r, entry) = widgets::entry_row("Name", "Seen by other computers.", &host, "nyxos", |_| {});
    with_tile(&r, "computer-symbolic");
    mark_system(&r);
    let mark = Rc::new(RowMark::new(&r));
    {
        let (p, now, mark) = (pending.clone(), host.clone(), mark.clone());
        let check = move |name: &str| {
            let name = name.trim().to_string();
            if name != now && !valid_hostname(&name) {
                window::toast("A name is letters, digits and dashes, not at either end, up to 63");
                p.set("hostname", None);
                mark.set(None);
                return;
            }
            let changed = name != now;
            p.set_shown("hostname", changed.then(|| Change::Hostname { value: Some(name.clone()) }), &now, &name);
            mark.set(changed.then_some(now.as_str()));
        };
        let c1 = check.clone();
        entry.connect_activate(move |e| c1(&e.text()));
        let focus = gtk::EventControllerFocus::new();
        let e2 = entry.clone();
        focus.connect_leave(move |_| check(&e2.text()));
        entry.add_controller(focus);
    }
    {
        // The picture follows the name as it's typed.
        let fallback = host.clone();
        entry.connect_changed(move |e| {
            let t = e.text().trim().to_string();
            let t = if t.is_empty() { fallback.clone() } else { t };
            name_label.set_text(&t);
            address.set_text(&format!("{t}.local"));
        });
    }
    entry.add_css_class("mono");
    entry.set_width_chars(12);
    widgets::keywords("hostname computer name network");
    cap_control(&r, 130);
    g.add(&r);

    pending.place(page);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_tab_coordinates_read() {
        let (lat, lon) = coordinates("+3843-00908").unwrap();
        assert!((lat - 38.716).abs() < 0.01 && (lon + 9.133).abs() < 0.01);
        let (lat, lon) = coordinates("+404251-0740023").unwrap();
        assert!((lat - 40.714).abs() < 0.01 && (lon + 74.006).abs() < 0.01);
    }

    #[test]
    fn offsets_read_as_people_say_them() {
        assert_eq!(utc_text(0), "UTC+0");
        assert_eq!(utc_text(-5 * 3600), "UTC−5");
        assert_eq!(utc_text(19800), "UTC+5:30");
        assert_eq!(ahead_of(5 * 3600, "New York"), "5 hours ahead of New York");
        assert_eq!(ahead_of(-1800, "Paris"), "30 minutes behind Paris");
        assert_eq!(city("America/Argentina/Buenos_Aires"), "Buenos Aires");
    }

    #[test]
    fn numbers_are_grouped() {
        assert_eq!(grouped(1_234_567, 89, ",", "."), "1,234,567.89");
        assert_eq!(grouped(1234, 56, ".", ","), "1.234,56");
        assert_eq!(grouped(12, 5, ",", "."), "12.05");
    }
}
