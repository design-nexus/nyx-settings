//! Bluetooth devices through `bluetoothctl`, with Omarchy's own helpers for
//! power and for pairing/connecting (they power the adapter up and trust the
//! device first).

use crate::cmd;
use anyhow::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub address: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

/// `bluetoothctl devices [Paired|Connected]`: "Device AA:BB:… Name".
pub fn parse_devices(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("Device "))
        .filter_map(|r| r.split_once(' ').map(|(a, n)| (a.to_string(), n.trim().to_string())))
        .collect()
}

/// A discovered device with no real name: BlueZ shows its address instead.
pub fn unnamed(address: &str, name: &str) -> bool {
    name.is_empty() || name.replace('-', ":").eq_ignore_ascii_case(address)
}

fn list(filter: Option<&str>) -> Vec<(String, String)> {
    let mut args = vec!["bluetoothctl", "devices"];
    args.extend(filter);
    parse_devices(&cmd::output(&args).unwrap_or_default())
}

/// Known and discovered devices: paired ones first, then nearby named ones.
pub fn devices() -> Vec<Device> {
    let paired: Vec<String> = list(Some("Paired")).into_iter().map(|(a, _)| a).collect();
    let connected: Vec<String> = list(Some("Connected")).into_iter().map(|(a, _)| a).collect();
    let mut out: Vec<Device> = list(None)
        .into_iter()
        .map(|(address, name)| Device {
            paired: paired.contains(&address),
            connected: connected.contains(&address),
            address,
            name,
        })
        .filter(|d| d.paired || !unnamed(&d.address, &d.name))
        .collect();
    out.sort_by(|a, b| {
        b.connected.cmp(&a.connected).then(b.paired.cmp(&a.paired)).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// Omarchy's scripts also unblock the radio (rfkill); elsewhere BlueZ's own commands.
fn omarchy(script: &str) -> bool {
    crate::capability::program(script)
}

pub fn powered() -> bool {
    if omarchy("omarchy-bluetooth-power") {
        return cmd::run(&["omarchy-bluetooth-power", "is-on"]).is_ok();
    }
    cmd::output(&["bluetoothctl", "show"]).is_some_and(|t| t.lines().any(|l| l.trim() == "Powered: yes"))
}

pub fn set_power(on: bool) -> Result<String> {
    if omarchy("omarchy-bluetooth-power") {
        return cmd::run(&["omarchy-bluetooth-power", if on { "on" } else { "off" }]);
    }
    if on {
        let _ = cmd::run(&["rfkill", "unblock", "bluetooth"]);
    }
    cmd::run(&["bluetoothctl", "power", if on { "on" } else { "off" }])
}

/// Look for nearby devices for a few seconds.
pub fn scan(seconds: u32) -> Result<String> {
    cmd::run(&["bluetoothctl", "--timeout", &seconds.to_string(), "scan", "on"])
}

/// pair, connect, disconnect or forget.
pub fn device(action: &str, address: &str) -> Result<String> {
    if omarchy("omarchy-bluetooth-device") {
        return cmd::run(&["omarchy-bluetooth-device", action, address]);
    }
    match action {
        // Trusted, so it reconnects by itself next time, as Omarchy's script does.
        "pair" => cmd::run(&["bluetoothctl", "--timeout", "20", "pair", address])
            .and_then(|_| cmd::run(&["bluetoothctl", "trust", address]))
            .and_then(|_| cmd::run(&["bluetoothctl", "--timeout", "10", "connect", address])),
        "connect" => cmd::run(&["bluetoothctl", "--timeout", "10", "connect", address]),
        "disconnect" => cmd::run(&["bluetoothctl", "disconnect", address]),
        "forget" => cmd::run(&["bluetoothctl", "remove", address]),
        other => cmd::run(&["bluetoothctl", other, address]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_lines() {
        let text = "Device 00:11:22:33:44:55 WH-1000XM4\nDevice AA:BB:CC:DD:EE:FF MX Keys Mini\nController 12:34 host\n";
        assert_eq!(
            parse_devices(text),
            [
                ("00:11:22:33:44:55".to_string(), "WH-1000XM4".to_string()),
                ("AA:BB:CC:DD:EE:FF".to_string(), "MX Keys Mini".to_string())
            ]
        );
    }

    #[test]
    fn spots_unnamed_devices() {
        assert!(unnamed("4A:1B:2C:3D:4E:5F", "4A-1B-2C-3D-4E-5F"));
        assert!(!unnamed("4A:1B:2C:3D:4E:5F", "Pixel Buds"));
    }
}
