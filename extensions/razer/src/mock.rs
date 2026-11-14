//! A pretend OpenRazer daemon for testing pages without a Razer device:
//! `RAZER_MOCK=basilisk-v3` answers the daemon's calls the way busctl prints them,
//! `RAZER_MOCK=no-daemon` has a Basilisk plugged in but the daemon not answering
//! (the setup page). Changes are kept in the state folder; nothing calls busctl,
//! pkexec or systemctl while it's on.

use anyhow::{Result, bail};
use nyx_ext::sys;
use serde_json::{Value, json};

pub const SERIAL: &str = "PM1234";

pub fn on() -> bool {
    std::env::var("RAZER_MOCK").is_ok_and(|v| !v.is_empty())
}

fn no_daemon() -> bool {
    std::env::var("RAZER_MOCK").is_ok_and(|v| v == "no-daemon")
}

fn state_file() -> std::path::PathBuf {
    sys::state_dir().join("razer-mock.json")
}

fn saved() -> Value {
    std::fs::read_to_string(state_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

/// The pretend daemon's reply to a call, as busctl prints it.
pub fn call(path: &str, iface: &str, method: &str, args: &[&str]) -> Result<String> {
    if no_daemon() {
        bail!("Call failed: The name org.razer was not provided by any .service files");
    }
    let dev = format!("/org/razer/device/{SERIAL}");
    if path != "/org/razer" && path != dev {
        bail!("Call failed: Unknown object '{path}'.");
    }
    let mut s = saved();
    let num = |k: &str, d: f64| s[k].as_f64().unwrap_or(d);
    let reply = match (iface, method) {
        ("razer.devices", "getDevices") => format!("as 1 \"{SERIAL}\""),
        ("razer.device.misc", "getDeviceName") => "s \"Razer Basilisk V3\"".into(),
        ("razer.device.misc", "getDeviceType") => "s \"mouse\"".into(),
        ("razer.device.dpi", "getDPI") => format!("ai 2 {0} {0}", num("dpi", 4000.0)),
        ("razer.device.dpi", "maxDPI") => "i 26000".into(),
        ("razer.device.misc", "getPollRate") => format!("i {}", num("poll", 1000.0)),
        ("razer.device.power", "getBattery") => "d 86".into(),
        ("razer.device.power", "isCharging") => "b true".into(),
        ("razer.device.power", "getIdleTime") => format!("q {}", num("idle", 300.0)),
        ("razer.device.lighting.chroma", "getEffect") => "s \"spectrum\"".into(),
        ("razer.device.lighting.brightness", "getBrightness") => format!("d {}", num("brightness", 80.0)),
        (_, set) if set.starts_with("set") => {
            let key = match set {
                "setDPI" => "dpi",
                "setPollRate" => "poll",
                "setIdleTime" => "idle",
                "setBrightness" => "brightness",
                _ => "",
            };
            if let Some(v) = args.get(1).and_then(|v| v.parse::<f64>().ok()).filter(|_| !key.is_empty()) {
                s[key] = json!(v);
                std::fs::write(state_file(), s.to_string())?;
            }
            String::new()
        }
        _ => bail!("Call failed: Unknown method {method} or interface {iface}."),
    };
    Ok(reply)
}

/// The Razer USB devices plugged in: (name, vendor:product).
pub fn usb() -> Vec<(String, String)> {
    vec![("Razer Basilisk V3".into(), "1532:0099".into())]
}

/// Whether the account is in `plugdev`: (in /etc/group, in this session).
pub fn plugdev() -> (bool, bool) {
    if no_daemon() { (false, false) } else { (true, true) }
}
