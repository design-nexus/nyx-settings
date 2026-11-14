//! Game controllers: Xbox, PlayStation, Switch Pro, 8BitDo and others the
//! kernel drives. Battery, a live tester (buttons, sticks and triggers, read
//! every second while the page shows), a rumble test, and the lightbar and
//! player lights on PlayStation controllers.

mod input;
mod mock;

use anyhow::{Context, Result, bail};
use input::Pad;
use nyx_ext::page::{self, Page};
use nyx_ext::{Extension, access, sys};
use serde_json::{Value, json};
use std::path::PathBuf;

/// PlayStation controllers' lights are root's until this rule lets the user write them.
static RULE: access::Rule = access::Rule {
    file: "47-nyx-controllers.rules",
    text: "# Written by Settings: lets the signed-in user set game controllers' lights.\n\
SUBSYSTEM==\"leds\", KERNEL==\"*:rgb:indicator\", RUN+=\"/bin/chmod 0666 /sys%p/brightness /sys%p/multi_intensity\"\n\
SUBSYSTEM==\"leds\", KERNEL==\"*:white:player-*\", RUN+=\"/bin/chmod 0666 /sys%p/brightness\"\n",
    subsystems: &["leds"],
};

/// The controller's LEDs: (lightbar folder, player LED folders).
fn leds(pad: &Pad) -> (Option<PathBuf>, Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir("/sys/class/leds") else { return (None, Vec::new()) };
    let mut bar = None;
    let mut players = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        // The LEDs sit on the same HID device as the controller's input.
        let dev = std::fs::canonicalize(e.path().join("device")).ok();
        let same = dev.as_ref().is_some_and(|d| pad.hid.as_ref().is_some_and(|h| d.starts_with(h) || h.starts_with(d)));
        if !same {
            continue;
        }
        if name.ends_with(":rgb:indicator") {
            bar = Some(e.path());
        } else if name.contains(":white:player-") {
            players.push(e.path());
        }
    }
    players.sort();
    (bar, players)
}

fn battery(pad: &Pad) -> Option<(i64, String)> {
    let rd = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if sys::read(p.join("scope")).as_deref() != Some("Device") {
            continue;
        }
        let dev = std::fs::canonicalize(p.join("device")).ok()?;
        if pad.hid.as_ref().is_some_and(|h| dev.starts_with(h) || h.starts_with(&dev)) {
            let cap = sys::read_num(p.join("capacity")).or_else(|| {
                // Some report a level instead of a number.
                sys::read(p.join("capacity_level")).map(|l| match l.as_str() {
                    "Full" => 100,
                    "High" => 75,
                    "Normal" => 50,
                    "Low" => 20,
                    "Critical" => 5,
                    _ => -1,
                })
            })?;
            return Some((cap, sys::read(p.join("status")).unwrap_or_default()));
        }
    }
    None
}

fn bar(v: f64) -> String {
    let n = ((v.clamp(0.0, 1.0)) * 10.0).round() as usize;
    format!("{}{} {:>3.0}%", "█".repeat(n), "░".repeat(10 - n), v * 100.0)
}

fn stick(x: f64, y: f64) -> String {
    format!("x {:+.0}%  y {:+.0}%", x * 100.0, y * 100.0)
}

/// What a page shows about one controller, read from it (or from the pretend one).
pub struct Snapshot {
    pub id: String,
    pub title: String,
    pub battery: Option<(i64, String)>,
    pub bluetooth: bool,
    pub ids: (u32, u32),
    pub rumble: bool,
    pub state: std::result::Result<input::State, String>,
    /// The lightbar's colour, when it has one.
    pub lightbar: Option<String>,
    pub lightbar_writable: bool,
    pub players: usize,
    pub players_lit: usize,
    pub players_writable: bool,
}

fn snapshot(pad: &Pad) -> Snapshot {
    let (lightbar, players) = leds(pad);
    let writable = |p: &PathBuf, f: &str| std::fs::OpenOptions::new().write(true).open(p.join(f)).is_ok();
    let players_writable = !players.is_empty() && players.iter().all(|p| writable(p, "brightness"));
    Snapshot {
        id: pad.id(),
        title: pad.title(),
        battery: battery(pad),
        bluetooth: pad.bluetooth,
        ids: (pad.vendor, pad.product),
        rumble: pad.rumble,
        state: input::state(pad).map_err(|e| e.to_string()),
        lightbar: lightbar.as_ref().map(|b| {
            let rgb: Vec<u8> = sys::read(b.join("multi_intensity")).unwrap_or_default().split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if rgb.len() == 3 { sys::hex(rgb[0], rgb[1], rgb[2]) } else { "#0000ff".into() }
        }),
        lightbar_writable: lightbar.as_ref().is_some_and(|b| writable(b, "multi_intensity")),
        players: players.len(),
        players_lit: if players_writable { players.iter().filter(|p| sys::read_num(p.join("brightness")).unwrap_or(0) > 0).count() } else { 0 },
        players_writable,
    }
}

fn preview_supported() -> bool {
    std::env::var("SETTINGS_KINDS").is_ok_and(|k| k.split(' ').any(|x| x == "preview"))
}

fn icon(mut row: Value, name: &str) -> Value {
    row["icon"] = json!(name);
    row
}

fn players_row(s: &Snapshot) -> Option<Value> {
    if !s.players_writable {
        return None;
    }
    let opts: Vec<(String, String)> = (0..=s.players).map(|n| (n.to_string(), if n == 0 { "Off".into() } else { n.to_string() })).collect();
    let mut row = page::choice_owned("players", "Player lights", &s.players_lit.to_string(), &opts);
    row["kind"] = json!("segmented");
    Some(row)
}

/// Lights it has but this account can't set yet.
fn needs_rule(s: &Snapshot, lights: &[Value]) -> bool {
    (s.lightbar.is_some() || s.players > 0) && lights.is_empty()
}

fn render(s: &Snapshot, pane: bool) -> Value {
    if pane { page_with_preview(s) } else { page(s) }
}

/// The page with the controller and its tester in the preview pane.
fn page_with_preview(s: &Snapshot) -> Value {
    let theme = sys::theme_accent();
    let mut lights = Vec::new();
    let mut picker = Vec::new();
    if let Some(hex) = s.lightbar.as_deref().filter(|_| s.lightbar_writable) {
        let follows = theme.as_deref().is_some_and(|t| t.eq_ignore_ascii_case(hex));
        lights.push(icon(page::desc(page::info("lightbar-now", "Lightbar", hex), if follows { "Follows the theme" } else { "" }), "display-brightness-symbolic"));
        picker.push(page::colour_with_theme("lightbar", "", hex, theme.as_deref()));
    }
    lights.extend(players_row(s).map(|r| icon(r, "avatar-default-symbolic")));
    if needs_rule(s, &lights) {
        lights = RULE.rows("setting the controller's lights").into_iter().map(|r| icon(r, "dialog-password-symbolic")).collect();
    }
    let mut feel = Vec::new();
    if s.rumble {
        feel.push(icon(page::buttons("rumble", "Try the rumble", &[("light", "Light"), ("strong", "Strong"), ("both", "Both")]), "input-gaming-symbolic"));
    }
    let mut v = Page::new("").poll(1).group("Lights", "", lights).group("Rumble", "", feel).build();
    v["preview"] = preview(s, picker);
    v
}

fn preview(s: &Snapshot, picker: Vec<Value>) -> Value {
    let mut picture = json!({ "kind": "picture", "picture": "gamepad" });
    if let Some(hex) = &s.lightbar {
        picture["accent"] = json!(hex);
    }
    let mut blocks = Vec::new();
    match &s.state {
        Ok(st) => {
            let mut held = st.buttons.clone();
            if let Some((x, y)) = st.dpad {
                for (on, name) in [(y < -0.5, "D-pad up"), (y > 0.5, "D-pad down"), (x < -0.5, "D-pad left"), (x > 0.5, "D-pad right")] {
                    if on && !held.iter().any(|h| h == name) {
                        held.push(name.to_string());
                    }
                }
            }
            let (value, sub) = if held.is_empty() { ("Nothing".to_string(), "Press and hold a button") } else { (held.join(" · "), "") };
            picture["items"] = json!([{ "label": "Held", "value": value, "sub": sub }]);
            blocks.push(picture);
            let mut sticks = Vec::new();
            for (label, axis) in [("Left stick", st.left), ("Right stick", st.right)] {
                if let Some((x, y)) = axis {
                    sticks.push(json!({ "label": label, "value": stick(x, y) }));
                }
            }
            for (label, t) in [("Left trigger", st.lt), ("Right trigger", st.rt)] {
                if let Some(t) = t {
                    sticks.push(json!({ "label": label, "value": format!("{:.0}%", t * 100.0) }));
                }
            }
            if !sticks.is_empty() {
                blocks.push(json!({ "kind": "facts", "items": sticks }));
            }
        }
        Err(e) => {
            picture["items"] = json!([{ "label": "Tester", "value": "Can't read it", "sub": e }]);
            blocks.push(picture);
        }
    }
    let mut stats = Vec::new();
    if let Some((cap, status)) = &s.battery {
        stats.push(if *cap < 0 {
            json!({ "label": "Battery", "value": status })
        } else {
            let tone = if *cap <= 15 { "bad" } else if *cap < 30 { "warn" } else { "good" };
            json!({ "label": "Battery", "value": format!("{cap}%"), "sub": status, "tone": tone })
        });
    }
    stats.push(json!({ "label": "Connected by", "value": if s.bluetooth { "Bluetooth" } else { "USB" } }));
    stats.push(json!({ "label": "USB id", "value": format!("{:04x}:{:04x}", s.ids.0, s.ids.1) }));
    blocks.push(json!({ "kind": "stats", "items": stats }));
    if !picker.is_empty() {
        blocks.push(json!({ "kind": "rows", "title": "Lightbar", "rows": picker }));
    }
    json!({ "title": "Tester", "line": "Hold buttons and move the sticks: this updates every second.", "blocks": blocks })
}

/// The page as it was before the preview pane.
fn page(s: &Snapshot) -> Value {
    let mut about = Vec::new();
    if let Some((cap, status)) = &s.battery {
        let v = if *cap < 0 { status.clone() } else { format!("{cap}% · {status}") };
        about.push(page::info("battery", "Battery", &v));
    }
    about.push(page::info("connection", "Connected by", if s.bluetooth { "Bluetooth" } else { "USB" }));
    about.push(page::info("ids", "USB id", &format!("{:04x}:{:04x}", s.ids.0, s.ids.1)));

    let mut tester = Vec::new();
    match &s.state {
        Ok(s) => {
            let held: Vec<&str> = s.buttons.iter().map(String::as_str).collect();
            tester.push(page::info("buttons", "Held", if held.is_empty() { "Nothing: press and hold a button" } else { "" }));
            if !held.is_empty() {
                tester.last_mut().unwrap()["value"] = json!(held.join("  "));
            }
            if let Some((x, y)) = s.left {
                tester.push(page::info("left", "Left stick", &stick(x, y)));
            }
            if let Some((x, y)) = s.right {
                tester.push(page::info("right", "Right stick", &stick(x, y)));
            }
            if let Some(t) = s.lt {
                tester.push(page::info("lt", "Left trigger", &bar(t)));
            }
            if let Some(t) = s.rt {
                tester.push(page::info("rt", "Right trigger", &bar(t)));
            }
            if let Some((x, y)) = s.dpad {
                tester.push(page::info("dpad", "D-pad", &stick(x, y)));
            }
        }
        Err(e) => tester.push(page::info("tester", "Tester", &format!("Can't read it: {e}"))),
    }
    let mut feel = Vec::new();
    if s.rumble {
        feel.push(page::buttons("rumble", "Rumble", &[("light", "Light"), ("strong", "Strong"), ("both", "Both")]));
    }
    let mut lights = Vec::new();
    if let Some(hex) = s.lightbar.as_deref().filter(|_| s.lightbar_writable) {
        lights.push(page::colour_with_theme("lightbar", "Lightbar", hex, sys::theme_accent().as_deref()));
    }
    lights.extend(players_row(s));
    let need_rule = needs_rule(s, &lights);
    let mut page = Page::new(&format!("<b>{}</b>", s.title.replace('&', "&amp;"))).poll(1).group("Controller", "", about).group(
        "Tester",
        "Hold buttons and move the sticks: this updates every second.",
        tester,
    );
    page = page.group("Rumble", "", feel).group("Lights", "", lights);
    if need_rule {
        page = page.group("Lights", "", RULE.rows("setting the controller's lights"));
    }
    page.build()
}

struct Controllers;

const KEYWORDS: &str = "controller gamepad joystick xbox playstation dualsense dualshock switch 8bitdo rumble tester battery";

impl Extension for Controllers {
    fn pages(&self) -> Value {
        let entry = |id: &str, title: &str, name: &str| {
            page::entry(id, title, "input-gaming-symbolic", "Battery, a live button tester, rumble and lights.", &format!("{KEYWORDS} {}", name.to_lowercase()))
        };
        if mock::on() {
            return Value::Array(mock::snapshots().iter().map(|s| entry(&s.id, &s.title, &s.title)).collect());
        }
        Value::Array(input::pads().iter().map(|p| entry(&p.id(), &p.title(), &p.name)).collect())
    }

    fn describe(&self, id: &str) -> Result<Value> {
        let snap = if mock::on() {
            mock::snapshots().into_iter().find(|s| s.id == id)
        } else {
            input::pads().into_iter().find(|p| p.id() == id).map(|p| snapshot(&p))
        };
        let snap = snap.context("that controller isn't connected any more")?;
        Ok(render(&snap, preview_supported()))
    }

    fn set(&self, id: &str, key: &str, value: &str) -> Result<Option<Value>> {
        if mock::on() {
            return mock::set(key, value);
        }
        if key == "access" {
            RULE.install()?;
            return Ok(nyx_ext::refresh());
        }
        let pad = input::pads().into_iter().find(|p| p.id() == id).context("that controller isn't connected any more")?;
        match key {
            "rumble" => {
                let (strong, weak) = match value {
                    "light" => (0, 0x8000),
                    "strong" => (0xc000, 0),
                    _ => (0xffff, 0xffff),
                };
                input::rumble(&pad, strong, weak, 600)?;
                Ok(None)
            }
            "lightbar" => {
                let (r, g, b) = sys::parse_hex(value).context("not a colour")?;
                let (Some(bar), _) = leds(&pad) else { bail!("this controller has no lightbar") };
                std::fs::write(bar.join("multi_intensity"), format!("{r} {g} {b}"))?;
                std::fs::write(bar.join("brightness"), "255")?;
                Ok(None)
            }
            "players" => {
                let n: usize = value.parse()?;
                let (_, players) = leds(&pad);
                for (i, p) in players.iter().enumerate() {
                    std::fs::write(p.join("brightness"), if i < n { "1" } else { "0" })?;
                }
                Ok(None)
            }
            _ => bail!("unknown setting {key}"),
        }
    }
}

fn main() {
    nyx_ext::run(&Controllers);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readouts() {
        assert_eq!(bar(0.5), "█████░░░░░  50%");
        assert_eq!(stick(-1.0, 0.25), "x -100%  y +25%");
    }

    fn rows(page: &Value) -> Vec<&Value> {
        page["groups"].as_array().unwrap().iter().flat_map(|g| g["rows"].as_array().unwrap()).collect()
    }

    #[test]
    fn preview_pane() {
        let pad = mock::snapshots().remove(0);
        let old = render(&pad, false);
        assert!(old.get("preview").is_none());
        assert_eq!(old["groups"][0]["title"], "Controller");
        assert!(rows(&old).iter().any(|r| r["key"] == "lightbar"));

        let new = render(&pad, true);
        let pv = &new["preview"];
        assert_eq!(pv["blocks"][0]["picture"], "gamepad");
        assert_eq!(pv["blocks"][0]["items"][0]["value"], "A / ✕ · RB / R1");
        assert_eq!(pv["blocks"][1]["items"][3], json!({ "label": "Right trigger", "value": "72%" }));
        assert_eq!(pv["blocks"][2]["items"][2]["value"], "054c:0ce6");
        assert_eq!(pv["blocks"][3]["kind"], "rows");
        assert_eq!(pv["blocks"][3]["rows"][0]["key"], "lightbar");
        let left = rows(&new);
        assert!(!left.is_empty() && left.iter().all(|r| r["icon"].as_str().is_some_and(|i| !i.is_empty())), "{left:?}");
        assert!(left.iter().all(|r| r["key"] != "lightbar"));
    }

    #[test]
    fn locked_lights_ask_for_access() {
        let mut pad = mock::snapshots().remove(0);
        pad.lightbar_writable = false;
        pad.players_writable = false;
        let new = render(&pad, true);
        assert_eq!(rows(&new)[0]["key"], "access");
        assert!(rows(&new).iter().all(|r| r.get("icon").is_some()));
        assert!(new["preview"]["blocks"].as_array().unwrap().iter().all(|b| b["kind"] != "rows"));
    }
}
