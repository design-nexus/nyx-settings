//! Tiling & Workspaces: window controls, and apps that draw their own titlebar. On a
//! floating workspace Omarchy adds a titlebar to every window; these apps keep only
//! their own, and the Nyx Dock's hover buttons can stand in for it everywhere.

use crate::backend::{store, titlebars};
use crate::widgets::{self, Page};
use gtk::gio;
use gtk::prelude::*;

/// (desktop id, name) of every app the launcher shows, by name.
pub(super) fn installed_apps() -> Vec<(String, String)> {
    let mut apps: Vec<(String, String)> = gio::AppInfo::all()
        .into_iter()
        .filter(|a| a.should_show())
        .filter_map(|a| Some((a.id()?.trim_end_matches(".desktop").to_string(), a.name().to_string())))
        .collect();
    apps.sort_by(|a, b| a.0.cmp(&b.0));
    apps.dedup_by(|a, b| a.0 == b.0);
    apps.sort_by_key(|a| a.1.to_lowercase());
    apps
}

fn name_of(id: &str, apps: &[(String, String)]) -> String {
    if let Some(class) = id.strip_prefix("class:") {
        return class.to_string();
    }
    apps.iter().find(|(i, _)| i == id).map(|(_, n)| n.clone()).unwrap_or_else(|| id.to_string())
}

fn set_list(list: Vec<String>) {
    store::update(true, move |s| s.own_titlebar_apps = Some(list));
    store::flush();
    super::workspaces::refresh_sub();
}

fn set_hover(on: bool) {
    store::update(true, move |s| s.hover_window_controls = on);
    store::flush();
    super::desktop::set_dock("window_controls", toml::Value::Boolean(on));
}

pub fn build(page: &Page) {
    if super::desktop::dock_installed() {
        let g = page.group("Window controls");
        let hover = store::read(|s| s.hover_window_controls);
        let (r, _) = widgets::choice_row(
            "On floating windows",
            "Omarchy's titlebar puts a bar with minimize, maximize and close above every window. Hover buttons are three small dots at the window's top-right corner that open into the same buttons, with no extra bar; drag a window by its own header or with Super and the mouse. Nyx apps always show their own.",
            vec![("titlebar".into(), "Omarchy titlebar".into()), ("hover".into(), "Hover buttons".into())],
            if hover { "hover" } else { "titlebar" },
            |v| set_hover(v == "hover"),
        );
        g.add(&r);
    }

    let g = page.group("Apps with their own titlebar");
    g.note("On a floating workspace every window gets a titlebar. Apps listed here draw their own, so they don't get a second one. The Switcher's overview can add the app in front.");
    let list = store::read(titlebars::apps);
    let apps = installed_apps();
    for (i, id) in list.iter().enumerate() {
        let del = gtk::Button::from_icon_name("user-trash-symbolic");
        del.add_css_class("flat");
        del.set_tooltip_text(Some("Give it a floating titlebar again"));
        del.set_valign(gtk::Align::Center);
        let l = list.clone();
        del.connect_clicked(move |_| {
            let mut l = l.clone();
            if i < l.len() {
                l.remove(i);
            }
            set_list(l);
        });
        g.add(&widgets::row(&name_of(id, &apps), "", Some(del.upcast_ref())));
    }
    let choices: Vec<(String, String)> = apps.into_iter().filter(|(id, _)| !list.contains(id)).collect();
    if let Some(first) = choices.first().map(|c| c.0.clone()) {
        let content = widgets::hbox(6);
        let dd = widgets::dropdown(&choices, &first);
        let add = gtk::Button::from_icon_name("list-add-symbolic");
        add.add_css_class("flat");
        add.set_tooltip_text(Some("Add this app"));
        add.set_valign(gtk::Align::Center);
        let (d2, l) = (dd.clone(), list.clone());
        add.connect_clicked(move |_| {
            let Some((id, _)) = choices.get(d2.selected() as usize) else { return };
            let mut l = l.clone();
            l.push(id.clone());
            set_list(l);
        });
        content.append(&dd);
        content.append(&add);
        g.add(&widgets::row("Add an app", "", Some(content.upcast_ref())));
    }
    widgets::keywords(
        "titlebar title bar double two floating window controls decorations hyprbars minimize maximize close buttons hover",
    );
}
