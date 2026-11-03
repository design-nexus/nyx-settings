//! Native Online Accounts preview. Credentials never enter argv or D-Bus payloads.
use crate::{
    cmd,
    widgets::{self, Page},
    window,
};
use gtk::prelude::*;
use nyx_accounts_native::{
    client::Client,
    model::{Account, Credentials, Service},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

fn work<T: Send + 'static>(operation: impl FnOnce(Client) -> anyhow::Result<T> + Send + 'static) {
    cmd::background(
        move || operation(Client::connect()?),
        |result| match result {
            Ok(_) => window::rebuild("online-accounts"),
            Err(_) => window::toast("Account operation failed. Check your configuration and unlock the keyring."),
        },
    );
}
fn provider_name(provider: &str) -> &str {
    match provider {
        "google" => "Google",
        "microsoft_personal" => "Microsoft personal",
        "fastmail" => "Fastmail",
        "zoho" => "Zoho",
        "nextcloud" => "Nextcloud",
        "imap_smtp" => "IMAP and SMTP",
        "caldav" => "CalDAV",
        "carddav" => "CardDAV",
        "webdav" => "WebDAV",
        _ => "Online account",
    }
}
fn status_name(status: &str) -> &str {
    match status {
        "connected" => "Connected",
        "configured" => "Ready to connect",
        "needs_sign_in" => "Needs sign-in",
        "authorizing" => "Signing in…",
        "removing" => "Removal needs attention",
        _ => "Unavailable",
    }
}
fn display_status(label: &gtk::Label, errors: &gtk::Box, account: &Account) {
    label.set_text(&format!("{} · {}", provider_name(&account.provider), status_name(&account.status)));
    while let Some(child) = errors.first_child() {
        errors.remove(&child);
    }
    if let Some(error) = &account.error {
        errors.append(&widgets::banner(&gtk::glib::markup_escape_text(error), true));
    }
}
#[derive(Clone, Copy)]
enum Screen {
    List,
    Add,
}
thread_local! { static SCREEN: std::cell::Cell<Screen> = const { std::cell::Cell::new(Screen::List) }; }
/// The accounts list is showing (not the Add screen).
pub fn on_list() -> bool {
    SCREEN.with(|s| matches!(s.get(), Screen::List))
}

fn show(screen: Screen) {
    SCREEN.with(|s| s.set(screen));
    window::rebuild("online-accounts");
}
pub fn build(page: &Page) {
    if SCREEN.with(|s| matches!(s.get(), Screen::Add)) {
        add_page(page);
        return;
    }
    let group = page.group("Accounts");
    group.note("Mail, calendar, contacts and files from your online accounts.");
    let holder = widgets::vbox(0);
    holder.append(&widgets::label("Looking for accounts…", "dim"));
    group.add(&holder);
    let observed = Rc::new(std::cell::RefCell::new(None::<String>));
    let initial = observed.clone();
    let weak = holder.downgrade();
    let pending = Rc::new(std::cell::Cell::new(false));
    gtk::glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
        let Some(holder) = weak.upgrade() else {
            return gtk::glib::ControlFlow::Break;
        };
        if !holder.is_mapped() {
            return gtk::glib::ControlFlow::Break;
        }
        if pending.replace(true) {
            return gtk::glib::ControlFlow::Continue;
        }
        let observed = observed.clone();
        let pending = pending.clone();
        cmd::background(
            || Client::connect()?.list(),
            move |result| {
                pending.set(false);
                if let Ok(accounts) = result {
                    let value = serde_json::to_string(&accounts).unwrap_or_default();
                    let changed = observed.borrow().as_ref().is_some_and(|old| old != &value);
                    *observed.borrow_mut() = Some(value);
                    if changed {
                        window::rebuild("online-accounts");
                    }
                }
            },
        );
        gtk::glib::ControlFlow::Continue
    });
    cmd::background(
        || Client::connect()?.list(),
        move |result| {
            widgets::begin_section("online-accounts");
            widgets::forget_rows(&holder);
            while let Some(child) = holder.first_child() {
                holder.remove(&child);
            }
            match result {
                Ok(accounts) => {
                    *initial.borrow_mut() = serde_json::to_string(&accounts).ok();
                    if accounts.is_empty() {
                        let empty =
                            widgets::row("No accounts yet", "Add one to use its mail, calendar, contacts and files.", None);
                        empty.prepend(&super::account_icons::image(super::account_icons::Logo::Generic, 32));
                        holder.append(&empty);
                    }
                    for account in accounts {
                        holder.append(&account_row(&account));
                    }
                    let (r, b) = widgets::button_row(
                        "Add account",
                        "Google, Microsoft personal, Fastmail, Nextcloud, Zoho and more",
                        "Add…",
                        |_| show(Screen::Add),
                    );
                    b.add_css_class("suggested-action");
                    r.add_css_class("add-account-row");
                    widgets::keywords("add connect sign in google microsoft fastmail nextcloud zoho imap webdav oauth");
                    holder.append(&r);
                }
                Err(_) => holder.append(&widgets::banner("Online Accounts is unavailable. Try opening this page again.", true)),
            }
        },
    );
    super::cloud::build(page);
}
fn account_row(account: &Account) -> gtk::Box {
    let services = account.enabled.iter().filter(|s| **s != Service::Files).map(service_name).collect::<Vec<_>>().join(", ");
    let provider = provider_name(&account.provider);
    let desc = if services.is_empty() { provider.to_string() } else { format!("{provider} · {services}") };
    let controls = widgets::hbox(6);
    let status = widgets::tag(status_name(&account.status));
    status.add_css_class(if account.status == "connected" { "ok-tag" } else { "warn-tag" });
    controls.append(&status);
    let open = widgets::icon_button("emblem-system-symbolic", "Account settings");
    let account_copy = account.clone();
    open.connect_clicked(move |_| manage(account_copy.clone()));
    controls.append(&open);
    let row = widgets::row(
        if account.label.is_empty() { &account.identity } else { &account.label },
        &gtk::glib::markup_escape_text(&desc),
        Some(controls.upcast_ref()),
    );
    row.add_css_class("account-row");
    row.prepend(&super::account_icons::image(super::account_icons::logo_for_provider(&account.provider), 32));
    widgets::keywords(&format!(
        "{} {} google microsoft fastmail nextcloud zoho webdav oauth",
        account.provider, account.identity
    ));
    row
}
fn add_page(page: &Page) {
    let back = widgets::labeled_button("go-previous-symbolic", "All accounts");
    back.add_css_class("flat");
    back.add_css_class("back-button");
    back.set_halign(gtk::Align::Start);
    back.connect_clicked(|_| show(Screen::List));
    page.body.append(&back);
    let group = page.plain_group("Add account");
    group.note("Choose a provider, then sign in.");
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_homogeneous(true);
    flow.set_min_children_per_line(2);
    flow.set_max_children_per_line(3);
    flow.set_column_spacing(10);
    flow.set_row_spacing(10);
    flow.add_css_class("picture-cards");
    flow.add_css_class("provider-cards");
    for (provider, desc) in [
        ("google", "Gmail, Calendar, Contacts and Drive"),
        ("microsoft_personal", "Outlook calendar, contacts and OneDrive"),
        ("fastmail", "Mail, calendar and contacts with an app password"),
        ("nextcloud", "Files, calendar and contacts on your server"),
        ("zoho", "Zoho Mail, Calendar and Contacts"),
        ("imap_smtp", "Any other mail provider"),
        ("webdav", "Calendars, contacts and files on a server"),
    ] {
        flow.append(&provider_card(
            provider_name(provider),
            desc,
            super::account_icons::logo_for_provider(provider),
            move || setup_account(provider),
        ));
    }
    group.add(&flow);
}
fn provider_card(name: &str, desc: &str, logo: super::account_icons::Logo, on_click: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("picture-card");
    b.add_css_class("add-account");
    let c = widgets::vbox(6);
    let i = super::account_icons::image(logo, 48);
    i.set_halign(gtk::Align::Center);
    c.append(&i);
    let t = widgets::label(name, "picture-card-title");
    t.set_halign(gtk::Align::Center);
    c.append(&t);
    let d = widgets::label(desc, "picture-card-label");
    d.set_wrap(true);
    d.set_justify(gtk::Justification::Center);
    d.set_halign(gtk::Align::Center);
    d.set_max_width_chars(24);
    c.append(&d);
    b.set_child(Some(&c));
    b.set_tooltip_text(Some(&format!("Add a {name} account")));
    b.connect_clicked(move |_| on_click());
    b
}

fn service_name(service: &Service) -> &'static str {
    match service {
        Service::Mail => "Mail",
        Service::Calendar => "Calendar",
        Service::Contacts => "Contacts",
        Service::Files => "Files",
        Service::Tasks => "Tasks",
    }
}
fn section(body: &gtk::Box, title: &str) -> gtk::Box {
    let label = widgets::label(title, "group-title");
    label.set_margin_top(12);
    body.append(&label);
    let card = widgets::vbox(0);
    card.add_css_class("settings-card");
    body.append(&card);
    card
}
fn entry(body: &gtk::Box, title: &str, placeholder: &str) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.set_hexpand(true);
    e.set_placeholder_text(Some(placeholder));
    body.append(&widgets::stacked_row(title, "", e.upcast_ref()));
    e
}
fn services(body: &gtk::Box, provider: &str, enabled: &BTreeSet<Service>) -> Rc<Vec<(Service, gtk::Switch)>> {
    let mut switches = Vec::new();
    let supported = nyx_accounts_native::providers::capabilities(provider).expect("Known provider").services;
    for service in [Service::Mail, Service::Calendar, Service::Contacts, Service::Files, Service::Tasks] {
        if !supported.contains(&service) {
            continue;
        }
        let title = if service == Service::Files && provider == "google" { "Google Drive" } else { service_name(&service) };
        let desc = match service {
            Service::Mail => "Read and send email",
            Service::Calendar => "View and edit your calendars",
            Service::Contacts => "Use your address book",
            Service::Files => "Access your cloud files",
            Service::Tasks => "Keep your tasks in sync",
        };
        let (row, switch) = widgets::switch_row(title, desc, enabled.contains(&service), |_| {});
        body.append(&row);
        switches.push((service, switch));
    }
    Rc::new(switches)
}
fn selected(switches: &[(Service, gtk::Switch)]) -> BTreeSet<Service> {
    switches.iter().filter(|(_, s)| s.is_active()).map(|(s, _)| s.clone()).collect()
}
// Shared Design 2.0 dialog, with its actions kept outside the scrolling form.
fn account_dialog(title: &str) -> (gtk::Window, gtk::Box) {
    let (dialog, card) = widgets::dialog(title, 600);
    dialog.add_css_class("settings-window");
    if let Some(parent) = window::window() {
        dialog.set_transient_for(Some(&parent));
    }
    dialog.set_default_height(620);
    let body = widgets::vbox(8);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(300)
        .vexpand(true)
        .child(&body)
        .build();
    card.append(&scroll);
    dialog.present();
    (dialog, body)
}
fn footer(body: &gtk::Box, dialog: &gtk::Window, action: &str) -> gtk::Button {
    let row = widgets::hbox(8);
    row.set_halign(gtk::Align::End);
    row.set_margin_top(12);
    let close = dialog.clone();
    let cancel = gtk::Button::with_label("Cancel");
    cancel.connect_clicked(move |_| close.close());
    row.append(&cancel);
    let save = gtk::Button::with_label(action);
    save.add_css_class("suggested-action");
    row.append(&save);
    if let Some(card) = dialog.child().and_then(|w| w.downcast::<gtk::Box>().ok()) {
        card.append(&row);
    } else {
        body.append(&row);
    }
    save
}
fn open_authorization(url: &str) {
    if gtk::gio::AppInfo::launch_default_for_uri(url, None::<&gtk::gio::AppLaunchContext>).is_err() {
        window::toast("Could not open your browser. Try signing in again.");
    }
}
fn setup_account(provider: &'static str) {
    let (dialog, body) = account_dialog(&format!("Connect {}", provider_name(provider)));
    let oauth = matches!(provider, "google" | "microsoft_personal");
    let profile = section(&body, "Your account");
    let identity = entry(&profile, "Email or username", "you@example.com");
    let label = entry(&profile, "Account name (optional)", "Personal");
    let mut region = None;
    let regions = ["zoho.com", "zoho.eu", "zoho.in", "zoho.com.au", "zoho.jp", "zohocloud.ca", "zoho.sa", "zoho.com.cn"];
    if provider == "zoho" {
        let r = gtk::DropDown::from_strings(&regions);
        profile.append(&widgets::row("Account region", "Choose the region shown in your Zoho account", Some(r.upcast_ref())));
        region = Some(r);
    }
    let mut client_id = None;
    let secret = gtk::PasswordEntry::new();
    secret.set_show_peek_icon(true);
    secret.set_hexpand(true);
    if oauth {
        let config = section(&body, "One-time provider setup");
        config
            .append(&widgets::label("This preview needs an app registration from your provider before you can sign in.", "dim"));
        let guide = gtk::Button::with_label("Open step-by-step setup guide");
        guide.set_halign(gtk::Align::Start);
        guide.connect_clicked(|_| {
            let personal =
                std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Projects/Nyx-OAuth-Setup-Guide.md");
            let path = if personal.exists() {
                personal
            } else {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent()?.parent().map(|p| p.join("share/doc/nyx-accounts/PROVIDERS.md")))
                    .unwrap_or(personal)
            };
            let file = gtk::gio::File::for_path(path);
            let _ = gtk::gio::AppInfo::launch_default_for_uri(&file.uri(), None::<&gtk::gio::AppLaunchContext>);
        });
        config.append(&guide);
        client_id = Some(entry(&config, "Client ID", "Paste the client ID from your provider"));
        if provider == "google" {
            config.append(&widgets::stacked_row(
                "Client secret (if provided)",
                "Stored securely in your keyring",
                secret.upcast_ref(),
            ));
        }
    } else {
        profile.append(&widgets::stacked_row(
            "App password",
            "Create an app password in your provider’s security settings",
            secret.upcast_ref(),
        ));
    }
    let mut endpoints = Vec::new();
    if !oauth {
        let (wrapper, servers) = widgets::disclosure("Server settings", "Enter the secure addresses supplied by your provider");
        for (key, title, hint, visible) in [
            ("imap", "Incoming mail", "imaps://imap.example.com:993", matches!(provider, "imap_smtp" | "zoho" | "fastmail")),
            (
                "smtp",
                "Outgoing mail",
                "smtp+starttls://smtp.example.com:587",
                matches!(provider, "imap_smtp" | "zoho" | "fastmail"),
            ),
            (
                "caldav",
                "Calendars",
                "https://server.example/dav/",
                matches!(provider, "caldav" | "nextcloud" | "zoho" | "fastmail" | "webdav"),
            ),
            (
                "carddav",
                "Contacts",
                "https://server.example/dav/",
                matches!(provider, "carddav" | "nextcloud" | "zoho" | "fastmail" | "webdav"),
            ),
            ("webdav", "Files", "https://server.example/dav/", matches!(provider, "nextcloud" | "webdav")),
        ] {
            if visible {
                endpoints.push((key, entry(&servers, title, hint)));
            }
        }
        body.append(&wrapper);
    }
    let service_body = section(&body, "Use this account for");
    // Least privilege: the user explicitly selects services before authorization.
    let switches = services(&service_body, provider, &BTreeSet::new());
    let error = widgets::label("", "dim");
    error.set_visible(false);
    body.append(&error);
    let add = footer(&body, &dialog, if oauth { "Continue to sign in" } else { "Connect account" });
    add.connect_clicked(move |button| {
        let enabled = selected(&switches);
        let problem = if identity.text().trim().is_empty() {
            Some("Enter your email or username.")
        } else if enabled.is_empty() {
            Some("Choose at least one service to connect.")
        } else if oauth && client_id.as_ref().is_none_or(|e| e.text().trim().is_empty()) {
            Some("Enter your client ID. The setup guide shows where to find it.")
        } else if !oauth && secret.text().is_empty() {
            Some("Enter your app password.")
        } else {
            None
        };
        if let Some(message) = problem {
            error.set_text(message);
            error.set_visible(true);
            return;
        }
        let mut account = Account {
            id: String::new(),
            provider: provider.to_string(),
            identity: identity.text().trim().to_string(),
            label: label.text().trim().to_string(),
            enabled,
            status: String::new(),
            client_id: client_id.as_ref().map(|e| e.text().trim().to_string()),
            endpoints: endpoints
                .iter()
                .filter(|(_, e)| !e.text().trim().is_empty())
                .map(|(k, e)| (k.to_string(), e.text().trim().to_string()))
                .collect::<BTreeMap<_, _>>(),
            legacy_id: None,
            error: None,
            provider_user_id: None,
        };
        if let Err(e) =
            nyx_accounts_native::providers::apply_presets(&mut account, region.as_ref().map(|r| regions[r.selected() as usize]))
                .and_then(|_| account.validate())
        {
            error.set_text(&e.to_string());
            error.set_visible(true);
            return;
        }
        let mut credentials = Credentials::empty();
        if oauth {
            credentials.client_secret = secret.text().to_string();
        } else {
            credentials.password = secret.text().to_string();
        }
        button.set_sensitive(false);
        button.set_label("Connecting…");
        let close = dialog.clone();
        let button = button.clone();
        let error = error.clone();
        let secret = secret.clone();
        cmd::background(
            move || {
                let client = Client::connect()?;
                let id = client.register(&account)?;
                // Return the ID even if a later step fails, so retry cannot create duplicates.
                let result = client
                    .save_secret(&id, &credentials)
                    .and_then(|_| if oauth { client.authorize(&id).map(Some) } else { Ok(None) });
                Ok::<_, anyhow::Error>((id, result))
            },
            move |result| match result {
                Ok((id, result)) => {
                    secret.set_text("");
                    close.close();
                    show(Screen::List);
                    match result {
                        Ok(Some(url)) => {
                            open_authorization(&url);
                            window::toast("Finish signing in in your browser, then return to Accounts.");
                        }
                        Ok(None) => window::toast("Account added."),
                        Err(_) => window::toast("Account added, but connection needs attention. Open its settings to retry."),
                    }
                    let _ = id;
                }
                Err(_) => {
                    button.set_sensitive(true);
                    button.set_label(if oauth { "Continue to sign in" } else { "Connect account" });
                    error.set_text("Could not save this account. Unlock your keyring and try again.");
                    error.set_visible(true);
                }
            },
        );
    });
}
fn manage(account: Account) {
    thread_local! { static OPEN: std::cell::RefCell<BTreeMap<String,gtk::glib::WeakRef<gtk::Window>>> = const { std::cell::RefCell::new(BTreeMap::new()) }; }
    if let Some(w) = OPEN.with(|s| s.borrow().get(&account.id).and_then(|w| w.upgrade())) {
        w.present();
        return;
    }
    let (popup, body) = super::cloud::dialog("Account settings");
    OPEN.with(|s| {
        s.borrow_mut().insert(account.id.clone(), popup.downgrade());
    });
    let page = Page { root: gtk::ScrolledWindow::new(), body: body.clone() };
    let info = page.group("Account");
    let errors = widgets::vbox(0);
    info.top(&errors);
    let head = widgets::row(
        if account.label.is_empty() { &account.identity } else { &account.label },
        &gtk::glib::markup_escape_text(provider_name(&account.provider)),
        None,
    );
    head.add_css_class("account-head");
    head.prepend(&super::account_icons::image(super::account_icons::logo_for_provider(&account.provider), 40));
    info.add(&head);
    let (status_row, status) = widgets::info_row("Status", status_name(&account.status));
    info.add(&status_row);
    let oauth = matches!(account.provider.as_str(), "google" | "microsoft_personal");
    if oauth {
        let id = account.id.clone();
        info.add(
            &widgets::button_row(
                "Sign in again",
                "Opens the browser when this account’s sign-in has expired or permissions have changed.",
                "Sign in",
                move |_| {
                    let id = id.clone();
                    cmd::background(
                        move || Client::connect()?.authorize(&id),
                        |result| match result {
                            Ok(url) => open_authorization(&url),
                            Err(_) => window::toast("Could not start sign-in. Check the connection settings."),
                        },
                    );
                },
            )
            .0,
        );
    }
    // Keep the original Services switches: each change saves immediately.
    let group = page.group("Services");
    group.note("Turning a service off stops this computer using it. Mail, events and files stay on the provider.");
    let service_body = widgets::vbox(0);
    group.add(&service_body);
    let switches = services(&service_body, &account.provider, &account.enabled);
    let enabled = Rc::new(std::cell::RefCell::new(account.enabled.clone()));
    let busy = Rc::new(std::cell::Cell::new(false));
    for (service, switch) in switches.iter() {
        let service = service.clone();
        let enabled = enabled.clone();
        let busy = busy.clone();
        let body = service_body.clone();
        let id = account.id.clone();
        let status = status.clone();
        let errors = errors.clone();
        let weak = popup.downgrade();
        switch.connect_active_notify(move |switch| {
            if busy.get() {
                return;
            }
            let before = enabled.borrow().clone();
            let mut next = before.clone();
            if switch.is_active() {
                next.insert(service.clone());
            } else {
                next.remove(&service);
            }
            if next == before {
                return;
            }
            busy.set(true);
            body.set_sensitive(false);
            let id = id.clone();
            let enabled = enabled.clone();
            let busy = busy.clone();
            let body = body.clone();
            let status = status.clone();
            let errors = errors.clone();
            let switch = switch.clone();
            let service = service.clone();
            let weak = weak.clone();
            cmd::background(
                move || {
                    let c = Client::connect()?;
                    c.set_services(&id, &next)?;
                    c.list()?.into_iter().find(|a| a.id == id).ok_or_else(|| anyhow::anyhow!("Account unavailable"))
                },
                move |result| {
                    if weak.upgrade().is_none() {
                        return;
                    }
                    match result {
                        Ok(a) => {
                            *enabled.borrow_mut() = a.enabled.clone();
                            update_account_status(&status, &errors, &a);
                        }
                        Err(_) => {
                            switch.set_active(before.contains(&service));
                            window::toast("Could not save the service change. Unlock the keyring and retry.");
                        }
                    }
                    busy.set(false);
                    body.set_sensitive(true);
                    window::rebuild("online-accounts");
                },
            );
        });
    }
    super::cloud::append_for_account(&body, &account.id);
    let remove = page.group("Remove");
    remove.note(
        "This disconnects the account from this computer. Mail, events, contacts, and files on the provider are not deleted.",
    );
    let id = account.id.clone();
    let close = popup.clone();
    let b = widgets::confirm_button("Remove", "Remove this account?", move |_| {
        let id = id.clone();
        let close = close.clone();
        cmd::background(
            move || {
                nyx_pim::Registry::connect()?.preserve_account_cache(&id)?;
                Client::connect()?.remove(&id)
            },
            move |result| match result {
                Ok(()) => {
                    close.close();
                    window::toast("Account disconnected from this computer");
                    show(Screen::List);
                }
                Err(_) => window::toast("Could not remove the account. Your local data has been preserved."),
            },
        );
    });
    remove.add(&widgets::row("Remove from this computer", "", Some(b.upcast_ref())));
    let advanced = page.collapsible("Connection settings", false);
    let id = account.id.clone();
    advanced.add(
        &widgets::button_row("Account configuration", "Client ID, app password and server settings", "Edit…", move |_| {
            let id = id.clone();
            cmd::background(
                move || {
                    Client::connect()?
                        .list()?
                        .into_iter()
                        .find(|a| a.id == id)
                        .ok_or_else(|| anyhow::anyhow!("Account unavailable"))
                },
                |result| match result {
                    Ok(a) => configure(a),
                    Err(_) => window::toast("Could not load this account."),
                },
            );
        })
        .0,
    );
    let id = account.id.clone();
    let revoke = widgets::confirm_button("Disconnect", "Disconnect this account?", move |_| {
        let id = id.clone();
        work(move |c| {
            let _: bool = c.call("RevokeAuthorization", (id,))?;
            Ok(())
        });
    });
    advanced.add(&widgets::row(
        "Revoke access",
        "Keep the account and cached data; remove its saved sign-in",
        Some(revoke.upcast_ref()),
    ));
    advanced.add(&widgets::button_row("Sync issues", "Review retained changes", "Review", |_| sync_issues()).0);
    advanced.add(&widgets::button_row("Saved versions", "Recover earlier versions as local copies", "Browse", |_| history()).0);
    let close = popup.clone();
    let button = gtk::Button::with_label("Close");
    button.set_halign(gtk::Align::End);
    button.connect_clicked(move |_| close.close());
    body.append(&button);
    update_account_status(&status, &errors, &account);
    let weak = popup.downgrade();
    let id = account.id.clone();
    let pending = Rc::new(std::cell::Cell::new(false));
    gtk::glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
        if weak.upgrade().is_none_or(|w| !w.is_visible()) {
            return gtk::glib::ControlFlow::Break;
        }
        if pending.replace(true) {
            return gtk::glib::ControlFlow::Continue;
        }
        let id = id.clone();
        let status = status.clone();
        let errors = errors.clone();
        let pending = pending.clone();
        cmd::background(
            move || {
                Client::connect()?.list()?.into_iter().find(|a| a.id == id).ok_or_else(|| anyhow::anyhow!("Account unavailable"))
            },
            move |result| {
                pending.set(false);
                if let Ok(a) = result {
                    update_account_status(&status, &errors, &a);
                }
            },
        );
        gtk::glib::ControlFlow::Continue
    });
}
fn update_account_status(status: &gtk::Label, errors: &gtk::Box, account: &Account) {
    status.set_text(status_name(&account.status));
    while let Some(c) = errors.first_child() {
        errors.remove(&c);
    }
    if let Some(error) = &account.error {
        errors.append(&widgets::banner(&gtk::glib::markup_escape_text(error), true));
    } else if account.status == "needs_sign_in" {
        errors.append(&widgets::banner("<b>This account needs you to sign in again.</b> Services will wait until you do.", true));
    }
}

fn configure(account: Account) {
    let (dialog, body) = super::cloud::dialog("Connection settings");
    let oauth = matches!(account.provider.as_str(), "google" | "microsoft_personal");
    let status = widgets::label("", "dim");
    let errors = widgets::vbox(0);
    display_status(&status, &errors, &account);
    body.append(&status);
    body.append(&errors);
    let config = widgets::vbox(0);
    body.append(&config);
    let client_id = if oauth {
        let e = entry(&config, "Client ID", "Desktop application client ID");
        e.set_text(account.client_id.as_deref().unwrap_or(""));
        Some(e)
    } else {
        None
    };
    let password = gtk::PasswordEntry::new();
    password.set_show_peek_icon(true);
    password.set_hexpand(true);
    if account.provider != "microsoft_personal" {
        config.append(&widgets::stacked_row(
            if oauth { "New client secret" } else { "New app password" },
            "Leave empty to keep the saved value",
            password.upcast_ref(),
        ));
    }
    let mut server_entries = Vec::new();
    if !oauth {
        for (key, title) in [
            ("imap", "Incoming mail"),
            ("smtp", "Outgoing mail"),
            ("caldav", "Calendars"),
            ("carddav", "Contacts"),
            ("webdav", "Files"),
        ] {
            if let Some(value) = account.endpoints.get(key) {
                let e = entry(&config, title, "Secure server address");
                e.set_text(value);
                server_entries.push((key.to_string(), e));
            }
        }
    }
    let id = account.id.clone();
    let check_status = status.clone();
    let check_errors = errors.clone();
    config.append(
        &widgets::button_row("Check connection", "Check your saved sign-in and renew it if needed", "Check", move |_| {
            let id = id.clone();
            let status = check_status.clone();
            let errors = check_errors.clone();
            cmd::background(
                move || {
                    let c = Client::connect()?;
                    let _: String = c.call("CheckAccount", (id.as_str(),))?;
                    c.list()?.into_iter().find(|a| a.id == id).ok_or_else(|| anyhow::anyhow!("Account unavailable"))
                },
                move |result| match result {
                    Ok(a) => display_status(&status, &errors, &a),
                    Err(_) => window::toast("Could not check your connection. Unlock the keyring and try again."),
                },
            );
        })
        .0,
    );
    let error = widgets::label("", "dim");
    error.set_visible(false);
    body.append(&error);
    let save = footer(&body, &dialog, "Save changes");
    save.connect_clicked(move |button| {
        let mut edited = account.clone();
        let enabled = account.enabled.clone();
        if let Some(e) = &client_id {
            edited.client_id = Some(e.text().trim().to_string());
        }
        for (key, e) in &server_entries {
            edited.endpoints.insert(key.clone(), e.text().trim().to_string());
        }
        let value = password.text().to_string();
        let config_changed = edited.client_id != account.client_id || edited.endpoints != account.endpoints || !value.is_empty();
        let services_changed = enabled != account.enabled;
        let needs_sign_in = oauth && (config_changed || services_changed);
        if oauth && edited.client_id.as_ref().is_none_or(|id| id.is_empty()) {
            error.set_text("Enter a client ID under Advanced settings.");
            error.set_visible(true);
            return;
        }
        if let Err(e) = edited.validate() {
            error.set_text(&e.to_string());
            error.set_visible(true);
            return;
        }
        button.set_sensitive(false);
        button.set_label("Saving…");
        let button = button.clone();
        let error = error.clone();
        let close = dialog.clone();
        let password = password.clone();
        cmd::background(
            move || {
                let c = Client::connect()?;
                if config_changed {
                    c.call::<()>("UpdateConfiguration", (serde_json::to_string(&edited)?,))?;
                    if !value.is_empty() {
                        let mut credentials = Credentials::empty();
                        if oauth {
                            credentials.client_secret = value;
                        } else {
                            credentials.password = value;
                        }
                        c.save_secret(&edited.id, &credentials)?;
                    }
                }
                if services_changed {
                    c.set_services(&edited.id, &enabled)?;
                }
                // Saving and opening the browser are separate outcomes: saved changes survive an authorization error.
                Ok::<_, anyhow::Error>(if needs_sign_in { Some(c.authorize(&edited.id)) } else { None })
            },
            move |result| match result {
                Ok(authorization) => {
                    password.set_text("");
                    close.close();
                    window::rebuild("online-accounts");
                    match authorization {
                        Some(Ok(url)) => open_authorization(&url),
                        Some(Err(_)) => window::toast("Changes saved. Sign in again to approve your new permissions."),
                        None => window::toast("Account updated."),
                    }
                }
                Err(_) => {
                    button.set_sensitive(true);
                    button.set_label("Save changes");
                    error.set_text("Could not save all changes. Unlock your keyring and try again.");
                    error.set_visible(true);
                }
            },
        );
    });
}

fn sync_issues() {
    let (dialog, body) = super::cloud::dialog("Synchronization issues");
    body.append(&widgets::label(
        "Review local and provider versions before deciding. Previous versions are retained in the local resolution history.",
        "dim",
    ));
    let list = widgets::vbox(12);
    body.append(&list);
    cmd::background(
        || nyx_pim::Registry::connect().and_then(|r| r.sync_issues()),
        move |result| match result {
            Ok(issues) => {
                if issues.is_empty() {
                    list.append(&widgets::label("No synchronization issues.", "dim"));
                }
                for issue in issues {
                    let action = gtk::Button::with_label("Review");
                    let detail = if issue.kind.ends_with("uncertain") {
                        "Creation needs confirmation"
                    } else if issue.remote_deleted {
                        "Removed on the provider"
                    } else {
                        "Local and provider versions differ"
                    };
                    list.append(&widgets::row(&issue.source_label, detail, Some(action.upcast_ref())));
                    action.connect_clicked(move |_| review_issue(issue.clone()));
                }
            }
            Err(error) => list.append(&widgets::label(&error.to_string(), "dim")),
        },
    );
    let close = dialog.clone();
    body.append(&widgets::button_row("", "", "Close", move |_| close.close()).0);
}
fn review_issue(issue: nyx_pim::SyncIssue) {
    let (dialog, body) = super::cloud::dialog("Resolve synchronization issue");
    body.append(&widgets::label(&issue.source_label, "title-3"));
    let records: Vec<(String, String, bool)> = serde_json::from_str(&issue.local).unwrap_or_default();
    let local = records
        .into_iter()
        .map(|(_, payload, deleted)| format!("{}{}", if deleted { "Local deletion queued\n" } else { "" }, payload))
        .collect::<Vec<_>>()
        .join("\n");
    for (title, value) in [
        ("Local version", local),
        (
            "Provider version",
            if issue.kind.ends_with("uncertain") {
                "The provider may have created this item. Check the provider before retrying to avoid a duplicate.".into()
            } else if issue.remote_deleted {
                "Removed on the provider.".into()
            } else {
                issue.remote.clone()
            },
        ),
    ] {
        body.append(&widgets::label(title, "title-3"));
        let text = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .build();
        text.buffer().set_text(&value);
        let scroll = gtk::ScrolledWindow::builder()
            .min_content_height(110)
            .max_content_height(180)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&text)
            .build();
        body.append(&scroll);
    }
    let status =
        widgets::label("Your changes are retained. Synchronization resumes in the owning application after resolution.", "dim");
    body.append(&status);
    let actions = widgets::vbox(6);
    body.append(&actions);
    let mut choices = vec![(
        "Keep a local copy",
        "Preserve the local item offline and accept the provider version",
        nyx_pim::Resolution::KeepOffline,
    )];
    if issue.kind.ends_with("uncertain") {
        choices.push((
            "Retry creation",
            "Only after checking the provider and confirming that the item was not created",
            nyx_pim::Resolution::RetryCreation,
        ));
    } else {
        choices.push((
            "Use provider version",
            "Accept the provider version; the previous local version remains archived",
            nyx_pim::Resolution::UseRemote,
        ));
        if !issue.remote_deleted {
            choices.push((
                "Keep local changes",
                "Queue your local version against the provider’s latest revision",
                nyx_pim::Resolution::KeepLocal,
            ));
        }
    }
    for (title, description, resolution) in choices {
        let issue = issue.clone();
        let action_controls = actions.clone();
        let status = status.clone();
        let dialog = dialog.clone();
        let button = widgets::confirm_button(title, "Apply this resolution?", move |_| {
            action_controls.set_sensitive(false);
            status.set_text("Saving resolution…");
            let issue = issue.clone();
            let actions = action_controls.clone();
            let status = status.clone();
            let dialog = dialog.clone();
            cmd::background(
                move || nyx_pim::Registry::connect()?.resolve_sync_issue(&issue, resolution),
                move |result| match result {
                    Ok(()) => {
                        dialog.close();
                        window::toast("Resolution saved. Refresh the owning application to synchronize.");
                    }
                    Err(error) => {
                        actions.set_sensitive(true);
                        status.set_text(&error.to_string());
                    }
                },
            );
        });
        actions.append(&widgets::row(title, description, Some(button.upcast_ref())));
    }
    let close = dialog.clone();
    body.append(&widgets::button_row("", "", "Cancel", move |_| close.close()).0);
}

fn history() {
    let (dialog, body) = super::cloud::dialog("Local synchronization history");
    let list = widgets::vbox(6);
    body.append(&list);
    cmd::background(
        || nyx_pim::Registry::connect().and_then(|r| r.history()),
        move |result| match result {
            Ok(entries) => {
                if entries.is_empty() {
                    list.append(&widgets::label("No archived versions yet.", "dim"));
                }
                for entry in entries {
                    let key = entry.key.clone();
                    list.append(
                        &widgets::button_row(
                            &entry.source,
                            &format!("{} · {}", entry.created, entry.title),
                            "Review",
                            move |_| history_detail(key.clone()),
                        )
                        .0,
                    );
                }
            }
            Err(error) => list.append(&widgets::label(&error.to_string(), "dim")),
        },
    );
    let close = dialog.clone();
    body.append(&widgets::button_row("", "", "Close", move |_| close.close()).0);
}
fn history_detail(key: String) {
    let (dialog, body) = super::cloud::dialog("Archived versions");
    let content = widgets::vbox(8);
    body.append(&content);
    let close = dialog.clone();
    body.append(&widgets::button_row("", "", "Close", move |_| close.close()).0);
    cmd::background(
        move || nyx_pim::Registry::connect().and_then(|r| r.history_detail(&key)),
        move |result| match result {
            Err(error) => content.append(&widgets::label(&error.to_string(), "dim")),
            Ok(detail) => {
                content.append(&widgets::label(&format!("{} · {}", detail.summary.source, detail.summary.created), "dim"));
                for (title, payload, version) in [
                    ("Archived local version", detail.local, nyx_pim::HistoryVersion::Local),
                    ("Archived provider version", detail.remote, nyx_pim::HistoryVersion::Provider),
                ] {
                    if payload.trim().is_empty() {
                        continue;
                    }
                    content.append(&widgets::label(title, "title-3"));
                    let text = gtk::TextView::builder()
                        .editable(false)
                        .cursor_visible(false)
                        .monospace(true)
                        .wrap_mode(gtk::WrapMode::WordChar)
                        .build();
                    text.buffer().set_text(&payload);
                    content.append(
                        &gtk::ScrolledWindow::builder()
                            .min_content_height(100)
                            .max_content_height(180)
                            .hscrollbar_policy(gtk::PolicyType::Never)
                            .child(&text)
                            .build(),
                    );
                    let key = detail.summary.key.clone();
                    let button = widgets::confirm_button("Recover offline copy", "Create a separate local copy?", move |_| {
                        let key = key.clone();
                        cmd::background(
                            move || nyx_pim::Registry::connect()?.recover_history(&key, version),
                            |result| match result {
                                Ok(_) => window::toast(
                                    "Recovered as a separate offline copy. Refresh the owning application to view it.",
                                ),
                                Err(error) => window::toast(&error.to_string()),
                            },
                        );
                    });
                    content.append(&button);
                }
            }
        },
    );
}
