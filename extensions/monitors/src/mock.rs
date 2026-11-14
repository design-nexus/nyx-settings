//! Pretend monitors, for trying the pages without one.
//!
//! `MONITORS_MOCK=dell-u2720q` answers like `ddcutil` for a DELL U2720Q on bus 4;
//! `MONITORS_MOCK=no-access` is the same monitor before the device rule is installed
//! (the setup page) until Allow is pressed. Its values are kept in
//! `SETTINGS_EXTENSION_STATE`; nothing here opens `/dev/i2c-*` or runs pkexec.

use anyhow::{Result, bail};
use nyx_ext::sys;
use serde_json::{Value, json};

const PROFILES: [&str; 2] = ["dell-u2720q", "no-access"];

const DETECT: &str = "Display 1\n   I2C bus:  /dev/i2c-4\n   DRM connector:  card1-DP-1\n   Monitor:  DEL:DELL U2720Q:CN0F2H3T7RA2\n\n\
                      Invalid display\n   I2C bus:  /dev/i2c-15\n   Monitor:  SDC:ATNA60DL01-0:\n";

const CAPABILITIES: &str = "Model: U2720Q\nMCCS version: 2.1\nCommands:\n   Op Code: 01 (VCP Request)\n   Op Code: 02 (VCP Response)\n\
VCP Features:\n   Feature: 02 (New control value)\n   Feature: 10 (Brightness)\n   Feature: 12 (Contrast)\n\
   Feature: 14 (Select color preset)\n      Values:\n         04: 5000 K\n         05: 6500 K\n         06: 7500 K\n         08: 9300 K\n         0b: User 1\n\
   Feature: 60 (Input Source)\n      Values:\n         0f: DisplayPort-1\n         11: HDMI-1\n         1b: USB-C\n\
   Feature: 62 (Audio speaker volume)\n\
   Feature: D6 (Power mode)\n      Values:\n         01: DPM: On,  DPMS: Off\n         04: DPM: Off, DPMS: Off\n";

/// Continuous features and their maximum.
const CONTINUOUS: [(&str, u32); 3] = [("10", 100), ("12", 100), ("62", 100)];
/// Features with a list of values, as in [`CAPABILITIES`].
const CHOICES: [(&str, &[u32]); 3] = [("14", &[0x04, 0x05, 0x06, 0x08, 0x0b]), ("60", &[0x0f, 0x11, 0x1b]), ("D6", &[0x01, 0x04])];

/// The pretend monitor asked for, if any.
pub fn profile() -> Option<&'static str> {
    let name = std::env::var("MONITORS_MOCK").ok().filter(|s| !s.trim().is_empty())?;
    match PROFILES.iter().find(|p| **p == name.trim()) {
        Some(p) => Some(p),
        None => {
            eprintln!("MONITORS_MOCK: no pretend monitor called {name} (try {})", PROFILES.join(", "));
            std::process::exit(1);
        }
    }
}

pub fn on() -> bool {
    profile().is_some()
}

fn path() -> std::path::PathBuf {
    sys::state_dir().join("mock-monitor.json")
}

fn load() -> Value {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({ "10": 75, "12": 75, "62": 40, "60": 0x1b, "14": 0x05, "D6": 0x01, "allowed": false }))
}

fn save(state: &Value) {
    let _ = std::fs::write(path(), state.to_string());
}

/// Whether the pretend account may open the monitor yet.
pub fn can_reach() -> bool {
    profile() != Some("no-access") || load()["allowed"] == true
}

/// Pressing Allow on the setup page.
pub fn allow() {
    let mut s = load();
    s["allowed"] = json!(true);
    save(&s);
}

/// The built-in screen's brightness, as in the mockup.
pub fn laptop_brightness() -> Option<u32> {
    Some(40)
}

/// External outputs with something plugged in.
pub fn outputs() -> Vec<String> {
    vec!["DP-1".into()]
}

/// `ddcutil ARGS` as the extension calls it.
pub fn ddcutil(args: &[&str]) -> Result<String> {
    if args == ["detect", "--terse"] {
        return Ok(DETECT.into());
    }
    let ["--bus", bus, rest @ ..] = args else { bail!("not something the pretend monitor answers: {}", args.join(" ")) };
    if *bus != "4" {
        bail!("No monitor detected on bus {bus}");
    }
    let rest: Vec<&str> = rest.iter().copied().filter(|a| *a != "--noverify" && *a != "--terse").collect();
    let mut state = load();
    match rest.as_slice() {
        ["capabilities"] => Ok(CAPABILITIES.into()),
        ["getvcp", codes @ ..] => Ok(codes
            .iter()
            .map(|c| {
                let c = c.to_uppercase();
                let v = state[c.as_str()].as_u64().unwrap_or(0);
                match CONTINUOUS.iter().find(|(k, _)| *k == c) {
                    Some((_, max)) => format!("VCP {c} C {v} {max}\n"),
                    None if state.get(c.as_str()).is_some() => format!("VCP {c} SNC x{v:02x}\n"),
                    None => format!("VCP {c} ERR\n"),
                }
            })
            .collect()),
        ["setvcp", code, value] => {
            let code = code.to_uppercase();
            let n = match value.strip_prefix('x') {
                Some(h) => u32::from_str_radix(h, 16)?,
                None => value.parse()?,
            };
            if let Some((_, max)) = CONTINUOUS.iter().find(|(k, _)| *k == code) {
                state[code.as_str()] = json!(n.min(*max));
            } else if let Some((_, ok)) = CHOICES.iter().find(|(k, _)| *k == code) {
                if !ok.contains(&n) {
                    bail!("Invalid value for feature {code}: {value}");
                }
                state[code.as_str()] = json!(n);
            } else {
                bail!("Feature {code} isn't supported");
            }
            save(&state);
            Ok(String::new())
        }
        _ => bail!("not something the pretend monitor answers: {}", args.join(" ")),
    }
}
