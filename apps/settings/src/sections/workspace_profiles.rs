//! Workspace profiles: named sets of workspaces ("Work", "Home"), each with how many
//! workspaces always exist, what each one is like (set under Tiling & Workspaces while
//! the profile is active) and which apps open on which workspace. Switching applies one
//! and moves its apps' open windows (`backend::profiles`).
//!
//! The profiles are a short list on the left; on the right the picked one is a board of
//! its workspaces with the apps that open on each, to drag between them.

use super::keyboard::{fit, hold_controls, more_card, more_page, tame_head, tiled};
use crate::backend::profiles::{self, ensure_default, placement_rules, store as profile_store};
use crate::backend::state::Bind;
use crate::backend::store;
use crate::dialog::{Field, ask};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use nyx_desktop::profiles::{App, Profile};
use std::cell::RefCell;
use std::rc::Rc;

const NEXT_KEY: &str = "Next workspace profile";

thread_local! {
    /// The profile shown on the board, kept across a rebuild of the page.
    static PICKED: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Apps taken off the board while the window is open, so they can go back on.
    static UNPLACED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

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

fn app_info(id: &str) -> Option<gio::AppInfo> {
    let file = format!("{id}.desktop");
    gio::AppInfo::all().into_iter().find(|a| a.id().is_some_and(|i| i == file.as_str()))
}

fn app_name(id: &str) -> String {
    if let Some(class) = id.strip_prefix("class:") {
        return class.to_string();
    }
    app_info(id).map(|a| a.name().to_string()).unwrap_or_else(|| id.to_string())
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

/// Save a profile's apps; the active one's rules are put in place too (without moving
/// windows).
fn save_apps(p: &Profile, active: bool) {
    if let Err(e) = profile_store().save(p) {
        window::toast(&format!("Couldn't save: {e}"));
        return;
    }
    if active {
        let rules = placement_rules(p, &nyx_desktop::apps::from_gio());
        store::update(true, move |s| s.profile_rules = rules);
    }
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

fn rebuild_soon() {
    glib::idle_add_local_once(|| window::rebuild("workspace-profiles"));
}

fn rename(id: String, name: String) {
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
}

fn duplicate(source: &Profile) {
    let store = profile_store();
    let mut dup = source.clone();
    dup.name = format!("{} copy", source.name);
    dup.id = store.new_id(&dup.name);
    match store.save(&dup) {
        Ok(()) => {
            PICKED.with(|p| *p.borrow_mut() = Some(dup.id.clone()));
            window::rebuild("workspace-profiles");
        }
        Err(e) => window::toast(&format!("Couldn't save: {e}")),
    }
}

fn delete(removed: Profile) {
    if let Err(e) = profile_store().delete(&removed.id) {
        window::toast(&format!("Couldn't delete: {e}"));
        return;
    }
    window::rebuild("workspace-profiles");
    window::toast_action(&format!("{} deleted", removed.name), "Undo", move || {
        let _ = profile_store().save(&removed);
        window::rebuild("workspace-profiles");
    });
}

fn new_profile() {
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
            PICKED.with(|k| *k.borrow_mut() = Some(p.id.clone()));
            window::rebuild("workspace-profiles");
            None
        },
    );
}

fn summary(p: &Profile) -> String {
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
    format!("{ws}{apps}")
}

/// A small button with an icon and a label.
fn labeled(icon: &str, text: &str) -> gtk::Button {
    let b = gtk::Button::new();
    let c = widgets::hbox(6);
    c.append(&gtk::Image::from_icon_name(icon));
    c.append(&gtk::Label::new(Some(text)));
    b.set_child(Some(&c));
    b.set_valign(gtk::Align::Center);
    b
}

pub fn build(page: &Page) {
    let pstore = profile_store();
    let list = pstore.list();
    let active = pstore.active();
    let picked_id = PICKED
        .with(|p| p.borrow().clone())
        .filter(|id| list.iter().any(|p| p.id == *id))
        .or_else(|| active.clone())
        .filter(|id| list.iter().any(|p| p.id == *id))
        .or_else(|| list.first().map(|p| p.id.clone()));
    let picked = picked_id.as_ref().and_then(|id| list.iter().find(|p| p.id == *id).cloned());
    let picked_active = picked.as_ref().is_some_and(|p| active.as_deref() == Some(p.id.as_str()));

    // ----- The picture: the picked profile's workspaces -----
    let pv = page.split();
    match &picked {
        Some(p) => board(&pv, p, picked_active),
        None => {
            // No profiles yet: the workspaces as they are, which become Default once an
            // app is placed or another profile is added.
            let mut now = Profile::new("", "Your workspaces");
            now.workspaces = store::read(|s| s.persistent_workspaces);
            board(&pv, &now, true);
        }
    }

    // ----- Profiles -----
    let g = page.group("Profiles");
    g.wrapper.add_css_class("wp-profiles");
    let mut first_radio: Option<gtk::CheckButton> = None;
    for p in &list {
        let is_active = active.as_deref() == Some(p.id.as_str());
        let radio = gtk::CheckButton::new();
        if let Some(f) = &first_radio {
            radio.set_group(Some(f));
        } else {
            first_radio = Some(radio.clone());
        }
        radio.set_active(picked_id.as_deref() == Some(p.id.as_str()));
        radio.set_valign(gtk::Align::Center);
        radio.set_tooltip_text(Some("Show it on the board"));
        let id = p.id.clone();
        radio.connect_toggled(move |r| {
            if r.is_active() {
                PICKED.with(|k| *k.borrow_mut() = Some(id.clone()));
                rebuild_soon();
            }
        });
        let end = widgets::hbox(6);
        if is_active {
            let pill = widgets::label("Active", "wp-active");
            pill.set_valign(gtk::Align::Center);
            end.append(&pill);
        } else {
            let go = gtk::Button::with_label("Switch");
            go.set_valign(gtk::Align::Center);
            go.set_tooltip_text(Some("Switch to this profile"));
            let id = p.id.clone();
            go.connect_clicked(move |_| switch_to(id.clone()));
            end.append(&go);
        }
        end.append(&profile_menu(p, is_active));
        let row = widgets::row(&p.name, &summary(p), Some(end.upcast_ref()));
        row.prepend(&widgets::tile(if is_active { "user-desktop-symbolic" } else { "view-grid-symbolic" }));
        row.prepend(&radio);
        row.add_css_class("with-tile");
        if picked_id.as_deref() == Some(p.id.as_str()) {
            row.add_css_class("wp-picked");
        }
        g.add(&row);
    }
    let add = widgets::hbox(10);
    add.add_css_class("settings-option");
    add.add_css_class("card-row");
    add.add_css_class("wp-add");
    let plus = gtk::Image::from_icon_name("list-add-symbolic");
    add.append(&plus);
    add.append(&widgets::label("New profile…", "wp-add-text"));
    add.set_cursor_from_name(Some("pointer"));
    let click = gtk::GestureClick::new();
    click.connect_released(|_, _, _, _| new_profile());
    add.add_controller(click);
    g.add(&add);
    widgets::keywords("workspace profiles sets work home switch duplicate rename new add delete");

    // ----- Switching -----
    let g = page.group("Switching");
    let current_key =
        store::read(|s| s.binds.iter().find(|b| b.description == NEXT_KEY).map(|b| b.keys.clone())).unwrap_or_default();
    let keys_box = widgets::hbox(4);
    keys_box.set_valign(gtk::Align::Center);
    if current_key.trim().is_empty() {
        keys_box.append(&widgets::label("No key", "row-value"));
    } else {
        for part in current_key.split('+').map(str::trim).filter(|s| !s.is_empty()) {
            let pretty = match part.to_uppercase().as_str() {
                "SUPER" => "Super".to_string(),
                "CTRL" | "CONTROL" => "Ctrl".to_string(),
                "ALT" => "Alt".to_string(),
                "SHIFT" => "Shift".to_string(),
                _ if part.chars().count() == 1 => part.to_uppercase(),
                _ => part.to_string(),
            };
            keys_box.append(&widgets::label(&pretty, "key-cap"));
        }
    }
    let edit = gtk::Button::from_icon_name("document-edit-symbolic");
    edit.add_css_class("flat");
    edit.set_tooltip_text(Some("Change the key"));
    edit.set_valign(gtk::Align::Center);
    {
        let current_key = current_key.clone();
        edit.connect_clicked(move |_| {
            ask(
                "Key for the next profile",
                "Keys as Hyprland writes them, like SUPER + CTRL + P. Leave it empty for no key. You can also run \
                 nyx-settings --workspace-profile NAME (or next) from anywhere.",
                vec![Field::text_with("SUPER + CTRL + P", &current_key)],
                "Save",
                None,
                |v, _| {
                    let keys = v.first().map(|s| s.trim().to_string()).unwrap_or_default();
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
                    rebuild_soon();
                    None
                },
            );
        });
    }
    let ctl = widgets::hbox(6);
    ctl.append(&keys_box);
    ctl.append(&edit);
    let order: Vec<String> = list.iter().map(|p| p.name.clone()).collect();
    let cycle = match order.len() {
        0 | 1 => "Goes to the next profile, and round again.".to_string(),
        _ => format!("Goes {}, and round again.", order.join(", ")),
    };
    g.add(&widgets::icon_row("input-keyboard-symbolic", "Key for the next profile", &cycle, Some(ctl.upcast_ref())));
    widgets::keywords("key shortcut next profile command");
    if let Some(p) = &picked {
        let sw = gtk::Switch::new();
        sw.set_active(p.move_open_windows);
        sw.set_valign(gtk::Align::Center);
        let id = p.id.clone();
        sw.connect_active_notify(move |sw| {
            let store = profile_store();
            let Ok(mut p) = store.load(&id) else { return };
            p.move_open_windows = sw.is_active();
            if let Err(e) = store.save(&p) {
                window::toast(&format!("Couldn't save: {e}"));
            }
        });
        g.add(&widgets::icon_row(
            "window-new-symbolic",
            "Move open windows",
            &format!("Switching to {} moves each placed app's windows to its workspace.", p.name),
            Some(sw.upcast_ref()),
        ));
        widgets::keywords("apps open on workspace place move windows");
    }

    // ----- Its workspaces, elsewhere -----
    let g = page.group("");
    g.wrapper.add_css_class("wp-more");
    let (r, _) = widgets::nav_row(
        Some("view-grid-symbolic"),
        "Names, screens and layouts",
        "On Tiling &amp; Workspaces, for the active profile",
        "",
        || window::navigate("workspaces"),
    );
    g.add(&r);
    let (copy_box, open_copy) = more_page("Copy a workspace's layout", 520);
    copy_workspace(&copy_box);
    let (r, _) = widgets::nav_row(
        Some("edit-copy-symbolic"),
        "Copy a workspace's layout",
        "Its layout and gaps, to other workspaces",
        "",
        move || open_copy(),
    );
    g.add(&r);
    widgets::keywords("copy workspace settings layout gaps name to other workspaces names screens monitors");
    hold_controls(page, 40);
}

/// Rename, duplicate or delete a profile.
fn profile_menu(p: &Profile, is_active: bool) -> gtk::MenuButton {
    let mb = gtk::MenuButton::new();
    mb.set_icon_name("view-more-symbolic");
    mb.add_css_class("flat");
    mb.set_tooltip_text(Some("Rename, duplicate or delete"));
    mb.set_valign(gtk::Align::Center);
    let pop = gtk::Popover::new();
    let col = widgets::vbox(2);
    col.add_css_class("wp-menu");
    let item = |text: &str| {
        let b = gtk::Button::with_label(text);
        b.add_css_class("flat");
        if let Some(l) = b.child().and_downcast::<gtk::Label>() {
            l.set_xalign(0.0);
        }
        col.append(&b);
        b
    };
    let (id, name) = (p.id.clone(), p.name.clone());
    let pop2 = pop.clone();
    item("Rename…").connect_clicked(move |_| {
        pop2.popdown();
        rename(id.clone(), name.clone());
    });
    let source = p.clone();
    let pop2 = pop.clone();
    item("Duplicate").connect_clicked(move |_| {
        pop2.popdown();
        duplicate(&source);
    });
    if !is_active {
        let removed = p.clone();
        let pop2 = pop.clone();
        let del = item("Delete");
        del.add_css_class("destructive-action");
        del.connect_clicked(move |_| {
            pop2.popdown();
            delete(removed.clone());
        });
    }
    pop.set_child(Some(&col));
    mb.set_popover(Some(&pop));
    mb
}

/// One workspace on the board: its number, name and screen.
struct Ws {
    id: u32,
    name: Option<String>,
    monitor: Option<String>,
}

/// The workspaces the board shows: the ones that always exist, then any an app or a
/// name is on.
fn board_workspaces(p: &Profile, active: bool) -> Vec<Ws> {
    let (count, named): (u32, Vec<Ws>) = if active {
        store::read(|s| {
            (
                s.persistent_workspaces,
                s.workspaces.iter().map(|r| Ws { id: r.workspace, name: r.name.clone(), monitor: r.monitor.clone() }).collect(),
            )
        })
    } else {
        (p.workspaces, p.workspace_list.iter().map(|w| Ws { id: w.id, name: w.name.clone(), monitor: w.monitor.clone() }).collect())
    };
    let highest = p
        .apps
        .iter()
        .map(|a| a.workspace)
        .chain(named.iter().filter(|w| w.name.is_some() || w.monitor.is_some()).map(|w| w.id))
        .max()
        .unwrap_or(0);
    // With no fixed number, one empty workspace past the last used one to drop on.
    let n = if count > 0 { count.max(highest) } else { (highest + 1).max(3) }.min(10);
    (1..=n)
        .map(|id| {
            let w = named.iter().find(|w| w.id == id);
            Ws { id, name: w.and_then(|w| w.name.clone()), monitor: w.and_then(|w| w.monitor.clone()) }
        })
        .collect()
}

/// A monitor as people know it: its connector, or the model from `desc:`.
fn monitor_label(m: &str) -> String {
    match m.strip_prefix("desc:") {
        Some(d) => d.split_whitespace().take(2).collect::<Vec<_>>().join(" "),
        None => m.to_string(),
    }
}

/// Move `app` to `workspace` in profile `id` (0 takes it off the board).
fn place(id: &str, active: bool, app: &str, workspace: u32) {
    let store = profile_store();
    // The board of workspaces before any profile exists: they become Default first.
    let id = match id {
        "" => match active_profile() {
            Some(p) => p.id,
            None => return,
        },
        id => id.to_string(),
    };
    let Ok(mut p) = store.load(&id) else { return };
    if workspace == 0 {
        p.apps.retain(|a| a.app != app);
        UNPLACED.with(|u| {
            let mut u = u.borrow_mut();
            u.retain(|a| a != app);
            u.insert(0, app.to_string());
        });
    } else {
        match p.apps.iter_mut().find(|a| a.app == app) {
            Some(a) => a.workspace = workspace,
            None => p.apps.push(App { app: app.to_string(), workspace }),
        }
        UNPLACED.with(|u| u.borrow_mut().retain(|a| a != app));
    }
    save_apps(&p, active);
    rebuild_soon();
}

/// An app on the board: its icon and name. Drag it to another workspace, or click it
/// for a list of where it can go.
fn app_chip(app: &str, profile: &Profile, active: bool, spaces: &[(u32, String)], placed: bool) -> gtk::MenuButton {
    let mb = gtk::MenuButton::new();
    mb.add_css_class("wp-chip");
    let c = widgets::hbox(6);
    let icon = match app_info(app).and_then(|a| a.icon()) {
        Some(i) => gtk::Image::from_gicon(&i),
        None => gtk::Image::from_icon_name("application-x-executable-symbolic"),
    };
    icon.set_pixel_size(16);
    icon.add_css_class("wp-chip-icon");
    c.append(&icon);
    let l = widgets::label(&app_name(app), "wp-chip-name");
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(14);
    c.append(&l);
    mb.set_child(Some(&c));
    mb.set_tooltip_text(Some("Drag to another workspace, or click to pick one"));

    let pop = gtk::Popover::new();
    let col = widgets::vbox(2);
    col.add_css_class("wp-menu");
    for (n, label) in spaces {
        let b = gtk::Button::with_label(label);
        b.add_css_class("flat");
        if let Some(l) = b.child().and_downcast::<gtk::Label>() {
            l.set_xalign(0.0);
        }
        let (id, app, n, pop) = (profile.id.clone(), app.to_string(), *n, pop.clone());
        b.connect_clicked(move |_| {
            pop.popdown();
            place(&id, active, &app, n);
        });
        col.append(&b);
    }
    if placed {
        let b = gtk::Button::with_label("Take off the board");
        b.add_css_class("flat");
        if let Some(l) = b.child().and_downcast::<gtk::Label>() {
            l.set_xalign(0.0);
        }
        let (id, app, pop) = (profile.id.clone(), app.to_string(), pop.clone());
        b.connect_clicked(move |_| {
            pop.popdown();
            place(&id, active, &app, 0);
        });
        col.append(&b);
    }
    pop.set_child(Some(&col));
    mb.set_popover(Some(&pop));

    let src = gtk::DragSource::new();
    src.set_actions(gdk::DragAction::MOVE);
    let id = app.to_string();
    src.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&id.to_value())));
    let chip = mb.clone();
    src.connect_drag_begin(move |s, _| {
        s.set_icon(Some(&gtk::WidgetPaintable::new(Some(&chip))), 12, 12);
    });
    mb.add_controller(src);
    mb
}

/// Let apps be dropped on `w`: onto workspace `n`, or off the board at 0.
fn drop_on(w: &impl IsA<gtk::Widget>, id: &str, active: bool, n: u32) {
    let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
    let id = id.to_string();
    target.connect_drop(move |_, value, _, _| {
        let Ok(app) = value.get::<String>() else { return false };
        place(&id, active, &app, n);
        true
    });
    let w2 = w.clone().upcast::<gtk::Widget>();
    target.connect_enter(move |_, _, _| {
        w2.add_css_class("wp-over");
        gdk::DragAction::MOVE
    });
    let w2 = w.clone().upcast::<gtk::Widget>();
    target.connect_leave(move |_| w2.remove_css_class("wp-over"));
    w.add_controller(target);
}

fn board(pv: &widgets::Preview, p: &Profile, active: bool) {
    let rename_b = labeled("document-edit-symbolic", "Rename");
    rename_b.add_css_class("flat");
    let (id, name) = (p.id.clone(), p.name.clone());
    rename_b.connect_clicked(move |_| rename(id.clone(), name.clone()));
    let dup = labeled("edit-copy-symbolic", "Duplicate");
    let source = p.clone();
    dup.connect_clicked(move |_| duplicate(&source));
    let end = widgets::hbox(6);
    let unsaved = p.id.is_empty();
    if !active {
        let go = gtk::Button::with_label(&format!("Switch to {}", p.name));
        go.add_css_class("suggested-action");
        let id = p.id.clone();
        go.connect_clicked(move |_| switch_to(id.clone()));
        end.append(&go);
    }
    if !unsaved {
        end.append(&rename_b);
        end.append(&dup);
    }
    let head = pv.head(
        &p.name,
        if unsaved {
            "As they are now. Place an app and they become your Default profile."
        } else {
            "Drag an app to another workspace, or off the board."
        },
        &[fit(&end).upcast_ref()],
    );
    tame_head(&head, 30);

    let spaces = board_workspaces(p, active);
    let labels: Vec<(u32, String)> = spaces
        .iter()
        .map(|w| (w.id, match &w.name {
            Some(n) => format!("Workspace {}: {n}", w.id),
            None => format!("Workspace {}", w.id),
        }))
        .collect();
    let grid = gtk::Grid::new();
    grid.set_column_homogeneous(true);
    grid.set_row_homogeneous(true);
    grid.set_column_spacing(10);
    grid.set_row_spacing(10);
    grid.add_css_class("wp-board");
    for (i, w) in spaces.iter().enumerate() {
        let apps: Vec<&App> = p.apps.iter().filter(|a| a.workspace == w.id).collect();
        let tile = widgets::vbox(8);
        tile.add_css_class("wp-ws");
        tile.add_css_class(&format!("wp-ws{}", (w.id - 1) % 6));
        let top = widgets::hbox(6);
        let num = widgets::label(&w.id.to_string(), "wp-num");
        num.set_xalign(0.5);
        top.append(&num);
        let title = widgets::label(w.name.as_deref().unwrap_or(if apps.is_empty() { "Empty" } else { "" }), "wp-ws-name");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        top.append(&title);
        if let Some(m) = &w.monitor {
            let screen = widgets::hbox(4);
            screen.add_css_class("wp-screen");
            let i = gtk::Image::from_icon_name("video-display-symbolic");
            i.set_pixel_size(12);
            screen.append(&i);
            let l = widgets::label(&monitor_label(m), "wp-screen-name");
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_max_width_chars(10);
            screen.append(&l);
            top.append(&screen);
        }
        tile.append(&top);
        let others: Vec<(u32, String)> = labels.iter().filter(|(n, _)| *n != w.id).cloned().collect();
        for a in &apps {
            let chip = app_chip(&a.app, p, active, &others, true);
            chip.set_halign(gtk::Align::Start);
            tile.append(&chip);
        }
        if apps.is_empty() {
            let hint = widgets::label("Drop an app here", "wp-drop-hint");
            hint.set_vexpand(true);
            hint.set_valign(gtk::Align::Center);
            hint.set_xalign(0.5);
            tile.append(&hint);
        }
        drop_on(&tile, &p.id, active, w.id);
        grid.attach(&tile, (i % 3) as i32, (i / 3) as i32, 1, 1);
    }
    pv.add(&grid);

    // The tray: apps taken off the board, and a way to place another.
    let tray = widgets::hbox(8);
    tray.add_css_class("preview-card");
    tray.add_css_class("wp-tray");
    tray.append(&widgets::label("Not placed", "wp-tray-title"));
    let chips = widgets::hbox(6);
    chips.set_hexpand(true);
    let placed: Vec<String> = p.apps.iter().map(|a| a.app.clone()).collect();
    let off: Vec<String> = UNPLACED.with(|u| u.borrow().iter().filter(|a| !placed.contains(a)).take(4).cloned().collect());
    if off.is_empty() {
        chips.append(&widgets::label("Drag an app here to take it off the board.", "wp-drop-hint"));
    }
    for a in &off {
        chips.append(&app_chip(a, p, active, &labels, false));
    }
    tray.append(&fit(&chips));
    let place_b = labeled("list-add-symbolic", "Place an app");
    {
        let (p, labels) = (p.clone(), labels.clone());
        place_b.connect_clicked(move |_| place_dialog(&p, active, &labels));
    }
    tray.append(&place_b);
    drop_on(&tray, &p.id, active, 0);
    pv.add(&tray);

    let while_active = if unsaved { String::new() } else { format!(" while {} is active", p.name) };
    let foot = match p.workspaces {
        0 => format!("Workspaces come and go as you use them{while_active}."),
        1 => format!("Workspace 1 always exists{while_active}."),
        n => format!("Workspaces 1 to {n} always exist{while_active}."),
    };
    pv.foot("dialog-information-symbolic", &foot);
}

/// Pick an app the launcher knows and the workspace it opens on.
fn place_dialog(p: &Profile, active: bool, spaces: &[(u32, String)]) {
    let placed: Vec<String> = p.apps.iter().map(|a| a.app.clone()).collect();
    let choices: Vec<(String, String)> = installed_apps().into_iter().filter(|(id, _)| !placed.contains(id)).collect();
    if choices.is_empty() {
        window::toast("Every app is placed already");
        return;
    }
    let mut ws: Vec<(String, String)> = spaces.iter().map(|(n, l)| (n.to_string(), l.clone())).collect();
    for n in 1..=10u32 {
        if !spaces.iter().any(|(s, _)| *s == n) {
            ws.push((n.to_string(), format!("Workspace {n}")));
        }
    }
    let (id, choices2) = (p.id.clone(), choices.clone());
    ask(
        "Place an app",
        &format!("Its windows open on this workspace while {} is active.", p.name),
        vec![Field::choice(choices, &choices2[0].0), Field::choice(ws, "1")],
        "Place",
        None,
        move |v, _| {
            let (Some(app), Some(n)) = (v.first(), v.get(1).and_then(|n| n.parse::<u32>().ok())) else {
                return Some("Pick an app and a workspace".into());
            };
            place(&id, active, app, n);
            None
        },
    );
}

/// Copy one workspace's settings (layout, gaps, decorations; not its name or screen) to
/// others in the active profile.
fn copy_workspace(content: &gtk::Box) {
    let card = more_card(content, "");
    let from = widgets::dropdown(&workspace_options(), "1");
    let labels: Vec<String> = (1..=10).map(|n| n.to_string()).collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let targets: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(vec![false; 10]));
    let t2 = targets.clone();
    let chips = widgets::chip_toggles(&label_refs, &[false; 10], move |v| *t2.borrow_mut() = v);
    let go = gtk::Button::with_label("Copy");
    go.add_css_class("suggested-action");
    go.set_halign(gtk::Align::End);
    let f2 = from.clone();
    go.connect_clicked(move |_| {
        let src = f2.selected() + 1;
        let to: Vec<u32> =
            targets.borrow().iter().enumerate().filter(|(_, on)| **on).map(|(i, _)| i as u32 + 1).filter(|n| *n != src).collect();
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
    let r = widgets::row("Copy from", "Its layout, gaps and decorations (names and screens stay).", Some(from.upcast_ref()));
    card.append(&tiled(r, "edit-copy-symbolic"));
    card.append(&tiled(widgets::stacked_row("To", "The workspaces to copy it to.", chips.upcast_ref()), "view-grid-symbolic"));
    go.add_css_class("wp-copy-go");
    content.append(&go);
}
