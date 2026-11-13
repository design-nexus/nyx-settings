//! The main window: the unified header (the app, the sidebar toggle, search), the
//! sidebar of sections, and the page: its section, its name, its config files and
//! the section's other pages as tabs over a stack of pages built the first time
//! they're shown.

use crate::capability::Visibility;
use crate::sections::{self, Section};
use crate::widgets::{self, SEARCH};
use crate::{prefs, theme};
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

struct Ui {
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    nav: gtk::Box,
    nav_list: gtk::Box,
    /// The sidebar's rows: Home, then one per section.
    categories: Vec<Category>,
    /// The section of each page in the sidebar (Home's is "").
    page_group: HashMap<&'static str, &'static str>,
    /// A Cell: it's set while the window is being shown, when the UI is already borrowed.
    compact: std::cell::Cell<bool>,
    pages: HashMap<&'static str, gtk::ScrolledWindow>,
    sections: Vec<Section>,
    current: &'static str,
    overlay: gtk::Overlay,
    collapse_button: gtk::Button,
    head: PageHead,
    /// The "nothing matches" page's message.
    empty_label: gtk::Label,
    /// The toast on screen, if any: a new one replaces it.
    toast: Option<gtk::Revealer>,
}

/// A sidebar row: a section, whose pages are the tabs over the page.
struct Category {
    group: &'static str,
    button: gtk::Button,
    pages: Vec<&'static str>,
}

/// Over the page: its section in the section's colour, its name and what it's for,
/// its config files, and the section's pages as tabs.
struct PageHead {
    root: gtk::Box,
    kicker: gtk::Label,
    title: gtk::Label,
    about: gtk::Label,
    edit: gtk::Box,
    tabs: gtk::Box,
    tabs_scroll: gtk::ScrolledWindow,
    /// The section colour class on the kicker and the tabs' underlines.
    section: RefCell<String>,
}

fn page_head() -> PageHead {
    let root = widgets::vbox(0);
    root.add_css_class("page-head");
    let top = widgets::hbox(8);
    let kicker = widgets::label("", "page-kicker");
    kicker.set_hexpand(true);
    kicker.set_valign(gtk::Align::Center);
    top.append(&kicker);
    let edit = widgets::hbox(0);
    top.append(&edit);
    root.append(&top);
    let line = widgets::hbox(12);
    line.add_css_class("page-head-line");
    let title = widgets::label("", "page-head-title");
    title.set_valign(gtk::Align::BaselineCenter);
    line.append(&title);
    let about = widgets::label("", "page-head-about");
    about.set_valign(gtk::Align::BaselineCenter);
    about.set_ellipsize(gtk::pango::EllipsizeMode::End);
    // Its width is what's left beside the name; a long one doesn't widen the window.
    about.set_max_width_chars(1);
    about.set_hexpand(true);
    line.append(&about);
    root.append(&line);
    let tabs = widgets::hbox(18);
    tabs.add_css_class("page-tabs");
    let tabs_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&tabs)
        .build();
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    let s = tabs_scroll.clone();
    wheel.connect_scroll(move |_, dx, dy| {
        let adj = s.hadjustment();
        adj.set_value(adj.value() + (dx + dy) * 40.0);
        glib::Propagation::Stop
    });
    tabs_scroll.add_controller(wheel);
    root.append(&tabs_scroll);
    PageHead { root, kicker, title, about, edit, tabs, tabs_scroll, section: RefCell::default() }
}

/// The stack page shown when a search finds nothing.
const EMPTY_PAGE: &str = "__no-results";

fn search_empty_page() -> (gtk::Box, gtk::Label) {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    page.add_css_class("search-empty");
    page.set_valign(gtk::Align::Start);
    let icon = gtk::Image::from_icon_name("system-search-symbolic");
    icon.set_pixel_size(48);
    page.append(&icon);
    let title = widgets::label("", "search-empty-title");
    title.set_halign(gtk::Align::Center);
    title.set_wrap(true);
    page.append(&title);
    let hint = widgets::label("Try a different word, such as what the setting changes.", "dim");
    hint.set_halign(gtk::Align::Center);
    page.append(&hint);
    (page, title)
}

/// Show the search field in the top bar and put the cursor in it.
fn focus_search(search: &gtk::SearchEntry) {
    search.set_visible(true);
    search.grab_focus();
}

/// Clear the search; the field stays in the header.
fn close_search(search: &gtk::SearchEntry) {
    search.set_text("");
    if let Some(w) = search.root().and_downcast::<gtk::Window>() {
        gtk::prelude::GtkWindowExt::set_focus(&w, None::<&gtk::Widget>);
    }
}

thread_local! {
    static UI: RefCell<Option<Rc<RefCell<Ui>>>> = const { RefCell::new(None) };
}

fn ui() -> Option<Rc<RefCell<Ui>>> {
    UI.with(|u| u.borrow().clone())
}

pub fn present(app: &gtk::Application, section: Option<&str>) {
    if let Some(ui) = ui() {
        let window = ui.borrow().window.clone();
        if let Some(s) = section {
            navigate(s);
        }
        window.present();
        return;
    }
    // The slowest pages' questions to the system, asked while the window opens.
    crate::cmd::fetch_ahead(sections::power::READS);
    crate::cmd::fetch_ahead(&sections::services::reads());
    if sections::assistant::client::installed() {
        crate::cmd::fetch_ahead(sections::assistant::client::READS);
    }
    theme::install();
    build(app);
    let p = prefs::get();
    // "Open on" is shared by every Nyx app now: carry Settings' own choice over once.
    if p.open_on == "last" {
        nyx_theme::update_suite(|s| s.start = "last".into());
        prefs::update(|p| p.open_on = "suite".into());
    }
    let start = section.map(String::from).unwrap_or_else(|| nyx_theme::start_page("home", &p.last_section));
    navigate(&start);
    // Developer aid: SETTINGS_SNAPSHOT=/path.png renders the window to a PNG
    // (invisibly) and quits, so layouts can be checked without a visible window.
    if let Some(out) = std::env::var_os("SETTINGS_SNAPSHOT") {
        snapshot_and_quit(app, std::path::PathBuf::from(out));
        return;
    }
    // Clone first: presenting realizes the window, and its handlers borrow the UI.
    if let Some(window) = ui().map(|u| u.borrow().window.clone()) {
        window.present();
    }
    // Bring the keyboard backlight timeout helper back if it isn't running.
    crate::cmd::background(crate::backend::kbdidle::ensure_running, |_| {});
    // Keep the theme hook that re-applies the chosen icon theme and extension settings in place.
    crate::cmd::background(crate::backend::themehook::ensure, |_| {});
    // Look for updates (at most every 20 hours); announce a new Settings once, and
    // install extension updates if that's switched on.
    if std::env::var_os("SETTINGS_SNAPSHOT").is_none() {
        crate::cmd::background(|| crate::backend::updates::check(false), after_update_check);
    }
    // Pages showing live system state redraw when it changes.
    glib::timeout_add_seconds_local(4, || {
        watch_current();
        glib::ControlFlow::Continue
    });
    // Build the other pages a little at a time while nothing else is happening,
    // so the first search finds everything without a pause.
    prebuild_after(std::time::Duration::from_millis(400));
    // The sidebar used what extensions said last time; ask again in case devices came or went.
    crate::cmd::background(crate::ext::refresh_pages, |changed| {
        if changed {
            reload_sections(false);
        }
    });
}

fn build(app: &gtk::Application) {
    crate::capability::start();
    let window =
        gtk::ApplicationWindow::builder().application(app).title("Settings").default_width(1120).default_height(800).build();
    window.add_css_class("settings-window");
    // No client-side titlebar: Hyprland manages the window.
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    nyx_ui::keys::install(&window, nyx_ui::keys::Common { shortcuts: Some(show_shortcuts), ..Default::default() });
    window.set_icon_name(Some("io.github.design_nexus.nyx.Settings"));

    let sections = sections::all();

    // ----- Sidebar -----
    let nav = gtk::Box::new(gtk::Orientation::Vertical, 0);
    nav.add_css_class("settings-navigation");
    // Labels inside expand; don't let that widen the sidebar itself.
    nav.set_hexpand(false);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let (categories, page_group) = fill_nav(&list, &sections, false);
    let nav_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        // Scrolls with wheel, trackpad and keyboard; no visible scrollbar.
        .vscrollbar_policy(gtk::PolicyType::External)
        .vexpand(true)
        .child(&list)
        .build();
    nav.append(&nav_scroll);

    // ----- Content -----
    let stack = gtk::Stack::new();
    stack.add_css_class("settings-content");
    stack.set_hexpand(true);
    // Sized by the page shown: measuring every built page on each search keystroke is slow.
    stack.set_hhomogeneous(false);
    stack.set_vhomogeneous(false);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(if prefs::get().reduce_motion { 0 } else { 160 });
    let (empty, empty_label) = search_empty_page();
    stack.add_named(&empty, Some(EMPTY_PAGE));

    let head = page_head();
    let main = widgets::vbox(0);
    main.append(&head.root);
    main.append(&stack);

    // ----- Header: the app, the sidebar toggle and search -----
    let header = nyx_ui::chrome::app_header(&window, "preferences-system-symbolic", "Settings");
    let collapse_button = widgets::bar_button("sidebar-show-symbolic", "Collapse the sidebar (Ctrl+B)");
    collapse_button.connect_clicked(|_| toggle_sidebar());
    header.start.append(&collapse_button);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search all settings  (Ctrl+F)"));
    search.add_css_class("bar-search");
    search.set_width_chars(34);
    header.center.append(&search);

    let frame = nyx_ui::frame::AppFrame::new(&header.root);
    frame.set_sidebar(&nav);
    frame.set_page(&main);
    // The page head's line reaches the sheet's sides; pages keep their own padding.
    frame.set_flush(true);
    let overlay = frame.install(&window);

    // ----- Keys -----
    let keys = gtk::EventControllerKey::new();
    let s2 = search.clone();
    let w2 = window.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        match key {
            gdk::Key::f if ctrl => {
                focus_search(&s2);
                glib::Propagation::Stop
            }
            gdk::Key::F1 => {
                show_shortcuts();
                glib::Propagation::Stop
            }
            gdk::Key::b if ctrl => {
                toggle_sidebar();
                glib::Propagation::Stop
            }
            gdk::Key::Left if mods.contains(gdk::ModifierType::ALT_MASK) => {
                go_back();
                glib::Propagation::Stop
            }
            gdk::Key::Right if mods.contains(gdk::ModifierType::ALT_MASK) => {
                go_forward();
                glib::Propagation::Stop
            }
            gdk::Key::q | gdk::Key::w if ctrl => {
                w2.close();
                glib::Propagation::Stop
            }
            gdk::Key::Escape if s2.is_visible() => {
                close_search(&s2);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    window.add_controller(keys);
    // The mouse's back and forward buttons.
    let side = gtk::GestureClick::new();
    side.set_button(0);
    side.connect_pressed(|g, _, _, _| match g.current_button() {
        8 => go_back(),
        9 => go_forward(),
        _ => {}
    });
    window.add_controller(side);
    search.connect_search_changed(|e| filter(&e.text()));
    let arrows = gtk::EventControllerKey::new();
    arrows.connect_key_pressed(|_, key, _, _| match key {
        gdk::Key::Down if step_hit(true) => glib::Propagation::Stop,
        gdk::Key::Up if step_hit(false) => glib::Propagation::Stop,
        _ => glib::Propagation::Proceed,
    });
    // Before the entry's text handling sees them.
    arrows.set_propagation_phase(gtk::PropagationPhase::Capture);
    search.add_controller(arrows);
    search.set_tooltip_text(Some("Up and Down go through the results; Enter goes to the one shown"));
    search.connect_activate(|_| focus_first_hit());

    // Narrow windows (a tiled half-screen) get an icon-only sidebar.
    window.connect_default_width_notify(apply_width);
    window.connect_realize(apply_width);
    // Tiled windows are resized by the compositor: an invisible layer over the
    // whole window reports each real size change.
    let probe = gtk::DrawingArea::new();
    probe.set_can_target(false);
    {
        let w2 = window.clone();
        probe.connect_resize(move |_, _, _| {
            let w2 = w2.clone();
            // After this layout pass, when the window's width is the new one.
            glib::idle_add_local_once(move || apply_width(&w2));
        });
    }
    overlay.add_overlay(&probe);

    window.connect_close_request(|_| {
        crate::backend::store::flush();
        glib::Propagation::Proceed
    });

    let ui = Ui {
        window,
        stack,
        nav: nav.clone(),
        nav_list: list,
        categories,
        page_group,
        compact: std::cell::Cell::new(false),
        pages: HashMap::new(),
        sections,
        current: "",
        overlay,
        collapse_button,
        head,
        empty_label,
        toast: None,
    };
    UI.with(|u| *u.borrow_mut() = Some(Rc::new(RefCell::new(ui))));
}

type NavParts = (Vec<Category>, HashMap<&'static str, &'static str>);

thread_local! {
    /// Counts shown after a page's name, e.g. updates on Home: on its tab, and on its
    /// section's row in the sidebar.
    static NAV_BADGES: RefCell<HashMap<&'static str, String>> = RefCell::default();
    /// The pages that match the search, while searching.
    static MATCHING: RefCell<Option<HashSet<&'static str>>> = const { RefCell::new(None) };
    /// The page last shown in each section, which the section's row opens again.
    static LAST_IN: RefCell<HashMap<&'static str, &'static str>> = RefCell::default();
}

fn nav_badge(text: &str) -> gtk::Label {
    let t = widgets::tag(text);
    t.add_css_class("nav-badge");
    t
}

fn search_count(n: usize, tooltip: &str) -> gtk::Label {
    let t = widgets::tag(&n.to_string());
    t.add_css_class("search-count");
    t.set_tooltip_text(Some(tooltip));
    t
}

/// Show `text` after a page's name on its tab and its section's row, or nothing.
pub fn set_nav_badge(id: &'static str, text: Option<&str>) {
    let changed = NAV_BADGES.with(|b| match text {
        Some(t) => b.borrow_mut().insert(id, t.to_string()).as_deref() != Some(t),
        None => b.borrow_mut().remove(id).is_some(),
    });
    if changed {
        refill_nav();
    }
}

/// The theme's name for a section's colour (`[sections.settings]`): Home's pages
/// have no group.
fn section_key(group: &str) -> String {
    match crate::platform::colour_group(group) {
        "" => "home".into(),
        "Assistant & AI" => "assistant".into(),
        g => g.to_lowercase().replace(' ', "-"),
    }
}

/// The icon on a section's tile: its own, or else its first page's.
fn category_icon(group: &str, first_page: &'static str) -> &'static str {
    match group {
        "" => "user-home-symbolic",
        "Connections" | "Network" => "network-wireless-symbolic",
        "Hardware" | "Displays" => "video-display-symbolic",
        "Personalization" | "Appearance" => "preferences-desktop-appearance-symbolic",
        "Desktop" => "view-grid-symbolic",
        "Input" => "input-keyboard-symbolic",
        "Accounts" => "avatar-default-symbolic",
        "Assistant & AI" => "settings-sparkle-symbolic",
        "Security" => "security-high-symbolic",
        "Apps & Defaults" => "view-app-grid-symbolic",
        "Storage & Backup" => "drive-harddisk-symbolic",
        "System" => "emblem-system-symbolic",
        _ => first_page,
    }
}

/// The sidebar's rows: Home, a line, then a row per section with its tile in the
/// section's colour, its name and its first pages. While searching only sections
/// with a matching page show, with how many settings match.
fn fill_nav(list: &gtk::Box, sections: &[Section], compact: bool) -> NavParts {
    let mut groups: Vec<(&'static str, Vec<&Section>)> = Vec::new();
    let mut page_group = HashMap::new();
    for s in sections {
        if !s.shown() || crate::platform::off_sidebar(s.id) {
            continue;
        }
        page_group.insert(s.id, s.group);
        match groups.iter_mut().find(|(g, _)| *g == s.group) {
            Some((_, pages)) => pages.push(s),
            None => groups.push((s.group, vec![s])),
        }
    }
    let matching = MATCHING.with(|m| m.borrow().clone());
    let counts = COUNTS.with(|c| c.borrow().clone());
    let mut categories = Vec::new();
    for (group, pages) in groups {
        let ids: Vec<&'static str> = pages.iter().map(|s| s.id).collect();
        let shown = matching.as_ref().is_none_or(|m| ids.iter().any(|id| m.contains(id)));
        let count = counts.as_ref().map(|c| ids.iter().filter_map(|id| c.get(id)).sum::<usize>());
        let button = category_row(group, &pages, compact, count);
        button.set_visible(shown);
        list.append(&button);
        if group.is_empty() {
            let line = gtk::Separator::new(gtk::Orientation::Horizontal);
            line.add_css_class("nav-split");
            line.set_visible(shown);
            list.append(&line);
        }
        categories.push(Category { group, button, pages: ids });
    }
    (categories, page_group)
}

/// A section's row. `count` is how many of its settings match, while searching:
/// after its name, or in its tooltip in the icon-only sidebar.
fn category_row(group: &'static str, pages: &[&Section], compact: bool, count: Option<usize>) -> gtk::Button {
    let name = if group.is_empty() { "Home" } else { group };
    let button = gtk::Button::new();
    button.add_css_class("nav-item");
    button.add_css_class("nav-category");
    let content = widgets::hbox(12);
    content.set_halign(if compact { gtk::Align::Center } else { gtk::Align::Fill });
    let tile = widgets::hbox(0);
    tile.add_css_class("category-tile");
    tile.add_css_class(&format!("section-{}", section_key(group)));
    tile.set_valign(gtk::Align::Center);
    tile.set_hexpand(false);
    let icon = gtk::Image::from_icon_name(category_icon(group, pages.first().map_or("", |s| s.icon)));
    icon.set_hexpand(true);
    icon.set_halign(gtk::Align::Center);
    tile.append(&icon);
    content.append(&tile);
    let titles: Vec<&str> = pages.iter().map(|s| s.title).collect();
    if !compact {
        let text = widgets::vbox(1);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        let summary =
            if group.is_empty() { "This computer and updates".to_string() } else { titles[..titles.len().min(3)].join(", ") };
        for (t, class) in [(name, "category-name"), (summary.as_str(), "category-pages")] {
            let l = widgets::label(t, class);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            // As wide as the sidebar, which doesn't grow to fit it.
            l.set_max_width_chars(1);
            text.append(&l);
        }
        content.append(&text);
        let badge = pages.iter().find_map(|s| NAV_BADGES.with(|b| b.borrow().get(s.id).cloned()));
        match (count, badge) {
            (Some(n), _) if n > 0 => content.append(&search_count(n, "Settings in this section that match")),
            (None, Some(text)) => content.append(&nav_badge(&text)),
            _ => {}
        }
    }
    button.set_child(Some(&content));
    let tip = match count {
        Some(n) if compact => format!("{name}: {n} matching {}", if n == 1 { "setting" } else { "settings" }),
        _ => format!("{name}\n{}", titles.join(", ")),
    };
    button.set_tooltip_text(Some(&tip));
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    button.connect_clicked(move |_| open_category(group));
    button
}

/// Open a section: the page last shown in it, or else its first (its first matching
/// one while searching).
fn open_category(group: &'static str) {
    let Some(ui) = ui() else { return };
    let target = {
        let u = ui.borrow();
        let Some(c) = u.categories.iter().find(|c| c.group == group) else { return };
        let matching = MATCHING.with(|m| m.borrow().clone());
        let ok = |id: &&'static str| matching.as_ref().is_none_or(|m| m.contains(id));
        let last = LAST_IN.with(|l| l.borrow().get(group).copied()).filter(|id| c.pages.contains(id) && ok(id));
        last.or_else(|| c.pages.iter().find(|id| ok(id)).copied())
    };
    if let Some(id) = target {
        navigate(id);
    }
}

/// The page head for the page shown: its section, name, config files and tabs.
fn show_head(u: &Ui) {
    let Some(s) = u.sections.iter().find(|s| s.id == u.current) else { return };
    let head = &u.head;
    let section = format!("section-{}", section_key(s.group));
    let old = head.section.replace(section.clone());
    if old != section {
        if !old.is_empty() {
            head.kicker.remove_css_class(&old);
        }
        head.kicker.add_css_class(&section);
    }
    head.kicker.set_text(&s.group.to_uppercase());
    head.kicker.set_visible(!s.group.is_empty());
    head.title.set_text(s.title);
    head.about.set_text(s.description);
    head.about.set_tooltip_text(Some(s.description));
    while let Some(c) = head.edit.first_child() {
        head.edit.remove(&c);
    }
    let files = s.config_files();
    if !files.is_empty() {
        head.edit.append(&widgets::open_config_button(&files));
    }
    fill_tabs(u);
}

/// The section's pages as tabs over the page (only the matching ones while
/// searching), each with how many of its settings match, or its badge.
fn fill_tabs(u: &Ui) {
    let head = &u.head;
    while let Some(c) = head.tabs.first_child() {
        head.tabs.remove(&c);
    }
    let Some(group) = u.sections.iter().find(|s| s.id == u.current).map(|s| s.group) else { return };
    head.tabs_scroll.set_visible(!group.is_empty());
    let matching = MATCHING.with(|m| m.borrow().clone());
    let counts = COUNTS.with(|c| c.borrow().clone());
    let section = head.section.borrow().clone();
    let mut active = None;
    for s in u.sections.iter().filter(|s| u.page_group.get(s.id) == Some(&group)) {
        if s.id != u.current && matching.as_ref().is_some_and(|m| !m.contains(s.id)) {
            continue;
        }
        let b = gtk::Button::new();
        b.add_css_class("page-tab");
        let col = widgets::vbox(0);
        let row = widgets::hbox(6);
        row.add_css_class("tab-label");
        row.append(&gtk::Label::new(Some(s.title)));
        let badge = NAV_BADGES.with(|b| b.borrow().get(s.id).cloned());
        match (counts.as_ref().and_then(|c| c.get(s.id)), badge) {
            (Some(n), _) => row.append(&search_count(*n, "Settings on this page that match")),
            (None, Some(text)) if counts.is_none() => row.append(&nav_badge(&text)),
            _ => {}
        }
        col.append(&row);
        let line = widgets::hbox(0);
        line.add_css_class("tab-line");
        line.add_css_class(&section);
        col.append(&line);
        b.set_child(Some(&col));
        b.update_property(&[gtk::accessible::Property::Label(s.title)]);
        match s.visibility() {
            Visibility::Disabled(why) => {
                b.add_css_class("nav-unavailable");
                b.set_tooltip_text(Some(&format!("{}\n{why}", s.description)));
            }
            _ => b.set_tooltip_text(Some(s.description)),
        }
        if s.id == u.current {
            b.add_css_class("active");
            active = Some(b.clone());
        }
        let id = s.id;
        // After this click is handled: going there redraws the tabs.
        b.connect_clicked(move |_| {
            glib::idle_add_local_once(move || navigate(id));
        });
        head.tabs.append(&b);
    }
    if let Some(b) = active {
        let (scroll, tabs) = (head.tabs_scroll.clone(), head.tabs.clone());
        glib::idle_add_local_once(move || {
            let Some(bounds) = b.compute_bounds(&tabs) else { return };
            let adj = scroll.hadjustment();
            let (x, w) = (f64::from(bounds.x()), f64::from(bounds.width()));
            if x < adj.value() {
                adj.set_value(x);
            } else if x + w > adj.value() + adj.page_size() {
                adj.set_value(x + w - adj.page_size());
            }
        });
    }
}

/// What to do with a finished update check.
pub fn after_update_check(s: crate::backend::updates::Status) {
    if s.settings_update() && prefs::get().announced_version != s.latest {
        let latest = s.latest.clone();
        if crate::backend::updates::self_updatable() {
            toast_action(&format!("Settings {latest} is available"), "Update", move || {
                let latest = latest.clone();
                toast("Updating Settings…");
                crate::cmd::background(
                    || crate::backend::updates::update_settings().map_err(|e| format!("{e:#}")),
                    move |r| match r {
                        Ok(()) => {
                            let here = ui().map(|u| u.borrow().current).unwrap_or("about");
                            toast_action(&format!("Settings {latest} is installed"), "Restart", move || restart(here));
                            rebuild_if_built("about");
                        }
                        Err(e) => toast(&format!("Couldn't update: {e}")),
                    },
                );
            });
        } else {
            toast_action(&format!("Settings {latest} is available"), "Details", || navigate("about"));
        }
        prefs::update(|p| p.announced_version = s.latest.clone());
    }
    if prefs::get().auto_update_extensions && !s.extensions.is_empty() {
        crate::cmd::background(
            || crate::backend::updates::update_extensions().map_err(|e| format!("{e:#}")),
            |r| match r {
                Ok(names) if !names.is_empty() => {
                    toast(&format!("Updated {}", names.join(", ")));
                    reload_sections(false);
                }
                Ok(_) => {}
                Err(e) => toast(&format!("Couldn't update extensions: {e}")),
            },
        );
    }
    rebuild_if_built("about");
    rebuild_if_built("extensions");
}

/// Start the installed Settings again on `section`, and quit this one.
pub fn restart(section: &str) {
    use std::os::unix::process::CommandExt;
    let exe = crate::paths::home().join(".local/bin/nyx-settings");
    let script = format!("sleep 1; exec '{}' --section '{}'", exe.display(), section.replace('\'', ""));
    let started = std::process::Command::new("sh")
        .args(["-c", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
    if let Err(e) = started {
        toast(&format!("Couldn't restart: {e}"));
        return;
    }
    crate::backend::store::flush();
    if let Some(app) = window().and_then(|w| w.application()) {
        app.quit();
    }
}

/// Re-read the list of pages (after an extension is installed, updated or removed,
/// or reports different hardware). Extension pages are built again when next shown.
/// `fresh` asks every extension again instead of using what it said last time.
pub fn reload_sections(fresh: bool) {
    let Some(ui) = ui() else { return };
    let sections = sections::all_with(fresh);
    let mut u = ui.borrow_mut();
    while let Some(c) = u.nav_list.first_child() {
        u.nav_list.remove(&c);
    }
    let (categories, page_group) = fill_nav(&u.nav_list, &sections, u.compact.get());
    // Drop built extension pages (their page may have changed) and pages that are gone.
    let stale: Vec<&'static str> = u
        .pages
        .keys()
        .copied()
        .filter(|id| sections.iter().find(|s| s.id == *id).is_none_or(|s| matches!(s.build, sections::Build::Extension(_))))
        .collect();
    for id in &stale {
        PAGE_BODIES.with(|b| b.borrow_mut().remove(id));
        if let Some(p) = u.pages.remove(id) {
            SEARCH.with(|s| s.borrow_mut().retain(|item| item.section != *id));
            u.stack.remove(&p);
        }
    }
    u.categories = categories;
    u.page_group = page_group;
    u.sections = sections;
    let current = u.current;
    u.current = "";
    drop(u);
    navigate(current);
}

thread_local! {
    static NARROW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

thread_local! {
    static SIZED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Below this the sidebar always shows only the sections' tiles. Above it, only when
/// the page wouldn't fit beside the full sidebar (half a 1920-pixel screen keeps the
/// names).
const NARROW_BELOW: i32 = 860;
/// The full sidebar's width, a little over what it measures.
const SIDEBAR_WIDTH: i32 = 236;
/// How much wider a page gets when it leaves the narrow padding (20 px a side more).
const NARROW_PADDING_SAVING: i32 = 40;

/// Narrow when the window is small, or when the full sidebar and the page shown
/// wouldn't both fit: then the page would be cut off at the window's edge.
fn apply_width(w: &gtk::ApplicationWindow) {
    let width = if w.width() > 0 { w.width() } else { w.default_width() };
    if width <= 0 {
        return;
    }
    let was = NARROW.with(|n| n.get());
    let page_min = ui()
        .and_then(|u| {
            let u = u.try_borrow().ok()?;
            u.pages.get(u.current).map(|p| p.measure(gtk::Orientation::Horizontal, -1).0)
        })
        .unwrap_or(0);
    // Measured as it is now; leaving narrow mode adds padding, so allow for it.
    let need = SIDEBAR_WIDTH + page_min + if was { NARROW_PADDING_SAVING } else { 0 };
    let narrow = width < NARROW_BELOW || width < need;
    if narrow == was && SIZED.with(|s| s.get()) {
        return;
    }
    SIZED.with(|s| s.set(true));
    set_narrow(narrow);
    apply_compact(narrow || prefs::get().sidebar_collapsed);
}

/// Every keyboard shortcut, for the shortcuts dialog.
pub const SHORTCUTS: &[(&[&str], &str)] = &[
    (&["Ctrl", "F"], "Search settings"),
    (&["↑ / ↓"], "Previous or next result, while searching"),
    (&["Enter"], "Go to the result shown"),
    (&["Esc"], "Clear the search"),
    (&["Alt", "←"], "Back to the page before"),
    (&["Alt", "→"], "Forward again"),
    (&["Ctrl", "B"], "Collapse or expand the sidebar"),
    (&["F1"], "Show these shortcuts (or ?)"),
    (&["Ctrl", "Q"], "Close (Ctrl+W too)"),
];

pub fn show_shortcuts() {
    let Some(win) = window() else { return };
    let dialog = gtk::Window::builder().transient_for(&win).modal(true).title("Keyboard shortcuts").default_width(520).build();
    dialog.add_css_class("settings-window");
    dialog.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    let card = widgets::vbox(12);
    card.add_css_class("dialog-card");
    card.append(&widgets::label("Keyboard shortcuts", "section-title"));
    dialog.set_child(Some(&card));
    let keys = gtk::EventControllerKey::new();
    let d = dialog.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            d.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    dialog.add_controller(keys);
    let list = widgets::vbox(0);
    list.add_css_class("settings-card");
    for (keys, what) in SHORTCUTS {
        let caps = widgets::hbox(4);
        for (i, k) in keys.iter().enumerate() {
            if i > 0 {
                caps.append(&widgets::label("+", "dim"));
            }
            caps.append(&widgets::label(k, "key-cap"));
        }
        list.append(&widgets::row(what, "", Some(caps.upcast_ref())));
    }
    widgets::mark_first_rows(list.upcast_ref());
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        // Scrolls without a scrollbar, which would cover the key caps.
        .vscrollbar_policy(gtk::PolicyType::External)
        .propagate_natural_height(true)
        .max_content_height(560)
        .child(&list)
        .build();
    card.append(&scroll);
    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    card.append(&close);
    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    let weak = dialog.downgrade();
    close.connect_clicked(move |_| {
        if let Some(dialog) = weak.upgrade() {
            dialog.close();
        }
    });
    card.append(&close);
    dialog.present();
}

/// The sidebar shows only the sections' tiles.
fn apply_compact(compact: bool) {
    let Some(ui) = ui() else { return };
    {
        let u = ui.borrow();
        if compact {
            u.nav.add_css_class("compact");
        } else {
            u.nav.remove_css_class("compact");
        }
        u.compact.set(compact);
        u.collapse_button.set_tooltip_text(Some(if compact {
            "Expand the sidebar (Ctrl+B)"
        } else {
            "Collapse the sidebar (Ctrl+B)"
        }));
    }
    refill_nav();
}

pub fn toggle_sidebar() {
    prefs::update(|p| p.sidebar_collapsed = !p.sidebar_collapsed);
    apply_compact(NARROW.with(|n| n.get()) || prefs::get().sidebar_collapsed);
}

fn set_narrow(narrow: bool) {
    NARROW.with(|n| n.set(narrow));
    let Some(ui) = ui() else { return };
    let u = ui.borrow();
    for page in u.pages.values() {
        mark_page(page, narrow);
    }
    if narrow {
        u.head.root.add_css_class("narrow");
    } else {
        u.head.root.remove_css_class("narrow");
    }
    u.head.about.set_visible(!narrow);
}

fn mark_page(page: &gtk::ScrolledWindow, narrow: bool) {
    // ScrolledWindow > Viewport > Clamp > page body.
    if let Some(body) = page.child().and_then(|v| v.first_child()).and_then(|c| c.first_child()) {
        if narrow {
            body.add_css_class("narrow");
        } else {
            body.remove_css_class("narrow");
        }
    }
}

thread_local! {
    /// The last state seen for each watched page, and whether a check is running.
    static SEEN: RefCell<HashMap<&'static str, String>> = RefCell::default();
    static CHECKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Check the page on screen, if it's one that shows live state; redraw it if that changed.
fn watch_current() {
    let Some(ui) = ui() else { return };
    let id = ui.borrow().current;
    // Pages whose changes are announced are checked when they are.
    if crate::live::sources_for(id).is_none() {
        check_live(id);
    }
}

thread_local! {
    static LIVE_WATCHED: RefCell<std::collections::HashSet<&'static str>> = RefCell::new(Default::default());
}

/// From the first time a live page is shown, check it whenever what it shows is
/// announced to have changed.
fn watch_live(id: &'static str) {
    let Some(sources) = crate::live::sources_for(id) else { return };
    if !LIVE_WATCHED.with(|w| w.borrow_mut().insert(id)) {
        return;
    }
    let (tx, rx) = async_channel::unbounded::<()>();
    nyx_capability::watch(sources, move || {
        let _ = tx.send_blocking(());
    });
    glib::spawn_future_local(async move {
        while rx.recv().await.is_ok() {
            check_live(id);
        }
    });
}

/// Redraw `id` if it's the page shown and what it shows has changed.
fn check_live(id: &'static str) {
    let Some(ui) = ui() else { return };
    let (current, visible) = {
        let u = ui.borrow();
        (u.current, u.window.is_visible() && u.window.is_active())
    };
    if current != id {
        return;
    }
    // Only for the window in use, and one check at a time.
    let Some(signature) = crate::live::signature_for(id) else { return };
    if !visible || CHECKING.with(|c| c.replace(true)) {
        return;
    }
    crate::cmd::background(signature, move |now| {
        CHECKING.with(|c| c.set(false));
        let before = SEEN.with(|s| s.borrow_mut().insert(id, now.clone()));
        let still_here = self::ui().is_some_and(|u| u.borrow().current == id);
        if before.is_some_and(|b| b != now) && still_here && !busy_on_page() {
            rebuild(id);
        }
    });
}

/// The user is typing in a field or has a menu open: don't redraw under them.
fn busy_on_page() -> bool {
    let Some(focus) = window().and_then(|w| gtk::prelude::GtkWindowExt::focus(&w)) else { return false };
    std::iter::successors(Some(focus), |w| w.parent()).any(|w| w.is::<gtk::Text>() || w.is::<gtk::Popover>())
}

/// Build one page that hasn't been yet, then come back for the next.
fn prebuild_next() {
    if let Some(id) = next_unbuilt() {
        ensure_built(id);
        prebuild_after(std::time::Duration::from_millis(60));
    }
}

/// After `wait`, once input and redraws are done, build the next page.
fn prebuild_after(wait: std::time::Duration) {
    glib::timeout_add_local_full(wait, glib::Priority::LOW, || {
        prebuild_next();
        glib::ControlFlow::Break
    });
}

thread_local! {
    static BUILT_ONCE: RefCell<std::collections::HashSet<&'static str>> = RefCell::default();
}

fn ensure_built(id: &'static str) {
    let Some(ui) = ui() else { return };
    if ui.borrow().pages.contains_key(id) {
        return;
    }
    let section = {
        let u = ui.borrow();
        u.sections.iter().find(|s| s.id == id).cloned()
    };
    let Some(section) = section else { return };
    let sid = section.id;
    let page = widgets::page(sid);
    // Only a page's first build may use answers fetched ahead; a redraw asks again.
    if BUILT_ONCE.with(|b| b.borrow_mut().insert(sid)) {
        crate::cmd::building(|| section.run(&page));
    } else {
        section.run(&page);
    }
    mark_page(&page.root, NARROW.with(|n| n.get()));
    let stack = ui.borrow().stack.clone();
    stack.add_named(&page.root, Some(sid));
    ui.borrow_mut().pages.insert(sid, page.root);
    PAGE_BODIES.with(|b| b.borrow_mut().insert(sid, (page.body, None)));
    show_availability(&section);
}

thread_local! {
    /// Each built page's body, and the banner saying why it can't be used, if it can't.
    static PAGE_BODIES: RefCell<HashMap<&'static str, (gtk::Box, Option<gtk::Box>)>> = RefCell::new(HashMap::new());
}

/// A page whose capability is unavailable says why at the top, and its controls can't be
/// used; one whose capability went while it was open stays, saying so, with its settings
/// kept. Shown normally again when the capability comes back.
fn show_availability(s: &Section) {
    PAGE_BODIES.with(|b| {
        let mut b = b.borrow_mut();
        let Some((body, banner)) = b.get_mut(s.id) else { return };
        if let Some(old) = banner.take() {
            body.remove(&old);
        }
        let visibility = s.visibility();
        let text = match &visibility {
            Visibility::Shown => None,
            Visibility::Disabled(why) => Some(format!("{} can't be used right now. {why}.", s.title)),
            Visibility::Hidden => Some(match crate::capability::why_hidden(s.needs) {
                Some(why) => format!("{} isn't available any more. {why}. Its settings are kept.", s.title),
                None => format!("{} isn't available any more. Its settings are kept.", s.title),
            }),
        };
        let mut child = body.first_child();
        while let Some(c) = child {
            c.set_sensitive(visibility == Visibility::Shown);
            child = c.next_sibling();
        }
        if let Some(text) = text {
            let new = widgets::banner(&text, true);
            body.prepend(&new);
            *banner = Some(new);
        }
    });
}

/// Something this computer can do came or went: the sidebar, the tabs and the pages
/// that depend on it follow. The page shown stays shown (saying why) even if its
/// section's row went.
pub fn capabilities_changed(changes: &[nyx_capability::Change]) {
    let Some(ui) = ui() else { return };
    let affected: Vec<Section> = {
        let u = ui.borrow();
        u.sections.iter().filter(|s| s.needs.iter().any(|n| changes.iter().any(|c| c.id == *n))).cloned().collect()
    };
    if affected.is_empty() {
        return;
    }
    refill_nav();
    for s in &affected {
        show_availability(s);
    }
}

/// Fill the sidebar and the tabs again from the sections as they are, keeping the
/// pages built and the one shown.
fn refill_nav() {
    let Some(ui) = ui() else { return };
    let mut u = ui.borrow_mut();
    while let Some(c) = u.nav_list.first_child() {
        u.nav_list.remove(&c);
    }
    let sections = u.sections.clone();
    let (categories, page_group) = fill_nav(&u.nav_list, &sections, u.compact.get());
    u.categories = categories;
    u.page_group = page_group;
    mark_category(&u);
    fill_tabs(&u);
}

/// Mark the row of the page shown's section.
fn mark_category(u: &Ui) {
    let group = u.sections.iter().find(|s| s.id == u.current).map(|s| s.group);
    for c in &u.categories {
        if Some(c.group) == group {
            c.button.add_css_class("active");
        } else {
            c.button.remove_css_class("active");
        }
    }
}

thread_local! {
    /// Pages to go back and forward to (Alt+Left / Alt+Right, the mouse's side buttons).
    static BACK: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    static FORWARD: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    /// Set while going through history, or while search moves between pages.
    static NO_HISTORY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn go_back() {
    step_history(true);
}

pub fn go_forward() {
    step_history(false);
}

fn step_history(back: bool) {
    let Some(current) = ui().map(|u| u.borrow().current) else { return };
    let (from, to) = if back { (&BACK, &FORWARD) } else { (&FORWARD, &BACK) };
    let Some(id) = from.with(|h| h.borrow_mut().pop()) else { return };
    if !current.is_empty() {
        to.with(|h| h.borrow_mut().push(current));
    }
    NO_HISTORY.with(|n| n.set(true));
    navigate(id);
    NO_HISTORY.with(|n| n.set(false));
}

pub fn navigate(id: &str) {
    let Some(ui) = ui() else { return };
    // Pages that were renamed or split: the old ids still work.
    let id = match id {
        "connectivity" => "wifi",
        other => other,
    };
    let resolved = {
        let u = ui.borrow();
        let shown = |s: &&Section| s.shown();
        u.sections
            .iter()
            .filter(shown)
            .find(|s| s.id == id)
            // An extension page asked for by its own id (`--section aura` finds `asus.aura`).
            .or_else(|| u.sections.iter().filter(shown).find(|s| s.id.split_once('.').is_some_and(|(_, p)| p == id)))
            // An extension's own id opens its first page (`--section webcam`).
            .or_else(|| u.sections.iter().filter(shown).find(|s| s.id.split_once('.').is_some_and(|(e, _)| e == id)))
            .or_else(|| u.sections.first())
            .map(|s| s.id)
    };
    let Some(id) = resolved else { return };
    ensure_built(id);
    let previous = ui.borrow().current;
    if !previous.is_empty() && previous != id && !NO_HISTORY.with(|n| n.get()) {
        BACK.with(|h| {
            let mut h = h.borrow_mut();
            h.push(previous);
            // Plenty to go back through, without growing for ever.
            if h.len() > 50 {
                h.remove(0);
            }
        });
        FORWARD.with(|h| h.borrow_mut().clear());
    }
    let mut u = ui.borrow_mut();
    u.stack.set_visible_child_name(id);
    u.current = id;
    if let Some(group) = u.page_group.get(id).copied() {
        LAST_IN.with(|l| l.borrow_mut().insert(group, id));
    }
    mark_category(&u);
    show_head(&u);
    let window = u.window.clone();
    drop(u);
    watch_live(id);
    prefs::update(|p| p.last_section = id.to_string());
    // A wider page may not fit beside the full sidebar.
    glib::idle_add_local_once(move || apply_width(&window));
}

/// Rebuild a section page from scratch (after a change that alters its layout).
pub fn rebuild(id: &'static str) {
    let Some(ui) = ui() else { return };
    let old = ui.borrow_mut().pages.remove(id);
    // Where the page was scrolled to, so redrawing it doesn't jump back to the top.
    let scrolled = old.as_ref().map(|p| p.vadjustment().value()).unwrap_or(0.0);
    if let Some(old) = old {
        SEARCH.with(|s| s.borrow_mut().retain(|item| item.section != id));
        ui.borrow().stack.remove(&old);
    }
    let current = ui.borrow().current;
    ensure_built(id);
    if current == id {
        ui.borrow().stack.set_visible_child_name(id);
    }
    if scrolled > 0.0
        && let Some(page) = ui.borrow().pages.get(id).cloned()
    {
        // Once laid out: before that the page has no height to scroll through.
        let adj = page.vadjustment();
        // The adjustment keeps the value within the page's height.
        let restore = adj.connect_changed(move |a| a.set_value(scrolled));
        // Only for the first layouts; later changes are the page's own.
        let adj2 = adj.clone();
        let restore = std::cell::Cell::new(Some(restore));
        glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
            if let Some(h) = restore.take() {
                adj2.disconnect(h);
            }
        });
    }
}

/// Rebuild a page only if it has been opened already (its content is stale otherwise).
pub fn rebuild_if_built(id: &'static str) {
    let built = ui().is_some_and(|u| u.borrow().pages.contains_key(id));
    if built {
        rebuild(id);
    }
}

thread_local! {
    /// Search results in page order, and which one Up / Down is on.
    static HITS: RefCell<Vec<(&'static str, gtk::Widget)>> = const { RefCell::new(Vec::new()) };
    static HIT_AT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// What's being searched for, while pages not built yet are built for it.
    static QUERY: RefCell<String> = const { RefCell::new(String::new()) };
    static CATCHING_UP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The first page in the sidebar that hasn't been built.
fn next_unbuilt() -> Option<&'static str> {
    let ui = ui()?;
    let u = ui.borrow();
    u.sections.iter().filter(|s| s.shown()).map(|s| s.id).find(|id| !u.pages.contains_key(id))
}

fn filter(query: &str) {
    QUERY.with(|q| *q.borrow_mut() = query.to_string());
    search_for(query, true);
    if !crate::search::terms(query).is_empty() && next_unbuilt().is_some() && !CATCHING_UP.with(|c| c.replace(true)) {
        glib::idle_add_local_full(glib::Priority::LOW, || {
            search_catch_up();
            glib::ControlFlow::Break
        });
    }
}

/// While searching, build the pages that aren't yet, one at a time between keystrokes
/// and redraws, and add what each one matches.
fn search_catch_up() {
    let query = QUERY.with(|q| q.borrow().clone());
    let next = if crate::search::terms(&query).is_empty() { None } else { next_unbuilt() };
    let Some(id) = next else {
        CATCHING_UP.with(|c| c.set(false));
        return;
    };
    ensure_built(id);
    search_for(&query, false);
    glib::idle_add_local_full(glib::Priority::LOW, || {
        search_catch_up();
        glib::ControlFlow::Break
    });
}

/// Show only what matches `query` on the pages built so far. `fresh` is a new query:
/// it starts on the first result; otherwise the result Up / Down is on stays.
fn search_for(query: &str, fresh: bool) {
    let Some(ui) = ui() else { return };
    let terms = crate::search::terms(query);
    let kept = if fresh { None } else { HITS.with(|h| h.borrow().get(HIT_AT.with(|a| a.get())).map(|(_, r)| r.clone())) };

    let mut section_hits: HashMap<String, usize> = HashMap::new();
    let title_hits: Vec<&'static str> = {
        let u = ui.borrow();
        u.sections
            .iter()
            .filter(|s| u.page_group.contains_key(s.id))
            .filter(|s| {
                crate::search::matches(
                    &crate::search::normalise(&format!("{} {} {}", s.title, s.description, s.keywords)),
                    &terms,
                )
            })
            .map(|s| s.id)
            .collect()
    };

    let mut hit_rows: Vec<(String, gtk::Widget)> = Vec::new();
    SEARCH.with(|s| {
        let items = s.borrow();
        let mut groups_visible: HashMap<gtk::Widget, bool> = HashMap::new();
        let mut groups_hit: Vec<gtk::Widget> = Vec::new();
        for item in items.iter() {
            item.row.remove_css_class("search-hit");
            let hit = crate::search::matches(&item.text, &terms);
            let whole_section = title_hits.iter().any(|id| *id == item.section);
            let show = terms.is_empty() || hit || whole_section;
            item.row.set_visible(show);
            if hit {
                if !terms.is_empty() {
                    widgets::reveal_disclosure_ancestors(&item.row);
                }
                *section_hits.entry(item.section.clone()).or_default() += 1;
                hit_rows.push((item.section.clone(), item.row.clone()));
            }
            if let Some(g) = &item.group {
                let e = groups_visible.entry(g.clone()).or_insert(false);
                *e |= show;
                if hit && !groups_hit.contains(g) {
                    groups_hit.push(g.clone());
                }
            }
        }
        // Folded groups open while a row inside matches, and close again after.
        widgets::reveal_for_search(None);
        for g in &groups_hit {
            widgets::reveal_for_search(Some(g));
        }
        for (g, visible) in groups_visible {
            g.set_visible(visible);
            widgets::mark_first_rows(&g);
        }
    });

    let u = ui.borrow();
    // Results in the sidebar's order, for Up and Down.
    let mut ordered: Vec<(&'static str, gtk::Widget)> = Vec::new();
    // Pages not in the sidebar (their capability went) aren't results.
    for s in u.sections.iter().filter(|s| u.page_group.contains_key(s.id)) {
        ordered.extend(hit_rows.iter().filter(|(sec, _)| sec == s.id).map(|(_, r)| (s.id, r.clone())));
    }
    HITS.with(|h| *h.borrow_mut() = ordered);
    HIT_AT.with(|h| h.set(0));
    let counts =
        (!terms.is_empty()).then(|| u.sections.iter().filter_map(|s| section_hits.get(s.id).map(|n| (s.id, *n))).collect());
    COUNTS.with(|c| *c.borrow_mut() = counts);

    let matching: Option<HashSet<&'static str>> = (!terms.is_empty()).then(|| {
        u.sections
            .iter()
            .filter(|s| u.page_group.contains_key(s.id))
            .filter(|s| section_hits.contains_key(s.id) || title_hits.contains(&s.id))
            .map(|s| s.id)
            .collect()
    });
    let first_match = matching
        .as_ref()
        .and_then(|m| u.sections.iter().map(|s| s.id).find(|id| m.contains(id) && u.page_group.contains_key(id)));
    let current = u.current;
    let current_visible = u.page_group.contains_key(current) && matching.as_ref().is_none_or(|m| m.contains(current));
    MATCHING.with(|m| *m.borrow_mut() = matching);
    let was_empty = u.stack.visible_child_name().as_deref() == Some(EMPTY_PAGE);
    let nothing = !terms.is_empty() && first_match.is_none();
    if nothing {
        u.empty_label.set_text(&format!("No settings match “{}”", query.trim()));
        u.stack.set_visible_child_name(EMPTY_PAGE);
    } else if was_empty {
        u.stack.set_visible_child_name(current);
    }
    drop(u);
    refill_nav();
    // Results found later on other pages don't take you away from the one shown.
    let move_on = if fresh { !current_visible || !section_hits.contains_key(current) } else { !current_visible || was_empty };
    if let Some(first) = first_match
        && move_on
    {
        // Typing moves between pages; only where the search started goes in the history.
        NO_HISTORY.with(|n| n.set(true));
        navigate(first);
        NO_HISTORY.with(|n| n.set(false));
    }
    if let Some(at) = kept.and_then(|row| HITS.with(|h| h.borrow().iter().position(|(_, r)| *r == row))) {
        HIT_AT.with(|h| h.set(at));
        HITS.with(|h| h.borrow()[at].1.add_css_class("search-hit"));
    } else if !terms.is_empty() {
        // Start on the first result on the page shown.
        let current = ui.borrow().current;
        let at = HITS.with(|h| h.borrow().iter().position(|(s, _)| *s == current));
        if let Some(at) = at {
            show_hit(at);
        }
    }
}

thread_local! {
    /// Each page's number of results while searching; `None` when not.
    static COUNTS: RefCell<Option<HashMap<&'static str, usize>>> = const { RefCell::new(None) };
}

/// Mark result `at`, going to its page and scrolling to it.
fn show_hit(at: usize) {
    let Some((section, row)) = HITS.with(|h| h.borrow().get(at).cloned()) else { return };
    HIT_AT.with(|h| h.set(at));
    HITS.with(|h| {
        for (_, r) in h.borrow().iter() {
            r.remove_css_class("search-hit");
        }
    });
    let current = ui().map(|u| u.borrow().current);
    if current != Some(section) {
        NO_HISTORY.with(|n| n.set(true));
        navigate(section);
        NO_HISTORY.with(|n| n.set(false));
    }
    row.add_css_class("search-hit");
    scroll_to(&row);
}

/// Up / Down in the search box: the previous or next result, across pages.
fn step_hit(forward: bool) -> bool {
    let n = HITS.with(|h| h.borrow().len());
    if n == 0 {
        return false;
    }
    let at = HIT_AT.with(|h| h.get());
    show_hit(if forward { (at + 1) % n } else { (at + n - 1) % n });
    true
}

fn scroll_to(row: &gtk::Widget) {
    let Some(ui) = ui() else { return };
    let current = ui.borrow().current;
    let Some(page) = ui.borrow().pages.get(current).cloned() else { return };
    let row = row.clone();
    glib::idle_add_local_once(move || {
        if let Some(child) = page.child()
            && let Some(p) = row.compute_point(&child, &gtk::graphene::Point::new(0.0, 0.0))
        {
            let adj = page.vadjustment();
            // Leave room above for the group's title.
            adj.set_value((p.y() as f64 - 56.0).max(0.0));
        }
    });
}

fn focus_first_hit() {
    let row = HITS.with(|h| h.borrow().get(HIT_AT.with(|a| a.get())).map(|(_, r)| r.clone()));
    if let Some(row) = row {
        row.child_focus(gtk::DirectionType::TabForward);
    }
}

/// Show a short message at the bottom of the window. A new one replaces the last.
pub fn toast(message: &str) {
    show_toast(message, None);
}

/// A toast with a button (Undo, Update, Open…): clicking it runs `action` and
/// closes the toast. It stays up a little longer, to give time to reach it.
pub fn toast_action(message: &str, button: &str, action: impl Fn() + 'static) {
    show_toast(message, Some((button, Box::new(action))));
}

type ToastAction<'a> = Option<(&'a str, Box<dyn Fn()>)>;

fn show_toast(message: &str, action: ToastAction) {
    let Some(ui) = ui() else {
        eprintln!("nyx-settings: {message}");
        return;
    };
    let overlay = ui.borrow().overlay.clone();
    if let Some(old) = ui.borrow_mut().toast.take() {
        overlay.remove_overlay(&old);
    }
    let label = gtk::Label::new(Some(message));
    label.set_wrap(true);
    label.set_max_width_chars(70);
    let bx = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bx.add_css_class("toast");
    bx.append(&label);
    let motion = !prefs::get().reduce_motion;
    let has_action = action.is_some();
    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .transition_duration(if motion { 180 } else { 0 })
        .child(&bx)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::End)
        // Clicks pass through a plain toast; one with a button has to take them.
        .can_target(has_action)
        .build();
    if let Some((text, run)) = action {
        let b = gtk::Button::with_label(text);
        b.add_css_class("toast-action");
        b.set_valign(gtk::Align::Center);
        let r = revealer.clone();
        b.connect_clicked(move |_| {
            run();
            dismiss_toast(&r, motion);
        });
        bx.append(&b);
    }
    overlay.add_overlay(&revealer);
    revealer.set_reveal_child(true);
    ui.borrow_mut().toast = Some(revealer.clone());
    drop(ui);
    let shown = if has_action { 7000 } else { 3500 };
    glib::timeout_add_local_once(std::time::Duration::from_millis(shown), move || dismiss_toast(&revealer, motion));
}

fn dismiss_toast(revealer: &gtk::Revealer, motion: bool) {
    revealer.set_reveal_child(false);
    let revealer = revealer.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(if motion { 200 } else { 0 }), move || {
        let Some(u) = self::ui() else { return };
        // Only if it hasn't been replaced already.
        let current = u.borrow().toast.as_ref() == Some(&revealer);
        if current {
            let overlay = u.borrow().overlay.clone();
            u.borrow_mut().toast = None;
            overlay.remove_overlay(&revealer);
        }
    });
}

/// A page's title by its id, if it's in the sidebar.
pub fn section_title(id: &str) -> Option<&'static str> {
    ui().and_then(|u| u.borrow().sections.iter().find(|s| s.id == id).map(|s| s.title))
}

/// The sidebar's own (static) id for a page id.
pub fn section_id(id: &str) -> Option<&'static str> {
    ui().and_then(|u| u.borrow().sections.iter().find(|s| s.id == id).map(|s| s.id))
}

/// Find a Hyprland option's row: go to its page and mark it.
pub fn show_option(section: &str, title: &str) {
    navigate(section);
    let row = SEARCH.with(|s| {
        let t = crate::search::normalise(title);
        s.borrow().iter().find(|i| i.section == section && i.text.starts_with(&t)).map(|i| i.row.clone())
    });
    if let Some(row) = row {
        widgets::reveal_for_search(None);
        if let Some(g) = SEARCH.with(|s| s.borrow().iter().find(|i| i.row == row).and_then(|i| i.group.clone())) {
            widgets::reveal_for_search(Some(&g));
        }
        row.add_css_class("search-hit");
        scroll_to(&row);
        let r = row.clone();
        glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || r.remove_css_class("search-hit"));
    }
}

pub fn window() -> Option<gtk::ApplicationWindow> {
    ui().map(|u| u.borrow().window.clone())
}

fn snapshot_and_quit(app: &gtk::Application, out: std::path::PathBuf) {
    let Some(ui) = ui() else { return };
    let window = ui.borrow().window.clone();
    window.set_opacity(0.01);
    // A distinct title lets a window rule float it at a set size for screenshots.
    window.set_title(Some("Settings snapshot"));
    window.set_default_size(
        std::env::var("SETTINGS_SNAPSHOT_W").ok().and_then(|v| v.parse().ok()).unwrap_or(1120),
        std::env::var("SETTINGS_SNAPSHOT_H").ok().and_then(|v| v.parse().ok()).unwrap_or(820),
    );
    window.present();
    let app = app.clone();
    // SETTINGS_SNAPSHOT_SEARCH=words shows what a search for them finds.
    if let Ok(q) = std::env::var("SETTINGS_SNAPSHOT_SEARCH") {
        filter(&q);
        apply_compact(false);
    }
    let wait = std::env::var("SETTINGS_SNAPSHOT_WAIT").ok().and_then(|v| v.parse().ok()).unwrap_or(1800);
    glib::timeout_add_local_once(std::time::Duration::from_millis(wait), move || {
        // SETTINGS_SNAPSHOT_PAGE=1 renders the whole current page, not just what fits.
        let target = if std::env::var_os("SETTINGS_SNAPSHOT_PAGE").is_some() {
            UI.with(|cell| cell.borrow().clone()).and_then(|u| {
                let u = u.borrow();
                u.pages.get(u.current).and_then(|p| p.child()).and_then(|v| v.first_child())
            })
        } else {
            window.child()
        };
        if let Some(child) = target {
            let paintable = gtk::WidgetPaintable::new(Some(&child));
            let (w, h) = (child.width(), child.height());
            let snapshot = gtk::Snapshot::new();
            snapshot.append_color(&gdk::RGBA::BLACK, &gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32));
            paintable.snapshot(&snapshot, w as f64, h as f64);
            if let (Some(node), Some(renderer)) = (snapshot.to_node(), window.renderer()) {
                let texture = renderer.render_texture(node, None);
                match texture.save_to_png(&out) {
                    Ok(()) => println!("snapshot {w}x{h} -> {}", out.display()),
                    Err(e) => eprintln!("snapshot failed: {e}"),
                }
            }
        }
        app.quit();
    });
}
