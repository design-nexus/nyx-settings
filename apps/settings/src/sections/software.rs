//! Apps and packages: your web and terminal apps, each shown as it looks in the app
//! launcher and as the window it opens; Omarchy's installers, a package search, the
//! update channel and the maintenance tools one level down. The Omarchy scripts ask
//! for sudo themselves, so the ones that need it run in a terminal.

use super::extensions::{preview_head, shrinkable};
use super::services::{mark_picked, pickable};
use crate::dialog::{Field, ask};
use crate::sections::accounts::terminal;
use crate::sketch::{self, Pal, R};
use crate::widgets::{self, Page};
use crate::{cmd, paths, window};
use gtk::gio;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

thread_local! {
    /// The launcher shown on the right (its file name), kept across the page's redraws.
    static PICKED: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn build(page: &Page) {
    let launchers: Rc<Vec<Launcher>> = Rc::new(launchers());
    let web = cmd::present("omarchy-webapp-install");
    let tui = cmd::present("omarchy-tui-install");

    let pv = page.split();
    let holder = widgets::vbox(14);
    pv.add(&shrinkable(&holder));

    let chosen = PICKED
        .with(|p| p.borrow().clone())
        .and_then(|id| launchers.iter().position(|l| l.id == id))
        .or(if launchers.is_empty() { None } else { Some(0) });
    let apps: Rc<Vec<gio::AppInfo>> = Rc::new(gio::AppInfo::all().into_iter().filter(|a| a.should_show()).collect());

    if web || tui {
        your_launchers(page, &launchers, &holder, &apps, chosen, web, tui);
    }
    if !(web || tui) || launchers.is_empty() {
        holder.append(&preview_head(
            "Your launchers",
            "A website or a terminal program you add here shows up in the app launcher.",
            &[],
        ));
    }
    more(page);
    pv.foot("emblem-ok-symbolic", "Changes save when you press Enter or leave a field. Removing a launcher can be undone for a few seconds.");
}

// ----- Launchers made by Omarchy -----

#[derive(Debug, PartialEq, Clone)]
enum Kind {
    Web(String),
    Tui(String, bool),
}

#[derive(Debug, PartialEq, Clone)]
struct Launcher {
    /// The file name without `.desktop`, which the remove scripts take.
    id: String,
    name: String,
    /// Icon name or file, from the desktop file.
    icon: String,
    kind: Kind,
}

/// Read one desktop file: a web app (`omarchy-launch-webapp URL`) or a terminal
/// app (`xdg-terminal-exec --app-id=TUI.float|tile -e COMMAND`).
fn parse_launcher(id: &str, text: &str) -> Option<Launcher> {
    let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)).map(|v| v.trim().to_string());
    let name = field("Name=").filter(|n| !n.is_empty()).unwrap_or_else(|| id.to_string());
    let exec = field("Exec=")?;
    let icon = field("Icon=").unwrap_or_default();
    if let Some(rest) = exec.strip_prefix("omarchy-launch-webapp ") {
        return Some(Launcher { id: id.into(), name, icon, kind: Kind::Web(rest.trim().trim_matches('"').to_string()) });
    }
    if exec.starts_with("xdg-terminal-exec") && exec.contains("app-id=TUI.") {
        let float = exec.contains("TUI.float");
        let command = exec.split_once(" -e ").map(|(_, c)| c.trim().to_string())?;
        return Some(Launcher { id: id.into(), name, icon, kind: Kind::Tui(command, float) });
    }
    None
}

fn launchers() -> Vec<Launcher> {
    let dir = paths::data_home().join("applications");
    let mut found: Vec<Launcher> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let id = path.file_name()?.to_str()?.strip_suffix(".desktop")?.to_string();
            parse_launcher(&id, &std::fs::read_to_string(&path).ok()?)
        })
        .collect();
    // Web apps first, then terminal apps, each by name.
    found.sort_by_key(|l| (matches!(l.kind, Kind::Tui(..)), l.name.to_lowercase()));
    found
}

/// A web address without its scheme and trailing slash: `figma.com`.
fn host(url: &str) -> String {
    let u = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    u.trim_end_matches('/').to_string()
}

impl Launcher {
    /// The line under its name: the address, or the command and how its window opens.
    fn detail(&self) -> String {
        match &self.kind {
            Kind::Web(url) => host(url),
            Kind::Tui(command, float) => format!("{command} · {}", if *float { "floating" } else { "tiled" }),
        }
    }
}

fn after(done: &'static str) -> impl FnOnce(anyhow::Result<String>) + 'static {
    move |r| {
        match r {
            Ok(_) => window::toast(done),
            Err(e) => window::toast(&format!("{e}")),
        }
        window::rebuild("software");
    }
}

/// The launcher's icon at the start of its row.
fn launcher_icon(icon: &str, fallback: &str, size: i32) -> gtk::Image {
    let img = if icon.starts_with('/') && std::path::Path::new(icon).exists() {
        gtk::Image::from_file(icon)
    } else if !icon.is_empty() && gtk::gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(icon)) {
        gtk::Image::from_icon_name(icon)
    } else {
        gtk::Image::from_icon_name(fallback)
    };
    img.set_pixel_size(size);
    img.set_valign(gtk::Align::Center);
    img.add_css_class("launcher-icon");
    img
}

/// The icon to hand `omarchy-tui-install` when a terminal app is made again: its own,
/// by name when it's already in the icon theme (the script copies a file into the theme
/// and fails copying one onto itself), else the theme's terminal icon.
fn tui_icon(icon: &str) -> String {
    let themed = paths::home().join(".local/share/icons/hicolor");
    let path = std::path::Path::new(icon);
    if icon.is_empty() {
        "utilities-terminal".into()
    } else if path.starts_with(&themed) {
        path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "utilities-terminal".into())
    } else {
        icon.to_string()
    }
}

fn fallback_icon(kind: &Kind) -> &'static str {
    match kind {
        Kind::Web(_) => "applications-internet-symbolic",
        Kind::Tui(..) => "utilities-terminal-symbolic",
    }
}

/// How many launchers show before the rest fold away under "N more".
const FIRST: usize = 6;

/// A filter box at the top of a long list of launchers. Filtering looks through the
/// folded ones too.
fn list_filter(g: &widgets::Group, rows: Vec<(gtk::Box, String)>, fold: Option<gtk::Box>) {
    if rows.len() <= 8 {
        return;
    }
    let f = gtk::SearchEntry::new();
    f.set_placeholder_text(Some(&format!("Filter {} launchers", rows.len())));
    f.set_hexpand(true);
    // The first row of the card, so the list stays one card.
    let holder = widgets::hbox(0);
    holder.add_css_class("settings-option");
    holder.add_css_class("card-row");
    holder.add_css_class("filter-row");
    holder.append(&f);
    g.list.prepend(&holder);
    let list = g.list.clone();
    f.connect_search_changed(move |f| {
        let terms = crate::search::terms(&f.text());
        for (row, text) in &rows {
            row.set_visible(terms.is_empty() || crate::search::matches(text, &terms));
        }
        if let Some(fold) = &fold {
            if !terms.is_empty() {
                widgets::open_disclosure(fold);
            }
            if let Some(button) = fold.first_child() {
                button.set_visible(terms.is_empty());
            }
        }
        widgets::mark_first_rows(list.upcast_ref());
    });
}

/// The list of launchers, with Add above it; picking one shows it on the right.
fn your_launchers(
    page: &Page,
    all: &Rc<Vec<Launcher>>,
    holder: &gtk::Box,
    apps: &Rc<Vec<gio::AppInfo>>,
    chosen: Option<usize>,
    web: bool,
    tui: bool,
) {
    let g = page.group("");
    let head = widgets::hbox(8);
    head.add_css_class("sf-head");
    let title = widgets::label("YOUR LAUNCHERS", "group-title");
    title.set_hexpand(true);
    head.append(&title);
    head.append(&add_button(web, tui));
    g.top(&head);

    if all.is_empty() {
        let r = widgets::row("No launchers yet", "Add a website or a terminal program, and it shows up in the app launcher.", None);
        g.add(&r);
    }
    let rows: Rc<RefCell<Vec<gtk::Box>>> = Rc::default();
    let mut filter = Vec::new();
    // A long list folds after the first few, keeping the rows under it in sight.
    let fold = (all.len() > FIRST + 2).then(|| {
        let (wrapper, content) = widgets::disclosure(&format!("{} more launchers", all.len() - FIRST), "");
        wrapper.add_css_class("sf-more");
        (wrapper, content)
    });
    for (i, l) in all.iter().enumerate() {
        let kind = widgets::tag(if matches!(l.kind, Kind::Web(_)) { "Web" } else { "Terminal" });
        let r = widgets::row(&l.name, &gtk::glib::markup_escape_text(&l.detail()), Some(kind.upcast_ref()));
        r.add_css_class("launcher-row");
        r.prepend(&launcher_icon(&l.icon, fallback_icon(&l.kind), 28));
        super::services::cap_text(&r, 30);
        match &l.kind {
            Kind::Web(url) => widgets::keywords(&format!("webapp web app website launcher pwa edit remove {url}")),
            Kind::Tui(command, _) => widgets::keywords(&format!("tui terminal app launcher command edit remove {command}")),
        }
        let (rows2, r2, l2, holder, apps) = (rows.clone(), r.clone(), l.clone(), holder.clone(), apps.clone());
        pickable(&r, move || {
            PICKED.with(|p| *p.borrow_mut() = Some(l2.id.clone()));
            mark_picked(rows2.borrow().iter(), Some(&r2));
            show(&holder, &l2, &apps);
        });
        match &fold {
            Some((_, content)) if i >= FIRST => content.append(&r),
            _ => g.add(&r),
        }
        filter.push((r.clone(), crate::search::normalise(&format!("{} {}", l.name, l.detail()))));
        rows.borrow_mut().push(r);
    }
    if let Some((wrapper, _)) = &fold {
        g.add(wrapper);
        if chosen.is_some_and(|i| i >= FIRST) {
            widgets::open_disclosure(wrapper);
        }
    }
    list_filter(&g, filter, fold.map(|f| f.0));
    if let Some(i) = chosen {
        mark_picked(rows.borrow().iter(), rows.borrow().get(i));
        show(holder, &all[i], apps);
    }
}

/// Add, with a choice of a web app or a terminal app.
fn add_button(web: bool, tui: bool) -> gtk::MenuButton {
    let b = gtk::MenuButton::new();
    b.add_css_class("sf-add");
    let inner = widgets::hbox(6);
    inner.append(&gtk::Image::from_icon_name("list-add-symbolic"));
    inner.append(&gtk::Label::new(Some("Add")));
    b.set_child(Some(&inner));
    b.set_valign(gtk::Align::Center);
    let menu = widgets::vbox(2);
    menu.add_css_class("sf-menu");
    let pop = gtk::Popover::new();
    pop.set_child(Some(&menu));
    b.set_popover(Some(&pop));
    let item = |label: &str, desc: &str, open: fn()| {
        let i = gtk::Button::new();
        i.add_css_class("flat");
        let t = widgets::vbox(1);
        t.append(&widgets::label(label, "sf-menu-title"));
        t.append(&widgets::label(desc, "sf-menu-desc"));
        i.set_child(Some(&t));
        let pop = pop.clone();
        i.connect_clicked(move |_| {
            pop.popdown();
            open();
        });
        menu.append(&i);
    };
    if web {
        item("Web app…", "A website in a window of its own", add_web_app);
    }
    if tui {
        item("Terminal app…", "A command-line program in the launcher", add_terminal_app);
    }
    widgets::keywords("new create add webapp website tui command");
    b
}

fn add_web_app() {
    ask(
        "Add a web app",
        "A website that opens in its own window and shows up in the app launcher. Its icon is fetched from the site.",
        vec![Field::text("Name"), Field::text("https://example.com")],
        "Add",
        None,
        |v, _| {
            let (n, u) = (v[0].trim().to_string(), v[1].trim().to_string());
            if n.is_empty() || u.is_empty() || n.contains('/') {
                return Some("Enter a name (without /) and a web address.".into());
            }
            PICKED.with(|p| *p.borrow_mut() = Some(n.clone()));
            // An empty icon makes Omarchy fetch the site's own.
            cmd::run_async(&["omarchy-webapp-install", &n, &u, ""], after("Web app added"));
            None
        },
    );
}

fn add_terminal_app() {
    ask(
        "Add a terminal app",
        "A command-line program with an entry in the app launcher.",
        vec![
            Field::text("Name"),
            Field::text("Command, e.g. btop"),
            Field::choice(widgets::opts(&[("float", "Floating window"), ("tile", "Tiled window")]), "float"),
        ],
        "Add",
        None,
        |v, _| {
            let (n, c) = (v[0].trim().to_string(), v[1].trim().to_string());
            if n.is_empty() || c.is_empty() || n.contains('/') {
                return Some("Enter a name (without /) and a command.".into());
            }
            PICKED.with(|p| *p.borrow_mut() = Some(n.clone()));
            // Omarchy needs an icon: use the theme's generic terminal one.
            cmd::run_async(&["omarchy-tui-install", &n, &c, &v[2], "utilities-terminal"], after("Terminal app added"));
            None
        },
    );
}

/// Replace a launcher: make the new one first, so a failed install keeps the old
/// one, then take the old one away if it had another name (the file is named after it).
fn replace_launcher(remove: &'static str, id: String, install: Vec<String>, done: &'static str) {
    if let Some(name) = install.get(1) {
        PICKED.with(|p| *p.borrow_mut() = Some(name.clone()));
    }
    cmd::background(
        move || {
            let refs: Vec<&str> = install.iter().map(String::as_str).collect();
            let out = cmd::run(&refs)?;
            if install.get(1).is_some_and(|name| *name != id) {
                cmd::run(&[remove, &id])?;
            }
            Ok(out)
        },
        after(done),
    );
}

/// Take a launcher away, with Undo to make it again.
fn remove_launcher(l: &Launcher) {
    let (name, kind, icon) = (l.name.clone(), l.kind.clone(), tui_icon(&l.icon));
    let script = match kind {
        Kind::Web(_) => "omarchy-webapp-remove",
        Kind::Tui(..) => "omarchy-tui-remove",
    };
    PICKED.with(|p| *p.borrow_mut() = None);
    cmd::run_async(&[script, &l.id], move |r| {
        window::rebuild("software");
        match r {
            Ok(_) => window::toast_action(&format!("Removed {name}"), "Undo", move || {
                PICKED.with(|p| *p.borrow_mut() = Some(name.clone()));
                match &kind {
                    Kind::Web(url) => cmd::run_async(&["omarchy-webapp-install", &name, url, ""], after("Web app restored")),
                    Kind::Tui(command, float) => cmd::run_async(
                        &["omarchy-tui-install", &name, command, if *float { "float" } else { "tile" }, &icon],
                        after("Terminal app restored"),
                    ),
                }
            }),
            Err(e) => window::toast(&format!("{e}")),
        }
    });
}

/// Start a launcher the way the app launcher would.
fn open_launcher(id: &str) {
    let file = format!("{id}.desktop");
    match gio::AppInfo::all().into_iter().find(|a| a.id().is_some_and(|i| i == file.as_str())) {
        Some(app) => {
            if let Err(e) = app.launch(&[], None::<&gio::AppLaunchContext>) {
                window::toast(&format!("Couldn't open it: {e}"));
            }
        }
        None => window::toast("Couldn't read its launcher."),
    }
}

fn icon_label_button(icon: &str, label: &str) -> gtk::Button {
    let b = gtk::Button::new();
    let inner = widgets::hbox(6);
    inner.append(&gtk::Image::from_icon_name(icon));
    inner.append(&gtk::Label::new(Some(label)));
    b.set_child(Some(&inner));
    b
}

/// The picked launcher: how it shows in the launcher, the window it opens, and its details.
fn show(holder: &gtk::Box, l: &Launcher, apps: &[gio::AppInfo]) {
    widgets::forget_rows(holder);
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let remove = icon_label_button("user-trash-symbolic", "Remove");
    remove.add_css_class("flat");
    {
        let l = l.clone();
        remove.connect_clicked(move |b| {
            b.set_sensitive(false);
            remove_launcher(&l);
        });
    }
    let open = icon_label_button("window-new-symbolic", "Open");
    {
        let id = l.id.clone();
        open.connect_clicked(move |_| open_launcher(&id));
    }
    let line = match &l.kind {
        Kind::Web(url) => format!("A web app: {} in a window of its own", host(url)),
        Kind::Tui(command, float) => {
            format!("A terminal app: {command} in a {} window", if *float { "floating" } else { "tiled" })
        }
    };
    holder.append(&preview_head(&l.name, &line, &[remove.upcast(), open.upcast()]));

    // In the launcher, and the window it opens.
    let stage = widgets::hbox(12);
    stage.add_css_class("preview-card");
    stage.add_css_class("sf-stage");
    let left = widgets::vbox(8);
    left.set_hexpand(true);
    left.append(&widgets::label("In the app launcher", "sf-cap"));
    left.append(&launcher_mock(l, apps));
    stage.append(&left);
    let arrow = gtk::Image::from_icon_name("go-next-symbolic");
    arrow.add_css_class("sf-arrow");
    arrow.set_valign(gtk::Align::Center);
    stage.append(&arrow);
    let right = widgets::vbox(8);
    right.set_hexpand(true);
    right.append(&widgets::label(
        match &l.kind {
            Kind::Web(_) => "When it opens: no browser bars",
            Kind::Tui(_, true) => "When it opens: a floating window",
            Kind::Tui(_, false) => "When it opens: a tile beside the others",
        },
        "sf-cap",
    ));
    let (name, kind) = (l.name.clone(), l.kind.clone());
    let pic = widgets::drawing(150, move |cr, p, _, w, h| draw_window(cr, p, w, h, &name, &kind));
    pic.set_vexpand(true);
    right.append(&pic);
    stage.append(&right);
    holder.append(&stage);

    holder.append(&details(l));
    widgets::forget_rows(holder);
}

/// What the app launcher shows while you type the launcher's name: it, and the other
/// apps on this computer whose names match.
fn launcher_mock(l: &Launcher, apps: &[gio::AppInfo]) -> gtk::Box {
    let typed: String = l.name.to_lowercase().chars().take(3).collect();
    let b = widgets::vbox(2);
    b.add_css_class("sf-launch");
    let search = widgets::hbox(6);
    search.add_css_class("sf-search");
    search.append(&gtk::Image::from_icon_name("system-search-symbolic"));
    let text = widgets::hbox(1);
    text.append(&widgets::label(&typed, "sf-typed"));
    let caret = widgets::hbox(0);
    caret.add_css_class("sf-caret");
    caret.set_valign(gtk::Align::Center);
    text.append(&caret);
    search.append(&text);
    b.append(&search);
    let entry = |icon: gtk::Image, name: &str, kind: &str, on: bool| {
        let r = widgets::hbox(8);
        r.add_css_class("sf-lr");
        if on {
            r.add_css_class("on");
        }
        icon.set_pixel_size(18);
        r.append(&icon);
        let n = widgets::label(name, "sf-lr-name");
        n.set_hexpand(true);
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        r.append(&n);
        r.append(&widgets::label(kind, "sf-lr-kind"));
        r
    };
    let kind = if matches!(l.kind, Kind::Web(_)) { "Web app" } else { "Terminal app" };
    b.append(&entry(launcher_icon(&l.icon, fallback_icon(&l.kind), 18), &l.name, kind, true));
    let own = format!("{}.desktop", l.id);
    let others = apps
        .iter()
        .filter(|a| a.id().is_none_or(|i| i != own.as_str()) && a.display_name().to_lowercase().contains(&typed))
        .take(3);
    for a in others {
        let icon = match a.icon() {
            Some(i) => gtk::Image::from_gicon(&i),
            None => gtk::Image::from_icon_name("application-x-executable-symbolic"),
        };
        b.append(&entry(icon, &a.display_name(), "App", false));
    }
    b
}

/// The launcher's details, edited in place: name, address or command, how its window
/// opens, its icon, and a way to a keyboard shortcut.
fn details(l: &Launcher) -> gtk::Box {
    let card = widgets::vbox(0);
    card.add_css_class("settings-card");
    card.add_css_class("sf-form");
    card.set_overflow(gtk::Overflow::Hidden);
    let state = Rc::new(RefCell::new(match &l.kind {
        Kind::Web(url) => (l.name.clone(), url.clone(), true),
        Kind::Tui(command, float) => (l.name.clone(), command.clone(), *float),
    }));
    let web = matches!(l.kind, Kind::Web(_));
    let save: Rc<dyn Fn()> = {
        let (state, id, icon) = (state.clone(), l.id.clone(), tui_icon(&l.icon));
        Rc::new(move || {
            let (n, v, float) = state.borrow().clone();
            let (n, v) = (n.trim().to_string(), v.trim().to_string());
            if n.is_empty() || v.is_empty() || n.contains('/') {
                window::toast(if web { "Enter a name (without /) and a web address" } else { "Enter a name (without /) and a command" });
                return;
            }
            if web {
                replace_launcher("omarchy-webapp-remove", id.clone(), vec!["omarchy-webapp-install".into(), n, v, String::new()], "Web app updated");
            } else {
                let style = if float { "float" } else { "tile" };
                replace_launcher(
                    "omarchy-tui-remove",
                    id.clone(),
                    vec!["omarchy-tui-install".into(), n, v, style.into(), icon.clone()],
                    "Terminal app updated",
                );
            }
        })
    };
    let (r, e) = widgets::entry_row("Name", "", &l.name, "Name", {
        let (state, save) = (state.clone(), save.clone());
        move |t| {
            state.borrow_mut().0 = t;
            save();
        }
    });
    e.set_width_chars(20);
    card.append(&r);
    let (title, placeholder, value) = match &l.kind {
        Kind::Web(url) => ("Address", "https://example.com", url.clone()),
        Kind::Tui(command, _) => ("Command", "e.g. btop", command.clone()),
    };
    let (r, e) = widgets::entry_row(title, "", &value, placeholder, {
        let (state, save) = (state.clone(), save.clone());
        move |t| {
            state.borrow_mut().1 = t;
            save();
        }
    });
    e.set_width_chars(20);
    card.append(&r);
    if let Kind::Tui(_, float) = &l.kind {
        let seg = nyx_ui::widgets::segmented(
            &widgets::opts(&[("float", "Floating"), ("tile", "Tiled")]),
            if *float { "float" } else { "tile" },
            {
                let (state, save) = (state.clone(), save.clone());
                move |id| {
                    let float = id == "float";
                    if state.borrow().2 != float {
                        state.borrow_mut().2 = float;
                        save();
                    }
                }
            },
        );
        card.append(&widgets::row("Window", "Floating sits over your other windows; tiled takes a tile beside them.", Some(seg.upcast_ref())));
    }
    let icon_end = widgets::hbox(8);
    icon_end.append(&launcher_icon(&l.icon, fallback_icon(&l.kind), 28));
    if let Kind::Web(url) = &l.kind {
        let again = icon_label_button("view-refresh-symbolic", "Fetch again");
        let (id, name, url) = (l.id.clone(), l.name.clone(), url.clone());
        again.connect_clicked(move |b| {
            b.set_sensitive(false);
            replace_launcher(
                "omarchy-webapp-remove",
                id.clone(),
                vec!["omarchy-webapp-install".into(), name.clone(), url.clone(), String::new()],
                "Icon fetched again",
            );
        });
        icon_end.append(&again);
    }
    card.append(&widgets::row(
        "Icon",
        if web {
            "Fetched from the site"
        } else if l.icon.is_empty() || l.icon == "utilities-terminal" {
            "The theme's terminal icon"
        } else {
            "Set in its launcher"
        },
        Some(icon_end.upcast_ref()),
    ));
    let (r, _) = widgets::nav_row(None, "Shortcut", "Open it with a key combination, on Keyboard Shortcuts", "", || {
        window::navigate("keybindings")
    });
    card.append(&r);
    widgets::mark_first_rows(card.upcast_ref());
    card
}

/// The window a launcher opens: a website with no browser bars, or a terminal
/// floating over the desktop or tiled beside the other windows.
fn draw_window(cr: &gtk::cairo::Context, p: &Pal, w: f64, h: f64, name: &str, kind: &Kind) {
    match kind {
        Kind::Web(_) => {
            let r = R::new(0.5, 0.5, w - 1.0, h - 1.0);
            sketch::rounded(cr, r.x, r.y, r.w, r.h, 7.0);
            sketch::set(cr, sketch::mix(p.bg, p.text, 0.05), 1.0);
            let _ = cr.fill_preserve();
            sketch::set(cr, p.dim, 0.3);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            let th = 20.0;
            for i in 0..3 {
                sketch::set(cr, p.dim, 0.5);
                cr.arc(r.x + 10.0 + i as f64 * 7.0, r.y + th / 2.0, 2.2, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
            sketch::set(cr, p.text, 0.9);
            cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
            cr.set_font_size(10.0);
            cr.move_to(r.x + 36.0, r.y + th / 2.0 + 3.5);
            let _ = cr.show_text(name);
            sketch::set(cr, p.dim, 0.2);
            cr.rectangle(r.x, r.y + th, r.w, 1.0);
            let _ = cr.fill();
            let body = R::new(r.x + 8.0, r.y + th + 9.0, r.w - 16.0, r.h - th - 17.0);
            let (side, canvas) = body.hsplit(0.22, 10.0);
            for i in 0..4 {
                let y = side.y + i as f64 * 9.0;
                let on = i == 2;
                sketch::set(cr, if on { p.accent } else { p.dim }, if on { 0.9 } else { 0.35 });
                sketch::rounded(cr, side.x, y, side.w * if i % 2 == 0 { 0.9 } else { 0.7 }, 3.5, 1.7);
                let _ = cr.fill();
            }
            sketch::set(cr, p.dim, 0.22);
            let mut y = canvas.y + 4.0;
            while y < canvas.y + canvas.h {
                let mut x = canvas.x + 4.0;
                while x < canvas.x + canvas.w {
                    cr.rectangle(x, y, 1.2, 1.2);
                    x += 9.0;
                }
                y += 9.0;
            }
            let _ = cr.fill();
            let shape = R::new(canvas.x + canvas.w * 0.14, canvas.y + canvas.h * 0.1, canvas.w * 0.4, canvas.h * 0.5);
            sketch::rounded(cr, shape.x, shape.y, shape.w, shape.h, 4.0);
            sketch::set(cr, p.accent, 0.35);
            let _ = cr.fill_preserve();
            sketch::set(cr, p.accent, 0.9);
            let _ = cr.stroke();
            sketch::set(cr, sketch::mix(p.accent, p.danger, 0.7), 0.9);
            cr.arc(canvas.x + canvas.w * 0.72, canvas.y + canvas.h * 0.28, (canvas.w.min(canvas.h) * 0.1).max(4.0), 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            sketch::set(cr, sketch::mix(p.accent, p.text, 0.35), 0.6);
            sketch::rounded(cr, canvas.x + canvas.w * 0.3, canvas.y + canvas.h * 0.75, canvas.w * 0.42, 9.0, 3.0);
            let _ = cr.fill();
        }
        Kind::Tui(command, float) => {
            let area = sketch::desktop(cr, p, R::new(0.0, 0.0, w, h));
            let gap = (w * 0.01).clamp(3.0, 8.0);
            let t = if *float {
                R::new(area.x + area.w * 0.17, area.y + area.h * 0.16, area.w * 0.66, area.h * 0.68)
            } else {
                area.hsplit(0.52, gap).0
            };
            if *float {
                sketch::rounded(cr, t.x + 2.0, t.y + 3.0, t.w, t.h, 5.0);
                sketch::set(cr, (0.0, 0.0, 0.0), 0.3);
                let _ = cr.fill();
            }
            sketch::rounded(cr, t.x, t.y, t.w, t.h, 5.0);
            sketch::set(cr, sketch::mix(p.bg, (0.0, 0.0, 0.0), 0.3), 1.0);
            let _ = cr.fill_preserve();
            sketch::set(cr, p.accent, 1.0);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            cr.save().ok();
            cr.rectangle(t.x + 2.0, t.y, t.w - 4.0, t.h);
            cr.clip();
            cr.select_font_face("monospace", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
            cr.set_font_size(9.5);
            sketch::set(cr, p.accent, 1.0);
            cr.move_to(t.x + 8.0, t.y + 15.0);
            let _ = cr.show_text("$ ");
            sketch::set(cr, p.text, 0.95);
            let _ = cr.show_text(command);
            for i in 0..4 {
                sketch::set(cr, p.dim, 0.45);
                let lw = (t.w - 16.0) * [0.7, 0.5, 0.82, 0.4][i];
                cr.rectangle(t.x + 8.0, t.y + 24.0 + i as f64 * 8.0, lw.max(0.0), 2.5);
                let _ = cr.fill();
            }
            cr.restore().ok();
        }
    }
}

// ----- One level down: installers, packages, updates -----

/// The rows under the list, each opening its own page.
fn more(page: &Page) {
    let g = page.group("");
    let installed = Rc::new(Installed::read());

    let available: usize = GROUPS.iter().map(|(_, _, items)| items.iter().filter(|i| item_button_kind(i, &installed).is_some()).count()).sum();
    let preinstalls = ["omarchy-install-preinstalls", "omarchy-remove-preinstalls"].iter().any(|s| cmd::present(s));
    if available > 0 || preinstalls {
        let on: usize = GROUPS.iter().map(|(_, _, items)| items.iter().filter(|i| installed.has(i.1)).count()).sum();
        let i2 = installed.clone();
        let (r, _) = widgets::nav_row(
            Some("system-software-install-symbolic"),
            "Install apps",
            "Editors, games, AI tools and services",
            &format!("{on} installed"),
            move || {
                let i = i2.clone();
                widgets::subpage("Install apps", 560, move |p| {
                    installers(p, &i);
                    widgets::forget_rows(&p.body);
                });
            },
        );
        let names: Vec<&str> = GROUPS.iter().flat_map(|(t, w, items)| [*t, *w].into_iter().chain(items.iter().map(|i| i.0))).collect();
        widgets::keywords(&format!("installers omarchy starter apps preinstalls default bundled bloat {}", names.join(" ")));
        g.add(&r);
    }

    if cmd::present("pacman") {
        let explicit = cmd::output(&["pacman", "-Qqe"]).map(|t| t.lines().count()).unwrap_or(0);
        let (r, _) = widgets::nav_row(
            Some("package-x-generic-symbolic"),
            "Packages",
            "Find anything in Arch or the AUR",
            &explicit.to_string(),
            move || {
                widgets::subpage("Packages", 560, move |p| {
                    packages(p, explicit);
                    widgets::forget_rows(&p.body);
                });
            },
        );
        widgets::keywords("pacman yay aur install uninstall search find a package app software installed on purpose");
        g.add(&r);
    }

    let channel = cmd::present("omarchy-channel-set").then(|| cmd::output(&["omarchy-channel-current"]).unwrap_or_else(|| "unknown".into()));
    let tools: Vec<&Tool> = TOOLS.iter().filter(|t| cmd::present(t.0)).collect();
    if channel.is_some() || !tools.is_empty() {
        let value = channel.as_deref().map(channel_title).unwrap_or_default();
        let words: Vec<String> = tools.iter().map(|t| format!("{} {}", t.1, t.3)).collect();
        let (r, _) = widgets::nav_row(
            Some("software-update-available-symbolic"),
            "Updates and maintenance",
            "Channel, firmware, the AUR, package keys",
            &value,
            move || {
                let channel = channel.clone();
                widgets::subpage("Updates and maintenance", 560, move |p| {
                    if let Some(c) = &channel {
                        channel_group(p, c);
                    }
                    maintenance(p);
                    widgets::forget_rows(&p.body);
                });
            },
        );
        widgets::keywords(&format!("update channel release edge rc beta stable omarchy version maintenance {}", words.join(" ")));
        g.add(&r);
    }
}

// ----- Packages -----

fn valid_package(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 100
        && !s.starts_with(['-', '.'])
        && s.chars().all(|c| c.is_ascii_alphanumeric() || "@._+-".contains(c))
}

fn packages(page: &Page, explicit: usize) {
    let g = page.group("");
    let (r, _) = widgets::info_row("Installed on purpose", &format!("{explicit} packages"));
    g.add(&r);
    let results = widgets::vbox(6);
    let (r, entry) = widgets::entry_row(
        "Find a package",
        "Search the Arch repositories and the AUR, by name. Press Enter.",
        "",
        "package name",
        {
            let results = results.clone();
            move |q| search(&results, q.trim())
        },
    );
    entry.set_width_chars(22);
    g.add(&r);
    let g = page.plain_group("");
    g.add(&results);
}

fn search(results: &gtk::Box, query: &str) {
    while let Some(c) = results.first_child() {
        results.remove(&c);
    }
    if query.is_empty() || query.starts_with('-') {
        return;
    }
    results.append(&widgets::label("Searching…", "dim"));
    let helper = if cmd::present("yay") { "yay" } else { "pacman" };
    let q = query.to_string();
    let results = results.clone();
    cmd::background(
        move || {
            let found = cmd::output(&[helper, "-Ssq", "--", &q]).unwrap_or_default();
            let installed: HashSet<String> =
                cmd::output(&["pacman", "-Qq"]).unwrap_or_default().lines().map(str::to_string).collect();
            (found, installed)
        },
        move |(found, installed)| {
            while let Some(c) = results.first_child() {
                results.remove(&c);
            }
            let names: Vec<&str> = found.lines().filter(|n| valid_package(n)).take(15).collect();
            if names.is_empty() {
                results.append(&widgets::label("Nothing found.", "dim"));
            }
            let card = widgets::vbox(0);
            card.add_css_class("settings-card");
            card.set_overflow(gtk::Overflow::Hidden);
            for n in &names {
                let on = installed.contains(*n);
                let b = gtk::Button::with_label(if on { "Remove…" } else { "Install…" });
                let name = n.to_string();
                b.connect_clicked(move |_| {
                    let q = cmd::shell_quote(&name);
                    terminal(&if on { format!("sudo pacman -Rns -- {q}") } else { format!("{} -S --needed -- {q}", helper) });
                });
                card.append(&widgets::icon_row("package-x-generic-symbolic", n, if on { "Installed" } else { "" }, Some(b.upcast_ref())));
            }
            if !names.is_empty() {
                widgets::mark_first_rows(card.upcast_ref());
                results.append(&card);
            }
            widgets::forget_rows(&results);
        },
    );
}

// ----- Omarchy installers -----

/// How to tell an app is installed.
#[derive(Clone, Copy)]
enum Check {
    /// Any of these packages.
    Pkgs(&'static [&'static str]),
    /// A launcher in `~/.local/share/applications` (by file name, without `.desktop`).
    Desktop(&'static str),
    /// A Flatpak app id.
    Flatpak(&'static str),
}

/// (name, how to tell, install script, remove script)
type Item = (&'static str, Check, &'static str, Option<&'static str>);

/// (title, search words, apps)
const GROUPS: &[(&str, &str, &[Item])] = &[
    (
        "Editors",
        "code editor vscode zed helix emacs",
        &[
            ("VS Code", Check::Pkgs(&["visual-studio-code-bin"]), "omarchy-install-editor-vscode", None),
            ("Zed", Check::Pkgs(&["zed"]), "omarchy-install-editor-zed", None),
            ("Helix", Check::Pkgs(&["helix"]), "omarchy-install-editor-helix", None),
            ("Emacs", Check::Pkgs(&["omarchy-emacs"]), "omarchy-install-editor-emacs", None),
        ],
    ),
    (
        "Games",
        "gaming steam lutris heroic retroarch battle.net geforce xbox controller",
        &[
            ("Steam", Check::Pkgs(&["steam"]), "omarchy-install-gaming-steam", Some("omarchy-remove-gaming-steam")),
            ("Lutris", Check::Pkgs(&["lutris"]), "omarchy-install-gaming-lutris", Some("omarchy-remove-gaming-lutris")),
            (
                "Heroic",
                Check::Pkgs(&["heroic-games-launcher-bin"]),
                "omarchy-install-gaming-heroic",
                Some("omarchy-remove-gaming-heroic"),
            ),
            (
                "RetroArch",
                Check::Pkgs(&["retroarch"]),
                "omarchy-install-gaming-retroarch",
                Some("omarchy-remove-gaming-retroarch"),
            ),
            (
                "Battle.net",
                Check::Desktop("battlenet"),
                "omarchy-install-gaming-battlenet",
                Some("omarchy-remove-gaming-battlenet"),
            ),
            (
                "GeForce Now",
                Check::Flatpak("com.nvidia.geforcenow"),
                "omarchy-install-gaming-geforce-now",
                Some("omarchy-remove-gaming-geforce-now"),
            ),
            (
                "Xbox Cloud Gaming",
                Check::Desktop("Xbox Cloud Gaming"),
                "omarchy-install-gaming-xbox-cloud",
                Some("omarchy-remove-gaming-xbox-cloud"),
            ),
            (
                "Xbox controllers",
                Check::Pkgs(&["xpadneo-dkms"]),
                "omarchy-install-gaming-xbox-controllers",
                Some("omarchy-remove-gaming-xbox-controllers"),
            ),
        ],
    ),
    (
        "AI",
        "ai assistant claude chatgpt ollama lm studio openclaw hermes",
        &[
            ("Claude", Check::Pkgs(&["claude-desktop"]), "omarchy-install-ai-claude", Some("omarchy-remove-ai-claude")),
            ("ChatGPT", Check::Pkgs(&["openai-codex-desktop"]), "omarchy-install-ai-chatgpt", Some("omarchy-remove-ai-chatgpt")),
            ("Hermes", Check::Desktop("hermes"), "omarchy-install-ai-hermes", Some("omarchy-remove-ai-hermes")),
            ("OpenClaw", Check::Pkgs(&["openclaw"]), "omarchy-install-ai-openclaw", Some("omarchy-remove-ai-openclaw")),
            ("T3 Code", Check::Pkgs(&["t3code-bin"]), "omarchy-install-ai-t3-code", Some("omarchy-remove-ai-t3-code")),
            (
                "Ollama",
                Check::Pkgs(&["ollama", "ollama-cuda", "ollama-rocm", "ollama-vulkan"]),
                "",
                Some("omarchy-remove-ai-ollama"),
            ),
            ("LM Studio", Check::Pkgs(&["lmstudio-bin"]), "", Some("omarchy-remove-ai-lm-studio")),
        ],
    ),
    (
        "Services",
        "service 1password dropbox signal spotify sunshine tailscale nordvpn",
        &[
            (
                "1Password",
                Check::Pkgs(&["1password"]),
                "omarchy-install-service-1password",
                Some("omarchy-remove-service-1password"),
            ),
            ("Dropbox", Check::Pkgs(&["dropbox"]), "omarchy-install-service-dropbox", Some("omarchy-remove-service-dropbox")),
            ("Signal", Check::Pkgs(&["signal-desktop"]), "omarchy-install-service-signal", None),
            ("Spotify", Check::Pkgs(&["spotify"]), "omarchy-install-service-spotify", None),
            ("Sunshine", Check::Pkgs(&["sunshine"]), "omarchy-install-service-sunshine", Some("omarchy-remove-service-sunshine")),
            (
                "Tailscale",
                Check::Pkgs(&["tailscale"]),
                "omarchy-install-service-tailscale",
                Some("omarchy-remove-service-tailscale"),
            ),
            ("NordVPN", Check::Pkgs(&["nordvpn-bin"]), "omarchy-install-service-nordvpn", None),
        ],
    ),
];

fn group_icon(title: &str) -> &'static str {
    match title {
        "Editors" => "accessories-text-editor-symbolic",
        "Games" => "input-gaming-symbolic",
        "AI" => "starred-symbolic",
        _ => "network-server-symbolic",
    }
}

/// What is installed, found once for the whole page.
struct Installed {
    packages: HashSet<String>,
    flatpaks: HashSet<String>,
}

impl Installed {
    fn read() -> Installed {
        let list = |args: &[&str]| {
            cmd::output(args).unwrap_or_default().lines().map(|l| l.split_whitespace().next().unwrap_or("").to_string()).collect()
        };
        let flatpaks =
            if cmd::present("flatpak") { list(&["flatpak", "list", "--app", "--columns=application"]) } else { HashSet::new() };
        Installed { packages: list(&["pacman", "-Qq"]), flatpaks }
    }

    fn has(&self, check: Check) -> bool {
        match check {
            Check::Pkgs(names) => names.iter().any(|n| self.packages.contains(*n)),
            Check::Desktop(stem) => paths::data_home().join("applications").join(format!("{stem}.desktop")).exists(),
            Check::Flatpak(id) => self.flatpaks.contains(id),
        }
    }
}

/// What an app's button does: run Omarchy's script, or take its packages off.
enum Action {
    Script(&'static str),
    RemovePackages(String),
}

/// The button an app gets, if any: Remove when it's installed (Omarchy's remover, or
/// pacman when there's none), Install when its installer is here.
fn item_button_kind(item: &Item, installed: &Installed) -> Option<(bool, Action)> {
    let (_, check, install, remove) = *item;
    if installed.has(check) {
        match remove.filter(|r| cmd::present(r)) {
            Some(script) => Some((true, Action::Script(script))),
            None => match check {
                Check::Pkgs(names) => {
                    let found: Vec<String> =
                        names.iter().filter(|n| installed.packages.contains(**n)).map(|n| cmd::shell_quote(n)).collect();
                    Some((true, Action::RemovePackages(format!("sudo pacman -Rns -- {}", found.join(" ")))))
                }
                _ => None,
            },
        }
    } else if !install.is_empty() && cmd::present(install) {
        Some((false, Action::Script(install)))
    } else {
        None
    }
}

fn installers(page: &Page, installed: &Installed) {
    let intro = page.plain_group("");
    intro.note("Omarchy's own installers. A terminal opens to show progress and ask for your password.");
    for (title, _, items) in GROUPS {
        let rows: Vec<gtk::Box> = items
            .iter()
            .filter_map(|item| {
                let (on, action) = item_button_kind(item, installed)?;
                let b = gtk::Button::with_label(if on { "Remove…" } else { "Install" });
                b.connect_clicked(move |_| match &action {
                    Action::Script(s) => terminal(s),
                    Action::RemovePackages(c) => terminal(c),
                });
                Some(widgets::icon_row(group_icon(title), item.0, if on { "Installed" } else { "" }, Some(b.upcast_ref())))
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        let g = page.group(title);
        for r in rows {
            g.add(&r);
        }
    }
    let controls = widgets::hbox(8);
    for (script, label) in [("omarchy-install-preinstalls", "Install…"), ("omarchy-remove-preinstalls", "Remove…")] {
        if cmd::present(script) {
            let b = gtk::Button::with_label(label);
            b.connect_clicked(move |_| terminal(script));
            controls.append(&b);
        }
    }
    if controls.first_child().is_some() {
        let g = page.group("Starter apps");
        g.add(&widgets::icon_row(
            "view-app-grid-symbolic",
            "Starter apps",
            "The apps Omarchy comes with, installed or removed together.",
            Some(controls.upcast_ref()),
        ));
    }
}

// ----- Channel and update tools -----

fn channel_title(id: &str) -> String {
    match id {
        "stable" => "Stable".into(),
        "rc" => "Release candidate".into(),
        "edge" => "Edge".into(),
        other => other.to_string(),
    }
}

fn channel_group(page: &Page, current: &str) {
    let g = page.group("Update channel");
    g.note("Switching opens a terminal, which asks for your password.");
    let mut choices: Vec<(&str, &str)> = vec![
        ("stable", "Tested the most. Right for nearly everyone."),
        ("rc", "The next version a week or two early."),
        ("edge", "Changes as they land; things may break."),
    ];
    if !["stable", "rc", "edge"].contains(&current) {
        choices.insert(0, (current, "The channel this computer is on now."));
    }
    let mut first: Option<gtk::CheckButton> = None;
    for (id, desc) in choices {
        let radio = gtk::CheckButton::new();
        radio.set_group(first.as_ref());
        radio.set_active(id == current);
        first.get_or_insert(radio.clone());
        let id = id.to_string();
        let switchable = ["stable", "rc", "edge"].contains(&id.as_str());
        radio.set_sensitive(switchable || id == current);
        {
            let id = id.clone();
            let current = current.to_string();
            radio.connect_toggled(move |r| {
                if r.is_active() && id != current && switchable {
                    terminal(&format!("omarchy-channel-set {id}"));
                }
            });
        }
        let r = widgets::row(&channel_title(&id), desc, None);
        if id == current {
            widgets::tag_row(&r, "You're here");
        }
        r.prepend(&radio);
        let click = gtk::GestureClick::new();
        let radio2 = radio.clone();
        click.connect_released(move |_, _, _, _| radio2.set_active(true));
        r.add_controller(click);
        g.add(&r);
    }
}

/// (script, title, what it does, search words)
type Tool = (&'static str, &'static str, &'static str, &'static str);

const TOOLS: &[Tool] = &[
    ("omarchy-update-firmware", "Firmware updates", "Update the firmware of your hardware.", "fwupd bios device"),
    ("omarchy-update-aur-pkgs", "AUR package updates", "Update the packages built from the AUR.", "yay aur"),
    ("omarchy-update-keyring", "Refresh package keys", "Fix 'invalid signature' errors when installing packages.", "pacman keyring gpg signature"),
    ("omarchy-update-orphan-pkgs", "Remove unused packages", "Packages nothing needs any more.", "orphans cleanup clean"),
    ("omarchy-update-analyze-logs", "Check the last update", "Look through the update log for problems.", "log errors"),
];

fn tool_icon(script: &str) -> &'static str {
    match script {
        "omarchy-update-firmware" => "computer-symbolic",
        "omarchy-update-aur-pkgs" => "package-x-generic-symbolic",
        "omarchy-update-keyring" => "dialog-password-symbolic",
        "omarchy-update-orphan-pkgs" => "user-trash-symbolic",
        _ => "document-open-recent-symbolic",
    }
}

fn maintenance(page: &Page) {
    let tools: Vec<&Tool> = TOOLS.iter().filter(|t| cmd::present(t.0)).collect();
    if tools.is_empty() {
        return;
    }
    let g = page.group("Keep it healthy");
    for (script, title, desc, _) in tools {
        let script = *script;
        let end = widgets::hbox(10);
        let b = gtk::Button::with_label("Run…");
        b.connect_clicked(move |_| terminal(script));
        if script == "omarchy-update-orphan-pkgs" {
            // What it would take off right now.
            let v = widgets::label("", "row-value");
            v.set_visible(false);
            end.append(&v);
            cmd::background(crate::backend::cleanup::orphans, move |(names, bytes)| {
                v.set_text(&if names.is_empty() {
                    "None now".to_string()
                } else {
                    format!("{} · {}", names.len(), crate::backend::sysinfo::bytes_text(bytes))
                });
                v.set_visible(true);
            });
        }
        end.append(&b);
        g.add(&widgets::icon_row(tool_icon(script), title, desc, Some(end.upcast_ref())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_launcher() {
        let l = parse_launcher(
            "Google Docs",
            "[Desktop Entry]\nName=Google Docs\nExec=omarchy-launch-webapp \"https://docs.google.com\"\n",
        )
        .unwrap();
        assert_eq!(l.kind, Kind::Web("https://docs.google.com".into()));
        assert_eq!(l.name, "Google Docs");
        assert_eq!(l.detail(), "docs.google.com");
    }

    #[test]
    fn terminal_launcher() {
        let l =
            parse_launcher("Disk Usage", "Name=Disk Usage\nExec=xdg-terminal-exec --app-id=TUI.float -e bash -c \"dua i /\"\n")
                .unwrap();
        assert_eq!(l.kind, Kind::Tui("bash -c \"dua i /\"".into(), true));
        let l = parse_launcher("Docker", "Exec=xdg-terminal-exec --app-id=TUI.tile -e omarchy-launch-docker-tui\n").unwrap();
        assert_eq!(l.kind, Kind::Tui("omarchy-launch-docker-tui".into(), false));
        assert_eq!(l.name, "Docker");
        assert_eq!(l.detail(), "omarchy-launch-docker-tui · tiled");
    }

    #[test]
    fn other_launchers_are_ignored() {
        assert!(parse_launcher("chromium", "Name=Chromium\nExec=chromium %U\n").is_none());
        assert!(parse_launcher("x", "Name=x\n").is_none());
    }

    #[test]
    fn package_names() {
        assert!(valid_package("lib32-gcc-libs") && valid_package("qt6-base") && valid_package("a+b"));
        assert!(!valid_package("-Rns") && !valid_package("a b") && !valid_package("a;b") && !valid_package(""));
    }

    #[test]
    fn hosts() {
        assert_eq!(host("https://web.whatsapp.com/"), "web.whatsapp.com");
        assert_eq!(host("figma.com"), "figma.com");
    }
}
