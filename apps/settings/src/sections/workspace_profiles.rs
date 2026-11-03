//! Workspace profiles: named sets of workspaces ("Work", "Home"), each with how many
//! workspaces always exist, what each one is like (set under Tiling & Workspaces while
//! the profile is active) and which apps open on which workspace. Switching applies one
//! and moves its apps' open windows (`backend::profiles`).

use crate::backend::profiles::{self, ensure_default, placement_rules, store as profile_store};
use crate::backend::state::Bind;
use crate::backend::store;
use crate::dialog::{Field, ask};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use gtk::gio;
use nyx_desktop::profiles::{App, Profile};
use std::cell::RefCell;
use std::rc::Rc;

const NEXT_KEY: &str = "Next workspace profile";

/// (desktop id, name) of every app the launcher shows, by name.
fn installed_apps() -> Vec<(String, String)> {
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

fn app_name(id: &str) -> String {
    if let Some(class) = id.strip_prefix("class:") {
        return class.to_string();
    }
    let file = format!("{id}.desktop");
    gio::AppInfo::all()
        .into_iter()
        .find(|a| a.id().is_some_and(|i| i == file.as_str()))
        .map(|a| a.name().to_string())
        .unwrap_or_else(|| id.to_string())
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("flat");
    b.set_tooltip_text(Some(tooltip));
    b.set_valign(gtk::Align::Center);
    b
}

fn workspace_options() -> Vec<(String, String)> {
    (1..=10).map(|n| (n.to_string(), format!("Workspace {n}"))).collect()
}

/// Make sure today's setup is a profile, and return the active one.
fn active_profile() -> Option<Profile> {
    let state = store::read(|s| s.clone());
    let id = ensure_default(&state).map_err(|e| window::toast(&format!("{e}"))).ok()?;
    profile_store().load(&id).ok()
}

/// Save the active profile's apps and put its rules in place (without moving windows).
fn save_apps(p: &Profile) {
    if let Err(e) = profile_store().save(p) {
        window::toast(&format!("Couldn't save: {e}"));
        return;
    }
    let rules = placement_rules(p, &nyx_desktop::apps::from_gio());
    store::update(true, move |s| s.profile_rules = rules);
}

fn switch_to(id: String) {
    store::flush();
    cmd::background(
        move || profiles::switch(&id).map_err(|e| format!("{e:#}")),
        |r| {
            match r {
                Ok(msg) => window::toast(&msg),
                Err(e) => window::toast(&e),
            }
            store::reload_workspaces();
            window::rebuild("workspace-profiles");
            window::rebuild_if_built("workspaces");
        },
    );
}

pub fn build(page: &Page) {
    let pstore = profile_store();
    let list = pstore.list();
    let active = pstore.active();

    // ----- Profiles -----
    let g = page.group("Profiles");
    if list.is_empty() {
        g.note("Your workspaces as they are now become the Default profile when you add another one.");
    }
    for p in &list {
        let is_active = active.as_deref() == Some(p.id.as_str());
        let content = widgets::hbox(4);
        if !is_active {
            let go = icon_button("object-select-symbolic", "Switch to this profile");
            let id = p.id.clone();
            go.connect_clicked(move |_| switch_to(id.clone()));
            content.append(&go);
        }
        let rename = icon_button("document-edit-symbolic", "Rename");
        let id = p.id.clone();
        let name = p.name.clone();
        rename.connect_clicked(move |_| {
            let id = id.clone();
            ask("Rename profile", "", vec![Field::text_with("Name", &name)], "Rename", None, move |v, _| {
                let name = v.first().map(|s| s.trim().to_string()).unwrap_or_default();
                if name.is_empty() {
                    return Some("A profile needs a name".into());
                }
                let store = profile_store();
                let Ok(mut p) = store.load(&id) else { return Some("That profile is gone".into()) };
                p.name = name;
                if let Err(e) = store.save(&p) {
                    return Some(e);
                }
                window::rebuild("workspace-profiles");
                None
            });
        });
        content.append(&rename);
        let copy = icon_button("edit-copy-symbolic", "Duplicate");
        let source = p.clone();
        copy.connect_clicked(move |_| {
            let store = profile_store();
            let mut dup = source.clone();
            dup.name = format!("{} copy", source.name);
            dup.id = store.new_id(&dup.name);
            match store.save(&dup) {
                Ok(()) => window::rebuild("workspace-profiles"),
                Err(e) => window::toast(&format!("Couldn't save: {e}")),
            }
        });
        content.append(&copy);
        if !is_active {
            let del = icon_button("user-trash-symbolic", "Delete");
            let removed = p.clone();
            del.connect_clicked(move |_| {
                let store = profile_store();
                if let Err(e) = store.delete(&removed.id) {
                    window::toast(&format!("Couldn't delete: {e}"));
                    return;
                }
                window::rebuild("workspace-profiles");
                let back = removed.clone();
                window::toast_action(&format!("{} deleted", removed.name), "Undo", move || {
                    let _ = profile_store().save(&back);
                    window::rebuild("workspace-profiles");
                });
            });
            content.append(&del);
        }
        let ws = match p.workspaces {
            0 => "Workspaces as Hyprland makes them".to_string(),
            1 => "1 workspace".to_string(),
            n => format!("{n} workspaces"),
        };
        let apps = match p.apps.len() {
            0 => String::new(),
            1 => " · 1 app placed".to_string(),
            n => format!(" · {n} apps placed"),
        };
        let desc = format!("{ws}{apps}");
        let row = widgets::row(&p.name, &desc, Some(content.upcast_ref()));
        if is_active {
            widgets::tag_row(&row, "Active");
        }
        g.add(&row);
    }
    let add = icon_button("list-add-symbolic", "New profile");
    add.connect_clicked(|_| {
        ask(
            "New workspace profile",
            "It starts as a copy of the active profile: change its workspaces under Tiling & Workspaces once you've switched to it.",
            vec![Field::text("Name, like Work")],
            "Add",
            None,
            |v, _| {
                let name = v.first().map(|s| s.trim().to_string()).unwrap_or_default();
                if name.is_empty() {
                    return Some("A profile needs a name".into());
                }
                let Some(base) = active_profile() else { return Some("Couldn't read the active profile".into()) };
                let store = profile_store();
                let mut p = base;
                p.id = store.new_id(&name);
                p.name = name;
                if let Err(e) = store.save(&p) {
                    return Some(e);
                }
                window::rebuild("workspace-profiles");
                None
            },
        );
    });
    g.add(&widgets::row("Add a profile", "", Some(add.upcast_ref())));
    widgets::keywords("workspace profiles sets work home switch duplicate rename");

    // ----- The active profile -----
    let Some(current) = active.as_ref().and_then(|id| pstore.load(id).ok()) else { return };
    let g = page.group(&format!("{}: apps", current.name));
    g.note("Each app opens on its workspace, and switching to this profile moves its open windows there.");
    let shared = Rc::new(RefCell::new(current.clone()));
    for (i, a) in current.apps.iter().enumerate() {
        let content = widgets::hbox(6);
        let dd = widgets::dropdown(&workspace_options(), &a.workspace.to_string());
        let sh = shared.clone();
        dd.connect_selected_notify(move |d| {
            let ws = d.selected() + 1;
            let mut p = sh.borrow_mut();
            if let Some(app) = p.apps.get_mut(i) {
                app.workspace = ws;
            }
            save_apps(&p);
        });
        content.append(&dd);
        let del = icon_button("user-trash-symbolic", "Remove");
        let sh = shared.clone();
        del.connect_clicked(move |_| {
            {
                let mut p = sh.borrow_mut();
                if i < p.apps.len() {
                    p.apps.remove(i);
                }
                save_apps(&p);
            }
            window::rebuild("workspace-profiles");
        });
        content.append(&del);
        g.add(&widgets::row(&app_name(&a.app), "", Some(content.upcast_ref())));
    }
    let apps = installed_apps();
    let placed: Vec<String> = current.apps.iter().map(|a| a.app.clone()).collect();
    let choices: Vec<(String, String)> = apps.into_iter().filter(|(id, _)| !placed.contains(id)).collect();
    if !choices.is_empty() {
        let content = widgets::hbox(6);
        let app_dd = widgets::dropdown(&choices, &choices[0].0);
        let ws_dd = widgets::dropdown(&workspace_options(), "1");
        let add = icon_button("list-add-symbolic", "Place this app");
        let sh = shared.clone();
        let (a2, w2) = (app_dd.clone(), ws_dd.clone());
        add.connect_clicked(move |_| {
            let Some((id, _)) = choices.get(a2.selected() as usize) else { return };
            {
                let mut p = sh.borrow_mut();
                p.apps.push(App { app: id.clone(), workspace: w2.selected() + 1 });
                save_apps(&p);
            }
            window::rebuild("workspace-profiles");
        });
        content.append(&app_dd);
        content.append(&ws_dd);
        content.append(&add);
        g.add(&widgets::row("Place an app", "", Some(content.upcast_ref())));
    }
    let sh = shared.clone();
    let (r, _) = widgets::switch_row(
        "Move open windows",
        "When switching to this profile, move its apps' open windows to their workspaces.",
        current.move_open_windows,
        move |on| {
            let mut p = sh.borrow_mut();
            p.move_open_windows = on;
            if let Err(e) = profile_store().save(&p) {
                window::toast(&format!("Couldn't save: {e}"));
            }
        },
    );
    g.add(&r);
    widgets::keywords("apps open on workspace place move windows");

    // ----- Its workspaces -----
    let g = page.group(&format!("{}: workspaces", current.name));
    let open = icon_button("go-next-symbolic", "Open Tiling & Workspaces");
    open.connect_clicked(|_| window::navigate("workspaces"));
    g.add(&widgets::row(
        "Names, screens, layouts and gaps",
        "Set under Tiling &amp; Workspaces; changes there belong to the active profile.",
        Some(open.upcast_ref()),
    ));
    copy_workspace(&g);
    widgets::keywords("copy workspace settings layout gaps name to other workspaces");

    // ----- Switching -----
    let g = page.group("Switching");
    let current_key = store::read(|s| s.binds.iter().find(|b| b.description == NEXT_KEY).map(|b| b.keys.clone())).unwrap_or_default();
    let (r, _) = widgets::entry_row(
        "Key for the next profile",
        "Keys as Hyprland writes them, like SUPER + CTRL + P. Empty: no key.",
        &current_key,
        "No key",
        |v| {
            let keys = v.trim().to_string();
            store::update(true, move |s| {
                s.binds.retain(|b| b.description != NEXT_KEY);
                if !keys.is_empty() {
                    s.binds.push(Bind {
                        keys: keys.clone(),
                        description: NEXT_KEY.into(),
                        command: "nyx-settings --workspace-profile next".into(),
                    });
                }
            });
        },
    );
    g.add(&r);
    g.note("Or run nyx-settings --workspace-profile NAME (or next) from anywhere.");
    widgets::keywords("key shortcut next profile command");
}

/// Copy one workspace's settings (layout, gaps, decorations; not its name or screen) to
/// others in the active profile.
fn copy_workspace(g: &widgets::Group) {
    let content = widgets::hbox(6);
    let from = widgets::dropdown(&workspace_options(), "1");
    let labels: Vec<String> = (1..=10).map(|n| n.to_string()).collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let targets: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(vec![false; 10]));
    let t2 = targets.clone();
    let chips = widgets::chip_toggles(&label_refs, &[false; 10], move |v| *t2.borrow_mut() = v);
    let go = icon_button("edit-copy-symbolic", "Copy to the chosen workspaces");
    let f2 = from.clone();
    go.connect_clicked(move |_| {
        let src = f2.selected() + 1;
        let to: Vec<u32> = targets.borrow().iter().enumerate().filter(|(_, on)| **on).map(|(i, _)| i as u32 + 1).filter(|n| *n != src).collect();
        if to.is_empty() {
            window::toast("Choose the workspaces to copy to");
            return;
        }
        let n = to.len();
        store::update(true, move |s| {
            let rule = s.workspaces.iter().find(|r| r.workspace == src).cloned().unwrap_or_default();
            for id in &to {
                let i = match s.workspaces.iter().position(|r| r.workspace == *id) {
                    Some(i) => i,
                    None => {
                        s.workspaces.push(crate::backend::state::WorkspaceRule { workspace: *id, ..Default::default() });
                        s.workspaces.len() - 1
                    }
                };
                let t = &mut s.workspaces[i];
                t.mode = rule.mode.clone();
                t.gaps = rule.gaps;
                t.no_border = rule.no_border;
                t.no_rounding = rule.no_rounding;
                t.no_shadow = rule.no_shadow;
            }
            s.workspaces.retain(|r| !r.is_empty());
            s.workspaces.sort_by_key(|r| r.workspace);
        });
        store::flush();
        window::rebuild_if_built("workspaces");
        window::toast(&format!("Copied workspace {src}'s layout to {n} workspace{}", if n == 1 { "" } else { "s" }));
    });
    content.append(&from);
    content.append(&chips);
    content.append(&go);
    g.add(&widgets::stacked_row(
        "Copy a workspace's settings",
        "Its layout, gaps and decorations, to the workspaces chosen here (names and screens stay).",
        content.upcast_ref(),
    ));
}
