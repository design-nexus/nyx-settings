use crate::backend::{bt, net, sysinfo};
use crate::charts;
use crate::sketch;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

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
    widgets::forget_rows(b);
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn esc(t: &str) -> String {
    glib::markup_escape_text(t).to_string()
}

/// A row's description label (rows made by `widgets::row` and friends).
fn desc_of(row: &impl IsA<gtk::Widget>) -> Option<gtk::Label> {
    fn find(w: &gtk::Widget) -> Option<gtk::Label> {
        let mut c = w.first_child();
        while let Some(child) = c {
            if child.has_css_class("settings-option-description") {
                return child.downcast::<gtk::Label>().ok();
            }
            if let Some(l) = find(&child) {
                return Some(l);
            }
            c = child.next_sibling();
        }
        None
    }
    find(row.upcast_ref())
}

/// A whole row that does something when clicked or activated with the keyboard.
fn on_click(row: &gtk::Box, f: impl Fn() + 'static) {
    row.add_css_class("cn-click");
    row.set_cursor_from_name(Some("pointer"));
    row.set_focusable(true);
    let f = Rc::new(f);
    let click = gtk::GestureClick::new();
    let f2 = f.clone();
    click.connect_released(move |_, _, _, _| f2());
    row.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::space) {
            f();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    row.add_controller(keys);
}

/// A group whose title line has a note and buttons at its right.
pub(super) fn titled_group(page: &Page, title: &str, end: &[&gtk::Widget]) -> widgets::Group {
    let g = page.group("");
    let head = widgets::hbox(8);
    head.add_css_class("cn-group-head");
    let t = widgets::label(&title.to_uppercase(), "group-title");
    t.set_hexpand(true);
    t.set_xalign(0.0);
    head.append(&t);
    for w in end {
        w.set_valign(gtk::Align::Center);
        head.append(*w);
    }
    g.top(&head);
    g
}

/// Give the preview its width from the start: its content arrives later, and the
/// window decides between side by side and stacked from the width the page asks for.
pub(super) fn steady(pv: &widgets::Preview) {
    pv.pane.set_size_request(PREVIEW_MIN, -1);
}

const PREVIEW_MIN: i32 = 520;

/// Children side by side while they fit, wrapping onto lines of their own when not.
fn flow(max: u32, homogeneous: bool) -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_homogeneous(homogeneous);
    f.set_min_children_per_line(1);
    f.set_max_children_per_line(max);
    f.set_column_spacing(12);
    f.set_row_spacing(12);
    f.add_css_class("cn-flow");
    f
}

fn flow_add(f: &gtk::FlowBox, w: &impl IsA<gtk::Widget>) {
    f.insert(w, -1);
    if let Some(c) = f.last_child() {
        c.set_focusable(false);
    }
}

fn card(class: &str, spacing: i32, vertical: bool) -> gtk::Box {
    let c = if vertical { widgets::vbox(spacing) } else { widgets::hbox(spacing) };
    c.add_css_class("cn-card");
    c.add_css_class(class);
    c
}

/// A preview heading built in place (so it can be rebuilt): title, a line, and controls.
fn pv_head(title: &str, line: &str, end: &[&gtk::Widget]) -> gtk::Box {
    let head = widgets::hbox(10);
    head.add_css_class("preview-head");
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(title, "preview-title");
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&t);
    if !line.is_empty() {
        let l = widgets::label(line, "preview-line");
        l.set_wrap(true);
        l.set_xalign(0.0);
        text.append(&l);
    }
    head.append(&text);
    for w in end {
        w.set_valign(gtk::Align::Center);
        head.append(*w);
    }
    head
}

/// Facts in three columns: a small name over each value.
fn facts(pairs: &[(&str, String)]) -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.add_css_class("cn-card");
    grid.add_css_class("cn-facts");
    grid.set_column_homogeneous(true);
    grid.set_column_spacing(16);
    grid.set_row_spacing(12);
    for (i, (k, v)) in pairs.iter().enumerate() {
        let cell = widgets::vbox(2);
        let key = widgets::label(k, "cn-fact-key");
        key.set_ellipsize(gtk::pango::EllipsizeMode::End);
        cell.append(&key);
        let l = widgets::label(v, "cn-fact-value");
        l.set_selectable(true);
        l.set_focusable(false);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_tooltip_text(Some(v));
        cell.append(&l);
        grid.attach(&cell, (i % 3) as i32, (i / 3) as i32, 1, 1);
    }
    grid
}

/// A small pill of text: "Down 2.1 MB/s".
fn chip(text: &str, accent: bool) -> gtk::Label {
    let l = widgets::label(text, "cn-chip");
    if accent {
        l.add_css_class("accent");
    }
    l
}

/// A ring gauge with its value written in the middle.
fn ring_with(size: i32, value: f64, text: &str) -> gtk::Overlay {
    let ring = charts::ring(size);
    ring.set(value);
    let o = gtk::Overlay::new();
    o.set_child(Some(&ring.area));
    let l = widgets::label(text, "cn-ring-value");
    l.set_halign(gtk::Align::Center);
    l.set_valign(gtk::Align::Center);
    o.add_overlay(&l);
    o.set_valign(gtk::Align::Center);
    o
}

/// Signal strength as four bars.
fn bars(signal: u8) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(18);
    a.set_content_height(16);
    a.set_valign(gtk::Align::Center);
    a.add_css_class("cn-bars");
    let lit = match signal {
        75.. => 4,
        50..=74 => 3,
        30..=49 => 2,
        _ => 1,
    };
    a.set_draw_func(move |_, cr, w, h| {
        let p = sketch::pal();
        let (w, h) = (w as f64, h as f64);
        let bw = (w - 3.0 * 2.0) / 4.0;
        for i in 0..4 {
            let bh = h * (0.35 + 0.65 * (i as f64 + 1.0) / 4.0);
            let x = i as f64 * (bw + 2.0);
            sketch::rounded(cr, x, h - bh, bw, bh, 1.2);
            sketch::set(cr, p.text, if i < lit { 0.9 } else { 0.2 });
            let _ = cr.fill();
        }
    });
    a
}

fn quality(signal: u8) -> &'static str {
    match signal {
        75.. => "Excellent signal",
        50..=74 => "Good signal",
        30..=49 => "Fair signal",
        _ => "Weak signal",
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

#[derive(Default)]
struct WifiData {
    on: bool,
    networks: Vec<net::Network>,
    /// (connection name, SSID)
    saved: Vec<(String, String)>,
    details: Vec<(String, String)>,
    device: Option<String>,
    link: Option<net::Link>,
    /// The saved connection in use for Wi-Fi.
    current: Option<String>,
    settings: Vec<(String, String)>,
    /// The band chosen for the network in use, and the bands it's on.
    band: Option<(String, Vec<String>)>,
    /// Wired connections in use.
    wired: Vec<String>,
}

/// `rescan`: `None` for what was last seen, without waiting for a scan.
fn load_wifi(rescan: Option<bool>) -> WifiData {
    let on = cmd::output(&["nmcli", "radio", "wifi"]).is_some_and(|s| s.trim() == "enabled");
    let status = cmd::output(&["nmcli", "-t", "-f", "TYPE,STATE,CONNECTION", "device"]).unwrap_or_default();
    let in_use = |kind: &str| -> Vec<String> {
        status
            .lines()
            .map(net::split_terse)
            .filter(|f| f.len() == 3 && f[0] == kind && f[1] == "connected")
            .map(|f| f[2].clone())
            .collect()
    };
    let current = in_use("wifi").into_iter().next();
    let settings = current
        .as_deref()
        .map(|c| net::connection_settings(c, "connection.autoconnect,802-11-wireless.cloned-mac-address"))
        .unwrap_or_default();
    let band = current.as_ref().and_then(|_| cmd::output(&["omarchy-network-band"])).as_deref().and_then(parse_band);
    let device = net::wifi_device();
    WifiData {
        on,
        networks: match (on, rescan) {
            (false, _) => Vec::new(),
            (true, None) => net::networks_seen(),
            (true, Some(fresh)) => net::networks(fresh),
        },
        saved: net::saved(),
        details: device.as_deref().map(net::details).unwrap_or_default(),
        link: current.as_ref().and_then(|_| net::link()),
        device,
        current,
        settings,
        band,
        wired: in_use("ethernet"),
    }
}

/// `omarchy-network-band`: the band chosen (`selected`, else the one in use) and those available.
fn parse_band(text: &str) -> Option<(String, Vec<String>)> {
    let (mut band, mut selected, mut available) = (None, None, Vec::new());
    for line in text.lines() {
        match line.split_once('\t') {
            Some(("band", v)) => band = Some(v.trim().to_string()),
            Some(("selected", v)) => selected = Some(v.trim().to_string()),
            Some(("available", v)) => available = v.split_whitespace().map(str::to_string).collect(),
            _ => {}
        }
    }
    selected.or(band).filter(|_| !available.is_empty()).map(|b| (b, available))
}

struct WifiUi {
    top_desc: Option<gtk::Label>,
    looked: gtk::Label,
    list: gtk::Box,
    saved_value: gtk::Label,
    saved: RefCell<Vec<(String, String)>>,
    in_range: RefCell<Vec<String>>,
    pv: gtk::Box,
    forget: gtk::Box,
    /// Bumped on every refill, so the old traffic sampler stops.
    generation: Cell<u64>,
    looked_at: Cell<i64>,
    /// What was last shown, so an unchanged answer doesn't redraw (and close a password box).
    shown: RefCell<String>,
}

fn now() -> i64 {
    glib::real_time() / 1_000_000
}

fn wifi(page: &Page) {
    let on = cmd::output(&["nmcli", "radio", "wifi"]).is_some_and(|s| s.trim() == "enabled");

    // The network you're on, filled in once it's been read.
    let pv = page.split();
    steady(&pv);
    let pv_body = widgets::vbox(14);
    pv_body.append(&pv_head("Wi-Fi", "Reading the networks nearby…", &[]));
    pv.add(&pv_body);
    let foot = pv.foot("dialog-information-symbolic", "DNS, VPN and the firewall are on the Network page.");
    let forget = widgets::hbox(6);
    if let Some(row) = foot.parent().and_downcast::<gtk::Box>() {
        let to_network = gtk::Button::with_label("Network");
        to_network.add_css_class("flat");
        to_network.set_valign(gtk::Align::Center);
        to_network.connect_clicked(|_| window::navigate("network"));
        row.append(&to_network);
        row.append(&forget);
    }

    let g = page.group("");
    let sw = gtk::Switch::new();
    sw.set_active(on);
    let top = widgets::icon_row(
        "network-wireless-symbolic",
        "Wi-Fi",
        if on { "Looking for networks…" } else { "Off" },
        Some(sw.upcast_ref()),
    );
    widgets::keywords("wireless airplane radio");
    g.add(&top);

    let looked = widgets::label("", "cn-note");
    let rescan = widgets::icon_button("view-refresh-symbolic", "Look again");
    rescan.add_css_class("flat");
    let g = titled_group(page, "Networks", &[looked.upcast_ref(), rescan.upcast_ref()]);
    let list = widgets::vbox(0);
    g.add(&list);
    let hidden = widgets::icon_row("list-add-symbolic", "Join a hidden network…", "One that doesn't show its name.", None);
    hidden.add_css_class("cn-add-row");
    widgets::keywords("wifi network join connect password ssid forget hotspot hidden");
    g.add(&hidden);

    let g = page.group("");
    let ui_slot: Rc<RefCell<Option<Rc<WifiUi>>>> = Rc::default();
    let (saved_row, saved_value) = widgets::nav_row(
        Some("document-open-recent-symbolic"),
        "Saved networks",
        "Forget the ones you don't use.",
        "",
        {
            let ui_slot = ui_slot.clone();
            move || {
                if let Some(ui) = ui_slot.borrow().as_ref() {
                    saved_page(ui);
                }
            }
        },
    );
    widgets::keywords("saved remembered known forget networks");
    g.add(&saved_row);

    let ui = Rc::new(WifiUi {
        top_desc: desc_of(&top),
        looked,
        list,
        saved_value,
        saved: RefCell::default(),
        in_range: RefCell::default(),
        pv: pv_body,
        forget,
        generation: Cell::new(0),
        looked_at: Cell::new(0),
        shown: RefCell::default(),
    });
    *ui_slot.borrow_mut() = Some(ui.clone());

    sw.connect_active_notify({
        let ui = ui.clone();
        move |s| {
            let on = s.is_active();
            let ui = ui.clone();
            cmd::run_async(&["nmcli", "radio", "wifi", if on { "on" } else { "off" }], move |r| {
                if let Err(e) = r {
                    window::toast(&format!("{e}"));
                }
                // Give the radio a moment to find networks again.
                glib::timeout_add_local_once(Duration::from_secs(3), move || fill_wifi(&ui, Some(false)));
            });
        }
    });
    rescan.connect_clicked({
        let ui = ui.clone();
        move |b| {
            b.set_sensitive(false);
            ui.looked.set_text("Looking…");
            let (b, ui) = (b.clone(), ui.clone());
            cmd::background(
                || load_wifi(Some(true)),
                move |data| {
                    b.set_sensitive(true);
                    show_wifi(&ui, data);
                },
            );
        }
    });
    on_click(&hidden, {
        let ui = ui.clone();
        move || join_hidden(&ui)
    });
    // "Looked 2 min ago" stays true while the page is open.
    {
        let ui = Rc::downgrade(&ui);
        glib::timeout_add_local(Duration::from_secs(15), move || {
            let Some(ui) = ui.upgrade() else { return glib::ControlFlow::Break };
            set_looked(&ui);
            glib::ControlFlow::Continue
        });
    }
    // What was seen last straight away, then again once a scan is done if it's old.
    let ui2 = ui.clone();
    cmd::background(
        || load_wifi(None),
        move |data| {
            show_wifi(&ui2, data);
            fill_wifi(&ui2, Some(false));
        },
    );
}

fn set_looked(ui: &WifiUi) {
    if ui.looked_at.get() > 0 {
        ui.looked.set_text(&format!("Looked {}", crate::units::ago(now() - ui.looked_at.get())));
    }
}

fn fill_wifi(ui: &Rc<WifiUi>, rescan: Option<bool>) {
    let ui = ui.clone();
    cmd::background(move || load_wifi(rescan), move |data| show_wifi(&ui, data));
}

fn show_wifi(ui: &Rc<WifiUi>, data: WifiData) {
    ui.looked_at.set(now());
    set_looked(ui);
    let sig = format!(
        "{} {:?} {:?} {:?} {:?} {:?}",
        data.on,
        data.networks.iter().map(|n| (&n.ssid, n.in_use, n.signal / 25)).collect::<Vec<_>>(),
        data.saved,
        data.current,
        data.settings,
        data.band
    );
    if *ui.shown.borrow() == sig {
        return;
    }
    *ui.shown.borrow_mut() = sig;
    *ui.saved.borrow_mut() = data.saved.clone();
    *ui.in_range.borrow_mut() = data.networks.iter().map(|n| n.ssid.clone()).collect();
    ui.saved_value.set_text(&data.saved.len().to_string());
    ui.saved_value.set_visible(!data.saved.is_empty());
    if let Some(d) = &ui.top_desc {
        d.set_text(&match (data.on, data.networks.len()) {
            (false, _) => "Off".to_string(),
            (true, 0) => "No networks nearby".to_string(),
            (true, 1) => "1 network nearby".to_string(),
            (true, n) => format!("{n} networks nearby"),
        });
    }
    show_networks(ui, &data);
    show_current(ui, &data);
}

fn show_networks(ui: &Rc<WifiUi>, data: &WifiData) {
    let list = &ui.list;
    clear(list);
    widgets::begin_section("wifi");
    if data.networks.is_empty() {
        let (title, desc) = if data.on {
            ("No networks found", "Look again in a moment, or join a hidden network.")
        } else {
            ("Wi-Fi is off", "Turn it on to see the networks nearby.")
        };
        list.append(&widgets::row(title, desc, None));
        return;
    }
    let refill: Rc<dyn Fn()> = {
        let ui = ui.clone();
        Rc::new(move || fill_wifi(&ui, None))
    };
    // The strongest few; the rest a click away.
    let more = widgets::vbox(0);
    for (i, n) in data.networks.iter().enumerate() {
        let holder = if i < SHOWN_NETWORKS { list.clone() } else { more.clone() };
        let saved_as = data.saved.iter().find(|(_, ssid)| *ssid == n.ssid).map(|(name, _)| name.clone());
        let security = if n.enterprise { "Enterprise" } else if n.secure { "Secured" } else { "Open" };
        let desc = if n.in_use {
            format!("Connected · {security}")
        } else if n.enterprise && saved_as.is_none() {
            "Needs a sign-in Settings can't do yet".to_string()
        } else if !n.secure {
            format!("{}Open · anyone nearby can see what you send", if saved_as.is_some() { "Saved · " } else { "" })
        } else if saved_as.is_some() {
            format!("Saved · {security}")
        } else {
            security.to_string()
        };
        let end = widgets::hbox(6);
        if n.in_use {
            let i = gtk::Image::from_icon_name("object-select-symbolic");
            i.add_css_class("cn-ok");
            end.append(&i);
        } else if !n.secure {
            end.append(&chip("Open", false));
        } else {
            let i = gtk::Image::from_icon_name("changes-prevent-symbolic");
            i.add_css_class("dim");
            end.append(&i);
        }
        let row = widgets::row(&esc(&n.ssid), &desc, Some(end.upcast_ref()));
        widgets::keywords("wifi network join connect");
        row.prepend(&bars(n.signal));
        row.add_css_class("cn-net");
        let wrap = widgets::vbox(0);
        wrap.append(&row);
        holder.append(&wrap);
        if n.in_use || (n.enterprise && saved_as.is_none()) {
            if n.in_use {
                row.add_css_class("cn-picked");
            }
            continue;
        }
        let desc_label = desc_of(&row);
        let ssid = n.ssid.clone();
        let (secure, refill) = (n.secure, refill.clone());
        let asking: Rc<RefCell<Option<gtk::Box>>> = Rc::default();
        on_click(&row, move || {
            if saved_as.is_none() && secure {
                // A new secured network: ask for its password under the row.
                if let Some(b) = asking.borrow_mut().take() {
                    wrap.remove(&b);
                    return;
                }
                let b = password_box(&ssid, refill.clone());
                wrap.append(&b);
                *asking.borrow_mut() = Some(b);
                return;
            }
            if let Some(d) = &desc_label {
                d.set_text("Joining…");
            }
            let (ssid, name, refill) = (ssid.clone(), saved_as.clone(), refill.clone());
            cmd::background(
                move || net::connect(&ssid, name.as_deref(), None),
                move |r| {
                    if let Err(e) = r {
                        window::toast(&format!("Couldn't join: {e}"));
                    }
                    refill();
                },
            );
        });
    }
    let hidden = data.networks.len().saturating_sub(SHOWN_NETWORKS);
    if hidden > 0 {
        more.set_visible(false);
        list.append(&more);
        let r = widgets::icon_row("view-more-symbolic", &format!("Show {hidden} more"), "Weaker networks further away.", None);
        r.add_css_class("cn-add-row");
        let r2 = r.clone();
        on_click(&r, move || {
            more.set_visible(true);
            r2.set_visible(false);
        });
        list.append(&r);
    }
}

const SHOWN_NETWORKS: usize = 7;

/// A password box and Join, under a network's row.
fn password_box(ssid: &str, refill: Rc<dyn Fn()>) -> gtk::Box {
    let b = widgets::hbox(8);
    b.add_css_class("cn-ask");
    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(true);
    entry.set_placeholder_text(Some("Password"));
    entry.set_hexpand(true);
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
    b.append(&entry);
    b.append(&join);
    let e = entry.clone();
    glib::idle_add_local_once(move || {
        e.grab_focus();
    });
    b
}

fn join_hidden(ui: &Rc<WifiUi>) {
    let ui = ui.clone();
    crate::dialog::ask(
        "Join a hidden network",
        "Its name has to match exactly. Leave the password empty for an open network.",
        vec![crate::dialog::Field::text("Network name"), crate::dialog::Field::secret("Password")],
        "Join",
        None,
        move |values, _| {
            let ssid = values.first().cloned().unwrap_or_default();
            let pw = values.get(1).cloned().unwrap_or_default();
            if ssid.trim().is_empty() {
                return Some("Type the network's name".into());
            }
            let ui = ui.clone();
            window::toast(&format!("Joining {ssid}…"));
            cmd::background(
                move || {
                    let mut args = vec!["nmcli", "device", "wifi", "connect", ssid.as_str()];
                    if !pw.is_empty() {
                        args.extend(["password", pw.as_str()]);
                    }
                    args.extend(["hidden", "yes"]);
                    cmd::run(&args)
                },
                move |r| {
                    match r {
                        Ok(_) => window::toast("Connected"),
                        Err(e) => window::toast(&format!("Couldn't join: {e}")),
                    }
                    fill_wifi(&ui, None);
                },
            );
            None
        },
    );
}

/// Every saved network, each with Forget.
fn saved_page(ui: &Rc<WifiUi>) {
    let saved = ui.saved.borrow().clone();
    let in_range = ui.in_range.borrow().clone();
    let ui = ui.clone();
    widgets::subpage("Saved networks", 520, move |p| {
        let g = p.group("");
        g.note("Forgetting one removes its password; you'd type it again to join.");
        if saved.is_empty() {
            g.add(&widgets::row("No saved networks", "Networks you join are remembered here.", None));
        }
        for (name, ssid) in saved {
            let here = in_range.contains(&ssid);
            let mut desc = if here { "In range".to_string() } else { "Not in range".to_string() };
            if name != ssid {
                desc = format!("{} · {desc}", esc(&ssid));
            }
            let ui = ui.clone();
            let n = name.clone();
            let forget = widgets::confirm_button("Forget", "Click again to forget", move |b| {
                b.set_sensitive(false);
                let (n, ui, b) = (n.clone(), ui.clone(), b.clone());
                cmd::background(
                    move || net::forget(&n),
                    move |r| match r {
                        Ok(_) => {
                            if let Some(row) = b.parent() {
                                row.set_visible(false);
                            }
                            fill_wifi(&ui, None);
                        }
                        Err(e) => {
                            b.set_sensitive(true);
                            window::toast(&format!("{e}"));
                        }
                    },
                );
            });
            let r = widgets::icon_row("network-wireless-symbolic", &esc(&name), &desc, Some(forget.upcast_ref()));
            g.add(&r);
        }
    });
}

/// The right side: the network you're on, or what to do when there isn't one.
fn show_current(ui: &Rc<WifiUi>, data: &WifiData) {
    let pv = &ui.pv;
    clear(pv);
    clear(&ui.forget);
    ui.generation.set(ui.generation.get() + 1);
    let (Some(name), Some(link)) = (data.current.clone(), data.link.clone()) else {
        let (title, line) = match (data.on, data.wired.first()) {
            (_, Some(wired)) => ("Not on Wi-Fi".to_string(), format!("Connected by cable through {wired}.")),
            (false, None) => ("Wi-Fi is off".to_string(), "Turn it on to see the networks nearby.".to_string()),
            (true, None) => ("Not connected".to_string(), "Pick a network on the left to join it.".to_string()),
        };
        pv.append(&pv_head(&title, &line, &[]));
        let empty = nyx_ui::widgets::empty_state("network-wireless-offline-symbolic", &title, &line, None);
        empty.add_css_class("cn-empty");
        pv.append(&empty);
        return;
    };
    let refill: Rc<dyn Fn()> = {
        let ui = ui.clone();
        Rc::new(move || fill_wifi(&ui, None))
    };
    let get = |k: &str| data.settings.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    let auto = get("connection.autoconnect") != Some("no");
    let n2 = name.clone();
    let disconnect = action_button("Disconnect", "Disconnecting…", move || net::disconnect(&n2), refill.clone());
    pv.append(&pv_head(
        &link.ssid,
        &format!("Connected{}", if auto { " · joins automatically" } else { "" }),
        &[disconnect.upcast_ref()],
    ));

    // Signal and live traffic.
    let sig = flow(2, false);
    sig.add_css_class("cn-card");
    sig.add_css_class("cn-wf-signal");
    sig.set_column_spacing(20);
    let gauge = widgets::hbox(16);
    gauge.append(&ring_with(84, link.signal as f64 / 100.0, &format!("{}%", link.signal)));
    let text = widgets::vbox(3);
    text.set_valign(gtk::Align::Center);
    text.append(&widgets::label(quality(link.signal), "cn-big"));
    let band = match link.freq {
        2400..=2499 => "2.4 GHz".to_string(),
        4900..=5924 => "5 GHz".to_string(),
        5925..=7124 => "6 GHz".to_string(),
        0 => String::new(),
        f => format!("{f} MHz"),
    };
    let mut bits: Vec<String> = Vec::new();
    if !band.is_empty() {
        bits.push(band);
    }
    if !link.channel.is_empty() {
        bits.push(format!("channel {}", link.channel));
    }
    if !link.rate.is_empty() && !link.rate.starts_with("0 ") {
        bits.push(link.rate.replace("Mbit/s", "Mb/s"));
    }
    let l = widgets::label(&bits.join(" · "), "dim");
    l.set_wrap(true);
    l.set_xalign(0.0);
    text.append(&l);
    gauge.append(&text);
    flow_add(&sig, &gauge);
    let traffic = widgets::vbox(6);
    traffic.set_hexpand(true);
    let th = widgets::hbox(6);
    let tt = widgets::label("Traffic", "cn-strong");
    tt.set_hexpand(true);
    th.append(&tt);
    let down = chip("Down", true);
    let up = chip("Up", false);
    th.append(&down);
    th.append(&up);
    traffic.append(&th);
    let chart = charts::line_chart(2, None, 64.0 * 1024.0, 56);
    chart.area.set_vexpand(false);
    traffic.append(&chart.area);
    traffic.set_valign(gtk::Align::Center);
    flow_add(&sig, &traffic);
    pv.append(&sig);

    // Addresses and what it is.
    let get_detail = |k: &str| data.details.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    let mac = match get("802-11-wireless.cloned-mac-address").unwrap_or("") {
        "random" => "Changes every time it joins",
        "stable" => "Private for this network",
        "" | "preserve" | "permanent" => "The Wi-Fi card's own",
        _ => "Set by hand",
    };
    let today = data.device.as_deref().map(net::traffic).unwrap_or_default();
    let total = facts(&[
        ("Address", get_detail("IP address").unwrap_or_else(|| "—".into())),
        ("Router", get_detail("Gateway").unwrap_or_else(|| "—".into())),
        ("DNS", get_detail("DNS").unwrap_or_else(|| "From the network".into())),
        ("Security", if link.security.is_empty() || link.security == "--" { "None".into() } else { link.security.clone() }),
        ("Hardware address", mac.to_string()),
        (
            "Since it came up",
            format!("{} down · {} up", sysinfo::bytes_text(today.0), sysinfo::bytes_text(today.1)),
        ),
    ]);
    pv.append(&total);

    // The choices that belong to this network, and sharing it.
    let row = flow(2, true);
    let opts = widgets::vbox(0);
    opts.add_css_class("settings-card");
    opts.add_css_class("cn-wf-opts");
    let n3 = name.clone();
    let (r, _) = widgets::switch_row("Join automatically", "When it's in range.", auto, move |on| {
        let n3 = n3.clone();
        cmd::run_async(&["nmcli", "connection", "modify", "id", &n3, "connection.autoconnect", if on { "yes" } else { "no" }], |r| {
            if let Err(e) = r {
                window::toast(&format!("{e}"));
            }
        });
    });
    widgets::keywords("autoconnect automatic join");
    opts.append(&r);
    if let Some((current, available)) = &data.band {
        let mut options = vec![("auto".to_string(), "Automatic".to_string())];
        options.extend(available.iter().map(|b| (b.clone(), format!("{b} GHz"))));
        let (r, dd) = widgets::choice_row("Wi-Fi band", "", options, current, move |b| {
            cmd::run_async(&["omarchy-network-band", &b], |r| match r {
                Ok(_) => window::toast("Band changed"),
                Err(e) => window::toast(&format!("{e}")),
            });
        });
        dd.set_tooltip_text(Some("2.4 GHz reaches further; 5 and 6 GHz are faster."));
        widgets::keywords("2.4 5ghz 6ghz frequency channel band");
        opts.append(&r);
    }
    if !link.security.contains("802.1X") && cmd::present("qrencode") {
        opts.append(&share_row(&name, &link));
    }
    flow_add(&row, &opts);
    pv.append(&row);

    let n4 = name.clone();
    let forget = widgets::confirm_button("Forget this network", "Click again to forget", move |b| {
        b.set_sensitive(false);
        let (n4, refill) = (n4.clone(), refill.clone());
        cmd::background(
            move || net::forget(&n4),
            move |r| {
                if let Err(e) = r {
                    window::toast(&format!("{e}"));
                }
                refill();
            },
        );
    });
    forget.add_css_class("flat");
    forget.add_css_class("cn-danger");
    forget.set_valign(gtk::Align::Center);
    ui.forget.append(&forget);

    // Sample the traffic once a second while it's on screen.
    if let Some(dev) = data.device.clone() {
        let generation = ui.generation.get();
        let weak = Rc::downgrade(ui);
        let last = Rc::new(Cell::new((net::traffic(&dev), glib::monotonic_time())));
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(ui) = weak.upgrade() else { return glib::ControlFlow::Break };
            if ui.generation.get() != generation || chart.area.root().is_none() {
                return glib::ControlFlow::Break;
            }
            if !chart.area.is_mapped() {
                return glib::ControlFlow::Continue;
            }
            let ((rx0, tx0), t0) = last.get();
            let (rx, tx) = net::traffic(&dev);
            let t = glib::monotonic_time();
            let secs = ((t - t0) as f64 / 1e6).max(0.001);
            let (d, u) = (rx.saturating_sub(rx0) as f64 / secs, tx.saturating_sub(tx0) as f64 / secs);
            last.set(((rx, tx), t));
            chart.push(&[d, u]);
            down.set_text(&format!("Down {}", sysinfo::rate_text(d)));
            up.set_text(&format!("Up {}", sysinfo::rate_text(u)));
            glib::ControlFlow::Continue
        });
    }
}

/// A code a phone can scan to join, shown when asked (it holds the password).
/// Share in the network's card: Show code opens the code a phone scans to join, so the
/// password is only drawn when asked for.
fn share_row(name: &str, link: &net::Link) -> gtk::Box {
    let show = gtk::MenuButton::new();
    show.set_label("Show code");
    let pop = gtk::Popover::new();
    let slot = widgets::vbox(8);
    slot.add_css_class("cn-qr-pop");
    pop.set_child(Some(&slot));
    show.set_popover(Some(&pop));
    let r = widgets::row("Share", "A phone scans the code to join without typing the password.", Some(show.upcast_ref()));
    widgets::keywords("qr code share password phone");
    let open = link.security.is_empty() || link.security == "--";
    let (name, ssid) = (name.to_string(), link.ssid.clone());
    pop.connect_show(move |_| {
        if slot.first_child().is_some() {
            return;
        }
        slot.append(&widgets::label("Making the code…", "dim"));
        let (name, ssid, slot) = (name.clone(), ssid.clone(), slot.clone());
        let for_code = ssid.clone();
        cmd::background(
            move || {
                let pw = if open { Some(String::new()) } else { net::password(&name) };
                pw.and_then(|pw| qr_png(&wifi_qr_text(&for_code, &pw, open)))
            },
            move |png| {
                clear(&slot);
                match png.and_then(|p| gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(p)).ok()) {
                    Some(tex) => {
                        let pic = gtk::Picture::for_paintable(&tex);
                        pic.set_can_shrink(true);
                        pic.set_size_request(180, 180);
                        slot.append(&pic);
                        slot.append(&widgets::label(&ssid, "cn-strong"));
                    }
                    None => slot.append(&widgets::label("Couldn't read this network's password.", "dim")),
                }
            },
        );
    });
    r
}

/// The `WIFI:` text phones read from a code.
fn wifi_qr_text(ssid: &str, password: &str, open: bool) -> String {
    let e = |s: &str| s.chars().flat_map(|c| if "\\;,:\"".contains(c) { vec!['\\', c] } else { vec![c] }).collect::<String>();
    if open { format!("WIFI:T:nopass;S:{};;", e(ssid)) } else { format!("WIFI:T:WPA;S:{};P:{};;", e(ssid), e(password)) }
}

/// A QR code as PNG, from `qrencode` (the text goes on stdin, never the command line).
fn qr_png(text: &str) -> Option<Vec<u8>> {
    use std::io::Write;
    let mut child = std::process::Command::new("qrencode")
        .args(["-t", "PNG", "-s", "6", "-m", "2", "-o", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(text.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
}

// ----- Bluetooth -----

thread_local! {
    /// The device shown on the right, kept while the page is rebuilt.
    static PICKED: RefCell<Option<String>> = const { RefCell::new(None) };
}

struct BtData {
    powered: bool,
    name: Option<String>,
    devices: Vec<(bt::Device, bt::Info)>,
}

fn load_bt() -> BtData {
    let devices = bt::devices().into_iter().map(|d| {
        let info = if d.paired { bt::info(&d.address) } else { bt::Info::default() };
        (d, info)
    });
    BtData { powered: bt::powered(), name: bt::adapter_name(), devices: devices.collect() }
}

/// A kind of device from BlueZ's icon name: (symbolic icon, what to call it).
fn kind(icon: &str) -> (&'static str, &'static str) {
    match icon {
        "audio-headset" | "audio-headphones" => ("audio-headphones-symbolic", "Headphones"),
        "audio-card" => ("audio-speakers-symbolic", "Speaker"),
        "input-mouse" => ("input-mouse-symbolic", "Mouse"),
        "input-keyboard" => ("input-keyboard-symbolic", "Keyboard"),
        "input-gaming" => ("input-gaming-symbolic", "Game controller"),
        "input-tablet" => ("input-tablet-symbolic", "Drawing tablet"),
        "phone" => ("phone-symbolic", "Phone"),
        "computer" => ("computer-symbolic", "Computer"),
        "camera-video" | "camera-photo" => ("camera-video-symbolic", "Camera"),
        "printer" => ("printer-symbolic", "Printer"),
        "multimedia-player" => ("multimedia-player-symbolic", "Media player"),
        _ => ("bluetooth-symbolic", "Device"),
    }
}

fn is_audio(icon: &str) -> bool {
    icon.starts_with("audio-")
}

/// A battery: a small bar and its percentage.
fn battery(percent: u8) -> gtk::Box {
    let b = widgets::hbox(6);
    b.set_valign(gtk::Align::Center);
    let m = charts::meter(6);
    m.never_warn();
    m.set(vec![percent as f64 / 100.0]);
    m.area.set_size_request(28, -1);
    m.area.set_hexpand(false);
    b.append(&m.area);
    b.append(&widgets::label(&format!("{percent}%"), "cn-battery"));
    b
}

struct BtUi {
    page_desc: Option<gtk::Label>,
    mine: gtk::Box,
    count: gtk::Label,
    nearby_group: widgets::Group,
    nearby: gtk::Box,
    controls: gtk::Box,
    pv: gtk::Box,
    unpair: gtk::Box,
    data: RefCell<Option<Rc<BtData>>>,
}

fn bluetooth(page: &Page) {
    let pv = page.split();
    steady(&pv);
    steady(&pv);
    let pv_body = widgets::vbox(14);
    pv.add(&pv_body);
    let foot = pv.foot("dialog-information-symbolic", "Unpair to stop it reconnecting; it has to be paired again to use it.");
    let unpair = widgets::hbox(0);
    if let Some(row) = foot.parent().and_downcast::<gtk::Box>() {
        row.append(&unpair);
    }

    let g = page.group("");
    let sw = gtk::Switch::new();
    sw.set_active(bt::powered());
    let top = widgets::icon_row("bluetooth-symbolic", "Bluetooth", "…", Some(sw.upcast_ref()));
    g.add(&top);

    let count = widgets::label("", "cn-note");
    let g = titled_group(page, "My devices", &[count.upcast_ref()]);
    let controls = g.wrapper.clone();
    widgets::keywords("bluetooth pair headphones earbuds keyboard mouse speaker controller connect forget");
    let mine = widgets::vbox(0);
    g.add(&mine);
    let pair = widgets::icon_row("list-add-symbolic", "Pair a new device…", "Put it in pairing mode first.", None);
    pair.add_css_class("cn-add-row");
    widgets::keywords("pair new find search discover");
    g.add(&pair);
    let nearby_group = titled_group(page, "Nearby", &[]);
    let nearby = widgets::vbox(0);
    nearby_group.add(&nearby);
    nearby_group.wrapper.set_visible(false);

    let ui = Rc::new(BtUi {
        page_desc: desc_of(&top),
        mine,
        count,
        nearby_group,
        nearby,
        controls,
        pv: pv_body,
        unpair,
        data: RefCell::default(),
    });
    sw.connect_active_notify({
        let ui = ui.clone();
        move |s| {
            let (on, ui) = (s.is_active(), ui.clone());
            cmd::background(
                move || bt::set_power(on),
                move |r| {
                    if let Err(e) = r {
                        window::toast(&format!("{e}"));
                    }
                    fill_bt(&ui);
                },
            );
        }
    });
    {
        let ui = ui.clone();
        let desc = desc_of(&pair);
        on_click(&pair, move || {
            if let Some(d) = &desc {
                d.set_text("Looking for devices… about 10 seconds.");
            }
            let (ui, desc) = (ui.clone(), desc.clone());
            cmd::background(
                || {
                    let _ = bt::set_power(true);
                    bt::scan(10)
                },
                move |_| {
                    if let Some(d) = &desc {
                        d.set_text("Put it in pairing mode first.");
                    }
                    fill_bt(&ui);
                },
            );
        });
    }
    fill_bt(&ui);
}

fn fill_bt(ui: &Rc<BtUi>) {
    let ui = ui.clone();
    cmd::background(load_bt, move |data| show_bt(&ui, Rc::new(data)));
}

fn show_bt(ui: &Rc<BtUi>, data: Rc<BtData>) {
    *ui.data.borrow_mut() = Some(data.clone());
    if let Some(d) = &ui.page_desc {
        d.set_text(&match (data.powered, &data.name) {
            (false, _) => "Off".to_string(),
            (true, Some(n)) => format!("On · other devices see this computer as {n}"),
            (true, None) => "On".to_string(),
        });
    }
    ui.controls.set_sensitive(data.powered);
    ui.nearby_group.wrapper.set_sensitive(data.powered);
    clear(&ui.mine);
    clear(&ui.nearby);
    widgets::begin_section("bluetooth");
    let refill: Rc<dyn Fn()> = {
        let ui = ui.clone();
        Rc::new(move || fill_bt(&ui))
    };
    let paired: Vec<&(bt::Device, bt::Info)> = data.devices.iter().filter(|(d, _)| d.paired).collect();
    ui.count.set_text(&match paired.len() {
        0 => String::new(),
        n => format!("{n} paired"),
    });
    // The device shown: the one picked before, else the first connected, else the first.
    let picked = PICKED
        .with(|p| p.borrow().clone())
        .filter(|a| paired.iter().any(|(d, _)| &d.address == a))
        .or_else(|| paired.iter().find(|(d, _)| d.connected).or(paired.first()).map(|(d, _)| d.address.clone()));
    PICKED.with(|p| *p.borrow_mut() = picked.clone());
    if paired.is_empty() {
        ui.mine.append(&widgets::row("No devices yet", "Pair one below; it shows up here.", None));
    }
    for (d, info) in &paired {
        let (icon, what) = kind(&info.icon);
        let end = widgets::hbox(8);
        if let Some(p) = info.battery {
            end.append(&battery(p));
        }
        let chevron = gtk::Image::from_icon_name("go-next-symbolic");
        chevron.add_css_class("chevron");
        end.append(&chevron);
        let state = if d.connected { "Connected" } else { "Not connected" };
        let r = widgets::icon_row(icon, &esc(&d.name), &format!("{state} · {what}"), Some(end.upcast_ref()));
        if d.connected {
            r.add_css_class("cn-connected");
        }
        if picked.as_deref() == Some(d.address.as_str()) {
            r.add_css_class("cn-picked");
        }
        let (ui2, a) = (ui.clone(), d.address.clone());
        on_click(&r, move || {
            PICKED.with(|p| *p.borrow_mut() = Some(a.clone()));
            if let Some(data) = ui2.data.borrow().clone() {
                show_bt(&ui2, data);
            }
        });
        ui.mine.append(&r);
    }
    let nearby: Vec<&bt::Device> = data.devices.iter().filter(|(d, _)| !d.paired).map(|(d, _)| d).collect();
    ui.nearby_group.wrapper.set_visible(!nearby.is_empty());
    for d in nearby {
        let a = d.address.clone();
        let pair = action_button("Pair", "Pairing…", move || bt::device("pair", &a), refill.clone());
        let r = widgets::icon_row("bluetooth-symbolic", &esc(&d.name), "Nearby, not paired", Some(pair.upcast_ref()));
        ui.nearby.append(&r);
    }
    let shown = picked.and_then(|a| paired.iter().find(|(d, _)| d.address == a).map(|(d, i)| (d.clone(), i.clone())));
    show_device(ui, &data, shown, refill);
}

/// The right side: the picked device.
fn show_device(ui: &Rc<BtUi>, data: &BtData, shown: Option<(bt::Device, bt::Info)>, refill: Rc<dyn Fn()>) {
    let pv = &ui.pv;
    clear(pv);
    clear(&ui.unpair);
    let Some((d, info)) = shown else {
        let (title, line) = if data.powered {
            ("No devices yet", "Choose Pair a new device on the left, with the device in pairing mode.")
        } else {
            ("Bluetooth is off", "Turn it on to connect your devices.")
        };
        pv.append(&pv_head(title, line, &[]));
        let empty = nyx_ui::widgets::empty_state("bluetooth-disabled-symbolic", title, line, None);
        empty.add_css_class("cn-empty");
        pv.append(&empty);
        return;
    };
    let (icon, what) = kind(&info.icon);
    let a = d.address.clone();
    let action = if d.connected {
        action_button("Disconnect", "Disconnecting…", move || bt::device("disconnect", &a), refill.clone())
    } else {
        action_button("Connect", "Connecting…", move || bt::device("connect", &a), refill.clone())
    };
    action.set_sensitive(data.powered);
    pv.append(&pv_head(&d.name, &format!("{what} · paired"), &[action.upcast_ref()]));

    // The device, big, with its state and battery.
    let hero = card("cn-bt-hero", 20, false);
    let art = gtk::Overlay::new();
    let tile = widgets::vbox(0);
    tile.add_css_class("cn-bt-art");
    let i = gtk::Image::from_icon_name(icon);
    i.set_pixel_size(48);
    i.set_vexpand(true);
    i.set_valign(gtk::Align::Center);
    tile.append(&i);
    art.set_child(Some(&tile));
    if d.connected {
        let dot = widgets::hbox(0);
        dot.add_css_class("cn-bt-live");
        dot.set_halign(gtk::Align::End);
        dot.set_valign(gtk::Align::End);
        art.add_overlay(&dot);
    }
    art.set_valign(gtk::Align::Center);
    hero.append(&art);
    let text = widgets::vbox(4);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let state = widgets::label(if d.connected { "Connected" } else { "Not connected" }, "cn-big");
    if d.connected {
        state.add_css_class("cn-okt");
    }
    text.append(&state);
    let line = match (d.connected, info.trusted) {
        (true, _) => format!("Connected to {}", data.name.clone().unwrap_or_else(|| "this computer".into())),
        (false, true) => "It connects by itself when it's on and in range.".to_string(),
        (false, false) => "Connect it with the button above.".to_string(),
    };
    let l = widgets::label(&line, "dim");
    l.set_wrap(true);
    l.set_xalign(0.0);
    text.append(&l);
    if let Some(p) = info.battery {
        let b = widgets::hbox(10);
        b.set_margin_top(6);
        b.append(&widgets::label("Battery", "cn-fact-key"));
        let m = charts::meter(10);
        m.set(vec![p as f64 / 100.0]);
        m.never_warn();
        m.area.set_size_request(80, -1);
        m.area.set_hexpand(false);
        b.append(&m.area);
        b.append(&widgets::label(&format!("{p}%"), "cn-strong"));
        text.append(&b);
    }
    hero.append(&text);
    pv.append(&hero);

    // What you can choose for it.
    let opts = widgets::vbox(0);
    opts.add_css_class("settings-card");
    let a = d.address.clone();
    let sw = gtk::Switch::new();
    sw.set_active(info.trusted);
    sw.connect_active_notify(move |s| {
        let on = s.is_active();
        cmd::run_async(&["bluetoothctl", if on { "trust" } else { "untrust" }, &a], |r| {
            if let Err(e) = r {
                window::toast(&format!("{e}"));
            }
        });
    });
    opts.append(&widgets::icon_row(
        "view-refresh-symbolic",
        "Connect when it's in range",
        "It reconnects by itself, without asking.",
        Some(sw.upcast_ref()),
    ));
    widgets::keywords("trust trusted reconnect automatically");
    if is_audio(&info.icon) {
        let (r, _) = widgets::nav_row(Some("audio-volume-high-symbolic"), "Sound", "Choose it for output and the microphone.", "Sound", || {
            window::navigate("audio")
        });
        opts.append(&r);
    }
    let entry = gtk::Entry::new();
    entry.set_text(if info.alias.is_empty() { &d.name } else { &info.alias });
    entry.set_width_chars(16);
    let (a, refill2) = (d.address.clone(), refill.clone());
    let rename = move |e: &gtk::Entry| {
        let name = e.text().trim().to_string();
        if name.is_empty() {
            return;
        }
        let (a, refill) = (a.clone(), refill2.clone());
        cmd::background(
            move || set_alias(&a, &name),
            move |r| {
                if let Err(e) = r {
                    window::toast(&format!("Couldn't rename it: {e}"));
                }
                refill();
            },
        );
    };
    entry.connect_activate(rename);
    opts.append(&widgets::icon_row("document-edit-symbolic", "Name", "What this computer calls it. Press Enter to keep it.", Some(entry.upcast_ref())));
    widgets::keywords("rename alias name");
    opts.set_sensitive(data.powered);
    pv.append(&opts);

    let mut f = vec![("Kind", what.to_string()), ("Address", d.address.clone())];
    if let Some(p) = info.battery {
        f.push(("Battery", format!("{p}%")));
    }
    pv.append(&facts(&f));

    let (a, refill) = (d.address.clone(), refill.clone());
    // Removes the pairing itself (the device has to be paired again), so it's called
    // that, apart from Disconnect.
    let unpair = widgets::confirm_button("Unpair", "Click again to unpair", move |b| {
        b.set_sensitive(false);
        let (a, refill) = (a.clone(), refill.clone());
        PICKED.with(|p| *p.borrow_mut() = None);
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
    unpair.add_css_class("flat");
    unpair.add_css_class("cn-danger");
    unpair.set_valign(gtk::Align::Center);
    ui.unpair.append(&unpair);
}

/// Rename a device through BlueZ's D-Bus property (bluetoothctl only renames the one it's talking to).
fn set_alias(address: &str, name: &str) -> anyhow::Result<String> {
    let dev = format!("dev_{}", address.replace(':', "_"));
    let mut last = anyhow::anyhow!("no Bluetooth adapter has this device");
    for hci in 0..4 {
        let path = format!("/org/bluez/hci{hci}/{dev}");
        match cmd::run(&["busctl", "--system", "set-property", "org.bluez", &path, "org.bluez.Device1", "Alias", "s", name]) {
            Ok(o) => return Ok(o),
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_prefers_the_choice() {
        let (b, a) = parse_band("band\t6\navailable\t2.4 5 6\nselected\tauto\n").unwrap();
        assert_eq!(b, "auto");
        assert_eq!(a, ["2.4", "5", "6"]);
        assert_eq!(parse_band("band\t5\navailable\t2.4 5\n").unwrap().0, "5");
        assert!(parse_band("nothing").is_none());
    }

    #[test]
    fn qr_text_is_escaped() {
        assert_eq!(wifi_qr_text("Café;1", "p:w", false), "WIFI:T:WPA;S:Café\\;1;P:p\\:w;;");
        assert_eq!(wifi_qr_text("Open", "", true), "WIFI:T:nopass;S:Open;;");
    }
}
