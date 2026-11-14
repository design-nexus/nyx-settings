//! Sharing: nearby sharing, your computers, NAS folders and what the Share
//! Sheet shows. Everything goes through Nyx Share's service (Share1); Settings
//! keeps none of it.

use crate::dialog::{self, Field};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::{gio, glib, prelude::*};
use nyx_share_client::{
    Client, ContentKind, ComputerInfo, Destination, Direction, Job, JobState, LinkedDevice, NasInfo, NearbyStatus, NearbyUpdate,
    PreferencesUpdate, ProviderInfo,
};
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;

struct Snapshot {
    nearby: NearbyStatus,
    computers: Vec<ComputerInfo>,
    devices: Value,
    nas: Vec<NasInfo>,
    destinations: Vec<Destination>,
    providers: Vec<ProviderInfo>,
    jobs: Vec<Job>,
    history: usize,
}

/// The transfers as they stand, for redrawing the page when they change.
pub fn signature() -> String {
    Client::connect()
        .and_then(|c| c.jobs())
        .map(|jobs| {
            jobs.iter()
                .take(12)
                .map(|j| format!("{} {:?} {}", j.id, j.state, j.fraction().map(|f| (f * 10.0) as u32).unwrap_or(0)))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn load() -> Result<Snapshot, String> {
    let c = Client::connect().map_err(|e| e.to_string())?;
    let text = |e: nyx_share_client::Error| e.to_string();
    Ok(Snapshot {
        nearby: c.nearby().map_err(text)?,
        computers: c.computers().map_err(text)?,
        devices: c.devices().map_err(text)?,
        nas: c.nas().map_err(text)?,
        destinations: c.destinations(&[], 0).map_err(text)?,
        providers: c.capabilities().map_err(text)?.providers,
        jobs: c.jobs().map_err(text)?,
        history: c.preferences().map_err(text)?.history_limit(),
    })
}

thread_local! {
    /// Refills the sub-page of linked devices, NAS folders and the Share Sheet while it's open.
    static MORE: RefCell<Option<Rc<dyn Fn()>>> = RefCell::new(None);
}

fn refresh() {
    window::rebuild_if_built("sharing");
    if let Some(fill) = MORE.with(|m| m.borrow().clone()) {
        fill();
    }
}

/// Run a call off the main thread; show what went wrong, then redraw.
fn act(work: impl FnOnce(&Client) -> Result<(), nyx_share_client::Error> + Send + 'static) {
    cmd::background(
        move || Client::connect().and_then(|c| work(&c)).map_err(|e| e.to_string()),
        |result: Result<(), String>| {
            if let Err(e) = result {
                window::toast(&e);
            }
            refresh();
        },
    );
}

fn clear(holder: &gtk::Box) {
    widgets::forget_rows(holder);
    while let Some(child) = holder.first_child() {
        holder.remove(&child);
    }
}

fn holder(group: &widgets::Group) -> gtk::Box {
    let b = widgets::vbox(0);
    group.add(&b);
    b
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn tiled(r: gtk::Box, icon: &str) -> gtk::Box {
    r.prepend(&widgets::tile(icon));
    r.add_css_class("with-tile");
    r
}

fn chip(text: &str, class: &str) -> gtk::Box {
    let l = widgets::label(text, "cn-chip");
    if !class.is_empty() {
        l.add_css_class(class);
    }
    // In a box, so the card's padding for labels in rows doesn't reach it.
    let b = widgets::hbox(0);
    b.set_valign(gtk::Align::Center);
    b.append(&l);
    b
}

/// Make a row open something when clicked.
fn clickable(r: &gtk::Box, open: impl Fn() + 'static) {
    r.add_css_class("cn-click");
    r.set_cursor_from_name(Some("pointer"));
    r.set_focusable(true);
    let click = gtk::GestureClick::new();
    click.connect_released(move |_, _, _, _| open());
    r.add_controller(click);
}

pub fn build(page: &Page) {
    widgets::keywords(
        "share sharing localsend nearby airdrop send receive tailscale computers linked devices link phone bluetooth nas smb sftp nfs flux phone share sheet",
    );
    let pv = page.split();
    super::connectivity::steady(&pv);
    let head = pv.head("Nearby right now", "Asking Share…", &[]);
    let line = head.first_child().and_then(|t| t.last_child()).and_downcast::<gtk::Label>();
    let badge = widgets::hbox(0);
    badge.set_valign(gtk::Align::Center);
    head.append(&badge);
    let radar = widgets::vbox(0);
    pv.add(&radar);
    let thead = widgets::hbox(8);
    thead.add_css_class("cn-sh-thead");
    let tt = widgets::label("Transfers", "cn-strong");
    tt.set_hexpand(true);
    tt.set_xalign(0.0);
    thead.append(&tt);
    let open = gtk::Button::new();
    open.add_css_class("flat");
    open.add_css_class("cn-link");
    let oc = widgets::hbox(6);
    oc.append(&gtk::Label::new(Some("Open Share")));
    oc.append(&gtk::Image::from_icon_name("window-new-symbolic"));
    open.set_child(Some(&oc));
    open.set_tooltip_text(Some("Send from the Share window, and see what you sent and received"));
    open.connect_clicked(|_| cmd::spawn(&["nyx-share-sheet"]));
    thead.append(&open);
    pv.add(&thead);
    let transfers = widgets::vbox(0);
    transfers.add_css_class("settings-card");
    pv.add(&transfers);

    let nearby = page.group("Nearby sharing");
    let nearby_rows = holder(&nearby);
    nearby_rows.append(&widgets::row("Asking Share…", "", None));
    let add = nyx_ui::widgets::labeled_button("list-add-symbolic", "Add…");
    add.add_css_class("flat");
    add.set_tooltip_text(Some("By its address on the network or tailnet. The pin beside a nearby device in Share saves it too."));
    add.connect_clicked(|_| add_computer());
    let computers = super::connectivity::titled_group(page, "My computers", &[add.upcast_ref()]);
    let computer_rows = holder(&computers);
    let more = page.group("");
    let (r, _) = widgets::nav_row(
        Some("view-grid-symbolic"),
        "Linked devices, NAS folders and the Share Sheet",
        "",
        "",
        open_more,
    );
    widgets::keywords("linked devices nas folders share sheet destinations pin hide order default device key history");
    more.add(&r);

    cmd::background(load, move |result: Result<Snapshot, String>| {
        clear(&nearby_rows);
        match result {
            Ok(s) => {
                fill_nearby(&nearby_rows, &s.nearby);
                fill_computers(&computer_rows, &s.computers);
                let me = if s.nearby.alias.is_empty() { "this computer" } else { s.nearby.alias.as_str() };
                if let Some(l) = &line {
                    l.set_text(&format!("Who can see {me}, and who you can send to."));
                }
                badge.append(&visible_chip(&s.nearby));
                fill_radar(&radar, &s);
                fill_transfers(&transfers, &s.jobs);
            }
            Err(e) => {
                for group in [&computers, &more] {
                    group.wrapper.set_visible(false);
                }
                thead.set_visible(false);
                transfers.set_visible(false);
                if let Some(l) = &line {
                    l.set_text("Share isn't running.");
                }
                let empty = nyx_ui::widgets::empty_state(
                    "folder-publicshare-symbolic",
                    "Share isn't available",
                    "Nearby sharing, your computers and the Share Sheet come from Nyx Share.",
                    None,
                );
                empty.add_css_class("cn-empty");
                radar.append(&empty);
                nearby_rows.append(&widgets::banner(
                    &glib::markup_escape_text(&format!(
                        "Share isn't available: {e}. If it isn't installed, install it with nyx install share."
                    )),
                    true,
                ));
            }
        }
    });
}

fn visible_chip(s: &NearbyStatus) -> gtk::Box {
    if s.enabled {
        chip("Visible now", "accent")
    } else if s.visibility == "off" {
        chip("Hidden", "")
    } else {
        chip("Not visible now", "")
    }
}

fn fill_nearby(rows: &gtk::Box, s: &NearbyStatus) {
    let (r, _) = widgets::switch_row(
        "Visible to nearby devices",
        "While Share is open, and a minute after you share.",
        s.visibility != "off",
        |on| {
            let visibility = Some(if on { "running" } else { "off" }.to_string());
            act(move |c| c.set_nearby(&NearbyUpdate { visibility, ..Default::default() }));
        },
    );
    r.set_tooltip_text(Some("Every offer asks you first, unless it comes from a trusted computer."));
    rows.append(&tiled(r, "view-reveal-symbolic"));
    if let Some(problem) = &s.problem {
        rows.append(&widgets::banner(&glib::markup_escape_text(problem), true));
    }
    let (r, e) = widgets::entry_row("Name", "What nearby devices see", &s.alias, "This computer", |name| {
        let name = name.trim().to_string();
        if !name.is_empty() {
            act(move |c| c.set_nearby(&NearbyUpdate { alias: Some(name), ..Default::default() }));
        }
    });
    e.set_width_chars(10);
    widgets::keywords("device name alias");
    rows.append(&tiled(r, "computer-symbolic"));
    let folder = std::path::Path::new(&s.receive_folder);
    let shown = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| s.receive_folder.clone());
    let current = s.receive_folder.clone();
    let (r, v) = widgets::nav_row(Some("folder-download-symbolic"), "Save what you receive to", "", &shown, move || {
        let dialog = gtk::FileDialog::builder().title("Save received files to").accept_label("Choose").modal(true).build();
        dialog.set_initial_folder(Some(&gio::File::for_path(&current)));
        dialog.select_folder(window::window().as_ref(), gio::Cancellable::NONE, |res| {
            let Some(path) = res.ok().and_then(|f| f.path()) else { return };
            let folder = Some(path.display().to_string());
            act(move |c| c.set_nearby(&NearbyUpdate { receive_folder: folder, ..Default::default() }));
        });
    });
    v.set_tooltip_text(Some(&s.receive_folder));
    widgets::keywords("save received files to folder downloads");
    rows.append(&r);
    let (desc, usable) = match s.tailscale_state.as_str() {
        "running" => ("Reach your devices from any network. They need LocalSend open.", true),
        "stopped" => ("Tailscale isn't connected on this computer.", true),
        _ => ("Install Tailscale to reach your devices from any network.", false),
    };
    let (r, sw) = widgets::switch_row("Also look over Tailscale", desc, s.tailscale, |on| {
        act(move |c| c.set_nearby(&NearbyUpdate { tailscale: Some(on), ..Default::default() }));
    });
    sw.set_sensitive(usable);
    widgets::keywords("find devices over tailscale tailnet");
    rows.append(&tiled(r, "network-vpn-symbolic"));
}

/// A device key as four-character groups, shortened in the middle.
fn key_text(f: &str) -> String {
    if f.len() >= 16 {
        format!("{} {} … {} {}", &f[..4], &f[4..8], &f[f.len() - 8..f.len() - 4], &f[f.len() - 4..])
    } else {
        "Made when nearby sharing first starts".into()
    }
}

fn presence(c: &ComputerInfo) -> (&'static str, &'static str) {
    match c.presence.as_str() {
        "lan" => ("On the network", "ok"),
        "tailscale" => ("Over Tailscale", "ok"),
        "offline" => ("Offline", ""),
        _ => ("Checking", ""),
    }
}

fn fill_computers(rows: &gtk::Box, list: &[ComputerInfo]) {
    if list.is_empty() {
        rows.append(&widgets::icon_row(
            "computer-symbolic",
            "None yet",
            "Computers you keep, known by their device key whatever their address.",
            None,
        ));
    }
    for c in list {
        let (state, class) = presence(c);
        let end = widgets::hbox(8);
        end.append(&chip(state, class));
        let chevron = gtk::Image::from_icon_name("go-next-symbolic");
        chevron.add_css_class("chevron");
        end.append(&chevron);
        let desc = if c.trusted { "Trusted · sends without asking" } else { "Asks first" };
        let r = widgets::icon_row("computer-symbolic", &esc(&c.name), desc, Some(end.upcast_ref()));
        widgets::keywords("trusted computer remove");
        let c = c.clone();
        clickable(&r, move || computer_page(&c));
        rows.append(&r);
    }
}

fn add_computer() {
    dialog::ask(
        "Add a computer",
        "Its IP address or Tailscale name, with :port if not 53317. It needs LocalSend or Share running, and nearby sharing on here.",
        vec![Field::text("192.168.1.20 or laptop.tailnet.ts.net")],
        "Add",
        None,
        |values, _| {
            let address = values.first().map(|s| s.trim().to_string()).unwrap_or_default();
            if address.is_empty() {
                return Some("Give an address".into());
            }
            act(move |c| c.add_computer(&address).map(|_| ()));
            None
        },
    );
}

/// One of your computers, a level down: whether it's trusted, how it's reached, and removing it.
fn computer_page(c: &ComputerInfo) {
    let win: Rc<RefCell<Option<gtk::Window>>> = Rc::default();
    let w2 = win.clone();
    let (c, title) = (c.clone(), c.name.clone());
    let w = widgets::subpage(&title, 480, move |page| {
        let g = page.group("");
        let edited = c.clone();
        let (r, _) = widgets::switch_row("Trusted", "Sends files to you without asking first.", c.trusted, move |on| {
            let mut c = edited.clone();
            c.trusted = on;
            act(move |client| client.save_computer(&c));
        });
        g.add(&tiled(r, "starred-symbolic"));
        let (r, _) = widgets::info_row("Now", presence(&c).0);
        g.add(&r);
        if let Some(a) = &c.lan {
            let (r, _) = widgets::info_row("On the network", a);
            g.add(&r);
        }
        if let Some(a) = &c.tailscale {
            let (r, _) = widgets::info_row("Over Tailscale", a);
            g.add(&r);
        }
        if !c.fingerprint.is_empty() {
            let (r, l) = widgets::info_row("Device key", &key_text(&c.fingerprint));
            l.set_tooltip_text(Some(&c.fingerprint));
            g.add(&r);
        }
        let g = page.group("");
        let id = c.id.clone();
        let remove = widgets::confirm_button("Remove", "Remove?", move |_| {
            let id = id.clone();
            act(move |client| client.remove_computer(&id));
            if let Some(w) = w2.borrow().as_ref() {
                w.close();
            }
        });
        remove.add_css_class("cn-danger");
        g.add(&widgets::row("Remove this computer", "Share forgets it, and whether it's trusted.", Some(remove.upcast_ref())));
    });
    *win.borrow_mut() = Some(w);
}

// ----- Who's nearby -----

/// Where each device sits around this computer: (halign, valign, side margin, top or bottom margin).
const SLOTS: [(gtk::Align, gtk::Align, i32, i32); 6] = [
    (gtk::Align::Start, gtk::Align::Start, 64, 16),
    (gtk::Align::End, gtk::Align::Start, 64, 16),
    (gtk::Align::Start, gtk::Align::End, 64, 16),
    (gtk::Align::End, gtk::Align::End, 64, 16),
    (gtk::Align::Start, gtk::Align::Center, 8, 0),
    (gtk::Align::End, gtk::Align::Center, 8, 0),
];

/// How a destination is reached, as people say it.
fn reached(d: &Destination) -> String {
    match d.status.as_deref() {
        Some("LAN") => "On the network".into(),
        Some("Tailscale") => "Over Tailscale".into(),
        Some(s) if s.starts_with("Offline") => "Offline".into(),
        Some(s) => s.to_string(),
        None => match d.provider.as_str() {
            "flux" => "Flux".into(),
            "localsend" => "LocalSend".into(),
            _ => String::new(),
        },
    }
}

fn device(d: &Destination, trusted: bool) -> gtk::Box {
    let b = widgets::vbox(3);
    b.add_css_class("cn-sh-dev");
    let how = reached(d);
    if how == "Offline" || !d.available {
        b.add_css_class("off");
    }
    let ic = widgets::hbox(0);
    ic.add_css_class("cn-sh-dev-ic");
    ic.set_halign(gtk::Align::Center);
    let icon = if d.icon.is_empty() { "computer-symbolic" } else { d.icon.as_str() };
    ic.append(&gtk::Image::from_icon_name(icon));
    b.append(&ic);
    let name = widgets::hbox(4);
    name.set_halign(gtk::Align::Center);
    let n = widgets::label(&d.name, "cn-strong");
    n.set_ellipsize(gtk::pango::EllipsizeMode::End);
    n.set_max_width_chars(14);
    name.append(&n);
    if trusted {
        let star = gtk::Image::from_icon_name("starred-symbolic");
        star.add_css_class("cn-sh-star");
        star.set_tooltip_text(Some("Trusted"));
        name.append(&star);
    }
    b.append(&name);
    let h = widgets::label(&how, "cn-note");
    h.set_ellipsize(gtk::pango::EllipsizeMode::End);
    h.set_max_width_chars(16);
    h.set_halign(gtk::Align::Center);
    b.append(&h);
    if let Some(s) = &d.status {
        b.set_tooltip_text(Some(s));
    }
    b
}

fn fill_radar(holder: &gtk::Box, s: &Snapshot) {
    let mut near: Vec<&Destination> = s
        .destinations
        .iter()
        .filter(|d| {
            !d.hidden
                && d.kinds.contains(&ContentKind::File)
                && matches!(d.provider.as_str(), "localsend" | "computers" | "flux" | "devices")
        })
        .collect();
    near.sort_by_key(|d| reached(d) == "Offline" || !d.available);
    let trusted: Vec<&str> = s.computers.iter().filter(|c| c.trusted).map(|c| c.name.as_str()).collect();
    let card = widgets::vbox(0);
    card.add_css_class("preview-card");
    card.add_css_class("cn-sh-near");
    let overlay = gtk::Overlay::new();
    let visible = s.nearby.enabled;
    let rings = widgets::drawing(280, move |cr, p, accent, w, h| {
        let (cx, cy) = (w / 2.0, h / 2.0);
        if visible {
            let g = gtk::cairo::RadialGradient::new(cx, cy, 30.0, cx, cy, 110.0);
            g.add_color_stop_rgba(0.0, accent.0, accent.1, accent.2, 0.16);
            g.add_color_stop_rgba(1.0, accent.0, accent.1, accent.2, 0.0);
            cr.arc(cx, cy, 110.0, 0.0, std::f64::consts::TAU);
            let _ = cr.set_source(&g);
            let _ = cr.fill();
        }
        cr.set_line_width(1.0);
        for (i, r) in [56.0, 96.0, 136.0].into_iter().enumerate() {
            if visible {
                crate::sketch::set(cr, accent, 0.28 - i as f64 * 0.07);
            } else {
                crate::sketch::set(cr, p.text, 0.1);
                cr.set_dash(&[4.0, 5.0], 0.0);
            }
            cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
        }
        cr.set_dash(&[], 0.0);
    });
    overlay.set_child(Some(&rings));
    let me = widgets::vbox(2);
    me.add_css_class("cn-sh-me");
    if !visible {
        me.add_css_class("off");
    }
    me.set_halign(gtk::Align::Center);
    me.set_valign(gtk::Align::Center);
    let inner = widgets::vbox(2);
    inner.set_vexpand(true);
    inner.set_valign(gtk::Align::Center);
    inner.set_halign(gtk::Align::Center);
    inner.append(&gtk::Image::from_icon_name("computer-symbolic"));
    me.append(&inner);
    let alias = if s.nearby.alias.is_empty() { "This computer" } else { s.nearby.alias.as_str() };
    let l = widgets::label(alias, "cn-sh-me-name");
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(9);
    l.set_xalign(0.5);
    inner.append(&l);
    overlay.add_overlay(&me);
    for (d, (h, v, side, edge)) in near.iter().zip(SLOTS) {
        let w = device(d, d.provider == "computers" && trusted.contains(&d.name.as_str()));
        w.set_halign(h);
        w.set_valign(v);
        if h == gtk::Align::Start {
            w.set_margin_start(side);
        } else {
            w.set_margin_end(side);
        }
        if v == gtk::Align::Start {
            w.set_margin_top(edge);
        } else if v == gtk::Align::End {
            w.set_margin_bottom(edge);
        }
        overlay.add_overlay(&w);
    }
    let note = if near.is_empty() {
        Some("No one nearby right now. Devices show up here when Share sees them.".to_string())
    } else if near.len() > SLOTS.len() {
        Some(format!("And {} more in Share", near.len() - SLOTS.len()))
    } else {
        None
    };
    if let Some(text) = note {
        let n = widgets::label(&text, "cn-note");
        n.set_halign(gtk::Align::Center);
        n.set_valign(gtk::Align::End);
        n.set_margin_bottom(10);
        overlay.add_overlay(&n);
    }
    card.append(&overlay);
    holder.append(&card);
}

// ----- Transfers -----

/// "just now", "5 min ago", "14:20", "Oct 31".
fn when(t: i64) -> String {
    let Ok(now) = glib::DateTime::now_local() else { return String::new() };
    let Ok(then) = glib::DateTime::from_unix_local(t) else { return String::new() };
    let secs = now.to_unix() - t;
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        format!("{} min ago", secs / 60)
    } else if then.ymd() == now.ymd() {
        then.format("%H:%M").map(|s| s.to_string()).unwrap_or_default()
    } else {
        then.format("%b %-d").map(|s| s.to_string()).unwrap_or_default()
    }
}

fn fill_transfers(rows: &gtk::Box, jobs: &[Job]) {
    if jobs.is_empty() {
        rows.append(&widgets::row("No transfers yet", "What you share and receive shows here.", None));
        return;
    }
    let b = crate::backend::sysinfo::bytes_text;
    for j in jobs.iter().take(6) {
        let what = j.summary.clone().unwrap_or_else(|| format!("{} items", j.items));
        let title = match j.direction {
            Direction::Receive => format!("{what} from {}", j.destination_name),
            Direction::Send => format!("{what} to {}", j.destination_name),
        };
        let size = (j.bytes_total > 0).then(|| b(j.bytes_total));
        let mut parts: Vec<String> = match j.state {
            JobState::Queued | JobState::Running => match (&j.step, j.fraction()) {
                (Some(step), _) => vec![step.clone()],
                (None, Some(_)) => vec![format!("Sending · {} of {}", b(j.bytes_done), b(j.bytes_total))],
                (None, None) => vec!["Sending".into()],
            },
            JobState::Completed => {
                let done = if j.direction == Direction::Receive { "Received" } else { "Sent" };
                [Some(done.to_string()), size].into_iter().flatten().collect()
            }
            JobState::Failed => vec![j.message.clone().unwrap_or_else(|| "It didn't go through".into())],
            JobState::Cancelled => vec!["Cancelled".into()],
        };
        let t = when(j.updated);
        if j.state.finished() && !t.is_empty() {
            parts.push(t);
        }
        let controls = widgets::hbox(6);
        match j.state {
            JobState::Queued | JobState::Running => {
                let id = j.id.clone();
                let b = gtk::Button::with_label("Cancel");
                b.add_css_class("flat");
                b.connect_clicked(move |_| {
                    let id = id.clone();
                    act(move |c| c.cancel(&id));
                });
                controls.append(&b);
            }
            JobState::Failed if j.retryable => {
                let id = j.id.clone();
                let b = gtk::Button::with_label("Retry");
                b.connect_clicked(move |_| {
                    let id = id.clone();
                    act(move |c| c.retry(&id).map(|_| ()));
                });
                controls.append(&b);
            }
            JobState::Completed => {
                let ok = gtk::Image::from_icon_name("object-select-symbolic");
                ok.add_css_class("cn-ok");
                controls.append(&ok);
            }
            _ => {}
        }
        let row = widgets::row(&esc(&title), &esc(&parts.join(" · ")), Some(controls.upcast_ref()));
        if let Some(m) = &j.message {
            row.set_tooltip_text(Some(m));
        }
        if let Some(text) = row.first_child().and_downcast::<gtk::Box>() {
            if j.state == JobState::Failed
                && let Some(d) = text.last_child()
            {
                d.add_css_class("cn-bad");
            }
            if let (false, Some(f)) = (j.state.finished(), j.fraction()) {
                let bar = gtk::ProgressBar::new();
                bar.set_fraction(f);
                bar.add_css_class("cn-sh-prog");
                text.append(&bar);
            }
        }
        let icon = gtk::Image::from_icon_name(match j.direction {
            Direction::Receive => "folder-download-symbolic",
            Direction::Send => "document-send-symbolic",
        });
        icon.add_css_class("cn-sh-tr-ic");
        icon.set_valign(gtk::Align::Center);
        row.prepend(&icon);
        rows.append(&row);
    }
}

// ----- A level down: linked devices, NAS folders, destinations and the Share Sheet -----

#[derive(Clone)]
struct MoreRows {
    key: gtk::Box,
    linked: gtk::Box,
    nas: gtk::Box,
    services: gtk::Box,
    sheet: gtk::Box,
    history: gtk::Box,
}

fn open_more() {
    let w = widgets::subpage("Linked devices, NAS folders and the Share Sheet", 660, |page| {
        let key = page.group("This computer");
        let key = holder(&key);
        let linked = page.group("Linked devices");
        linked.note("One entry for a phone or computer you reach several ways. Share sends the way that's up, and tries the next if one fails.");
        let linked = holder(&linked);
        let nas = page.group("NAS folders");
        nas.note(
            "Folders on an SMB, SFTP or NFS share. Files are copied in; nothing is moved or deleted. Passwords stay in your keyring.",
        );
        let nas = holder(&nas);
        let sheet = page.group("Share Sheet");
        sheet.note("What the Share Sheet offers, in this order. Pinned ones come first; hidden ones wait behind More.");
        let sheet = holder(&sheet);
        let services = page.group("Kinds of destination");
        services.note("Turn off a kind of destination to leave it out of the Share Sheet and of every app's Send to.");
        let services = holder(&services);
        let history = page.group("Transfers");
        history.note("What you sent and received. A failed send can be tried again for half an hour.");
        let history = holder(&history);
        let rows = MoreRows { key, linked, nas, services, sheet, history };
        let fill: Rc<dyn Fn()> = Rc::new(move || {
            let rows = rows.clone();
            cmd::background(load, move |result: Result<Snapshot, String>| {
                for b in [&rows.key, &rows.linked, &rows.nas, &rows.services, &rows.sheet, &rows.history] {
                    clear(b);
                }
                match result {
                    Ok(s) => {
                        let (r, l) = widgets::info_row("Device key", &key_text(&s.nearby.fingerprint));
                        l.set_tooltip_text(Some(&s.nearby.fingerprint));
                        rows.key.append(&r);
                        fill_devices(&rows.linked, &s.devices);
                        fill_nas(&rows.nas, &s.nas);
                        fill_sheet(&rows.sheet, &s.destinations);
                        fill_services(&rows.services, &s.providers);
                        fill_history(&rows.history, s.history);
                    }
                    Err(e) => rows.key.append(&widgets::banner(&esc(&format!("Share isn't available: {e}")), true)),
                }
            });
        });
        fill();
        MORE.with(|m| *m.borrow_mut() = Some(fill));
    });
    w.connect_destroy(|_| MORE.with(|m| *m.borrow_mut() = None));
}

fn fill_history(rows: &gtk::Box, history: usize) {
    let sizes: Vec<(String, String)> = [10, 50, 100, 500].iter().map(|n| (n.to_string(), format!("The last {n}"))).collect();
    let current = if [10, 50, 100, 500].contains(&history) { history.to_string() } else { "50".into() };
    let (r, _) = widgets::choice_row("Keep", "How many finished transfers are remembered.", sizes, &current, |n| {
        let n: u32 = n.parse().unwrap_or(50);
        act(move |c| c.set_preferences(&PreferencesUpdate { history: Some(n), ..Default::default() }));
    });
    rows.append(&r);
    let clear = widgets::confirm_button("Clear", "Clear?", |_| act(|c| c.clear_history()));
    clear.set_valign(gtk::Align::Center);
    rows.append(&widgets::row("Clear the history", "Running transfers stay.", Some(clear.upcast_ref())));
}

/// How a route reaches the device: "Flux", "Bluetooth"…
fn way(routes: &[Value], id: &str) -> &'static str {
    match routes.iter().find(|r| r["id"] == id).and_then(|r| r["provider"].as_str()) {
        Some("flux") => "Flux",
        Some("bluetooth") => "Bluetooth",
        Some("localsend") => "Nearby",
        Some(_) => "Computer",
        None => "Not around now",
    }
}

/// "Pandora · Flux": a route as the user knows it.
fn route_label(routes: &[Value], id: &str) -> String {
    match routes.iter().find(|r| r["id"] == id).and_then(|r| r["name"].as_str()) {
        Some(name) => format!("{name} · {}", way(routes, id)),
        None => format!("{id} · Not around now"),
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn fill_devices(rows: &gtk::Box, list: &Value) {
    let routes = list["routes"].as_array().cloned().unwrap_or_default();
    for d in list["devices"].as_array().into_iter().flatten() {
        let ids = strings(&d["routes"]);
        let desc = ids.iter().map(|id| route_label(&routes, id)).collect::<Vec<_>>().join(", then ");
        let id = d["id"].as_str().unwrap_or_default().to_string();
        let remove = widgets::confirm_button("Unlink", "Unlink?", move |_| {
            let id = id.clone();
            act(move |c| c.remove_device(&id));
        });
        remove.set_valign(gtk::Align::Center);
        let name = d["name"].as_str().unwrap_or_default();
        rows.append(&widgets::row(name, &glib::markup_escape_text(&desc), Some(remove.upcast_ref())));
    }
    for s in list["suggestions"].as_array().into_iter().flatten() {
        let (name, ids) = (s["name"].as_str().unwrap_or_default().to_string(), strings(&s["routes"]));
        let ways = ids.iter().map(|id| way(&routes, id)).collect::<Vec<_>>().join(" and ");
        let device = LinkedDevice { name: name.clone(), routes: ids, ..Default::default() };
        let (r, _) = widgets::button_row(
            &format!("Link {name}?"),
            &format!("Looks like one device, reached by {ways}."),
            "Link",
            move |_| {
                let device = device.clone();
                act(move |c| c.save_device(&device).map(|_| ()));
            },
        );
        rows.append(&r);
    }
    let options: Vec<(String, String)> =
        routes.iter().filter_map(|r| r["id"].as_str().map(|id| (id.to_string(), route_label(&routes, id)))).collect();
    let (r, b) = widgets::button_row(
        "Link devices",
        "Pick the ways you reach one device, in the order to try them.",
        "Link…",
        move |_| link_dialog(options.clone()),
    );
    if routes.len() < 2 {
        b.set_sensitive(false);
        b.set_tooltip_text(Some("Share sees fewer than two devices right now"));
    }
    rows.append(&r);
}

fn link_dialog(options: Vec<(String, String)>) {
    let mut third = vec![(String::new(), "Nothing else".to_string())];
    third.extend(options.iter().cloned());
    let second = options.get(1).map(|(id, _)| id.clone()).unwrap_or_default();
    dialog::ask(
        "Link devices",
        "Share shows them as one entry and tries them in this order.",
        vec![
            Field::text("Name, like My phone"),
            Field::choice(options.clone(), options.first().map(|(id, _)| id.as_str()).unwrap_or("")),
            Field::choice(options, &second),
            Field::choice(third, ""),
        ],
        "Link",
        None,
        |values, _| {
            let name = values.first().map(|s| s.trim().to_string()).unwrap_or_default();
            if name.is_empty() {
                return Some("Give it a name".into());
            }
            let mut routes: Vec<String> = Vec::new();
            for id in values.into_iter().skip(1).filter(|id| !id.is_empty()) {
                if routes.contains(&id) {
                    return Some("Pick a different way each time".into());
                }
                routes.push(id);
            }
            let device = LinkedDevice { name, routes, ..Default::default() };
            act(move |c| c.save_device(&device).map(|_| ()));
            None
        },
    );
}

fn fill_nas(rows: &gtk::Box, list: &[NasInfo]) {
    for n in list {
        let mut notes = vec![n.uri.clone()];
        if n.tailscale_uri.is_some() {
            notes.push("Tailscale fallback".into());
        }
        if let Some(user) = &n.username {
            notes.push(if n.has_password { format!("{user}, password saved") } else { user.clone() });
        }
        if n.conflict == "skip" {
            notes.push("Skips files already there".into());
        }
        let controls = widgets::hbox(6);
        let test = gtk::Button::with_label("Test");
        test.set_valign(gtk::Align::Center);
        let id = n.id.clone();
        test.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_label("Testing…");
            let (id, b) = (id.clone(), b.clone());
            cmd::background(
                move || Client::connect().and_then(|c| c.test_nas(&id)),
                move |result| {
                    b.set_sensitive(true);
                    b.set_label("Test");
                    window::toast(&result.unwrap_or_else(|e| e.to_string()));
                },
            );
        });
        let id = n.id.clone();
        let remove = widgets::confirm_button("Remove", "Remove?", move |_| {
            let id = id.clone();
            act(move |c| c.remove_nas(&id));
        });
        remove.set_valign(gtk::Align::Center);
        controls.append(&test);
        controls.append(&remove);
        rows.append(&widgets::row(&n.name, &glib::markup_escape_text(&notes.join(" · ")), Some(controls.upcast_ref())));
    }
    let (r, _) =
        widgets::button_row("Add a NAS folder", "An SMB, SFTP or NFS share, or a mounted folder.", "Add…", |_| add_nas());
    rows.append(&r);
}

fn add_nas() {
    dialog::ask(
        "Add a NAS folder",
        "Like smb://nas.local/Documents/Inbox or sftp://ken@homelab/srv/inbox. The folder is made if it isn't there.",
        vec![
            Field::text("Name, like NAS Documents"),
            Field::text("Address"),
            Field::text("Address over Tailscale (optional)"),
            Field::text("User name (optional)"),
            Field::secret("Password (optional)"),
        ],
        "Add",
        Some(("Skip files already there instead of keeping both", false)),
        |values, skip| {
            let get = |i: usize| values.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
            let optional = |i: usize| Some(get(i)).filter(|s| !s.is_empty());
            let (name, uri) = (get(0), get(1));
            if name.is_empty() || uri.is_empty() {
                return Some("A name and an address are needed".into());
            }
            let folder = NasInfo {
                name,
                uri,
                tailscale_uri: optional(2),
                username: optional(3),
                password: values.get(4).filter(|p| !p.is_empty()).cloned(),
                conflict: if skip { "skip" } else { "keep-both" }.into(),
                ..Default::default()
            };
            act(move |c| c.save_nas(&folder).map(|_| ()));
            None
        },
    );
}

fn fill_services(rows: &gtk::Box, providers: &[ProviderInfo]) {
    let disabled: Vec<String> = providers.iter().filter(|p| !p.enabled).map(|p| p.id.clone()).collect();
    for p in providers {
        let desc = match (&p.status, p.available) {
            (Some(s), false) => s.clone(),
            _ if !p.enabled => "Off".into(),
            _ if p.available => "Ready".into(),
            _ => "Nothing to offer right now".into(),
        };
        let (id, now) = (p.id.clone(), disabled.clone());
        let (r, _) = widgets::switch_row(&p.name, &glib::markup_escape_text(&desc), p.enabled, move |on| {
            let mut list = now.clone();
            list.retain(|x| *x != id);
            if !on {
                list.push(id.clone());
            }
            act(move |c| c.set_preferences(&PreferencesUpdate { disabled: Some(list), ..Default::default() }));
        });
        rows.append(&r);
    }
}

fn fill_sheet(rows: &gtk::Box, list: &[Destination]) {
    let mut options = vec![(String::new(), "None".to_string())];
    options.extend(list.iter().filter(|d| !d.hidden).map(|d| (d.id.clone(), d.name.clone())));
    let current = list.iter().find(|d| d.default).map(|d| d.id.clone()).unwrap_or_default();
    let (r, _) = widgets::choice_row(
        "Default destination",
        "Listed first and chosen with Enter in the Share Sheet.",
        options,
        &current,
        |id| {
            act(move |c| c.set_preferences(&PreferencesUpdate { default: Some(id), ..Default::default() }));
        },
    );
    rows.append(&r);
    let order: Vec<String> = list.iter().map(|d| d.id.clone()).collect();
    let pinned: Vec<String> = list.iter().filter(|d| d.pinned).map(|d| d.id.clone()).collect();
    let hidden: Vec<String> = list.iter().filter(|d| d.hidden).map(|d| d.id.clone()).collect();
    for (i, d) in list.iter().enumerate() {
        let controls = widgets::hbox(6);
        let pin = gtk::ToggleButton::with_label(if d.pinned { "Pinned" } else { "Pin" });
        pin.set_active(d.pinned);
        pin.set_valign(gtk::Align::Center);
        let (id, list_now) = (d.id.clone(), pinned.clone());
        pin.connect_toggled(move |b| {
            b.set_label(if b.is_active() { "Pinned" } else { "Pin" });
            let mut list = list_now.clone();
            list.retain(|x| *x != id);
            if b.is_active() {
                list.push(id.clone());
            }
            act(move |c| c.set_preferences(&PreferencesUpdate { pinned: Some(list), ..Default::default() }));
        });
        let shown = gtk::Switch::new();
        shown.set_active(!d.hidden);
        shown.set_valign(gtk::Align::Center);
        shown.set_tooltip_text(Some("Shown in the Share Sheet"));
        let (id, list_now) = (d.id.clone(), hidden.clone());
        shown.connect_state_set(move |_, on| {
            let mut list = list_now.clone();
            list.retain(|x| *x != id);
            if !on {
                list.push(id.clone());
            }
            act(move |c| c.set_preferences(&PreferencesUpdate { hidden: Some(list), ..Default::default() }));
            glib::Propagation::Proceed
        });
        let mover = |icon: &str, tip: &str, to: Option<usize>| {
            let b = gtk::Button::from_icon_name(icon);
            b.add_css_class("flat");
            b.set_tooltip_text(Some(tip));
            b.set_valign(gtk::Align::Center);
            b.set_sensitive(to.is_some_and(|j| j < order.len()));
            let mut swapped = order.clone();
            if let Some(j) = to.filter(|j| *j < order.len()) {
                swapped.swap(i, j);
            }
            b.connect_clicked(move |_| {
                let order = swapped.clone();
                act(move |c| c.set_preferences(&PreferencesUpdate { order: Some(order), ..Default::default() }));
            });
            b
        };
        controls.append(&mover("go-up-symbolic", "Move up", i.checked_sub(1)));
        controls.append(&mover("go-down-symbolic", "Move down", Some(i + 1)));
        controls.append(&pin);
        controls.append(&shown);
        let desc = d.status.clone().unwrap_or_default();
        rows.append(&widgets::row(&d.name, &glib::markup_escape_text(&desc), Some(controls.upcast_ref())));
    }
}
