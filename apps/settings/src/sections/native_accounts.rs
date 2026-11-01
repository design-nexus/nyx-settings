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
        "imap_smtp" => "Email",
        "caldav" => "CalDAV",
        "carddav" => "CardDAV",
        "webdav" => "DAV services",
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
pub fn build(page: &Page) {
    let group = page.group("Online Accounts · Preview");
    group.note("Native accounts are separate from your existing accounts. Nothing is imported or deleted automatically.");
    let add = widgets::button_row("Add account", "Mail, calendars, contacts and files", "Add", |_| add_account()).0;
    group.add(&add);
    let refresh = widgets::button_row("Connection status", "Refresh after browser sign-in", "Refresh", |_| {
        window::rebuild("online-accounts")
    })
    .0;
    group.add(&refresh);
    group.add(
        &widgets::button_row("Synchronization issues", "Review retained calendar and task conflicts", "Review", |_| {
            sync_issues()
        })
        .0,
    );
    let holder = widgets::vbox(0);
    group.add(&holder);
    cmd::background(
        || Client::connect()?.list(),
        move |result| match result {
            Ok(accounts) => {
                if accounts.is_empty() {
                    holder.append(&widgets::label("No native accounts yet.", "dim"));
                }
                for account in accounts {
                    let label = if account.label.is_empty() { &account.identity } else { &account.label };
                    let button = gtk::Button::with_label("Manage");
                    let row = widgets::row(
                        label,
                        &format!("{} · {}", provider_name(&account.provider), status_name(&account.status)),
                        Some(button.upcast_ref()),
                    );
                    row.prepend(&super::account_icons::image(super::account_icons::logo_for_provider(&account.provider), 36));
                    button.connect_clicked(move |_| manage(account.clone()));
                    holder.append(&row);
                }
            }
            Err(_) => holder.append(&widgets::banner("The native accounts preview service is unavailable.", true)),
        },
    );
}
fn entry(body: &gtk::Box, title: &str, placeholder: &str) -> gtk::Entry {
    let (row, entry) = widgets::entry_row(title, "", "", placeholder, |_| {});
    body.append(&row);
    entry
}
fn services(body: &gtk::Box, enabled: &BTreeSet<Service>) -> Rc<Vec<(Service, gtk::Switch)>> {
    let mut switches = Vec::new();
    for (service, label) in [
        (Service::Mail, "Mail"),
        (Service::Calendar, "Calendar"),
        (Service::Contacts, "Contacts"),
        (Service::Files, "Files"),
        (Service::Tasks, "Tasks"),
    ] {
        let (row, switch) = widgets::switch_row(label, "", enabled.contains(&service), |_| {});
        body.append(&row);
        switches.push((service, switch));
    }
    Rc::new(switches)
}
fn selected(switches: &[(Service, gtk::Switch)]) -> BTreeSet<Service> {
    switches.iter().filter(|(_, s)| s.is_active()).map(|(s, _)| s.clone()).collect()
}
fn add_account() {
    let (dialog, body) = super::cloud::dialog("Add online account");
    let providers = ["google", "microsoft_personal", "fastmail", "zoho", "nextcloud", "imap_smtp", "caldav", "carddav", "webdav"];
    let labels =
        ["Google", "Microsoft personal", "Fastmail", "Zoho", "Nextcloud", "IMAP and SMTP", "CalDAV", "CardDAV", "WebDAV"];
    let provider = gtk::DropDown::from_strings(&labels);
    body.append(&widgets::row("Provider", "Microsoft supports personal accounts only", Some(provider.upcast_ref())));
    let regions = ["zoho.com", "zoho.eu", "zoho.in", "zoho.com.au", "zoho.jp", "zohocloud.ca", "zoho.sa", "zoho.com.cn"];
    let region = gtk::DropDown::from_strings(&regions);
    let region_row = widgets::row("Zoho data centre", "Use the region shown in your Zoho account", Some(region.upcast_ref()));
    region_row.set_visible(false);
    body.append(&region_row);
    provider.connect_selected_notify(move |provider| region_row.set_visible(provider.selected() == 3));
    let identity = entry(&body, "Email or username", "you@example.com");
    let label = entry(&body, "Account name", "Personal");
    let client_id = entry(&body, "OAuth client ID", "Required for Google and Microsoft");
    let secret = gtk::PasswordEntry::new();
    secret.set_show_peek_icon(true);
    body.append(&widgets::row(
        "App password or OAuth client secret",
        "Google desktop client secrets, if supplied, are stored in the keyring",
        Some(secret.upcast_ref()),
    ));
    body.append(&widgets::label("Google: create a Desktop OAuth client, enable the selected APIs, and add yourself as a test user. Microsoft: register a public desktop app for personal accounts with a loopback redirect. See the installed provider setup guide.", "dim"));
    let guide = gtk::Button::with_label("Provider setup guide");
    guide.connect_clicked(|_| {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(prefix) = exe.parent().and_then(|p| p.parent()) {
                let path = prefix.join("share/doc/nyx-accounts/PROVIDERS.md");
                let file = gtk::gio::File::for_path(path);
                let _ = gtk::gio::AppInfo::launch_default_for_uri(&file.uri(), None::<&gtk::gio::AppLaunchContext>);
            }
        }
    });
    body.append(&guide);
    let mut endpoints = Vec::new();
    for (key, title, hint) in [
        ("imap", "IMAP endpoint", "imaps://imap.example.com:993"),
        ("smtp", "SMTP endpoint", "smtp+starttls://smtp.example.com:587"),
        ("caldav", "Calendar endpoint", "https://server.example/dav/"),
        ("carddav", "Contacts endpoint", "https://server.example/dav/"),
        ("webdav", "Files endpoint", "https://server.example/dav/"),
    ] {
        endpoints.push((key, entry(&body, title, hint)));
    }
    let switches = services(&body, &BTreeSet::new());
    let add = gtk::Button::with_label("Save account");
    body.append(&add);
    add.connect_clicked(move |button| {
        let provider = providers[provider.selected() as usize].to_string();
        let oauth = matches!(provider.as_str(), "google" | "microsoft_personal");
        let mut account = Account {
            id: String::new(),
            provider,
            identity: identity.text().to_string(),
            label: label.text().to_string(),
            enabled: selected(&switches),
            status: String::new(),
            client_id: oauth.then(|| client_id.text().to_string()),
            endpoints: endpoints
                .iter()
                .filter(|(_, e)| !e.text().trim().is_empty())
                .map(|(k, e)| (k.to_string(), e.text().to_string()))
                .collect::<BTreeMap<_, _>>(),
            legacy_id: None,
            error: None,
            provider_user_id: None,
        };
        if let Err(error) = nyx_accounts_native::providers::apply_presets(&mut account, Some(regions[region.selected() as usize]))
            .and_then(|_| account.validate())
        {
            window::toast(&error.to_string());
            return;
        }
        let mut credentials = Credentials::empty();
        if oauth {
            credentials.client_secret = secret.text().to_string();
        } else {
            credentials.password = secret.text().to_string();
        }
        secret.set_text("");
        button.set_sensitive(false);
        let close = dialog.clone();
        cmd::background(
            move || {
                let client = Client::connect()?;
                let id = client.register(&account)?;
                client.save_secret(&id, &credentials)?;
                Ok::<_, anyhow::Error>(())
            },
            move |result| {
                if result.is_ok() {
                    close.close();
                    window::rebuild("online-accounts");
                } else {
                    window::toast("Account save failed. Unlock the keyring, refresh the account list, and retry.");
                    close.close();
                    window::rebuild("online-accounts");
                }
            },
        );
    });
}
fn manage(account: Account) {
    let (dialog, body) = super::cloud::dialog(&account.identity);
    let status = widgets::label("", "dim");
    let errors = widgets::vbox(0);
    display_status(&status, &errors, &account);
    body.append(&status);
    body.append(&errors);
    let check_id = account.id.clone();
    let check_status = status.clone();
    let check_errors = errors.clone();
    body.append(
        &widgets::button_row(
            "Check authorization",
            "Check the keyring and refresh OAuth credentials if needed",
            "Check",
            move |_| {
                let id = check_id.clone();
                let status = check_status.clone();
                let errors = check_errors.clone();
                cmd::background(
                    move || {
                        let client = Client::connect()?;
                        let _: String = client.call("CheckAccount", (id.as_str(),))?;
                        client.list()?.into_iter().find(|a| a.id == id).ok_or_else(|| anyhow::anyhow!("Account unavailable"))
                    },
                    move |result| {
                        match result {
                            Ok(account) => display_status(&status, &errors, &account),
                            Err(_) => window::toast("Authorization status is unavailable. Unlock the keyring and retry."),
                        }
                        window::rebuild("online-accounts");
                    },
                );
            },
        )
        .0,
    );
    let client_id = entry(&body, "OAuth client ID", "Desktop application client ID");
    client_id.set_text(account.client_id.as_deref().unwrap_or(""));
    let password = gtk::PasswordEntry::new();
    password.set_show_peek_icon(true);
    body.append(&widgets::row(
        "New app password or client secret",
        "Leave empty to retain the existing credential",
        Some(password.upcast_ref()),
    ));
    let mut server_entries = Vec::new();
    if !matches!(account.provider.as_str(), "google" | "microsoft_personal") {
        let (wrapper, server_body) = widgets::disclosure("Server settings", "Secure protocol endpoints");
        for (key, title) in
            [("imap", "IMAP"), ("smtp", "SMTP"), ("caldav", "CalDAV"), ("carddav", "CardDAV"), ("webdav", "WebDAV")]
        {
            let server = entry(&server_body, title, "Secure endpoint URL");
            server.set_text(account.endpoints.get(key).map(String::as_str).unwrap_or(""));
            server_entries.push((key.to_owned(), server));
        }
        body.append(&wrapper);
    }
    let edited = account.clone();
    body.append(
        &widgets::button_row("Account configuration", "Save before signing in again", "Save", move |_| {
            let mut edited = edited.clone();
            if matches!(edited.provider.as_str(), "google" | "microsoft_personal") {
                edited.client_id = Some(client_id.text().to_string());
            }
            for (key, entry) in &server_entries {
                let value = entry.text().to_string();
                if value.trim().is_empty() {
                    edited.endpoints.remove(key);
                } else {
                    edited.endpoints.insert(key.clone(), value);
                }
            }
            if let Err(error) = edited.validate() {
                window::toast(&error.to_string());
                return;
            }
            let value = password.text().to_string();
            password.set_text("");
            work(move |client| {
                client.call::<()>("UpdateConfiguration", (serde_json::to_string(&edited)?,))?;
                if !value.is_empty() {
                    let mut credentials = Credentials::empty();
                    if matches!(edited.provider.as_str(), "google" | "microsoft_personal") {
                        credentials.client_secret = value;
                    } else {
                        credentials.password = value;
                    }
                    client.save_secret(&edited.id, &credentials)?;
                }
                Ok(())
            });
        })
        .0,
    );
    let switches = services(&body, &account.enabled);
    let id = account.id.clone();
    body.append(
        &widgets::button_row("Enabled services", "OAuth permission changes require sign-in again", "Save", move |_| {
            let enabled = selected(&switches);
            let id = id.clone();
            work(move |c| c.set_services(&id, &enabled));
        })
        .0,
    );
    if matches!(account.provider.as_str(), "google" | "microsoft_personal") {
        let id = account.id.clone();
        body.append(
            &widgets::button_row("Sign in", "Authorize in your browser, then refresh connection status", "Sign in", move |_| {
                let id = id.clone();
                cmd::background(
                    move || Client::connect()?.authorize(&id),
                    |result| match result {
                        Ok(url) => {
                            if gtk::gio::AppInfo::launch_default_for_uri(&url, None::<&gtk::gio::AppLaunchContext>).is_err() {
                                window::toast("Could not open the browser.");
                            }
                        }
                        Err(_) => window::toast("Could not start authorization. Check the OAuth client configuration."),
                    },
                );
            })
            .0,
        );
    }
    if account.enabled.contains(&Service::Files) {
        super::cloud::append_for_account(&body, &account.id);
    }
    let revoke_id = account.id.clone();
    let revoke = widgets::confirm_button("Revoke access", "Disconnect and revoke access?", move |_| {
        let id = revoke_id.clone();
        cmd::background(
            move || Client::connect()?.call::<bool>("RevokeAuthorization", (id,)),
            |result| {
                match result {
                    Ok(true) => window::toast("Provider authorization was revoked."),
                    Ok(false) => window::toast(
                        "Local credentials were removed. Revoke the app password or app permission in your provider’s security settings.",
                    ),
                    Err(_) => window::toast(
                        "Revocation could not be completed. Unlock the keyring and retry, or revoke access in your provider’s security settings.",
                    ),
                }
                window::rebuild("online-accounts");
            },
        );
    });
    body.append(&widgets::row(
        "Revoke authorization",
        "Stops credential access; provider data is preserved",
        Some(revoke.upcast_ref()),
    ));
    let id = account.id.clone();
    let remove = widgets::confirm_button("Remove account", "Disconnect this account?", move |_| {
        let id = id.clone();
        work(move |c| c.remove(&id));
        dialog.close();
    });
    body.append(&widgets::row(
        "Remove account",
        "Provider data and local cached copies are preserved",
        Some(remove.upcast_ref()),
    ));
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
                    let detail = if issue.kind == "uncertain" {
                        "Task creation needs confirmation"
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
            if issue.kind == "uncertain" {
                "The provider may have created this task. Check the provider before retrying to avoid a duplicate.".into()
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
    if issue.kind == "uncertain" {
        choices.push((
            "Retry creation",
            "Only after checking the provider and confirming that the task was not created",
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
