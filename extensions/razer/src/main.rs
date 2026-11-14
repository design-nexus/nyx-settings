//! Razer mice, keyboards and headsets through OpenRazer's daemon (D-Bus,
//! `org.razer` on the session bus): DPI, polling rate, battery, idle time,
//! lighting effects and brightness.

mod mock;

use anyhow::{Context, Result, bail};
use nyx_ext::page::{self, Page, desc, refreshes};
use nyx_ext::{Extension, sys};
use serde_json::{Value, json};

const SETUP: &str = "setup";

/// A `busctl --user call` to the daemon; its reply as text.
fn call(path: &str, iface: &str, method: &str, args: &[&str]) -> Result<String> {
    if mock::on() {
        return mock::call(path, iface, method, args);
    }
    let mut a = vec!["--user", "call", "org.razer", path, iface, method];
    a.extend(args);
    sys::run("busctl", &a)
}

/// busctl's reply: its type, then the values ("s \"Basilisk\"", "aq 800 800", "d 87.5").
fn values(reply: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = reply.trim();
    // Skip the signature.
    if let Some(i) = rest.find(' ') {
        rest = &rest[i + 1..];
    } else {
        return out;
    }
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            let mut s = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        if let Some(n) = chars.next() {
                            s.push(n);
                        }
                    }
                    '"' => break,
                    _ => s.push(c),
                }
            }
            out.push(s);
        } else if !c.is_whitespace() {
            let mut s = c.to_string();
            while let Some(&n) = chars.peek() {
                if n.is_whitespace() {
                    break;
                }
                s.push(n);
                chars.next();
            }
            out.push(s);
        }
    }
    out
}

fn get(path: &str, iface: &str, method: &str) -> Option<Vec<String>> {
    call(path, iface, method, &[]).ok().map(|r| values(&r))
}

fn first(path: &str, iface: &str, method: &str) -> Option<String> {
    get(path, iface, method)?.into_iter().last()
}

#[derive(Debug)]
struct Device {
    serial: String,
    path: String,
    name: String,
    kind: String,
}

fn devices() -> Vec<Device> {
    let Some(serials) = get("/org/razer", "razer.devices", "getDevices") else { return Vec::new() };
    // "as N serial…": the first value is the count.
    serials
        .into_iter()
        .skip(1)
        .map(|serial| {
            let path = format!("/org/razer/device/{serial}");
            let name = first(&path, "razer.device.misc", "getDeviceName").unwrap_or_else(|| "Razer device".into());
            let kind = first(&path, "razer.device.misc", "getDeviceType").unwrap_or_default();
            Device { serial, path, name, kind }
        })
        .collect()
}

fn icon(kind: &str) -> &'static str {
    match kind {
        "mouse" => "input-mouse-symbolic",
        "keyboard" | "keypad" => "input-keyboard-symbolic",
        "headset" => "audio-headset-symbolic",
        _ => "input-gaming-symbolic",
    }
}

/// The lighting effects offered, by the method each uses.
const EFFECTS: &[(&str, &str)] = &[
    ("setStatic", "Still colour"),
    ("setBreathSingle", "Breathing"),
    ("setSpectrum", "Colour cycle"),
    ("setWave", "Wave"),
    ("setReactive", "React to presses"),
    ("setNone", "Off"),
];

/// What the device page shows, read from the daemon.
struct Readings {
    /// Sensitivity now and the most it goes to.
    dpi: Option<(f64, f64)>,
    poll: Option<String>,
    /// Battery percent and whether it's charging.
    battery: Option<(f64, bool)>,
    /// Seconds before it sleeps (read only for devices with a battery).
    idle: Option<f64>,
    lighting: bool,
    brightness: Option<f64>,
}

impl Readings {
    fn read(p: &str) -> Readings {
        let dpi = get(p, "razer.device.dpi", "getDPI").and_then(|dpi| {
            let max = first(p, "razer.device.dpi", "maxDPI").and_then(|m| m.parse::<f64>().ok()).unwrap_or(16000.0);
            dpi.get(1).and_then(|v| v.parse::<f64>().ok()).map(|x| (x, max))
        });
        let battery = first(p, "razer.device.power", "getBattery")
            .and_then(|v| v.parse::<f64>().ok())
            .map(|b| (b, first(p, "razer.device.power", "isCharging").is_some_and(|c| c == "true")));
        let idle = battery.and_then(|_| first(p, "razer.device.power", "getIdleTime").and_then(|v| v.parse::<f64>().ok()));
        let lighting = call(p, "razer.device.lighting.chroma", "getEffect", &[]).is_ok() || call(p, "razer.device.lighting.brightness", "getBrightness", &[]).is_ok();
        let brightness = lighting.then(|| first(p, "razer.device.lighting.brightness", "getBrightness").and_then(|v| v.parse::<f64>().ok())).flatten();
        Readings { dpi, poll: first(p, "razer.device.misc", "getPollRate"), battery, idle, lighting, brightness }
    }
}

fn preview_supported() -> bool {
    std::env::var("SETTINGS_KINDS").is_ok_and(|k| k.split(' ').any(|x| x == "preview"))
}

fn with_icon(mut row: Value, name: &str) -> Value {
    row["icon"] = json!(name);
    row
}

fn picture(kind: &str) -> &'static str {
    match kind {
        "keyboard" | "keypad" => "keyboard",
        "headset" => "headset",
        _ => "mouse",
    }
}

/// 26000 → "26,000".
fn thousands(n: f64) -> String {
    let digits = format!("{:.0}", n.round());
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 && c.is_ascii_digit() {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn battery_tone(b: f64) -> &'static str {
    if b <= 15.0 { "bad" } else if b < 30.0 { "warn" } else { "good" }
}

fn poll_row(rate: &str) -> Value {
    let opts: Vec<(String, String)> = ["125", "500", "1000"].iter().map(|r| (r.to_string(), format!("{r} Hz"))).collect();
    let mut row = page::choice_owned("poll", "Polling rate", rate, &opts);
    row["kind"] = json!("segmented");
    row
}

fn effect_row(value: &str) -> Value {
    let opts: Vec<(String, String)> = EFFECTS.iter().map(|(m, l)| (m.to_string(), l.to_string())).collect();
    refreshes(page::choice_owned("effect", "Effect", value, &opts))
}

fn colour_row(serial: &str, title: &str) -> Value {
    page::colour_with_theme("colour", title, &sys::read(sys::state_dir().join(format!("{serial}.colour"))).unwrap_or_else(|| "#00ff00".into()), sys::theme_accent().as_deref())
}

fn render(d: &Device, r: &Readings, pane: bool) -> Value {
    if pane { page_with_preview(d, r) } else { page(d, r) }
}

/// The page as it was before the preview pane.
fn page(d: &Device, r: &Readings) -> Value {
    let mut perf = Vec::new();
    if let Some((x, max)) = r.dpi {
        perf.push(desc(page::slider("dpi", "Sensitivity (DPI)", x, (100.0, max, 50.0), ""), "Higher moves the pointer further for the same hand movement."));
    }
    if let Some(rate) = &r.poll {
        perf.push(desc(poll_row(rate), "How often it reports its position. 1000 Hz is smoothest; lower saves a wireless battery."));
    }
    let mut power = Vec::new();
    if let Some((b, charging)) = r.battery {
        power.push(page::info("battery", "Battery", &format!("{b:.0}%{}", if charging { " · charging" } else { "" })));
        if let Some(idle) = r.idle {
            power.push(desc(page::slider("idle", "Sleep after", (idle / 60.0).round(), (1.0, 15.0, 1.0), " min"), "Idle time before it sleeps to save the battery."));
        }
    }
    let mut light = Vec::new();
    if r.lighting {
        light.push(effect_row(""));
        light.push(colour_row(&d.serial, "Colour"));
        if let Some(b) = r.brightness {
            light.push(page::slider("brightness", "Brightness", b.round(), (0.0, 100.0, 5.0), " %"));
        }
    }
    let info = vec![page::info("kind", "Kind", &d.kind), page::info("serial", "Serial", &d.serial)];
    Page::new(&format!("<b>{}</b>", d.name.replace('&', "&amp;")))
        .group("Pointer", "", perf)
        .group("Power", "", power)
        .group("Lighting", "", light)
        .group("Device", "", info)
        .build()
}

/// The page with the device, its colour and readings in the preview pane.
fn page_with_preview(d: &Device, r: &Readings) -> Value {
    let mut perf = Vec::new();
    if let Some((x, max)) = r.dpi {
        perf.push(with_icon(desc(page::slider("dpi", "Sensitivity", x, (100.0, max, 50.0), " dpi"), "Higher is faster"), "input-mouse-symbolic"));
    }
    if let Some(rate) = &r.poll {
        perf.push(with_icon(desc(poll_row(rate), "Lower saves battery"), "view-refresh-symbolic"));
    }
    let mut power = Vec::new();
    if let Some(idle) = r.idle {
        power.push(with_icon(desc(page::slider("idle", "Sleep after", (idle / 60.0).round(), (1.0, 15.0, 1.0), " min"), "Idle time before it sleeps"), "weather-clear-night-symbolic"));
    }
    let mut light = Vec::new();
    let mut picker = Vec::new();
    if r.lighting {
        let last = sys::read(sys::state_dir().join(format!("{}.effect", d.serial))).unwrap_or_default();
        light.push(with_icon(effect_row(&last), "starred-symbolic"));
        picker.push(colour_row(&d.serial, ""));
        if let Some(b) = r.brightness {
            light.push(with_icon(page::slider("brightness", "Brightness", b.round(), (0.0, 100.0, 5.0), " %"), "display-brightness-symbolic"));
        }
    }
    let mut blocks = vec![json!({ "kind": "picture", "picture": picture(&d.kind) })];
    if !picker.is_empty() {
        blocks.push(json!({ "kind": "rows", "title": "Colour", "rows": picker }));
    }
    let mut stats = Vec::new();
    if let Some((b, charging)) = r.battery {
        stats.push(json!({ "label": "Battery", "value": format!("{b:.0}%"), "sub": if charging { "Charging" } else { "" }, "tone": battery_tone(b) }));
    }
    if let Some((x, max)) = r.dpi {
        stats.push(json!({ "label": "Sensitivity", "value": format!("{} dpi", thousands(x)), "sub": format!("Up to {}", thousands(max)) }));
    }
    if let Some(rate) = &r.poll {
        stats.push(json!({ "label": "Polling", "value": format!("{rate} Hz") }));
    }
    if !stats.is_empty() {
        blocks.push(json!({ "kind": "stats", "items": stats }));
    }
    let preview = json!({ "title": d.name, "line": format!("Through OpenRazer · serial {}", d.serial), "blocks": blocks });
    let mut v = Page::new("").group("Pointer", "", perf).group("Power", "", power).group("Lighting", "", light).build();
    v["preview"] = preview;
    v
}

const SETUP_BANNER: &str = "OpenRazer is installed, but its daemon isn't answering. Your account needs to be in the <tt>plugdev</tt> group, and the daemon running.";

fn setup_row() -> Value {
    desc(page::button("setup", "Set up OpenRazer", "Set up…"), "Adds you to <tt>plugdev</tt> and starts the daemon. Asks for your password; sign out and in afterwards.")
}

/// The setup page with what was found and what's missing in the preview pane.
fn setup_with_preview(usb: &[(String, String)], answering: bool, (in_group, in_session): (bool, bool)) -> Value {
    let item = |icon: &str, label: &str, sub: &str| json!({ "icon": icon, "label": label, "sub": sub });
    let mut items: Vec<Value> = usb.iter().map(|(name, id)| item("emblem-ok-symbolic", &format!("{name} found"), &format!("USB {id}"))).collect();
    items.push(if answering {
        item("emblem-ok-symbolic", "OpenRazer is answering", "But it doesn't list this device")
    } else {
        item("dialog-warning-symbolic", "OpenRazer isn't answering", "Its daemon isn't running for this account")
    });
    items.push(match (in_group, in_session) {
        (_, true) => item("emblem-ok-symbolic", "In the plugdev group", ""),
        (true, false) => item("avatar-default-symbolic", "Sign out and in", "So the new group takes effect"),
        (false, false) => item("dialog-warning-symbolic", "Not in the plugdev group", "Set up adds you; sign out and in afterwards"),
    });
    let title = if usb.is_empty() { "OpenRazer" } else { "A Razer device is plugged in" };
    let mut v = Page::new("").banner(SETUP_BANNER, true).group("Set up", "", vec![with_icon(setup_row(), "applications-engineering-symbolic")]).build();
    v["preview"] = json!({
        "title": title, "line": "DPI, polling rate, battery and lighting appear once OpenRazer answers.",
        "blocks": [{ "kind": "picture", "picture": "mouse" }, { "kind": "list", "items": items }],
    });
    v
}

/// The Razer USB devices plugged in: (name, vendor:product).
fn razer_usb() -> Vec<(String, String)> {
    if mock::on() {
        return mock::usb();
    }
    let Ok(rd) = std::fs::read_dir("/sys/bus/usb/devices") else { return Vec::new() };
    let mut out: Vec<(String, String)> = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if sys::read(p.join("idVendor")).as_deref() != Some("1532") {
            continue;
        }
        let id = format!("1532:{}", sys::read(p.join("idProduct")).unwrap_or_default());
        if !out.iter().any(|(_, i)| *i == id) {
            out.push((sys::read(p.join("product")).unwrap_or_else(|| "Razer device".into()), id));
        }
    }
    out.sort();
    out
}

/// Whether the account is in `plugdev`: (in /etc/group or this session, in this session).
fn plugdev() -> (bool, bool) {
    if mock::on() {
        return mock::plugdev();
    }
    let user = std::env::var("USER").unwrap_or_default();
    let in_file = sys::read("/etc/group").is_some_and(|t| {
        t.lines().any(|l| l.starts_with("plugdev:") && l.rsplit(':').next().is_some_and(|m| m.split(',').any(|u| u == user)))
    });
    let in_session = sys::run("id", &["-nG"]).is_ok_and(|g| g.split_whitespace().any(|x| x == "plugdev"));
    (in_file || in_session, in_session)
}

struct Razer;

impl Extension for Razer {
    fn pages(&self) -> Value {
        if !mock::on() && !sys::present("openrazer-daemon") && !std::path::Path::new("/usr/bin/openrazer-daemon").exists() {
            return json!([]);
        }
        let devs = devices();
        if devs.is_empty() {
            // The daemon may not be running, or the user not in its group yet.
            return if razer_plugged_in() {
                json!([page::entry(SETUP, "Razer", "input-mouse-symbolic", "Set up OpenRazer for your Razer device.", "razer openrazer")])
            } else {
                json!([])
            };
        }
        Value::Array(
            devs.iter()
                .map(|d| page::entry(&d.serial, &d.name, icon(&d.kind), "DPI, polling rate, battery, lighting.", &format!("razer {} dpi polling battery lighting chroma {}", d.kind, d.name.to_lowercase())))
                .collect(),
        )
    }

    fn describe(&self, id: &str) -> Result<Value> {
        if id == SETUP {
            if preview_supported() {
                let answering = call("/org/razer", "razer.devices", "getDevices", &[]).is_ok();
                return Ok(setup_with_preview(&razer_usb(), answering, plugdev()));
            }
            return Ok(Page::new("").banner(SETUP_BANNER, true).group("Set up", "", vec![setup_row()]).build());
        }
        let d = devices().into_iter().find(|d| d.serial == id).context("that device isn't connected any more")?;
        Ok(render(&d, &Readings::read(&d.path), preview_supported()))
    }

    fn set(&self, id: &str, key: &str, value: &str) -> Result<Option<Value>> {
        if key == "setup" && mock::on() {
            return Ok(nyx_ext::reload("Done: sign out and back in to finish"));
        }
        if key == "setup" {
            let user = std::env::var("USER").context("who am I?")?;
            sys::run_root("gpasswd", &["-a", &user, "plugdev"])?;
            let _ = sys::run("systemctl", &["--user", "enable", "--now", "openrazer-daemon.service"]);
            return Ok(nyx_ext::reload("Done: sign out and back in to finish"));
        }
        let p = format!("/org/razer/device/{id}");
        if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            bail!("no such device");
        }
        let colour = || sys::read(sys::state_dir().join(format!("{id}.colour"))).and_then(|c| sys::parse_hex(&c)).unwrap_or((0, 255, 0));
        match key {
            "dpi" => {
                let v = (value.parse::<f64>()?.round() as u32).to_string();
                call(&p, "razer.device.dpi", "setDPI", &["qq", &v, &v])?;
            }
            "poll" => {
                call(&p, "razer.device.misc", "setPollRate", &["q", value])?;
            }
            "idle" => {
                let secs = (value.parse::<f64>()? * 60.0).round() as u32;
                call(&p, "razer.device.power", "setIdleTime", &["q", &secs.to_string()])?;
            }
            "brightness" => {
                call(&p, "razer.device.lighting.brightness", "setBrightness", &["d", value])?;
            }
            "colour" => {
                let (r, g, b) = sys::parse_hex(value).context("not a colour")?;
                std::fs::write(sys::state_dir().join(format!("{id}.colour")), value)?;
                call(&p, "razer.device.lighting.chroma", "setStatic", &["yyy", &r.to_string(), &g.to_string(), &b.to_string()])?;
            }
            "effect" => {
                let (r, g, b) = colour();
                let (r, g, b) = (r.to_string(), g.to_string(), b.to_string());
                let args: Vec<&str> = match value {
                    "setStatic" | "setBreathSingle" => vec!["yyy", &r, &g, &b],
                    "setWave" => vec!["i", "1"],
                    "setReactive" => vec!["yyyy", &r, &g, &b, "2"],
                    "setSpectrum" | "setNone" => vec![],
                    _ => bail!("not an effect this offers"),
                };
                call(&p, "razer.device.lighting.chroma", value, &args)?;
                std::fs::write(sys::state_dir().join(format!("{id}.effect")), value)?;
            }
            _ => bail!("unknown setting {key}"),
        }
        Ok(None)
    }
}

/// Any Razer USB device (vendor 1532) plugged in.
fn razer_plugged_in() -> bool {
    if mock::on() {
        return true;
    }
    std::fs::read_dir("/sys/bus/usb/devices")
        .map(|rd| rd.flatten().any(|e| sys::read(e.path().join("idVendor")).as_deref() == Some("1532")))
        .unwrap_or(false)
}

fn main() {
    nyx_ext::run(&Razer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_busctl_replies() {
        assert_eq!(values("s \"Razer Basilisk V3\""), vec!["Razer Basilisk V3"]);
        assert_eq!(values("aq 2 800 800"), vec!["2", "800", "800"]);
        assert_eq!(values("as 2 \"PM1234\" \"XX9\""), vec!["2", "PM1234", "XX9"]);
        assert_eq!(values("d 87.5"), vec!["87.5"]);
        assert_eq!(values("s \"say \\\"hi\\\"\""), vec!["say \"hi\""]);
        assert!(values("").is_empty());
    }

    fn rows(page: &Value) -> Vec<&Value> {
        page["groups"].as_array().unwrap().iter().flat_map(|g| g["rows"].as_array().unwrap()).collect()
    }

    fn all_have_icons(page: &Value) -> bool {
        let left = rows(page);
        !left.is_empty() && left.iter().all(|r| r["icon"].as_str().is_some_and(|i| !i.is_empty()))
    }

    fn basilisk() -> (Device, Readings) {
        let d = Device { serial: "PM1234".into(), path: "/org/razer/device/PM1234".into(), name: "Razer Basilisk V3".into(), kind: "mouse".into() };
        let r = Readings { dpi: Some((4000.0, 26000.0)), poll: Some("1000".into()), battery: Some((86.0, true)), idle: Some(300.0), lighting: true, brightness: Some(80.0) };
        (d, r)
    }

    #[test]
    fn preview_pane() {
        let (d, r) = basilisk();
        let old = render(&d, &r, false);
        assert!(old.get("preview").is_none());
        assert_eq!(old["groups"][3]["title"], "Device");
        assert!(rows(&old).iter().any(|r| r["key"] == "colour"));

        let new = render(&d, &r, true);
        let pv = &new["preview"];
        assert_eq!(pv["line"], "Through OpenRazer · serial PM1234");
        assert_eq!(pv["blocks"][0]["picture"], "mouse");
        assert_eq!(pv["blocks"][1]["rows"][0]["key"], "colour");
        assert_eq!(pv["blocks"][2]["items"][1], json!({ "label": "Sensitivity", "value": "4,000 dpi", "sub": "Up to 26,000" }));
        assert_eq!(pv["blocks"][2]["items"][0]["tone"], "good");
        assert!(all_have_icons(&new));
        assert!(rows(&new).iter().all(|r| r["key"] != "colour" && r["key"] != "battery"));
    }

    #[test]
    fn setup_pane() {
        let new = setup_with_preview(&[("Razer Basilisk V3".into(), "1532:0099".into())], false, (false, false));
        assert_eq!(new["banners"][0]["warning"], true);
        assert!(all_have_icons(&new));
        let items = new["preview"]["blocks"][1]["items"].as_array().unwrap();
        assert_eq!(items[0]["label"], "Razer Basilisk V3 found");
        assert_eq!(items[1]["label"], "OpenRazer isn't answering");
        assert_eq!(items[2]["label"], "Not in the plugdev group");
    }

    #[test]
    fn groups_digits() {
        assert_eq!(thousands(26000.0), "26,000");
        assert_eq!(thousands(800.0), "800");
        assert_eq!(thousands(1234567.0), "1,234,567");
    }
}
