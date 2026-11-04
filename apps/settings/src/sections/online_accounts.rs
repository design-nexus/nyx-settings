//! Native account management in Settings. On NyxOS this is the Accounts page: you at
//! the top, your online accounts, how you sign in, and the other users.
use crate::widgets::{self, Page};
use crate::window;
use gtk::prelude::*;
pub fn build(page: &Page) {
    if super::vpn_accounts::adding() {
        super::vpn_accounts::add_page(page);
        return;
    }
    let nyxos = crate::nyxos::active() && super::native_accounts::on_list();
    if nyxos {
        you(page);
    }
    super::native_accounts::build(page);
    if super::native_accounts::on_list() {
        super::vpn_accounts::build(page);
    }
    if nyxos {
        sign_in_and_users(page);
    }
}

fn you(page: &Page) {
    let me = crate::backend::accounts::me();
    let name =
        crate::backend::accounts::people().into_iter().find(|u| u.name == me).map(|u| u.full_name).filter(|n| !n.is_empty());
    let g = page.group("You");
    let picture = gtk::Image::from_icon_name("avatar-default-symbolic");
    picture.set_pixel_size(48);
    let (r, _) = widgets::button_row(name.as_deref().unwrap_or(&me), &format!("Signed in as {me}"), "Edit…", |_| {
        window::navigate("accounts")
    });
    r.prepend(&picture);
    widgets::keywords("profile name picture password me");
    g.add(&r);
}

fn sign_in_and_users(page: &Page) {
    let g = page.group("Biometrics");
    let (r, _) = widgets::button_row("Face and fingerprint", "Checking…", "Set Up", |_| window::navigate("sign-in"));
    widgets::keywords("biometrics face fingerprint security key");
    g.add(&r);
    let row = r.clone();
    super::sign_in::summary(move |text| {
        if let Some(desc) = find_description(&row) {
            desc.set_text(&text);
        }
    });
    let g = page.group("");
    let (r, _) = widgets::button_row("Other users", "Accounts for other people on this computer.", "Open", |_| {
        window::navigate("accounts")
    });
    widgets::keywords("users people add user family");
    g.add(&r);
}

/// A row's description label (its title's sibling).
fn find_description(row: &gtk::Box) -> Option<gtk::Label> {
    let text = row.first_child()?;
    let mut child = text.first_child();
    while let Some(c) = child {
        if c.has_css_class("settings-option-description") {
            return c.downcast().ok();
        }
        child = c.next_sibling();
    }
    None
}
