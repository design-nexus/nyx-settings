//! Remembered devices: displays, mice and sound devices with their own settings keep
//! them while unplugged, and get them back when they return.
//!
//! The settings stay where they always were (display and mouse rules in `state.json`,
//! sound devices in `audio-devices.toml`, the dock's per-screen section in its own file)
//! and are restored by the subsystem matching the device itself: Hyprland's `desc:`
//! monitor rules and `hl.device` names, a WirePlumber rule by `node.name`. The index
//! (`~/.config/nyx/devices.json`) knows each one's name and when it was last seen, and
//! Forget goes through every owner.

use crate::backend::state::Monitor;
use crate::backend::store;
use crate::{cmd, paths};
use nyx_devices::{Category, Connected, DeviceId, Index, Owner};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A connected display, as Hyprland reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveMonitor {
    pub connector: String,
    pub description: String,
    pub make: String,
    pub model: String,
    pub serial: String,
}

impl LiveMonitor {
    pub fn from_json(m: &Value) -> Option<Self> {
        let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let connector = s("name");
        (!connector.is_empty()).then(|| LiveMonitor {
            connector,
            description: s("description"),
            make: s("make"),
            model: s("model"),
            serial: s("serial"),
        })
    }

    pub fn id(&self) -> DeviceId {
        DeviceId::monitor(&self.make, &self.model, &self.serial, &self.connector)
    }

    /// Whether it reports who it is (virtual outputs don't).
    pub fn identified(&self) -> bool {
        !self.make.trim().is_empty() || !self.model.trim().is_empty()
    }

    pub fn label(&self) -> String {
        let name = format!("{} {}", self.make.trim(), self.model.trim()).trim().to_string();
        if name.is_empty() { self.connector.clone() } else { name }
    }
}

pub fn live_monitors() -> Vec<LiveMonitor> {
    crate::backend::hypr::json(&["monitors", "all"])
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(LiveMonitor::from_json)
        .collect()
}

/// The key a display's settings are kept under: its identity, unless it has none or
/// another connected one looks just the same (no serial), when it's the port.
pub fn monitor_key(m: &LiveMonitor, all: &[LiveMonitor]) -> String {
    if !m.identified() {
        return m.connector.clone();
    }
    let connected: Vec<Connected> =
        all.iter().map(|o| Connected { id: o.id(), weak: nyx_devices::monitor_is_weak(&o.serial) }).collect();
    if nyx_devices::ambiguous(&connected).contains(&m.id()) {
        return m.connector.clone();
    }
    m.id().key().to_string()
}

/// Whether two connected displays look the same, so each keeps its settings by port.
pub fn ambiguous(m: &LiveMonitor, all: &[LiveMonitor]) -> bool {
    m.identified() && monitor_key(m, all) == m.connector
}

/// Fill in what a display's entry needs to be found again (`desc`, its name).
pub fn describe(entry: &mut Monitor, m: &LiveMonitor, key: &str) {
    if key != m.connector {
        entry.desc = Some(nyx_devices::monitor_match(&m.description));
        entry.label = Some(m.label());
    }
}

/// Entries kept by port from before Settings knew displays by identity move to the
/// display's identity once it's seen connected (and `desc:` rules take over). Returns
/// whether anything moved.
pub fn adopt_monitors(monitors: &mut BTreeMap<String, Monitor>, live: &[LiveMonitor]) -> bool {
    let mut moved = false;
    for m in live {
        let key = monitor_key(m, live);
        if key == m.connector || monitors.contains_key(&key) {
            continue;
        }
        if let Some(mut entry) = monitors.remove(&m.connector) {
            describe(&mut entry, m, &key);
            monitors.insert(key, entry);
            moved = true;
        }
    }
    moved
}

/// Remember a display the user just configured.
pub fn remember_monitor(m: &LiveMonitor, key: &str) {
    if key == m.connector {
        return;
    }
    let mut ix = Index::load_standard();
    ix.remember(m.id(), Category::Monitor, &m.label(), &nyx_devices::monitor_match(&m.description), Some(&m.connector));
    let _ = ix.save();
}

/// Note the connected displays as seen (for "last seen" on remembered ones), and adopt
/// entries kept by port. Off the main thread for the Hyprland call; the store on it.
pub fn sync_monitors() {
    cmd::background(live_monitors, |live| {
        let mut monitors = store::read(|s| s.monitors.clone());
        if adopt_monitors(&mut monitors, &live) {
            store::update(true, move |s| s.monitors = monitors);
        }
        let mut ix = Index::load_standard();
        let seen: Vec<(DeviceId, Option<String>)> = live.iter().map(|m| (m.id(), Some(m.connector.clone()))).collect();
        if ix.seen(&seen) {
            let _ = ix.save();
        }
    });
}

/// Displays with settings that aren't connected now: key, entry, and what the index
/// knows (when it was last seen, on which port).
pub fn remembered_monitors(live: &[LiveMonitor]) -> Vec<(String, Monitor, Option<nyx_devices::Profile>)> {
    let ix = Index::load_standard();
    let connected: Vec<String> = live.iter().map(|m| monitor_key(m, live)).collect();
    store::read(|s| s.monitors.clone())
        .into_iter()
        .filter(|(k, m)| m.desc.is_some() && !connected.contains(k))
        .map(|(k, m)| {
            let p = ix.get(&DeviceId::new(Category::Monitor, &k)).cloned();
            (k, m, p)
        })
        .collect()
}

/// "Last seen 3 days ago on DP-1".
pub fn last_seen(p: Option<&nyx_devices::Profile>) -> String {
    let Some(p) = p else { return "Not connected".into() };
    let ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().saturating_sub(p.last_seen))
        .unwrap_or(0);
    let when = match ago {
        0..=3599 => "less than an hour ago".to_string(),
        3600..=86399 => format!("{} hour{} ago", ago / 3600, if ago / 3600 == 1 { "" } else { "s" }),
        _ => format!("{} day{} ago", ago / 86400, if ago / 86400 == 1 { "" } else { "s" }),
    };
    match &p.connector {
        Some(c) => format!("Last seen {when}, on {c}"),
        None => format!("Last seen {when}"),
    }
}

// ---------- Owners ----------

/// Settings' Hyprland display rules.
struct DisplayRules;

impl Owner for DisplayRules {
    fn name(&self) -> &str {
        "display settings"
    }
    fn holds(&self, id: &DeviceId) -> bool {
        id.category() == Some(Category::Monitor) && store::read(|s| s.monitors.contains_key(id.key()))
    }
    fn forget(&self, id: &DeviceId) -> Result<(), String> {
        let key = id.key().to_string();
        store::update(true, move |s| {
            s.monitors.remove(&key);
        });
        store::flush();
        Ok(())
    }
}

/// The dock's per-screen section (`[monitors."<stable id>"]` in `~/.config/nyx/dock.toml`).
struct DockScreens;

impl DockScreens {
    fn layers() -> nyx_config::Layers {
        nyx_config::Layers::standard()
    }
}

impl Owner for DockScreens {
    fn name(&self) -> &str {
        "the dock"
    }
    fn holds(&self, id: &DeviceId) -> bool {
        id.category() == Some(Category::Monitor)
            && Self::layers()
                .read(nyx_config::Layer::User, "dock")
                .ok()
                .and_then(|t| t.get("monitors").and_then(|m| m.as_table()).map(|m| m.contains_key(id.key())))
                .unwrap_or(false)
    }
    fn forget(&self, id: &DeviceId) -> Result<(), String> {
        let layers = Self::layers();
        let mut monitors = layers
            .read(nyx_config::Layer::User, "dock")
            .map_err(|e| format!("{e:#}"))?
            .get("monitors")
            .and_then(|m| m.as_table())
            .cloned()
            .unwrap_or_default();
        monitors.remove(id.key());
        let r = if monitors.is_empty() {
            layers.reset(nyx_config::Layer::User, "dock", "monitors")
        } else {
            layers.set(nyx_config::Layer::User, "dock", "monitors", toml::Value::Table(monitors))
        };
        r.map_err(|e| format!("{e:#}"))
    }
}

/// Shell Core's per-screen settings (`monitors.screens."<stable id>"`), through its socket.
struct ShellScreens;

impl Owner for ShellScreens {
    fn name(&self) -> &str {
        "Shell Core"
    }
    fn holds(&self, id: &DeviceId) -> bool {
        id.category() == Some(Category::Monitor) && crate::backend::shell_core::screen(id.key()).is_some()
    }
    fn forget(&self, id: &DeviceId) -> Result<(), String> {
        crate::backend::shell_core::forget_screen(id.key())
    }
}

/// Settings' per-mouse rules (`hl.device`).
struct MouseRules;

impl Owner for MouseRules {
    fn name(&self) -> &str {
        "mouse settings"
    }
    fn holds(&self, id: &DeviceId) -> bool {
        id.category() == Some(Category::Mouse) && store::read(|s| s.devices.contains_key(id.key()))
    }
    fn forget(&self, id: &DeviceId) -> Result<(), String> {
        store::reset_device(id.key());
        Ok(())
    }
}

/// Settings' sound device preferences.
struct SoundDevices;

impl Owner for SoundDevices {
    fn name(&self) -> &str {
        "sound settings"
    }
    fn holds(&self, id: &DeviceId) -> bool {
        matches!(id.category(), Some(Category::AudioOutput | Category::AudioInput)) && load_audio().devices.contains_key(id.key())
    }
    fn forget(&self, id: &DeviceId) -> Result<(), String> {
        let mut a = load_audio();
        a.devices.remove(id.key());
        save_audio(&a).map_err(|e| format!("{e:#}"))
    }
}

/// Drop a device's settings everywhere they're kept, then the device.
pub fn forget(id: &DeviceId) -> Result<(), String> {
    let owners: [&dyn Owner; 5] = [&DisplayRules, &DockScreens, &ShellScreens, &MouseRules, &SoundDevices];
    let mut ix = Index::load_standard();
    let result = ix.forget(id, &owners).map_err(|e| e.join("; "));
    let _ = ix.save();
    result
}

// ---------- Mice ----------

/// Remember a mouse given its own settings (once: a slider calls this as it moves).
pub fn remember_mouse(name: &str) {
    let mut ix = Index::load_standard();
    let id = DeviceId::new(Category::Mouse, name);
    if ix.get(&id).is_none() {
        ix.remember(id, Category::Mouse, name, name, None);
        let _ = ix.save();
    }
}

/// Mice with their own settings that aren't connected now.
pub fn remembered_mice(connected: &[String]) -> Vec<(String, Option<nyx_devices::Profile>)> {
    let ix = Index::load_standard();
    store::read(|s| s.devices.keys().cloned().collect::<Vec<_>>())
        .into_iter()
        .filter(|n| !n.contains("touchpad") && !connected.contains(n))
        .map(|n| {
            let p = ix.get(&DeviceId::new(Category::Mouse, &n)).cloned();
            (n, p)
        })
        .collect()
}

// ---------- Sound devices ----------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioDevice {
    /// `output` or `input`.
    pub kind: String,
    /// Its description, for when it's not connected.
    pub name: String,
    /// Used whenever it's connected (a higher priority than the rest).
    pub prefer: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioDevices {
    /// By PipeWire `node.name`.
    pub devices: BTreeMap<String, AudioDevice>,
}

fn audio_file() -> PathBuf {
    paths::app_dir().join("audio-devices.toml")
}

/// WirePlumber reads this when a device appears, so a preference applies on reconnection
/// without Settings running.
pub fn wireplumber_file() -> PathBuf {
    paths::config_home().join("wireplumber/wireplumber.conf.d/51-nyx-settings.conf")
}

pub fn load_audio() -> AudioDevices {
    std::fs::read_to_string(audio_file()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_audio(a: &AudioDevices) -> anyhow::Result<()> {
    cmd::atomic_write(&audio_file(), &toml::to_string_pretty(a)?)?;
    let rules = wireplumber_rules(a);
    let path = wireplumber_file();
    if rules.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    cmd::atomic_write(&path, &rules)
}

/// Preferred devices get a session priority above any built-in or USB device's default.
pub fn wireplumber_rules(a: &AudioDevices) -> String {
    let preferred: Vec<&String> = a.devices.iter().filter(|(_, d)| d.prefer).map(|(n, _)| n).collect();
    if preferred.is_empty() {
        return String::new();
    }
    let matches: Vec<String> = preferred.iter().map(|n| format!("{{ node.name = \"{}\" }}", n.replace('"', ""))).collect();
    let rule = format!(
        "  {{\n    matches = [ {} ]\n    actions = {{ update-props = {{ priority.session = 3000, priority.driver = 3000 }} }}\n  }}\n",
        matches.join(", ")
    );
    format!(
        "# Written by Nyx Settings (Sound): the devices to use whenever they're connected.\n\
         monitor.alsa.rules = [\n{rule}]\n\
         monitor.bluez.rules = [\n{rule}]\n"
    )
}

pub fn set_prefer(node: &str, kind: &str, description: &str, prefer: bool) -> anyhow::Result<()> {
    let mut a = load_audio();
    let d = a.devices.entry(node.to_string()).or_default();
    d.kind = kind.to_string();
    if !description.is_empty() {
        d.name = description.to_string();
    }
    d.prefer = prefer;
    if !prefer {
        a.devices.remove(node);
    }
    save_audio(&a)?;
    let category = if kind == "input" { Category::AudioInput } else { Category::AudioOutput };
    let mut ix = Index::load_standard();
    let id = DeviceId::new(category, node);
    if prefer {
        ix.remember(id, category, description, node, None);
    } else {
        let _ = ix.forget(&id, &[]);
    }
    let _ = ix.save();
    Ok(())
}

/// Sound devices with settings that aren't connected now.
pub fn remembered_audio(kind: &str, connected: &[String]) -> Vec<(String, AudioDevice, Option<nyx_devices::Profile>)> {
    let ix = Index::load_standard();
    let category = if kind == "input" { Category::AudioInput } else { Category::AudioOutput };
    load_audio()
        .devices
        .into_iter()
        .filter(|(n, d)| d.kind == kind && !connected.contains(n))
        .map(|(n, d)| {
            let p = ix.get(&DeviceId::new(category, &n)).cloned();
            (n, d, p)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(connector: &str, make: &str, model: &str, serial: &str) -> LiveMonitor {
        LiveMonitor {
            connector: connector.into(),
            description: format!("{make} {model} {serial}").trim().to_string(),
            make: make.into(),
            model: model.into(),
            serial: serial.into(),
        }
    }

    #[test]
    fn displays_are_kept_by_identity_except_when_they_cannot_be_told_apart() {
        let dell = mon("DP-1", "Dell Inc.", "U2723QE", "ABC");
        let a = mon("DP-2", "Acme", "M24", "");
        let b = mon("HDMI-A-1", "Acme", "M24", "");
        let headless = mon("HEADLESS-1", "", "", "");
        let all = vec![dell.clone(), a.clone(), b.clone(), headless.clone()];
        assert_eq!(monitor_key(&dell, &all), "Dell Inc.|U2723QE|ABC");
        assert_eq!(monitor_key(&a, &all), "DP-2");
        assert_eq!(monitor_key(&b, &all), "HDMI-A-1");
        assert!(ambiguous(&a, &all));
        assert_eq!(monitor_key(&headless, &all), "HEADLESS-1");
        assert!(!ambiguous(&headless, &all));
        assert_eq!(monitor_key(&a, &[a.clone()]), "Acme|M24|", "one of them alone is fine");
    }

    #[test]
    fn a_port_entry_moves_to_the_display_seen_on_it() {
        let panel = LiveMonitor {
            connector: "eDP-2".into(),
            description: "Samsung Display Corp. ATNA60DL01-0".into(),
            make: "Samsung Display Corp.".into(),
            model: "ATNA60DL01-0 ".into(),
            serial: "".into(),
        };
        let mut monitors = BTreeMap::new();
        monitors.insert("eDP-2".to_string(), Monitor { scale: Some(1.6), ..Default::default() });
        monitors.insert("DP-9".to_string(), Monitor { scale: Some(2.0), ..Default::default() });
        assert!(adopt_monitors(&mut monitors, std::slice::from_ref(&panel)));
        let m = &monitors["Samsung Display Corp.|ATNA60DL01-0|"];
        assert_eq!(m.scale, Some(1.6));
        assert_eq!(m.desc.as_deref(), Some("desc:Samsung Display Corp. ATNA60DL01-0"));
        assert_eq!(m.output("ignored"), "desc:Samsung Display Corp. ATNA60DL01-0");
        assert!(monitors.contains_key("DP-9"), "a port with nothing seen on it keeps its entry");
        assert!(!adopt_monitors(&mut monitors, &[panel]), "only once");
    }

    #[test]
    fn preferred_sound_devices_become_one_wireplumber_rule() {
        let mut a = AudioDevices::default();
        assert_eq!(wireplumber_rules(&a), "");
        a.devices.insert(
            "alsa_output.usb-Focusrite-00.analog-stereo".into(),
            AudioDevice { kind: "output".into(), name: "Scarlett".into(), prefer: true },
        );
        a.devices.insert(
            "bluez_output.AA_BB.1".into(),
            AudioDevice { kind: "output".into(), name: "Headphones".into(), prefer: false },
        );
        let r = wireplumber_rules(&a);
        assert!(r.contains("node.name = \"alsa_output.usb-Focusrite-00.analog-stereo\""));
        assert!(!r.contains("bluez_output.AA_BB.1"));
        assert!(r.contains("monitor.alsa.rules") && r.contains("monitor.bluez.rules"));
    }
}
