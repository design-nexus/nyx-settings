//! `RGB_MOCK=1`: a pretend OpenRGB with five devices (a motherboard, memory, a
//! graphics card, fans and a mouse), for tests and screenshots. While it's set
//! nothing talks to OpenRGB; changes only update the pretend state, kept in
//! `SETTINGS_EXTENSION_STATE`.

use anyhow::{Context, Result, bail};
use nyx_ext::openrgb::{
    COLOR_MODE_NONE, COLOR_MODE_PER_LED, COLOR_MODE_SPECIFIC, Device, HAS_BRIGHTNESS, HAS_MODE_SPECIFIC_COLOR, HAS_PER_LED_COLOR,
    HAS_SPEED, Mode, Rgb, Zone,
};
use nyx_ext::{lighting, sys};
use serde_json::{Value, json};

pub fn on() -> bool {
    std::env::var("RGB_MOCK").is_ok_and(|v| !v.trim().is_empty())
}

struct Spec {
    kind: i32,
    name: &'static str,
    vendor: &'static str,
    version: &'static str,
    location: &'static str,
    modes: &'static [&'static str],
    mode: usize,
    brightness: u32,
    speed: u32,
    colour: &'static str,
    follow: bool,
    zones: &'static [(&'static str, u32)],
}

const AURA: &[&str] = &["Direct", "Static", "Breathing", "Rainbow wave", "Off"];
const BASIC: &[&str] = &["Direct", "Static", "Breathing", "Rainbow wave"];

const SPECS: &[Spec] = &[
    Spec {
        kind: 0,
        name: "ROG STRIX Z790-E",
        vendor: "ASUS",
        version: "AULA3-AR32-0207",
        location: "HID: /dev/hidraw4",
        modes: AURA,
        mode: 2,
        brightness: 70,
        speed: 25,
        colour: "#ef4444",
        follow: false,
        zones: &[("Aura Mainboard", 8), ("Aura Addressable 1", 36), ("Aura Addressable 2", 0), ("Aura Addressable 3", 12)],
    },
    Spec {
        kind: 1,
        name: "Vengeance RGB × 2",
        vendor: "Corsair",
        version: "",
        location: "I2C: /dev/i2c-1, address 0x58",
        modes: BASIC,
        mode: 1,
        brightness: 80,
        speed: 50,
        colour: "#8b5cf6",
        follow: false,
        zones: &[("Stick 1", 10), ("Stick 2", 10)],
    },
    Spec {
        kind: 2,
        name: "RTX 4080 SUPER",
        vendor: "NVIDIA",
        version: "",
        location: "I2C: /dev/i2c-3, address 0x49",
        modes: BASIC,
        mode: 1,
        brightness: 100,
        speed: 50,
        colour: "#22c55e",
        follow: false,
        zones: &[("Logo", 1)],
    },
    Spec {
        kind: 3,
        name: "LL120 fans × 3",
        vendor: "Corsair",
        version: "",
        location: "HID: /dev/hidraw7",
        modes: BASIC,
        mode: 1,
        brightness: 100,
        speed: 50,
        colour: "#3b82f6",
        follow: false,
        zones: &[("Fan 1", 16), ("Fan 2", 16), ("Fan 3", 16)],
    },
    Spec {
        kind: 6,
        name: "Basilisk V3",
        vendor: "Razer",
        version: "",
        location: "HID: /dev/hidraw9",
        modes: &["Direct", "Static", "Breathing", "Spectrum cycle", "Off"],
        mode: 1,
        brightness: 100,
        speed: 50,
        colour: "#14b8a6",
        follow: false,
        zones: &[("Logo", 1), ("Scroll wheel", 1), ("Underglow", 11)],
    },
];

fn mode(name: &str, brightness: u32, speed: u32, colour: Rgb) -> Mode {
    let (flags, color_mode) = match name {
        "Direct" => (HAS_PER_LED_COLOR, COLOR_MODE_PER_LED),
        "Static" => (HAS_MODE_SPECIFIC_COLOR | HAS_BRIGHTNESS, COLOR_MODE_SPECIFIC),
        "Breathing" => (HAS_MODE_SPECIFIC_COLOR | HAS_BRIGHTNESS | HAS_SPEED, COLOR_MODE_SPECIFIC),
        "Off" => (0, COLOR_MODE_NONE),
        _ => (HAS_BRIGHTNESS | HAS_SPEED, COLOR_MODE_NONE),
    };
    let specific = color_mode == COLOR_MODE_SPECIFIC;
    Mode {
        name: name.into(),
        flags,
        speed_max: 100,
        brightness_max: 100,
        colors_min: specific as u32,
        colors_max: specific as u32,
        speed,
        brightness,
        color_mode,
        colors: if specific { vec![colour] } else { vec![] },
        ..Default::default()
    }
}

fn state_file() -> std::path::PathBuf {
    sys::state_dir().join("rgb-mock.json")
}

fn load() -> Value {
    std::fs::read_to_string(state_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({}))
}

fn build(index: u32, s: &Spec, saved: &Value, theme: Option<Rgb>) -> Device {
    let num = |k: &str, default: u32| saved[k].as_u64().map(|v| v as u32).unwrap_or(default);
    let mut colour = saved["colour"].as_str().and_then(Rgb::parse).or_else(|| Rgb::parse(s.colour)).unwrap_or_default();
    if saved["follow"].as_bool().unwrap_or(s.follow)
        && let Some(t) = theme
    {
        colour = t;
    }
    let active = (num("mode", s.mode as u32) as usize).min(s.modes.len() - 1);
    let (brightness, speed) = (num("brightness", s.brightness), num("speed", s.speed));
    let leds: Vec<String> = s.zones.iter().flat_map(|(z, n)| (1..=*n).map(move |i| format!("{z} LED {i}"))).collect();
    Device {
        index,
        kind: s.kind,
        name: s.name.into(),
        vendor: s.vendor.into(),
        description: "Pretend device (RGB_MOCK)".into(),
        version: s.version.into(),
        location: s.location.into(),
        active_mode: active as i32,
        modes: s.modes.iter().map(|m| mode(m, brightness, speed, colour)).collect(),
        zones: s.zones.iter().map(|(z, n)| Zone { name: z.to_string(), kind: 1, leds_count: *n }).collect(),
        colors: vec![colour; leds.len().max(1)],
        leds,
        ..Default::default()
    }
}

pub fn devices() -> Vec<Device> {
    let state = load();
    let theme = sys::theme_accent().and_then(|t| Rgb::parse(&t));
    SPECS
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let d = build(i as u32, s, &Value::Null, None);
            build(i as u32, s, &state["devices"][d.id()], theme)
        })
        .collect()
}

fn following(id: &str) -> bool {
    let state = load();
    SPECS
        .iter()
        .enumerate()
        .find(|(i, s)| build(*i as u32, s, &Value::Null, None).id() == id)
        .is_some_and(|(_, s)| state["devices"][id]["follow"].as_bool().unwrap_or(s.follow))
}

/// The shared lighting rows, with "follow the theme" from the pretend state.
pub fn rows(d: &Device) -> Vec<Value> {
    let mut rows = lighting::rows(d);
    for r in rows.iter_mut().filter(|r| r["key"] == "light/theme") {
        r["value"] = json!(following(&d.id()));
    }
    rows
}

pub fn profiles() -> Vec<String> {
    let mut out = vec!["Evening".to_string(), "Gaming".to_string()];
    out.extend(load()["profiles"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(str::to_string)));
    out
}

/// A change to the pretend state: what `lighting::set` and the `all/` and
/// `profile/` keys do to real devices.
pub fn set(device: &str, key: &str, value: &str) -> Result<Option<Value>> {
    let mut state = load();
    let all = devices();
    let reply = match key {
        k if k.starts_with("light/") => {
            let d = all.iter().find(|d| d.id() == device).context("OpenRGB doesn't see that device any more")?;
            let entry = &mut state["devices"][d.id()];
            match k {
                "light/effect" => {
                    let i: usize = value.parse()?;
                    d.modes.get(i).context("no such effect")?;
                    entry["mode"] = json!(i);
                    nyx_ext::refresh()
                }
                "light/colour" => {
                    entry["colour"] = json!(Rgb::parse(value).context("not a colour")?.hex());
                    entry["follow"] = json!(false);
                    None
                }
                "light/brightness" | "light/speed" => {
                    let v: f64 = value.parse()?;
                    entry[&k["light/".len()..]] = json!(v.round() as u32);
                    None
                }
                "light/theme" => {
                    entry["follow"] = json!(value == "true");
                    nyx_ext::refresh()
                }
                "light/keep" => nyx_ext::toast("Saved on the device"),
                _ => bail!("unknown setting {key}"),
            }
        }
        "all/colour" | "all/theme" | "all/off" => {
            let colour = match key {
                "all/colour" => Rgb::parse(value).context("not a colour")?,
                "all/theme" => sys::theme_accent().and_then(|t| Rgb::parse(&t)).context("the theme has no accent colour")?,
                _ => Rgb(0, 0, 0),
            };
            for d in &all {
                let entry = &mut state["devices"][d.id()];
                entry["colour"] = json!(colour.hex());
                entry["follow"] = json!(false);
                if let Some(i) = d.modes.iter().position(|m| m.name == "Static") {
                    entry["mode"] = json!(i);
                }
            }
            nyx_ext::toast(&format!("{} devices set", all.len()))
        }
        "profile/save" => {
            let name = value.trim();
            if name.is_empty() || name.contains('/') {
                bail!("Give the profile a name");
            }
            if !state["profiles"].is_array() {
                state["profiles"] = json!([]);
            }
            state["profiles"].as_array_mut().unwrap().push(json!(name));
            nyx_ext::refresh()
        }
        "profile/load" => nyx_ext::toast(&format!("Loaded {value}")),
        _ => bail!("unknown setting {key}"),
    };
    std::fs::write(state_file(), state.to_string())?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mockups_devices() {
        let d: Vec<Device> = SPECS.iter().enumerate().map(|(i, s)| build(i as u32, s, &Value::Null, None)).collect();
        assert_eq!(d.len(), 5);
        assert_eq!(d[0].mode().unwrap().name, "Breathing");
        assert_eq!(d[0].mode().unwrap().colors, vec![Rgb(0xef, 0x44, 0x44)]);
        assert_eq!(d[0].zones.iter().map(|z| z.leds_count).collect::<Vec<_>>(), [8, 36, 0, 12]);
        assert_eq!(d[0].leds.len(), 56);
        assert_eq!(d[4].kind_name(), "Mouse");
        let saved = json!({ "mode": 3, "colour": "#123456", "brightness": 10 });
        let changed = build(0, &SPECS[0], &saved, None);
        assert_eq!(
            (changed.mode().unwrap().name.as_str(), changed.colors[0], changed.mode().unwrap().brightness),
            ("Rainbow wave", Rgb(0x12, 0x34, 0x56), 10)
        );
        let followed = build(0, &SPECS[0], &json!({ "follow": true }), Some(Rgb(1, 2, 3)));
        assert_eq!(followed.colors[0], Rgb(1, 2, 3));
    }
}
