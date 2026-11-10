//! Nyx Connect in Settings: fluxd's state, followed while Settings runs, for the
//! Connected Devices page (sections/connect.rs). The page redraws when what it shows
//! changes (a device comes or goes, a pairing moves on, an access switch changes).

use nyx_connect::{Access, Event, Snapshot, Watch};
use nyx_devices::Index;
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) };
    static LAST: RefCell<Option<Arc<Snapshot>>> = const { RefCell::new(None) };
    /// Why fluxd can't be reached, once the watch has said so.
    static AWAY: RefCell<Option<String>> = const { RefCell::new(None) };
    static SHOWN: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Start following fluxd (once).
pub fn follow() {
    if WATCH.with(|w| w.borrow().is_some()) {
        return;
    }
    let (tx, rx) = async_channel::unbounded::<Event>();
    let watch = Watch::start(move |e| {
        let _ = tx.send_blocking(e);
    });
    WATCH.with(|w| *w.borrow_mut() = Some(watch));
    gtk::glib::spawn_future_local(async move {
        while let Ok(event) = rx.recv().await {
            match event {
                Event::State(snap) => {
                    let mut index = Index::load_standard();
                    if nyx_connect::seen(&mut index, &snap) {
                        let _ = index.save();
                    }
                    AWAY.with(|a| *a.borrow_mut() = None);
                    LAST.with(|l| *l.borrow_mut() = Some(snap.clone()));
                    redraw(&signature(&snap));
                }
                Event::Unavailable(e) => {
                    AWAY.with(|a| *a.borrow_mut() = Some(e.message.clone()));
                    LAST.with(|l| *l.borrow_mut() = None);
                    redraw("away");
                }
                Event::Toast(_) => {}
            }
        }
    });
}

fn redraw(signature: &str) {
    let changed = SHOWN.with(|s| {
        let mut s = s.borrow_mut();
        let changed = *s != signature;
        *s = signature.to_string();
        changed
    });
    if changed {
        crate::window::rebuild_if_built("devices");
    }
}

/// What the page shows, apart from figures that change all the time (battery).
fn signature(s: &Snapshot) -> String {
    let mut out = format!(
        "{}|{}|{}|{}|{}|{:?}|",
        s.this.version,
        s.settings.notifications,
        s.settings.share_home,
        s.settings.auto_clipboard,
        s.settings.download_dir,
        s.settings.device_rules
    );
    for d in &s.devices {
        out.push_str(&format!("{}:{}:{:?}:{}:{:?};", d.id, d.name, d.status(), d.pair_key, d.plugins));
    }
    out.push_str(&nyx_connect::Prefs::load().clipboard_sync.to_string());
    out
}

pub fn snapshot() -> Option<Arc<Snapshot>> {
    LAST.with(|l| l.borrow().clone())
}

/// Why fluxd can't be reached; `None` while it's there or not asked yet.
pub fn away() -> Option<String> {
    AWAY.with(|a| a.borrow().clone())
}

/// The user configured this device: list it under Remembered Devices while it's away.
pub fn remember(id: &str) {
    let Some(snap) = snapshot() else { return };
    let Some(device) = snap.device(id) else { return };
    let mut index = Index::load_standard();
    nyx_connect::remember(&mut index, device);
    let _ = index.save();
}

/// The accesses a device's switches show: those it has a feature for, and whose
/// global switch is on (a device can't be let in past it).
pub fn accesses(s: &Snapshot, d: &nyx_connect::Device) -> Vec<Access> {
    use nyx_connect::Feature;
    let mut out = Vec::new();
    if d.has(Feature::Clipboard) {
        out.push(Access::Clipboard);
    }
    if d.has(Feature::Notifications) && s.settings.notifications {
        out.push(Access::Notifications);
    }
    if s.settings.share_home {
        out.push(Access::ShareHome);
    }
    if s.settings.remote_input {
        out.push(Access::RemoteInput);
    }
    if s.settings.remote_desktop {
        out.push(Access::RemoteDesktop);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(json: serde_json::Value) -> Snapshot {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn battery_changes_dont_redraw_but_pairing_does() {
        let a = snap(
            serde_json::json!({ "devices": [{ "id": "p", "name": "Pandora", "paired": true, "online": true, "battery": { "charge": 50 } }] }),
        );
        let b = snap(
            serde_json::json!({ "devices": [{ "id": "p", "name": "Pandora", "paired": true, "online": true, "battery": { "charge": 49 } }] }),
        );
        assert_eq!(signature(&a), signature(&b));
        let c = snap(serde_json::json!({ "devices": [{ "id": "p", "name": "Pandora", "paired": true, "online": false }] }));
        assert_ne!(signature(&a), signature(&c));
    }

    #[test]
    fn switches_follow_features_and_global_switches() {
        let s = snap(serde_json::json!({
            "devices": [{ "id": "p", "name": "Pandora", "paired": true, "plugins": ["clipboard", "notifications"] }],
            "settings": { "notifications": true, "shareHome": false }
        }));
        assert_eq!(accesses(&s, &s.devices[0]), vec![Access::Clipboard, Access::Notifications]);
        let tv = snap(
            serde_json::json!({ "devices": [{ "id": "t", "name": "TV", "paired": true, "plugins": [] }], "settings": { "shareHome": true } }),
        );
        assert_eq!(accesses(&tv, &tv.devices[0]), vec![Access::ShareHome]);
    }
}
