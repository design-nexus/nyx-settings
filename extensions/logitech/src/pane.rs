//! The preview pane beside a device's page: the device with what its wheels and
//! buttons do, its readings, its Easy-Switch computers and details. Only sent when
//! Settings can draw it; everything in it was read from the device.

use crate::hidpp::device::{Info, Kind, feature as f};
use crate::hidpp::features::battery::{Battery, Charging};
use crate::hidpp::features::buttons::{self, Control};
use crate::hidpp::features::gaming::Onboard;
use crate::hidpp::features::keyboard::Backlight;
use crate::hidpp::features::pointer::Dpi;
use crate::hidpp::features::wheel::WheelMode;
use crate::hidpp::hidraw::{BUS_BLUETOOTH, NodeInfo};
use crate::hidpp::link::Link;
use crate::hidpp::receiver::ReceiverKind;
use serde_json::{Value, json};

/// Whether this Settings draws a preview pane beside the groups.
pub fn supported() -> bool {
    std::env::var("SETTINGS_KINDS").is_ok_and(|k| k.split(' ').any(|x| x == "preview"))
}

/// What the page read from the device, for the pane.
#[derive(Debug, Default)]
pub struct Seen {
    pub battery: Option<Battery>,
    pub dpi: Option<Dpi>,
    pub rate: Option<u32>,
    pub wheel: Option<WheelMode>,
    pub thumb_inverted: Option<bool>,
    /// The buttons shown on the page.
    pub buttons: Vec<Control>,
    pub disabled: Vec<&'static str>,
    pub backlight: Option<Backlight>,
    /// (computers, current), 0-based.
    pub hosts: Option<(u8, u8)>,
    /// Each computer's (paired, name), where the device knows them.
    pub host_names: Vec<(bool, String)>,
    pub onboard: Option<Onboard>,
    /// The first lit zone's colour.
    pub lit: Option<[u8; 3]>,
    /// Colour rows, drawn in the pane instead of the Lighting group.
    pub colours: Vec<Value>,
}

/// Each Easy-Switch computer's (paired, name), from HostsInfo (0x1815).
pub fn host_names(l: &mut Link, info: &Info) -> Vec<(bool, String)> {
    let Ok(r) = info.call(l, f::HOSTS_INFO, 0, &[]) else { return vec![] };
    if r.len() < 3 || r[0] & 0x01 == 0 {
        return vec![];
    }
    let mut out = Vec::new();
    for host in 0..r[2] {
        let Ok(h) = info.call(l, f::HOSTS_INFO, 1, &[host]) else { return vec![] };
        if h.len() < 5 {
            return vec![];
        }
        let len = h[4] as usize;
        let mut name = Vec::new();
        while name.len() < len {
            let Ok(piece) = info.call(l, f::HOSTS_INFO, 3, &[host, name.len() as u8]) else { break };
            let take = (len - name.len()).min(14).min(piece.len().saturating_sub(2));
            if take == 0 {
                break;
            }
            name.extend_from_slice(&piece[2..2 + take]);
        }
        out.push((h[1] != 0, String::from_utf8_lossy(&name).trim_matches(char::from(0)).trim().to_string()));
    }
    out
}

pub fn picture(kind: Kind) -> &'static str {
    match kind {
        Kind::Keyboard | Kind::Numpad => "keyboard",
        Kind::Headset => "headset",
        Kind::Touchpad => "tablet",
        _ => "mouse",
    }
}

fn thousands(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn battery_item(b: &Battery) -> Value {
    let value = match (b.charging, b.approximate) {
        (Charging::Full, _) => "Full".to_string(),
        (_, true) => format!("About {}%", b.percent),
        _ => format!("{}%", b.percent),
    };
    let (sub, tone) = match b.charging {
        _ if b.is_low() => ("Charge soon", "warn"),
        Charging::Discharging => ("Discharging", "good"),
        Charging::Charging => ("Charging", "good"),
        Charging::Full => ("Charged", "good"),
        Charging::Error => ("Charging problem", "bad"),
    };
    json!({ "label": "Battery", "value": value, "sub": sub, "tone": tone })
}

/// What the wheels and the side and gesture buttons do, as labels beside the mouse.
fn labels(seen: &Seen) -> Vec<Value> {
    let mut items = Vec::new();
    if let Some(m) = seen.wheel {
        let mode = match m {
            WheelMode::Ratchet => "Ratchet",
            WheelMode::FreeSpin => "Free spin",
            WheelMode::SmartShift(_) => "SmartShift",
        };
        items.push(json!({ "label": "Wheel", "value": mode }));
    }
    if let Some(inverted) = seen.thumb_inverted {
        items.push(json!({ "label": "Thumb wheel", "value": if inverted { "Reversed" } else { "Scrolls sideways" } }));
    }
    let does = |cid: u16| {
        seen.buttons.iter().find(|c| c.cid == cid).map(|c| match buttons::name(c.remap) {
            _ if c.remap == c.cid => "Default".to_string(),
            n if n.is_empty() => "Remapped".to_string(),
            n => n,
        })
    };
    match (does(0x53), does(0x56)) {
        (Some(b), Some(fw)) if b == fw => items.push(json!({ "label": "Back · Forward", "value": b })),
        (b, fw) => {
            for (label, v) in [("Back", b), ("Forward", fw)] {
                if let Some(v) = v {
                    items.push(json!({ "label": label, "value": v }));
                }
            }
        }
    }
    if let Some(g) = does(0xC3).or_else(|| does(0x60)) {
        items.push(json!({ "label": "Gesture", "value": g }));
    }
    items
}

fn stats(seen: &Seen) -> Vec<Value> {
    let mut items = Vec::new();
    if let Some(b) = &seen.battery {
        items.push(battery_item(b));
    }
    if let Some(d) = &seen.dpi {
        let (min, max, _) = d.choices.bounds();
        items.push(json!({
            "label": "Speed", "value": format!("{} dpi", thousands(d.current as u32)),
            "sub": format!("{}–{}", thousands(min as u32), thousands(max as u32)),
        }));
    }
    if let Some(hz) = seen.rate.filter(|hz| *hz > 0) {
        let ms = format!("{:.3}", 1000.0 / hz as f64).trim_end_matches('0').trim_end_matches('.').to_string();
        items.push(json!({ "label": "Report rate", "value": format!("{hz} Hz"), "sub": format!("Every {ms} ms") }));
    }
    if let Some(b) = &seen.backlight {
        let value = match (b.enabled, b.levels) {
            (false, _) => "Off".to_string(),
            (true, n) if n > 1 => format!("{} of {}", b.level.min(n - 1), n - 1),
            _ => "On".to_string(),
        };
        let sub = if b.enabled { format!("Off after {} s", b.timeout) } else { String::new() };
        items.push(json!({ "label": "Backlight", "value": value, "sub": sub }));
    }
    match seen.disabled.as_slice() {
        [] => {}
        [one] => items.push(json!({ "label": one, "value": "Turned off", "sub": "Does nothing" })),
        many => items.push(json!({ "label": "Turned off", "value": format!("{} keys", many.len()), "sub": many.join(", ") })),
    }
    if let Some(o) = &seen.onboard {
        items.push(if o.onboard {
            json!({ "label": "Profile", "value": format!("{} of {}", o.current, o.profiles), "sub": "Stored in the mouse" })
        } else {
            json!({ "label": "Profiles", "value": "Off", "sub": "Set from this computer" })
        });
    }
    items
}

fn easy_switch(seen: &Seen) -> Option<Value> {
    let (count, current) = seen.hosts.filter(|(count, _)| *count > 1)?;
    let items: Vec<Value> = (0..count)
        .map(|h| {
            let (paired, name) = seen.host_names.get(h as usize).cloned().unwrap_or((true, String::new()));
            let sub = if !name.is_empty() {
                name
            } else if !paired {
                "Not paired".to_string()
            } else {
                String::new()
            };
            let mut item = json!({ "icon": "computer-symbolic", "label": format!("Computer {}", h + 1), "sub": sub });
            if h == current {
                item["value"] = json!("This computer");
                item["tone"] = json!("accent");
            }
            item
        })
        .collect();
    Some(json!({ "kind": "list", "title": "Easy-Switch", "items": items }))
}

fn facts(facts: &[(String, String)]) -> Value {
    json!({ "kind": "facts", "items": facts.iter().map(|(l, v)| json!({ "label": l, "value": v })).collect::<Vec<_>>() })
}

/// The pane for a connected device.
pub fn preview(info: &Info, seen: Seen, details: &[(String, String)]) -> Value {
    let mut blocks = Vec::new();
    if let Some(b) = seen.battery.filter(Battery::is_low) {
        blocks.push(json!({ "kind": "banner", "text": format!("<b>Charge soon:</b> {}% left.", b.percent), "warning": true }));
    }
    let mut pic = json!({ "kind": "picture", "picture": picture(info.kind) });
    if let Some(c) = seen.lit {
        pic["accent"] = json!(crate::state::hex_colour(c));
    }
    let labels = labels(&seen);
    if !labels.is_empty() {
        pic["items"] = json!(labels);
    }
    blocks.push(pic);
    if !seen.colours.is_empty() {
        blocks.push(json!({ "kind": "rows", "title": "Colour", "rows": seen.colours }));
    }
    for chunk in stats(&seen).chunks(4) {
        blocks.push(json!({ "kind": "stats", "items": chunk }));
    }
    blocks.extend(easy_switch(&seen));
    blocks.push(facts(details));
    json!({ "title": info.name, "blocks": blocks })
}

/// The pane for a paired device that isn't connected.
pub fn offline(info: &Info, details: &[(String, String)]) -> Value {
    json!({ "title": info.name, "blocks": [
        { "kind": "picture", "picture": picture(info.kind) },
        { "kind": "caption", "text": "Not connected: switched off, asleep or out of range." },
        facts(details),
    ]})
}

/// The line under the pane's title: how it's connected, its firmware, whether settings are kept.
pub fn line(via: &str, info: &Info, kept: bool) -> String {
    let mut parts = vec![via.to_string()];
    let version = info
        .firmware
        .iter()
        .find_map(|fw| fw.strip_prefix("Firmware "))
        .and_then(|rest| rest.split_whitespace().last())
        .map(|v| v.split(".B").next().unwrap_or(v));
    if let Some(v) = version {
        parts.push(format!("firmware {v}"));
    }
    if kept {
        parts.push("settings put back on connect".into());
    }
    parts.join(" · ")
}

/// The setup page's pane: what was found, and what's missing.
pub fn setup(nodes: &[NodeInfo]) -> Value {
    let title = match nodes.len() {
        1 => "1 Logitech device found".to_string(),
        n => format!("{n} Logitech devices found"),
    };
    let mut items: Vec<Value> = nodes
        .iter()
        .map(|n| {
            let receiver = ReceiverKind::from_product(n.product);
            let bluetooth = n.bus == BUS_BLUETOOTH;
            let icon = match (receiver, bluetooth) {
                (Some(_), _) => "network-wireless-symbolic",
                (None, true) => "bluetooth-symbolic",
                _ => "input-mouse-symbolic",
            };
            json!({
                "icon": icon, "label": receiver.map(|r| r.label().to_string()).unwrap_or_else(|| n.name.clone()),
                "sub": if bluetooth { "Connected over Bluetooth" } else { "Plugged in over USB" }, "value": "Found", "tone": "good",
            })
        })
        .collect();
    items.push(json!({
        "icon": "system-lock-screen-symbolic", "label": "Your account can't open them yet",
        "sub": "Only root can until the rule is installed", "value": "Missing", "tone": "warn",
    }));
    items.push(json!({ "icon": "security-high-symbolic", "label": "One rule, asked once", "sub": "Asks for your password; nothing else changes" }));
    json!({
        "title": title, "line": "Battery, buttons and lighting all need it.",
        "blocks": [{ "kind": "picture", "picture": "mouse" }, { "kind": "list", "items": items }],
    })
}

fn icon_for(row: &Value, pointing: bool) -> &'static str {
    let key = row["key"].as_str().unwrap_or("");
    let title = row["title"].as_str().unwrap_or("");
    match key {
        "dpi" => "input-mouse-symbolic",
        "report-rate" | "backlight-timeout" => "preferences-system-time-symbolic",
        "wheel" => "view-continuous-symbolic",
        "smartshift" => "emblem-system-symbolic",
        "invert-wheel" => "object-flip-vertical-symbolic",
        "invert-thumb" => "object-flip-horizontal-symbolic",
        "fn-swap" | "disabled-keys" => "input-keyboard-symbolic",
        "backlight" => "keyboard-brightness-symbolic",
        "backlight-level" => "display-brightness-symbolic",
        "host" => "computer-symbolic",
        "onboard" => "input-gaming-symbolic",
        "profile" => "view-list-ordered-symbolic",
        "forget" => "edit-undo-symbolic",
        "keep-applied" => "view-refresh-symbolic",
        "allow" => "system-lock-screen-symbolic",
        k if k.starts_with("button:") && pointing => "input-mouse-symbolic",
        k if k.starts_with("button:") => "input-keyboard-symbolic",
        k if k.ends_with(":effect") => "starred-symbolic",
        k if k.ends_with(":period") => "preferences-system-time-symbolic",
        k if k.ends_with(":colour") => "preferences-color-symbolic",
        _ if row["kind"] == "disclosure" => "preferences-desktop-keyboard-shortcuts-symbolic",
        _ if row["value"].as_str().is_some_and(|v| v.starts_with("Couldn't read")) => "dialog-warning-symbolic",
        _ if title == "Keep settings applied" => "view-refresh-symbolic",
        _ if title.starts_with("Lighting") => "starred-symbolic",
        _ if title.contains("isn't connected") => "network-wireless-offline-symbolic",
        _ => "emblem-system-symbolic",
    }
}

/// An icon on every row, nested rows too.
pub fn add_icons(rows: &mut [Value], pointing: bool) {
    for row in rows {
        if row.get("icon").is_none() {
            row["icon"] = json!(icon_for(row, pointing));
        }
        if let Some(inner) = row.get_mut("rows").and_then(Value::as_array_mut) {
            add_icons(inner, pointing);
        }
    }
}

/// Icons on every row of every group.
pub fn add_group_icons(groups: &mut [Value], pointing: bool) {
    for g in groups {
        if let Some(rows) = g.get_mut("rows").and_then(Value::as_array_mut) {
            add_icons(rows, pointing);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(thousands(25600), "25,600");
        assert_eq!(thousands(800), "800");
    }

    #[test]
    fn firmware_line() {
        let i = Info { firmware: vec!["Firmware RBM 12.02.B0014".into(), "Bootloader BL2 02.00.B0005".into()], ..Default::default() };
        assert_eq!(line("Bolt receiver", &i, true), "Bolt receiver · firmware 12.02 · settings put back on connect");
        assert_eq!(line("USB", &Info::default(), false), "USB");
    }
}
