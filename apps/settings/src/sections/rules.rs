//! Programs started with the session, and rules for how windows and shell layers behave.

use crate::backend::autostart;
use crate::backend::rules::{self, Value};
use crate::backend::state::Rule;
use crate::backend::{hypr, store};
use crate::dialog::{Field, ask};
use crate::widgets::{self, Page};
use crate::window;
use gtk::gio;
use gtk::prelude::*;

pub fn build(page: &Page) {
    startup(page);
    rule_group(page, false);
    rule_group(page, true);
}

// ----- Startup apps -----

fn launcher_icon(icon: &str, fallback: &str) -> gtk::Image {
    let img = if icon.starts_with('/') && std::path::Path::new(icon).exists() {
        gtk::Image::from_file(icon)
    } else if !icon.is_empty() && gtk::gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(icon)) {
        gtk::Image::from_icon_name(icon)
    } else {
        gtk::Image::from_icon_name(fallback)
    };
    img.set_pixel_size(24);
    img.set_valign(gtk::Align::Center);
    img.add_css_class("launcher-icon");
    img
}

fn xdg_row(g: &widgets::Group, item: &autostart::Item) {
    let controls = widgets::hbox(6);
    let sw = gtk::Switch::new();
    sw.set_active(item.enabled);
    sw.set_valign(gtk::Align::Center);
    {
        let id = item.id.clone();
        let is_system = item.is_system;
        let reverting = std::cell::Cell::new(false);
        sw.connect_active_notify(move |s| {
            if reverting.replace(false) {
                return;
            }
            let active = s.is_active();
            if let Err(e) = autostart::set_enabled(&id, is_system, active) {
                window::toast(&e);
                reverting.set(true);
                s.set_active(!active);
            }
        });
    }
    controls.append(&sw);

    if !item.is_system {
        let id = item.id.clone();
        let remove = widgets::confirm_button("Remove", "Remove?", move |_| {
            if let Err(e) = autostart::remove(&id, false) {
                window::toast(&e);
            } else {
                window::rebuild("rules");
            }
        });
        controls.append(&remove);
    }

    let desc = if !item.comment.is_empty() {
        gtk::glib::markup_escape_text(&item.comment).to_string()
    } else if !item.exec.is_empty() {
        format!("<tt>{}</tt>", gtk::glib::markup_escape_text(&item.exec))
    } else {
        String::new()
    };

    let r = widgets::row(&item.name, &desc, Some(controls.upcast_ref()));
    r.prepend(&launcher_icon(&item.icon, "application-x-executable-symbolic"));
    g.add(&r);
    widgets::keywords("autostart startup login session boot launch");
}

fn startup(page: &Page) {
    let g = page.group("Startup apps");
    g.note("Programs that start when you sign in, unless they're already running.");

    let (system_items, own_items): (Vec<_>, Vec<_>) = autostart::list().into_iter().partition(|i| i.is_system);
    for item in &own_items {
        xdg_row(&g, item);
    }

    // The night light schedule manages its own entry.
    let entries: Vec<(String, String)> = store::read(|s| {
        s.autostart.iter().filter(|(k, _)| k.as_str() != "hyprsunset").map(|(k, v)| (k.clone(), v.clone())).collect()
    });
    for (process, command) in entries {
        let edit = gtk::Button::from_icon_name("document-edit-symbolic");
        edit.add_css_class("flat");
        edit.set_tooltip_text(Some("Edit"));
        {
            let (process, command) = (process.clone(), command.clone());
            edit.connect_clicked(move |_| {
                let (process, command) = (process.clone(), command.clone());
                ask(
                    "Edit startup program",
                    "The command that starts it.",
                    vec![Field::text_with("command", &command)],
                    "Save",
                    None,
                    move |v, _| {
                        let text = v[0].trim().to_string();
                        let Some(new_process) = rules::process_of(&text) else {
                            return Some("Couldn't tell which program that starts.".into());
                        };
                        let new_command = if text.starts_with("uwsm-app") { text } else { format!("uwsm-app -- {text}") };
                        let old = process.clone();
                        store::update(true, move |s| {
                            s.autostart.remove(&old);
                            s.autostart.insert(new_process, new_command);
                        });
                        window::rebuild("rules");
                        None
                    },
                );
            });
        }
        let key = process.clone();
        let remove = widgets::confirm_button("Remove", "Remove?", move |_| {
            let key = key.clone();
            store::update(true, move |s| {
                s.autostart.remove(&key);
            });
            window::rebuild("rules");
        });
        let controls = widgets::hbox(6);
        controls.append(&edit);
        controls.append(&remove);
        let r = widgets::row(&process, &gtk::glib::markup_escape_text(&command), Some(controls.upcast_ref()));
        r.prepend(&launcher_icon("", "utilities-terminal-symbolic"));
        g.add(&r);
        widgets::keywords("autostart startup login session boot launch");
    }

    let mut installed_apps: Vec<(String, String)> = gio::AppInfo::all()
        .into_iter()
        .filter(|a| a.should_show())
        .filter_map(|a| {
            let id = a.id()?.to_string();
            let name = a.name().to_string();
            Some((id, name))
        })
        .collect();
    installed_apps.sort_by(|a, b| a.0.cmp(&b.0));
    installed_apps.dedup_by(|a, b| a.0 == b.0);
    installed_apps.sort_by_key(|a| a.1.to_lowercase());

    if !installed_apps.is_empty() {
        let mut options = vec![(String::new(), "Pick an installed app to start at login…".to_string())];
        options.extend(installed_apps);
        let (picker_row, _) = widgets::choice_row("Add an installed app", "Starts when you sign in.", options, "", |id| {
            if id.is_empty() {
                return;
            }
            if let Some(app) = gio::AppInfo::all().into_iter().find(|a| a.id().is_some_and(|i| i == id.as_str())) {
                match autostart::add_app(&app) {
                    Ok(name) => {
                        window::toast(&format!("{name} added to startup"));
                        window::rebuild("rules");
                    }
                    Err(e) => window::toast(&e),
                }
            }
        });
        g.add(&picker_row);
    }

    let (r, _) = widgets::entry_row(
        "Add a command",
        "A custom command, e.g. nm-applet or discord --start-minimized. Press Enter.",
        "",
        "command",
        |text| {
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            let Some(process) = rules::process_of(&text) else {
                window::toast("Couldn't tell which program that starts");
                return;
            };
            let command = if text.starts_with("uwsm-app") { text } else { format!("uwsm-app -- {text}") };
            store::update(true, move |s| {
                s.autostart.insert(process, command);
            });
            window::rebuild("rules");
        },
    );
    widgets::keywords("autostart startup login session add launch");
    g.add(&r);

    if !system_items.is_empty() {
        let sg = page.group("Started by the system");
        sg.note("Installed with your apps and system. You can turn them off here, but not remove them.");
        for item in &system_items {
            xdg_row(&sg, item);
        }
    }
}

// ----- Rules -----

fn rule_group(page: &Page, layer: bool) {
    let (title, note) = if layer {
        ("Layer rules", "Adjust the shell's own surfaces, such as the bar or menus, by their namespace.")
    } else {
        ("Window rules", "Make windows open a certain way. Match by app class and/or title; both are patterns.")
    };
    let g = page.group(title);
    g.note(note);
    let list: Vec<Rule> = store::read(|s| if layer { s.layer_rules.clone() } else { s.window_rules.clone() });
    for (i, rule) in list.iter().enumerate() {
        let (who, what) = rules::describe(layer, rule);
        let edit = gtk::Button::from_icon_name("document-edit-symbolic");
        edit.add_css_class("flat");
        edit.set_tooltip_text(Some("Edit"));
        {
            let rule = rule.clone();
            edit.connect_clicked(move |_| edit_rule(layer, i, &rule));
        }
        let remove = widgets::confirm_button("Remove", "Remove?", move |_| {
            store::update(true, move |s| {
                let rules = if layer { &mut s.layer_rules } else { &mut s.window_rules };
                if i < rules.len() {
                    rules.remove(i);
                }
            });
            window::rebuild("rules");
        });
        let controls = widgets::hbox(6);
        controls.append(&edit);
        controls.append(&remove);
        g.add(&widgets::row(&who, &gtk::glib::markup_escape_text(&what), Some(controls.upcast_ref())));
        widgets::keywords("rule window layer match class title float opacity workspace blur");
    }
    g.add(&widgets::form_disclosure(if layer { "Add a layer rule…" } else { "Add a window rule…" }, "", &rule_form(layer)));
    widgets::keywords("new add rule window layer");
}

/// Change a rule in place: what it matches, what it does and its value.
fn edit_rule(layer: bool, index: usize, rule: &Rule) {
    let effects = rules::effects(layer);
    let options: Vec<(String, String)> = effects.iter().map(|e| (e.key.to_string(), e.label.to_string())).collect();
    let mut fields = vec![Field::text_with(if layer { "Namespace" } else { "App class (pattern)" }, &rule.class)];
    if !layer {
        fields.push(Field::text_with("Title (pattern, optional)", &rule.title));
    }
    fields.push(Field::choice(options, &rule.effect));
    fields.push(Field::text_with("Value, if the effect needs one (e.g. 0.9 0.8)", &rule.value));
    ask(if layer { "Edit layer rule" } else { "Edit window rule" }, "", fields, "Save", None, move |v, _| {
        let mut it = v.into_iter();
        let class = it.next().unwrap_or_default().trim().to_string();
        let title = if layer { String::new() } else { it.next().unwrap_or_default().trim().to_string() };
        let effect = it.next().unwrap_or_default();
        let value = it.next().unwrap_or_default().trim().to_string();
        // On/off effects take no value; don't keep one typed for a previous effect.
        let value = if effects.iter().any(|e| e.key == effect && e.value == Value::Flag) { String::new() } else { value };
        let new = Rule { class, title, effect, value };
        if rules::lua_line(layer, &new).is_none() {
            return Some("That rule isn't complete: it needs something to match and, for some effects, a value.".into());
        }
        store::update(true, move |s| {
            let list = if layer { &mut s.layer_rules } else { &mut s.window_rules };
            if index < list.len() {
                list[index] = new;
            }
        });
        window::rebuild("rules");
        None
    });
}

/// Classes of the windows open now, for picking instead of typing.
fn open_classes() -> Vec<String> {
    let mut classes: Vec<String> = hypr::json(&["clients"])
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|c| c.get("class").and_then(|c| c.as_str()).map(str::to_string))
        .filter(|c| !c.is_empty())
        .collect();
    classes.sort();
    classes.dedup();
    classes
}

fn rule_form(layer: bool) -> gtk::Box {
    let form = widgets::vbox(8);
    let class = gtk::Entry::new();
    class.set_placeholder_text(Some(if layer { "Namespace, e.g. omarchy-bar" } else { "App class, e.g. ^mpv$" }));
    let title = gtk::Entry::new();
    title.set_placeholder_text(Some("Title pattern (optional)"));
    if !layer {
        let open = open_classes();
        if !open.is_empty() {
            let mut options = vec![(String::new(), "Pick an open window…".to_string())];
            options.extend(open.iter().map(|c| (c.clone(), c.clone())));
            let dd = widgets::dropdown(&options, "");
            let class = class.clone();
            let options2 = options.clone();
            dd.connect_selected_notify(move |d| {
                if let Some((value, _)) = options2.get(d.selected() as usize).filter(|(v, _)| !v.is_empty()) {
                    class.set_text(&format!("^{}$", value.replace('.', "\\.")));
                }
            });
            form.append(&dd);
        }
    }
    form.append(&class);
    if !layer {
        form.append(&title);
    }
    let effects = rules::effects(layer);
    let options: Vec<(String, String)> = effects.iter().map(|e| (e.key.to_string(), e.label.to_string())).collect();
    let effect = widgets::dropdown(&options, effects[0].key);
    let value = gtk::Entry::new();
    value.set_visible(false);
    let sync = {
        let (effect, value) = (effect.clone(), value.clone());
        move || {
            if let Some(e) = effects.get(effect.selected() as usize) {
                value.set_visible(e.value != Value::Flag);
                value.set_placeholder_text(Some(e.hint));
            }
        }
    };
    sync();
    effect.connect_selected_notify(move |_| sync());
    form.append(&effect);
    form.append(&value);
    let add = gtk::Button::with_label("Add rule");
    add.add_css_class("suggested-action");
    add.connect_clicked(move |_| {
        let Some(chosen) = effects.get(effect.selected() as usize) else { return };
        let rule = Rule {
            class: class.text().trim().to_string(),
            title: title.text().trim().to_string(),
            effect: chosen.key.to_string(),
            value: if chosen.value == Value::Flag { String::new() } else { value.text().trim().to_string() },
        };
        if rules::lua_line(layer, &rule).is_none() {
            window::toast(if layer {
                "Enter a namespace and, if needed, a value"
            } else {
                "Enter a class or title and, if needed, a value"
            });
            return;
        }
        store::update(true, move |s| if layer { s.layer_rules.push(rule) } else { s.window_rules.push(rule) });
        window::rebuild("rules");
    });
    form.append(&add);
    form
}
