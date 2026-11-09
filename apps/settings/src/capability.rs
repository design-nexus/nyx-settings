//! What this computer can do, for deciding which pages and controls show.
//!
//! One [`Registry`] for the app. Programs, hardware and Shell Core are probed before the
//! window is built (they're file checks), the rest in the background; after that the
//! registry is told by inotify, udev, D-Bus and PipeWire when something changes, and
//! the sidebar and bound controls follow.
//!
//! A page or control is shown when what it needs is available or can be started from
//! it, shown but disabled (with the reason) when it's temporarily unavailable or not
//! allowed, and hidden when the software isn't installed or the hardware isn't there.

use gtk::glib;
use gtk::prelude::*;
use nyx_capability::providers::{Audio, BusNames, Fingerprint, Hardware, Programs, Services, ShellCore};
use nyx_capability::{Change, Provider, Registry, State};
use std::cell::RefCell;
use std::sync::OnceLock;

/// Programs whose presence a page or row depends on.
const PROGRAMS: &[&str] = &[
    "nyx-dock",
    "nyx-switcher",
    "nyx-control-center",
    "nyx-notification-center",
    "nyx-ai",
    "nyx-assistant",
    "nyx-authctl",
    "nyx-shellctl",
    "nyx-system",
    "nyx",
    "hyprsunset",
    "lpstat",
    "bluetoothctl",
    "nmcli",
    "pactl",
    "powerprofilesctl",
    "brightnessctl",
    "tailscale",
    "nordvpn",
    "omarchy-version",
    "omarchy-shell",
    "omarchy-bar",
    "omarchy-plugin-list",
    "omarchy-bluetooth-power",
    "omarchy-bluetooth-device",
    "omarchy-brightness-display",
    "omarchy-toggle-nightlight",
    "omarchy-restart-hyprsunset",
    "omarchy-dns",
    "omarchy-network-band",
    "omarchy-network-speedtest",
    "omarchy-toggle-touchpad",
    "omarchy-toggle-touchscreen",
    "omarchy-hw-touchpad",
    "omarchy-hw-touchscreen",
    "omarchy-capture-screenshot",
    "omarchy-capture-screenrecording",
];

/// D-Bus services: `(system bus, name)`.
const BUS: &[(bool, &str)] = &[
    (false, "io.github.design_nexus.nyx.Notifications"),
    (false, "io.github.design_nexus.nyx.Theme"),
    (true, "io.github.nyxos_project.System"),
    (true, "org.bluez"),
    (true, "org.freedesktop.UPower"),
    (true, "net.hadess.PowerProfiles"),
    (true, "org.freedesktop.UDisks2"),
];

const UNITS: &[(bool, &str)] = &[(false, "nyx-notifyd"), (false, "nyx-shell-core"), (true, "bluetooth"), (true, "cups")];

static REGISTRY: OnceLock<Registry> = OnceLock::new();

pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| {
        let providers: Vec<Box<dyn Provider>> = vec![
            Box::new(Programs::new(PROGRAMS.iter().copied())),
            Box::new(Hardware::default()),
            Box::new(ShellCore),
            Box::new(BusNames::new(BUS.iter().copied())),
            Box::new(Services::new(UNITS.iter().copied())),
            Box::new(Audio),
            Box::new(Fingerprint),
        ];
        Registry::new(providers)
    })
}

/// Probe what the sidebar needs now, then everything in the background and keep
/// watching. Changes arrive on the main loop.
pub fn start() {
    let r = registry();
    for quick in ["programs", "hardware", "shell-core"] {
        r.refresh(quick);
    }
    let bg = r.clone();
    std::thread::Builder::new().name("capabilities".into()).spawn(move || bg.start()).ok();
    r.subscribe_local(changed);
}

pub fn state(id: &str) -> State {
    // Before a provider has answered, anything not known yet counts as there, so pages
    // don't flash hidden and back.
    match registry().get(id) {
        Some(c) => c.state,
        None => match id.strip_prefix("program:") {
            Some(name) if Programs::find(name).is_some() => State::Available,
            Some(_) => State::NotInstalled,
            None if id.starts_with("hw:") => State::NotInstalled,
            None => State::Available,
        },
    }
}

pub fn reason(id: &str) -> Option<String> {
    registry().get(id).and_then(|c| c.reason)
}

/// Whether the program is installed (on `PATH` or a NyxOS profile).
pub fn program(name: &str) -> bool {
    state(&format!("program:{name}")).is_available()
}

/// How a page or control shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visibility {
    Shown,
    /// Shown, but it can't do anything right now: why.
    Disabled(String),
    Hidden,
}

/// Everything in `needs` together: hidden if any is missing, disabled if any can't be
/// used right now, shown otherwise.
pub fn visibility(needs: &[&str]) -> Visibility {
    let mut disabled = None;
    for id in needs {
        match state(id) {
            State::Available | State::Inactive => {}
            State::TemporarilyUnavailable | State::PermissionRestricted => {
                disabled.get_or_insert_with(|| reason(id).unwrap_or_else(|| "Not available right now".into()));
            }
            State::NotInstalled | State::Unsupported => return Visibility::Hidden,
        }
    }
    disabled.map(Visibility::Disabled).unwrap_or(Visibility::Shown)
}

/// Why `needs` makes something hidden: the first missing capability's reason.
pub fn why_hidden(needs: &[&str]) -> Option<String> {
    needs.iter().find(|id| !state(id).exists()).and_then(|id| reason(id))
}

/// A control that follows a capability, what it needs, and its own tooltip.
type Bound = (glib::WeakRef<gtk::Widget>, &'static [&'static str], Option<String>);

thread_local! {
    /// Controls that follow a capability: shown, made insensitive with the reason, or hidden.
    static BOUND: RefCell<Vec<Bound>> = const { RefCell::new(Vec::new()) };
}

fn apply(w: &gtk::Widget, needs: &[&str], tooltip: Option<&str>) {
    match visibility(needs) {
        Visibility::Shown => {
            w.set_visible(true);
            w.set_sensitive(true);
            w.set_tooltip_text(tooltip);
        }
        Visibility::Disabled(why) => {
            w.set_visible(true);
            w.set_sensitive(false);
            w.set_tooltip_text(Some(&why));
        }
        Visibility::Hidden => w.set_visible(false),
    }
}

/// Show `widget` only when everything in `needs` is there, and keep it so as that
/// changes.
pub fn bind(widget: &impl IsA<gtk::Widget>, needs: &'static [&'static str]) {
    let w = widget.upcast_ref::<gtk::Widget>();
    let tooltip = w.tooltip_text().map(|t| t.to_string());
    apply(w, needs, tooltip.as_deref());
    BOUND.with(|b| b.borrow_mut().push((w.downgrade(), needs, tooltip)));
}

thread_local! {
    static REBUILD_ON: RefCell<Vec<(&'static [&'static str], &'static str)>> = const { RefCell::new(Vec::new()) };
}

/// Redraw `section` (if it's been built) when one of `caps` changes: for a page whose
/// rows depend on hardware coming and going (a fingerprint reader, a keyboard light).
pub fn rebuild_on(caps: &'static [&'static str], section: &'static str) {
    REBUILD_ON.with(|r| {
        let mut r = r.borrow_mut();
        if !r.iter().any(|(c, s)| *s == section && std::ptr::eq(*c, caps)) {
            r.push((caps, section));
        }
    });
}

fn changed(changes: &[Change]) {
    let redraw: Vec<&'static str> = REBUILD_ON.with(|r| {
        r.borrow().iter().filter(|(caps, _)| changes.iter().any(|c| caps.contains(&c.id.as_str()))).map(|(_, s)| *s).collect()
    });
    for s in redraw {
        crate::window::rebuild_if_built(s);
    }
    BOUND.with(|b| {
        let mut b = b.borrow_mut();
        b.retain(|(w, _, _)| w.upgrade().is_some());
        for (w, needs, tooltip) in b.iter() {
            if changes.iter().any(|c| needs.contains(&c.id.as_str()))
                && let Some(w) = w.upgrade()
            {
                apply(&w, needs, tooltip.as_deref());
            }
        }
    });
    crate::window::capabilities_changed(changes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_programs_and_hardware_are_missing_but_services_are_assumed() {
        assert_eq!(visibility(&["program:surely-not-a-program"]), Visibility::Hidden);
        assert_eq!(visibility(&[]), Visibility::Shown);
        assert_eq!(visibility(&["bus:session:not.asked.yet"]), Visibility::Shown);
    }
}
