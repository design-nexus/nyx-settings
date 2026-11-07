use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;

fn refresh_later() {
    glib::timeout_add_local_once(std::time::Duration::from_millis(800), || window::rebuild("plugins"));
}

pub fn build(page: &Page) {
    if crate::platform::PLATFORM == crate::platform::Platform::NyxOS {
        return nyx_plugins(page);
    }
    let list: Vec<Value> = cmd::output(&["omarchy-plugin-list", "--json"])
        .and_then(|t| serde_json::from_str(&t).ok())
        .and_then(|v: Value| v.as_array().cloned())
        .unwrap_or_default();
    if list.is_empty() {
        page.banner("Couldn't list plugins.", true);
        return;
    }

    let g = page.group("Keep up to date");
    let (r, _) = widgets::button_row(
        "Update plugins",
        "Fetch the latest version of every plugin installed from git.",
        "Update all",
        |b| {
            b.set_sensitive(false);
            let b = b.clone();
            cmd::run_async(&["omarchy-plugin-update", "--yes"], move |r| {
                b.set_sensitive(true);
                match r {
                    Ok(_) => window::toast("Plugins updated"),
                    Err(e) => window::toast(&format!("{e}")),
                }
                refresh_later();
            });
        },
    );
    g.add(&r);
    let (r, _) = widgets::button_row("Find more", "Browse the Omarchy plugin catalog.", "Catalog", |_| {
        cmd::spawn(&["xdg-open", "https://plugins.omarchy.org/"]);
    });
    g.add(&r);

    for (title, first_party) in [("Third-party", false), ("Omarchy", true)] {
        let g = page.group(title);
        let mut items: Vec<&Value> =
            list.iter().filter(|p| p.get("firstParty").and_then(|v| v.as_bool()) == Some(first_party)).collect();
        items.sort_by_key(|p| p.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase());
        for p in items {
            let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or(&id).to_string();
            let kinds: Vec<String> = p
                .get("kinds")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|k| k.as_str().map(|s| s.replace('-', " "))).collect())
                .unwrap_or_default();
            let enabled = p.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
            let can_disable = p.get("canDisable").and_then(|v| v.as_bool()).unwrap_or(true);
            let controls = widgets::hbox(8);
            if !first_party {
                let remove = gtk::Button::from_icon_name("user-trash-symbolic");
                remove.add_css_class("flat");
                remove.set_tooltip_text(Some("Remove"));
                let id2 = id.clone();
                let name2 = name.clone();
                remove.connect_clicked(move |b| {
                    // Two clicks: the first arms it.
                    if !b.has_css_class("destructive-action") {
                        b.add_css_class("destructive-action");
                        b.set_tooltip_text(Some("Click again to remove"));
                        window::toast(&format!("Click the bin again to remove {name2}"));
                        return;
                    }
                    let n = name2.clone();
                    cmd::run_async(&["omarchy-plugin-remove", &id2, "--yes"], move |r| {
                        match r {
                            Ok(_) => window::toast(&format!("Removed {n}")),
                            Err(e) => window::toast(&format!("{e}")),
                        }
                        refresh_later();
                    });
                });
                controls.append(&remove);
            }
            let sw = gtk::Switch::new();
            sw.set_active(enabled);
            sw.set_sensitive(can_disable);
            sw.set_valign(gtk::Align::Center);
            let id2 = id.clone();
            sw.connect_active_notify(move |s| {
                let verb = if s.is_active() { "omarchy-plugin-enable" } else { "omarchy-plugin-disable" };
                cmd::run_async(&[verb, &id2], |r| {
                    if let Err(e) = r {
                        window::toast(&format!("{e}"));
                    }
                });
            });
            controls.append(&sw);
            let desc = format!("<tt>{}</tt> · {}", glib::markup_escape_text(&id), kinds.join(", "));
            g.add(&widgets::row(&name, &desc, Some(controls.upcast_ref())));
        }
    }
}

/// NyxOS: Nyx plugins through `nyx plugin` (the bar's now; the Control Center's and the
/// desktop's later). The ones NyxOS comes with can only be turned off.
fn nyx_plugins(page: &Page) {
    let list: Vec<Value> = cmd::output(&["nyx", "plugin", "list", "--json"])
        .and_then(|t| serde_json::from_str(&t).ok())
        .and_then(|v: Value| v.as_array().cloned())
        .unwrap_or_default();
    if list.is_empty() {
        page.banner("Couldn't list plugins.", true);
        return;
    }

    let g = page.group("Keep up to date");
    let (r, _) = widgets::button_row("Update plugins", "Fetch the latest version of every plugin added from git.", "Update all", |b| {
        b.set_sensitive(false);
        let b = b.clone();
        cmd::run_async(&["nyx", "plugin", "update", "--yes"], move |r| {
            b.set_sensitive(true);
            match r {
                Ok(_) => window::toast("Plugins updated"),
                Err(e) => window::toast(&format!("{e}")),
            }
            refresh_later();
        });
    });
    g.add(&r);

    for (title, packaged) in [("Added", false), ("Included with NyxOS", true)] {
        let mut items: Vec<&Value> =
            list.iter().filter(|p| (p.get("source").and_then(Value::as_str) == Some("packaged")) == packaged).collect();
        if items.is_empty() {
            continue;
        }
        items.sort_by_key(|p| p.get("name").and_then(Value::as_str).unwrap_or("").to_lowercase());
        let g = page.group(title);
        for p in items {
            let id = p.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let name = p.get("name").and_then(Value::as_str).unwrap_or(&id).to_string();
            let kinds: Vec<String> = p
                .get("kinds")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|k| k.as_str().map(|s| s.replace('-', " "))).collect())
                .unwrap_or_default();
            let enabled = p
                .get("enabled")
                .and_then(Value::as_object)
                .is_some_and(|parts| parts.values().any(|v| v.as_array().is_some_and(|a| !a.is_empty())));
            let controls = widgets::hbox(8);
            if !packaged {
                let remove = gtk::Button::from_icon_name("user-trash-symbolic");
                remove.add_css_class("flat");
                remove.set_tooltip_text(Some("Remove"));
                let (id, name) = (id.clone(), name.clone());
                remove.connect_clicked(move |b| {
                    if !b.has_css_class("destructive-action") {
                        b.add_css_class("destructive-action");
                        b.set_tooltip_text(Some("Click again to remove"));
                        window::toast(&format!("Click the bin again to remove {name}"));
                        return;
                    }
                    let n = name.clone();
                    cmd::run_async(&["nyx", "plugin", "remove", &id, "--yes"], move |r| {
                        match r {
                            Ok(_) => window::toast(&format!("Removed {n}")),
                            Err(e) => window::toast(&format!("{e}")),
                        }
                        refresh_later();
                    });
                });
                controls.append(&remove);
            }
            let sw = gtk::Switch::new();
            sw.set_active(enabled);
            sw.set_valign(gtk::Align::Center);
            let id2 = id.clone();
            sw.connect_active_notify(move |s| {
                let verb = if s.is_active() { "enable" } else { "disable" };
                cmd::run_async(&["nyx", "plugin", verb, &id2], |r| {
                    if let Err(e) = r {
                        window::toast(&format!("{e}"));
                    }
                });
            });
            controls.append(&sw);
            let desc = format!("<tt>{}</tt> · {}", glib::markup_escape_text(&id), kinds.join(", "));
            g.add(&widgets::row(&name, &desc, Some(controls.upcast_ref())));
        }
    }
}
