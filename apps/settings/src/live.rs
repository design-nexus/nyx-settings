//! Pages that show the system as it is (Wi-Fi, Bluetooth, sound, services…) stay
//! current while open: a cheap check runs in the background when what the page shows
//! may have changed (NetworkManager, BlueZ, PipeWire, udev or systemd says so), or
//! every few seconds for pages nothing announces changes for, and the page is redrawn
//! only when its answer changes.

use crate::backend::{audio, bt, net, streams, sysinfo};
use crate::cmd;
use nyx_capability::Source;

fn lines(cmd: &[&str], matching: &[&str]) -> Source {
    Source::Lines { cmd: cmd.iter().map(|s| s.to_string()).collect(), matching: matching.iter().map(|s| s.to_string()).collect() }
}

/// What announces a change to a watched page; `None`: it's checked on a timer.
pub fn sources_for(id: &str) -> Option<Vec<Source>> {
    Some(match id {
        "wifi" => vec![lines(&["nmcli", "monitor"], &[])],
        "bluetooth" => vec![lines(
            &["dbus-monitor", "--system", "type='signal',sender='org.bluez'"],
            &["member=InterfacesAdded", "member=InterfacesRemoved", "member=PropertiesChanged"],
        )],
        "audio" => vec![lines(&["pactl", "subscribe"], &["sink", "source", "card", "server"])],
        "power" => vec![
            Source::Udev("power_supply".into()),
            lines(&["dbus-monitor", "--system", "type='signal',sender='net.hadess.PowerProfiles'"], &["member=PropertiesChanged"]),
        ],
        "services" => vec![Source::Units { system: true }, Source::Units { system: false }],
        "displays" => vec![Source::Udev("drm".into())],
        _ => return None,
    })
}

/// How a page's state is summed up, for the pages that are watched.
pub fn signature_for(id: &str) -> Option<fn() -> String> {
    Some(match id {
        "wifi" => wifi,
        "bluetooth" => bluetooth,
        "audio" => sound,
        "power" => power,
        "services" => services,
        "displays" => displays,
        "printers" => printers,
        "sharing" => crate::sections::sharing::signature,
        "assistant-activity" => crate::sections::assistant::activity::signature,
        "assistant-performance" => crate::sections::assistant::performance::signature,
        _ => return None,
    })
}

fn wifi() -> String {
    // Names and which is in use; signal strength wobbles too much to redraw for.
    let mut out: Vec<String> = net::networks(false).into_iter().map(|n| format!("{}:{}", n.ssid, n.in_use)).collect();
    out.sort();
    out.push(cmd::output(&["nmcli", "-t", "-f", "NAME,TYPE", "connection", "show"]).unwrap_or_default());
    out.push(cmd::output(&["nmcli", "radio", "wifi"]).unwrap_or_default());
    out.join("\n")
}

fn bluetooth() -> String {
    let mut out: Vec<String> =
        bt::devices().into_iter().map(|d| format!("{} {} {} {}", d.address, d.name, d.paired, d.connected)).collect();
    out.push(bt::powered().to_string());
    out.join("\n")
}

fn sound() -> String {
    // Devices, the defaults and which apps are playing where; not volumes, which
    // move while their sliders are dragged.
    let mut out: Vec<String> = audio::hardware_sinks().into_iter().map(|d| d.name).collect();
    out.extend(audio::sources().into_iter().map(|d| d.name));
    out.push(audio::default_sink());
    out.push(audio::default_source());
    out.extend(streams::list().into_iter().map(|s| format!("{} {} {}", s.index, s.app, s.sink)));
    out.join("\n")
}

fn power() -> String {
    let profile = cmd::output(&["powerprofilesctl", "get"]).unwrap_or_default();
    // Whole battery percent and charging state.
    let battery = sysinfo::battery().map(|b| format!("{} {}", b.percent, b.status)).unwrap_or_default();
    format!("{profile}\n{battery}")
}

fn services() -> String {
    cmd::output(&["systemctl", "list-units", "--type=service", "--all", "--plain", "--no-legend", "--no-pager"])
        .unwrap_or_default()
        .lines()
        // unit, load, active, sub
        .map(|l| l.split_whitespace().take(4).collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn displays() -> String {
    cmd::output(&["hyprctl", "monitors", "all", "-j"])
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|m| format!("{} {} {} {} {}", m["name"], m["disabled"], m["description"], m["vrr"], m["hardwareCursorsInUse"]))
        .collect::<Vec<_>>()
        .join("\n")
}

fn printers() -> String {
    cmd::output(&["lpstat", "-p", "-d"]).unwrap_or_default()
}
