//! NyxOS's Accounts page: you at the top, your online accounts, files and VPN, then how
//! you sign in and the other people (Users, opened from here).

use super::users::avatar;
use crate::backend::{accounts as acc, passwd};
use crate::platform::nyxos;
use crate::sections::{native_accounts, online_accounts, sign_in, vpn_accounts};
use crate::widgets::{self, Page};
use crate::window;
use gtk::prelude::*;

pub fn build(page: &Page) {
    let listing = native_accounts::on_list() && !vpn_accounts::adding();
    let before = page.body.last_child();
    online_accounts::build(page);
    // When the page splits into the list and the account picked, the list stays while an
    // account is added; otherwise adding one takes the whole page, without you or the people.
    let split = nyxos::descendant(&page.body, "split-controls").is_some();
    if !split && !listing {
        return;
    }
    let you = you(page);
    if let Some(parent) = you.parent().and_downcast::<gtk::Box>() {
        parent.reorder_child_after(&you, if split { None } else { before.as_ref() });
    }
    sign_in_and_users(page);
}

/// You: your face, name and role, and Edit, which opens Users.
fn you(page: &Page) -> gtk::Box {
    let me = acc::me();
    let user = acc::people().into_iter().find(|u| u.name == me);
    let full = user.as_ref().map(|u| u.full_name.clone()).unwrap_or_default();
    let admin = user.as_ref().is_some_and(|u| acc::is_admin(u, &passwd::groups()));
    let g = page.group("");
    let edit = gtk::Button::with_label("Edit");
    edit.set_tooltip_text(Some("Your name and password, in Users"));
    edit.connect_clicked(|_| window::navigate("accounts"));
    let role = if admin { "Administrator" } else { "Standard account" };
    let r = widgets::row(if full.is_empty() { &me } else { &full }, &format!("Signed in as {me} · {role}"), Some(edit.upcast_ref()));
    r.prepend(&avatar(&full, &me, 44));
    r.add_css_class("nxu-you");
    widgets::keywords("you profile name picture password me");
    g.add(&r);
    g.wrapper
}

fn sign_in_and_users(page: &Page) {
    let g = page.group("");
    let (r, value) = widgets::nav_row(Some("auth-fingerprint-symbolic"), "Face and fingerprint", "", "Checking…", || {
        window::navigate("sign-in")
    });
    widgets::keywords("biometrics face fingerprint security key");
    g.add(&r);
    sign_in::summary(move |text| {
        // "Face, 2 fingerprints. Your password works too." → "Face, 2 fingerprints".
        let short = text.split(". ").next().unwrap_or(&text).trim_end_matches('.').to_string();
        value.set_text(&short);
        value.set_tooltip_text(Some(&text));
    });
    let me = acc::me();
    let others = acc::people().iter().filter(|u| u.name != me).count();
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname").map(|h| h.trim().to_string()).unwrap_or_default();
    let title = if host.is_empty() { "Other people".to_string() } else { format!("Other people on {host}") };
    let (r, _) = widgets::nav_row(
        Some("system-users-symbolic"),
        &title,
        if others == 0 { "Add someone else who uses this computer." } else { "" },
        &if others == 0 { String::new() } else { others.to_string() },
        || window::navigate("accounts"),
    );
    widgets::keywords("users people add user family other users");
    g.add(&r);
}
