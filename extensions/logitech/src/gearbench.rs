//! Gearbench (github.com/gearbench/gearbench) drives Logitech G keyboards, mice and
//! headsets through a daemon of its own. Two programs with one device open read
//! each other's replies, so while Gearbench is set up here the devices it drives
//! are left to it: never opened, never written, named from what udev already knows.

use crate::hidpp::device::{Info, Kind};
use crate::hidpp::hidraw::{BUS_BLUETOOTH, NodeInfo};
use crate::hidpp::receiver::ReceiverKind;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// How a model's product id is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Via {
    Cable,
    /// A wireless id behind one of `RECEIVERS`.
    Receiver,
    /// The id of the model's own dongle.
    Own(&'static str),
}

use Kind::{Headset as H, Keyboard as K, Mouse as M};
use Via::{Cable, Own, Receiver};

/// Every product id Gearbench attaches to (vendor 046d), from its guide's "The models
/// and their ids". Gearbench drives nothing over Bluetooth.
const MODELS: &[(u16, &str, Kind, Via)] = &[
    (0xC33E, "G915", K, Cable),
    (0xC541, "G915", K, Own("Lightspeed receiver")),
    (0xC358, "G515 TKL", K, Cable),
    (0xC355, "G515 LIGHTSPEED TKL", K, Cable),
    (0x40B4, "G515 LIGHTSPEED TKL", K, Receiver),
    (0xC343, "G915 TKL or G913 TKL", K, Cable),
    (0xC545, "G915 TKL or G913 TKL", K, Own("Lightspeed receiver")),
    (0x408E, "G915 TKL or G913 TKL", K, Receiver),
    (0xC33F, "G815 or G813", K, Cable),
    (0xC352, "PRO X TKL LIGHTSPEED", K, Cable),
    (0x40B0, "PRO X TKL LIGHTSPEED", K, Receiver),
    (0xC359, "G915 X", K, Cable),
    (0xC356, "G915 X LIGHTSPEED", K, Cable),
    (0x40B5, "G915 X LIGHTSPEED", K, Receiver),
    (0xC357, "G915 X LIGHTSPEED TKL", K, Cable),
    (0x40B6, "G915 X LIGHTSPEED TKL", K, Receiver),
    (0xC361, "G316 X", K, Cable),
    (0xC36B, "G316 X 98", K, Cable),
    (0xC35F, "G512 X 75", K, Cable),
    (0xC360, "G512 X 98", K, Cable),
    (0xC336, "G213 Prodigy", K, Cable),
    (0x4099, "G502 X PLUS", M, Receiver),
    (0xC095, "G502 X PLUS", M, Cable),
    (0x4074, "G305", M, Receiver),
    (0xC08B, "G502 HERO", M, Cable),
    (0x409F, "G502 X LIGHTSPEED", M, Receiver),
    (0xC098, "G502 X LIGHTSPEED", M, Cable),
    (0x4093, "PRO X SUPERLIGHT", M, Receiver),
    (0xC094, "PRO X SUPERLIGHT", M, Cable),
    (0xC099, "G502 X", M, Cable),
    (0x40B2, "G502 X PLUS MILLENNIUM FALCON", M, Receiver),
    (0xC09E, "G502 X PLUS MILLENNIUM FALCON", M, Cable),
    (0x407F, "G502 LIGHTSPEED", M, Receiver),
    (0xC08D, "G502 LIGHTSPEED", M, Cable),
    (0x4079, "PRO WIRELESS", M, Receiver),
    (0xC088, "PRO WIRELESS", M, Cable),
    (0x4087, "G903 LIGHTSPEED HERO", M, Receiver),
    (0xC091, "G903 LIGHTSPEED HERO", M, Cable),
    (0x40A9, "PRO X SUPERLIGHT 2", M, Receiver),
    (0xC09B, "PRO X SUPERLIGHT 2", M, Cable),
    (0x40B8, "PRO X SUPERLIGHT 2 DEX", M, Receiver),
    (0xC0A0, "PRO X SUPERLIGHT 2 DEX", M, Cable),
    (0x40B7, "PRO X SUPERLIGHT 2c", M, Receiver),
    (0xC09F, "PRO X SUPERLIGHT 2c", M, Cable),
    (0x40BD, "PRO X2 SUPERSTRIKE", M, Receiver),
    (0xC0A8, "PRO X2 SUPERSTRIKE", M, Cable),
    (0xC092, "G203 LIGHTSYNC", M, Cable),
    (0xC09D, "G102 LIGHTSYNC", M, Cable),
    (0x0A9C, "G432", H, Own("USB adapter")),
    (0x0A9B, "G431", H, Own("USB adapter")),
    (0x0A6D, "G433", H, Own("USB adapter")),
    (0x0A5C, "G633", H, Cable),
    (0x0A89, "G635", H, Cable),
    (0x0A66, "G533", H, Own("USB receiver")),
    (0x0AC4, "G535", H, Own("USB receiver")),
    (0x0AB5, "G733", H, Own("USB receiver")),
    (0x0AFE, "G733", H, Own("USB receiver")),
    (0x0B1F, "G733", H, Own("USB receiver")),
    (0x0A5B, "G933", H, Own("USB receiver")),
    (0x0A87, "G935", H, Own("USB receiver")),
    (0x0ABA, "PRO X Wireless", H, Own("USB receiver")),
    (0x0B0B, "Astro A50 X", H, Own("base station")),
    (0x0B1C, "Astro A50 Gen 5", H, Own("base station")),
    (0x0AF7, "PRO X 2 LIGHTSPEED", H, Own("USB receiver")),
    (0x0B18, "G522 LIGHTSPEED", H, Own("USB receiver")),
    (0x0B19, "G522 LIGHTSPEED", H, Cable),
    (0x0B21, "G325 LIGHTSPEED", H, Own("USB receiver")),
];

/// The receivers the wireless ids above are paired to.
const RECEIVERS: &[u16] = &[0xC547, 0xC53F, 0xC539, 0xC53A, 0xC54D];

fn model(product: u16) -> Option<&'static (u16, &'static str, Kind, Via)> {
    MODELS.iter().find(|m| m.0 == product)
}

/// A device Gearbench drives, as known without opening it.
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub info: Info,
    /// How it's connected: "USB cable", "Lightspeed receiver", …
    pub via: String,
}

fn held(product: u16, via: String) -> Option<Held> {
    let &(wpid, name, kind, _) = model(product)?;
    Some(Held { info: Info { name: name.to_string(), kind, wpid, ..Default::default() }, via })
}

/// Split nodes into the ones this extension may open and the devices Gearbench drives.
/// A receiver with any of Gearbench's models behind it is left alone entirely, with
/// the devices behind it named from `behind` (product ids) rather than by asking it.
pub fn split(nodes: Vec<NodeInfo>, behind: impl Fn(&NodeInfo) -> Vec<u16>) -> (Vec<NodeInfo>, Vec<Held>) {
    let mut ours = Vec::new();
    let mut theirs: Vec<Held> = Vec::new();
    for node in nodes {
        if node.bus == BUS_BLUETOOTH {
            ours.push(node);
        } else if let Some(&(_, _, _, via)) = model(node.product) {
            let via = match via {
                Cable => "USB cable",
                Receiver => "Lightspeed receiver",
                Own(v) => v,
            };
            theirs.extend(held(node.product, via.to_string()));
        } else if RECEIVERS.contains(&node.product) {
            let label = ReceiverKind::from_product(node.product).map_or("Receiver", |k| k.label());
            let found: Vec<Held> = behind(&node).into_iter().filter_map(|p| held(p, label.to_string())).collect();
            if found.is_empty() {
                ours.push(node);
            } else {
                theirs.extend(found);
            }
        } else {
            ours.push(node);
        }
    }
    let mut seen = std::collections::HashSet::new();
    theirs.retain(|h| seen.insert(h.info.key()));
    (ours, theirs)
}

/// Product ids of the devices hid-logitech-dj found behind a receiver: its child HID
/// devices in sysfs (`…/0003:046D:409F.000B`). Nothing is sent to the receiver.
pub fn sysfs_behind(node: &NodeInfo) -> Vec<u16> {
    let Some(name) = node.path.file_name() else { return vec![] };
    let Ok(entries) = std::fs::read_dir(Path::new("/sys/class/hidraw").join(name).join("device")) else { return vec![] };
    entries
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            let mut parts = n.split(':');
            let (_, vendor, rest) = (parts.next()?, parts.next()?, parts.next()?);
            let product = rest.split('.').next()?;
            (vendor.eq_ignore_ascii_case("046d")).then(|| u16::from_str_radix(product, 16).ok()).flatten()
        })
        .collect()
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })))
}

/// `gearbench-ctl`, on `PATH` or where its package (`/usr/bin`) or `install.sh`
/// (`~/.local/bin`) puts it.
fn ctl_path() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.push(PathBuf::from("/usr/bin"));
    dirs.push(crate::paths::home().join(".local/bin"));
    dirs.into_iter().map(|d| d.join("gearbench-ctl")).find(|p| p.is_file())
}

/// The daemon's user unit, from the package (`/usr/lib/systemd/user/`) or a checkout
/// (`~/.config/systemd/user/`).
fn unit_installed() -> bool {
    [
        crate::paths::config_home().join("systemd/user"),
        PathBuf::from("/etc/systemd/user"),
        PathBuf::from("/usr/lib/systemd/user"),
        PathBuf::from("/usr/local/lib/systemd/user"),
    ]
    .iter()
    .any(|d| d.join("gearbench.service").is_file())
}

/// The daemon's control socket is there, as while it runs.
fn daemon_socket() -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(runtime_dir().join("gearbench.sock")).is_ok_and(|m| m.file_type().is_socket())
}

fn mock_item(item: &str) -> Option<bool> {
    crate::devices::mock_spec().map(|s| s.split(',').any(|x| x.trim() == item))
}

/// Whether Gearbench is set up here: its daemon's socket is there, or `gearbench-ctl`
/// and its user unit are installed. Checked once per run, from files alone.
pub fn set_up() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| mock_item("gearbench").unwrap_or_else(|| daemon_socket() || (ctl_path().is_some() && unit_installed())))
}

/// What opens Gearbench's window (`gearbench-ctl gui`), if it's installed where we can find it.
pub fn launcher() -> Option<PathBuf> {
    match mock_item("gearbench") {
        Some(on) => on.then(|| PathBuf::from("gearbench-ctl")),
        None => ctl_path(),
    }
}

/// Open Gearbench's window, without waiting for it.
pub fn open() -> anyhow::Result<()> {
    use std::process::{Command, Stdio};
    if crate::devices::mock_spec().is_some() {
        return Ok(());
    }
    let ctl = ctl_path().ok_or_else(|| anyhow::anyhow!("gearbench-ctl isn't installed"))?;
    Command::new(ctl).arg("gui").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hidpp::sim;

    #[test]
    fn models_by_id() {
        assert_eq!(model(0x409F).unwrap().1, "G502 X LIGHTSPEED");
        assert_eq!(model(0xC541).unwrap().2, Kind::Keyboard);
        assert!(model(0xB034).is_none(), "MX Master 3S isn't Gearbench's");
        assert!(model(0xC547).is_none(), "receivers aren't models");
        let mut ids: Vec<u16> = MODELS.iter().map(|m| m.0).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MODELS.len(), "an id listed twice");
    }

    fn behind_sim(spec: &str) -> impl Fn(&NodeInfo) -> Vec<u16> + '_ {
        move |n: &NodeInfo| sim::nodes(spec).into_iter().find(|(i, _)| i.path == n.path).map(|(_, s)| s.paired_ids()).unwrap_or_default()
    }

    #[test]
    fn leaves_gearbench_receivers_alone() {
        let spec = "mx-master-3s,mx-keys,g502x,mx-anywhere-3s";
        let infos: Vec<NodeInfo> = sim::nodes(spec).into_iter().map(|(i, _)| i).collect();
        let (ours, theirs) = split(infos.clone(), behind_sim(spec));
        assert_eq!(ours.iter().map(|n| n.path.display().to_string()).collect::<Vec<_>>(), ["/mock/bolt", "/mock/bluetooth"]);
        assert_eq!(theirs.len(), 1);
        assert_eq!((theirs[0].info.name.as_str(), theirs[0].via.as_str()), ("G502 X LIGHTSPEED", "Lightspeed receiver"));
        assert_eq!(theirs[0].info.key(), "409f");

        let (ours, theirs) = split(infos, |_| vec![0x1234]);
        assert_eq!(ours.len(), 3, "a receiver with none of Gearbench's models stays ours");
        assert!(theirs.is_empty());
    }

    #[test]
    fn wired_and_bluetooth() {
        let node = |product, bus| NodeInfo { path: "/dev/hidraw9".into(), name: String::new(), bus, product, short: true, long: true };
        let (ours, theirs) = split(vec![node(0xC098, 0x0003)], |_| vec![]);
        assert!(ours.is_empty());
        assert_eq!(theirs[0].via, "USB cable");
        let (ours, theirs) = split(vec![node(0xC098, BUS_BLUETOOTH)], |_| vec![]);
        assert_eq!((ours.len(), theirs.len()), (1, 0), "Gearbench drives nothing over Bluetooth");
    }
}
