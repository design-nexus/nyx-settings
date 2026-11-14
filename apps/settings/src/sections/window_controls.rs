//! Tiling & Workspaces › Window controls (adaptive window controls): Nyx apps' single
//! Close button and the window controls it opens, everywhere; on NyxOS, with its
//! decorations (nyx-decor) loaded, the headerbar, the control on floating terminals,
//! and how each app's controls show and whether it opens floating.

use crate::backend::window_controls as wc;
use crate::widgets::{self, Page, hypr_choice_typed, hypr_slider, hypr_switch};
use crate::window;
use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};
use std::cell::RefCell;

/// How an app's controls show (`controls`): `auto` leaves it unset.
const PRESENTATIONS: &[(&str, &str)] = &[
    ("auto", "Automatic"),
    ("custom-headerbar", "Nyx headerbar"),
    ("chromeless-floating", "Small control while floating"),
    ("preserve-native", "Its own controls"),
];

/// Whether it opens floating (`float`).
const OPEN_AS: &[(&str, &str)] = &[("auto", "As Hyprland decides"), ("float", "Floating"), ("tile", "Tiled")];

fn options(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(id, l)| (id.to_string(), l.to_string())).collect()
}

fn report(r: Result<(), String>) {
    if let Err(e) = r {
        window::toast(&e);
    }
}

thread_local! {
    static PENDING_DELAY: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
}

/// Write the hover delay once the slider rests, not at every step of a drag.
fn set_delay_later(ms: u64) {
    PENDING_DELAY.with(|p| {
        if let Some(id) = p.borrow_mut().take() {
            id.remove();
        }
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
            PENDING_DELAY.with(|p| p.borrow_mut().take());
            report(wc::set("hover_delay_ms", json!(ms)));
        });
        *p.borrow_mut() = Some(id);
    });
}

pub fn build(page: &Page) {
    let s = wc::settings();
    let nyxos = wc::decorations();
    let g = page.group(if nyxos { "Window controls" } else { "Nyx apps' window controls" });
    let (r, _) = widgets::switch_row(
        "Adaptive window controls",
        if nyxos {
            "Nyx apps show one Close button that opens the other window controls; apps without controls get a Nyx headerbar, and floating terminals a small control."
        } else {
            "Nyx apps show one Close button at the end of their header; resting the pointer on it, a right-click or the Menu key opens the other window controls. Off, they show none."
        },
        wc::enabled(&s),
        |on| report(wc::set("enabled", json!(on))),
    );
    g.add(&r);
    let (r, _) = widgets::slider_row(
        "Open the other controls after",
        "How long the pointer rests on a Nyx app's Close button first. Apps take a change when they next open.",
        (0.0, 1000.0, 50.0),
        wc::hover_delay_ms(&s) as f64,
        0,
        " ms",
        |v| set_delay_later(v.round() as u64),
    );
    g.add(&r);

    if !nyxos {
        widgets::keywords("window controls close button hover delay popover maximize minimize float tile");
        return;
    }
    g.add(&hypr_slider(
        "plugin.nyx_decor.height",
        "Headerbar height",
        "The Nyx headerbar on apps without controls of their own.",
        (24.0, 48.0, 1.0),
        0,
        " px",
        true,
    ));
    g.add(&hypr_switch(
        "plugin.nyx_decor.double_click_maximize",
        "Double-click a headerbar to maximize",
        "Again to bring the window back to its size.",
    ));
    g.add(&hypr_switch(
        "plugin.nyx_decor.restore_float_geometry",
        "Float windows where they were",
        "A window you tile and float again goes back to where it floated, inside its screen.",
    ));
    g.add(&hypr_switch(
        "plugin.nyx_decor.corner_button",
        "Control on floating terminals",
        "A small button in a corner of a floating terminal (or another app without chrome) that opens its window menu.",
    ));
    g.add(&hypr_choice_typed(
        "plugin.nyx_decor.corner_button_corner",
        "Its corner",
        "",
        options(&[("0", "Top right"), ("1", "Top left"), ("2", "Bottom right"), ("3", "Bottom left")]),
        true,
    ));
    per_app(page, &s);
    widgets::keywords(
        "window controls close button hover delay popover headerbar titlebar decorations terminal chromeless corner double click maximize float tile geometry per app",
    );
}

/// The per-app rules, saved whole (app ids hold dots, which a settings key can't).
fn save(apps: serde_json::Map<String, Value>) {
    report(wc::set_apps(apps));
    super::workspaces::refresh_sub();
}

fn per_app(page: &Page, s: &Value) {
    let g = page.group("Window controls per app");
    g.note("How an app's window controls show, and whether it opens floating or tiled. Automatic: Nyx apps keep their own header, terminals get the small control while they float, other apps keep what they draw.");
    let apps = wc::apps(s);
    let installed = super::own_titlebars::installed_apps();
    let name_of = |key: &str| {
        installed
            .iter()
            .find(|(id, _)| id.eq_ignore_ascii_case(key) || wc::app_key(id).eq_ignore_ascii_case(key))
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| key.to_string())
    };
    for (key, rule) in &apps {
        let content = widgets::hbox(6);
        let controls = rule.get("controls").and_then(Value::as_str).unwrap_or("auto");
        let shown = widgets::dropdown(&options(PRESENTATIONS), controls);
        shown.set_tooltip_text(Some("How its window controls show"));
        let float = match rule.get("float").and_then(Value::as_bool) {
            Some(true) => "float",
            Some(false) => "tile",
            None => "auto",
        };
        let open = widgets::dropdown(&options(OPEN_AS), float);
        open.set_tooltip_text(Some("How it opens"));
        let del = gtk::Button::from_icon_name("user-trash-symbolic");
        del.add_css_class("flat");
        del.set_tooltip_text(Some("Back to automatic"));
        del.set_valign(gtk::Align::Center);

        let (k, a) = (key.clone(), apps.clone());
        shown.connect_selected_notify(move |d| {
            let Some((id, _)) = PRESENTATIONS.get(d.selected() as usize) else { return };
            let mut a = a.clone();
            let entry = a.entry(k.clone()).or_insert_with(|| json!({}));
            if *id == "auto" {
                entry.as_object_mut().map(|o| o.remove("controls"));
            } else {
                entry["controls"] = json!(id);
            }
            save(a);
        });
        let (k, a) = (key.clone(), apps.clone());
        open.connect_selected_notify(move |d| {
            let Some((id, _)) = OPEN_AS.get(d.selected() as usize) else { return };
            let mut a = a.clone();
            let entry = a.entry(k.clone()).or_insert_with(|| json!({}));
            match *id {
                "float" => entry["float"] = json!(true),
                "tile" => entry["float"] = json!(false),
                _ => {
                    entry.as_object_mut().map(|o| o.remove("float"));
                }
            }
            save(a);
        });
        let (k, a) = (key.clone(), apps.clone());
        del.connect_clicked(move |_| {
            let mut a = a.clone();
            a.remove(&k);
            save(a);
        });
        content.append(&shown);
        content.append(&open);
        content.append(&del);
        g.add(&widgets::row(&name_of(key), key, Some(content.upcast_ref())));
    }

    let choices: Vec<(String, String)> = installed
        .into_iter()
        .filter(|(id, _)| wc::valid_app_id(&wc::app_key(id)) && !apps.keys().any(|k| k.eq_ignore_ascii_case(&wc::app_key(id))))
        .collect();
    if let Some(first) = choices.first().map(|c| c.0.clone()) {
        let content = widgets::hbox(6);
        let dd = widgets::dropdown(&choices, &first);
        let add = gtk::Button::from_icon_name("list-add-symbolic");
        add.add_css_class("flat");
        add.set_tooltip_text(Some("Give this app a Nyx headerbar"));
        add.set_valign(gtk::Align::Center);
        let (d2, a) = (dd.clone(), apps.clone());
        add.connect_clicked(move |_| {
            let Some((id, _)) = choices.get(d2.selected() as usize) else { return };
            let mut a = a.clone();
            a.insert(wc::app_key(id), json!({ "controls": "custom-headerbar" }));
            save(a);
        });
        content.append(&dd);
        content.append(&add);
        g.add(&widgets::row("Add an app", "It starts with a Nyx headerbar; change it above.", Some(content.upcast_ref())));
    }
}
