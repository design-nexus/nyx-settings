//! Sharing: nearby sharing, your computers, NAS folders and what the Share
//! Sheet shows. Everything goes through Nyx Share's service (Share1); Settings
//! keeps none of it.

use crate::dialog::{self, Field};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::{gio, glib, prelude::*};
use nyx_share_client::{
    Client, ComputerInfo, Destination, Direction, Job, JobState, NasInfo, NearbyStatus, NearbyUpdate, PreferencesUpdate, ProviderInfo,
};

struct Snapshot {
    nearby: NearbyStatus,
    computers: Vec<ComputerInfo>,
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
        nas: c.nas().map_err(text)?,
        destinations: c.destinations(&[], 0).map_err(text)?,
        providers: c.capabilities().map_err(text)?.providers,
        jobs: c.jobs().map_err(text)?,
        history: c.preferences().map_err(text)?.history_limit(),
    })
}

fn refresh() {
    window::rebuild_if_built("sharing");
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

pub fn build(page: &Page) {
    let nearby = page.group("Nearby sharing");
    nearby.note("Send to and receive from phones and computers running LocalSend or Share on your network or tailnet.");
    let nearby_rows = holder(&nearby);
    nearby_rows.append(&widgets::label("Asking Share…", "dim"));
    let transfers = page.group("Transfers");
    transfers.note("What you sent and received. A failed send can be tried again for half an hour.");
    let transfer_rows = holder(&transfers);
    let computers = page.group("My computers");
    computers.note(
        "Computers you keep, known by their device key whatever their address. A trusted one can send to you without asking.",
    );
    let computer_rows = holder(&computers);
    let nas = page.group("NAS folders");
    nas.note(
        "Folders on an SMB, SFTP or NFS share. Files are copied in; nothing is moved or deleted. Passwords stay in your keyring.",
    );
    let nas_rows = holder(&nas);
    let services = page.group("Destinations");
    services.note("Turn off a kind of destination to leave it out of the Share Sheet and of every app's Send to.");
    let service_rows = holder(&services);
    let sheet = page.group("Share Sheet");
    sheet.note("What the Share Sheet offers, in this order. Pinned ones come first; hidden ones wait behind More.");
    let sheet_rows = holder(&sheet);
    widgets::keywords(
        "share sharing localsend nearby airdrop send receive tailscale computers nas smb sftp nfs flux phone share sheet",
    );

    cmd::background(load, move |result: Result<Snapshot, String>| {
        clear(&nearby_rows);
        match result {
            Ok(s) => {
                fill_nearby(&nearby_rows, &s.nearby);
                fill_computers(&computer_rows, &s.computers);
                fill_nas(&nas_rows, &s.nas);
                fill_transfers(&transfer_rows, &s.jobs, s.history);
                fill_services(&service_rows, &s.providers);
                fill_sheet(&sheet_rows, &s.destinations);
            }
            Err(e) => {
                for group in [&transfers, &computers, &nas, &services, &sheet] {
                    group.wrapper.set_visible(false);
                }
                nearby_rows.append(&widgets::banner(
                    &glib::markup_escape_text(&format!("Share isn't available: {e}. If it isn't installed, install it with nyx install share.")),
                    true,
                ));
            }
        }
    });
}

fn fill_nearby(rows: &gtk::Box, s: &NearbyStatus) {
    let (r, _) = widgets::switch_row(
        "Visible to nearby devices",
        "While Share is open, and a minute after you share. Every offer asks you first, unless it comes from a trusted computer.",
        s.visibility != "off",
        |on| {
            let visibility = Some(if on { "running" } else { "off" }.to_string());
            act(move |c| c.set_nearby(&NearbyUpdate { visibility, ..Default::default() }));
        },
    );
    rows.append(&r);
    if let Some(problem) = &s.problem {
        rows.append(&widgets::banner(&glib::markup_escape_text(problem), true));
    }
    let (r, _) = widgets::entry_row("Device name", "What nearby devices see.", &s.alias, "This computer", |name| {
        let name = name.trim().to_string();
        if !name.is_empty() {
            act(move |c| c.set_nearby(&NearbyUpdate { alias: Some(name), ..Default::default() }));
        }
    });
    rows.append(&r);
    let folder = std::path::Path::new(&s.receive_folder);
    let shown = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| s.receive_folder.clone());
    let current = s.receive_folder.clone();
    let (r, b) = widgets::button_row("Save received files to", "", &shown, move |_| {
        let dialog = gtk::FileDialog::builder().title("Save received files to").accept_label("Choose").modal(true).build();
        dialog.set_initial_folder(Some(&gio::File::for_path(&current)));
        dialog.select_folder(window::window().as_ref(), gio::Cancellable::NONE, |res| {
            let Some(path) = res.ok().and_then(|f| f.path()) else { return };
            let folder = Some(path.display().to_string());
            act(move |c| c.set_nearby(&NearbyUpdate { receive_folder: folder, ..Default::default() }));
        });
    });
    b.set_tooltip_text(Some(&s.receive_folder));
    rows.append(&r);
    let (desc, usable) = match s.tailscale_state.as_str() {
        "running" => ("Also look on your tailnet. Devices there need LocalSend open.", true),
        "stopped" => ("Tailscale isn't connected on this computer.", true),
        _ => ("Install Tailscale to reach your devices from any network.", false),
    };
    let (r, sw) = widgets::switch_row("Find devices over Tailscale", desc, s.tailscale, |on| {
        act(move |c| c.set_nearby(&NearbyUpdate { tailscale: Some(on), ..Default::default() }));
    });
    sw.set_sensitive(usable);
    rows.append(&r);
    let f = &s.fingerprint;
    let key = if f.len() >= 16 {
        format!("{} {} … {} {}", &f[..4], &f[4..8], &f[f.len() - 8..f.len() - 4], &f[f.len() - 4..])
    } else {
        "Made when nearby sharing first starts".into()
    };
    let (r, _) = widgets::info_row("Device key", &key);
    rows.append(&r);
    let (r, _) = widgets::button_row("Share", "Send from the Share window, and see what you sent and received.", "Open", |_| {
        cmd::spawn(&["nyx-share-sheet"]);
    });
    rows.append(&r);
}

fn fill_computers(rows: &gtk::Box, list: &[ComputerInfo]) {
    for c in list {
        let presence = match c.presence.as_str() {
            "lan" => "Online on the network",
            "tailscale" => "Online over Tailscale",
            "offline" => "Offline",
            _ => "Checking",
        };
        let routes: Vec<String> =
            [c.lan.clone().map(|a| format!("LAN {a}")), c.tailscale.clone().map(|a| format!("Tailscale {a}"))]
                .into_iter()
                .flatten()
                .collect();
        let controls = widgets::hbox(6);
        let label = widgets::label("Trusted", "dim");
        label.set_valign(gtk::Align::Center);
        let trusted = gtk::Switch::new();
        trusted.set_active(c.trusted);
        trusted.set_valign(gtk::Align::Center);
        trusted.set_tooltip_text(Some("Receive files from it without asking"));
        let edited = c.clone();
        trusted.connect_state_set(move |_, on| {
            let mut c = edited.clone();
            c.trusted = on;
            act(move |client| client.save_computer(&c));
            glib::Propagation::Proceed
        });
        let id = c.id.clone();
        let remove = widgets::confirm_button("Remove", "Remove?", move |_| {
            let id = id.clone();
            act(move |client| client.remove_computer(&id));
        });
        remove.set_valign(gtk::Align::Center);
        controls.append(&label);
        controls.append(&trusted);
        controls.append(&remove);
        let desc = format!("{presence} · {}", routes.join(" · "));
        rows.append(&widgets::row(&c.name, &glib::markup_escape_text(&desc), Some(controls.upcast_ref())));
    }
    let (r, _) = widgets::button_row(
        "Add a computer",
        "By its address on the network or tailnet. The pin beside a nearby device in Share saves it too.",
        "Add…",
        |_| {
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
        },
    );
    rows.append(&r);
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

fn fill_transfers(rows: &gtk::Box, jobs: &[Job], history: usize) {
    let shown: Vec<&Job> = jobs.iter().take(12).collect();
    if shown.is_empty() {
        rows.append(&widgets::row("No transfers yet", "What you share and receive shows here.", None));
    }
    for j in shown {
        let what = j.summary.clone().unwrap_or_else(|| format!("{} items", j.items));
        let title = match j.direction {
            Direction::Receive => format!("{what} from {}", j.destination_name),
            Direction::Send => format!("{what} to {}", j.destination_name),
        };
        let status = match j.state {
            JobState::Queued | JobState::Running => match (&j.step, j.fraction()) {
                (Some(step), _) => step.clone(),
                (None, Some(f)) => format!("{:.0}%", f * 100.0),
                (None, None) => "Sending".into(),
            },
            _ => j.message.clone().unwrap_or_default(),
        };
        let when = glib::DateTime::from_unix_local(j.updated).and_then(|d| d.format("%b %-d, %H:%M")).map(|s| s.to_string()).unwrap_or_default();
        let controls = widgets::hbox(6);
        match j.state {
            JobState::Queued | JobState::Running => {
                let id = j.id.clone();
                let b = gtk::Button::with_label("Cancel");
                b.set_valign(gtk::Align::Center);
                b.connect_clicked(move |_| {
                    let id = id.clone();
                    act(move |c| c.cancel(&id));
                });
                controls.append(&b);
            }
            JobState::Failed if j.retryable => {
                let id = j.id.clone();
                let b = gtk::Button::with_label("Retry");
                b.set_valign(gtk::Align::Center);
                b.connect_clicked(move |_| {
                    let id = id.clone();
                    act(move |c| c.retry(&id).map(|_| ()));
                });
                controls.append(&b);
            }
            _ => {}
        }
        let desc = if when.is_empty() { status } else { format!("{status} · {when}") };
        let row = widgets::row(&title, &glib::markup_escape_text(&desc), Some(controls.upcast_ref()));
        if j.state == JobState::Failed {
            row.add_css_class("error");
        }
        rows.append(&row);
    }
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
    let (r, _) = widgets::choice_row("Default destination", "Listed first and chosen with Enter in the Share Sheet.", options, &current, |id| {
        act(move |c| c.set_preferences(&PreferencesUpdate { default: Some(id), ..Default::default() }));
    });
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
