//! Connected Devices: phones, tablets and computers paired through Flux (Nyx Connect).
//!
//! fluxd does the pairing, the connection and the phone features; this page drives its
//! pairing (always with the key the user compared), shows each device's state and what it
//! offers, and edits what each may do (fluxd's per-device access). Devices that are
//! paired but away are under Remembered Devices, where their access can still change.
//! Unpair removes the pairing; Forget also drops this computer's choices for the device.

use crate::backend::connect as flux;
use crate::widgets::{self, Page};
use crate::{capability, cmd, platform, window};
use gtk::glib;
use gtk::prelude::*;
use nyx_connect::{Access, Client, Device, Feature, PairStep, Prefs, Snapshot, Status};

pub fn build(page: &Page) {
    flux::follow();
    let Some(snap) = flux::snapshot() else {
        unavailable(page);
        return;
    };
    general(page, &snap);
    devices(page, &snap);
    remembered(page, &snap);
    diagnostics(page, &snap);
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

/// Run `work` against fluxd off the main thread; say what went wrong.
fn act<T: Send + 'static>(
    work: impl FnOnce(&Client) -> Result<T, nyx_connect::Error> + Send + 'static,
    done: impl FnOnce(T) + 'static,
) {
    cmd::background(
        move || work(&Client::new()),
        move |r| match r {
            Ok(v) => done(v),
            Err(e) => window::toast(&e.message),
        },
    );
}

fn unavailable(page: &Page) {
    let g = page.group("");
    widgets::keywords("flux phone tablet pair connect");
    let Some(why) = flux::away() else {
        g.add(&widgets::row("Looking for Flux…", "", None));
        return;
    };
    if capability::state("daemon:flux") == nyx_capability::State::Inactive && platform::connect_on_trusted_networks() && !paired_before() {
        let (r, _) = widgets::button_row(
            "Pair a phone",
            "Nyx Connect starts once a phone is paired. Start pairing, open Flux on the phone, and compare the key on both screens.",
            "Start pairing",
            |b| {
                b.set_sensitive(false);
                start_pairing();
            },
        );
        g.add(&r);
        trust_row(&g);
    } else if capability::state("daemon:flux") == nyx_capability::State::Inactive {
        let (r, _) = widgets::button_row(
            "Flux isn't running",
            "Flux connects this computer to your phone. Start it to pair and use your devices.",
            "Start Flux",
            |b| {
                b.set_sensitive(false);
                cmd::run_async(&["systemctl", "--user", "start", "fluxd.service"], |r| {
                    if let Err(e) = r {
                        window::toast(&format!("Couldn't start Flux: {e}"));
                    }
                });
            },
        );
        g.add(&r);
    } else {
        g.add(&widgets::row("Flux isn't reachable", &esc(&why), None));
    }
}

/// Whether a phone was ever paired here (fluxd's list of paired devices exists).
fn paired_before() -> bool {
    gtk::glib::user_data_dir().join("flux/devices.json").exists()
}

/// fluxd waits for a pairing on NyxOS: say one is starting, then start it.
fn start_pairing() {
    let flag = gtk::glib::user_runtime_dir().join("nyx-connect/pairing");
    if let Err(e) = flag.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&flag, "")) {
        window::toast(&format!("Couldn't start pairing: {e}"));
        return;
    }
    cmd::run_async(&["systemctl", "--user", "start", "fluxd.service"], |r| {
        if let Err(e) = r {
            window::toast(&format!("Couldn't start Flux: {e}"));
        }
    });
}

/// The network phones would reach this computer on: the connected Wi-Fi or wired
/// connection, and whether it's trusted (NetworkManager's home, work or trusted zone).
struct Network {
    uuid: String,
    device: String,
    name: String,
    trusted: bool,
}

fn network() -> Option<Network> {
    let status = cmd::output(&["nmcli", "-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"])?;
    let line = status.lines().find(|l| {
        let f: Vec<&str> = l.splitn(4, ':').collect();
        f.len() == 4 && matches!(f[1], "wifi" | "ethernet") && f[2] == "connected"
    })?;
    let f: Vec<&str> = line.splitn(4, ':').collect();
    let device = f[0].to_string();
    let uuid = cmd::output(&["nmcli", "-g", "GENERAL.CON-UUID", "device", "show", &device])?.trim().to_string();
    let zone = cmd::output(&["nmcli", "-g", "connection.zone", "connection", "show", &uuid]).unwrap_or_default();
    Some(Network { uuid, device, name: f[3].replace("\\:", ":"), trusted: matches!(zone.trim(), "home" | "work" | "trusted") })
}

/// On NyxOS, phones reach Nyx Connect only on trusted networks: this one, trusted or not.
fn trust_row(g: &widgets::Group) {
    if !platform::connect_on_trusted_networks() {
        return;
    }
    let Some(net) = network() else {
        g.note("Connect to your home or work network to pair a phone.");
        return;
    };
    let (r, _) = widgets::switch_row(
        &format!("Trust {}", esc(&net.name)),
        "Phones reach this computer only on trusted networks. Trust your home or work network, not a café's or hotel's.",
        net.trusted,
        move |on| {
            let device = net.device.clone();
            let zone = if on { "home" } else { "" };
            cmd::run_async(&["nmcli", "connection", "modify", &net.uuid, "connection.zone", zone], move |r| match r {
                // Reapplying it lets the firewall open or close the port now.
                Ok(_) => cmd::run_async(&["nmcli", "device", "reapply", &device], |r| {
                    if let Err(e) = r {
                        window::toast(&format!("Couldn't apply it to the network: {e}"));
                    }
                }),
                Err(e) => window::toast(&format!("Couldn't change the network: {e}")),
            });
        },
    );
    g.add(&r);
}

// ----- What every device shares -----

fn general(page: &Page, s: &Snapshot) {
    let g = page.group("");
    widgets::keywords("flux phone clipboard sync copy paste notifications browse files home folder");
    if capability::program("nyx-clipboardd") {
        let on = Prefs::load().clipboard_sync;
        let (r, _) = widgets::switch_row(
            "Sync the clipboard",
            "What you copy goes to your devices after Nyx Clipboard's filters: copies from excluded apps, passwords and secrets stay here. What you copy on them lands in your clipboard history.",
            on,
            |on| {
                cmd::background(
                    move || nyx_connect::set_clipboard_sync(&Client::new(), on),
                    |r| {
                        if let Err(e) = r {
                            window::toast(&format!("Couldn't change clipboard sync: {e}"));
                        }
                    },
                );
            },
        );
        g.add(&r);
    } else {
        let (r, _) = widgets::switch_row(
            "Sync the clipboard",
            "Flux sends what you copy to your devices, and puts what you copy on them on this computer's clipboard.",
            s.settings.auto_clipboard,
            |on| act(move |c| c.set("autoClipboard", on.into()), |_| {}),
        );
        g.add(&r);
    }
    let (r, _) = widgets::switch_row(
        "Show notifications from devices",
        "Their notifications appear here, in the Notification Center.",
        s.settings.notifications,
        |on| act(move |c| c.set("notifications", on.into()), |_| {}),
    );
    g.add(&r);
    let (r, _) = widgets::switch_row(
        "Let devices browse this computer",
        "A paired device can open and copy any file in your home folder while this is on. Each device can be kept out below.",
        s.settings.share_home,
        |on| act(move |c| c.set("shareHome", on.into()), |_| {}),
    );
    g.add(&r);
    let (r, _) = widgets::info_row("Files from devices are saved in", &s.settings.download_dir);
    g.add(&r);
}

// ----- Devices here now -----

fn devices(page: &Page, s: &Snapshot) {
    let g = page.group("Devices");
    widgets::keywords("pair new phone tablet android iphone flux verification key accept reject unpair forget ring");
    let (r, _) = widgets::button_row(
        "Pair a new device",
        "Open Flux on the phone, on the same network. It shows up here; then compare the key on both screens.",
        "Look for devices",
        |b| {
            b.set_sensitive(false);
            let b = b.clone();
            act(|c| c.discover(), move |_| b.set_sensitive(true));
        },
    );
    g.add(&r);
    trust_row(&g);
    let here: Vec<&Device> = s.devices.iter().filter(|d| d.status() != Status::Disconnected).collect();
    if here.is_empty() {
        g.add(&widgets::row("No devices nearby", "Paired devices show here while they're connected.", None));
    }
    for d in here {
        let name = d.display_name(&s.devices);
        match d.status() {
            Status::Discovered => g.add(&discovered(d, &name)),
            Status::Pairing(step) => g.add(&pairing(d, &name, step)),
            Status::Connected => g.add(&connected(s, d, &name)),
            Status::Disconnected => {}
        }
    }
}

fn discovered(d: &Device, name: &str) -> gtk::Box {
    let id = d.id.clone();
    let pair = gtk::Button::with_label("Pair");
    pair.connect_clicked(move |b| {
        b.set_sensitive(false);
        b.set_label("Asking…");
        let id = id.clone();
        act(
            move |c| c.pair_request(&id),
            |p| window::toast(&format!("Check that {} shows {}", p.name, nyx_connect::group_key(&p.key))),
        );
    });
    let desc = if d.old_app {
        "Its Flux app is too old to pair. Update Flux on it first.".to_string()
    } else {
        "Nearby, not paired".into()
    };
    if d.old_app {
        pair.set_sensitive(false);
    }
    widgets::row(&esc(name), &desc, Some(pair.upcast_ref()))
}

fn pairing(d: &Device, name: &str, step: PairStep) -> gtk::Box {
    let key = d.key_groups();
    let controls = widgets::hbox(8);
    let (id, raw) = (d.id.clone(), d.pair_key.clone());
    let desc = match step {
        PairStep::Requested => {
            let cancel = gtk::Button::with_label("Cancel");
            cancel.connect_clicked(move |b| {
                b.set_sensitive(false);
                let id = id.clone();
                act(move |c| c.pair_reject(&id, None), |_| {});
            });
            controls.append(&cancel);
            format!("Accept on the device if it shows <b>{key}</b>.")
        }
        PairStep::Confirm | PairStep::Incoming => {
            let accept = gtk::Button::with_label(if step == PairStep::Confirm { "Confirm" } else { "Accept" });
            accept.add_css_class("suggested-action");
            let (id2, raw2) = (id.clone(), raw.clone());
            accept.connect_clicked(move |b| {
                b.set_sensitive(false);
                let (id, raw) = (id2.clone(), raw2.clone());
                act(
                    move |c| c.pair_accept(&id, &raw),
                    move |p| {
                        flux::remember(&p.device);
                        window::toast(&format!("Paired with {}", p.name));
                    },
                );
            });
            let reject = gtk::Button::with_label("Reject");
            reject.connect_clicked(move |b| {
                b.set_sensitive(false);
                let (id, raw) = (id.clone(), raw.clone());
                act(move |c| c.pair_reject(&id, Some(&raw)), |_| {});
            });
            controls.append(&reject);
            controls.append(&accept);
            match step {
                PairStep::Confirm => format!("It accepted. Confirm if it shows <b>{key}</b>."),
                _ => format!("Wants to pair. Accept only if it shows <b>{key}</b>."),
            }
        }
    };
    widgets::row(&esc(name), &format!("{desc} Pairing ends after 30 seconds."), Some(controls.upcast_ref()))
}

fn describe(d: &Device) -> String {
    let mut parts = vec![if d.online { "Connected".to_string() } else { "Not connected".to_string() }];
    if let Some(b) = d.battery.filter(|_| d.online) {
        parts.push(format!("{}% battery{}", b.charge, if b.charging { ", charging" } else { "" }));
    }
    let app = match d.app.as_str() {
        "android" | "android-debug" => "Flux for Android",
        "ios" => "Flux for iOS",
        "macos" => "Flux for macOS",
        "fluxd" => "Flux",
        _ => "",
    };
    if !app.is_empty() {
        parts.push(format!("{app} {}", d.app_version).trim().to_string());
    }
    if !d.app_update.is_empty() {
        parts.push(format!("Flux {} is available for it", d.app_update));
    }
    esc(&parts.join(" · "))
}

fn connected(s: &Snapshot, d: &Device, name: &str) -> gtk::Box {
    let body = widgets::vbox(8);
    let features: Vec<&str> = d.features().iter().map(|f| f.label()).collect();
    if !features.is_empty() {
        let l = widgets::label(&format!("Offers: {}", features.join(", ")), "dim");
        l.set_xalign(0.0);
        l.set_wrap(true);
        body.append(&l);
    }
    for r in access_rows(s, d) {
        body.append(&r);
    }
    let buttons = widgets::hbox(8);
    if d.has(Feature::Ring) {
        let id = d.id.clone();
        let ring = gtk::Button::with_label("Ring");
        ring.set_tooltip_text(Some("Ring it, to find it"));
        ring.connect_clicked(move |_| {
            let id = id.clone();
            act(move |c| c.ring(&id), |_| {});
        });
        buttons.append(&ring);
    }
    unpair_forget(&buttons, s, d);
    body.append(&buttons);
    widgets::stacked_row(&esc(name), &describe(d), body.upcast_ref())
}

fn access_rows(s: &Snapshot, d: &Device) -> Vec<gtk::Box> {
    flux::accesses(s, d)
        .into_iter()
        .map(|access| {
            let desc = match access {
                Access::Clipboard => "Clipboard sync, and clips sent from it by hand.",
                Access::Notifications => "Its notifications appear here.",
                Access::ShareHome => "It can open and copy files in your home folder.",
                Access::RemoteInput => "It can move the pointer and type here.",
                Access::RemoteDesktop => "It can show this screen.",
            };
            let id = d.id.clone();
            let (r, _) = widgets::switch_row(access.label(), desc, s.allowed(&d.id, access), move |on| {
                let id = id.clone();
                let id2 = id.clone();
                act(move |c| c.set_access(&id, access, Some(on)), move |_| flux::remember(&id2));
            });
            r
        })
        .collect()
}

fn unpair_forget(buttons: &gtk::Box, s: &Snapshot, d: &Device) {
    let id = d.id.clone();
    let unpair = widgets::confirm_button("Unpair", "Click again to unpair", move |b| {
        b.set_sensitive(false);
        let id = id.clone();
        act(
            move |c| c.unpair(&id),
            |p| window::toast(&format!("Unpaired {}. Its settings here stay for when it pairs again.", p.name)),
        );
    });
    unpair.set_tooltip_text(Some("Remove the pairing. It has to be paired again to connect."));
    buttons.append(&unpair);
    buttons.append(&forget_button(s, &d.id));
}

fn forget_button(s: &Snapshot, id: &str) -> gtk::Button {
    let (id, snap) = (id.to_string(), s.clone());
    let forget = widgets::confirm_button("Forget", "Click again to forget", move |b| {
        b.set_sensitive(false);
        let (id, snap) = (id.clone(), snap.clone());
        cmd::background(
            move || {
                let mut index = nyx_devices::Index::load_standard();
                let r = nyx_connect::forget(&Client::new(), &snap, &mut index, &id);
                let _ = index.save();
                r
            },
            |r| match r {
                Ok(()) => window::toast("Device forgotten"),
                Err(e) => window::toast(&format!("Couldn't forget all of it: {e}")),
            },
        );
    });
    forget.set_tooltip_text(Some("Unpair it and drop what this computer keeps for it: its access choices and its place here."));
    forget
}

// ----- Paired, but away -----

fn remembered(page: &Page, s: &Snapshot) {
    let away: Vec<&Device> = s.paired().filter(|d| !d.online).collect();
    let index = nyx_devices::Index::load_standard();
    let orphans = nyx_connect::orphans(&index, s);
    if away.is_empty() && orphans.is_empty() {
        return;
    }
    let g = page.collapsible("Remembered devices", false);
    g.note("Paired devices that aren't connected now. Their access can change here; it applies when they're back.");
    widgets::keywords("remembered offline away disconnected phone forget unpair");
    for d in away {
        let body = widgets::vbox(8);
        for r in access_rows(s, d) {
            body.append(&r);
        }
        let buttons = widgets::hbox(8);
        unpair_forget(&buttons, s, d);
        body.append(&buttons);
        let profile = index.get(&nyx_connect::device_id(&d.id));
        let seen = match (profile, d.last_seen) {
            (Some(p), _) => crate::backend::devices::last_seen(Some(p)),
            (None, t) if t > 0 => crate::backend::devices::last_seen(Some(&nyx_devices::Profile {
                category: nyx_devices::Category::Linked,
                name: d.name.clone(),
                matches: String::new(),
                connector: None,
                last_seen: t as u64,
            })),
            _ => "Not connected".into(),
        };
        g.add(&widgets::stacked_row(&esc(&d.display_name(&s.devices)), &esc(&seen), body.upcast_ref()));
    }
    for (id, profile) in orphans {
        let buttons = widgets::hbox(8);
        buttons.append(&forget_button(s, id.key()));
        g.add(&widgets::stacked_row(
            &esc(&profile.name),
            "No longer paired: it was unpaired on the device, or Flux forgot it. Pair it again above, or forget it.",
            buttons.upcast_ref(),
        ));
    }
}

// ----- What's going on -----

fn diagnostics(page: &Page, s: &Snapshot) {
    let g = page.collapsible("Diagnostics", false);
    widgets::keywords("flux version port status debug clipboard sync");
    let version = if s.this.version.is_empty() { "earlier than 0.8".to_string() } else { s.this.version.clone() };
    let rows = [
        ("Flux", version),
        ("This computer", format!("{} ({})", s.this.name, s.this.kind)),
        ("Port", s.this.tcp_port.to_string()),
        ("Socket", nyx_connect::socket_path().display().to_string()),
        ("Paired devices", format!("{} ({} connected)", s.paired().count(), s.paired().filter(|d| d.online).count())),
        ("Files on their way", s.transfers.iter().filter(|t| !t.finished()).count().to_string()),
        (
            "Clipboard sync",
            match (Prefs::load().clipboard_sync, s.settings.auto_clipboard) {
                (true, false) => "Nyx Clipboard".to_string(),
                (true, true) => "Nyx Clipboard (Flux's own is still on)".to_string(),
                (false, true) => "Flux".to_string(),
                (false, false) => "Off".to_string(),
            },
        ),
    ];
    for (k, v) in rows {
        let (r, _) = widgets::info_row(k, &v);
        g.add(&r);
    }
}
