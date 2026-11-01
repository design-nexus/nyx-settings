//! rclone accounts managed by the independent Cloud user service.
use crate::{
    cmd,
    widgets::{self, Page},
    window,
};
use gtk::{glib, prelude::*};
use nyx_cloud::{Client, Remote, SetupSession};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex},
};

type Session = Arc<Mutex<SetupSession>>;
fn refresh() {
    window::rebuild("online-accounts");
}
fn report<T>(result: anyhow::Result<T>) {
    match result {
        Ok(_) => refresh(),
        Err(_) => window::toast("Cloud operation failed. Check the account status and retry."),
    }
}
fn work<T: Send + 'static>(f: impl FnOnce(Client) -> anyhow::Result<T> + Send + 'static) {
    cmd::background(move || f(Client::connect()?), report);
}
fn clear(holder: &gtk::Box) {
    widgets::forget_rows(holder);
    while let Some(child) = holder.first_child() {
        holder.remove(&child);
    }
}

pub fn build(page: &Page) {
    watch_once();
    let accounts = page.group("Storage-only accounts");
    accounts.note("Connect cloud accounts to browse them in Files. Account removal never deletes cloud files.");
    let holder = widgets::vbox(0);
    holder.append(&widgets::label("Looking for cloud accounts…", "dim"));
    accounts.add(&holder);
    cmd::background(
        || {
            let accounts = nyx_accounts_native::Client::connect().ok().and_then(|c| c.list().ok()).unwrap_or_default();
            Ok(Client::connect()?
                .list()?
                .into_iter()
                .filter(|r| r.account_id.as_ref().is_none_or(|id| !accounts.iter().any(|a| &a.id == id)))
                .collect())
        },
        move |result: anyhow::Result<Vec<Remote>>| {
            clear(&holder);
            match result {
                Ok(remotes) => {
                    if remotes.is_empty() {
                        holder.append(&widgets::label("No storage-only accounts.", "dim"));
                    }
                    for remote in remotes {
                        append_account(&holder, remote);
                    }
                }
                Err(_) => {
                    holder.append(&widgets::banner(
                        "Cloud Storage is unavailable. Install nyx-cloud and rclone, then retry.",
                        true,
                    ));
                }
            }
            let (row, _) = widgets::button_row(
                "Add storage-only account",
                "Google Drive, Dropbox, OneDrive, Nextcloud, WebDAV, SFTP or another rclone provider.",
                "Add",
                |_| onboarding(None, None),
            );
            holder.append(&row);
            let (row, _) = widgets::button_row(
                "Existing rclone accounts",
                "Choose which existing accounts appear in Files. Configuration stays unchanged.",
                "Discover",
                |_| discover(),
            );
            holder.append(&row);
            let (row, _) =
                widgets::button_row("Refresh status", "Show current connections and mount status.", "Refresh", |_| refresh());
            holder.append(&row);
        },
    );
}
fn append_account(holder: &gtk::Box, remote: Remote) {
    let status = format!(
        "{} · {} · {}{}",
        remote.provider,
        remote.connection.replace('_', " "),
        remote.mount_status,
        if remote.error.is_empty() { String::new() } else { format!(" · {}", remote.error) }
    );
    let controls = widgets::hbox(6);
    let button = gtk::Button::with_label("Settings…");
    let r = remote.clone();
    button.connect_clicked(move |_| account_dialog(r.clone()));
    controls.append(&button);
    holder.append(&widgets::row(&remote.name, &glib::markup_escape_text(&status), Some(controls.upcast_ref())));
}
pub(super) fn dialog(title: &str) -> (gtk::Window, gtk::Box) {
    // Stay a transient dialog, not another Settings application window.
    // Hyprland tiles those to the right of the parent and they go off-screen;
    // wrapping labels then grow the mapped window further to the right.
    const WIDTH: i32 = 650;
    const HEIGHT: i32 = 550;
    let w = gtk::Window::builder().title(title).default_width(WIDTH).default_height(HEIGHT).modal(true).build();
    if let Some(parent) = window::window() {
        w.set_transient_for(Some(&parent));
    }
    w.add_css_class("settings-window");
    w.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    let body = widgets::vbox(12);
    body.set_margin_start(20);
    body.set_margin_end(20);
    body.set_margin_top(20);
    body.set_margin_bottom(20);
    let clamp = nyx_ui::clamp::Clamp::new(&body, WIDTH - 40);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .propagate_natural_width(false)
        .min_content_width(WIDTH)
        .max_content_width(WIDTH)
        .min_content_height(HEIGHT)
        .max_content_height(HEIGHT)
        .child(&clamp)
        .build();
    w.set_child(Some(&scroll));
    let keys = gtk::EventControllerKey::new();
    let close = w.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            close.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    w.add_controller(keys);
    w.present();
    (w, body)
}
fn field(body: &gtk::Box, label: &str, value: &str, secret: bool) -> gtk::Entry {
    body.append(&widgets::label(label, "dim"));
    let e = gtk::Entry::new();
    e.set_text(value);
    e.set_visibility(!secret);
    body.append(&e);
    e
}
fn account_dialog(remote: Remote) {
    thread_local! {static OPEN: RefCell<std::collections::HashMap<String,glib::WeakRef<gtk::Window>>> = RefCell::new(std::collections::HashMap::new());}
    if let Some(w) = OPEN.with(|s| s.borrow().get(&remote.id).and_then(|w| w.upgrade())) {
        w.present();
        return;
    }
    let (w, body) = dialog(&remote.name);
    OPEN.with(|s| {
        s.borrow_mut().insert(remote.id.clone(), w.downgrade());
    });
    service_controls(&body, &remote);
    append_connection_controls(&body, &remote);
    append_preferences(&body, remote, w.downgrade());
}
fn append_preferences(body: &gtk::Box, remote: Remote, close: glib::WeakRef<gtk::Window>) {
    let name = field(body, "Display name", &remote.name, false);
    let (row, login) = widgets::switch_row(
        "Mount on login",
        "Connect automatically when you sign in.",
        remote.preferences.mount_on_login,
        |_| {},
    );
    body.append(&row);
    let cache = field(body, "Cache target (MiB, soft limit)", &remote.preferences.cache_mib.to_string(), false);
    let bandwidth = field(body, "Bandwidth limit (rclone format; blank is unlimited)", &remote.preferences.bandwidth, false);
    body.append(&widgets::label(
        "Cache and bandwidth changes take effect on the next connection. Pending uploads are retained.",
        "dim",
    ));
    let save = gtk::Button::with_label("Save");
    let r = remote.clone();
    let weak = close.clone();
    save.connect_clicked(move |_| {
        let Ok(cache_mib) = cache.text().parse::<u64>() else {
            window::toast("Enter a valid cache size.");
            return;
        };
        let mut prefs = r.preferences.clone();
        prefs.mount_on_login = login.is_active();
        prefs.cache_mib = cache_mib;
        prefs.bandwidth = bandwidth.text().into();
        let id = r.id.clone();
        let name = name.text().to_string();
        work(move |c| c.update(&id, &name, &prefs));
        if let Some(w) = weak.upgrade() {
            w.close();
        }
    });
    body.append(&save);
    let r = remote.clone();
    let reauth = gtk::Button::with_label("Sign in again");
    reauth.connect_clicked(move |_| {
        if r.native_account {
            let Some(id)=r.account_id.clone()else{window::toast("The native account link is unavailable.");return;};
            cmd::background(move||nyx_accounts_native::Client::connect()?.authorize(&id),|result|match result{
                Ok(url)=>{let _=gtk::gio::AppInfo::launch_default_for_uri(&url,None::<&gtk::gio::AppLaunchContext>);},
                Err(_)=>window::toast("Update this account’s app password or OAuth client configuration above, then retry."),
            });
        }else if r.managed { onboarding(Some(r.clone()), None); }
        else { window::toast("This account uses an external rclone configuration. Reauthenticate with its original configuration tool, then retry here."); }
    });
    body.append(&reauth);
    body.append(&widgets::label("Stop managing removes this account from Files. Removing configuration also removes local sign-in details. Neither action deletes cloud files.", "dim"));
    let id = remote.id.clone();
    let weak = close.clone();
    let forget = widgets::confirm_button("Stop managing", "Confirm stop managing", move |_| {
        let id = id.clone();
        work(move |c| c.remove(&id, false));
        if let Some(w) = weak.upgrade() {
            w.close();
        }
    });
    body.append(&forget);
    if remote.managed {
        let id = remote.id;
        let weak = close.clone();
        body.append(&widgets::confirm_button("Remove local configuration", "Confirm remove configuration", move |_| {
            let id = id.clone();
            work(move |c| c.remove(&id, true));
            if let Some(w) = weak.upgrade() {
                w.close();
            }
        }));
    }
}
fn discover() {
    let (_, body) = dialog("Existing rclone accounts");
    body.append(&widgets::label("Looking for accounts…", "dim"));
    cmd::background(
        || Client::connect()?.discover(),
        move |result: anyhow::Result<Vec<nyx_cloud::Discovery>>| {
            clear(&body);
            match result {
                Ok(remotes) => {
                    if remotes.is_empty() {
                        body.append(&widgets::label("No existing remotes found. Use Add cloud account to get started.", "dim"));
                    }
                    for r in remotes {
                        let name = r.name.clone();
                        let (row, _) = widgets::button_row(&r.name, &r.provider, "Manage", move |_| {
                            let name = name.clone();
                            work(move |c| c.import_remote(&name));
                        });
                        body.append(&row);
                    }
                }
                Err(_) => body.append(&widgets::banner(
                    "Unable to discover accounts. An encrypted configuration may need unlocking.",
                    true,
                )),
            }
        },
    );
}
fn onboarding(replace: Option<Remote>, link_account: Option<String>) {
    let (w, body) = dialog("Add storage-only account");
    if replace.is_some() {
        body.append(&widgets::banner("Disconnect this account before signing in again. Your current credentials are kept until the new connection passes its test.", false));
    }
    body.append(&widgets::label("Choose a provider", "dim"));
    let providers = [
        ("Google Drive", "drive"),
        ("Dropbox", "dropbox"),
        ("Microsoft OneDrive", "onedrive"),
        ("Nextcloud / WebDAV", "webdav"),
        ("SFTP", "sftp"),
        ("Other provider (advanced)", ""),
    ];
    let names: Vec<_> = providers.iter().map(|p| p.0).collect();
    let choice = gtk::DropDown::from_strings(&names);
    body.append(&choice);
    let name = field(&body, "Display name", replace.as_ref().map(|r| r.name.as_str()).unwrap_or(""), false);
    let provider_type = field(&body, "Advanced provider type (used for Other provider)", "", false);
    let provider_catalog = gtk::DropDown::from_strings(&["Load available providers…"]);
    body.append(&provider_catalog);
    let provider_entry = provider_type.clone();
    cmd::background(
        || {
            let mut setup = Client::connect()?.setup()?;
            setup.request(json!({"action":"providers"}))
        },
        move |result: anyhow::Result<Value>| {
            if let Ok(catalog) = result
                && let Some(providers) = catalog["providers"].as_array()
            {
                let ids: Vec<String> = providers.iter().filter_map(|p| p["Name"].as_str().map(str::to_string)).collect();
                let names: Vec<String> = providers
                    .iter()
                    .map(|p| format!("{} — {}", p["Name"].as_str().unwrap_or(""), p["Description"].as_str().unwrap_or("")))
                    .collect();
                let labels: Vec<&str> = names.iter().map(String::as_str).collect();
                provider_catalog.set_model(Some(&gtk::StringList::new(&labels)));
                provider_catalog.connect_selected_notify(move |catalog| {
                    if let Some(id) = ids.get(catalog.selected() as usize) {
                        provider_entry.set_text(id);
                    }
                });
            }
        },
    );
    let forms = widgets::vbox(6);
    body.append(&forms);
    let fields: Rc<RefCell<Vec<(String, gtk::Entry)>>> = Rc::new(RefCell::new(Vec::new()));
    let build_fields = {
        let forms = forms.clone();
        let fields = fields.clone();
        move |index: u32| {
            clear(&forms);
            fields.borrow_mut().clear();
            let specs: &[(&str, &str, bool)] = match index {
                3 => &[
                    ("url", "Server URL", false),
                    ("user", "Username", false),
                    ("pass", "Password or app password", true),
                    ("vendor", "Vendor: nextcloud or other", false),
                ],
                4 => &[
                    ("host", "Host", false),
                    ("user", "Username", false),
                    ("port", "Port (optional)", false),
                    ("pass", "Password (optional with SSH key)", true),
                    ("key_file", "SSH private key path (optional)", false),
                    ("key_file_pass", "Private key password (optional)", true),
                    ("known_hosts_file", "Known hosts file (required)", false),
                ],
                5 => &[],
                _ => &[
                    ("client_id", "Custom OAuth client ID (optional)", false),
                    ("client_secret", "Custom OAuth client secret (optional)", true),
                ],
            };
            for (key, label, secret) in specs {
                let e = field(&forms, label, if *key == "vendor" { "nextcloud" } else { "" }, *secret);
                fields.borrow_mut().push((key.to_string(), e));
            }
        }
    };
    let initial = replace.as_ref().and_then(|r| providers.iter().position(|p| p.1 == r.provider)).unwrap_or(0) as u32;
    choice.set_selected(initial);
    if replace.is_some() {
        choice.set_sensitive(false);
    }
    build_fields(initial);
    choice.connect_selected_notify(move |choice| build_fields(choice.selected()));
    let advanced = field(&body, "Additional provider options (JSON object, optional)", "", true);
    body.append(&widgets::label("OAuth opens your browser. Follow any additional configuration questions below. Passwords and tokens are sent only through the private setup channel.", "dim"));
    let status = widgets::label("", "dim");
    status.set_wrap(true);
    body.append(&status);
    let next = gtk::Button::with_label("Connect");
    body.append(&next);
    let question_body = widgets::vbox(6);
    body.append(&question_body);
    let session: Arc<Mutex<Option<std::os::unix::net::UnixStream>>> = Arc::new(Mutex::new(None));
    let session_slot = session.clone();
    let weak = w.downgrade();
    let replacement = replace.as_ref().map(|r| r.id.clone());
    next.connect_clicked(move |button| {
        let index = choice.selected() as usize;
        let provider = if index < 5 { providers[index].1.to_string() } else { provider_type.text().to_string() };
        if provider.trim().is_empty() || name.text().trim().is_empty() {
            status.set_text("Enter a display name and provider.");
            return;
        }
        let mut parameters = if advanced.text().trim().is_empty() {
            json!({})
        } else {
            match serde_json::from_str::<Value>(&advanced.text()) {
                Ok(v) if v.is_object() => v,
                _ => {
                    status.set_text("Additional options must be a JSON object.");
                    return;
                }
            }
        };
        for (key, entry) in fields.borrow().iter() {
            if !entry.text().is_empty() {
                parameters[key] = json!(entry.text().as_str());
            }
        }
        if provider == "sftp" && parameters["known_hosts_file"].as_str().unwrap_or("").is_empty() {
            status.set_text("Choose a known hosts file to verify the server identity.");
            return;
        }
        button.set_sensitive(false);
        status.set_text("Connecting… Complete authorization in your browser if requested.");
        let replacement = replacement.clone();
        let link_account = link_account.clone();
        let slot = session_slot.clone();
        let button = button.clone();
        let status = status.clone();
        let questions = question_body.clone();
        let name = name.text().to_string();
        let weak = weak.clone();
        cmd::background(
            move || {
                let mut setup = Client::connect()?.setup()?;
                *slot.lock().unwrap() = Some(setup.cancellation_handle()?);
                let result = setup
                    .request(json!({"action":"configure", "provider":provider,"parameters":parameters,"replace":replacement}))?;
                let session = Arc::new(Mutex::new(setup));

                Ok((session, result, provider))
            },
            move |result: anyhow::Result<(Session, Value, String)>| {
                if !weak.upgrade().is_some_and(|w| w.is_visible()) {
                    return;
                }
                match result {
                    Ok((session, response, provider)) => {
                        setup_step(&questions, &status, &button, session, response, name, provider, weak, link_account)
                    }
                    Err(_) => {
                        button.set_sensitive(true);
                        status.set_text("Could not configure this account. Check details and retry.");
                    }
                }
            },
        );
    });
    w.connect_close_request(move |_| {
        if let Some(stream) = session.lock().unwrap().take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        glib::Propagation::Proceed
    });
}
#[allow(clippy::too_many_arguments)]
fn setup_step(
    body: &gtk::Box,
    status: &gtk::Label,
    start: &gtk::Button,
    session: Session,
    response: Value,
    name: String,
    provider: String,
    window: glib::WeakRef<gtk::Window>,
    link_account: Option<String>,
) {
    clear(body);
    let state = response["State"].as_str().unwrap_or("").to_string();
    if state.is_empty() {
        status.set_text("Account configured. Choose connection preferences, then test and save.");
        let (row, login) = widgets::switch_row("Mount on login", "Connect automatically when you sign in.", false, |_| {});
        body.append(&row);
        let cache = field(body, "Cache target (MiB, soft limit)", "2048", false);
        let bandwidth = field(body, "Bandwidth limit (blank is unlimited)", "", false);
        let finish = gtk::Button::with_label("Test and save");
        body.append(&finish);
        let status = status.clone();
        let start = start.clone();
        finish.connect_clicked(move |button| {
            let Ok(cache_mib) = cache.text().parse::<u64>() else { status.set_text("Enter a valid cache size."); return; };
            if cache_mib == 0 { status.set_text("Cache size must be greater than zero."); return; }
            let request = json!({"action":"finish","account_id":link_account,"name":name,"provider":provider,"preferences":{"mount_on_login":login.is_active(),"cache_mib":cache_mib,"bandwidth":bandwidth.text().as_str()}});
            button.set_sensitive(false); status.set_text("Testing the connection…");
            let session = session.clone(); let status = status.clone(); let start = start.clone(); let window = window.clone();
            cmd::background(move || session.lock().unwrap().request(request), move |result| {
                if result.is_ok() { refresh(); if let Some(w) = window.upgrade() { w.close(); } }
                else { status.set_text("Connection test failed. Check authorization and retry."); start.set_sensitive(true); }
            });
        });
        return;
    }
    let option = &response["Option"];
    status.set_text(option["Help"].as_str().unwrap_or("Additional account details are required."));
    if let Some(examples) = option["Examples"].as_array() {
        for ex in examples {
            body.append(&widgets::label(
                &format!("{} — {}", ex["Value"].as_str().unwrap_or(""), ex["Help"].as_str().unwrap_or("")),
                "dim",
            ));
        }
    }
    let answer = field(
        body,
        option["Name"].as_str().unwrap_or("Answer"),
        option["Default"].as_str().unwrap_or(""),
        option["IsPassword"].as_bool().unwrap_or(false),
    );
    let next = gtk::Button::with_label("Continue");
    body.append(&next);
    let weak_body = body.downgrade();
    let status = status.clone();
    let start = start.clone();
    next.connect_clicked(move |button| {
        button.set_sensitive(false);
        let request = json!({"action":"configure","provider":provider,"parameters":{},"opt":{"continue":true,"state":state,"result":answer.text().as_str(),"nonInteractive":true}});
        let session = session.clone(); let next_session = session.clone(); let weak_body = weak_body.clone(); let status = status.clone(); let start = start.clone(); let name = name.clone(); let provider = provider.clone(); let window = window.clone(); let link_account = link_account.clone();
        cmd::background(move || session.lock().unwrap().request(request), move |result| {
            if let Some(body) = weak_body.upgrade() { match result {
                Ok(response) => setup_step(&body, &status, &start, next_session, response, name, provider, window, link_account),
                Err(_) => { status.set_text("Authorization failed. Retry adding the account."); start.set_sensitive(true); }
            } }
        });
    });
}

fn watch_once() {
    thread_local! {
        static WATCH: RefCell<Option<gtk::gio::SignalSubscription>> = const { RefCell::new(None) };
        static STARTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if STARTED.with(|started| started.replace(true)) {
        return;
    }
    glib::spawn_future_local(async {
        let Ok(bus) = gtk::gio::bus_get_future(gtk::gio::BusType::Session).await else {
            STARTED.with(|started| started.set(false));
            return;
        };
        let subscription = bus.subscribe_to_signal(
            Some(nyx_cloud::BUS),
            Some(nyx_cloud::BUS),
            Some("Changed"),
            Some(nyx_cloud::PATH),
            None,
            gtk::gio::DBusSignalFlags::NONE,
            |_| {
                glib::timeout_add_local_once(std::time::Duration::from_millis(200), || {
                    window::rebuild_if_built("online-accounts")
                });
            },
        );
        WATCH.with(|slot| slot.replace(Some(subscription)));
    });
}

fn append_connection_controls(body: &gtk::Box, remote: &Remote) {
    let status = widgets::label(&format!("Storage: {} · {}", remote.connection.replace('_', " "), remote.mount_status), "dim");
    status.set_wrap(true);
    body.append(&status);
    let refresh = gtk::Button::with_label("Refresh status");
    let id = remote.id.clone();
    let output = status.downgrade();
    refresh.connect_clicked(move |button| {
        button.set_sensitive(false);
        let weak = button.downgrade();
        let id = id.clone();
        let output = output.clone();
        cmd::background(
            move || {
                Client::connect()?.list()?.into_iter().find(|r| r.id == id).ok_or_else(|| anyhow::anyhow!("Account unavailable"))
            },
            move |result| {
                if let Some(b) = weak.upgrade() {
                    b.set_sensitive(true);
                }
                if let Some(label) = output.upgrade() {
                    match result {
                        Ok(r) => label.set_text(&format!(
                            "Storage: {} · {}{}",
                            r.connection.replace('_', " "),
                            r.mount_status,
                            if r.error.is_empty() { String::new() } else { format!(" · {}", r.error) }
                        )),
                        Err(_) => label.set_text("Storage status unavailable. Check Cloud and retry."),
                    }
                }
            },
        );
    });
    let controls = widgets::hbox(6);
    for (label, action) in [("Connect", 0), ("Disconnect", 1), ("Test connection", 2)] {
        let b = gtk::Button::with_label(label);
        let id = remote.id.clone();
        b.connect_clicked(move |_| {
            let id = id.clone();
            work(move |c| match action {
                0 => c.mount(&id),
                1 => c.unmount(&id),
                _ => c.test(&id),
            });
        });
        controls.append(&b);
    }
    controls.append(&refresh);
    body.append(&controls);
}
/// Storage and Notes use Cloud even when GOA does not advertise a Files interface.
pub fn append_for_account(body: &gtk::Box, id: &str) {
    let holder = widgets::vbox(8);
    body.append(&holder);
    let resolved = Rc::new(std::cell::Cell::new(false));
    load_services(&holder, id.to_owned(), resolved.clone());
    let weak = holder.downgrade();
    let id = id.to_owned();
    glib::spawn_future_local(async move {
        let Ok(bus) = gtk::gio::bus_get_future(gtk::gio::BusType::Session).await else { return };
        let weak_callback = weak.clone();
        let subscription = bus.subscribe_to_signal(
            Some(nyx_cloud::BUS),
            Some(nyx_cloud::BUS),
            Some("Changed"),
            Some(nyx_cloud::PATH),
            None,
            gtk::gio::DBusSignalFlags::NONE,
            move |_| {
                if !resolved.get()
                    && let Some(holder) = weak_callback.upgrade()
                {
                    load_services(&holder, id.clone(), resolved.clone());
                }
            },
        );
        if let Some(holder) = weak.upgrade() {
            let subscription = RefCell::new(Some(subscription));
            holder.connect_destroy(move |_| {
                subscription.borrow_mut().take();
            });
        }
    });
}
fn load_services(holder: &gtk::Box, account_id: String, resolved: Rc<std::cell::Cell<bool>>) {
    clear(holder);
    holder.append(&widgets::label("Looking for storage services…", "dim"));
    let weak = holder.downgrade();
    cmd::background(
        || Client::connect()?.list(),
        move |result: anyhow::Result<Vec<Remote>>| {
            let Some(holder) = weak.upgrade() else { return };
            clear(&holder);
            match result {
                Ok(remotes) => {
                    if let Some(remote) = remotes.iter().find(|r| r.account_id.as_deref() == Some(&account_id)) {
                        resolved.set(true);
                        service_controls(&holder, remote);
                        append_connection_controls(&holder, remote);
                        append_preferences(&holder, remote.clone(), glib::WeakRef::new());
                        return;
                    }
                    holder.append(&widgets::label("Storage and Notes", "section-title"));
                    holder.append(&widgets::label("Connect using this account’s native authorization.", "dim"));
                    let connect = gtk::Button::with_label("Connect storage");
                    let id = account_id.clone();
                    connect.connect_clicked(move |_| {
                        let id=id.clone();
                        work(move |client|client.link_native_account(&id));
                    });
                    holder.append(&connect);
                }
                Err(_) => holder
                    .append(&widgets::label("Storage services are unavailable. Install Cloud and rclone, then retry.", "dim")),
            }
        },
    );
}
fn link_dialog(account_id: &str) {
    let (window, body) = dialog("Connect account storage");
    body.append(&widgets::label("Choose an existing connection for this account, or authorize a new one. Confirm that the selected connection belongs to this account.", "dim"));
    let id = account_id.to_owned();
    let authorize = gtk::Button::with_label("Authorize storage…");
    let link = id.clone();
    authorize.connect_clicked(move |_| onboarding(None, Some(link.clone())));
    body.append(&authorize);
    let weak = window.downgrade();
    cmd::background(
        || Client::connect()?.list(),
        move |result: anyhow::Result<Vec<Remote>>| {
            if let Ok(remotes) = result {
                for remote in remotes.into_iter().filter(|r| r.account_id.is_none()) {
                    let button = gtk::Button::with_label(&format!("Use {} ({})", remote.name, remote.provider));
                    let id = id.clone();
                    let weak = weak.clone();
                    button.connect_clicked(move |_| {
                        let id = id.clone();
                        let remote = remote.clone();
                        let weak = weak.clone();
                        cmd::background(
                            move || Client::connect()?.configure_services(&remote.id, Some(&id), true, &remote.notes),
                            move |result| {
                                report(result);
                                if let Some(w) = weak.upgrade() {
                                    w.close();
                                }
                            },
                        );
                    });
                    body.append(&button);
                }
            }
        },
    );
}
fn service_controls(body: &gtk::Box, remote: &Remote) {
    body.append(&widgets::label("Services", "section-title"));
    let (row, storage) =
        widgets::switch_row("Storage", "Show this account under Cloud Storage in Files.", remote.storage_enabled, |_| {});
    body.append(&row);
    let (row, notes) = widgets::switch_row(
        "Notes",
        "Synchronize a local Markdown vault. Storage visibility in Files is independent.",
        remote.notes.enabled,
        |_| {},
    );
    body.append(&row);
    let vault = field(body, "Local Notes vault", &remote.notes.vault, false);
    let folder = field(
        body,
        "Dedicated cloud Notes folder",
        if remote.notes.folder.is_empty() { "Nyx Notes" } else { &remote.notes.folder },
        false,
    );
    body.append(&widgets::label("Cloud Notes keeps immutable Markdown revisions in this folder to protect simultaneous edits. Existing personal WebDAV sync settings remain available in Notes.","dim"));
    let save = gtk::Button::with_label("Save services");
    let remote = remote.clone();
    save.connect_clicked(move |button| {
        let notes = nyx_cloud::NotesPreferences {
            enabled: notes.is_active(),
            vault: vault.text().into(),
            folder: folder.text().trim().into(),
        };
        if notes.enabled && (!std::path::Path::new(&notes.vault).is_dir() || !std::path::Path::new(&notes.vault).is_absolute()) {
            window::toast("Choose an existing local Notes vault.");
            return;
        }
        button.set_sensitive(false);
        let weak = button.downgrade();
        let id = remote.id.clone();
        let account = remote.account_id.clone();
        let storage = storage.is_active();
        cmd::background(
            move || Client::connect()?.configure_services(&id, account.as_deref(), storage, &notes),
            move |result| {
                if let Some(b) = weak.upgrade() {
                    b.set_sensitive(true);
                }
                match result {
                    Ok(()) => {
                        window::toast("Account services saved");
                        refresh();
                    }
                    Err(_) => window::toast(
                        "Services could not be saved. Check the vault/folder and disconnect safely before retrying.",
                    ),
                }
            },
        );
    });
    body.append(&save);
}
