//! Laptops of any brand, through the kernel's standard controls: battery
//! health and charge limits, power profile, keyboard backlight, and switches
//! for the microphone and camera. ThinkPad, Dell, Framework, HP, LG, Lenovo,
//! Huawei, Samsung and others expose these the same way.
//!
//! On an ASUS with asusctl the ASUS extension does all this, so this one
//! stays out of the way.

mod mock;

use anyhow::{Context, Result, bail};
use nyx_ext::page::{self, Page, desc, refreshes};
use nyx_ext::{Extension, sys};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const PAGE: &str = "laptop";
const RULE: &str = "/etc/udev/rules.d/46-nyx-battery.rules";

/// A system path, or its stand-in on the pretend laptop.
fn at(path: &str) -> PathBuf {
    if mock::on() { mock::path(path) } else { PathBuf::from(path) }
}

fn run(program: &str, args: &[&str]) -> Result<String> {
    if mock::on() { mock::run(program, args) } else { sys::run(program, args) }
}

fn run_root(program: &str, args: &[&str]) -> Result<String> {
    if mock::on() { mock::run(program, args) } else { sys::run_root(program, args) }
}

/// Whether this Settings draws a preview pane beside the groups.
fn preview_supported() -> bool {
    std::env::var("SETTINGS_KINDS").is_ok_and(|k| k.split(' ').any(|x| x == "preview"))
}

fn battery() -> Option<PathBuf> {
    let rd = std::fs::read_dir(at("/sys/class/power_supply")).ok()?;
    let mut bats: Vec<PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| sys::read(p.join("type")).as_deref() == Some("Battery") && p.join("capacity").exists()).collect();
    bats.sort();
    // The laptop's own battery, not a mouse's or a controller's.
    bats.into_iter().find(|p| sys::read(p.join("scope")).as_deref() != Some("Device"))
}

fn kbd_backlight() -> Option<PathBuf> {
    std::fs::read_dir(at("/sys/class/leds")).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().contains("kbd_backlight")))
}

/// An ASUS that the ASUS extension looks after.
fn asus_handled() -> bool {
    if std::env::var_os("NYX_LAPTOP_SHOW").is_some() {
        return false;
    }
    sys::read(at("/sys/class/dmi/id/sys_vendor")).is_some_and(|v| v.to_uppercase().contains("ASUS")) && sys::present("asusctl")
}

fn laptop() -> bool {
    let chassis = sys::read_num(at("/sys/class/dmi/id/chassis_type")).unwrap_or(0);
    // 8 portable, 9 laptop, 10 notebook, 14 sub-notebook, 31 convertible, 32 detachable.
    battery().is_some() && ([8, 9, 10, 14, 31, 32].contains(&chassis) || chassis == 0)
}

/// The rule that sets the charge limits at every start (the firmware forgets
/// them on many laptops), from the values now set.
fn rule_text(name: &str, start: Option<i64>, end: Option<i64>) -> String {
    let mut attrs = String::new();
    if let Some(s) = start {
        attrs.push_str(&format!(", ATTR{{charge_control_start_threshold}}=\"{s}\""));
    }
    if let Some(e) = end {
        attrs.push_str(&format!(", ATTR{{charge_control_end_threshold}}=\"{e}\""));
    }
    format!("# Written by Settings: battery charge limits, set again at every start.\nACTION==\"add\", SUBSYSTEM==\"power_supply\", KERNEL==\"{name}\"{attrs}\n")
}

/// Write the limits now and keep them, as root (asks for a password).
fn set_limits(bat: &Path, start: Option<i64>, end: Option<i64>) -> Result<()> {
    if mock::on() {
        return mock::set_limits(bat, start, end);
    }
    let name = bat.file_name().context("no battery")?.to_string_lossy().to_string();
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let tmp = dir.join("46-nyx-battery.rules");
    std::fs::write(&tmp, rule_text(&name, start, end))?;
    // Lowering the stop limit below the start limit is refused: change start first.
    let mut script = String::new();
    for (file, v) in [("charge_control_start_threshold", start), ("charge_control_end_threshold", end)] {
        if let Some(v) = v {
            script.push_str(&format!("echo {v} > '{}' 2>/dev/null; ", bat.join(file).display()));
        }
    }
    for (file, v) in [("charge_control_end_threshold", end), ("charge_control_start_threshold", start)] {
        if let Some(v) = v {
            script.push_str(&format!("echo {v} > '{}'; ", bat.join(file).display()));
        }
    }
    script.push_str(&format!("install -Dm644 '{}' '{RULE}'", tmp.display()));
    let r = sys::run("pkexec", &["sh", "-c", &script]);
    let _ = std::fs::remove_file(&tmp);
    r.map(|_| ())
}

fn profiles() -> (String, Vec<String>) {
    parse_profiles(&run("powerprofilesctl", &["list"]).unwrap_or_default())
}

/// The current profile and all of them, slowest first. Profile names are the
/// unindented (or `* `-marked) lines ending in a colon; their details are indented.
fn parse_profiles(text: &str) -> (String, Vec<String>) {
    let mut current = String::new();
    let mut all = Vec::new();
    for line in text.lines() {
        let Some(name) = line.trim().trim_start_matches('*').trim().strip_suffix(':') else { continue };
        if line.starts_with("   ") || name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            continue;
        }
        if line.trim_start().starts_with('*') {
            current = name.to_string();
        }
        all.push(name.to_string());
    }
    all.reverse();
    (current, all)
}

fn profile_label(p: &str) -> &str {
    match p {
        "power-saver" => "Battery saver",
        "balanced" => "Balanced",
        "performance" => "Performance",
        other => other,
    }
}

/// The shorter labels the pane layout's narrower column has room for.
fn profile_label_short(p: &str) -> &str {
    if p == "power-saver" { "Saver" } else { profile_label(p) }
}

fn mic_muted() -> Option<bool> {
    run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SOURCE@"]).ok().map(|t| t.contains("[MUTED]"))
}

fn camera_on() -> bool {
    sys::read(at("/proc/modules")).is_some_and(|m| m.lines().any(|l| l.starts_with("uvcvideo ")))
}

/// A row with an icon, for the pane layout's left column.
fn icon(mut row: Value, name: &str) -> Value {
    row["icon"] = json!(name);
    row
}

/// What the battery reports.
struct Charge {
    percent: i64,
    status: String,
    health: Option<i64>,
    cycles: Option<i64>,
    start: Option<i64>,
    end: Option<i64>,
}

fn charge(bat: &Path) -> Charge {
    let full = sys::read_num(bat.join("charge_full")).or_else(|| sys::read_num(bat.join("energy_full")));
    let design = sys::read_num(bat.join("charge_full_design")).or_else(|| sys::read_num(bat.join("energy_full_design")));
    Charge {
        percent: sys::read_num(bat.join("capacity")).unwrap_or(0),
        status: sys::read(bat.join("status")).unwrap_or_default(),
        health: full.zip(design).filter(|(_, d)| *d > 0).map(|(f, d)| f * 100 / d),
        cycles: sys::read_num(bat.join("cycle_count")).filter(|c| *c > 0),
        start: sys::read_num(bat.join("charge_control_start_threshold")),
        end: sys::read_num(bat.join("charge_control_end_threshold")),
    }
}

/// The line under the charge: what it's doing, and where it'll stop.
fn charge_line(c: &Charge) -> String {
    match (c.status.as_str(), c.end) {
        ("Charging", Some(e)) if e < 100 => format!("Charging to {e}%"),
        ("Charging", _) => "Charging".into(),
        ("Discharging", _) => "On battery".into(),
        ("Full" | "Not charging", _) => "Plugged in".into(),
        (other, _) => other.into(),
    }
}

/// The laptop and its battery, and the charge window the limits make.
fn preview(c: &Charge, product: &str, vendor: &str) -> Value {
    let tone = if c.percent <= 15 && c.status == "Discharging" { "bad" } else { "good" };
    let mut items = vec![json!({ "label": "Charge", "value": format!("{}%", c.percent), "sub": charge_line(c), "tone": tone })];
    if let Some(h) = c.health {
        items.push(json!({ "label": "Health", "value": format!("{h}%"), "sub": "Of its original capacity", "tone": if h < 80 { "warn" } else { "" } }));
    }
    if let Some(n) = c.cycles {
        items.push(json!({ "label": "Cycles", "value": n.to_string(), "sub": "Charge cycles" }));
    }
    let line = if vendor.is_empty() { "Battery and power".to_string() } else { format!("{vendor} · battery and power") };
    let mut blocks = vec![json!({
        "kind": "device", "picture": "laptop", "title": if product.is_empty() { "Laptop" } else { product }, "line": line, "items": items,
    })];
    let mut window = Vec::new();
    if let Some(s) = c.start {
        window.push(json!({ "label": "Starts again below", "value": format!("{s}%") }));
    }
    if let Some(e) = c.end {
        window.push(json!({ "label": "Stops at", "value": format!("{e}%"), "sub": if e == 100 { "No limit" } else { "" } }));
    }
    if !window.is_empty() {
        blocks.push(json!({ "kind": "stats", "items": window }));
    }
    let mut pane = json!({ "blocks": blocks });
    if c.health.is_some() {
        pane["foot"] = json!("Batteries hold less as they age; below about 80% health you'll notice shorter runs.");
        pane["foot_icon"] = json!("dialog-information-symbolic");
    }
    pane
}

struct Laptop;

impl Extension for Laptop {
    fn pages(&self) -> Value {
        if !laptop() || asus_handled() {
            return json!([]);
        }
        json!([page::entry(PAGE, "Laptop", "computer-laptop-symbolic", "Battery health and charge limit, power profile, keyboard light, mic and camera.",
            "laptop battery charge limit threshold health cycles power profile keyboard backlight camera microphone")])
    }

    fn describe(&self, _id: &str) -> Result<Value> {
        let bat = battery().context("there's no battery")?;
        let pane = preview_supported();
        let c = charge(&bat);
        let mut battery_rows = Vec::new();
        battery_rows.push(page::info("charge", "Charge", &format!("{}% · {}", c.percent, c.status)));
        if let Some(h) = c.health {
            battery_rows.push(desc(
                page::info("health", "Health", &format!("{h}% of its original capacity")),
                "Batteries hold less as they age; below about 80% you'll notice shorter runs.",
            ));
        }
        if let Some(n) = c.cycles {
            battery_rows.push(page::info("cycles", "Charge cycles", &n.to_string()));
        }
        let mut limit_rows = Vec::new();
        let mut full = None;
        if let Some(end) = c.end {
            let row = page::slider("end", "Stop charging at", end as f64, (50.0, 100.0, 5.0), " %");
            limit_rows.push(if pane {
                icon(desc(row, "80% is a good everyday limit"), "battery-level-80-symbolic")
            } else {
                desc(row, "Keeping it below full makes the battery last more years. 80% is a good everyday limit.")
            });
            if end < 100 {
                let row = page::button("full", "Charge to 100% once", "Charge fully");
                full = Some(if pane { icon(desc(row, "Until you set a limit again"), "battery-full-charging-symbolic") } else { row });
            }
        }
        // The pane layout puts the one-off charge last, after both limits.
        if !pane {
            limit_rows.extend(full.take());
        }
        if let Some(start) = c.start {
            let row = page::slider("start", "Start charging below", start as f64, (20.0, 95.0, 5.0), " %");
            limit_rows.push(if pane {
                icon(desc(row, "Plugged in, it waits till then"), "ac-adapter-symbolic")
            } else {
                desc(row, "Plugged in, it waits until the charge drops this low before charging again.")
            });
        }
        limit_rows.extend(full);
        let (current, all) = profiles();
        let mut power = Vec::new();
        if !all.is_empty() {
            let label = if pane { profile_label_short } else { profile_label };
            let opts: Vec<(String, String)> = all.iter().map(|p| (p.clone(), label(p).to_string())).collect();
            let mut row = page::choice_owned("profile", "Power profile", &current, &opts);
            row["kind"] = json!("segmented");
            power.push(if pane {
                icon(row, "power-profile-balanced-symbolic")
            } else {
                desc(row, "Battery saver runs cooler and longer; Performance is faster and louder.")
            });
        }
        if let Some(led) = kbd_backlight() {
            let max = sys::read_num(led.join("max_brightness")).unwrap_or(0);
            if max > 0 {
                let row = page::slider("kbd", "Keyboard light", sys::read_num(led.join("brightness")).unwrap_or(0) as f64, (0.0, max as f64, 1.0), "");
                power.push(if pane { icon(row, "keyboard-brightness-symbolic") } else { row });
            }
        }
        let mut privacy = Vec::new();
        if let Some(muted) = mic_muted() {
            let row = page::switch("mic", "Microphone", !muted);
            privacy.push(if pane {
                icon(desc(row, "Mutes the default microphone for every app"), "audio-input-microphone-symbolic")
            } else {
                desc(row, "Off mutes the default microphone for every app.")
            });
        }
        if at("/sys/module/uvcvideo").exists() || camera_on() || has_camera() {
            let row = page::switch("camera", "Camera", camera_on());
            privacy.push(refreshes(if pane {
                icon(desc(row, "Off unloads its driver, so no app can use it"), "camera-web-symbolic")
            } else {
                desc(row, "Off unloads the camera's driver, so no app can use it until it's back on. Asks for your password.")
            }));
        }
        let vendor = sys::read(at("/sys/class/dmi/id/sys_vendor")).unwrap_or_default();
        let product = sys::read(at("/sys/class/dmi/id/product_name")).unwrap_or_default();
        if pane {
            let note = |key: &str, rows: &[Value], text: &'static str| if rows.iter().any(|r| r["key"] == key) { text } else { "" };
            let power_note = note("profile", &power, "Saver runs cooler and longer; Performance is faster and louder.");
            let privacy_note = note("camera", &privacy, "Turning the camera on or off asks for your password.");
            let mut page = Page::new("")
                .poll(30)
                .group("Charge limit", "Keeping it below full makes the battery last more years.", limit_rows)
                .group("Power", power_note, power)
                .group("Privacy", privacy_note, privacy)
                .build();
            page["preview"] = preview(&c, &product, &vendor);
            return Ok(page);
        }
        Ok(Page::new(&format!("<b>{}</b> · {}", product.replace('&', "&amp;"), vendor.replace('&', "&amp;")))
            .poll(30)
            .group("Battery", "", battery_rows)
            .group("Charge limit", "", limit_rows)
            .group("Power", "", power)
            .group("Privacy", "", privacy)
            .build())
    }

    fn set(&self, _page: &str, key: &str, value: &str) -> Result<Option<Value>> {
        let bat = battery().context("there's no battery")?;
        let read = |f: &str| sys::read_num(bat.join(f));
        match key {
            "end" | "start" | "full" => {
                let mut start = read("charge_control_start_threshold");
                let mut end = read("charge_control_end_threshold");
                let v = if key == "full" { 100 } else { value.parse::<f64>()?.round() as i64 };
                if key == "start" {
                    start = Some(v.clamp(0, 99));
                } else {
                    end = Some(v.clamp(50, 100));
                }
                // Starting has to be below stopping.
                if let (Some(s), Some(e)) = (start, end)
                    && s >= e
                {
                    start = Some((e - 5).max(0));
                }
                set_limits(&bat, start, end)?;
                Ok(if key == "full" { nyx_ext::toast("Charging to 100% until you set a limit again") } else { nyx_ext::refresh() })
            }
            "profile" => {
                run("powerprofilesctl", &["set", value])?;
                Ok(None)
            }
            "kbd" => {
                let led = kbd_backlight().context("no keyboard light")?;
                let name = led.file_name().context("no keyboard light")?.to_string_lossy().to_string();
                let v = value.parse::<f64>()?.round() as u32;
                run(
                    "busctl",
                    &["call", "org.freedesktop.login1", "/org/freedesktop/login1/session/auto", "org.freedesktop.login1.Session", "SetBrightness", "ssu", "leds", &name, &v.to_string()],
                )?;
                Ok(None)
            }
            "mic" => {
                run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SOURCE@", if value == "true" { "0" } else { "1" }])?;
                Ok(None)
            }
            "camera" => {
                let on = value == "true";
                run_root("modprobe", if on { &["uvcvideo"][..] } else { &["-r", "uvcvideo"][..] })
                    .map_err(|e| if on { e } else { e.context("an app is probably using the camera; close it first") })?;
                Ok(nyx_ext::refresh())
            }
            _ => bail!("unknown setting {key}"),
        }
    }
}

fn has_camera() -> bool {
    std::fs::read_dir(at("/sys/bus/usb/devices"))
        .map(|rd| rd.flatten().any(|e| sys::read(e.path().join("bInterfaceClass")).as_deref() == Some("0e")))
        .unwrap_or(false)
}

fn main() {
    nyx_ext::run(&Laptop);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_rule() {
        let r = rule_text("BAT0", Some(75), Some(80));
        assert!(r.contains("KERNEL==\"BAT0\", ATTR{charge_control_start_threshold}=\"75\", ATTR{charge_control_end_threshold}=\"80\""));
        let r = rule_text("BAT1", None, Some(80));
        assert!(!r.contains("start_threshold") && r.contains("end_threshold}=\"80\""));
    }

    #[test]
    fn profile_names() {
        assert_eq!(profile_label("power-saver"), "Battery saver");
        assert_eq!(profile_label("custom"), "custom");
        assert_eq!(profile_label_short("power-saver"), "Saver");
    }

    #[test]
    fn reads_profiles() {
        let text = "* performance:\n    CpuDriver:\tintel_pstate\n    Degraded:   no\n\n  balanced:\n    CpuDriver:\tintel_pstate\n\n  power-saver:\n    CpuDriver:\tintel_pstate\n";
        let (current, all) = parse_profiles(text);
        assert_eq!(current, "performance");
        assert_eq!(all, vec!["power-saver", "balanced", "performance"]);
    }

    /// The pretend laptop, with and without the pane layout. Environment variables
    /// are the whole process's, so this is one test.
    #[test]
    fn pretend_laptop_pages() {
        let state = std::env::temp_dir().join(format!("laptop-mock-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        unsafe {
            std::env::set_var("SETTINGS_EXTENSION_STATE", &state);
            std::env::set_var("LAPTOP_MOCK", "framework-13");
            std::env::remove_var("SETTINGS_KINDS");
        }
        assert_eq!(Laptop.pages()[0]["id"], "laptop");

        let old = Laptop.describe(PAGE).unwrap();
        assert!(old.get("preview").is_none());
        assert_eq!(old["subtitle"], "<b>Framework Laptop 13</b> · Framework");
        let titles: Vec<&str> = old["groups"].as_array().unwrap().iter().map(|g| g["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Battery", "Charge limit", "Power", "Privacy"]);
        assert!(old["groups"].as_array().unwrap().iter().flat_map(|g| g["rows"].as_array().unwrap()).all(|r| r.get("icon").is_none()));

        unsafe { std::env::set_var("SETTINGS_KINDS", "info switch slider segmented button preview icon") };
        let new = Laptop.describe(PAGE).unwrap();
        let groups = new["groups"].as_array().unwrap();
        let titles: Vec<&str> = groups.iter().map(|g| g["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Charge limit", "Power", "Privacy"]);
        for row in groups.iter().flat_map(|g| g["rows"].as_array().unwrap()) {
            assert!(row["icon"].as_str().is_some_and(|i| i.ends_with("-symbolic")), "{row}");
        }
        let keys: Vec<&str> = groups[0]["rows"].as_array().unwrap().iter().map(|r| r["key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["end", "start", "full"]);
        let pv = &new["preview"];
        let device = &pv["blocks"][0];
        assert_eq!((device["kind"].as_str(), device["picture"].as_str()), (Some("device"), Some("laptop")));
        assert_eq!(device["title"], "Framework Laptop 13");
        assert_eq!(device["items"][0], json!({ "label": "Charge", "value": "78%", "sub": "Charging to 80%", "tone": "good" }));
        assert_eq!(device["items"][1]["value"], "92%");
        assert_eq!(device["items"][2]["value"], "214");
        assert_eq!(pv["blocks"][1]["items"][0]["value"], "75%");
        assert_eq!(pv["blocks"][1]["items"][1]["value"], "80%");

        // Changes land on the pretend machine only.
        assert!(mock::on());
        Laptop.set(PAGE, "end", "90").unwrap();
        Laptop.set(PAGE, "profile", "power-saver").unwrap();
        Laptop.set(PAGE, "kbd", "3").unwrap();
        Laptop.set(PAGE, "mic", "false").unwrap();
        Laptop.set(PAGE, "camera", "false").unwrap();
        let after = Laptop.describe(PAGE).unwrap();
        let row = |key: &str| after["groups"].as_array().unwrap().iter().flat_map(|g| g["rows"].as_array().unwrap()).find(|r| r["key"] == key).cloned().unwrap();
        assert_eq!(row("end")["value"], 90.0);
        assert_eq!(row("profile")["value"], "power-saver");
        assert_eq!(row("kbd")["value"], 3.0);
        assert_eq!(row("mic")["value"], false);
        assert_eq!(row("camera")["value"], false);
        assert_eq!(after["preview"]["blocks"][0]["items"][0]["sub"], "Charging to 90%");
        assert!(state.join("laptop-mock/sys/class/power_supply/BAT1/charge_control_end_threshold").exists());
        let _ = std::fs::remove_dir_all(&state);
    }
}
