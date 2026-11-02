use crate::backend::{bt, net};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;

pub fn build_wifi(page: &Page) {
    if cmd::present("nmcli") {
        wifi(page);
    } else {
        page.banner("Wi-Fi needs NetworkManager (nmcli), which isn't installed.", true);
    }
}

pub fn build_bluetooth(page: &Page) {
    if cmd::present("bluetoothctl") {
        bluetooth(page);
    } else {
        page.banner("Bluetooth needs bluez-utils (bluetoothctl), which isn't installed.", true);
    }
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// A button that greys out and shows `busy` while `work` runs, then calls `done`.
fn action_button<T: Send + 'static>(
    label: &str,
    busy: &str,
    work: impl FnOnce() -> anyhow::Result<T> + Send + Clone + 'static,
    done: Rc<dyn Fn()>,
) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    let busy = busy.to_string();
    b.connect_clicked(move |b| {
        b.set_sensitive(false);
        b.set_label(&busy);
        let done = done.clone();
        cmd::background(work.clone(), move |r| {
            if let Err(e) = r {
                window::toast(&format!("{e}"));
            }
            done();
        });
    });
    b
}

// ----- Wi-Fi -----

fn wifi(page: &Page) {
    let g = page.group("");
    let on = cmd::output(&["nmcli", "radio", "wifi"]).is_some_and(|s| s.trim() == "enabled");
    let list = widgets::vbox(6);
    let refill: Rc<dyn Fn(bool)> = {
        let list = list.clone();
        Rc::new(move |rescan| fill_networks(&list, rescan))
    };
    let (r, _) = widgets::switch_row("Wi-Fi", "", on, {
        let refill = refill.clone();
        move |on| {
            let refill = refill.clone();
            cmd::run_async(&["nmcli", "radio", "wifi", if on { "on" } else { "off" }], move |r| {
                if let Err(e) = r {
                    window::toast(&format!("{e}"));
                }
                // Give the radio a moment to find networks again.
                glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || refill(false));
            });
        }
    });
    widgets::keywords("wireless airplane radio");
    g.add(&r);

    let status = cmd::output(&["nmcli", "-t", "-f", "TYPE,STATE,CONNECTION", "device"]).unwrap_or_default();
    let connected: Vec<String> = status
        .lines()
        .map(net::split_terse)
        .filter(|f| f.len() == 3 && (f[0] == "wifi" || f[0] == "ethernet") && f[1] == "connected")
        .map(|f| format!("{} ({})", f[2], if f[0] == "wifi" { "Wi-Fi" } else { "Ethernet" }))
        .collect();
    let text = if connected.is_empty() { "Nothing".to_string() } else { connected.join(", ") };
    let (r, _) = widgets::info_row("Connected to", &text);
    g.add(&r);

    let g = page.group("Networks");
    widgets::keywords("wifi network join connect password ssid forget hotspot");
    let scan = gtk::Button::with_label("Scan");
    {
        let list = list.clone();
        scan.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_label("Scanning…");
            let (b, list) = (b.clone(), list.clone());
            cmd::background(
                || (net::networks(true), net::saved(), net::wifi_device().map(|d| net::details(&d)).unwrap_or_default()),
                move |data| {
                    b.set_sensitive(true);
                    b.set_label("Scan");
                    show_networks(&list, data);
                },
            );
        });
    }
    g.add(&widgets::row("Nearby networks", "Choose a network to join it.", Some(scan.upcast_ref())));
    g.add(&list);
    refill(false);
}

type NetData = (Vec<net::Network>, Vec<(String, String)>, Vec<(String, String)>);

fn fill_networks(list: &gtk::Box, rescan: bool) {
    let list = list.clone();
    cmd::background(
        move || (net::networks(rescan), net::saved(), net::wifi_device().map(|d| net::details(&d)).unwrap_or_default()),
        move |data| show_networks(&list, data),
    );
}

fn show_networks(list: &gtk::Box, (networks, saved, details): NetData) {
    clear(list);
    if networks.is_empty() {
        list.append(&widgets::row("No networks found", "Turn Wi-Fi on, or press Scan.", None));
        return;
    }
    let refill: Rc<dyn Fn()> = {
        let list = list.clone();
        Rc::new(move || fill_networks(&list, false))
    };
    for n in networks {
        let saved_as = saved.iter().find(|(_, ssid)| *ssid == n.ssid).map(|(name, _)| name.clone());
        let mut parts = Vec::new();
        if n.in_use {
            parts.push("Connected".to_string());
            if let Some((_, ip)) = details.iter().find(|(k, _)| k == "IP address") {
                parts.push(ip.clone());
            }
        } else if saved_as.is_some() {
            parts.push("Saved".to_string());
        }
        parts.push(
            if n.enterprise {
                "Enterprise"
            } else if n.secure {
                "Secured"
            } else {
                "Open"
            }
            .to_string(),
        );
        parts.push(format!("Signal {}%", n.signal));
        let controls = widgets::hbox(8);
        let row = widgets::row(&n.ssid, &glib::markup_escape_text(&parts.join(" · ")), Some(controls.upcast_ref()));
        let ssid = n.ssid.clone();
        match (&saved_as, n.in_use) {
            (Some(name), true) => {
                let name2 = name.clone();
                controls.append(&action_button("Disconnect", "Disconnecting…", move || net::disconnect(&name2), refill.clone()));
                controls.append(&forget_button(name, &refill));
            }
            (Some(name), false) => {
                let (name2, ssid2) = (name.clone(), ssid.clone());
                controls.append(&action_button(
                    "Connect",
                    "Connecting…",
                    move || net::connect(&ssid2, Some(&name2), None),
                    refill.clone(),
                ));
                controls.append(&forget_button(name, &refill));
            }
            (None, _) if n.enterprise => {
                controls.append(&widgets::label("Needs a sign-in Settings can't do yet", "dim"));
            }
            (None, _) if n.secure => {
                let join = gtk::Button::with_label("Connect");
                let (controls2, refill2) = (controls.clone(), refill.clone());
                join.connect_clicked(move |_| ask_password(&controls2, &ssid, refill2.clone()));
                controls.append(&join);
            }
            (None, _) => {
                controls.append(&action_button(
                    "Connect",
                    "Connecting…",
                    move || net::connect(&ssid, None, None),
                    refill.clone(),
                ));
            }
        }
        list.append(&row);
    }
    if !details.is_empty() {
        let (d, content) = widgets::disclosure("Connection details", "");
        for (k, v) in &details {
            let (r, _) = widgets::info_row(k, v);
            content.append(&r);
        }
        list.append(&d);
    }
}

fn forget_button(name: &str, refill: &Rc<dyn Fn()>) -> gtk::Button {
    let (name, refill) = (name.to_string(), refill.clone());
    widgets::confirm_button("Forget", "Click again to forget", move |b| {
        b.set_sensitive(false);
        let (name, refill) = (name.clone(), refill.clone());
        cmd::background(
            move || net::forget(&name),
            move |r| {
                if let Err(e) = r {
                    window::toast(&format!("{e}"));
                }
                refill();
            },
        );
    })
}

/// Swap a network's buttons for a password box.
fn ask_password(controls: &gtk::Box, ssid: &str, refill: Rc<dyn Fn()>) {
    clear(controls);
    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(true);
    entry.set_placeholder_text(Some("Password"));
    entry.set_width_chars(18);
    let join = gtk::Button::with_label("Join");
    join.add_css_class("suggested-action");
    let submit = {
        let (entry, join, ssid) = (entry.clone(), join.clone(), ssid.to_string());
        move || {
            let pw = entry.text().to_string();
            if pw.is_empty() {
                entry.grab_focus();
                return;
            }
            entry.set_sensitive(false);
            join.set_sensitive(false);
            join.set_label("Joining…");
            let (ssid, refill) = (ssid.clone(), refill.clone());
            cmd::background(
                move || net::connect(&ssid, None, Some(&pw)),
                move |r| {
                    match r {
                        Ok(_) => window::toast("Connected"),
                        Err(e) => window::toast(&format!("Couldn't join: {e}")),
                    }
                    refill();
                },
            );
        }
    };
    let s = submit.clone();
    entry.connect_activate(move |_| s());
    join.connect_clicked(move |_| submit());
    controls.append(&entry);
    controls.append(&join);
    entry.grab_focus();
}

// ----- Bluetooth -----

fn bluetooth(page: &Page) {
    let g = page.group("");
    let list = widgets::vbox(6);
    let refill: Rc<dyn Fn()> = {
        let list = list.clone();
        Rc::new(move || fill_devices(&list))
    };
    let (r, _) = widgets::switch_row("Bluetooth", "", bt::powered(), {
        let refill = refill.clone();
        move |on| {
            let refill = refill.clone();
            cmd::background(
                move || bt::set_power(on),
                move |r| {
                    if let Err(e) = r {
                        window::toast(&format!("{e}"));
                    }
                    refill();
                },
            );
        }
    });
    g.add(&r);

    let g = page.group("Devices");
    widgets::keywords("bluetooth pair headphones earbuds keyboard mouse speaker controller connect forget");
    let find = gtk::Button::with_label("Find devices");
    {
        let refill = refill.clone();
        find.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_label("Searching…");
            let (b, refill) = (b.clone(), refill.clone());
            cmd::background(
                || {
                    let _ = bt::set_power(true);
                    bt::scan(10)
                },
                move |_| {
                    b.set_sensitive(true);
                    b.set_label("Find devices");
                    refill();
                },
            );
        });
    }
    g.add(&widgets::row(
        "Pair a new device",
        "Put the device in pairing mode, then search. It takes about 10 seconds.",
        Some(find.upcast_ref()),
    ));
    g.add(&list);
    refill();
}

fn fill_devices(list: &gtk::Box) {
    let list = list.clone();
    cmd::background(bt::devices, move |devices| {
        clear(&list);
        let refill: Rc<dyn Fn()> = {
            let l = list.clone();
            Rc::new(move || fill_devices(&l))
        };
        if devices.is_empty() {
            list.append(&widgets::row("No devices", "Paired devices and ones found nearby show up here.", None));
            return;
        }
        for d in devices {
            let status = if d.connected {
                "Connected"
            } else if d.paired {
                "Paired"
            } else {
                "Nearby, not paired"
            };
            let controls = widgets::hbox(8);
            let row = widgets::row(&d.name, status, Some(controls.upcast_ref()));
            let a = d.address.clone();
            if d.connected {
                controls.append(&action_button(
                    "Disconnect",
                    "Disconnecting…",
                    move || bt::device("disconnect", &a),
                    refill.clone(),
                ));
            } else if d.paired {
                controls.append(&action_button("Connect", "Connecting…", move || bt::device("connect", &a), refill.clone()));
            } else {
                controls.append(&action_button("Pair", "Pairing…", move || bt::device("pair", &a), refill.clone()));
            }
            if d.paired {
                let (a, refill) = (d.address.clone(), refill.clone());
                let forget = widgets::confirm_button("Forget", "Click again to forget", move |b| {
                    b.set_sensitive(false);
                    let (a, refill) = (a.clone(), refill.clone());
                    cmd::background(
                        move || bt::device("forget", &a),
                        move |r| {
                            if let Err(e) = r {
                                window::toast(&format!("{e}"));
                            }
                            refill();
                        },
                    );
                });
                controls.append(&forget);
            }
            list.append(&row);
        }
    });
}
