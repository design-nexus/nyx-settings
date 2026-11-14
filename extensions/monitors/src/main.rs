//! External monitors through DDC/CI (ddcutil): brightness, contrast, volume,
//! input, colour preset and power, on the monitor itself.
//!
//! One page per monitor that answers DDC; the laptop's own screen doesn't,
//! so it never gets one.

mod mock;

use anyhow::{Result, bail};
use nyx_ext::page::{self, Page, desc, refreshes};
use nyx_ext::{Extension, access, sys};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const SETUP: &str = "setup";

static RULE: access::Rule = access::Rule {
    file: "45-nyx-ddc.rules",
    text: "# Written by Settings: lets the signed-in user talk to monitors over DDC/CI.\nKERNEL==\"i2c-[0-9]*\", SUBSYSTEM==\"i2c-dev\", TAG+=\"uaccess\"\n",
    subsystems: &["i2c-dev"],
};

#[derive(Debug, Clone, PartialEq)]
struct Monitor {
    bus: u32,
    maker: String,
    model: String,
    serial: String,
}

impl Monitor {
    fn id(&self) -> String {
        format!("bus{}", self.bus)
    }
    fn title(&self) -> String {
        if self.model.is_empty() { format!("Monitor on bus {}", self.bus) } else { self.model.clone() }
    }
}

/// `ddcutil detect --terse`: the monitors that answer, skipping "Invalid display".
fn parse_detect(text: &str) -> Vec<Monitor> {
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let block = block.trim();
        if !block.starts_with("Display") {
            continue;
        }
        let field = |name: &str| block.lines().find_map(|l| l.trim().strip_prefix(name).map(|v| v.trim().to_string()));
        let Some(bus) = field("I2C bus:").and_then(|b| b.rsplit('-').next().and_then(|n| n.parse().ok())) else { continue };
        let mon = field("Monitor:").unwrap_or_default();
        let mut parts = mon.splitn(3, ':');
        out.push(Monitor {
            bus,
            maker: parts.next().unwrap_or("").to_string(),
            model: parts.next().unwrap_or("").to_string(),
            serial: parts.next().unwrap_or("").to_string(),
        });
    }
    out
}

/// `getvcp … --terse`: code → (current, max) for continuous values, or
/// (value, 0) for choices.
fn parse_values(text: &str) -> BTreeMap<String, (u32, u32)> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["VCP", code, "C", cur, max] => {
                if let (Ok(c), Ok(m)) = (cur.parse(), max.parse()) {
                    out.insert(code.to_uppercase(), (c, m));
                }
            }
            ["VCP", code, "SNC", v, ..] | ["VCP", code, "CNC", v, ..] => {
                if let Ok(n) = u32::from_str_radix(v.trim_start_matches('x').trim_start_matches("0x"), 16) {
                    out.insert(code.to_uppercase(), (n, 0));
                }
            }
            _ => {}
        }
    }
    out
}

/// `capabilities`: each feature's choices (code → [(value, label)]).
fn parse_capabilities(text: &str) -> BTreeMap<String, Vec<(u32, String)>> {
    let mut out: BTreeMap<String, Vec<(u32, String)>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Feature:") {
            current = rest.split_whitespace().next().map(|c| c.to_uppercase());
            if let Some(c) = &current {
                out.entry(c.clone()).or_default();
            }
            continue;
        }
        if let (Some(c), Some((v, label))) = (&current, t.split_once(": "))
            && v.len() <= 2
            && let Ok(n) = u32::from_str_radix(v, 16)
        {
            out.entry(c.clone()).or_default().push((n, label.trim().to_string()));
        }
    }
    out
}

fn ddc(args: &[&str]) -> Result<String> {
    if mock::on() { mock::ddcutil(args) } else { sys::run("ddcutil", args) }
}

fn monitors() -> Vec<Monitor> {
    ddc(&["detect", "--terse"]).map(|t| parse_detect(&t)).unwrap_or_default()
}

/// Capabilities don't change: read once per monitor and keep them.
fn capabilities(m: &Monitor) -> BTreeMap<String, Vec<(u32, String)>> {
    if mock::on() {
        return parse_capabilities(&ddc(&["--bus", &m.bus.to_string(), "capabilities"]).unwrap_or_default());
    }
    let cache = sys::state_dir().join(format!("caps-{}-{}.txt", m.model.replace('/', "_"), m.serial.replace('/', "_")));
    let text = match std::fs::read_to_string(&cache) {
        Ok(t) => t,
        Err(_) => {
            let t = ddc(&["--bus", &m.bus.to_string(), "capabilities"]).unwrap_or_default();
            if !t.is_empty() {
                let _ = std::fs::write(&cache, &t);
            }
            t
        }
    };
    parse_capabilities(&text)
}

fn can_reach() -> bool {
    if mock::on() {
        return mock::can_reach();
    }
    std::fs::read_dir("/dev")
        .map(|rd| {
            let buses: Vec<_> = rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("i2c-")).collect();
            buses.is_empty() || buses.iter().any(|e| std::fs::OpenOptions::new().read(true).write(true).open(e.path()).is_ok())
        })
        .unwrap_or(true)
}

struct Monitors;

impl Extension for Monitors {
    fn pages(&self) -> Value {
        if !mock::on() && !sys::present("ddcutil") {
            return json!([]);
        }
        if !can_reach() {
            return json!([page::entry(SETUP, "Monitors", "video-display-symbolic", "Allow your account to control external monitors.", "monitor ddc brightness")]);
        }
        Value::Array(
            monitors()
                .iter()
                .map(|m| {
                    page::entry(
                        &m.id(),
                        &m.title(),
                        "video-display-symbolic",
                        "Brightness, contrast, volume, input and power on the monitor itself.",
                        &format!("monitor display ddc brightness contrast input hdmi displayport volume {}", m.model.to_lowercase()),
                    )
                })
                .collect(),
        )
    }

    fn describe(&self, id: &str) -> Result<Value> {
        let pane = preview_supported();
        if id == SETUP {
            if pane {
                return Ok(setup_pane());
            }
            return Ok(Page::new("").banner(access::BANNER, true).group("Device access", "", RULE.rows("monitor control")).build());
        }
        let Some(m) = monitors().into_iter().find(|m| m.id() == id) else { bail!("that monitor isn't answering any more") };
        let bus = m.bus.to_string();
        let values = parse_values(&ddc(&["--bus", &bus, "getvcp", "10", "12", "62", "60", "14", "D6", "--terse"]).unwrap_or_default());
        let caps = capabilities(&m);
        let laptop = laptop_brightness();
        // Beside the preview pane, every row has an icon and a one-line description.
        let row = |r: Value, icon: &str, short: &str, long: &str| -> Value {
            if pane {
                let mut r = desc(r, short);
                r["icon"] = json!(icon);
                r
            } else {
                desc(r, long)
            }
        };
        let mut picture = Vec::new();
        for (code, key, title, icon) in [("10", "10", "Brightness", "display-brightness-symbolic"), ("12", "12", "Contrast", "night-light-symbolic")] {
            if let Some((cur, max)) = values.get(code).filter(|(_, max)| *max > 0) {
                let slider = row(page::slider(key, title, *cur as f64, (0.0, *max as f64, 1.0), ""), icon, "", "");
                // The pane shows the brightness as a reading too.
                picture.push(if pane && code == "10" { refreshes(slider) } else { slider });
            }
        }
        if values.contains_key("10") && laptop.is_some() {
            picture.push(row(
                page::button("match", "Match the laptop screen", "Match"),
                "computer-symbolic",
                "Sets this monitor's brightness to the built-in screen's",
                "Sets this monitor's brightness to the built-in screen's.",
            ));
        }
        let choice_row = |code: &str, title: &str| -> Option<Value> {
            let opts = caps.get(code).filter(|o| !o.is_empty())?;
            let cur = values.get(code).map(|v| v.0).unwrap_or(0);
            let opts: Vec<(String, String)> = opts.iter().map(|(v, l)| (format!("{v:02x}"), l.clone())).collect();
            Some(refreshes(page::choice_owned(code, title, &format!("{cur:02x}"), &opts)))
        };
        if let Some(r) = choice_row("14", "Colour preset") {
            picture.push(row(r, "preferences-color-symbolic", "", ""));
        }
        let mut sound_input = Vec::new();
        if let Some(r) = choice_row("60", "Input") {
            sound_input.push(row(
                r,
                "video-joined-displays-symbolic",
                "Switching away shows another computer; switch back on the monitor",
                "Switching away shows another computer's picture; switch back on the monitor.",
            ));
        }
        if let Some((cur, max)) = values.get("62").filter(|(_, max)| *max > 0) {
            sound_input.push(row(page::slider("62", "Speaker volume", *cur as f64, (0.0, *max as f64, 1.0), ""), "audio-volume-medium-symbolic", "", ""));
        }
        let mut power = Vec::new();
        if caps.contains_key("D6") {
            power.push(row(
                page::button("standby", "Turn the monitor off", "Off"),
                "system-shutdown-symbolic",
                "It wakes with its power button or a new picture",
                "It wakes again with its power button or a new picture.",
            ));
        }
        if pane {
            let mut out = Page::new("").group("Picture", "", picture).group("Input and sound", "", sound_input).group("Power", "", power).build();
            out["preview"] = preview(&m, &values, &caps, laptop);
            return Ok(out);
        }
        let mut info = vec![page::info("model", "Model", &m.model), page::info("maker", "Maker code", &m.maker)];
        if !m.serial.is_empty() {
            info.push(page::info("serial", "Serial", &m.serial));
        }
        info.push(page::info("bus", "I2C bus", &format!("/dev/i2c-{}", m.bus)));
        Ok(Page::new(&format!("<b>{}</b> · DDC/CI", m.title().replace('&', "&amp;")))
            .group("Picture", "", picture)
            .group("Input and sound", "", sound_input)
            .group("Power", "", power)
            .group("Monitor", "", info)
            .build())
    }

    fn set(&self, page: &str, key: &str, value: &str) -> Result<Option<Value>> {
        if key == "access" {
            if mock::on() {
                mock::allow();
            } else {
                RULE.install()?;
            }
            return Ok(nyx_ext::reload("Monitor control allowed"));
        }
        let bus = page.trim_start_matches("bus").to_string();
        if bus.parse::<u32>().is_err() {
            bail!("no such monitor");
        }
        match key {
            "10" | "12" | "62" => {
                let v: f64 = value.parse()?;
                ddc(&["--bus", &bus, "--noverify", "setvcp", key, &(v.round() as u32).to_string()])?;
                Ok(None)
            }
            "60" | "14" => {
                if !value.chars().all(|c| c.is_ascii_hexdigit()) || value.len() > 2 {
                    bail!("not a value this monitor offers");
                }
                ddc(&["--bus", &bus, "setvcp", key, &format!("x{value}")])?;
                Ok(nyx_ext::refresh())
            }
            "standby" => {
                ddc(&["--bus", &bus, "--noverify", "setvcp", "D6", "04"])?;
                Ok(nyx_ext::toast("Monitor off"))
            }
            "match" => {
                let pct = laptop_brightness().unwrap_or(100);
                let max = parse_values(&ddc(&["--bus", &bus, "getvcp", "10", "--terse"])?).get("10").map(|v| v.1).unwrap_or(100);
                ddc(&["--bus", &bus, "--noverify", "setvcp", "10", &(pct * max / 100).to_string()])?;
                Ok(nyx_ext::refresh())
            }
            _ => bail!("unknown setting {key}"),
        }
    }
}

/// Whether this Settings draws a preview pane beside the groups.
fn preview_supported() -> bool {
    std::env::var("SETTINGS_KINDS").is_ok_and(|k| k.split(' ').any(|x| x == "preview"))
}

/// The monitor, what it's set to now, and how it's reached.
fn preview(m: &Monitor, values: &BTreeMap<String, (u32, u32)>, caps: &BTreeMap<String, Vec<(u32, String)>>, laptop: Option<u32>) -> Value {
    let current = |code: &str| {
        let v = values.get(code)?.0;
        caps.get(code)?.iter().find(|(n, _)| *n == v).map(|(_, l)| l.clone())
    };
    let mut stats = Vec::new();
    if let Some((cur, max)) = values.get("10").filter(|(_, max)| *max > 0) {
        let sub = laptop.map(|l| format!("Laptop screen: {l}%")).unwrap_or_default();
        stats.push(json!({ "label": "Brightness", "value": format!("{}%", (cur * 100 + max / 2) / max), "sub": sub }));
    }
    for (code, label) in [("60", "Input"), ("14", "Preset")] {
        if let Some(v) = current(code) {
            stats.push(json!({ "label": label, "value": v }));
        }
    }
    let mut facts = Vec::new();
    for (label, value) in [("Model", &m.model), ("Maker code", &m.maker), ("Serial", &m.serial)] {
        if !value.is_empty() {
            facts.push(json!({ "label": label, "value": value }));
        }
    }
    facts.push(json!({ "label": "I2C bus", "value": format!("/dev/i2c-{}", m.bus) }));
    let mut blocks = vec![json!({ "kind": "picture", "picture": "monitor" })];
    if !stats.is_empty() {
        blocks.push(json!({ "kind": "stats", "items": stats }));
    }
    blocks.push(json!({ "kind": "facts", "items": facts }));
    json!({
        "title": m.title(),
        "line": format!("DDC/CI · bus {} · set on the monitor itself, not by the computer", m.bus),
        "blocks": blocks,
    })
}

/// Before the device rule is installed: the button on the left, what's plugged in on the right.
fn setup_pane() -> Value {
    let mut rows = RULE.rows("monitor control");
    for r in &mut rows {
        r["icon"] = json!("system-lock-screen-symbolic");
        r["label"] = json!("Allow access");
    }
    let mut out = Page::new("").banner(access::BANNER, true).group("Device access", "", rows).build();
    let outputs = connected_outputs();
    let title = match outputs.len() {
        0 => "No external monitor found".to_string(),
        1 => "An external monitor is connected".to_string(),
        n => format!("{n} external monitors are connected"),
    };
    let found = if outputs.is_empty() {
        json!({ "icon": "video-display-symbolic", "label": "Nothing plugged in yet", "sub": "The laptop's own screen doesn't take DDC/CI" })
    } else {
        json!({ "icon": "checkbox-checked-symbolic", "label": "Plugged in and found", "sub": format!("{} connected", outputs.join(" · ")) })
    };
    out["preview"] = json!({
        "title": title,
        "line": "Brightness, input and volume on the monitor itself, over DDC/CI.",
        "blocks": [
            { "kind": "picture", "picture": "monitor" },
            { "kind": "list", "items": [
                found,
                { "icon": "system-lock-screen-symbolic", "label": "Your account can't open it yet", "sub": "Only root can until the rule is installed" },
                { "icon": "security-high-symbolic", "label": "One rule, asked once", "sub": "Asks for your password; nothing else changes" },
            ] },
        ],
    });
    out
}

/// External outputs with a monitor plugged in (`DP-1`, `HDMI-A-1`), from DRM.
fn connected_outputs() -> Vec<String> {
    if mock::on() {
        return mock::outputs();
    }
    let mut out: Vec<String> = std::fs::read_dir("/sys/class/drm")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let (_, connector) = name.split_once('-')?;
            let internal = ["eDP", "LVDS", "DSI", "Writeback"].iter().any(|p| connector.starts_with(p));
            (!internal && sys::read(e.path().join("status")).as_deref() == Some("connected")).then(|| connector.to_string())
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The built-in screen's brightness in percent.
fn laptop_brightness() -> Option<u32> {
    if mock::on() {
        return mock::laptop_brightness();
    }
    let dir = std::fs::read_dir("/sys/class/backlight").ok()?.flatten().next()?.path();
    let cur = sys::read_num(dir.join("brightness"))? as f64;
    let max = sys::read_num(dir.join("max_brightness"))? as f64;
    (max > 0.0).then(|| (cur / max * 100.0).round() as u32)
}

fn main() {
    nyx_ext::run(&Monitors);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_monitors_that_answer() {
        let t = "Display 1\n   I2C bus:  /dev/i2c-4\n   DRM connector:  card1-DP-1\n   Monitor:  DEL:DELL U2720Q:7ABC\n\n\
                 Invalid display\n   I2C bus:  /dev/i2c-15\n   Monitor:  SDC:ATNA60DL01-0:\n";
        let m = parse_detect(t);
        assert_eq!(m, vec![Monitor { bus: 4, maker: "DEL".into(), model: "DELL U2720Q".into(), serial: "7ABC".into() }]);
        assert_eq!(m[0].id(), "bus4");
    }

    #[test]
    fn reads_values() {
        let v = parse_values("VCP 10 C 50 100\nVCP 12 C 75 100\nVCP 60 SNC x0f\nVCP D6 SNC x01\nVCP 62 ERR\n");
        assert_eq!(v["10"], (50, 100));
        assert_eq!(v["60"], (0x0f, 0));
        assert_eq!(v["D6"], (1, 0));
        assert!(!v.contains_key("62"));
    }

    #[test]
    fn reads_choices() {
        let t = "VCP Features:\n   Feature: 10 (Brightness)\n   Feature: 60 (Input Source)\n      Values:\n         0f: DisplayPort-1\n         11: HDMI-1\n         1b: USB-C\n   Feature: 14 (Select color preset)\n      Values:\n         05: 6500 K\n";
        let c = parse_capabilities(t);
        assert_eq!(c["60"], vec![(0x0f, "DisplayPort-1".into()), (0x11, "HDMI-1".into()), (0x1b, "USB-C".into())]);
        assert_eq!(c["14"], vec![(5, "6500 K".into())]);
        assert!(c["10"].is_empty());
    }

    fn rows(page: &Value) -> Vec<&Value> {
        page["groups"].as_array().unwrap().iter().flat_map(|g| g["rows"].as_array().unwrap()).collect()
    }

    // One test, since it changes the environment.
    #[test]
    fn pretend_pages() {
        let state = std::env::temp_dir().join(format!("monitors-mock-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        let pane = "info switch slider choice segmented buttons entry button colour chips curve disclosure camera meter preview icon";
        let set_env = |mock: &str, kinds: &str| unsafe {
            std::env::set_var("MONITORS_MOCK", mock);
            std::env::set_var("SETTINGS_EXTENSION_STATE", &state);
            std::env::set_var("SETTINGS_KINDS", kinds);
        };

        set_env("dell-u2720q", pane);
        let p = Monitors.describe("bus4").unwrap();
        let titles: Vec<&str> = p["groups"].as_array().unwrap().iter().map(|g| g["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Picture", "Input and sound", "Power"]);
        for r in rows(&p) {
            assert!(r["icon"].as_str().is_some_and(|i| !i.is_empty()), "{} has no icon", r["title"]);
        }
        let blocks = p["preview"]["blocks"].as_array().unwrap();
        assert_eq!(blocks.iter().map(|b| b["kind"].as_str().unwrap()).collect::<Vec<_>>(), ["picture", "stats", "facts"]);
        assert_eq!(blocks[0]["picture"], "monitor");
        let stats: Vec<&str> = blocks[1]["items"].as_array().unwrap().iter().map(|i| i["value"].as_str().unwrap()).collect();
        assert_eq!(stats, ["75%", "USB-C", "6500 K"]);
        assert_eq!(blocks[1]["items"][0]["sub"], "Laptop screen: 40%");
        assert_eq!(blocks[2]["items"][3]["value"], "/dev/i2c-4");

        Monitors.set("bus4", "60", "11").unwrap();
        Monitors.set("bus4", "10", "30").unwrap();
        let p = Monitors.describe("bus4").unwrap();
        assert_eq!(p["preview"]["blocks"][1]["items"][0]["value"], "30%");
        assert_eq!(p["preview"]["blocks"][1]["items"][1]["value"], "HDMI-1");
        assert!(Monitors.set("bus4", "60", "22").is_err(), "not an input it offers");

        set_env("dell-u2720q", "info switch slider choice button");
        let p = Monitors.describe("bus4").unwrap();
        assert!(p.get("preview").is_none());
        assert!(rows(&p).iter().all(|r| r.get("icon").is_none()));
        assert_eq!(p["subtitle"], "<b>DELL U2720Q</b> · DDC/CI");
        assert_eq!(p["groups"][3]["title"], "Monitor");

        set_env("no-access", pane);
        assert_eq!(Monitors.pages()[0]["id"], SETUP);
        let p = Monitors.describe(SETUP).unwrap();
        assert_eq!(p["banners"][0]["warning"], true);
        assert_eq!(p["groups"][0]["rows"][0]["icon"], "system-lock-screen-symbolic");
        assert_eq!(p["preview"]["title"], "An external monitor is connected");
        assert_eq!(p["preview"]["blocks"][1]["items"].as_array().unwrap().len(), 3);
        set_env("no-access", "");
        assert!(Monitors.describe(SETUP).unwrap().get("preview").is_none());
        Monitors.set(SETUP, "access", "").unwrap();
        assert_eq!(Monitors.pages()[0]["id"], "bus4");

        unsafe {
            std::env::remove_var("MONITORS_MOCK");
            std::env::remove_var("SETTINGS_KINDS");
        }
        assert!(!mock::on());
        let _ = std::fs::remove_dir_all(&state);
    }
}
