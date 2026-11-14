//! Date and time, language and the computer's name. Changes use the root helper.
//!
//! Plain names on the left (Chicago · America, English · United States); on the right a
//! world map with your time zone lit and where it's night, and samples in your formats.

use crate::cmd;
use crate::sections::accounts::{facts, head, require_helper, roomy, run_admin, subhead};
use crate::sketch;
use crate::widgets::{self, Page};
use gtk::glib;
use gtk::prelude::*;
use std::collections::HashMap;

/// `timedatectl show` prints `Key=Value` lines.
fn parse_show(text: &str) -> HashMap<String, String> {
    text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// `LANG` from `/etc/locale.conf`.
fn parse_lang(text: &str) -> Option<String> {
    text.lines().find_map(|l| l.strip_prefix("LANG=")).map(|v| v.trim().trim_matches('"').to_string()).filter(|v| !v.is_empty())
}

/// `de_DE.utf8` and `de_DE.UTF-8` name the same locale.
fn same_locale(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.to_lowercase().replace("utf-8", "utf8");
    norm(a) == norm(b)
}

/// `locale -a` says `de_DE.utf8`; locale.conf and localectl use `de_DE.UTF-8`.
fn canonical_locale(l: &str) -> String {
    match l.split_once('.') {
        Some((base, rest)) if rest.to_lowercase().starts_with("utf8") => format!("{base}.UTF-8{}", &rest[4..]),
        _ => l.to_string(),
    }
}

/// A locale's language and country, from its definition in `/usr/share/i18n/locales`:
/// `de_DE.utf8` is "German · Germany". The code itself when there's no definition.
fn locale_name(l: &str) -> String {
    let (base, modifier) = match l.split_once('.') {
        Some((b, rest)) => (b.to_string(), rest.split_once('@').map(|(_, m)| format!("@{m}")).unwrap_or_default()),
        None => (l.to_string(), String::new()),
    };
    let text = std::fs::read_to_string(format!("/usr/share/i18n/locales/{base}{modifier}"))
        .or_else(|_| std::fs::read_to_string(format!("/usr/share/i18n/locales/{base}")))
        .unwrap_or_default();
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.trim().strip_prefix(key).filter(|r| r.starts_with([' ', '\t'])))
            .map(|v| v.trim().trim_matches('"').to_string())
            .filter(|v| !v.is_empty())
    };
    match (field("language"), field("territory")) {
        (Some(lang), Some(land)) => format!("{lang} · {land}{}", if modifier.is_empty() { String::new() } else { format!(" ({})", &modifier[1..]) }),
        (Some(lang), None) => lang,
        _ => l.to_string(),
    }
}

/// A zone's city and region: `America/Argentina/Buenos_Aires` is "Buenos Aires · Argentina".
fn zone_name(zone: &str) -> String {
    let parts: Vec<String> = zone.split('/').map(|p| p.replace('_', " ")).collect();
    match parts.as_slice() {
        [] => zone.to_string(),
        [one] => one.clone(),
        [.., region, city] => format!("{city} · {region}"),
    }
}

/// The zone a legacy name stands for (`US/Central` is `America/Chicago`), from tzdata's links.
fn canonical_zone(zone: &str) -> String {
    let text = std::fs::read_to_string("/usr/share/zoneinfo/tzdata.zi").unwrap_or_default();
    text.lines()
        .filter_map(|l| l.strip_prefix("L "))
        .find_map(|l| l.split_once(' ').filter(|(_, alias)| *alias == zone).map(|(target, _)| target.to_string()))
        .unwrap_or_else(|| zone.to_string())
}

/// Every legacy zone name and the zone it stands for.
fn zone_links() -> HashMap<String, String> {
    std::fs::read_to_string("/usr/share/zoneinfo/tzdata.zi")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("L "))
        .filter_map(|l| l.split_once(' ').map(|(target, alias)| (alias.to_string(), target.to_string())))
        .collect()
}

/// Where a zone's main city is (latitude, longitude), from `zone1970.tab`.
fn zone_place(zone: &str) -> Option<(f64, f64)> {
    let text = std::fs::read_to_string("/usr/share/zoneinfo/zone1970.tab")
        .or_else(|_| std::fs::read_to_string("/usr/share/zoneinfo/zone.tab"))
        .ok()?;
    let coords = text.lines().filter(|l| !l.starts_with('#')).find_map(|l| {
        let f: Vec<&str> = l.split('\t').collect();
        (f.len() >= 3 && f[2] == zone).then(|| f[1].to_string())
    })?;
    parse_coords(&coords)
}

/// ISO 6709 as zone.tab writes it: `+415100-0873900` or `+4151-08739`.
fn parse_coords(c: &str) -> Option<(f64, f64)> {
    let split = c[1..].find(['+', '-'])? + 1;
    let (lat, lon) = c.split_at(split);
    let degrees = |s: &str, deg_digits: usize| -> Option<f64> {
        let sign = if s.starts_with('-') { -1.0 } else { 1.0 };
        let d = &s[1..];
        let whole: f64 = d.get(..deg_digits)?.parse().ok()?;
        let min: f64 = d.get(deg_digits..deg_digits + 2).and_then(|m| m.parse().ok()).unwrap_or(0.0);
        let sec: f64 = d.get(deg_digits + 2..deg_digits + 4).and_then(|m| m.parse().ok()).unwrap_or(0.0);
        Some(sign * (whole + min / 60.0 + sec / 3600.0))
    };
    Some((degrees(lat, 2)?, degrees(lon, 3)?))
}

/// A time zone by its name, if the system knows it.
fn time_zone(zone: &str) -> Option<glib::TimeZone> {
    let tz = glib::TimeZone::new(Some(zone));
    (tz.identifier() == zone).then_some(tz)
}

/// The time now in `zone`.
fn now_in(zone: &str) -> Option<glib::DateTime> {
    glib::DateTime::now(&time_zone(zone)?).ok()
}

/// "UTC−6", "UTC+5:30".
fn utc_offset(dt: &glib::DateTime) -> String {
    let secs = dt.utc_offset().as_seconds();
    let sign = if secs < 0 { "−" } else { "+" };
    let (h, m) = (secs.abs() / 3600, secs.abs() % 3600 / 60);
    if secs == 0 {
        "UTC".into()
    } else if m == 0 {
        format!("UTC{sign}{h}")
    } else {
        format!("UTC{sign}{h}:{m:02}")
    }
}

/// When the clocks last changed or next change in `zone`, within a few weeks: "clocks went
/// back an hour on 1 November".
fn clock_change(zone: &str) -> Option<String> {
    let now = now_in(zone)?;
    let offset = |d: &glib::DateTime| d.utc_offset().as_seconds();
    let at = |days: i32| now.add_days(days).ok();
    let phrase = |before: i64, after: i64, day: &glib::DateTime, past: bool| {
        let diff = after - before;
        let amount = if diff.abs() == 3600 { "an hour".to_string() } else { format!("{} minutes", diff.abs() / 60) };
        let way = match (diff > 0, past) {
            (true, true) => "went forward",
            (true, false) => "go forward",
            (false, true) => "went back",
            (false, false) => "go back",
        };
        format!("clocks {way} {amount} on {}", day.format("%-d %B").map(|s| s.to_string()).unwrap_or_default())
    };
    for d in 1..=21 {
        let (a, b) = (at(-d)?, at(-d + 1)?);
        if offset(&a) != offset(&b) {
            return Some(phrase(offset(&a), offset(&b), &b, true));
        }
    }
    for d in 0..30 {
        let (a, b) = (at(d)?, at(d + 1)?);
        if offset(&a) != offset(&b) {
            return Some(phrase(offset(&a), offset(&b), &b, false));
        }
    }
    None
}

/// What a locale says about its formats (`locale -k`), read for `locale`.
fn locale_facts(locale: &str) -> HashMap<String, String> {
    let all = format!("LC_ALL={locale}");
    cmd::output(&["env", &all, "locale", "-k", "LC_NUMERIC", "LC_MONETARY", "LC_TIME", "LC_MEASUREMENT", "LC_PAPER"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.trim_matches('"').to_string()))
        .collect()
}

/// `n` written with these separators and groups of three.
fn group_digits(n: u64, sep: &str) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push_str(sep);
        }
        out.push(c);
    }
    out
}

/// Whether the locale writes times on a 12-hour clock.
fn twelve_hour(f: &HashMap<String, String>) -> bool {
    f.get("t_fmt").is_some_and(|t| t.contains("%r") || t.contains("%I") || t.contains("%l") || t.contains("%p"))
}

/// Samples of dates, numbers and money in `locale`'s formats: (what, how it looks).
fn samples(locale: &str, f: &HashMap<String, String>) -> Vec<(&'static str, String)> {
    if f.is_empty() {
        return Vec::new();
    }
    let all = format!("LC_ALL={locale}");
    let dates = cmd::output(&["env", &all, "date", "+%A|%x|%X"]).unwrap_or_default();
    let d: Vec<&str> = dates.split('|').collect();
    let mut out = Vec::new();
    if let [day, short, time] = d.as_slice() {
        out.push(("Today", day.to_string()));
        out.push(("Short date", short.to_string()));
        out.push(("Time", time.to_string()));
    }
    let dot = f.get("decimal_point").cloned().unwrap_or_else(|| ".".into());
    let sep = f.get("thousands_sep").cloned().unwrap_or_default();
    out.push(("Numbers", format!("{}{dot}89", group_digits(1_234_567, &sep))));
    if let Some(sym) = f.get("currency_symbol").filter(|s| !s.is_empty()) {
        let mdot = f.get("mon_decimal_point").filter(|s| !s.is_empty()).cloned().unwrap_or(dot.clone());
        let msep = f.get("mon_thousands_sep").cloned().unwrap_or(sep.clone());
        let digits: usize = f.get("frac_digits").and_then(|v| v.parse().ok()).filter(|d: &usize| *d < 5).unwrap_or(2);
        let amount = if digits == 0 { group_digits(1234, &msep) } else { format!("{}{mdot}{}", group_digits(1234, &msep), &"56789"[..digits]) };
        let space = if f.get("p_sep_by_space").is_some_and(|v| v != "0") { " " } else { "" };
        let money = if f.get("p_cs_precedes").is_none_or(|v| v == "1") { format!("{sym}{space}{amount}") } else { format!("{amount}{space}{sym}") };
        out.push(("Money", money));
    }
    // The week's first day: `week-1stday` is a date that fell on day 1, and `first_weekday` counts from it.
    if let (Some(base), Some(first)) = (f.get("week-1stday"), f.get("first_weekday").and_then(|v| v.parse::<i32>().ok())) {
        let day = (base.len() == 8)
            .then(|| glib::DateTime::from_utc(base[..4].parse().ok()?, base[4..6].parse().ok()?, base[6..].parse().ok()?, 12, 0, 0.0).ok())
            .flatten()
            .and_then(|d| d.add_days(first - 1).ok())
            .and_then(|d| cmd::output(&["env", &all, "date", "-d", &d.format("%Y-%m-%d").map(|s| s.to_string()).unwrap_or_default(), "+%A"]));
        if let Some(day) = day {
            out.push(("Week starts", day));
        }
    }
    match f.get("measurement").map(String::as_str) {
        Some("1") => out.push(("Measurements", "Metres, kilograms, °C".into())),
        Some("2") => out.push(("Measurements", "Feet, pounds, °F".into())),
        _ => {}
    }
    match (f.get("height").map(String::as_str), f.get("width").map(String::as_str)) {
        (Some("297"), Some("210")) => out.push(("Paper", "A4".into())),
        (Some("279"), Some("216")) => out.push(("Paper", "Letter".into())),
        (Some(h), Some(w)) => out.push(("Paper", format!("{w} × {h} mm"))),
        _ => {}
    }
    out
}

/// A keyboard layout's name (`us` is "English (US)"), from XKB's list.
fn layout_name(code: &str) -> String {
    let text = std::fs::read_to_string("/usr/share/X11/xkb/rules/evdev.lst").unwrap_or_default();
    let names: Vec<String> = code
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(|c| {
            text.lines()
                .skip_while(|l| l.trim() != "! layout")
                .skip(1)
                .take_while(|l| !l.starts_with('!'))
                .find_map(|l| l.trim().split_once(char::is_whitespace).filter(|(k, _)| *k == c).map(|(_, n)| n.trim().to_string()))
                .unwrap_or_else(|| c.to_string())
        })
        .collect();
    names.join(", ")
}

pub fn build(page: &Page) {
    let can_edit = require_helper(page, "region", "changing the date, language and name");
    let show = parse_show(&cmd::output(&["timedatectl", "show"]).unwrap_or_default());
    let zone = show.get("Timezone").cloned().unwrap_or_default();
    let ntp = show.get("NTP").is_some_and(|v| v == "yes");
    let synced = show.get("NTPSynchronized").is_some_and(|v| v == "yes");
    let current = parse_lang(&std::fs::read_to_string("/etc/locale.conf").unwrap_or_default()).unwrap_or_else(|| "C".into());
    let formats = locale_facts(&current);

    preview(page, &zone, &current, &formats, ntp, synced);
    time_group(page, can_edit, &zone, ntp, synced);
    language_group(page, can_edit, &current);
    name_group(page, can_edit);
}

/// The picture: your time zone on a map of day and night, and samples in your formats.
fn preview(page: &Page, zone: &str, locale: &str, formats: &HashMap<String, String>, ntp: bool, synced: bool) {
    let pv = page.split();
    let canonical = canonical_zone(zone);
    let city = canonical.rsplit('/').next().unwrap_or(&canonical).replace('_', " ");
    let now = now_in(zone);
    let mut line = vec![if canonical == zone { zone.to_string() } else { format!("{zone} ({canonical})") }];
    if let Some(n) = &now {
        line.push(utc_offset(n));
        if let Ok(abbr) = n.format("%Z")
            && !abbr.starts_with(['+', '-'])
        {
            line.push(abbr.to_string());
        }
    }
    if let Some(change) = clock_change(zone) {
        line.push(change);
    }
    let twelve = twelve_hour(formats);
    let clock = widgets::hbox(4);
    clock.add_css_class("rgn-now");
    let time = widgets::label("", "rgn-now-time");
    let ampm = widgets::label("", "rgn-now-ampm");
    ampm.set_valign(gtk::Align::End);
    clock.append(&time);
    clock.append(&ampm);
    let tick = {
        let (time, ampm, zone) = (time.clone(), ampm.clone(), zone.to_string());
        move || {
            if let Some(n) = now_in(&zone) {
                let t = n.format(if twelve { "%l:%M" } else { "%H:%M" }).map(|s| s.trim().to_string()).unwrap_or_default();
                time.set_text(&t);
                ampm.set_text(&if twelve { if n.hour() < 12 { "AM".to_string() } else { "PM".to_string() } } else { String::new() });
            }
        }
    };
    tick();
    let content = roomy(&pv);
    content.append(&head(&city, &line.join(" · "), &[clock.upcast()]));

    let place = zone_place(&canonical);
    let offset_hours = now.as_ref().map_or(0.0, |n| n.utc_offset().as_seconds() as f64 / 3600.0);
    let card = widgets::vbox(0);
    card.add_css_class("preview-card");
    card.add_css_class("rgn-map");
    let map = {
        let (city, zone) = (city.clone(), zone.to_string());
        widgets::drawing(270, move |cr, p, section, w, h| draw_map(cr, p, section, w, h, &zone, &city, place, offset_hours, twelve))
    };
    card.append(&map);
    content.append(&card);
    {
        let map = map.clone();
        glib::timeout_add_seconds_local(30, move || {
            if map.root().is_none() {
                return glib::ControlFlow::Break;
            }
            tick();
            map.queue_draw();
            glib::ControlFlow::Continue
        });
    }

    let samples = samples(locale, formats);
    if !samples.is_empty() {
        content.append(&subhead("In your formats", &locale_name(locale)));
        let c = widgets::vbox(0);
        c.add_css_class("preview-card");
        c.add_css_class("rgn-samples");
        let (flow, _) = facts(&samples);
        flow.set_max_children_per_line(4);
        flow.set_homogeneous(true);
        c.append(&flow);
        content.append(&c);
    }
    let (icon, text) = if ntp && synced {
        ("emblem-ok-symbolic", "The clock is right: it's synced with the internet.")
    } else if ntp {
        ("dialog-information-symbolic", "Automatic time is on, but hasn't synced with the internet yet.")
    } else {
        ("dialog-information-symbolic", "Automatic time is off. The clock keeps the time you set.")
    };
    pv.foot(icon, text);
}

/// The land, roughly, as (longitude, latitude) outlines.
const LAND: &[&[(f64, f64)]] = &[
    &[(-168., 66.), (-162., 70.), (-140., 70.), (-125., 71.), (-95., 72.), (-80., 73.), (-62., 67.), (-55., 52.), (-66., 45.), (-75., 38.), (-81., 31.), (-80., 25.), (-90., 29.), (-97., 26.), (-97., 20.), (-90., 16.), (-83., 9.), (-79., 8.), (-85., 14.), (-105., 20.), (-110., 24.), (-117., 32.), (-124., 40.), (-124., 48.), (-135., 58.), (-150., 60.), (-165., 60.)],
    &[(-55., 60.), (-45., 60.), (-20., 70.), (-20., 80.), (-60., 82.), (-72., 78.), (-60., 70.)],
    &[(-80., 9.), (-72., 12.), (-60., 10.), (-50., 0.), (-35., -6.), (-38., -15.), (-48., -26.), (-58., -38.), (-65., -42.), (-68., -55.), (-74., -50.), (-72., -30.), (-71., -18.), (-81., -5.)],
    &[(-10., 36.), (-9., 44.), (-2., 44.), (-5., 48.), (2., 51.), (8., 54.), (5., 58.), (10., 63.), (20., 70.), (30., 71.), (40., 68.), (45., 60.), (50., 50.), (40., 44.), (30., 45.), (28., 41.), (22., 37.), (15., 38.), (12., 44.), (8., 44.), (3., 42.), (-5., 36.)],
    &[(-17., 21.), (-17., 14.), (-12., 7.), (-5., 5.), (8., 4.), (9., -1.), (13., -12.), (12., -18.), (18., -34.), (26., -34.), (33., -26.), (40., -15.), (40., -3.), (51., 12.), (43., 12.), (37., 20.), (33., 31.), (20., 32.), (10., 37.), (-5., 36.), (-10., 30.)],
    &[(28., 41.), (36., 36.), (35., 30.), (44., 13.), (55., 17.), (58., 23.), (62., 25.), (67., 24.), (73., 20.), (77., 8.), (80., 14.), (88., 22.), (92., 21.), (98., 16.), (100., 7.), (104., 1.), (106., 10.), (110., 20.), (118., 24.), (122., 30.), (122., 40.), (128., 35.), (130., 43.), (140., 48.), (142., 55.), (135., 58.), (155., 60.), (160., 62.), (170., 65.), (180., 68.), (180., 72.), (140., 74.), (110., 78.), (80., 74.), (68., 72.), (60., 68.), (45., 68.), (40., 68.), (45., 60.), (50., 50.), (40., 44.)],
    &[(130., 31.), (136., 34.), (140., 36.), (142., 41.), (141., 45.), (139., 40.), (133., 34.)],
    &[(95., 5.), (105., -6.), (115., -8.), (125., -9.), (140., -8.), (141., -3.), (131., -1.), (120., 2.), (117., 7.), (110., 2.), (100., 3.)],
    &[(114., -22.), (114., -34.), (122., -34.), (132., -31.), (138., -35.), (146., -39.), (150., -37.), (153., -28.), (146., -19.), (142., -11.), (136., -12.), (130., -12.), (122., -17.)],
    &[(-6., 50.), (2., 51.), (0., 53.), (-2., 56.), (-3., 58.), (-6., 58.), (-5., 54.), (-3., 52.)],
    &[(44., -25.), (47., -25.), (50., -15.), (49., -12.), (44., -17.)],
    &[(172., -34.), (178., -38.), (174., -41.), (167., -46.), (171., -44.)],
];

/// The top and height of the map, in degrees of latitude.
const TOP: f64 = 80.0;
const SPAN: f64 = 140.0;
const STEP: f64 = 3.6;

fn inside(x: f64, y: f64, poly: &[(f64, f64)]) -> bool {
    let mut c = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let ((xi, yi), (xj, yj)) = (poly[i], poly[j]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            c = !c;
        }
        j = i;
    }
    c
}

/// The dots of land, worked out once.
fn land_dots() -> &'static [(f64, f64)] {
    static DOTS: std::sync::OnceLock<Vec<(f64, f64)>> = std::sync::OnceLock::new();
    DOTS.get_or_init(|| {
        let mut out = Vec::new();
        let mut lat = TOP - 2.0;
        while lat > TOP - SPAN {
            let mut lon = -178.0;
            while lon < 180.0 {
                if LAND.iter().any(|p| inside(lon, lat, p)) {
                    out.push((lon, lat));
                }
                lon += STEP;
            }
            lat -= STEP;
        }
        out
    })
}

/// Cities whose time the map shows besides yours: (zone, name).
const OTHERS: [(&str, &str); 5] = [
    ("Europe/London", "London"),
    ("Asia/Tokyo", "Tokyo"),
    ("Australia/Sydney", "Sydney"),
    ("America/New_York", "New York"),
    ("Asia/Kolkata", "Mumbai"),
];

#[allow(clippy::too_many_arguments)]
fn draw_map(
    cr: &gtk::cairo::Context,
    p: &sketch::Pal,
    section: sketch::Rgb,
    w: f64,
    h: f64,
    zone: &str,
    city: &str,
    place: Option<(f64, f64)>,
    offset: f64,
    twelve: bool,
) {
    let legend = 22.0;
    let mh = h - legend;
    let px = |lon: f64| (lon + 180.0) / 360.0 * w;
    let py = |lat: f64| (TOP - lat) / SPAN * mh;
    let now = glib::DateTime::now_utc().ok();
    // Where the sun is overhead: its longitude from the time of day, its latitude from the season.
    let (sun_lon, dec) = now.as_ref().map_or((0.0, 0.0), |n| {
        let hours = n.hour() as f64 + n.minute() as f64 / 60.0;
        let doy = n.day_of_year() as f64;
        (-15.0 * (hours - 12.0), (-23.44 * (std::f64::consts::TAU / 365.0 * (doy + 10.0)).cos()).to_radians())
    });
    let night = |lon: f64, lat: f64| {
        let la = lat.to_radians();
        la.sin() * dec.sin() + la.cos() * dec.cos() * (lon - sun_lon).to_radians().cos() < 0.0
    };
    // Your time zone's band of longitude.
    let (z0, z1) = (offset * 15.0 - 7.5, offset * 15.0 + 7.5);
    sketch::set(cr, section, 0.12);
    cr.rectangle(px(z0), 0.0, px(z1) - px(z0), mh);
    let _ = cr.fill();
    for &(lon, lat) in land_dots() {
        let (c, a) = if lon >= z0 && lon < z1 {
            (section, 0.9)
        } else if night(lon, lat) {
            (p.dim, 0.28)
        } else {
            (sketch::mix(p.dim, p.text, 0.45), 0.85)
        };
        sketch::set(cr, c, a);
        cr.arc(px(lon), py(lat), 1.9, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
    let fmt = if twelve { "%l:%M %p" } else { "%H:%M" };
    let time_in = |z: &str| now_in(z).and_then(|n| n.format(fmt).ok()).map(|s| s.trim().to_string()).unwrap_or_default();
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
    cr.set_font_size(10.0);
    // Other cities, with their time now.
    let canonical = canonical_zone(zone);
    for (z, name) in OTHERS.iter().filter(|(z, _)| *z != canonical).take(3) {
        let Some((lat, lon)) = zone_place(z) else { continue };
        let (x, y) = (px(lon), py(lat));
        sketch::set(cr, p.text, 0.85);
        cr.arc(x, y, 3.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        let text = format!("{name} {}", time_in(z));
        if let Ok(ext) = cr.text_extents(&text) {
            let right = x + 7.0 + ext.width() < w - 4.0;
            sketch::set(cr, p.text, 0.75);
            cr.move_to(if right { x + 7.0 } else { x - 7.0 - ext.width() }, y + 3.5);
            let _ = cr.show_text(&text);
        }
    }
    // You: a pin, and a tag with the city and its time.
    if let Some((lat, lon)) = place {
        let (x, y) = (px(lon), py(lat));
        sketch::set(cr, section, 0.25);
        cr.arc(x, y, 12.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        sketch::set(cr, section, 1.0);
        cr.arc(x, y, 5.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        let line2 = format!("{} · {}", time_in(zone), now_in(zone).map(|n| utc_offset(&n)).unwrap_or_default());
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
        cr.set_font_size(11.0);
        let w1 = cr.text_extents(city).map(|e| e.width()).unwrap_or(60.0);
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
        cr.set_font_size(10.0);
        let w2 = cr.text_extents(&line2).map(|e| e.width()).unwrap_or(60.0);
        let (tw, th) = (w1.max(w2) + 18.0, 36.0);
        let tx = if x + 14.0 + tw < w { x + 14.0 } else { x - 14.0 - tw };
        let ty = (y - th - 6.0).max(2.0);
        sketch::set(cr, p.bg, 0.88);
        sketch::rounded(cr, tx, ty, tw, th, 7.0);
        let _ = cr.fill_preserve();
        sketch::set(cr, p.text, 0.12);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
        cr.set_font_size(11.0);
        sketch::set(cr, p.text, 1.0);
        cr.move_to(tx + 9.0, ty + 15.0);
        let _ = cr.show_text(city);
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
        cr.set_font_size(10.0);
        sketch::set(cr, p.dim, 1.0);
        cr.move_to(tx + 9.0, ty + 29.0);
        let _ = cr.show_text(&line2);
    }
    // The legend.
    cr.set_font_size(10.5);
    let mut x = 4.0;
    let y = h - 7.0;
    for (c, a, text) in [(sketch::mix(p.dim, p.text, 0.45), 0.85, "Day"), (p.dim, 0.35, "Night"), (section, 1.0, "Your time zone")] {
        sketch::set(cr, c, a);
        cr.arc(x + 4.0, y - 3.5, 3.5, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        sketch::set(cr, p.dim, 1.0);
        cr.move_to(x + 12.0, y);
        let _ = cr.show_text(text);
        x += 22.0 + cr.text_extents(text).map(|e| e.width()).unwrap_or(30.0);
    }
}

fn time_group(page: &Page, can_edit: bool, zone: &str, ntp: bool, synced: bool) {
    let g = page.group("Date and time");
    let sw = gtk::Switch::new();
    sw.set_active(ntp);
    sw.set_sensitive(can_edit);
    sw.connect_active_notify(move |s| {
        let now = s.is_active();
        if now != ntp {
            run_admin(
                &["set-ntp", if now { "1" } else { "0" }],
                None,
                if now { "Automatic time is on" } else { "Automatic time is off" },
                "region",
            );
        }
    });
    let r = widgets::icon_row(
        "view-refresh-symbolic",
        "Set the time automatically",
        if !ntp {
            "Off: the clock keeps the time you set"
        } else if synced {
            "From the internet · in sync"
        } else {
            "From the internet · not synced yet"
        },
        Some(sw.upcast_ref()),
    );
    widgets::keywords("ntp network time sync clock");
    g.add(&r);

    let zones: Vec<String> =
        cmd::output(&["timedatectl", "list-timezones"]).unwrap_or_default().lines().map(str::to_string).collect();
    if !zones.is_empty() {
        let links = zone_links();
        let label = |z: &str| match links.get(z) {
            Some(target) => format!("{} ({z})", zone_name(target)),
            None => zone_name(z),
        };
        let mut options: Vec<(String, String)> = zones.iter().map(|z| (z.clone(), label(z))).collect();
        options.sort_by(|a, b| a.1.cmp(&b.1));
        // Keep a zone like US/Central that the list doesn't offer, so it shows as current.
        if !zone.is_empty() && !zones.iter().any(|z| z == zone) {
            let canonical = canonical_zone(zone);
            options.insert(0, (zone.to_string(), format!("{} ({zone})", zone_name(&canonical))));
        }
        let dd = widgets::dropdown(&options, zone);
        dd.set_enable_search(true);
        dd.set_sensitive(can_edit);
        let (zone, opts) = (zone.to_string(), options.clone());
        dd.connect_selected_notify(move |d| {
            if let Some((z, _)) = opts.get(d.selected() as usize)
                && *z != zone
            {
                run_admin(&["set-timezone", z], None, "Time zone changed", "region");
            }
        });
        let r = widgets::icon_row("find-location-symbolic", "Time zone", "", Some(dd.upcast_ref()));
        r.set_tooltip_text(Some("Where this computer is. Changes the clock shown everywhere."));
        widgets::keywords("timezone clock location utc city");
        g.add(&r);
    }

    if !ntp && can_edit {
        let (r, e) = widgets::entry_row(
            "Set the time",
            "Date and time as 2026-01-31 14:05:00. Press Enter.",
            "",
            "2026-01-31 14:05:00",
            |t| {
                let t = t.trim().to_string();
                if !t.is_empty() {
                    run_admin(&["set-time", &t], None, "Time set", "region");
                }
            },
        );
        e.set_width_chars(18);
        r.prepend(&widgets::tile("alarm-symbolic"));
        r.add_css_class("with-tile");
        widgets::keywords("manual date clock");
        g.add(&r);
    }
}

fn language_group(page: &Page, can_edit: bool, current: &str) {
    let g = page.group("Language and formats");
    let built: Vec<String> =
        cmd::output(&["locale", "-a"]).unwrap_or_default().lines().filter(|l| l.contains("utf8")).map(str::to_string).collect();
    if !built.is_empty() {
        let mut options: Vec<(String, String)> = built.iter().map(|l| (l.clone(), locale_name(l))).collect();
        let selected = built.iter().find(|b| same_locale(b, current)).cloned().unwrap_or_else(|| {
            options.insert(0, (current.to_string(), locale_name(current)));
            current.to_string()
        });
        let dd = widgets::dropdown(&options, &selected);
        dd.set_sensitive(can_edit);
        let opts = options.clone();
        let chosen = selected.clone();
        dd.connect_selected_notify(move |d| {
            if let Some((l, _)) = opts.get(d.selected() as usize)
                && *l != chosen
            {
                run_admin(&["set-locale", &canonical_locale(l)], None, "Language changed. Sign out and back in to use it.", "region");
            }
        });
        let r = widgets::icon_row(
            "preferences-desktop-locale-symbolic",
            "Language",
            "Menus and formats",
            Some(dd.upcast_ref()),
        );
        r.set_tooltip_text(Some("Language and formats for the whole system. Applies after you sign in again."));
        widgets::keywords("locale lang english german french formats region system language");
        g.add(&r);
    }
    if can_edit {
        let (r, _) = widgets::nav_row(Some("list-add-symbolic"), "Add a language", "Install another to choose it above", "", add_language);
        widgets::keywords("locale-gen generate install add language");
        g.add(&r);
    }
}

/// The languages this computer can build: (`de_DE.UTF-8 UTF-8`, its name), from `SUPPORTED`.
fn supported() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_to_string("/usr/share/i18n/SUPPORTED")
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| l.ends_with(" UTF-8"))
        .map(|l| (l.to_string(), l.split_whitespace().next().unwrap_or(l).to_string()))
        .map(|(entry, code)| {
            let name = locale_name(&code);
            (entry, if name == code { code } else { format!("{name} ({code})") })
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

fn add_language() {
    widgets::subpage("Add a language", 560, |page| {
        let g = page.group("Add a language");
        g.note("It's built for this computer, then you can choose it under Language.");
        let options = supported();
        if !options.is_empty() {
            let dd = widgets::dropdown(&options, "");
            dd.set_enable_search(true);
            let add = gtk::Button::with_label("Add");
            add.add_css_class("suggested-action");
            let opts = options.clone();
            let d2 = dd.clone();
            add.connect_clicked(move |_| {
                if let Some((entry, _)) = opts.get(d2.selected() as usize) {
                    run_admin(&["enable-locale", entry], None, "Language added", "region");
                }
            });
            let controls = widgets::hbox(8);
            controls.append(&dd);
            controls.append(&add);
            g.add(&widgets::row("Language", "Type to search the list.", Some(controls.upcast_ref())));
        }
        let (r, _) = widgets::entry_row(
            "Or type it",
            "As it appears in /usr/share/i18n/SUPPORTED, like de_DE.UTF-8 UTF-8. Press Enter.",
            "",
            "de_DE.UTF-8 UTF-8",
            |t| {
                let t = t.trim().to_string();
                if !t.is_empty() {
                    run_admin(&["enable-locale", &t], None, "Language added", "region");
                }
            },
        );
        g.add(&r);
    });
}

fn name_group(page: &Page, can_edit: bool) {
    let g = page.group("This computer");
    let host = cmd::output(&["hostnamectl", "hostname"]).unwrap_or_default();
    if can_edit {
        let (r, e) = widgets::entry_row("Name", "On the network", &host, "name", {
            let host = host.clone();
            move |n| {
                let n = n.trim().to_string();
                if !n.is_empty() && n != host {
                    run_admin(&["set-hostname", &n], None, "Name changed", "region");
                }
            }
        });
        e.set_width_chars(14);
        e.set_tooltip_text(Some("How this computer appears on the network: letters, digits and -. Press Enter to save."));
        r.prepend(&widgets::tile("computer-laptop-symbolic"));
        r.add_css_class("with-tile");
        widgets::keywords("hostname computer name network");
        g.add(&r);
    } else {
        let (r, _) = widgets::info_row("Name", &host);
        r.prepend(&widgets::tile("computer-laptop-symbolic"));
        r.add_css_class("with-tile");
        g.add(&r);
    }
    let layout = crate::backend::hypr::get_str("input.kb_layout").unwrap_or_default();
    let layout = layout.trim_matches('"');
    let (r, _) = widgets::nav_row(
        Some("input-keyboard-symbolic"),
        "Keyboard layout",
        "On the Keyboard page",
        &if layout.is_empty() { String::new() } else { layout_name(layout) },
        || crate::window::navigate("keyboard"),
    );
    widgets::keywords("keyboard layout xkb");
    g.add(&r);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_and_lang() {
        let m = parse_show("Timezone=US/Central\nNTP=yes\nNTPSynchronized=yes\n");
        assert_eq!(m["Timezone"], "US/Central");
        assert_eq!(parse_lang("# x\nLANG=en_US.UTF-8\n"), Some("en_US.UTF-8".into()));
        assert_eq!(parse_lang("LANG=\n"), None);
        assert_eq!(parse_lang(""), None);
    }

    #[test]
    fn locale_names_match() {
        assert!(same_locale("en_US.utf8", "en_US.UTF-8"));
        assert!(!same_locale("en_US.utf8", "de_DE.UTF-8"));
        assert_eq!(canonical_locale("de_DE.utf8"), "de_DE.UTF-8");
        assert_eq!(canonical_locale("sr_RS.utf8@latin"), "sr_RS.UTF-8@latin");
        assert_eq!(canonical_locale("C"), "C");
    }

    #[test]
    fn zones_and_places() {
        assert_eq!(zone_name("America/Argentina/Buenos_Aires"), "Buenos Aires · Argentina");
        assert_eq!(zone_name("UTC"), "UTC");
        let (lat, lon) = parse_coords("+415100-0873900").unwrap();
        assert!((lat - 41.85).abs() < 0.01 && (lon + 87.65).abs() < 0.01);
        let (lat, lon) = parse_coords("-3352+15113").unwrap();
        assert!((lat + 33.87).abs() < 0.01 && (lon - 151.22).abs() < 0.01);
        assert_eq!(group_digits(1_234_567, ","), "1,234,567");
        assert_eq!(group_digits(12, ","), "12");
    }
}
