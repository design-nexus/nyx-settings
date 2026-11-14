//! Native Online Accounts preview. Credentials never enter argv or D-Bus payloads.
use super::accounts::{Link, add_row, clear, clickable, facts, head, linked, roomy, title_end};
use crate::{
    cmd,
    widgets::{self, Page},
    window,
};
use gtk::{glib, prelude::*};
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
/// Watch the accounts and redraw the page when they change (signing in finishes in the
/// browser). Returns where the first answer is kept, to compare later ones with.
fn watch(holder: &gtk::Box) -> Rc<std::cell::RefCell<Option<String>>> {
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
    initial
}

thread_local! {
    /// The account whose apps the picture shows, kept across redraws.
    static SELECTED: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Online Accounts' own page: the accounts on the left, and on the right the chosen one
/// with a line to each app it feeds, or the providers to add one from.
pub fn build_with_preview(page: &Page, pv: &widgets::Preview) {
    let shown = roomy(pv);
    let foot = pv.foot("dialog-information-symbolic", "");
    let group = page.group("Accounts");
    title_end(&group, &widgets::label("Mail, calendars, contacts and files", "dim"));
    let holder = widgets::vbox(0);
    holder.append(&widgets::label("Looking for accounts…", "dim"));
    group.add(&holder);
    let initial = watch(&holder);
    cmd::background(
        || Client::connect()?.list(),
        move |result| {
            widgets::begin_section("online-accounts");
            clear(&holder);
            let accounts = match result {
                Ok(accounts) => accounts,
                Err(_) => {
                    holder.append(&widgets::banner("Online Accounts is unavailable. Try opening this page again.", true));
                    clear(&shown);
                    shown.append(&widgets::label("Nothing to show until Online Accounts answers.", "dim"));
                    return;
                }
            };
            *initial.borrow_mut() = serde_json::to_string(&accounts).ok();
            let accounts = Rc::new(accounts);
            let rows: Rc<std::cell::RefCell<Vec<(Option<String>, gtk::Box)>>> = Rc::default();
            // `None` shows the providers to add an account from.
            let render: Rc<std::cell::RefCell<Option<Rc<dyn Fn(Option<String>)>>>> = Rc::default();
            let f: Rc<dyn Fn(Option<String>)> = {
                let (accounts, rows, shown, foot, render) = (accounts.clone(), rows.clone(), shown.clone(), foot.clone(), render.clone());
                Rc::new(move |id: Option<String>| {
                    let id = id.filter(|id| accounts.iter().any(|a| a.id == *id));
                    SCREEN.with(|s| s.set(if id.is_some() { Screen::List } else { Screen::Add }));
                    if id.is_some() {
                        SELECTED.with(|s| *s.borrow_mut() = id.clone());
                    }
                    for (rid, r) in rows.borrow().iter() {
                        if *rid == id {
                            r.add_css_class("selected");
                        } else {
                            r.remove_css_class("selected");
                        }
                    }
                    match id.and_then(|id| accounts.iter().find(|a| a.id == id)) {
                        Some(a) => match settings_open(&a.id) {
                            Some(part) => connection_settings(&shown, &foot, a, part),
                            None => {
                                close_settings();
                                account_preview(&shown, &foot, a);
                            }
                        },
                        None => {
                            let back = render.borrow().clone();
                            let first = accounts.first().map(|a| a.id.clone());
                            add_preview(&shown, &foot, first.zip(back).map(|(first, back)| move || back(Some(first.clone()))));
                        }
                    }
                })
            };
            *render.borrow_mut() = Some(f.clone());
            for account in accounts.iter() {
                let row = list_row(account);
                let (f, id) = (f.clone(), account.id.clone());
                clickable(&row, move || f(Some(id.clone())));
                rows.borrow_mut().push((Some(account.id.clone()), row.clone()));
                holder.append(&row);
            }
            let add = {
                let f = f.clone();
                add_row("Add an account", move || f(None))
            };
            widgets::keywords("add connect sign in google microsoft fastmail nextcloud zoho imap webdav oauth");
            rows.borrow_mut().push((None, add.clone()));
            holder.append(&add);
            let chosen = if on_list() {
                SELECTED.with(|s| s.borrow().clone()).filter(|id| accounts.iter().any(|a| a.id == *id)).or_else(|| accounts.first().map(|a| a.id.clone()))
            } else {
                None
            };
            f(chosen);
        },
    );
    super::cloud::build(page);
}

/// An account in the list: its logo, its name and address, and whether it's working.
fn list_row(account: &Account) -> gtk::Box {
    let provider = provider_name(&account.provider);
    let (title, desc) = if account.label.is_empty() {
        (account.identity.as_str(), provider.to_string())
    } else {
        (account.label.as_str(), account.identity.clone())
    };
    let row = widgets::row(title, &gtk::glib::markup_escape_text(&desc), None);
    row.add_css_class("account-row");
    row.add_css_class("with-tile");
    row.prepend(&super::account_icons::image(super::account_icons::logo_for_provider(&account.provider), 32));
    row.append(&status_line(account));
    let services = account.enabled.iter().map(service_name).collect::<Vec<_>>().join(" ");
    widgets::keywords(&format!(
        "{} {} {provider} {services} google microsoft fastmail nextcloud zoho webdav oauth",
        account.provider, account.identity
    ));
    row
}

/// A dot and a word: Connected, or what it needs.
fn status_line(account: &Account) -> gtk::Box {
    let b = widgets::hbox(6);
    b.add_css_class("oac-status");
    b.set_valign(gtk::Align::Center);
    let ok = account.status == "connected";
    b.add_css_class(if ok { "ok" } else { "warn" });
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("oac-dot");
    dot.set_valign(gtk::Align::Center);
    b.append(&dot);
    let text = if account.status == "needs_sign_in" { "Sign in again" } else { status_name(&account.status) };
    b.append(&gtk::Label::new(Some(text)));
    b
}

fn service_icon(service: &Service) -> &'static str {
    match service {
        Service::Mail => "mail-unread-symbolic",
        Service::Calendar => "x-office-calendar-symbolic",
        Service::Contacts => "x-office-address-book-symbolic",
        Service::Files => "folder-remote-symbolic",
        Service::Tasks => "emblem-ok-symbolic",
    }
}

/// The chosen account, a line to each app it can feed, and a switch for each.
fn account_preview(shown: &gtk::Box, foot: &gtk::Label, account: &Account) {
    clear(shown);
    let provider = provider_name(&account.provider);
    let title = if account.label.is_empty() { provider } else { account.label.as_str() };
    let oauth = matches!(account.provider.as_str(), "google" | "microsoft_personal");

    let mut end: Vec<gtk::Widget> = Vec::new();
    if account.status == "needs_sign_in" && oauth {
        let b = widgets::labeled_button("view-refresh-symbolic", "Sign in again");
    b.add_css_class("acc-small");
        b.add_css_class("suggested-action");
        let id = account.id.clone();
        b.connect_clicked(move |_| {
            let id = id.clone();
            cmd::background(
                move || Client::connect()?.authorize(&id),
                |result| match result {
                    Ok(url) => open_authorization(&url),
                    Err(_) => window::toast("Could not start sign-in. Check the connection settings."),
                },
            );
        });
        end.push(b.upcast());
    }
    let settings = widgets::labeled_button("emblem-system-symbolic", "Connection settings");
    settings.add_css_class("acc-small");
    settings.set_tooltip_text(Some("Sign-in, storage and notes, server settings and revoking access"));
    {
        let (shown, foot, a) = (shown.clone(), foot.clone(), account.clone());
        settings.connect_clicked(move |_| connection_settings(&shown, &foot, &a, Part::SignIn));
    }
    end.push(settings.upcast());
    let id = account.id.clone();
    let remove = widgets::confirm_button("Remove…", "Remove it?", move |_| {
        let id = id.clone();
        cmd::background(
            move || {
                nyx_pim::Registry::connect()?.preserve_account_cache(&id)?;
                Client::connect()?.remove(&id)
            },
            move |result| match result {
                Ok(()) => {
                    window::toast("Account disconnected from this computer");
                    SELECTED.with(|s| *s.borrow_mut() = None);
                    show(Screen::List);
                }
                Err(_) => window::toast("Could not remove the account. Your local data has been preserved."),
            },
        );
    });
    remove.set_tooltip_text(Some("Disconnect it from this computer. Nothing is deleted on the provider."));
    remove.add_css_class("acc-danger-text");
    remove.add_css_class("acc-small");
    end.push(remove.upcast());
    let head = head(title, status_name(&account.status), &end);
    head.set_tooltip_text(Some(if oauth { "Signs in through your browser" } else { "Signs in with an app password" }));
    shown.append(&head);

    if let Some(error) = &account.error {
        shown.append(&widgets::banner(&gtk::glib::markup_escape_text(error), true));
    } else if account.status == "needs_sign_in" {
        shown.append(&widgets::banner("<b>This account needs you to sign in again.</b> Services will wait until you do.", true));
    }

    // The account, and the apps it feeds.
    let supported = nyx_accounts_native::providers::capabilities(&account.provider).map(|c| c.services).unwrap_or_default();
    let feeds = widgets::hbox(0);
    feeds.add_css_class("oac-feeds");
    let source = widgets::vbox(6);
    source.add_css_class("preview-card");
    source.add_css_class("oac-source");
    source.set_size_request(132, -1);
    source.set_valign(gtk::Align::Fill);
    let inner = widgets::vbox(6);
    inner.set_valign(gtk::Align::Center);
    inner.set_vexpand(true);
    let logo = super::account_icons::image(super::account_icons::logo_for_provider(&account.provider), 56);
    logo.set_halign(gtk::Align::Center);
    inner.append(&logo);
    let name = widgets::label(title, "oac-source-name");
    name.set_xalign(0.5);
    inner.append(&name);
    let who = widgets::label(&account.identity, "oac-source-id");
    who.set_xalign(0.5);
    who.set_justify(gtk::Justification::Center);
    who.set_wrap(true);
    who.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    who.set_max_width_chars(18);
    inner.append(&who);
    source.append(&inner);
    feeds.append(&source);
    let gap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    gap.set_size_request(54, -1);
    feeds.append(&gap);
    let apps = widgets::vbox(8);
    apps.set_hexpand(true);
    feeds.append(&apps);

    let links: super::accounts::Links = Rc::default();
    let switches: Rc<std::cell::RefCell<Vec<(Service, gtk::Switch)>>> = Rc::default();
    for service in [Service::Mail, Service::Calendar, Service::Contacts, Service::Files, Service::Tasks] {
        if !supported.contains(&service) {
            continue;
        }
        let on = account.enabled.contains(&service);
        let title = if service == Service::Files && account.provider == "google" { "Google Drive" } else { service_name(&service) };
        let desc = if !on {
            format!("Off · stays on {provider}")
        } else {
            match service {
                Service::Mail => "Read and send email",
                Service::Calendar => "Your calendars",
                Service::Contacts => "Your address book",
                Service::Files => "Your files, in Files",
                Service::Tasks => "Your tasks",
            }
            .to_string()
        };
        let app = widgets::hbox(12);
        app.add_css_class("oac-app");
        if on {
            app.add_css_class("on");
        }
        app.append(&widgets::tile(service_icon(&service)));
        let text = widgets::vbox(2);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.append(&widgets::label(title, "oac-app-title"));
        let d = widgets::label(&desc, "oac-app-line");
        d.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&d);
        app.append(&text);
        let sw = gtk::Switch::new();
        sw.set_active(on);
        sw.set_valign(gtk::Align::Center);
        app.append(&sw);
        apps.append(&app);
        links.borrow_mut().push((source.clone().upcast(), app.upcast(), if on { Link::On } else { Link::Off }));
        switches.borrow_mut().push((service, sw));
    }
    for (service, sw) in switches.borrow().iter() {
        let (service, switches, apps, id, before) =
            (service.clone(), switches.clone(), apps.clone(), account.id.clone(), account.enabled.clone());
        sw.connect_active_notify(move |sw| {
            let mut next = before.clone();
            if sw.is_active() {
                next.insert(service.clone());
            } else {
                next.remove(&service);
            }
            if next == before {
                return;
            }
            apps.set_sensitive(false);
            let (id, sw, service, before, apps) = (id.clone(), sw.clone(), service.clone(), before.clone(), apps.clone());
            let _ = &switches;
            cmd::background(
                move || Client::connect()?.set_services(&id, &next),
                move |result| {
                    apps.set_sensitive(true);
                    match result {
                        Ok(()) => window::rebuild("online-accounts"),
                        Err(_) => {
                            sw.set_active(before.contains(&service));
                            window::toast("Could not save the service change. Unlock the keyring and retry.");
                        }
                    }
                },
            );
        });
    }
    let (overlay, _) = linked(&feeds, links);
    shown.append(&overlay);

    // Facts, and the tools for what didn't sync.
    let card = widgets::hbox(12);
    card.add_css_class("preview-card");
    card.add_css_class("oac-facts");
    let in_use = format!("{} of {}", account.enabled.iter().filter(|s| supported.contains(s)).count(), supported.len());
    let (flow, values) = facts(&[("Provider", provider.to_string()), ("In use", in_use), ("Sync issues", "…".into())]);
    flow.set_hexpand(true);
    card.append(&flow);
    let review = widgets::labeled_button("dialog-warning-symbolic", "Review");
    review.add_css_class("acc-small");
    review.set_valign(gtk::Align::Center);
    review.set_visible(false);
    {
        let (shown, foot, a) = (shown.clone(), foot.clone(), account.clone());
        review.connect_clicked(move |_| connection_settings(&shown, &foot, &a, Part::Sync));
    }
    card.append(&review);
    let saved = widgets::labeled_button("document-open-recent-symbolic", "Saved versions");
    saved.add_css_class("acc-small");
    saved.set_valign(gtk::Align::Center);
    saved.set_tooltip_text(Some("Recover earlier versions as local copies"));
    {
        let (shown, foot, a) = (shown.clone(), foot.clone(), account.clone());
        saved.connect_clicked(move |_| connection_settings(&shown, &foot, &a, Part::Sync));
    }
    card.append(&saved);
    shown.append(&card);
    if let Some(issues) = values.get(2).cloned() {
        cmd::background(
            || nyx_pim::Registry::connect().and_then(|r| r.sync_issues()).map(|i| i.len()),
            move |n| match n {
                Ok(0) => issues.set_text("None"),
                Ok(n) => {
                    issues.set_text(&n.to_string());
                    review.set_visible(true);
                }
                Err(_) => issues.set_text("Unknown"),
            },
        );
    }
    foot.set_text(&format!("Turning one off stops this computer using it; nothing is deleted on {provider}."));
}

/// Who an account can be with, as the Add screen offers them: (provider, name, line).
const PROVIDERS: &[(&str, &str, &str)] = &[
    ("google", "Google", "Gmail, Calendar, Contacts and Drive"),
    ("microsoft_personal", "Microsoft", "Outlook, calendar, contacts and OneDrive"),
    ("fastmail", "Fastmail", "Mail, calendar and contacts with an app password"),
    ("nextcloud", "Nextcloud", "Files, calendar and contacts on your server"),
    ("zoho", "Zoho", "Mail, Calendar and Contacts"),
    ("imap_smtp", "Other mail", "Any provider, with IMAP and SMTP"),
    ("webdav", "Calendar or contacts server", "Calendars, contacts and files on a server"),
];

/// The providers to add an account from, and cloud storage for files alone.
fn add_preview(shown: &gtk::Box, foot: &gtk::Label, close: Option<impl Fn() + 'static>) {
    clear(shown);
    let mut end: Vec<gtk::Widget> = Vec::new();
    if let Some(close) = close {
        let x = widgets::icon_button("window-close-symbolic", "Back to the account");
        x.connect_clicked(move |_| close());
        end.push(x.upcast());
    }
    shown.append(&head("Add an account", "Choose who it's with, then sign in.", &end));
    let grid = gtk::Grid::new();
    grid.set_column_homogeneous(true);
    grid.set_column_spacing(10);
    grid.set_row_spacing(10);
    grid.add_css_class("oac-providers");
    for (i, (provider, name, desc)) in PROVIDERS.iter().enumerate() {
        let b = gtk::Button::new();
        b.add_css_class("oac-provider");
        let c = widgets::hbox(12);
        c.append(&super::account_icons::image(super::account_icons::logo_for_provider(provider), 32));
        let text = widgets::vbox(2);
        text.set_valign(gtk::Align::Center);
        let t = widgets::label(name, "oac-app-title");
        t.set_wrap(true);
        text.append(&t);
        let d = widgets::label(desc, "oac-app-line");
        d.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&d);
        c.append(&text);
        b.set_child(Some(&c));
        b.set_tooltip_text(Some(&format!("Add a {name} account: {desc}")));
        let provider: &'static str = provider;
        b.connect_clicked(move |_| setup_account(provider));
        grid.attach(&b, (i % 2) as i32, (i / 2) as i32, 1, 1);
    }
    shown.append(&grid);

    let files = widgets::hbox(12);
    files.add_css_class("preview-card");
    files.add_css_class("oac-files");
    files.append(&super::account_icons::image(super::account_icons::Logo::WebDav, 32));
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&widgets::label("Only want files?", "oac-app-title"));
    let d = widgets::label(
        "Add cloud storage instead: Google Drive, Dropbox, OneDrive, Nextcloud, SFTP and more. It shows up in Files.",
        "oac-app-line",
    );
    d.set_wrap(true);
    text.append(&d);
    files.append(&text);
    let add = gtk::Button::with_label("Add cloud storage");
    add.set_valign(gtk::Align::Center);
    add.connect_clicked(|_| super::cloud::onboarding(None, None));
    files.append(&add);
    shown.append(&files);
    foot.set_text("Google and Microsoft sign in through your browser; the others use an app password. Sign-in details are kept in your keyring.");
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
/// A part of an account's connection settings.
#[derive(Clone, Copy, PartialEq)]
enum Part {
    SignIn,
    Files,
    Notes,
    Sync,
}

impl Part {
    const ALL: [(Part, &'static str, &'static str); 4] =
        [(Part::SignIn, "signin", "Sign-in"), (Part::Files, "files", "Files"), (Part::Notes, "notes", "Notes"), (Part::Sync, "sync", "Sync & versions")];

    fn key(self) -> &'static str {
        Part::ALL.iter().find(|(p, ..)| *p == self).map(|(_, k, _)| *k).unwrap_or("signin")
    }
}

thread_local! {
    /// The account whose connection settings show in the picture, and which part; kept
    /// across redraws so saving or a status change doesn't send you back to the account.
    static SETTINGS: std::cell::RefCell<Option<(String, Part)>> = const { std::cell::RefCell::new(None) };
}

/// The connection settings open for `id`, if they are. For snapshots,
/// `SETTINGS_SNAPSHOT_ACCOUNT_PART=signin|files|notes|sync` opens them on the first
/// account drawn.
fn settings_open(id: &str) -> Option<Part> {
    thread_local! { static ASKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
    if !ASKED.with(|a| a.replace(true))
        && let Ok(key) = std::env::var("SETTINGS_SNAPSHOT_ACCOUNT_PART")
        && let Some((part, ..)) = Part::ALL.iter().find(|(_, k, _)| *k == key)
    {
        SETTINGS.with(|s| *s.borrow_mut() = Some((id.to_string(), *part)));
    }
    SETTINGS.with(|s| s.borrow().as_ref().filter(|(open, _)| open == id).map(|(_, p)| *p))
}

fn close_settings() {
    SETTINGS.with(|s| *s.borrow_mut() = None);
}

/// Connection settings in place of the account's picture: a way back to it, the
/// account, and one part at a time (Sign-in, Files, Notes, Sync & versions).
fn connection_settings(shown: &gtk::Box, foot: &gtk::Label, account: &Account, part: Part) {
    SETTINGS.with(|s| *s.borrow_mut() = Some((account.id.clone(), part)));
    clear(shown);
    let provider = provider_name(&account.provider);
    let title = if account.label.is_empty() { provider } else { account.label.as_str() };

    let crumbs = widgets::hbox(6);
    crumbs.add_css_class("oac-crumbs");
    let back = widgets::icon_button("go-previous-symbolic", &format!("Back to {title}"));
    {
        let (shown, foot, account) = (shown.clone(), foot.clone(), account.clone());
        back.connect_clicked(move |_| {
            close_settings();
            account_preview(&shown, &foot, &account);
        });
    }
    crumbs.append(&back);
    crumbs.append(&widgets::label(title, "dim"));
    crumbs.append(&widgets::label("›", "dim"));
    crumbs.append(&widgets::label("Connection settings", "oac-crumb"));
    shown.append(&crumbs);

    let head = head(title, &account.identity, &[status_line(account).upcast()]);
    head.prepend(&super::account_icons::image(super::account_icons::logo_for_provider(&account.provider), 36));
    shown.append(&head);

    let options: Vec<(String, String)> = Part::ALL.iter().map(|(_, k, n)| (k.to_string(), n.to_string())).collect();
    let seg = {
        let (shown, foot, account) = (shown.clone(), foot.clone(), account.clone());
        widgets::segmented(&options, part.key(), move |key| {
            let Some(next) = Part::ALL.iter().find(|(_, k, _)| *k == key).map(|(p, ..)| *p) else { return };
            let (shown, foot, account) = (shown.clone(), foot.clone(), account.clone());
            glib::idle_add_local_once(move || connection_settings(&shown, &foot, &account, next));
        })
    };
    seg.set_halign(gtk::Align::Start);
    seg.add_css_class("oac-parts");
    shown.append(&seg);

    let body = widgets::vbox(0);
    body.add_css_class("oac-settings");
    shown.append(&body);
    let page = Page::new(gtk::ScrolledWindow::new(), body.clone());
    let redo: Rc<dyn Fn()> = {
        let (shown, foot, account) = (shown.clone(), foot.clone(), account.clone());
        Rc::new(move || connection_settings(&shown, &foot, &account, part))
    };
    match part {
        Part::SignIn => sign_in_part(&page, &body, account, redo),
        Part::Files | Part::Notes => cloud_part(&body, account, part, redo),
        Part::Sync => sync_part(&page),
    }
    foot.set_text(&format!("Switches and lists change at once; typed changes wait for Save. Nothing here deletes anything on {provider}."));
}

/// A bar at the foot that shows once something typed has changed: Cancel and Save.
fn save_bar(body: &gtk::Box, redo: Rc<dyn Fn()>) -> (gtk::Box, gtk::Button, gtk::Label) {
    let bar = widgets::hbox(8);
    bar.add_css_class("oac-savebar");
    bar.set_visible(false);
    let note = widgets::label("Unsaved changes", "dim");
    note.set_hexpand(true);
    note.set_xalign(1.0);
    bar.append(&note);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.connect_clicked(move |_| redo());
    bar.append(&cancel);
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    bar.append(&save);
    body.append(&bar);
    (bar, save, note)
}

fn reveal_on_edit(entry: &impl IsA<gtk::Editable>, bar: &gtk::Box) {
    let bar = bar.clone();
    entry.connect_changed(move |_| bar.set_visible(true));
}

fn sign_in_part(page: &Page, body: &gtk::Box, account: &Account, redo: Rc<dyn Fn()>) {
    let oauth = matches!(account.provider.as_str(), "google" | "microsoft_personal");
    let provider = provider_name(&account.provider);
    let errors = widgets::vbox(0);
    body.append(&errors);
    let status = widgets::label("", "dim");
    display_status(&status, &errors, account);

    let g = page.group("Sign-in");
    let how = if oauth { "Signed in through your browser" } else { "Signs in with an app password" };
    let result = widgets::label("", "oac-checked");
    result.set_xalign(0.0);
    result.set_wrap(true);
    result.set_visible(false);
    let check = gtk::Button::with_label("Check now");
    {
        let (id, result, status, errors) = (account.id.clone(), result.clone(), status.clone(), errors.clone());
        check.connect_clicked(move |b| {
            b.set_sensitive(false);
            let (id, result, status, errors, b) = (id.clone(), result.clone(), status.clone(), errors.clone(), b.clone());
            cmd::background(
                move || {
                    let c = Client::connect()?;
                    let _: String = c.call("CheckAccount", (id.as_str(),))?;
                    c.list()?.into_iter().find(|a| a.id == id).ok_or_else(|| anyhow::anyhow!("Account unavailable"))
                },
                move |r| {
                    b.set_sensitive(true);
                    result.set_visible(true);
                    match r {
                        Ok(a) => {
                            display_status(&status, &errors, &a);
                            result.set_text(if a.status == "connected" {
                                "Checked just now: it answers."
                            } else {
                                "Checked just now: it needs attention, as the note above says."
                            });
                        }
                        Err(_) => result.set_text("Couldn't check it. Unlock the keyring and try again."),
                    }
                },
            );
        });
    }
    let ok = account.status == "connected";
    g.add(&widgets::icon_row(
        if ok { "emblem-ok-symbolic" } else { "dialog-warning-symbolic" },
        how,
        status_name(&account.status),
        Some(check.upcast_ref()),
    ));
    g.add(&result);
    if oauth {
        let id = account.id.clone();
        let again = gtk::Button::with_label("Sign in");
        again.connect_clicked(move |_| {
            let id = id.clone();
            cmd::background(
                move || Client::connect()?.authorize(&id),
                |r| match r {
                    Ok(url) => open_authorization(&url),
                    Err(_) => window::toast("Couldn't start signing in. Check the app details below."),
                },
            );
        });
        g.add(&widgets::icon_row(
            "view-refresh-symbolic",
            "Sign in again",
            &format!("After changing your {provider} password or what this computer may use"),
            Some(again.upcast_ref()),
        ));
    }
    let id = account.id.clone();
    let revoke = widgets::confirm_button("Revoke…", "Revoke access?", move |_| {
        let id = id.clone();
        work(move |c| {
            let _: bool = c.call("RevokeAuthorization", (id,))?;
            Ok(())
        });
    });
    revoke.add_css_class("acc-danger-text");
    g.add(&widgets::icon_row(
        "changes-prevent-symbolic",
        "Revoke access",
        "Forgets the sign-in; the account and what's on this computer stay",
        Some(revoke.upcast_ref()),
    ));
    widgets::keywords("sign in check revoke access authorization");

    // What it signs in with: your own app (Google, Microsoft) or the app password and servers.
    let g = page.group(if oauth { "Your own app" } else { "Sign-in details" });
    if oauth {
        g.note(&format!("Only if you made one for {provider}; otherwise Nyx's own is used."));
    }
    let (bar, save, note) = save_bar(body, redo.clone());
    let client_id = oauth.then(|| {
        let e = gtk::Entry::new();
        e.set_placeholder_text(Some("Nyx's own"));
        e.set_text(account.client_id.as_deref().unwrap_or(""));
        e.set_hexpand(true);
        reveal_on_edit(&e, &bar);
        g.add(&widgets::stacked_row("Client ID", "", e.upcast_ref()));
        e
    });
    let password = gtk::PasswordEntry::new();
    password.set_show_peek_icon(true);
    password.set_hexpand(true);
    password.set_placeholder_text(Some("Empty keeps the saved one"));
    if account.provider != "microsoft_personal" {
        reveal_on_edit(&password, &bar);
        g.add(&widgets::stacked_row(if oauth { "Client secret" } else { "App password" }, "", password.upcast_ref()));
    }
    let mut servers = Vec::new();
    if !oauth {
        for (key, title) in [("imap", "Incoming mail"), ("smtp", "Outgoing mail"), ("caldav", "Calendars"), ("carddav", "Contacts"), ("webdav", "Files")] {
            if let Some(value) = account.endpoints.get(key) {
                let e = gtk::Entry::new();
                e.set_text(value);
                e.set_hexpand(true);
                e.set_placeholder_text(Some("Secure server address"));
                reveal_on_edit(&e, &bar);
                g.add(&widgets::stacked_row(title, "", e.upcast_ref()));
                servers.push((key.to_string(), e));
            }
        }
    }
    widgets::keywords("client id secret app password server imap smtp caldav carddav webdav");
    let account = account.clone();
    save.connect_clicked(move |button| {
        let mut edited = account.clone();
        if let Some(e) = &client_id {
            let text = e.text().trim().to_string();
            edited.client_id = (!text.is_empty()).then_some(text);
        }
        for (key, e) in &servers {
            edited.endpoints.insert(key.clone(), e.text().trim().to_string());
        }
        let secret = password.text().to_string();
        let changed = edited.client_id != account.client_id || edited.endpoints != account.endpoints || !secret.is_empty();
        if !changed {
            redo();
            return;
        }
        if let Err(e) = edited.validate() {
            note.set_text(&e.to_string());
            return;
        }
        button.set_sensitive(false);
        let (button, note, redo) = (button.clone(), note.clone(), redo.clone());
        cmd::background(
            move || {
                let c = Client::connect()?;
                c.call::<()>("UpdateConfiguration", (serde_json::to_string(&edited)?,))?;
                if !secret.is_empty() {
                    let mut credentials = Credentials::empty();
                    if oauth {
                        credentials.client_secret = secret;
                    } else {
                        credentials.password = secret;
                    }
                    c.save_secret(&edited.id, &credentials)?;
                }
                // Saving and opening the browser are separate outcomes: saved changes survive an authorization error.
                Ok::<_, anyhow::Error>(if oauth { Some(c.authorize(&edited.id)) } else { None })
            },
            move |r| match r {
                Ok(authorization) => {
                    redo();
                    match authorization {
                        Some(Ok(url)) => open_authorization(&url),
                        Some(Err(_)) => window::toast("Saved. Sign in again to approve the new app."),
                        None => window::toast("Saved"),
                    }
                }
                Err(_) => {
                    button.set_sensitive(true);
                    note.set_text("Couldn't save. Unlock the keyring and try again.");
                }
            },
        );
    });
}

/// A row with an icon and a switch that applies at once.
fn icon_switch(icon: &str, title: &str, desc: &str, active: bool, on_change: impl Fn(bool) + 'static) -> gtk::Box {
    let sw = gtk::Switch::new();
    sw.set_active(active);
    sw.set_valign(gtk::Align::Center);
    sw.connect_active_notify(move |s| on_change(s.is_active()));
    widgets::icon_row(icon, title, desc, Some(sw.upcast_ref()))
}

/// A row with an icon and a list to choose from, applied at once.
fn icon_choice(icon: &str, title: &str, desc: &str, options: Vec<(String, String)>, current: &str, on_change: impl Fn(String) + 'static) -> gtk::Box {
    let dd = widgets::dropdown(&options, current);
    dd.set_valign(gtk::Align::Center);
    dd.connect_selected_notify(move |d| {
        if let Some((id, _)) = options.get(d.selected() as usize) {
            on_change(id.clone());
        }
    });
    widgets::icon_row(icon, title, desc, Some(dd.upcast_ref()))
}

/// The cloud storage this account feeds Files and Notes through, if any.
fn cloud_part(body: &gtk::Box, account: &Account, part: Part, redo: Rc<dyn Fn()>) {
    let holder = widgets::vbox(0);
    holder.append(&widgets::label("Looking for its storage…", "dim"));
    body.append(&holder);
    let account = account.clone();
    let page = Page::new(gtk::ScrolledWindow::new(), holder.clone());
    let body = body.clone();
    let weak = holder.downgrade();
    cmd::background(
        || nyx_cloud::Client::connect()?.list(),
        move |result: anyhow::Result<Vec<nyx_cloud::Remote>>| {
            let Some(holder) = weak.upgrade() else { return };
            clear(&holder);
            let remote = match result {
                Ok(remotes) => remotes.into_iter().find(|r| r.account_id.as_deref() == Some(account.id.as_str())),
                Err(_) => {
                    holder.append(&widgets::banner("Files and Notes need Nyx Cloud and rclone. Install them, then open this again.", true));
                    return;
                }
            };
            let Some(remote) = remote else {
                let g = page.group(if part == Part::Files { "Files" } else { "Notes" });
                let connect = gtk::Button::with_label("Connect");
                let id = account.id.clone();
                let redo = redo.clone();
                connect.connect_clicked(move |b| {
                    b.set_sensitive(false);
                    let (id, redo) = (id.clone(), redo.clone());
                    cmd::background(
                        move || nyx_cloud::Client::connect()?.link_native_account(&id),
                        move |r| match r {
                            Ok(_) => redo(),
                            Err(_) => window::toast("Couldn't connect its storage. Check the account's sign-in and try again."),
                        },
                    );
                });
                g.add(&widgets::icon_row(
                    "folder-remote-symbolic",
                    "Connect its storage",
                    "So its files show in Files and Notes can sync through it",
                    Some(connect.upcast_ref()),
                ));
                return;
            };
            if part == Part::Files {
                files_part(&page, &account, remote, redo.clone());
            } else {
                notes_part(&page, &body, &account, remote, redo.clone());
            }
        },
    );
}

fn storage_name(account: &Account) -> String {
    match account.provider.as_str() {
        "google" => "Google Drive".into(),
        "microsoft_personal" => "OneDrive".into(),
        _ => format!("{} storage", provider_name(&account.provider)),
    }
}

/// Apply a change to the storage straight away, then redraw.
fn cloud_now(redo: Rc<dyn Fn()>, f: impl FnOnce(nyx_cloud::Client) -> anyhow::Result<()> + Send + 'static) {
    cmd::background(
        move || f(nyx_cloud::Client::connect()?),
        move |r| match r {
            Ok(()) => redo(),
            Err(_) => window::toast("Couldn't change it. Check the account's status and try again."),
        },
    );
}

/// "2 GB", "500 MB".
fn size_label(mib: u64) -> String {
    if mib >= 1024 && mib % 1024 == 0 { format!("{} GB", mib / 1024) } else { format!("{mib} MB") }
}

fn files_part(page: &Page, account: &Account, remote: nyx_cloud::Remote, redo: Rc<dyn Fn()>) {
    let name = storage_name(account);
    let g = page.group(&format!("{name} in Files"));
    {
        let (r, redo) = (remote.clone(), redo.clone());
        let row = icon_switch("folder-symbolic", "Show in Files", "Under Cloud in its sidebar", remote.storage_enabled, move |on| {
            let (id, account_id, notes) = (r.id.clone(), r.account_id.clone(), r.notes.clone());
            cloud_now(redo.clone(), move |c| c.configure_services(&id, account_id.as_deref(), on, &notes));
        });
        g.add(&row);
    }
    let connected = remote.mount_status == "mounted";
    let (title, desc) = if !remote.error.is_empty() {
        ("Needs attention".to_string(), remote.error.clone())
    } else if connected {
        ("Connected".to_string(), format!("Opens at {}", remote.mount_path))
    } else {
        ("Not connected".to_string(), "It connects the first time you open it in Files".to_string())
    };
    let controls = widgets::hbox(6);
    let test = gtk::Button::with_label("Test");
    {
        let (id, redo) = (remote.id.clone(), redo.clone());
        test.connect_clicked(move |_| {
            let (id, redo) = (id.clone(), redo.clone());
            cmd::background(
                move || nyx_cloud::Client::connect()?.test(&id),
                move |r| match r {
                    Ok(_) => {
                        window::toast("It answers");
                        redo();
                    }
                    Err(_) => window::toast("It didn't answer. Check the account's sign-in."),
                },
            );
        });
    }
    controls.append(&test);
    let toggle = gtk::Button::with_label(if connected { "Disconnect" } else { "Connect" });
    {
        let (id, redo) = (remote.id.clone(), redo.clone());
        toggle.connect_clicked(move |_| {
            let id = id.clone();
            cloud_now(redo.clone(), move |c| if connected { c.unmount(&id) } else { c.mount(&id) }.map(|_| ()));
        });
    }
    controls.append(&toggle);
    g.add(&widgets::icon_row(
        if remote.error.is_empty() && connected { "emblem-ok-symbolic" } else { "folder-remote-symbolic" },
        &title,
        &gtk::glib::markup_escape_text(&desc),
        Some(controls.upcast_ref()),
    ));
    {
        let (r, redo) = (remote.clone(), redo.clone());
        let row = icon_switch(
            "system-log-out-symbolic",
            "Connect when you sign in",
            "Otherwise it connects the first time you open it",
            remote.preferences.mount_on_login,
            move |on| {
                let (id, name, mut prefs) = (r.id.clone(), r.name.clone(), r.preferences.clone());
                prefs.mount_on_login = on;
                cloud_now(redo.clone(), move |c| c.update(&id, &name, &prefs));
            },
        );
        g.add(&row);
    }
    widgets::keywords("files drive onedrive storage connect disconnect test mount");

    let g = page.group("Space and speed");
    g.note("Changes apply the next time it connects; uploads waiting to go stay queued.");
    let mut sizes: Vec<u64> = vec![512, 1024, 2048, 5120, 10240, 20480, 51200];
    if !sizes.contains(&remote.preferences.cache_mib) {
        sizes.push(remote.preferences.cache_mib);
        sizes.sort();
    }
    let options: Vec<(String, String)> = sizes.iter().map(|m| (m.to_string(), size_label(*m))).collect();
    {
        let (r, redo) = (remote.clone(), redo.clone());
        let row = icon_choice(
            "drive-harddisk-symbolic",
            "Keep on this computer",
            "Files you opened lately stay here, up to this much",
            options,
            &remote.preferences.cache_mib.to_string(),
            move |v| {
                let Ok(mib) = v.parse::<u64>() else { return };
                let (id, name, mut prefs) = (r.id.clone(), r.name.clone(), r.preferences.clone());
                prefs.cache_mib = mib;
                cloud_now(redo.clone(), move |c| c.update(&id, &name, &prefs));
            },
        );
        g.add(&row);
    }
    let mut limits: Vec<(String, String)> =
        [("", "No limit"), ("1M", "1 MB/s"), ("5M", "5 MB/s"), ("10M", "10 MB/s"), ("50M", "50 MB/s")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    if !limits.iter().any(|(k, _)| *k == remote.preferences.bandwidth) {
        limits.push((remote.preferences.bandwidth.clone(), remote.preferences.bandwidth.clone()));
    }
    {
        let (r, redo) = (remote.clone(), redo.clone());
        let row = icon_choice("network-transmit-receive-symbolic", "Speed limit", "So big uploads don't slow everything else", limits, &remote.preferences.bandwidth, move |v| {
            let (id, name, mut prefs) = (r.id.clone(), r.name.clone(), r.preferences.clone());
            prefs.bandwidth = v;
            cloud_now(redo.clone(), move |c| c.update(&id, &name, &prefs));
        });
        g.add(&row);
    }
    widgets::keywords("cache space keep offline bandwidth speed limit upload");
}

fn notes_part(page: &Page, body: &gtk::Box, account: &Account, remote: nyx_cloud::Remote, redo: Rc<dyn Fn()>) {
    let name = storage_name(account);
    let g = page.group("Notes");
    g.note("Each save is kept as its own version, so edits from two computers never overwrite each other.");
    let (bar, save, note) = save_bar(body, redo.clone());
    let on = gtk::Switch::new();
    on.set_active(remote.notes.enabled);
    on.set_valign(gtk::Align::Center);
    {
        let bar = bar.clone();
        on.connect_active_notify(move |_| bar.set_visible(true));
    }
    g.add(&widgets::icon_row(
        "accessories-text-editor-symbolic",
        &format!("Sync Notes through {name}"),
        "Your notes on every computer signed in to this account",
        Some(on.upcast_ref()),
    ));
    let vault = gtk::Entry::new();
    vault.set_text(&remote.notes.vault);
    vault.set_placeholder_text(Some("A folder on this computer"));
    vault.set_hexpand(true);
    reveal_on_edit(&vault, &bar);
    let choose = gtk::Button::with_label("Choose…");
    {
        let vault = vault.clone();
        choose.connect_clicked(move |_| {
            let vault = vault.clone();
            gtk::FileDialog::builder().title("Notes folder").modal(true).build().select_folder(
                window::window().as_ref(),
                gtk::gio::Cancellable::NONE,
                move |r| {
                    if let Some(path) = r.ok().and_then(|f| f.path()) {
                        vault.set_text(&path.to_string_lossy());
                    }
                },
            );
        });
    }
    let line = widgets::hbox(6);
    line.append(&vault);
    line.append(&choose);
    g.add(&widgets::stacked_row("On this computer", "", line.upcast_ref()));
    let folder = gtk::Entry::new();
    folder.set_text(if remote.notes.folder.is_empty() { "Nyx Notes" } else { &remote.notes.folder });
    folder.set_hexpand(true);
    reveal_on_edit(&folder, &bar);
    g.add(&widgets::stacked_row(&format!("On {name}"), "A folder of its own", folder.upcast_ref()));
    widgets::keywords("notes sync vault folder markdown versions");
    save.connect_clicked(move |button| {
        let notes = nyx_cloud::NotesPreferences { enabled: on.is_active(), vault: vault.text().trim().into(), folder: folder.text().trim().into() };
        if notes.enabled && !(std::path::Path::new(&notes.vault).is_absolute() && std::path::Path::new(&notes.vault).is_dir()) {
            note.set_text("Choose a folder on this computer first.");
            return;
        }
        button.set_sensitive(false);
        let (id, account_id, storage) = (remote.id.clone(), remote.account_id.clone(), remote.storage_enabled);
        let (button, note, redo) = (button.clone(), note.clone(), redo.clone());
        cmd::background(
            move || nyx_cloud::Client::connect()?.configure_services(&id, account_id.as_deref(), storage, &notes),
            move |r| match r {
                Ok(()) => {
                    window::toast("Saved");
                    redo();
                }
                Err(_) => {
                    button.set_sensitive(true);
                    note.set_text("Couldn't save. Check the folders and try again.");
                }
            },
        );
    });
}

/// Changes that couldn't sync, and the earlier versions kept on this computer.
fn sync_part(page: &Page) {
    let g = page.group("Sync issues");
    let issues = widgets::vbox(0);
    issues.append(&widgets::label("Looking…", "dim"));
    g.add(&issues);
    cmd::background(
        || nyx_pim::Registry::connect().and_then(|r| r.sync_issues()),
        move |result| {
            clear(&issues);
            match result {
                Ok(list) if list.is_empty() => issues.append(&widgets::icon_row(
                    "emblem-ok-symbolic",
                    "Nothing waiting",
                    "A change this computer couldn't send shows here, kept until you decide.",
                    None,
                )),
                Ok(list) => {
                    for issue in list {
                        let detail = if issue.kind.ends_with("uncertain") {
                            "It may already be on the provider: check before sending it again"
                        } else if issue.remote_deleted {
                            "Removed on the provider"
                        } else {
                            "Changed both here and on the provider"
                        };
                        let review = gtk::Button::with_label("Review…");
                        let i = issue.clone();
                        review.connect_clicked(move |_| review_issue(i.clone()));
                        issues.append(&widgets::icon_row("dialog-warning-symbolic", &issue.source_label, detail, Some(review.upcast_ref())));
                    }
                }
                Err(_) => issues.append(&widgets::label("Couldn't read them. Open this again in a moment.", "dim")),
            }
        },
    );
    let g = page.group("Saved versions");
    g.note("Restoring makes a separate copy; nothing is overwritten.");
    let versions = widgets::vbox(0);
    versions.append(&widgets::label("Looking…", "dim"));
    g.add(&versions);
    cmd::background(
        || nyx_pim::Registry::connect().and_then(|r| r.history()),
        move |result| {
            clear(&versions);
            match result {
                Ok(list) if list.is_empty() => versions.append(&widgets::label("No earlier versions yet.", "dim")),
                Ok(list) => {
                    for entry in list {
                        let key = entry.key.clone();
                        let open = gtk::Button::with_label("Review…");
                        open.connect_clicked(move |_| history_detail(key.clone()));
                        versions.append(&widgets::icon_row(
                            "document-open-recent-symbolic",
                            &entry.title,
                            &gtk::glib::markup_escape_text(&format!("{} · {}", entry.source, entry.created)),
                            Some(open.upcast_ref()),
                        ));
                    }
                }
                Err(_) => versions.append(&widgets::label("Couldn't read them. Open this again in a moment.", "dim")),
            }
        },
    );
    widgets::keywords("sync issues conflicts saved versions history restore recover");
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
