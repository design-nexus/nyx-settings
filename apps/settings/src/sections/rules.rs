//! Programs started with the session, and rules for how windows and shell layers behave.
//!
//! Everything is per app: one list of the apps something is set for (they start at
//! sign-in, or a window rule matches them), each tagged with what's set; the picked
//! one shows all of it on the right. The shell's layer rules and the system's own
//! startup items are a level down.

use super::keyboard::{clear, hold_controls, tiled};
use crate::backend::autostart;
use crate::backend::rules::{self, Value};
use crate::backend::state::Rule;
use crate::backend::{hypr, store};
use crate::dialog::{Field, ask};
use crate::sketch;
use crate::widgets::{self, Page};
use crate::window;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    /// The app picked in the list, by its key, kept across a rebuild of the page.
    static PICK: RefCell<Option<String>> = const { RefCell::new(None) };
}

// ---------- Apps and what's set for each ----------

#[derive(Clone)]
enum Start {
    /// An autostart entry of the user's own.
    Xdg(autostart::Item),
    /// A command Settings starts, by the process it looks for.
    Command { process: String, command: String },
}

#[derive(Clone)]
struct AppEntry {
    key: String,
    name: String,
    icon: Option<gio::Icon>,
    /// An icon name or path from an autostart entry, when there's no app.
    icon_name: String,
    /// The installed app, when one is known.
    app: Option<gio::AppInfo>,
    /// Lower-case names its windows and process go by, to link a rule to it.
    words: Vec<String>,
    start: Option<Start>,
    /// Its window rules, as indices into the state's list.
    rules: Vec<usize>,
    /// What its rules match (or would match, for a new one).
    class: String,
    title: String,
}

/// A rule's class pattern as a plain class name, when it's one (`^chat$`, `(?i)^Chat$`).
fn pattern_word(p: &str) -> Option<String> {
    let s = p.trim().trim_start_matches("(?i)").trim_start_matches('^').trim_end_matches('$').replace("\\.", ".");
    (!s.is_empty() && !s.contains(['|', '(', ')', '[', ']', '*', '+', '?', '\\', '{'])).then(|| s.to_lowercase())
}

/// A class pattern that matches `class` exactly, ignoring case.
fn class_pattern(class: &str) -> String {
    format!("(?i)^{}$", class.replace('.', "\\."))
}

fn find_app(apps: &[gio::AppInfo], stem: &str) -> Option<gio::AppInfo> {
    let file = format!("{stem}.desktop");
    apps.iter().find(|a| a.id().is_some_and(|i| i.eq_ignore_ascii_case(&file))).cloned()
}

fn collect(window_rules: &[Rule]) -> Vec<AppEntry> {
    let index = nyx_desktop::apps::from_gio();
    let apps = gio::AppInfo::all();
    let mut out: Vec<AppEntry> = Vec::new();
    let entry = |key: String, name: String, app: Option<gio::AppInfo>, words: Vec<String>| AppEntry {
        key,
        icon: app.as_ref().and_then(|a| a.icon()),
        name,
        icon_name: String::new(),
        app,
        words,
        start: None,
        rules: Vec::new(),
        class: String::new(),
        title: String::new(),
    };
    for item in autostart::list().into_iter().filter(|i| !i.is_system) {
        let stem = item.id.trim_end_matches(".desktop").to_string();
        let app = find_app(&apps, &stem);
        let mut words = vec![stem.to_lowercase(), item.name.to_lowercase()];
        words.extend(index.classes_of(&stem));
        if let Some(p) = rules::process_of(&autostart::strip_field_codes(&item.exec)) {
            words.push(p.to_lowercase());
        }
        let mut e = entry(format!("xdg:{}", item.id), item.name.clone(), app, words);
        e.icon_name = item.icon.clone();
        e.start = Some(Start::Xdg(item));
        out.push(e);
    }
    // The night light schedule manages its own entry.
    let commands: Vec<(String, String)> =
        store::read(|s| s.autostart.iter().filter(|(k, _)| k.as_str() != "hyprsunset").map(|(k, v)| (k.clone(), v.clone())).collect());
    for (process, command) in commands {
        let lower = process.to_lowercase();
        if let Some(e) = out.iter_mut().find(|e| e.start.is_none() && e.words.contains(&lower)) {
            e.start = Some(Start::Command { process, command });
            continue;
        }
        let app = index.find(&lower).and_then(|id| find_app(&apps, id.trim_end_matches(".desktop")));
        let name = app.as_ref().map(|a| a.name().to_string()).unwrap_or_else(|| process.clone());
        let mut words = vec![lower.clone()];
        if let Some(a) = &app
            && let Some(id) = a.id()
        {
            words.extend(index.classes_of(id.trim_end_matches(".desktop")));
        }
        let mut e = entry(format!("cmd:{process}"), name, app, words);
        e.start = Some(Start::Command { process, command });
        out.push(e);
    }
    for (i, r) in window_rules.iter().enumerate() {
        let word = pattern_word(&r.class);
        // A rule with a title is for some of an app's windows (its picture-in-picture,
        // say): an entry of its own.
        let found = out.iter_mut().find(|e| {
            (e.class == r.class && e.title == r.title && !e.rules.is_empty())
                || (r.title.is_empty() && e.title.is_empty() && word.as_ref().is_some_and(|w| e.words.contains(w)))
        });
        if let Some(e) = found {
            if e.rules.is_empty() {
                e.class = r.class.clone();
            }
            e.rules.push(i);
            continue;
        }
        let app = word.as_deref().and_then(|w| index.find(w)).and_then(|id| find_app(&apps, id.trim_end_matches(".desktop")));
        let title_word = r.title.trim().trim_start_matches('^').trim_end_matches('$').to_string();
        let name = match (&app, title_word.is_empty()) {
            (_, false) => title_word.clone(),
            (Some(a), true) => a.name().to_string(),
            (None, true) => word.clone().unwrap_or_else(|| r.class.clone()),
        };
        let mut e = entry(format!("rule:{}|{}", r.class, r.title), name, app, word.into_iter().collect());
        e.class = r.class.clone();
        e.title = r.title.clone();
        e.rules.push(i);
        out.push(e);
    }
    for e in &mut out {
        if e.class.is_empty()
            && let Some(w) = e.words.iter().find(|w| index.find(w).is_some()).or(e.words.first())
        {
            e.class = class_pattern(w);
        }
    }
    out.sort_by_key(|e| e.name.to_lowercase());
    out
}

/// A rule's effect as a short tag: "Floating", "Workspace 4".
fn tag_of(r: &Rule) -> String {
    let v = r.value.trim();
    match r.effect.as_str() {
        "float" => "Floating".into(),
        "tile" => "Tiled".into(),
        "center" => "Centred".into(),
        "pin" => "Every workspace".into(),
        "fullscreen" => "Fullscreen".into(),
        "no_focus" => "Don't take focus".into(),
        "no_blur" => "No blur".into(),
        "no_shadow" => "No shadow".into(),
        "no_anim" => "No animations".into(),
        "opacity" => format!("Opacity {v}"),
        "workspace" => format!("Workspace {}", v.trim_end_matches(" silent")),
        "size" => format!("Size {v}"),
        "rounding" => format!("Rounding {v}"),
        other => other.to_string(),
    }
}

fn effect_label(effect: &str) -> String {
    rules::effects(false).iter().find(|e| e.key == effect).map(|e| e.label.to_string()).unwrap_or_else(|| effect.to_string())
}

fn effect_icon(effect: &str) -> &'static str {
    match effect {
        "workspace" => "view-grid-symbolic",
        "no_focus" => "view-reveal-symbolic",
        "float" | "center" | "size" => "window-restore-symbolic",
        "fullscreen" => "view-fullscreen-symbolic",
        "pin" => "view-pin-symbolic",
        "opacity" | "no_blur" => "display-brightness-symbolic",
        _ => "applications-system-symbolic",
    }
}

fn effect_desc(effect: &str) -> &'static str {
    match effect {
        "float" => "Its windows float above the others.",
        "tile" => "Even when it would float.",
        "center" => "In the middle of the screen.",
        "pin" => "It comes with you to every workspace.",
        "fullscreen" => "It fills the screen.",
        "no_focus" => "It opens without pulling you away.",
        "opacity" => "When it's active, then when it isn't.",
        _ => "",
    }
}

fn tags(e: &AppEntry, window_rules: &[Rule]) -> Vec<String> {
    let mut out = Vec::new();
    match &e.start {
        Some(Start::Xdg(i)) if !i.enabled => out.push("Start is off".to_string()),
        Some(_) => out.push("Starts at sign-in".to_string()),
        None => {}
    }
    out.extend(e.rules.iter().filter_map(|i| window_rules.get(*i)).map(tag_of));
    out
}

fn app_icon(e: &AppEntry, size: i32) -> gtk::Image {
    let img = match &e.icon {
        Some(i) => gtk::Image::from_gicon(i),
        None if e.icon_name.starts_with('/') && std::path::Path::new(&e.icon_name).exists() => gtk::Image::from_file(&e.icon_name),
        None if !e.icon_name.is_empty()
            && gtk::gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(&e.icon_name)) =>
        {
            gtk::Image::from_icon_name(&e.icon_name)
        }
        None => gtk::Image::from_icon_name(match e.start {
            Some(Start::Command { .. }) => "utilities-terminal-symbolic",
            _ => "application-x-executable-symbolic",
        }),
    };
    img.set_pixel_size(size);
    img.set_valign(gtk::Align::Center);
    img.add_css_class("launcher-icon");
    img
}

fn rebuild() {
    glib::idle_add_local_once(|| window::rebuild("rules"));
}

fn remove_rule(i: usize) {
    let removed = store::read(|s| s.window_rules.get(i).cloned());
    store::update(true, move |s| {
        if i < s.window_rules.len() {
            s.window_rules.remove(i);
        }
    });
    rebuild();
    if let Some(back) = removed {
        window::toast_action(&format!("{} removed", effect_label(&back.effect)), "Undo", move || {
            let back = back.clone();
            store::update(true, move |s| s.window_rules.insert(i.min(s.window_rules.len()), back));
            rebuild();
        });
    }
}

// ---------- The page ----------

pub fn build(page: &Page) {
    let wr: Vec<Rule> = store::read(|s| s.window_rules.clone());
    let entries = Rc::new(collect(&wr));
    let window_rules: Rc<Vec<Rule>> = Rc::new(wr);
    let picked = PICK
        .with(|p| p.borrow().clone())
        .and_then(|k| entries.iter().position(|e| e.key == k))
        .or(if entries.is_empty() { None } else { Some(0) });
    let current = Rc::new(Cell::new(picked));

    // ----- The picture: everything about the picked app -----
    let pv = page.split();
    let detail = widgets::vbox(14);
    detail.add_css_class("rl-detail");
    pv.add(&detail);
    let foot = pv.foot("emblem-ok-symbolic", "");
    let rows: Rc<RefCell<Vec<(gtk::Box, usize, String)>>> = Rc::default();
    let show: Rc<dyn Fn(Option<usize>)> = {
        let (entries, window_rules, detail, foot, rows, current) =
            (entries.clone(), window_rules.clone(), detail.clone(), foot.clone(), rows.clone(), current.clone());
        Rc::new(move |i: Option<usize>| {
            current.set(i);
            PICK.with(|p| *p.borrow_mut() = i.and_then(|i| entries.get(i)).map(|e| e.key.clone()));
            for (row, n, _) in rows.borrow().iter() {
                if Some(*n) == i {
                    row.add_css_class("rl-picked");
                } else {
                    row.remove_css_class("rl-picked");
                }
            }
            widgets::forget_rows(&detail);
            clear(&detail);
            widgets::begin_section("rules");
            match i.and_then(|i| entries.get(i)) {
                Some(e) => fill_detail(&detail, &foot, e, &window_rules),
                None => {
                    let head = widgets::vbox(6);
                    head.add_css_class("rl-empty");
                    head.append(&widgets::label("Nothing set for any app yet", "preview-title"));
                    let l = widgets::label(
                        "Start an app when you sign in, or give its windows a rule: where they open, whether they \
                         float, if they take focus. Everything set for an app shows here together.",
                        "rl-empty-line",
                    );
                    l.set_wrap(true);
                    l.set_xalign(0.0);
                    head.append(&l);
                    detail.append(&head);
                    let card = preview_card(&detail, "");
                    for (icon, title, desc, label, act) in [
                        ("system-run-symbolic", "Start an app when you sign in", "Unless it's already running.", "Pick an app", add_startup_app as fn()),
                        ("utilities-terminal-symbolic", "Start a command", "Like nm-applet or discord --start-minimized.", "Add", add_command as fn()),
                        ("window-restore-symbolic", "A rule for a window", "Pick an open window, then what it does.", "Add", (|| rule_sheet("", "")) as fn()),
                    ] {
                        let b = gtk::Button::with_label(label);
                        b.connect_clicked(move |_| act());
                        card.append(&widgets::icon_row(icon, title, desc, Some(b.upcast_ref())));
                    }
                    foot.set_text("Programs start when you sign in, unless they're already running.");
                }
            }
        })
    };

    // ----- The controls: the apps -----
    let g = page.plain_group("");
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search apps and rules"));
    g.add(&search);

    let g = page.group("Apps");
    g.wrapper.add_css_class("rl-apps");
    for (i, e) in entries.iter().enumerate() {
        let tags = tags(e, &window_rules);
        let row = widgets::hbox(12);
        row.add_css_class("settings-option");
        row.add_css_class("card-row");
        row.add_css_class("rl-row");
        let icon_box = widgets::hbox(0);
        icon_box.add_css_class("rl-icon");
        icon_box.append(&app_icon(e, 22));
        icon_box.set_valign(gtk::Align::Center);
        row.append(&icon_box);
        let text = widgets::vbox(4);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        let t = widgets::label(&e.name, "settings-option-title");
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&t);
        let tag_line = widgets::hbox(4);
        tag_line.add_css_class("rl-tags");
        for (n, tag) in tags.iter().enumerate() {
            if n == 3 {
                tag_line.append(&widgets::label(&format!("+{}", tags.len() - 3), "rl-tag"));
                break;
            }
            let l = widgets::label(tag, "rl-tag");
            if tag == "Start is off" {
                l.add_css_class("rl-tag-off");
            }
            tag_line.append(&l);
        }
        text.append(&super::keyboard::fit(&tag_line));
        row.append(&text);
        let chev = gtk::Image::from_icon_name("go-next-symbolic");
        chev.add_css_class("chevron");
        row.append(&chev);
        row.set_cursor_from_name(Some("pointer"));
        let click = gtk::GestureClick::new();
        let show2 = show.clone();
        click.connect_released(move |_, _, _, _| show2(Some(i)));
        row.add_controller(click);
        g.add(&row);
        let text = crate::search::normalise(&format!("{} {} {}", e.name, tags.join(" "), e.words.join(" ")));
        widgets::keywords(&format!("{} {}", e.name, tags.join(" ")));
        rows.borrow_mut().push((row, i, text));
    }
    widgets::keywords("autostart startup login session boot launch rule window match class title float opacity workspace");
    g.add(&add_menu());
    {
        let rows = rows.clone();
        let wrapper = g.wrapper.clone();
        search.connect_search_changed(move |f| {
            let terms = crate::search::terms(&f.text());
            for (row, _, text) in rows.borrow().iter() {
                row.set_visible(terms.is_empty() || crate::search::matches(text, &terms));
            }
            widgets::mark_first_rows(wrapper.upcast_ref());
        });
    }

    // ----- The shell and the system, a level down -----
    let g = page.group("Shell");
    let layer_count = store::read(|s| s.layer_rules.len());
    let (r, _) = widgets::nav_row(
        Some("view-dual-symbolic"),
        "Bar, menus and other surfaces",
        "Blur and dim rules for the shell",
        &match layer_count {
            0 => String::new(),
            1 => "1 rule".to_string(),
            n => format!("{n} rules"),
        },
        || {
            widgets::subpage("Bar, menus and other surfaces", 520, |p| layer_rules(p));
        },
    );
    g.add(&r);
    widgets::keywords("layer rules shell surfaces namespace blur dim bar menus xray");
    let system: Vec<autostart::Item> = autostart::list().into_iter().filter(|i| i.is_system).collect();
    if !system.is_empty() {
        let n = system.len();
        let (r, _) = widgets::nav_row(Some("emblem-system-symbolic"), "Started by the system", "", &n.to_string(), move || {
            let system = system.clone();
            widgets::subpage("Started by the system", 520, move |p| {
                let g = p.group("");
                g.note("Installed with your apps and system. You can turn them off here, but not remove them.");
                for item in &system {
                    xdg_row(&g, item);
                }
            });
        });
        g.add(&r);
        widgets::keywords("system autostart startup applets services xdg");
    }

    show(picked);
    hold_controls(page, 40);
}

/// Start an app or a command at sign-in, or add a window rule.
fn add_menu() -> gtk::MenuButton {
    let mb = gtk::MenuButton::new();
    mb.add_css_class("rl-add");
    mb.add_css_class("flat");
    let c = widgets::hbox(10);
    c.append(&gtk::Image::from_icon_name("list-add-symbolic"));
    c.append(&widgets::label("Add an app…", "rl-add-text"));
    mb.set_child(Some(&c));
    mb.set_always_show_arrow(false);
    let pop = gtk::Popover::new();
    let col = widgets::vbox(2);
    col.add_css_class("rl-menu");
    let item = |text: &str, act: Box<dyn Fn()>| {
        let b = gtk::Button::with_label(text);
        b.add_css_class("flat");
        if let Some(l) = b.child().and_downcast::<gtk::Label>() {
            l.set_xalign(0.0);
        }
        let pop = pop.clone();
        b.connect_clicked(move |_| {
            pop.popdown();
            act();
        });
        col.append(&b);
    };
    item("Start an app when you sign in…", Box::new(add_startup_app));
    item("Start a command when you sign in…", Box::new(add_command));
    item("A rule for a window…", Box::new(|| rule_sheet("", "")));
    pop.set_child(Some(&col));
    mb.set_popover(Some(&pop));
    mb
}

fn installed() -> Vec<(String, String)> {
    let mut apps: Vec<(String, String)> = gio::AppInfo::all()
        .into_iter()
        .filter(|a| a.should_show())
        .filter_map(|a| Some((a.id()?.to_string(), a.name().to_string())))
        .collect();
    apps.sort_by(|a, b| a.0.cmp(&b.0));
    apps.dedup_by(|a, b| a.0 == b.0);
    apps.sort_by_key(|a| a.1.to_lowercase());
    apps
}

fn start_app(id: &str) {
    if let Some(app) = gio::AppInfo::all().into_iter().find(|a| a.id().is_some_and(|i| i == id)) {
        match autostart::add_app(&app) {
            Ok(name) => {
                window::toast(&format!("{name} added to startup"));
                PICK.with(|p| *p.borrow_mut() = Some(format!("xdg:{id}")));
                rebuild();
            }
            Err(e) => window::toast(&e),
        }
    }
}

fn add_startup_app() {
    let apps = installed();
    let Some(first) = apps.first().map(|a| a.0.clone()) else {
        window::toast("No installed apps were found");
        return;
    };
    ask(
        "Start an app when you sign in",
        "It starts each time you sign in, unless it's already running.",
        vec![Field::choice(apps, &first)],
        "Add",
        None,
        |v, _| {
            if let Some(id) = v.first().filter(|id| !id.is_empty()) {
                start_app(id);
            }
            None
        },
    );
}

fn command_line(text: &str) -> Option<(String, String)> {
    let text = text.trim().to_string();
    let process = rules::process_of(&text)?;
    let command = if text.starts_with("uwsm-app") { text } else { format!("uwsm-app -- {text}") };
    Some((process, command))
}

fn add_command() {
    ask(
        "Start a command when you sign in",
        "A command, like nm-applet or discord --start-minimized. It isn't started again while it's running.",
        vec![Field::text("command")],
        "Add",
        None,
        |v, _| {
            let text = v.first().map(|s| s.trim().to_string()).unwrap_or_default();
            if text.is_empty() {
                return Some("Type the command".into());
            }
            let Some((process, command)) = command_line(&text) else {
                return Some("Couldn't tell which program that starts.".into());
            };
            PICK.with(|p| *p.borrow_mut() = Some(format!("cmd:{process}")));
            store::update(true, move |s| {
                s.autostart.insert(process, command);
            });
            rebuild();
            None
        },
    );
}

fn edit_command(process: String, command: String) {
    ask(
        "Edit startup program",
        "The command that starts it.",
        vec![Field::text_with("command", &command)],
        "Save",
        None,
        move |v, _| {
            let Some((new_process, new_command)) = command_line(&v[0]) else {
                return Some("Couldn't tell which program that starts.".into());
            };
            let old = process.clone();
            store::update(true, move |s| {
                s.autostart.remove(&old);
                s.autostart.insert(new_process, new_command);
            });
            rebuild();
            None
        },
    );
}

/// A card in the preview with a heading over it.
fn preview_card(detail: &gtk::Box, title: &str) -> gtk::Box {
    if !title.is_empty() {
        let l = widgets::label(title, "rl-h");
        l.set_xalign(0.0);
        detail.append(&l);
    }
    let card = widgets::vbox(0);
    card.add_css_class("settings-card");
    card.set_overflow(gtk::Overflow::Hidden);
    detail.append(&card);
    card
}

fn fill_detail(detail: &gtk::Box, foot: &gtk::Label, e: &AppEntry, window_rules: &[Rule]) {
    // Who it is, and what its rules match.
    let head = widgets::hbox(12);
    head.add_css_class("rl-head");
    let big = widgets::hbox(0);
    big.add_css_class("rl-icon");
    big.add_css_class("rl-icon-big");
    big.append(&app_icon(e, 30));
    head.append(&big);
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&widgets::label(&e.name, "preview-title"));
    let matches = match (e.class.is_empty(), e.title.is_empty()) {
        (false, true) => format!("Matches app <tt>{}</tt> · any title", glib::markup_escape_text(&e.class)),
        (false, false) => format!(
            "Matches app <tt>{}</tt> · title <tt>{}</tt>",
            glib::markup_escape_text(&e.class),
            glib::markup_escape_text(&e.title)
        ),
        (true, false) => format!("Matches any app titled <tt>{}</tt>", glib::markup_escape_text(&e.title)),
        (true, true) => String::new(),
    };
    let line = gtk::Label::new(None);
    widgets::set_markup_or_text(&line, &match &e.start {
        Some(Start::Command { command, .. }) if e.rules.is_empty() => {
            format!("Runs <tt>{}</tt>", glib::markup_escape_text(command))
        }
        _ => matches,
    });
    line.add_css_class("rl-empty-line");
    line.set_xalign(0.0);
    line.set_wrap(true);
    line.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    text.append(&line);
    head.append(&text);
    if !e.rules.is_empty() {
        let change = gtk::Button::with_label("Change match");
        change.set_valign(gtk::Align::Center);
        let (rules_of, class, title) = (e.rules.clone(), e.class.clone(), e.title.clone());
        change.connect_clicked(move |_| change_match(rules_of.clone(), &class, &title));
        head.append(&change);
    }
    detail.append(&head);

    // When you sign in.
    let card = preview_card(detail, "When you sign in");
    let start_title = format!("Start {}", e.name);
    match &e.start {
        Some(Start::Xdg(item)) => {
            let controls = widgets::hbox(6);
            let id = item.id.clone();
            let remove = widgets::confirm_button("Remove", "Remove?", move |_| {
                if let Err(err) = autostart::remove(&id, false) {
                    window::toast(&err);
                } else {
                    rebuild();
                }
            });
            controls.append(&remove);
            controls.append(&enabled_switch(item));
            let desc = if item.comment.is_empty() { "Unless it's already running.".to_string() } else { glib::markup_escape_text(&item.comment).to_string() };
            card.append(&widgets::icon_row("system-run-symbolic", &start_title, &desc, Some(controls.upcast_ref())));
        }
        Some(Start::Command { process, command }) => {
            let controls = widgets::hbox(6);
            let edit = gtk::Button::from_icon_name("document-edit-symbolic");
            edit.add_css_class("flat");
            edit.set_tooltip_text(Some("Edit the command"));
            let (p, c) = (process.clone(), command.clone());
            edit.connect_clicked(move |_| edit_command(p.clone(), c.clone()));
            controls.append(&edit);
            let key = process.clone();
            let remove = widgets::confirm_button("Remove", "Remove?", move |_| {
                let key = key.clone();
                store::update(true, move |s| {
                    s.autostart.remove(&key);
                });
                rebuild();
            });
            controls.append(&remove);
            card.append(&widgets::icon_row(
                "utilities-terminal-symbolic",
                &start_title,
                &format!("<tt>{}</tt>", glib::markup_escape_text(command)),
                Some(controls.upcast_ref()),
            ));
        }
        None => match e.app.as_ref().and_then(|a| a.id()) {
            Some(id) => {
                let add = gtk::Button::with_label("Start it");
                let id = id.to_string();
                add.connect_clicked(move |_| start_app(&id));
                card.append(&widgets::icon_row("system-run-symbolic", &start_title, "It doesn't start when you sign in.", Some(add.upcast_ref())));
            }
            None => {
                card.append(&widgets::icon_row("system-run-symbolic", &start_title, "Only apps and commands can start when you sign in.", None));
            }
        },
    }

    // How its windows open.
    let card = preview_card(detail, "How its windows open");
    for &i in &e.rules {
        let Some(r) = window_rules.get(i) else { continue };
        card.append(&rule_row(i, r));
    }
    if e.rules.is_empty() {
        card.append(&widgets::row("Wherever you are, as the layout puts it", "No rules for its windows yet.", None));
    }
    let add = widgets::hbox(10);
    add.add_css_class("settings-option");
    add.add_css_class("card-row");
    add.add_css_class("rl-add-row");
    add.append(&gtk::Image::from_icon_name("list-add-symbolic"));
    add.append(&widgets::label(&format!("Add a rule for {}…", e.name), "rl-add-text"));
    add.set_cursor_from_name(Some("pointer"));
    let click = gtk::GestureClick::new();
    let (class, title) = (e.class.clone(), e.title.clone());
    click.connect_released(move |_, _, _, _| rule_sheet(&class, &title));
    add.add_controller(click);
    card.append(&add);

    // Where it opens.
    let rules_here: Vec<&Rule> = e.rules.iter().filter_map(|i| window_rules.get(*i)).collect();
    let workspace = rules_here
        .iter()
        .find(|r| r.effect == "workspace")
        .and_then(|r| r.value.split_whitespace().next()?.parse::<u32>().ok());
    let pinned = rules_here.iter().any(|r| r.effect == "pin");
    let silent = rules_here.iter().any(|r| r.effect == "workspace" && r.value.contains("silent"));
    let no_focus = rules_here.iter().any(|r| r.effect == "no_focus");
    let here = hypr::json(&["activeworkspace"]).and_then(|v| v.get("id")?.as_i64()).filter(|n| *n > 0).map(|n| n as u32);
    if workspace.is_some() || pinned {
        let strip_card = widgets::vbox(8);
        strip_card.add_css_class("preview-card");
        strip_card.add_css_class("rl-where");
        strip_card.append(&widgets::label("Where it opens", "rl-h-small"));
        let count = workspace.unwrap_or(1).max(here.unwrap_or(1)).max(5).min(10);
        let name = e.name.clone();
        let strip = widgets::drawing(64, move |cr, p, sec, w, h| draw_strip(cr, p, sec, w, h, count, workspace, pinned, here, &name));
        strip_card.append(&strip);
        detail.append(&strip_card);
    }

    // A sentence for the foot.
    let starts = match &e.start {
        Some(Start::Xdg(i)) if !i.enabled => format!("{} doesn't start at sign-in (it's off)", e.name),
        Some(_) => format!("{} starts when you sign in", e.name),
        None => format!("{} starts when you open it", e.name),
    };
    let opens = match (workspace, pinned) {
        (_, true) => " and follows you to every workspace".to_string(),
        (Some(n), false) if no_focus || silent => match here {
            Some(h) if h != n => format!("; it opens quietly on {n} and you stay on {h}"),
            _ => format!(" and opens quietly on workspace {n}"),
        },
        (Some(n), false) => format!(" and opens on workspace {n}"),
        (None, false) if no_focus => " and opens without taking focus".to_string(),
        (None, false) => String::new(),
    };
    foot.set_text(&format!("{starts}{opens}."));
}

/// The workspaces in a row, the app on the one its rule names (or on every one), and
/// the one you're on marked.
#[allow(clippy::too_many_arguments)]
fn draw_strip(cr: &gtk::cairo::Context, p: &sketch::Pal, sec: sketch::Rgb, w: f64, h: f64, count: u32, target: Option<u32>, pinned: bool, here: Option<u32>, name: &str) {
    let gap = 8.0;
    let tw = (w - gap * (count as f64 - 1.0)) / count as f64;
    for n in 1..=count {
        let x = (n - 1) as f64 * (tw + gap);
        let on = pinned || target == Some(n);
        let g = gtk::cairo::LinearGradient::new(x, 0.0, x + tw, h);
        let a = sketch::mix(p.bg, p.accent, 0.18 + 0.05 * (n % 3) as f64);
        let b = sketch::mix(p.bg, sec, 0.12);
        g.add_color_stop_rgb(0.0, a.0, a.1, a.2);
        g.add_color_stop_rgb(1.0, b.0, b.1, b.2);
        sketch::rounded(cr, x + 1.0, 1.0, tw - 2.0, h - 2.0, 8.0);
        let _ = cr.set_source(&g);
        let _ = cr.fill_preserve();
        if on {
            sketch::set(cr, sec, 1.0);
            cr.set_line_width(2.0);
        } else {
            sketch::set(cr, p.text, 0.08);
            cr.set_line_width(1.0);
        }
        let _ = cr.stroke();
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
        cr.set_font_size(11.0);
        sketch::set(cr, (0.0, 0.0, 0.0), 0.35);
        sketch::rounded(cr, x + 7.0, 7.0, 18.0, 17.0, 5.0);
        let _ = cr.fill();
        sketch::set(cr, (1.0, 1.0, 1.0), 0.95);
        let num = n.to_string();
        if let Ok(ext) = cr.text_extents(&num) {
            cr.move_to(x + 16.0 - ext.width() / 2.0 - ext.x_bearing(), 15.5 - ext.height() / 2.0 - ext.y_bearing());
            let _ = cr.show_text(&num);
        }
        let pill = |cr: &gtk::cairo::Context, text: &str, fill: sketch::Rgb, alpha: f64| {
            cr.set_font_size(10.5);
            let Ok(ext) = cr.text_extents(text) else { return };
            let pw = (ext.x_advance() + 14.0).min(tw - 10.0);
            let px = x + tw - pw - 6.0;
            let py = h - 24.0;
            sketch::set(cr, fill, alpha);
            sketch::rounded(cr, px, py, pw, 18.0, 6.0);
            let _ = cr.fill();
            cr.save().ok();
            cr.rectangle(px, py, pw, 18.0);
            cr.clip();
            sketch::set(cr, (1.0, 1.0, 1.0), 0.95);
            cr.move_to(px + 7.0 - ext.x_bearing(), py + 9.0 - ext.height() / 2.0 - ext.y_bearing());
            let _ = cr.show_text(text);
            cr.restore().ok();
        };
        if on {
            pill(cr, name, sketch::mix((0.0, 0.0, 0.0), sec, 0.55), 0.9);
        } else if here == Some(n) {
            pill(cr, "You", (0.0, 0.0, 0.0), 0.45);
        }
    }
}

fn enabled_switch(item: &autostart::Item) -> gtk::Switch {
    let sw = gtk::Switch::new();
    sw.set_active(item.enabled);
    sw.set_valign(gtk::Align::Center);
    let id = item.id.clone();
    let is_system = item.is_system;
    let reverting = Cell::new(false);
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
    sw
}

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

/// A system autostart entry with its switch.
fn xdg_row(g: &widgets::Group, item: &autostart::Item) {
    let desc = if !item.comment.is_empty() {
        glib::markup_escape_text(&item.comment).to_string()
    } else if !item.exec.is_empty() {
        format!("<tt>{}</tt>", glib::markup_escape_text(&item.exec))
    } else {
        String::new()
    };
    let r = widgets::row(&item.name, &desc, Some(enabled_switch(item).upcast_ref()));
    r.prepend(&launcher_icon(&item.icon, "application-x-executable-symbolic"));
    g.add(&r);
    widgets::keywords("autostart startup login session boot launch");
}

/// One of an app's rules: a switch for an on/off effect (off removes it), the value
/// with a way to change it otherwise.
fn rule_row(i: usize, r: &Rule) -> gtk::Box {
    let effect = rules::effects(false).iter().find(|e| e.key == r.effect);
    let label = effect_label(&r.effect);
    let icon = effect_icon(&r.effect);
    let desc = effect_desc(&r.effect);
    if effect.is_some_and(|e| e.value == Value::Flag) {
        let sw = gtk::Switch::new();
        sw.set_active(true);
        sw.set_tooltip_text(Some("Off removes the rule"));
        sw.connect_active_notify(move |s| {
            if !s.is_active() {
                remove_rule(i);
            }
        });
        return tiled(widgets::row(&label, desc, Some(sw.upcast_ref())), icon);
    }
    let controls = widgets::hbox(6);
    let mut parts = r.value.split_whitespace();
    let number = parts.next().and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=10).contains(n));
    let rest: Vec<&str> = parts.collect();
    if r.effect == "workspace"
        && let Some(n) = number
    {
        let options: Vec<(String, String)> = (1..=10).map(|n| (n.to_string(), format!("Workspace {n}"))).collect();
        let dd = widgets::dropdown(&options, &n.to_string());
        let suffix = rest.join(" ");
        dd.connect_selected_notify(move |d| {
            let n = d.selected() + 1;
            let value = if suffix.is_empty() { n.to_string() } else { format!("{n} {suffix}") };
            store::update(true, move |s| {
                if let Some(r) = s.window_rules.get_mut(i) {
                    r.value = value;
                }
            });
            rebuild();
        });
        controls.append(&dd);
    } else {
        let v = widgets::label(&r.value, "row-value");
        v.add_css_class("mono");
        controls.append(&v);
    }
    let edit = gtk::Button::from_icon_name("document-edit-symbolic");
    edit.add_css_class("flat");
    edit.set_tooltip_text(Some("Edit the rule"));
    let rule = r.clone();
    edit.connect_clicked(move |_| edit_rule(false, i, &rule));
    controls.append(&edit);
    let remove = gtk::Button::from_icon_name("window-close-symbolic");
    remove.add_css_class("flat");
    remove.set_tooltip_text(Some("Remove the rule"));
    remove.connect_clicked(move |_| remove_rule(i));
    controls.append(&remove);
    let desc = if desc.is_empty() && r.effect == "workspace" && r.value.contains("silent") { "Without taking you there." } else { desc };
    tiled(widgets::row(&label, desc, Some(controls.upcast_ref())), icon)
}

/// Change what an app's rules match, all of them at once.
fn change_match(indices: Vec<usize>, class: &str, title: &str) {
    ask(
        "Change match",
        "Which windows its rules are for. Both are patterns; leave the title empty for any title.",
        vec![Field::text_with("App class (pattern)", class), Field::text_with("Title (pattern, optional)", title)],
        "Save",
        None,
        move |v, _| {
            let class = v.first().map(|s| s.trim().to_string()).unwrap_or_default();
            let title = v.get(1).map(|s| s.trim().to_string()).unwrap_or_default();
            if class.is_empty() && title.is_empty() {
                return Some("Match an app class, a title, or both".into());
            }
            let probe = Rule { class: class.clone(), title: title.clone(), effect: "float".into(), value: String::new() };
            if rules::lua_line(false, &probe).is_none() {
                return Some("Patterns can't have quotes or backslashes.".into());
            }
            let indices = indices.clone();
            PICK.with(|p| *p.borrow_mut() = None);
            store::update(true, move |s| {
                for i in &indices {
                    if let Some(r) = s.window_rules.get_mut(*i) {
                        r.class = class.clone();
                        r.title = title.clone();
                    }
                }
            });
            rebuild();
            None
        },
    );
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
        rebuild();
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

/// What a new rule's picture shows.
#[derive(Default)]
struct Picture {
    float: bool,
    center: bool,
    fullscreen: bool,
    pin: bool,
    opacity: Option<f64>,
}

fn draw_picture(cr: &gtk::cairo::Context, p: &sketch::Pal, sec: sketch::Rgb, w: f64, h: f64, pic: &Picture) {
    let area = sketch::desktop(cr, p, sketch::R::new(1.0, 1.0, w - 2.0, h - 2.0));
    let alpha = pic.opacity.unwrap_or(1.0).clamp(0.2, 1.0);
    let r = if pic.fullscreen {
        area
    } else if pic.float {
        let (ww, wh) = (area.w * 0.42, area.h * 0.52);
        if pic.center {
            sketch::R::new(area.cx() - ww / 2.0, area.cy() - wh / 2.0, ww, wh)
        } else {
            sketch::R::new(area.x + area.w * 0.48, area.y + area.h * 0.08, ww, wh)
        }
    } else {
        // Tiled: it takes the top right place.
        let (_, right) = area.hsplit(0.52, (w * 0.01).clamp(3.0, 8.0));
        right.vsplit(0.48, (w * 0.01).clamp(3.0, 8.0)).0
    };
    sketch::rounded(cr, r.x, r.y, r.w, r.h, 6.0);
    sketch::set(cr, sketch::mix(p.surface, sec, 0.18), alpha);
    let _ = cr.fill_preserve();
    sketch::set(cr, sec, 1.0);
    cr.set_line_width(2.0);
    let _ = cr.stroke();
    let mut words = Vec::new();
    if pic.fullscreen {
        words.push("Fullscreen");
    } else if pic.float {
        words.push("Floating");
        if pic.center {
            words.push("centred");
        }
    } else {
        words.push("Tiled");
    }
    if pic.pin {
        words.push("on every workspace");
    }
    let text = words.join(", ");
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
    cr.set_font_size(11.0);
    if let Ok(ext) = cr.text_extents(&text) {
        let (pw, ph) = (ext.x_advance() + 16.0, 22.0);
        let px = (r.x + r.w - pw - 6.0).max(r.x + 4.0);
        let py = r.y + 6.0;
        sketch::set(cr, (0.0, 0.0, 0.0), 0.6);
        sketch::rounded(cr, px, py, pw, ph, 6.0);
        let _ = cr.fill();
        sketch::set(cr, (1.0, 1.0, 1.0), 0.95);
        cr.move_to(px + 8.0 - ext.x_bearing(), py + ph / 2.0 - ext.height() / 2.0 - ext.y_bearing());
        let _ = cr.show_text(&text);
    }
}

/// A new window rule: which windows (an open one, or a pattern), what it does as chips,
/// and a picture of how they'll open.
fn rule_sheet(class: &str, title: &str) {
    let (class, title) = (class.to_string(), title.to_string());
    let win: Rc<RefCell<Option<gtk::Window>>> = Rc::default();
    let w2 = win.clone();
    let w = widgets::subpage("New window rule", 820, move |p| {
        let pic = Rc::new(RefCell::new(Picture::default()));
        let pv = p.split();
        let area = {
            let pic = pic.clone();
            pv.drawing(220, move |cr, pal, sec, w, h| draw_picture(cr, pal, sec, w, h, &pic.borrow()))
        };
        area.add_css_class("rl-picture");
        let caption = widgets::label("How its windows will open", "rl-caption");
        caption.set_xalign(0.5);
        pv.add(&caption);

        let g = p.plain_group("Which windows");
        let class_e = gtk::Entry::new();
        class_e.set_placeholder_text(Some("App class, like ^mpv$"));
        class_e.add_css_class("mono");
        class_e.set_text(&class);
        let title_e = gtk::Entry::new();
        title_e.set_placeholder_text(Some("Any title"));
        title_e.set_text(&title);
        let index = nyx_desktop::apps::from_gio();
        let apps = gio::AppInfo::all();
        let open = open_classes();
        if !open.is_empty() {
            let picks = super::keyboard::flow();
            picks.set_column_spacing(6);
            for c in open.iter().take(8) {
                let b = gtk::Button::new();
                b.add_css_class("rl-pick");
                let bc = widgets::hbox(6);
                let app = index.find(c).and_then(|id| find_app(&apps, id.trim_end_matches(".desktop")));
                let img = match app.as_ref().and_then(|a| a.icon()) {
                    Some(i) => gtk::Image::from_gicon(&i),
                    None => gtk::Image::from_icon_name("application-x-executable-symbolic"),
                };
                img.set_pixel_size(18);
                bc.append(&img);
                bc.append(&gtk::Label::new(Some(&app.map(|a| a.name().to_string()).unwrap_or_else(|| c.clone()))));
                b.set_child(Some(&bc));
                b.set_tooltip_text(Some(c));
                let (class_e, c, caption) = (class_e.clone(), c.clone(), caption.clone());
                b.connect_clicked(move |b| {
                    class_e.set_text(&format!("^{}$", c.replace('.', "\\.")));
                    if let Some(l) = b.child().and_then(|c| c.last_child()).and_downcast::<gtk::Label>() {
                        caption.set_text(&format!("How {} will open", l.text()));
                    }
                });
                super::keyboard::flow_add(&picks, &b);
            }
            let note = widgets::label("Open now: pick one, or match by name below.", "group-note");
            note.set_xalign(0.0);
            g.add(&note);
            g.add(&picks);
        }
        let fields = widgets::vbox(0);
        fields.add_css_class("settings-card");
        fields.append(&widgets::row("App", "", Some(class_e.upcast_ref())));
        fields.append(&widgets::row("Title", "", Some(title_e.upcast_ref())));
        g.add(&fields);

        let g = p.plain_group("What it does");
        let chips = super::keyboard::flow();
        chips.set_column_spacing(6);
        let values = widgets::vbox(6);
        let mut toggles: Vec<(&'static rules::Effect, gtk::ToggleButton, Option<gtk::Entry>)> = Vec::new();
        for eff in rules::WINDOW {
            let b = gtk::ToggleButton::with_label(&if eff.value == Value::Flag { eff.label.to_string() } else { format!("{}…", eff.label) });
            b.add_css_class("rl-chip");
            let entry = (eff.value != Value::Flag).then(|| {
                let e = gtk::Entry::new();
                e.set_placeholder_text(Some(eff.hint));
                e.add_css_class("mono");
                let r = widgets::row(eff.label, "", Some(e.upcast_ref()));
                r.set_visible(false);
                values.append(&r);
                let r2 = r.clone();
                b.connect_toggled(move |b| r2.set_visible(b.is_active()));
                e
            });
            {
                let (pic, area) = (pic.clone(), area.clone());
                let key = eff.key;
                b.connect_toggled(move |b| {
                    let on = b.is_active();
                    let mut p = pic.borrow_mut();
                    match key {
                        "float" => p.float = on,
                        "center" => p.center = on,
                        "fullscreen" => p.fullscreen = on,
                        "pin" => p.pin = on,
                        _ => {}
                    }
                    drop(p);
                    area.queue_draw();
                });
            }
            if let Some(e) = &entry
                && eff.key == "opacity"
            {
                let (pic, area) = (pic.clone(), area.clone());
                e.connect_changed(move |e| {
                    pic.borrow_mut().opacity = e.text().split_whitespace().next().and_then(|v| v.parse().ok());
                    area.queue_draw();
                });
            }
            super::keyboard::flow_add(&chips, &b);
            toggles.push((eff, b, entry));
        }
        g.add(&chips);
        g.add(&values);

        let add = gtk::Button::with_label("Add rule");
        add.add_css_class("suggested-action");
        add.set_halign(gtk::Align::End);
        let w3 = w2.clone();
        add.connect_clicked(move |_| {
            let (class, title) = (class_e.text().trim().to_string(), title_e.text().trim().to_string());
            let mut new = Vec::new();
            for (eff, b, entry) in &toggles {
                if !b.is_active() {
                    continue;
                }
                let value = entry.as_ref().map(|e| e.text().trim().to_string()).unwrap_or_default();
                let r = Rule { class: class.clone(), title: title.clone(), effect: eff.key.to_string(), value };
                if rules::lua_line(false, &r).is_none() {
                    window::toast(&format!("{}: enter which windows and a value", eff.label));
                    return;
                }
                new.push(r);
            }
            if new.is_empty() {
                window::toast(if class.is_empty() && title.is_empty() { "Pick a window, or match an app or a title" } else { "Pick what the rule does" });
                return;
            }
            PICK.with(|p| *p.borrow_mut() = Some(format!("rule:{class}|{title}")));
            store::update(true, move |s| s.window_rules.extend(new));
            if let Some(w) = w3.borrow().as_ref() {
                w.close();
            }
            rebuild();
        });
        p.plain_group("").add(&add);
    });
    *win.borrow_mut() = Some(w);
}

/// The shell's layer rules, with a way to add one.
fn layer_rules(p: &Page) {
    let g = p.group("");
    g.note("Adjust the shell's own surfaces, such as the bar or menus, by their namespace.");
    let list: Vec<Rule> = store::read(|s| s.layer_rules.clone());
    for (i, rule) in list.iter().enumerate() {
        let (who, what) = rules::describe(true, rule);
        let edit = gtk::Button::from_icon_name("document-edit-symbolic");
        edit.add_css_class("flat");
        edit.set_tooltip_text(Some("Edit"));
        {
            let rule = rule.clone();
            edit.connect_clicked(move |_| edit_rule(true, i, &rule));
        }
        let remove = widgets::confirm_button("Remove", "Remove?", move |_| {
            store::update(true, move |s| {
                if i < s.layer_rules.len() {
                    s.layer_rules.remove(i);
                }
            });
            rebuild();
        });
        let controls = widgets::hbox(6);
        controls.append(&edit);
        controls.append(&remove);
        g.add(&widgets::row(&who, &glib::markup_escape_text(&what), Some(controls.upcast_ref())));
    }
    g.add(&widgets::form_disclosure("Add a layer rule…", "", &layer_form()));
}

fn layer_form() -> gtk::Box {
    let form = widgets::vbox(8);
    let class = gtk::Entry::new();
    class.set_placeholder_text(Some("Namespace, e.g. omarchy-bar"));
    form.append(&class);
    let effects = rules::effects(true);
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
            title: String::new(),
            effect: chosen.key.to_string(),
            value: if chosen.value == Value::Flag { String::new() } else { value.text().trim().to_string() },
        };
        if rules::lua_line(true, &rule).is_none() {
            window::toast("Enter a namespace and, if needed, a value");
            return;
        }
        store::update(true, move |s| s.layer_rules.push(rule));
        rebuild();
    });
    form.append(&add);
    form
}
