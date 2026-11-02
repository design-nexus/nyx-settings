//! Nyx Assistant: its settings, what it's allowed to do, what it's doing and its models.
//! Every value is read and written through `nyx-assistant config`, which checks it.

pub mod activity;
pub mod appearance;
pub mod client;
pub mod general;
pub mod models;
pub mod performance;
pub mod permissions;
pub mod voice;

use crate::widgets::{self, Debounce, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;

/// Save one setting off the main thread; say why if the assistant refused it.
pub fn save(key: &str, value: String) {
    let key = key.to_string();
    cmd::background(
        move || client::set(&key, &value),
        |r| {
            if let Err(e) = r {
                window::toast(&format!("Couldn't save: {e}"));
            }
        },
    );
}

/// The settings, or a banner saying why they couldn't be read (the page then stops).
pub fn config_or_banner(page: &Page) -> Option<Value> {
    let cfg = client::config();
    if cfg.is_object() {
        return Some(cfg);
    }
    page.banner("Couldn't read the assistant's settings. Update Nyx Assistant (./install.sh assistant) and try again.", true);
    None
}

pub fn switch(cfg: &Value, key: &'static str, title: &str, desc: &str) -> gtk::Box {
    let on = client::get(cfg, key).as_bool().unwrap_or(false);
    widgets::switch_row(title, desc, on, move |on| save(key, on.to_string())).0
}

pub fn choice(cfg: &Value, key: &'static str, title: &str, desc: &str, options: &[(&str, &str)]) -> gtk::Box {
    let options: Vec<(String, String)> = options.iter().map(|(id, l)| (id.to_string(), l.to_string())).collect();
    let current = client::get_str(cfg, key);
    widgets::choice_row(title, desc, options, &current, move |id| save(key, client::quoted(&id))).0
}

/// A number setting; saved once the slider rests.
pub fn slider(
    cfg: &Value,
    key: &'static str,
    title: &str,
    desc: &str,
    range: (f64, f64, f64),
    digits: u32,
    unit: &str,
) -> gtk::Box {
    let value = client::get(cfg, key).as_f64().unwrap_or(range.0);
    let wait = Debounce::default();
    widgets::slider_row(title, desc, range, value, digits, unit, move |v| {
        let text = if digits == 0 { format!("{}", v.round() as i64) } else { format!("{:.*}", digits as usize, v) };
        wait.call(400, move || save(key, text));
    })
    .0
}

/// A list of words, edited as comma-separated text.
pub fn words(cfg: &Value, key: &'static str, title: &str, desc: &str, placeholder: &str) -> gtk::Box {
    let now: Vec<String> = client::get(cfg, key)
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let (row, entry) = widgets::entry_row(title, desc, &now.join(", "), placeholder, move |text| {
        let items: Vec<String> = text.split(',').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect();
        save(key, client::list(&items));
    });
    entry.set_width_chars(30);
    row
}

/// "5.3 GB", "640 MB".
pub fn size(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", bytes as f64 / 1e6) }
}

/// "14:03:22" today, else "Oct 12 14:03".
pub fn when(ms: u64) -> String {
    let Some(t) = glib::DateTime::from_unix_local((ms / 1000) as i64).ok() else { return String::new() };
    let today = glib::DateTime::now_local().ok().map(|n| n.ymd()) == Some(t.ymd());
    t.format(if today { "%H:%M:%S" } else { "%b %e %H:%M" }).map(|s| s.to_string()).unwrap_or_default()
}

/// A dim one-line label.
pub fn dim(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("dim");
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l
}
