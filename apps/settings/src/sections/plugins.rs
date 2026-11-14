//! Bar Plugins: the plugins you added as a short list with switches, beside the picked
//! one's details (what it adds, who made it, where it came from, Update and Remove).
//! The ones Omarchy or NyxOS comes with are one row that opens their own list.

use crate::widgets::{self, Page};
use crate::{cmd, paths, window};
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

fn refresh_later() {
    glib::timeout_add_local_once(std::time::Duration::from_millis(800), || window::rebuild("plugins"));
}

fn nyxos() -> bool {
    crate::platform::PLATFORM == crate::platform::Platform::NyxOS
}

/// A plugin, from the plugin list and its own manifest.
#[derive(Clone)]
struct Plugin {
    id: String,
    name: String,
    kinds: Vec<String>,
    enabled: bool,
    can_disable: bool,
    /// Came with Omarchy or NyxOS (can't be removed).
    own: bool,
    dir: Option<PathBuf>,
    manifest: Value,
    /// The plugin it was copied from (Omarchy's `clonedFrom`).
    cloned_from: Option<String>,
}

impl Plugin {
    fn meta(&self, key: &str) -> Option<String> {
        self.manifest.get(key).and_then(Value::as_str).map(String::from).filter(|s| !s.trim().is_empty())
    }

    /// Where it came from: its git remote, else the manifest's repository or homepage.
    fn source(&self) -> Option<String> {
        let git = self.dir.as_ref().and_then(|d| std::fs::read_to_string(d.join(".git/config")).ok()).and_then(|t| {
            let mut in_origin = false;
            for line in t.lines().map(str::trim) {
                if line.starts_with('[') {
                    in_origin = line == "[remote \"origin\"]";
                } else if in_origin && let Some(url) = line.strip_prefix("url = ") {
                    return Some(url.trim().to_string());
                }
            }
            None
        });
        git.or_else(|| self.meta("repository")).or_else(|| self.meta("homepage")).map(|u| {
            u.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches(".git").to_string()
        })
    }

    fn kinds_text(&self) -> String {
        self.kinds.join(" · ")
    }

    fn icon(&self) -> &'static str {
        if let Some(super::barlayout::Glyph::Icon(i)) = super::barlayout::glyph(&self.id) {
            return i;
        }
        if self.kinds.iter().any(|k| k == "bar widget") {
            "view-grid-symbolic"
        } else if self.kinds.iter().any(|k| k == "panel" || k == "overlay") {
            "sidebar-show-symbolic"
        } else if self.kinds.iter().any(|k| k == "menu") {
            "open-menu-symbolic"
        } else {
            "system-run-symbolic"
        }
    }

    fn preview_picture(&self) -> Option<PathBuf> {
        let d = self.dir.as_ref()?;
        ["preview.png", "preview.jpg", "screenshot.png"].iter().map(|n| d.join(n)).find(|p| p.exists())
    }
}

fn kinds_of(p: &Value) -> Vec<String> {
    p.get("kinds")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|k| k.as_str().map(|s| s.replace('-', " "))).collect())
        .unwrap_or_default()
}

fn read_manifest(dir: &Option<PathBuf>) -> Value {
    dir.as_ref()
        .and_then(|d| std::fs::read_to_string(d.join("manifest.json")).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

/// Omarchy's shell plugins.
fn omarchy_plugins() -> Option<Vec<Plugin>> {
    let list: Vec<Value> = cmd::output(&["omarchy-plugin-list", "--json"])
        .and_then(|t| serde_json::from_str(&t).ok())
        .and_then(|v: Value| v.as_array().cloned())?;
    Some(
        list.iter()
            .map(|p| {
                let id = p.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                let dir = Some(paths::omarchy_config().join("plugins").join(&id)).filter(|d| d.is_dir());
                Plugin {
                    name: p.get("name").and_then(Value::as_str).unwrap_or(&id).to_string(),
                    kinds: kinds_of(p),
                    enabled: p.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                    can_disable: p.get("canDisable").and_then(Value::as_bool).unwrap_or(true),
                    own: p.get("firstParty").and_then(Value::as_bool) == Some(true),
                    cloned_from: p.get("clonedFrom").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from),
                    manifest: read_manifest(&dir),
                    dir,
                    id,
                }
            })
            .collect(),
    )
}

/// NyxOS: Nyx plugins through `nyx plugin` (the bar's now; the Control Center's and the
/// desktop's later). The ones NyxOS comes with can only be turned off.
fn nyx_plugins() -> Option<Vec<Plugin>> {
    let list: Vec<Value> = cmd::output(&["nyx", "plugin", "list", "--json"])
        .and_then(|t| serde_json::from_str(&t).ok())
        .and_then(|v: Value| v.as_array().cloned())?;
    Some(
        list.iter()
            .map(|p| {
                let id = p.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                let dir = p.get("path").and_then(Value::as_str).map(PathBuf::from).filter(|d| d.is_dir());
                Plugin {
                    name: p.get("name").and_then(Value::as_str).unwrap_or(&id).to_string(),
                    kinds: kinds_of(p),
                    enabled: p
                        .get("enabled")
                        .and_then(Value::as_object)
                        .is_some_and(|parts| parts.values().any(|v| v.as_array().is_some_and(|a| !a.is_empty()))),
                    can_disable: true,
                    own: p.get("source").and_then(Value::as_str) == Some("packaged"),
                    cloned_from: None,
                    manifest: if dir.is_some() { read_manifest(&dir) } else { p.clone() },
                    dir,
                    id,
                }
            })
            .collect(),
    )
}

fn set_enabled(id: &str, on: bool) {
    let args: Vec<String> = if nyxos() {
        vec!["nyx".into(), "plugin".into(), if on { "enable" } else { "disable" }.into(), id.into()]
    } else {
        vec![if on { "omarchy-plugin-enable" } else { "omarchy-plugin-disable" }.into(), id.into()]
    };
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    cmd::run_async(&refs, |r| {
        if let Err(e) = r {
            window::toast(&format!("{e}"));
        }
    });
}

fn plugin_switch(p: &Plugin) -> gtk::Switch {
    let sw = gtk::Switch::new();
    sw.set_active(p.enabled);
    sw.set_sensitive(p.can_disable);
    sw.set_valign(gtk::Align::Center);
    let id = p.id.clone();
    sw.connect_active_notify(move |s| set_enabled(&id, s.is_active()));
    sw
}

pub fn build(page: &Page) {
    let Some(list) = (if nyxos() { nyx_plugins() } else { omarchy_plugins() }).filter(|l| !l.is_empty()) else {
        page.banner("Couldn't list plugins.", true);
        return;
    };
    let mut added: Vec<Plugin> = list.iter().filter(|p| !p.own).cloned().collect();
    added.sort_by_key(|p| p.name.to_lowercase());
    let mut own: Vec<Plugin> = list.iter().filter(|p| p.own).cloned().collect();
    own.sort_by_key(|p| p.name.to_lowercase());

    // The picked plugin's details fill the preview.
    let pv = page.split();
    let detail = widgets::vbox(14);
    detail.add_css_class("pg-detail");
    pv.add(&detail);
    pv.foot("security-high-symbolic", "Plugins are added from git and run as you; only add ones you trust.");
    let show: Rc<dyn Fn(&Plugin)> = {
        let detail = detail.clone();
        Rc::new(move |p: &Plugin| fill_detail(&detail, p))
    };
    match added.first().or(own.first()) {
        Some(p) => show(p),
        None => detail.append(&widgets::label("No plugins yet.", "dim")),
    }

    // ----- Keep up to date -----
    let g = page.group("");
    let (r, _) = widgets::button_row("Update plugins", "Fetch the latest version of every plugin added from git.", "Update all", |b| {
        b.set_sensitive(false);
        let b = b.clone();
        let args: &[&str] = if nyxos() { &["nyx", "plugin", "update", "--yes"] } else { &["omarchy-plugin-update", "--yes"] };
        cmd::run_async(args, move |r| {
            b.set_sensitive(true);
            match r {
                Ok(_) => window::toast("Plugins updated"),
                Err(e) => window::toast(&format!("{e}")),
            }
            refresh_later();
        });
    });
    r.prepend(&widgets::tile("software-update-available-symbolic"));
    r.add_css_class("with-tile");
    r.add_css_class("pg-update");
    widgets::keywords("update upgrade git keep up to date");
    g.add(&r);

    // ----- Added by you -----
    let g = page.plain_group("");
    let head = widgets::hbox(6);
    head.add_css_class("pg-heading");
    let t = widgets::label(if nyxos() { "ADDED" } else { "ADDED BY YOU" }, "group-title");
    head.append(&t);
    head.append(&widgets::label(&format!("· {}", added.len()), "pg-count"));
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    head.append(&spacer);
    if !nyxos() {
        let find = widgets::labeled_button("edit-find-symbolic", "Find more");
        find.add_css_class("flat");
        find.set_valign(gtk::Align::Center);
        find.set_tooltip_text(Some("Browse the Omarchy plugin catalog"));
        find.connect_clicked(|_| cmd::spawn(&["xdg-open", "https://plugins.omarchy.org/"]));
        head.append(&find);
    }
    g.add(&head);
    let card = widgets::vbox(0);
    card.add_css_class("settings-card");
    card.set_overflow(gtk::Overflow::Hidden);
    let rows: Rc<RefCell<Vec<gtk::Box>>> = Rc::default();
    for (i, p) in added.iter().enumerate() {
        let r = widgets::icon_row(p.icon(), &p.name, &p.kinds_text(), Some(plugin_switch(p).upcast_ref()));
        r.add_css_class("pg-row");
        widgets::keywords(&format!("{} plugin {}", p.id, p.meta("description").unwrap_or_default()));
        if i == 0 {
            r.add_css_class("picked");
        }
        let click = gtk::GestureClick::new();
        {
            let (show, p, rows, r2) = (show.clone(), p.clone(), rows.clone(), r.clone());
            click.connect_released(move |_, _, _, _| {
                for row in rows.borrow().iter() {
                    row.remove_css_class("picked");
                }
                r2.add_css_class("picked");
                show(&p);
            });
        }
        r.add_controller(click);
        r.set_cursor_from_name(Some("pointer"));
        rows.borrow_mut().push(r.clone());
        card.append(&r);
    }
    if added.is_empty() {
        card.append(&widgets::row("None yet", if nyxos() { "Plugins you add appear here." } else { "Find more in the Omarchy plugin catalog." }, None));
    }
    g.add(&card);

    // ----- The ones that came with the system -----
    if !own.is_empty() {
        let g = page.group("");
        let on = own.iter().filter(|p| p.enabled).count();
        let title = if nyxos() { "Included with NyxOS" } else { "Omarchy's own" };
        let names: Vec<&str> = own.iter().take(3).map(|p| p.name.as_str()).collect();
        let desc = format!("{} and more", names.join(", "));
        let own2 = own.clone();
        let (r, _) = widgets::nav_row(Some("emblem-system-symbolic"), title, &desc, &format!("{on} of {} on", own.len()), move || {
            let own = own2.clone();
            let d = widgets::subpage(title, 600, move |p| {
                let g = p.group(title);
                g.note(if nyxos() { "These come with NyxOS; they can be turned off, not removed." } else { "These come with Omarchy; they can be turned off, not removed." });
                for plugin in &own {
                    g.add(&widgets::row(&plugin.name, &plugin.kinds_text(), Some(plugin_switch(plugin).upcast_ref())));
                }
            });
            widgets::forget_rows(&d);
        });
        widgets::keywords(&format!("first party built in system plugins {}", own.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(" ")));
        g.add(&r);
    }
}

/// Where a bar widget sits on Omarchy's bar.
fn bar_section(id: &str) -> Option<&'static str> {
    let layout = super::barlayout::read_layout();
    super::barlayout::SECTIONS.iter().zip(layout).find(|(_, ids)| ids.iter().any(|x| x == id)).map(|(s, _)| match *s {
        "left" => "In the bar's left section",
        "center" => "In the bar's center",
        _ => "In the bar's right section",
    })
}

fn about_kind(p: &Plugin, kind: &str) -> String {
    match kind {
        "bar widget" if nyxos() => "A part of the bar".into(),
        "bar widget" => bar_section(&p.id).map(String::from).unwrap_or_else(|| if p.enabled { "Not placed on the bar".into() } else { "Off".into() }),
        "panel" => "Opens from its widget".into(),
        "overlay" => "Shows over the screen".into(),
        "menu" => "Adds to a menu".into(),
        "service" => "Runs in the background".into(),
        "bar" => "A bar of its own".into(),
        _ => String::new(),
    }
}

fn fill_detail(detail: &gtk::Box, p: &Plugin) {
    while let Some(c) = detail.first_child() {
        detail.remove(&c);
    }
    // Who made it, then Update and Remove.
    let head = widgets::hbox(12);
    head.add_css_class("pg-head");
    let tile = widgets::tile(p.icon());
    tile.add_css_class("pg-big-tile");
    tile.set_valign(gtk::Align::Start);
    head.append(&tile);
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    let name = widgets::label(&p.name, "preview-title");
    name.set_wrap(true);
    text.append(&name);
    let mut by = Vec::new();
    if let Some(a) = p.meta("author") {
        by.push(format!("By {a}"));
    }
    if let Some(v) = p.meta("version") {
        by.push(format!("version {v}"));
    }
    if p.own {
        by.push(if nyxos() { "comes with NyxOS".into() } else { "comes with Omarchy".into() });
    } else if let Some(s) = p.source() {
        by.push(format!("from {}", s.split('/').next().unwrap_or(&s)));
    }
    if !by.is_empty() {
        let l = widgets::label(&by.join(" · "), "preview-line");
        l.set_wrap(true);
        text.append(&l);
    }
    let buttons = widgets::hbox(8);
    buttons.set_margin_top(6);
    if !p.own && (p.dir.as_ref().is_some_and(|d| d.join(".git").exists()) || nyxos()) {
        let update = widgets::labeled_button("software-update-available-symbolic", "Update");
        let id = p.id.clone();
        update.connect_clicked(move |b| {
            b.set_sensitive(false);
            let b = b.clone();
            let args: Vec<String> = if nyxos() {
                vec!["nyx".into(), "plugin".into(), "update".into(), id.clone(), "--yes".into()]
            } else {
                vec!["omarchy-plugin-update".into(), id.clone(), "--yes".into()]
            };
            cmd::background(
                move || {
                    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                    cmd::run(&refs)
                },
                move |r| {
                    b.set_sensitive(true);
                    match r {
                        Ok(out) => window::toast(out.lines().last().unwrap_or("Updated")),
                        Err(e) => window::toast(&format!("{e}")),
                    }
                    refresh_later();
                },
            );
        });
        buttons.append(&update);
    }
    if !p.own {
        let (id, n) = (p.id.clone(), p.name.clone());
        let remove = widgets::confirm_button("Remove", "Click again to remove", move |b| {
            b.set_sensitive(false);
            let n = n.clone();
            let args: Vec<String> = if nyxos() {
                vec!["nyx".into(), "plugin".into(), "remove".into(), id.clone(), "--yes".into()]
            } else {
                vec!["omarchy-plugin-remove".into(), id.clone(), "--yes".into()]
            };
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            cmd::run_async(&refs, move |r| {
                match r {
                    Ok(_) => window::toast(&format!("Removed {n}")),
                    Err(e) => window::toast(&format!("{e}")),
                }
                refresh_later();
            });
        });
        remove.add_css_class("destructive-action");
        buttons.append(&remove);
    }
    if buttons.first_child().is_some() {
        text.append(&buttons);
    }
    head.append(&text);
    detail.append(&head);

    if let Some(d) = p.meta("description") {
        let l = widgets::label(&d, "pg-desc");
        l.set_wrap(true);
        detail.append(&l);
    }

    // What it adds: each kind, where it shows; its own picture when it has one.
    if !p.kinds.is_empty() {
        detail.append(&widgets::label("What it adds", "pg-section"));
        let adds = super::barlayout::wrap_box(8);
        for kind in &p.kinds {
            let c = widgets::hbox(8);
            c.add_css_class("pg-add");
            c.append(&widgets::label(kind, "pg-kind"));
            let about = about_kind(p, kind);
            if !about.is_empty() {
                c.append(&widgets::label(&about, "pg-add-about"));
            }
            adds.append(&c);
        }
        detail.append(&adds);
    }
    if let Some(pic) = p.preview_picture() {
        let picture = gtk::Picture::for_filename(pic);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_size_request(-1, 200);
        picture.add_css_class("pg-picture");
        detail.append(&picture);
    }

    // The facts, two to a line.
    let facts = gtk::Grid::new();
    facts.add_css_class("pg-facts");
    facts.set_column_homogeneous(true);
    facts.set_column_spacing(16);
    facts.set_row_spacing(10);
    let n = std::cell::Cell::new(0);
    let fact = |k: &str, v: String| {
        let b = widgets::vbox(1);
        b.append(&widgets::label(k, "pg-fact-key"));
        let l = widgets::label(&v, "pg-fact-value");
        l.set_wrap(true);
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        l.set_selectable(true);
        l.set_can_focus(false);
        b.append(&l);
        let i = n.get();
        facts.attach(&b, i % 2, i / 2, 1, 1);
        n.set(i + 1);
    };
    fact("Plugin", p.id.clone());
    if let Some(s) = p.source() {
        fact("Source", s);
    }
    if let Some(l) = p.meta("license") {
        fact("Licence", l);
    }
    if let Some(from) = &p.cloned_from {
        fact("Made from", from.clone());
    }
    fact("State", if p.enabled { "On".into() } else { "Off".into() });
    detail.append(&facts);
}
