//! Finding devices: open every Logitech HID++ node, ask receivers what's paired
//! and connected, and identify each device. No GTK here; the window's
//! the extension and the `watch` service share it.

use crate::gearbench;
use crate::hidpp::device::{self, Info, Kind};
use crate::hidpp::hidraw::{self, Hidraw, NodeInfo};
use crate::hidpp::link::{DIRECT, Io, Link};
use crate::hidpp::receiver::{self, Notice, ReceiverKind};
use crate::hidpp::sim;
use std::collections::HashMap;
use std::io;
use std::sync::{Mutex, OnceLock};

/// `LOGI_MOCK=mx-master-3s,mx-keys,g502x,mx-anywhere-3s` runs against simulated hardware.
pub fn mock_spec() -> Option<String> {
    std::env::var("LOGI_MOCK").ok().filter(|s| !s.trim().is_empty())
}

/// `denied` in `LOGI_MOCK`: the simulated nodes refuse to open, as without the udev rule.
fn mock_denied() -> bool {
    mock_spec().is_some_and(|s| s.split(',').any(|x| x.trim() == "denied"))
}

/// Where pretend mode keeps what it saves (`SETTINGS_EXTENSION_STATE`), never the real files.
pub fn mock_state() -> Option<std::path::PathBuf> {
    mock_spec()?;
    Some(
        std::env::var_os("SETTINGS_EXTENSION_STATE")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| std::env::temp_dir().join("settings-logitech-mock")),
    )
}

/// Simulated nodes are created once and handed out on first open.
static MOCK: OnceLock<Mutex<HashMap<String, sim::SimNode>>> = OnceLock::new();

fn mock_nodes() -> &'static Mutex<HashMap<String, sim::SimNode>> {
    MOCK.get_or_init(|| {
        let nodes = mock_spec().map(|s| sim::nodes(&s)).unwrap_or_default();
        Mutex::new(nodes.into_iter().map(|(info, node)| (info.path.display().to_string(), node)).collect())
    })
}

fn all_nodes() -> Vec<NodeInfo> {
    match mock_spec() {
        Some(spec) => sim::nodes(&spec).into_iter().map(|(i, _)| i).collect(),
        None => hidraw::scan(),
    }
}

/// The devices on each pretend receiver.
fn mock_behind(node: &NodeInfo) -> Vec<u16> {
    let spec = mock_spec().unwrap_or_default();
    sim::nodes(&spec).into_iter().find(|(i, _)| i.path == node.path).map(|(_, n)| n.paired_ids()).unwrap_or_default()
}

fn split() -> (Vec<NodeInfo>, Vec<gearbench::Held>) {
    let nodes = all_nodes();
    if !gearbench::set_up() {
        return (nodes, vec![]);
    }
    if mock_spec().is_some() { gearbench::split(nodes, mock_behind) } else { gearbench::split(nodes, gearbench::sysfs_behind) }
}

/// Every HID++ node present now that this extension may open: not the ones Gearbench drives.
pub fn scan() -> Vec<NodeInfo> {
    split().0
}

/// The devices Gearbench drives, known without opening them.
pub fn held() -> Vec<gearbench::Held> {
    split().1
}

pub fn open(info: &NodeInfo) -> io::Result<Link> {
    let io: Box<dyn Io> = if mock_spec().is_some() {
        if mock_denied() {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        let key = info.path.display().to_string();
        let node = mock_nodes().lock().unwrap().remove(&key).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        Box::new(node)
    } else {
        Box::new(Hidraw::open(&info.path)?)
    };
    Ok(Link::new(io, info.short, info.long))
}

/// A paired or connected device, as last seen.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub info: Info,
    pub online: bool,
    pub battery: Option<crate::hidpp::features::battery::Battery>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReceiverFound {
    pub kind: ReceiverKind,
    pub serial: String,
}

/// What one node holds: a receiver and its devices, or one direct device.
#[derive(Debug, Clone, PartialEq)]
pub struct Discovered {
    pub receiver: Option<ReceiverFound>,
    pub devices: Vec<Found>,
}

/// Identify a device and read its battery.
pub fn identify(link: &mut Link, index: u8, wpid: u16, serial: &str, name: &str, kind: Kind) -> crate::hidpp::link::Result<Found> {
    let info = device::probe(link, index, wpid, serial, name, kind)?;
    let battery = crate::hidpp::features::battery::read(link, &info).ok().flatten();
    Ok(Found { info, online: true, battery })
}

/// Everything behind one node. Receivers are also told to send notifications.
pub fn discover(link: &mut Link, node: &NodeInfo) -> crate::hidpp::link::Result<Discovered> {
    let Some(kind) = ReceiverKind::from_product(node.product) else {
        let found = identify(link, DIRECT, node.product, "", &node.name, Kind::Other)?;
        return Ok(Discovered { receiver: None, devices: vec![found] });
    };
    let _ = receiver::enable_notifications(link);
    let serial = receiver::serial(link, kind);
    let mut devices = Vec::new();
    for slot in 1..=kind.slots() {
        if let Some(p) = receiver::paired(link, kind, slot) {
            let info = Info {
                index: slot,
                name: if p.name.is_empty() { format!("{} device", p.kind.label()) } else { p.name.clone() },
                kind: p.kind,
                wpid: p.wpid,
                serial: p.serial.clone(),
                ..Default::default()
            };
            devices.push(Found { info, online: false, battery: None });
        }
    }
    // Which of them are connected right now.
    let _ = receiver::request_connections(link);
    let events = link.listen(400).unwrap_or_default();
    for r in &events {
        if let Some(Notice::Connection { slot, online: true, wpid, kind: k }) = receiver::parse_notice(r) {
            if !devices.iter().any(|d| d.info.index == slot) {
                let info = Info { index: slot, kind: k, wpid, name: format!("{} device", k.label()), ..Default::default() };
                devices.push(Found { info, online: false, battery: None });
            }
            if let Some(d) = devices.iter_mut().find(|d| d.info.index == slot) {
                d.online = true;
            }
        }
    }
    for d in devices.iter_mut().filter(|d| d.online) {
        let i = d.info.clone();
        match identify(link, i.index, i.wpid, &i.serial, &i.name, i.kind) {
            Ok(found) => *d = found,
            Err(e) if e.is_offline() => d.online = false,
            Err(e) => eprintln!("settings-logitech: couldn't read {}: {e}", i.name),
        }
    }
    // Lightspeed receivers don't always keep pairing info; the notice is enough.
    devices.sort_by_key(|d| d.info.index);
    Ok(Discovered { receiver: Some(ReceiverFound { kind, serial }), devices })
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::hidpp::device::feature as f;

    fn sim_link(spec: &str, which: usize) -> (NodeInfo, Link) {
        let (info, node) = sim::nodes(spec).into_iter().nth(which).unwrap();
        let link = Link::new(Box::new(node), info.short, info.long);
        (info, link)
    }

    #[test]
    fn discovers_bolt_receiver() {
        let (node, mut link) = sim_link("mx-master-3s,mx-keys:off", 0);
        let d = discover(&mut link, &node).unwrap();
        assert_eq!(d.receiver.as_ref().unwrap().kind, ReceiverKind::Bolt);
        assert_eq!(d.devices.len(), 2);
        let mouse = &d.devices[0];
        assert!(mouse.online);
        assert_eq!(mouse.info.name, "MX Master 3S");
        assert_eq!(mouse.info.kind, Kind::Mouse);
        assert!(mouse.info.has(f::SMART_SHIFT_ENHANCED));
        assert_eq!(mouse.info.key(), "b034-8a215c03");
        assert_eq!(mouse.battery.unwrap().percent, 70);
        let keys = &d.devices[1];
        assert!(!keys.online);
        assert_eq!(keys.info.name, "MX Keys");
        assert_eq!(keys.info.kind, Kind::Keyboard);
    }

    #[test]
    fn discovers_direct_device() {
        let (node, mut link) = sim_link("mx-anywhere-3s", 0);
        let d = discover(&mut link, &node).unwrap();
        assert!(d.receiver.is_none());
        assert_eq!(d.devices[0].info.name, "MX Anywhere 3S");
        assert_eq!(d.devices[0].info.index, DIRECT);
    }

    #[test]
    fn reads_and_writes_features() {
        use crate::hidpp::features::{buttons, gaming, keyboard, pointer, wheel};
        let (node, mut link) = sim_link("mx-master-3s,mx-keys,g502x", 0);
        let d = discover(&mut link, &node).unwrap();
        let mouse = d.devices[0].info.clone();
        let dpi = pointer::read_dpi(&mut link, &mouse).unwrap();
        assert_eq!(dpi.choices, pointer::DpiChoices::Range { min: 200, max: 8000, step: 50 });
        pointer::write_dpi(&mut link, &mouse, 2400).unwrap();
        assert_eq!(pointer::read_dpi(&mut link, &mouse).unwrap().current, 2400);
        wheel::write_mode(&mut link, &mouse, wheel::WheelMode::FreeSpin).unwrap();
        assert_eq!(wheel::read_mode(&mut link, &mouse).unwrap(), wheel::WheelMode::FreeSpin);
        wheel::write_mode(&mut link, &mouse, wheel::WheelMode::SmartShift(30)).unwrap();
        assert_eq!(wheel::read_mode(&mut link, &mouse).unwrap(), wheel::WheelMode::SmartShift(30));
        wheel::write_hires(&mut link, &mouse, true, true).unwrap();
        assert!(wheel::read_hires(&mut link, &mouse).unwrap().inverted);
        let controls = buttons::read(&mut link, &mouse).unwrap();
        assert_eq!(controls.len(), 7);
        buttons::remap(&mut link, &mouse, 0x53, 0x52).unwrap();
        assert_eq!(buttons::read(&mut link, &mouse).unwrap()[3].remap, 0x52);

        let keys = d.devices[1].info.clone();
        assert!(keyboard::read_fn_swap(&mut link, &keys).unwrap());
        keyboard::write_fn_swap(&mut link, &keys, false).unwrap();
        assert!(!keyboard::read_fn_swap(&mut link, &keys).unwrap());
        assert_eq!(keyboard::read_disabled_keys(&mut link, &keys).unwrap(), (0x19, 0x01));
        let mut b = keyboard::read_backlight(&mut link, &keys).unwrap();
        assert_eq!(b.levels, 8);
        b.level = 6;
        keyboard::write_backlight(&mut link, &keys, &b, true).unwrap();
        assert_eq!(keyboard::read_backlight(&mut link, &keys).unwrap().level, 6);
        assert_eq!(keyboard::read_hosts(&mut link, &keys).unwrap(), (3, 0));

        let (node, mut link) = sim_link("mx-master-3s,mx-keys,g502x", 1);
        let d = discover(&mut link, &node).unwrap();
        assert_eq!(d.receiver.as_ref().unwrap().kind, ReceiverKind::Lightspeed);
        let g = d.devices[0].info.clone();
        assert!(g.is_gaming());
        let rate = pointer::read_report_rate(&mut link, &g).unwrap();
        assert_eq!(rate.choices, vec![125, 250, 500, 1000]);
        assert_eq!(rate.current, 1000);
        pointer::write_report_rate(&mut link, &g, 500).unwrap();
        assert_eq!(pointer::read_report_rate(&mut link, &g).unwrap().current, 500);
        let ob = gaming::read_onboard(&mut link, &g).unwrap();
        assert_eq!((ob.profiles, ob.onboard, ob.current), (5, true, 2));
        let zones = gaming::read_zones(&mut link, &g).unwrap();
        assert_eq!(zones.len(), 2);
        assert_eq!(zones[0].effect, gaming::Effect::Breathe);
        assert_eq!(zones[0].period, 3000);
        gaming::write_zone(&mut link, &g, &zones[0], gaming::Effect::Static, [1, 2, 3], 0).unwrap();
        assert_eq!(gaming::read_zones(&mut link, &g).unwrap()[0].colour, [1, 2, 3]);
    }

    #[test]
    fn unpairs() {
        let (node, mut link) = sim_link("mx-master-3s,mx-keys", 0);
        receiver::unpair(&mut link, ReceiverKind::Bolt, 2).unwrap();
        let d = discover(&mut link, &node).unwrap();
        assert_eq!(d.devices.len(), 1);
    }

    #[test]
    fn saved_settings_apply() {
        let (node, mut link) = sim_link("mx-master-3s", 0);
        let d = discover(&mut link, &node).unwrap();
        let mouse = d.devices[0].info.clone();
        let s = crate::state::Saved { dpi: Some(3200), wheel: Some("ratchet".into()), ..Default::default() };
        assert!(crate::state::apply(&mut link, &mouse, &s).is_empty());
        assert_eq!(crate::hidpp::features::pointer::read_dpi(&mut link, &mouse).unwrap().current, 3200);
    }
}
