//! Connected Devices: phones, tablets and computers paired through Flux (Nyx Connect).
//!
//! fluxd does the pairing, the connection and the phone features; this page drives its
//! pairing (always with the key the user compared), shows each device's state and what it
//! offers, and edits what each may do (fluxd's per-device access). Devices that are
//! paired but away stay in the list, where their access can still change.
//! Unpair removes the pairing; Forget also drops this computer's choices for the device.

use crate::backend::connect as flux;
use crate::widgets::{self, Page};
use crate::{capability, cmd, platform, window};
use gtk::glib;
use gtk::prelude::*;
use nyx_connect::{Access, Client, Device, Feature, PairStep, Prefs, Snapshot, Status};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

pub fn build(page: &Page) {
    flux::follow();
    let Some(snap) = flux::snapshot() else {
        unavailable(page);
        return;
    };
    let pv = page.split();
    pv.pane.set_size_request(520, -1);
    let body = widgets::vbox(14);
    pv.add(&body);
    let foot = pv.foot("dialog-information-symbolic", "Unpair to stop it connecting; Forget also drops its choices here.");
    let removal = widgets::hbox(4);
    if let Some(row) = foot.parent().and_downcast::<gtk::Box>() {
        row.append(&removal);
    }
    let ui = Rc::new(Ui { snap: snap.clone(), body, removal, foot, rows: RefCell::default() });
    devices(page, &ui);
    general(page, &snap);
    trust_group(page);
    diagnostics(page, &snap);
    show_picked(&ui);
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
    // The same split as with Flux there, so the page keeps its width when it arrives.
    let pv = page.split();
    pv.pane.set_size_request(520, -1);
    let (title, line) = match flux::away() {
        None => ("Looking for Flux…", "Flux connects this computer to your phones and tablets."),
        Some(_) => ("Flux isn't here", "Flux connects this computer to your phones and tablets."),
    };
    let empty = nyx_ui::widgets::empty_state("phone-symbolic", title, line, None);
    empty.add_css_class("cn-empty");
    pv.add(&empty);
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
        "Phones reach this computer only on networks you trust: home or work, not a café.",
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
    widgets::keywords("trusted network zone home work");
    r.prepend(&widgets::tile("security-high-symbolic"));
    r.add_css_class("with-tile");
    g.add(&r);
}

// ----- What every device shares -----

fn general(page: &Page, s: &Snapshot) {
    let g = page.group("For every device");
    widgets::keywords("flux phone clipboard sync copy paste notifications browse files home folder");
    let tiled = |r: gtk::Box, icon: &str| {
        r.prepend(&widgets::tile(icon));
        r.add_css_class("with-tile");
        r
    };
    if capability::program("nyx-clipboardd") {
        let on = Prefs::load().clipboard_sync;
        let (r, _) = widgets::switch_row(
            "Sync the clipboard",
            "Passwords and copies from excluded apps stay here; what you copy on them lands in your clipboard history.",
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
        g.add(&tiled(r, "edit-copy-symbolic"));
    } else {
        let (r, _) = widgets::switch_row(
            "Sync the clipboard",
            "Copies go to your devices, and theirs come here.",
            s.settings.auto_clipboard,
            |on| act(move |c| c.set("autoClipboard", on.into()), |_| {}),
        );
        g.add(&tiled(r, "edit-copy-symbolic"));
    }
    let (r, _) = widgets::switch_row(
        "Show their notifications",
        "In the Notification Center.",
        s.settings.notifications,
        |on| act(move |c| c.set("notifications", on.into()), |_| {}),
    );
    widgets::keywords("show notifications from devices");
    g.add(&tiled(r, "preferences-system-notifications-symbolic"));
    let (r, _) = widgets::switch_row(
        "Let them browse this computer",
        "Your home folder. Each device can still be kept out.",
        s.settings.share_home,
        |on| act(move |c| c.set("shareHome", on.into()), |_| {}),
    );
    widgets::keywords("let devices browse this computer home folder");
    g.add(&tiled(r, "folder-symbolic"));
    let home = glib::home_dir();
    let dir = std::path::Path::new(&s.settings.download_dir);
    let shown = match dir.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "Home folder".to_string(),
        Ok(rest) => rest.display().to_string(),
        Err(_) => s.settings.download_dir.clone(),
    };
    let (r, l) = widgets::info_row("Files they send", &shown);
    l.set_tooltip_text(Some(&s.settings.download_dir));
    widgets::keywords("files from devices are saved in downloads folder");
    g.add(&tiled(r, "folder-download-symbolic"));
}

fn trust_group(page: &Page) {
    if !platform::connect_on_trusted_networks() {
        return;
    }
    let g = page.group("");
    trust_row(&g);
}

// ----- The devices -----

thread_local! {
    /// The device shown on the right, kept while the page is redrawn.
    static PICKED: RefCell<Option<String>> = const { RefCell::new(None) };
}

struct Ui {
    snap: Arc<Snapshot>,
    body: gtk::Box,
    removal: gtk::Box,
    foot: gtk::Label,
    /// Each paired device's row, to mark the one picked.
    rows: RefCell<Vec<(String, gtk::Box)>>,
}

fn app_name(d: &Device) -> String {
    let app = match d.app.as_str() {
        "android" | "android-debug" => "Flux for Android",
        "ios" => "Flux for iOS",
        "macos" => "Flux for macOS",
        "fluxd" => "Flux",
        _ => "",
    };
    format!("{app} {}", d.app_version).trim().to_string()
}

/// "Connected · 82% battery, charging" or "Not connected · last seen yesterday".
fn describe(d: &Device) -> String {
    let mut parts = Vec::new();
    if d.online {
        parts.push("Connected".to_string());
        if let Some(b) = d.battery {
            parts.push(format!("{}% battery{}", b.charge, if b.charging { ", charging" } else { "" }));
        }
    } else {
        let index = nyx_devices::Index::load_standard();
        let seen = match (index.get(&nyx_connect::device_id(&d.id)), d.last_seen) {
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
        parts.push(esc(&if seen == "Not connected" { seen } else { format!("Not connected · {}", lower_first(&seen)) }));
    }
    parts.join(" · ")
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_lowercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn devices(page: &Page, ui: &Rc<Ui>) {
    let s = &ui.snap;
    let g = page.group("Devices");
    widgets::keywords("pair new phone tablet android iphone flux verification key accept reject unpair forget ring");
    // Connected first, then the ones asking to pair or nearby, then the ones away.
    let mut list: Vec<&Device> = s.devices.iter().collect();
    list.sort_by_key(|d| match d.status() {
        Status::Connected => 0,
        Status::Pairing(_) => 1,
        Status::Discovered => 2,
        Status::Disconnected => 3,
    });
    let paired: Vec<&Device> = s.paired().collect();
    let picked = PICKED
        .with(|p| p.borrow().clone())
        .filter(|id| paired.iter().any(|d| &d.id == id))
        .or_else(|| paired.iter().find(|d| d.online).or(paired.first()).map(|d| d.id.clone()));
    PICKED.with(|p| *p.borrow_mut() = picked);
    for d in list {
        let name = d.display_name(&s.devices);
        match d.status() {
            Status::Discovered => g.add(&discovered(d, &name)),
            Status::Pairing(step) => g.add(&pairing(d, &name, step)),
            Status::Connected | Status::Disconnected => {
                let chevron = gtk::Image::from_icon_name("go-next-symbolic");
                chevron.add_css_class("chevron");
                let r = widgets::icon_row(d.icon(), &esc(&name), &describe(d), Some(chevron.upcast_ref()));
                widgets::keywords("phone tablet device paired remembered");
                let (ui2, id) = (ui.clone(), d.id.clone());
                r.add_css_class("cn-click");
                r.set_cursor_from_name(Some("pointer"));
                r.set_focusable(true);
                let click = gtk::GestureClick::new();
                click.connect_released(move |_, _, _, _| {
                    PICKED.with(|p| *p.borrow_mut() = Some(id.clone()));
                    show_picked(&ui2);
                });
                r.add_controller(click);
                ui.rows.borrow_mut().push((d.id.clone(), r.clone()));
                g.add(&r);
            }
        }
    }
    // Remembered here, but no longer paired.
    let index = nyx_devices::Index::load_standard();
    for (id, profile) in nyx_connect::orphans(&index, s) {
        let forget = forget_button(s, id.key());
        let r = widgets::icon_row(
            "phone-symbolic",
            &esc(&profile.name),
            "No longer paired. Pair it again, or forget it.",
            Some(forget.upcast_ref()),
        );
        widgets::keywords("remembered offline away disconnected phone forget unpair");
        g.add(&r);
    }
    let pair = widgets::icon_row("list-add-symbolic", "Pair a new device…", "Open Flux on the phone, on the same network.", None);
    pair.add_css_class("cn-add-row");
    pair.add_css_class("cn-click");
    pair.set_cursor_from_name(Some("pointer"));
    widgets::keywords("pair a new device look for devices discover");
    let desc = pair.first_child().and_then(|t| t.next_sibling()).and_then(|t| t.last_child()).and_downcast::<gtk::Label>();
    let click = gtk::GestureClick::new();
    click.connect_released(move |_, _, _, _| {
        if let Some(d) = &desc {
            d.set_text("Looking… it shows up above; then compare the key on both screens.");
        }
        act(|c| c.discover(), |_| {});
    });
    pair.add_controller(click);
    g.add(&pair);
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
    widgets::icon_row(d.icon(), &esc(name), &desc, Some(pair.upcast_ref()))
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
                        PICKED.with(|x| *x.borrow_mut() = Some(p.device.clone()));
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
    let r = widgets::icon_row(d.icon(), &esc(name), &format!("{desc} Pairing ends after 30 seconds."), Some(controls.upcast_ref()));
    r.add_css_class("cn-picked");
    r
}

// ----- The picked device -----

fn show_picked(ui: &Rc<Ui>) {
    let picked = PICKED.with(|p| p.borrow().clone());
    for (id, row) in ui.rows.borrow().iter() {
        if picked.as_deref() == Some(id.as_str()) {
            row.add_css_class("cn-picked");
        } else {
            row.remove_css_class("cn-picked");
        }
    }
    let body = &ui.body;
    while let Some(c) = body.first_child() {
        body.remove(&c);
    }
    while let Some(c) = ui.removal.first_child() {
        ui.removal.remove(&c);
    }
    let s = &ui.snap;
    let Some(d) = picked.and_then(|id| s.paired().find(|d| d.id == id)) else {
        ui.foot.set_text("Phones and tablets with the Flux app pair with this computer on the same network.");
        let empty = nyx_ui::widgets::empty_state(
            "phone-symbolic",
            "No devices yet",
            "Open Flux on your phone, then choose Pair a new device on the left and compare the key on both screens.",
            None,
        );
        empty.add_css_class("cn-empty");
        body.append(&empty);
        return;
    };
    ui.foot.set_text("Unpair to stop it connecting; Forget also drops its choices here.");
    let name = d.display_name(&s.devices);

    // The heading: its name, its app, and what you can do with it now.
    let head = widgets::hbox(8);
    head.add_css_class("preview-head");
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    let t = widgets::label(&name, "preview-title");
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&t);
    let mut line = vec![app_name(d)];
    if !d.app_update.is_empty() {
        line.push(format!("Flux {} is available for it", d.app_update));
    }
    let l = widgets::label(&line.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · "), "preview-line");
    l.set_wrap(true);
    l.set_xalign(0.0);
    text.append(&l);
    head.append(&text);
    if d.online && d.has(Feature::Clipboard) {
        let b = nyx_ui::widgets::labeled_button("edit-copy-symbolic", "Send clipboard");
        b.set_valign(gtk::Align::Center);
        let id = d.id.clone();
        b.connect_clicked(move |b| {
            let id = id.clone();
            b.display().clipboard().read_text_async(gtk::gio::Cancellable::NONE, move |r| match r {
                Ok(Some(text)) => {
                    let text = text.to_string();
                    act(move |c| c.send_clipboard(&id, &text), |_| window::toast("Sent"));
                }
                _ => window::toast("There's no text on the clipboard to send"),
            });
        });
        head.append(&b);
    }
    if d.online && d.has(Feature::Ring) {
        let ring = nyx_ui::widgets::labeled_button("audio-volume-high-symbolic", "Ring");
        ring.set_valign(gtk::Align::Center);
        ring.set_tooltip_text(Some("Ring it, to find it"));
        let id = d.id.clone();
        ring.connect_clicked(move |_| {
            let id = id.clone();
            act(move |c| c.ring(&id), |_| {});
        });
        head.append(&ring);
    }
    body.append(&head);

    // The phone, and beside it what it may do here and what it offers.
    let row = widgets::hbox(18);
    let stage = widgets::vbox(8);
    stage.set_valign(gtk::Align::Start);
    stage.append(&phone(s, d));
    if let Some(b) = d.battery.filter(|_| d.online) {
        let pct = widgets::label(&format!("{}%", b.charge), "cn-big");
        pct.set_halign(gtk::Align::Center);
        stage.append(&pct);
        if b.charging {
            let c = widgets::label("Charging", "cn-okt");
            c.set_halign(gtk::Align::Center);
            stage.append(&c);
        }
    }
    row.append(&stage);
    let side = widgets::vbox(8);
    side.set_hexpand(true);
    let accesses = flux::accesses(s, d);
    if !accesses.is_empty() {
        side.append(&widgets::label("What it may do here", "cn-strong"));
        for access in accesses {
            side.append(&access_tile(s, d, access));
        }
    }
    let features = d.features();
    if !features.is_empty() {
        let h = widgets::label("It offers", "cn-strong");
        h.set_margin_top(8);
        side.append(&h);
        let chips = gtk::FlowBox::new();
        chips.set_selection_mode(gtk::SelectionMode::None);
        chips.set_max_children_per_line(8);
        chips.set_row_spacing(6);
        chips.set_column_spacing(6);
        chips.add_css_class("cn-flow");
        for f in features {
            let c = widgets::label(f.label(), "cn-chip");
            c.set_halign(gtk::Align::Start);
            chips.insert(&c, -1);
        }
        side.append(&chips);
    }
    row.append(&side);
    body.append(&row);

    // Unpair and Forget at the foot.
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
    let forget = forget_button(s, &d.id);
    for b in [&unpair, &forget] {
        b.add_css_class("flat");
        b.add_css_class("cn-danger");
        b.set_valign(gtk::Align::Center);
        ui.removal.append(b);
    }
}

/// The phone, drawn: its clock, Flux connected, and a tile for each thing it may do.
fn phone(s: &Snapshot, d: &Device) -> gtk::Box {
    let ph = widgets::vbox(10);
    ph.add_css_class("cn-phone");
    if !d.online {
        ph.add_css_class("away");
    }
    ph.set_halign(gtk::Align::Center);
    let status = widgets::hbox(4);
    status.add_css_class("cn-phone-status");
    let time = glib::DateTime::now_local().ok().and_then(|t| t.format("%H:%M").ok()).map(|t| t.to_string()).unwrap_or_default();
    let clock = widgets::label(&time, "cn-phone-clock");
    clock.set_hexpand(true);
    clock.set_xalign(0.0);
    status.append(&clock);
    if let Some(b) = d.battery.filter(|_| d.online) {
        status.append(&widgets::label(&format!("{}%", b.charge), "cn-phone-clock"));
    }
    ph.append(&status);
    let mid = widgets::vbox(4);
    mid.set_vexpand(true);
    mid.set_valign(gtk::Align::Center);
    let logo = widgets::hbox(0);
    logo.add_css_class("cn-phone-logo");
    logo.set_halign(gtk::Align::Center);
    let i = gtk::Image::from_icon_name("insert-link-symbolic");
    i.set_pixel_size(22);
    logo.append(&i);
    mid.append(&logo);
    let f = widgets::label("Flux", "cn-phone-title");
    f.set_halign(gtk::Align::Center);
    mid.append(&f);
    let c = widgets::label(
        &if d.online { format!("Connected to {}", s.this.name) } else { "Not connected".to_string() },
        "cn-phone-line",
    );
    c.set_halign(gtk::Align::Center);
    c.set_ellipsize(gtk::pango::EllipsizeMode::End);
    c.set_max_width_chars(16);
    mid.append(&c);
    ph.append(&mid);
    let grid = gtk::Grid::new();
    grid.set_row_spacing(6);
    grid.set_column_spacing(6);
    grid.set_halign(gtk::Align::Center);
    let tiles = [
        ("edit-copy-symbolic", Some(Access::Clipboard)),
        ("preferences-system-notifications-symbolic", Some(Access::Notifications)),
        ("folder-symbolic", Some(Access::ShareHome)),
        ("send-to-symbolic", None),
    ];
    for (i, (icon, access)) in tiles.into_iter().enumerate() {
        let t = widgets::hbox(0);
        t.add_css_class("cn-phone-tile");
        if access.is_some_and(|a| !s.allowed(&d.id, a)) {
            t.add_css_class("off");
        }
        let img = gtk::Image::from_icon_name(icon);
        img.set_hexpand(true);
        t.append(&img);
        grid.attach(&t, (i % 2) as i32, (i / 2) as i32, 1, 1);
    }
    ph.append(&grid);
    ph
}

fn access_tile(s: &Snapshot, d: &Device, access: Access) -> gtk::Box {
    let (icon, desc) = match access {
        Access::Clipboard => ("edit-copy-symbolic", "Copies go both ways."),
        Access::Notifications => ("preferences-system-notifications-symbolic", "Its notifications show here."),
        Access::ShareHome => ("folder-symbolic", "It can open your home folder."),
        Access::RemoteInput => ("input-mouse-symbolic", "It can move the pointer and type."),
        Access::RemoteDesktop => ("video-display-symbolic", "It can show what's on screen."),
    };
    let on = s.allowed(&d.id, access);
    let sw = gtk::Switch::new();
    sw.set_active(on);
    let r = widgets::icon_row(icon, access.label(), desc, Some(sw.upcast_ref()));
    r.add_css_class("cn-dv-access");
    if on {
        r.add_css_class("on");
    }
    let (id, r2) = (d.id.clone(), r.clone());
    sw.connect_active_notify(move |sw| {
        let on = sw.is_active();
        if on {
            r2.add_css_class("on");
        } else {
            r2.remove_css_class("on");
        }
        let id = id.clone();
        let id2 = id.clone();
        act(move |c| c.set_access(&id, access, Some(on)), move |_| flux::remember(&id2));
    });
    r
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

// ----- What's going on -----

fn diagnostics(page: &Page, s: &Snapshot) {
    let g = page.group("");
    let version = if s.this.version.is_empty() { "earlier than 0.8".to_string() } else { s.this.version.clone() };
    let rows = vec![
        ("Flux", version.clone()),
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
    let (r, value) = widgets::nav_row(
        Some("utilities-terminal-symbolic"),
        "Diagnostics",
        "Flux's version, port and what's on its way.",
        &format!("Flux {version}"),
        move || {
            let rows = rows.clone();
            widgets::subpage("Diagnostics", 520, move |p| {
                let g = p.group("");
                for (k, v) in rows {
                    let (r, _) = widgets::info_row(k, &v);
                    g.add(&r);
                }
            });
        },
    );
    value.set_ellipsize(gtk::pango::EllipsizeMode::None);
    widgets::keywords("flux version port status debug clipboard sync socket");
    g.add(&r);
}
