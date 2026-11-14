//! Drawing tablets (Wacom, XP-Pen, Huion and others): which screen the pen
//! draws on, the part of the tablet that's used, left-handed and mouse-like
//! use. Hyprland does the work; these settings live in
//! `~/.config/hypr/nyx-tablet.lua`, loaded from `hyprland.lua`, and apply at once.

mod mock;

use anyhow::{Context, Result, bail};
use nyx_ext::page::{self, Page, desc, refreshes};
use nyx_ext::{Extension, sys};
use serde_json::{Value, json};
use std::path::PathBuf;

const PAGE: &str = "tablet";
const REQUIRE: &str = "require(\"hypr.nyx-tablet\") -- Drawing tablets (Settings)";

fn lua_file() -> PathBuf {
    sys::home().join(".config/hypr/nyx-tablet.lua")
}

fn state_file() -> PathBuf {
    sys::state_dir().join(if mock::on() { "tablet-mock.json" } else { "tablet.json" })
}

/// What's set here, kept as JSON beside the Lua it writes.
#[derive(Debug, Clone, PartialEq, Default)]
struct Settings {
    output: String,
    left_handed: bool,
    relative: bool,
    /// Percent of the tablet used (100: all), and whether it keeps the screen's shape.
    area: u32,
    match_shape: bool,
}

impl Settings {
    fn load() -> Settings {
        let v: Value = std::fs::read_to_string(state_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        Settings {
            output: v["output"].as_str().unwrap_or("").to_string(),
            left_handed: v["left_handed"].as_bool().unwrap_or(false),
            relative: v["relative"].as_bool().unwrap_or(false),
            area: v["area"].as_u64().unwrap_or(100) as u32,
            match_shape: v["match_shape"].as_bool().unwrap_or(false),
        }
    }

    fn save(&self) -> Result<()> {
        let v = json!({ "output": self.output, "left_handed": self.left_handed, "relative": self.relative, "area": self.area, "match_shape": self.match_shape });
        std::fs::write(state_file(), v.to_string())?;
        Ok(())
    }
}

/// The tablet's size in mm, from libwacom (it lists it in inches).
fn tablet_size() -> Option<(f64, f64)> {
    let text = sys::run("libwacom-list-local-devices", &["--format=datafile"]).ok()?;
    let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(&format!("{k}=")).and_then(|v| v.trim().parse::<f64>().ok()));
    Some((field("Width")? * 25.4, field("Height")? * 25.4))
}

/// libwacom's description of the tablet: name and button count.
fn tablet_info() -> Option<(String, usize)> {
    let text = sys::run("libwacom-list-local-devices", &["--format=datafile"]).ok()?;
    let name = text.lines().find_map(|l| l.strip_prefix("Name="))?.trim().to_string();
    let mut buttons = 0;
    let mut in_buttons = false;
    for l in text.lines() {
        if l.starts_with('[') {
            in_buttons = l.trim() == "[Buttons]";
            continue;
        }
        if in_buttons && let Some((k, v)) = l.split_once('=') && ["Left", "Right", "Top", "Bottom"].contains(&k.trim()) {
            buttons += v.split(';').filter(|b| !b.trim().is_empty()).count();
        }
    }
    Some((name, buttons))
}

/// The active area (mm, top-left) for `s`, centred on the tablet.
fn area(s: &Settings, size: (f64, f64), screen: Option<(f64, f64)>) -> Option<((f64, f64), (f64, f64))> {
    if s.area >= 100 && !s.match_shape {
        return None;
    }
    let k = s.area.clamp(20, 100) as f64 / 100.0;
    let (mut w, mut h) = (size.0 * k, size.1 * k);
    if s.match_shape && let Some((sw, sh)) = screen.filter(|(w, h)| *w > 0.0 && *h > 0.0) {
        // Narrow the longer side so the area has the screen's shape.
        let want = sw / sh;
        if w / h > want { w = h * want } else { h = w / want }
    }
    Some(((w.round(), h.round()), (((size.0 - w) / 2.0).round(), ((size.1 - h) / 2.0).round())))
}

fn monitors() -> Vec<(String, f64, f64)> {
    let Ok(text) = sys::run("hyprctl", &["monitors", "-j"]) else { return Vec::new() };
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| Some((m["name"].as_str()?.to_string(), m["width"].as_f64()?, m["height"].as_f64()?)))
                .collect()
        })
        .unwrap_or_default()
}

/// The Lua for `s`.
fn lua(s: &Settings) -> String {
    let screens = monitors();
    let screen = screens.iter().find(|m| m.0 == s.output).or(screens.first()).map(|m| (m.1, m.2));
    let mut fields = vec![
        format!("output = {:?}", s.output),
        format!("left_handed = {}", s.left_handed),
        format!("relative_input = {}", s.relative),
    ];
    match tablet_size().and_then(|size| area(s, size, screen)) {
        Some(((w, h), (x, y))) => {
            fields.push(format!("active_area_size = {{ {w}, {h} }}"));
            fields.push(format!("active_area_position = {{ {x}, {y} }}"));
        }
        None => {
            fields.push("active_area_size = { 0, 0 }".into());
            fields.push("active_area_position = { 0, 0 }".into());
        }
    }
    format!("hl.config({{ input = {{ tablet = {{ {} }} }} }})", fields.join(", "))
}

/// Write the file, make sure hyprland.lua loads it, and apply it now.
fn apply(s: &Settings) -> Result<()> {
    let code = lua(s);
    std::fs::write(lua_file(), format!("-- Written by Settings (Drawing tablets). Change it there.\n{code}\n"))?;
    let main = sys::home().join(".config/hypr/hyprland.lua");
    if let Ok(text) = std::fs::read_to_string(&main)
        && !text.contains("hypr.nyx-tablet")
    {
        // Before Settings' own file, which is loaded last.
        let new = match text.find("require(\"hypr.settings\")") {
            Some(i) => format!("{}{REQUIRE}\n{}", &text[..i], &text[i..]),
            None => format!("{}\n{REQUIRE}\n", text.trim_end()),
        };
        std::fs::write(&main, new)?;
    }
    let out = sys::run("hyprctl", &["eval", &code])?;
    if out.to_lowercase().contains("error") {
        bail!("Hyprland: {}", out.trim());
    }
    s.save()
}

fn tablets() -> Vec<String> {
    let Ok(text) = sys::run("hyprctl", &["devices", "-j"]) else { return Vec::new() };
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    v["tablets"].as_array().map(|a| a.iter().filter_map(|t| t["name"].as_str().map(String::from)).collect()).unwrap_or_default()
}

/// What a page shows about the tablets, from Hyprland and libwacom (or the pretend one).
pub struct Hardware {
    pub tablets: Vec<String>,
    pub monitors: Vec<(String, f64, f64)>,
    /// The drawing area in mm.
    pub size: Option<(f64, f64)>,
    /// libwacom's name for it and its button count.
    pub info: Option<(String, usize)>,
    /// Whether OpenTabletDriver's window is installed.
    pub otd: bool,
}

impl Hardware {
    fn read() -> Hardware {
        if mock::on() {
            return mock::hardware();
        }
        Hardware { tablets: tablets(), monitors: monitors(), size: tablet_size(), info: tablet_info(), otd: sys::present("otd-gui") }
    }
}

fn preview_supported() -> bool {
    std::env::var("SETTINGS_KINDS").is_ok_and(|k| k.split(' ').any(|x| x == "preview"))
}

fn icon(mut row: Value, name: &str) -> Value {
    row["icon"] = json!(name);
    row
}

fn screens(hw: &Hardware) -> Vec<(String, String)> {
    let mut screens: Vec<(String, String)> = vec![(String::new(), "All screens".into())];
    screens.extend(hw.monitors.iter().map(|(n, w, h)| (n.clone(), format!("{n} ({w:.0}×{h:.0})"))));
    screens
}

fn render(s: &Settings, hw: &Hardware, pane: bool) -> Value {
    if pane { page_with_preview(s, hw) } else { page(s, hw) }
}

/// The page with the tablet and its facts in the preview pane.
fn page_with_preview(s: &Settings, hw: &Hardware) -> Value {
    let mut pen = vec![icon(desc(page::choice_owned("output", "Draw on", &s.output, &screens(hw)), "The whole tablet maps to this screen"), "video-display-symbolic")];
    if hw.size.is_some() {
        pen.push(icon(refreshes(desc(page::slider("area", "Area used", s.area as f64, (20.0, 100.0, 5.0), " %"), "Smaller means shorter moves")), "view-fullscreen-symbolic"));
        pen.push(icon(refreshes(desc(page::switch("shape", "Match the screen's shape", s.match_shape), "Keeps circles round")), "zoom-fit-best-symbolic"));
    }
    pen.push(icon(desc(page::switch("left", "Left-handed", s.left_handed), "Turns the tablet around, buttons on the right"), "object-flip-horizontal-symbolic"));
    pen.push(icon(desc(page::switch("relative", "Move like a mouse", s.relative), "The pen moves the pointer from where it is"), "input-mouse-symbolic"));
    let mut extra = Vec::new();
    if hw.otd {
        extra.push(icon(
            desc(page::button("otd", "Pressure curve and more", "Open OpenTabletDriver"), "OpenTabletDriver sets the pressure curve and button actions"),
            "preferences-system-symbolic",
        ));
    }
    let mut v = Page::new("").group("Pen", "Applied straight away.", pen).group("More", "", extra).build();
    v["preview"] = preview(s, hw);
    v
}

fn preview(s: &Settings, hw: &Hardware) -> Value {
    let title = hw.info.as_ref().map(|i| i.0.clone()).or_else(|| hw.tablets.first().cloned()).unwrap_or_else(|| "Drawing tablet".into());
    let onto = if s.output.is_empty() { "every screen".to_string() } else { s.output.clone() };
    let mut picture = json!({ "kind": "picture", "picture": "tablet" });
    let mut line = String::new();
    if hw.size.is_some() {
        line = format!("The area the pen uses, mapped onto {onto}");
        let shape = if s.match_shape { "In the screen's shape" } else { "" };
        picture["items"] = json!([{ "label": "Area used", "value": format!("{}%", s.area), "sub": shape }]);
    }
    let mut facts = Vec::new();
    for t in &hw.tablets {
        facts.push(json!({ "label": "Connected", "value": t }));
    }
    if let Some((name, buttons)) = &hw.info {
        facts.push(json!({ "label": "Model", "value": name }));
        if let Some((w, h)) = hw.size {
            facts.push(json!({ "label": "Drawing area", "value": format!("{w:.0} × {h:.0} mm") }));
        }
        if *buttons > 0 {
            facts.push(json!({ "label": "Tablet buttons", "value": buttons.to_string(), "sub": "Drawing apps choose what they do" }));
        }
    } else if let Some((w, h)) = hw.size {
        facts.push(json!({ "label": "Drawing area", "value": format!("{w:.0} × {h:.0} mm") }));
    }
    let mut blocks = vec![picture];
    if !facts.is_empty() {
        blocks.push(json!({ "kind": "facts", "items": facts }));
    }
    json!({ "title": title, "line": line, "blocks": blocks })
}

/// The page as it was before the preview pane.
fn page(s: &Settings, hw: &Hardware) -> Value {
    let mut pen = vec![desc(page::choice_owned("output", "Draw on", &s.output, &screens(hw)), "The whole tablet maps to this screen.")];
    if hw.size.is_some() {
        pen.push(refreshes(desc(page::slider("area", "Area used", s.area as f64, (20.0, 100.0, 5.0), " %"), "A smaller area, centred, means shorter pen moves.")));
        pen.push(desc(page::switch("shape", "Match the screen's shape", s.match_shape), "Keeps circles round: trims the tablet's area to the screen's proportions."));
    }
    pen.push(desc(page::switch("left", "Left-handed", s.left_handed), "Turns the tablet around, with the buttons on the right."));
    pen.push(desc(page::switch("relative", "Move like a mouse", s.relative), "The pen moves the pointer from where it is, instead of jumping to where it touches."));
    let mut about = Vec::new();
    for t in &hw.tablets {
        about.push(page::info(t, "Connected", t));
    }
    if let Some((name, buttons)) = &hw.info {
        about.push(page::info("model", "Model", name));
        if *buttons > 0 {
            about.push(desc(page::info("buttons", "Tablet buttons", &buttons.to_string()), "Drawing apps (Krita, GIMP) choose what they do."));
        }
    }
    if let Some((w, h)) = hw.size {
        about.push(page::info("size", "Drawing area", &format!("{w:.0} × {h:.0} mm")));
    }
    let mut extra = Vec::new();
    if hw.otd {
        extra.push(desc(page::button("otd", "Pressure curve and more", "Open OpenTabletDriver"), "OpenTabletDriver is installed: its window sets the pressure curve and button actions."));
    }
    Page::new("Settings for the pen, applied straight away.").group("Pen", "", pen).group("Tablet", "", about).group("More", "", extra).build()
}

struct Tablet;

impl Extension for Tablet {
    fn pages(&self) -> Value {
        let tablets = if mock::on() { mock::hardware().tablets } else { tablets() };
        if tablets.is_empty() {
            return json!([]);
        }
        json!([page::entry(PAGE, "Drawing tablet", "input-tablet-symbolic", "Which screen the pen draws on, the area used, left-handed and mouse-like use.",
            "tablet wacom xp-pen huion pen stylus drawing area screen left-handed")])
    }

    fn describe(&self, _id: &str) -> Result<Value> {
        Ok(render(&Settings::load(), &Hardware::read(), preview_supported()))
    }

    fn set(&self, _page: &str, key: &str, value: &str) -> Result<Option<Value>> {
        let mut s = Settings::load();
        match key {
            "output" => {
                let screens = if mock::on() { mock::hardware().monitors } else { monitors() };
                if !value.is_empty() && !screens.iter().any(|m| m.0 == value) {
                    bail!("that screen isn't connected");
                }
                s.output = value.to_string();
            }
            "area" => s.area = value.parse::<f64>().context("not a number")?.round() as u32,
            "shape" => s.match_shape = value == "true",
            "left" => s.left_handed = value == "true",
            "relative" => s.relative = value == "true",
            "otd" if mock::on() => return Ok(None),
            "otd" => {
                std::process::Command::new("otd-gui").spawn()?;
                return Ok(None);
            }
            _ => bail!("unknown setting {key}"),
        }
        if mock::on() {
            s.save()?;
            return Ok(None);
        }
        apply(&s)?;
        Ok(None)
    }
}

fn main() {
    nyx_ext::run(&Tablet);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_tablet_needs_no_area() {
        let s = Settings { area: 100, ..Default::default() };
        assert_eq!(area(&s, (152.0, 95.0), Some((2560.0, 1600.0))), None);
    }

    #[test]
    fn smaller_area_is_centred() {
        let s = Settings { area: 50, ..Default::default() };
        assert_eq!(area(&s, (200.0, 100.0), None), Some(((100.0, 50.0), (50.0, 25.0))));
    }

    #[test]
    fn matching_the_screen_keeps_its_shape() {
        let s = Settings { area: 100, match_shape: true, ..Default::default() };
        let ((w, h), (x, _)) = area(&s, (216.0, 135.0), Some((1920.0, 1080.0))).unwrap();
        assert!(((w / h) - 16.0 / 9.0).abs() < 0.02, "{w}×{h}");
        assert_eq!(w, 216.0);
        assert_eq!(x, 0.0);
    }

    fn rows(page: &Value) -> Vec<&Value> {
        page["groups"].as_array().unwrap().iter().flat_map(|g| g["rows"].as_array().unwrap()).collect()
    }

    #[test]
    fn preview_pane() {
        let s = Settings { area: 80, match_shape: true, output: "eDP-1".into(), ..Default::default() };
        let hw = mock::hardware();
        let old = render(&s, &hw, false);
        assert!(old.get("preview").is_none());
        assert_eq!(old["groups"][1]["title"], "Tablet");

        let new = render(&s, &hw, true);
        let pv = &new["preview"];
        assert_eq!(pv["title"], "Wacom Intuos S (CTL-4100)");
        assert_eq!(pv["line"], "The area the pen uses, mapped onto eDP-1");
        assert_eq!(pv["blocks"][0]["picture"], "tablet");
        assert_eq!(pv["blocks"][0]["items"][0]["value"], "80%");
        let facts: Vec<&str> = pv["blocks"][1]["items"].as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
        assert_eq!(facts, ["Connected", "Model", "Drawing area", "Tablet buttons"]);
        assert_eq!(pv["blocks"][1]["items"][2]["value"], "152 × 95 mm");
        let groups: Vec<&str> = new["groups"].as_array().unwrap().iter().map(|g| g["title"].as_str().unwrap()).collect();
        assert_eq!(groups, ["Pen", "More"]);
        let left = rows(&new);
        assert_eq!(left.len(), 6);
        assert!(left.iter().all(|r| r["icon"].as_str().is_some_and(|i| !i.is_empty())), "{left:?}");
    }

    #[test]
    fn preview_without_libwacom() {
        let hw = Hardware { tablets: vec!["xp-pen".into()], monitors: vec![], size: None, info: None, otd: false };
        let new = render(&Settings::default(), &hw, true);
        assert_eq!(new["preview"]["title"], "xp-pen");
        assert_eq!(new["preview"]["line"], "");
        assert!(new["preview"]["blocks"][0].get("items").is_none());
        assert_eq!(rows(&new).len(), 3);
    }
}
