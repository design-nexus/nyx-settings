//! A small modal form: text, password and choice fields with Cancel and an action
//! button. Used for adding and editing things.

use crate::{widgets, window};
use gtk::prelude::*;
use std::rc::Rc;

pub enum Field {
    Text {
        placeholder: &'static str,
        value: String,
    },
    Secret(&'static str),
    /// A drop-down of (id, label) options with the current id.
    Choice {
        options: Vec<(String, String)>,
        current: String,
    },
}

impl Field {
    pub fn text(placeholder: &'static str) -> Field {
        Field::Text { placeholder, value: String::new() }
    }

    /// A text field that starts out filled in, for editing.
    pub fn text_with(placeholder: &'static str, value: &str) -> Field {
        Field::Text { placeholder, value: value.to_string() }
    }

    pub fn secret(placeholder: &'static str) -> Field {
        Field::Secret(placeholder)
    }

    pub fn choice(options: Vec<(String, String)>, current: &str) -> Field {
        Field::Choice { options, current: current.to_string() }
    }
}

/// Show the form. `on_ok` gets each field's value in order (the id for a choice)
/// and the checkbox state, and returns an error to show in the dialog, or `None`
/// to close it.
pub fn ask(
    title: &str,
    intro: &str,
    fields: Vec<Field>,
    ok: &str,
    extra: Option<(&str, bool)>,
    on_ok: impl Fn(Vec<String>, bool) -> Option<String> + 'static,
) {
    let Some(win) = window::window() else { return };
    let dialog = gtk::Window::builder().transient_for(&win).modal(true).title(title).build();
    dialog.add_css_class("settings-window");
    dialog.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    let card = widgets::vbox(12);
    card.add_css_class("dialog-card");
    card.append(&widgets::label(title, "section-title"));
    if !intro.is_empty() {
        let l = widgets::label(intro, "dim");
        l.set_wrap(true);
        card.append(&l);
    }
    let mut getters: Vec<Box<dyn Fn() -> String>> = Vec::new();
    let mut first: Option<gtk::Widget> = None;
    let mut last_entry: Option<gtk::Widget> = None;
    for f in fields {
        match f {
            Field::Text { placeholder, value } => {
                let e = gtk::Entry::new();
                e.set_placeholder_text(Some(placeholder));
                e.set_text(&value);
                e.set_width_chars(28);
                card.append(&e);
                first.get_or_insert(e.clone().upcast());
                last_entry = Some(e.clone().upcast());
                getters.push(Box::new(move || e.text().to_string()));
            }
            Field::Secret(placeholder) => {
                let e = gtk::PasswordEntry::new();
                e.set_show_peek_icon(true);
                e.set_placeholder_text(Some(placeholder));
                card.append(&e);
                first.get_or_insert(e.clone().upcast());
                last_entry = Some(e.clone().upcast());
                getters.push(Box::new(move || e.text().to_string()));
            }
            Field::Choice { options, current } => {
                let dd = widgets::dropdown(&options, &current);
                card.append(&dd);
                last_entry = None;
                getters.push(Box::new(move || options.get(dd.selected() as usize).map(|(id, _)| id.clone()).unwrap_or_default()));
            }
        }
    }
    let check = extra.map(|(label, on)| {
        let c = gtk::CheckButton::with_label(label);
        c.set_active(on);
        card.append(&c);
        c
    });
    let error = widgets::label("", "dim");
    error.set_wrap(true);
    error.set_visible(false);
    card.append(&error);
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let go = gtk::Button::with_label(ok);
    go.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&go);
    card.append(&buttons);
    dialog.set_child(Some(&card));

    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());
    let submit: Rc<dyn Fn()> = Rc::new({
        let (dialog, error) = (dialog.clone(), error.clone());
        move || {
            let values: Vec<String> = getters.iter().map(|g| g()).collect();
            match on_ok(values, check.as_ref().is_some_and(|c| c.is_active())) {
                Some(msg) => {
                    error.set_text(&msg);
                    error.set_visible(true);
                }
                None => dialog.close(),
            }
        }
    });
    let s = submit.clone();
    go.connect_clicked(move |_| s());
    if let Some(last) = last_entry {
        let s = submit.clone();
        if let Some(e) = last.downcast_ref::<gtk::Entry>() {
            e.connect_activate(move |_| s());
        } else if let Some(e) = last.downcast_ref::<gtk::PasswordEntry>() {
            e.connect_activate(move |_| s());
        }
    }

    escape(&dialog);
    dialog.present();
    if let Some(first) = first {
        first.grab_focus();
    }
}

/// Let focused controls handle Escape first, then dismiss through close-request guards.
fn escape(window: &gtk::Window) {
    let keys = gtk::EventControllerKey::new();
    let weak = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });
    window.add_controller(keys);
}
