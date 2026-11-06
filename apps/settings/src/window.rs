//! The main window: the unified header (the app and the page, the sidebar
//! toggle, search, the page's config files), the navigation sidebar, a stack of
//! section pages built the first time they're shown.

use crate::sections::{self, Section};
use crate::widgets::{self, SEARCH};
use crate::{prefs, theme};
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

struct Ui {
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    nav: gtk::Box,
    nav_list: gtk::Box,
    nav_items: HashMap<&'static str, gtk::Button>,
    nav_groups: Vec<(gtk::Label, Vec<&'static str>)>,
    /// Hidden in the icon-only sidebar: fixed parts, and the labels of the current items.
    compact_fixed: Vec<gtk::Widget>,
    compact_items: Vec<gtk::Widget>,
    /// A Cell: it's set while the window is being shown, when the UI is already borrowed.
    compact: std::cell::Cell<bool>,
    pages: HashMap<&'static str, gtk::ScrolledWindow>,
    sections: Vec<Section>,
    current: &'static str,
    overlay: gtk::Overlay,
    collapse_button: gtk::Button,
    /// The header, which shows the page's name after the app's.
    header: nyx_ui::chrome::AppHeader,
    /// Holds the current page's "open the config file" button.
    edit: gtk::Box,
    /// The "nothing matches" page's message.
    empty_label: gtk::Label,
    /// The toast on screen, if any: a new one replaces it.
    toast: Option<gtk::Revealer>,
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
    let compact_hide: Vec<gtk::Widget> = Vec::new();

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let (nav_items, nav_groups, compact_items) = fill_nav(&list, &sections);
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

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.set_vexpand(true);
    body.append(&nav);
    body.append(&stack);

    // ----- Header: the app and the page, the sidebar toggle, search, and the
    // page's config files -----
    let header = nyx_ui::chrome::app_header(&window, "preferences-system-symbolic", "Settings");
    let collapse_button = widgets::bar_button("sidebar-show-symbolic", "Collapse the sidebar (Ctrl+B)");
    collapse_button.connect_clicked(|_| toggle_sidebar());
    header.start.append(&collapse_button);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search all settings  (Ctrl+F)"));
    search.add_css_class("bar-search");
    search.set_width_chars(34);
    header.center.append(&search);
    let edit = widgets::hbox(0);
    header.end.append(&edit);

    let frame = widgets::vbox(0);
    frame.add_css_class("window-frame");
    frame.append(&header.root);
    frame.append(&body);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&frame));
    window.set_child(Some(&overlay));

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
        nav_items,
        nav_groups,
        compact_fixed: compact_hide,
        compact_items,
        compact: std::cell::Cell::new(false),
        pages: HashMap::new(),
        sections,
        current: "",
        overlay,
        collapse_button,
        header,
        edit,
        empty_label,
        toast: None,
    };
    UI.with(|u| *u.borrow_mut() = Some(Rc::new(RefCell::new(ui))));
}

type NavParts = (HashMap<&'static str, gtk::Button>, Vec<(gtk::Label, Vec<&'static str>)>, Vec<gtk::Widget>);

thread_local! {
    /// Counts shown after a page's name in the sidebar, e.g. updates on Home.
    static NAV_BADGES: RefCell<HashMap<&'static str, String>> = RefCell::default();
}

fn nav_badge(text: &str) -> gtk::Label {
    let t = widgets::tag(text);
    t.add_css_class("nav-badge");
    t
}

/// Show `text` after a page's name in the sidebar, or nothing.
pub fn set_nav_badge(id: &'static str, text: Option<&str>) {
    NAV_BADGES.with(|b| match text {
        Some(t) => b.borrow_mut().insert(id, t.to_string()),
        None => b.borrow_mut().remove(id),
    });
    let Some(ui) = ui() else { return };
    let mut u = ui.borrow_mut();
    let Some(content) = u.nav_items.get(id).and_then(|b| b.child()).and_downcast::<gtk::Box>() else { return };
    if let Some(old) = content.last_child().filter(|w| w.has_css_class("nav-badge")) {
        content.remove(&old);
        u.compact_items.retain(|w| w != &old);
    }
    if let Some(text) = text {
        let t = nav_badge(text);
        t.set_visible(!u.compact.get());
        content.append(&t);
        u.compact_items.push(t.upcast());
    }
}

/// The sidebar entries: a heading per group, a button per page.
/// The theme's name for a group of pages (`[sections.settings]`): Home's pages
/// have no group.
fn section_key(group: &str) -> String {
    let group = crate::platform::colour_group(group);
    if group.is_empty() { "home".into() } else { group.to_lowercase().replace(' ', "-") }
}

fn fill_nav(list: &gtk::Box, sections: &[Section]) -> NavParts {
    let mut nav_items = HashMap::new();
    let mut nav_groups: Vec<(gtk::Label, Vec<&'static str>)> = Vec::new();
    let mut compact_items: Vec<gtk::Widget> = Vec::new();
    let mut last_group = "";
    for s in sections {
        if !(s.visible)() || crate::platform::off_sidebar(s.id) {
            continue;
        }
        if s.group != last_group {
            // The icon-only sidebar shows a thin line where a group's heading would be.
            let divider = gtk::Separator::new(gtk::Orientation::Horizontal);
            divider.add_css_class("nav-divider");
            divider.set_visible(false);
            // The group's section colour (from the theme) on its heading and divider.
            let section = format!("section-{}", section_key(s.group));
            divider.add_css_class(&section);
            list.append(&divider);
            let g = widgets::label(&s.group.to_uppercase(), "nav-group");
            g.add_css_class(&section);
            compact_items.push(g.clone().upcast());
            list.append(&g);
            nav_groups.push((g, Vec::new()));
            last_group = s.group;
        }
        let button = gtk::Button::new();
        button.add_css_class("nav-item");
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        content.append(&gtk::Image::from_icon_name(s.icon));
        let l = widgets::label(s.title, "nav-label");
        l.set_hexpand(true);
        compact_items.push(l.clone().upcast());
        content.append(&l);
        if let Some(text) = NAV_BADGES.with(|b| b.borrow().get(s.id).cloned()) {
            let t = nav_badge(&text);
            compact_items.push(t.clone().upcast());
            content.append(&t);
        }
        button.set_child(Some(&content));
        button.set_tooltip_text(Some(s.description));
        let id = s.id;
        button.connect_clicked(move |_| navigate(id));
        list.append(&button);
        nav_items.insert(s.id, button);
        if let Some(g) = nav_groups.last_mut() {
            g.1.push(s.id);
        }
    }
    (nav_items, nav_groups, compact_items)
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
    let (items, groups, compact_items) = fill_nav(&u.nav_list, &sections);
    for w in &compact_items {
        w.set_visible(!u.compact.get());
    }
    show_dividers(&u.nav_list, u.compact.get());
    // Drop built extension pages (their page may have changed) and pages that are gone.
    let stale: Vec<&'static str> = u
        .pages
        .keys()
        .copied()
        .filter(|id| sections.iter().find(|s| s.id == *id).is_none_or(|s| matches!(s.build, sections::Build::Extension(_))))
        .collect();
    for id in &stale {
        if let Some(p) = u.pages.remove(id) {
            SEARCH.with(|s| s.borrow_mut().retain(|item| item.section != *id));
            u.stack.remove(&p);
        }
    }
    u.nav_items = items;
    u.nav_groups = groups;
    u.compact_items = compact_items;
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

/// Below this the sidebar always shows only icons. Above it, only when the page
/// wouldn't fit beside the full sidebar (half a 1920-pixel screen keeps the headings).
const NARROW_BELOW: i32 = 760;
/// The full sidebar's width, a little over what it measures.
const SIDEBAR_WIDTH: i32 = 272;
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

/// The sidebar shows only icons: hide the labels, centre the icons and the toggle.
fn apply_compact(compact: bool) {
    let Some(ui) = ui() else { return };
    let u = ui.borrow();
    if compact {
        u.nav.add_css_class("compact");
    } else {
        u.nav.remove_css_class("compact");
    }
    u.compact.set(compact);
    for wdg in u.compact_fixed.iter().chain(&u.compact_items) {
        wdg.set_visible(!compact);
    }
    show_group_labels(&u);
    show_dividers(&u.nav_list, compact);
    show_search_counts(&u);
    u.collapse_button.set_tooltip_text(Some(if compact {
        "Expand the sidebar (Ctrl+B)"
    } else {
        "Collapse the sidebar (Ctrl+B)"
    }));
    centre_icons(u.nav.upcast_ref(), compact);
}

/// The full sidebar's group headings: shown for groups with a page showing.
fn show_group_labels(u: &Ui) {
    for (label, ids) in &u.nav_groups {
        let any = ids.iter().any(|id| u.nav_items.get(id).is_some_and(|b| b.is_visible()));
        label.set_visible(any && !u.compact.get());
    }
}

/// The icon-only sidebar's group dividers: shown for groups with a page showing.
fn show_dividers(list: &gtk::Box, compact: bool) {
    let mut c = list.first_child();
    while let Some(w) = c {
        if w.has_css_class("nav-divider") {
            let any = std::iter::successors(w.next_sibling(), |n| n.next_sibling())
                .take_while(|n| !n.has_css_class("nav-divider"))
                .any(|n| n.has_css_class("nav-item") && n.is_visible());
            w.set_visible(compact && any);
        }
        c = w.next_sibling();
    }
}

fn centre_icons(w: &gtk::Widget, compact: bool) {
    if w.has_css_class("nav-item")
        && let Some(content) = w.downcast_ref::<gtk::Button>().and_then(|b| b.child())
    {
        content.set_halign(if compact { gtk::Align::Center } else { gtk::Align::Fill });
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        centre_icons(&c, compact);
        child = c.next_sibling();
    }
}

pub fn toggle_sidebar() {
    prefs::update(|p| p.sidebar_collapsed = !p.sidebar_collapsed);
    apply_compact(NARROW.with(|n| n.get()) || prefs::get().sidebar_collapsed);
}

fn set_narrow(narrow: bool) {
    NARROW.with(|n| n.set(narrow));
    let Some(ui) = ui() else { return };
    for page in ui.borrow().pages.values() {
        mark_page(page, narrow);
    }
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
    let (id, visible) = {
        let u = ui.borrow();
        (u.current, u.window.is_visible() && u.window.is_active())
    };
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
        let shown = |s: &&Section| (s.visible)();
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
    if let Some(prev) = u.nav_items.get(u.current) {
        prev.remove_css_class("active");
    }
    if let Some(b) = u.nav_items.get(id) {
        b.add_css_class("active");
    }
    u.stack.set_visible_child_name(id);
    u.current = id;
    if let Some(s) = u.sections.iter().find(|s| s.id == id) {
        u.header.set_subtitle(s.title);
        nyx_theme::set_section(Some(&section_key(s.group)));
        while let Some(c) = u.edit.first_child() {
            u.edit.remove(&c);
        }
        let files = s.config_files();
        if !files.is_empty() {
            u.edit.append(&widgets::config_button(&files));
        }
    }
    let window = u.window.clone();
    drop(u);
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
    u.sections.iter().filter(|s| (s.visible)()).map(|s| s.id).find(|id| !u.pages.contains_key(id))
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
    for s in &u.sections {
        ordered.extend(hit_rows.iter().filter(|(sec, _)| sec == s.id).map(|(_, r)| (s.id, r.clone())));
    }
    HITS.with(|h| *h.borrow_mut() = ordered);
    HIT_AT.with(|h| h.set(0));
    let counts =
        (!terms.is_empty()).then(|| u.sections.iter().filter_map(|s| section_hits.get(s.id).map(|n| (s.id, *n))).collect());
    COUNTS.with(|c| *c.borrow_mut() = counts);
    show_search_counts(&u);

    let mut first_match: Option<&'static str> = None;
    for s in &u.sections {
        let Some(button) = u.nav_items.get(s.id) else { continue };
        let visible = terms.is_empty() || section_hits.contains_key(s.id) || title_hits.contains(&s.id);
        button.set_visible(visible);
        if visible && first_match.is_none() && !terms.is_empty() {
            first_match = Some(s.id);
        }
    }
    show_group_labels(&u);
    show_dividers(&u.nav_list, u.compact.get());
    let current = u.current;
    let current_visible = u.nav_items.get(current).is_some_and(|b| b.is_visible());
    let was_empty = u.stack.visible_child_name().as_deref() == Some(EMPTY_PAGE);
    let nothing = !terms.is_empty() && first_match.is_none();
    if nothing {
        u.empty_label.set_text(&format!("No settings match “{}”", query.trim()));
        u.stack.set_visible_child_name(EMPTY_PAGE);
    } else if was_empty {
        u.stack.set_visible_child_name(current);
    }
    drop(u);
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

/// The number of results after each page's name in the sidebar while searching, or
/// in its tooltip when the sidebar shows only icons (a count there would widen it).
fn show_search_counts(u: &Ui) {
    let compact = u.compact.get();
    COUNTS.with(|counts| {
        let counts = counts.borrow();
        for s in &u.sections {
            let Some(button) = u.nav_items.get(s.id) else { continue };
            let count = counts.as_ref().and_then(|c| c.get(s.id).copied());
            set_search_count(button, if compact { None } else { count });
            let tip = match count {
                Some(n) if compact => format!("{}: {n} matching {}", s.title, if n == 1 { "setting" } else { "settings" }),
                _ => s.description.to_string(),
            };
            button.set_tooltip_text(Some(&tip));
        }
    });
}

/// How many results a page has, after its name in the sidebar while searching.
fn set_search_count(button: &gtk::Button, count: Option<usize>) {
    let Some(content) = button.child().and_downcast::<gtk::Box>() else { return };
    let existing = std::iter::successors(content.first_child(), |c| c.next_sibling()).find(|c| c.has_css_class("search-count"));
    match (count, existing) {
        (Some(n), Some(l)) => {
            if let Some(l) = l.downcast_ref::<gtk::Label>() {
                l.set_text(&n.to_string());
            }
            l.set_visible(true);
        }
        (Some(n), None) => {
            let l = widgets::tag(&n.to_string());
            l.add_css_class("search-count");
            l.set_tooltip_text(Some("Settings on this page that match"));
            content.append(&l);
        }
        (None, Some(l)) => l.set_visible(false),
        (None, None) => {}
    }
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
