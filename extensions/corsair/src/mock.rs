//! OpenRGB while `CORSAIR_MOCK` is set: the simulated Galleon 100 SD's lighting
//! and a pair of Vengeance RGB sticks, for tests and screenshots. Nothing talks
//! to OpenRGB then; changes only update the pretend state, kept in
//! `SETTINGS_EXTENSION_STATE`.

use crate::usb;
use anyhow::{Context, Result, bail};
use nyx_ext::openrgb::{
    COLOR_MODE_NONE, COLOR_MODE_PER_LED, COLOR_MODE_SPECIFIC, Device, HAS_BRIGHTNESS, HAS_MODE_SPECIFIC_COLOR, HAS_PER_LED_COLOR,
    HAS_SPEED, Mode, Rgb, Zone,
};
use nyx_ext::{lighting, sys};
use serde_json::{Value, json};

pub fn on() -> bool {
    usb::mock_spec().is_some()
}

struct Spec {
    kind: i32,
    name: &'static str,
    location: &'static str,
    modes: &'static [&'static str],
    mode: usize,
    brightness: u32,
    speed: u32,
    colour: &'static str,
    follow: bool,
    zones: &'static [(&'static str, u32)],
}

const SPECS: &[Spec] = &[
    Spec {
        kind: 5,
        name: "Corsair Galleon 100 SD",
        location: "HID: /dev/hidraw91",
        modes: &["Direct", "Static", "Breathing", "Rainbow wave", "Off"],
        mode: 1,
        brightness: 80,
        speed: 50,
        colour: "#f2c14e",
        follow: true,
        zones: &[("Keyboard", 99)],
    },
    Spec {
        kind: 1,
        name: "Corsair Vengeance RGB",
        location: "I2C: /dev/i2c-1, address 0x58",
        modes: &["Direct", "Static", "Breathing", "Rainbow wave"],
        mode: 3,
        brightness: 60,
        speed: 50,
        colour: "#8b5cf6",
        follow: false,
        zones: &[("Stick 1", 10), ("Stick 2", 10)],
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
    sys::state_dir().join("corsair-mock.json")
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
        vendor: "Corsair".into(),
        description: "Pretend device (CORSAIR_MOCK)".into(),
        location: s.location.into(),
        active_mode: active as i32,
        modes: s.modes.iter().map(|m| mode(m, brightness, speed, colour)).collect(),
        zones: s.zones.iter().map(|(z, n)| Zone { name: z.to_string(), kind: 1, leds_count: *n }).collect(),
        colors: vec![colour; leds.len().max(1)],
        leds,
        ..Default::default()
    }
}

/// The pretend OpenRGB devices: none unless the Galleon is simulated.
pub fn devices() -> Vec<Device> {
    if !usb::mock_spec().is_some_and(|s| s.split(',').any(|t| t.trim() == "galleon-100-sd")) {
        return Vec::new();
    }
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

/// What `lighting::set` does to a real device, done to the pretend state.
pub fn set(device: &str, key: &str, value: &str) -> Result<Option<Value>> {
    let mut state = load();
    let all = devices();
    let d = all.iter().find(|d| d.id() == device).context("OpenRGB doesn't see that device any more")?;
    let entry = &mut state["devices"][d.id()];
    let reply = match key {
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
            entry[&key["light/".len()..]] = json!(v.round() as u32);
            None
        }
        "light/theme" => {
            entry["follow"] = json!(value == "true");
            nyx_ext::refresh()
        }
        "light/keep" => nyx_ext::toast("Saved on the device"),
        _ => bail!("unknown setting {key}"),
    };
    std::fs::write(state_file(), state.to_string())?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mockups_lights() {
        let d: Vec<Device> = SPECS.iter().enumerate().map(|(i, s)| build(i as u32, s, &Value::Null, None)).collect();
        assert_eq!(d[0].mode().unwrap().colors, vec![Rgb(0xf2, 0xc1, 0x4e)]);
        assert_eq!(d[1].kind_name(), "Memory");
        assert_eq!(d[1].mode().unwrap().name, "Rainbow wave");
        assert_eq!(d[1].mode().unwrap().brightness, 60);
        assert_eq!(d[1].zones.iter().map(|z| z.leds_count).collect::<Vec<_>>(), [10, 10]);
        let saved = json!({ "mode": 1, "colour": "#123456", "follow": false });
        assert_eq!(build(0, &SPECS[0], &saved, Some(Rgb(1, 2, 3))).colors[0], Rgb(0x12, 0x34, 0x56));
        assert_eq!(build(0, &SPECS[0], &Value::Null, Some(Rgb(1, 2, 3))).colors[0], Rgb(1, 2, 3));
    }
}
