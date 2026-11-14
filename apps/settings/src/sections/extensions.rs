//! Extensions: device support for more brands, installed from GitHub.

use super::Section;
use super::services::{mark_picked, pickable};
use crate::ext::{self, Extension, manage};
use crate::widgets::{self, Page};
use crate::{cmd, units, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    /// The catalog is fetched once per run; after that the page uses the copy on disk.
    static FETCHED: Cell<bool> = const { Cell::new(false) };
    /// The extension shown on the right, kept across redraws of the page.
    static SELECTED: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn after_change() {
    // An extension may want (or no longer want) to hear about theme changes.
    cmd::background(crate::backend::themehook::ensure, |_| {});
    window::reload_sections(false);
    window::rebuild("extensions");
}

/// Disable a button and show progress while `work` runs on a worker thread.
fn busy<T: Send + 'static>(
    b: &gtk::Button,
    label: &str,
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    done: impl FnOnce(T) + 'static,
) {
    let old = b.label().map(|l| l.to_string()).unwrap_or_default();
    b.set_sensitive(false);
    b.set_label(label);
    let b = b.clone();
    cmd::background(
        move || work().map_err(|e| format!("{e:#}")),
        move |r| {
            b.set_sensitive(true);
            b.set_label(&old);
            match r {
                Ok(v) => done(v),
                Err(e) => window::toast(&e),
            }
        },
    );
}

/// Close the window (a sub-page) a button is in.
fn close_from(b: &glib::WeakRef<gtk::Button>) {
    if let Some(w) = b.upgrade().and_then(|b| b.root()).and_downcast::<gtk::Window>()
        && Some(&w) != window::window().map(|m| m.upcast::<gtk::Window>()).as_ref()
    {
        w.close();
    }
}

fn is_asus_laptop() -> bool {
    std::fs::read_to_string("/sys/class/dmi/id/sys_vendor").is_ok_and(|v| v.to_uppercase().contains("ASUS"))
}

// ---------- Where an extension's pages go ----------

/// The sidebar group an extension's manifest asks for (as the sidebar files it).
fn manifest_group(category: &str) -> &'static str {
    match category {
        "Connections" => "Connections",
        "Personalization" => "Personalization",
        "Desktop" => "Desktop",
        "Input" => "Input",
        "Accounts" => "Accounts",
        "System" => "System",
        _ => "Hardware",
    }
}

/// The theme's name for a group's colour, as the sidebar uses it.
fn colour_key(group: &str) -> String {
    match crate::platform::colour_group(group) {
        "" => "home".into(),
        "Assistant & AI" => "assistant".into(),
        g => g.to_lowercase().replace(' ', "-"),
    }
}

/// The icon on a group's tile in the sidebar.
fn group_icon(group: &str, first_page: &'static str) -> &'static str {
    match group {
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

/// An icon on a tile in a group's colour.
fn colour_tile(icon: &str, key: &str) -> gtk::Box {
    let t = widgets::hbox(0);
    t.add_css_class("row-tile");
    t.add_css_class(&format!("section-{key}"));
    t.set_valign(gtk::Align::Center);
    t.set_halign(gtk::Align::Center);
    let i = gtk::Image::from_icon_name(icon);
    i.set_halign(gtk::Align::Center);
    i.set_valign(gtk::Align::Center);
    t.append(&i);
    t
}

/// The sidebar's groups in order, each with its pages' titles and whether a page is this
/// extension's.
fn groups(sections: &[Section], ext_id: &str) -> Vec<(&'static str, &'static str, Vec<(&'static str, bool)>)> {
    let prefix = format!("{ext_id}.");
    let mut out: Vec<(&'static str, &'static str, Vec<(&'static str, bool)>)> = Vec::new();
    for s in sections.iter().filter(|s| !s.group.is_empty() && s.shown()) {
        let mine = s.id.starts_with(&prefix);
        match out.iter_mut().find(|(g, _, _)| *g == s.group) {
            Some((_, _, pages)) => pages.push((s.title, mine)),
            None => out.push((s.group, s.icon, vec![(s.title, mine)])),
        }
    }
    out
}

fn installed_desc(e: &Extension) -> String {
    let m = &e.manifest;
    let missing = e.missing();
    let version = if m.version.is_empty() { String::new() } else { format!(" · {}", glib::markup_escape_text(&m.version)) };
    if !ext::enabled(e.id()) {
        return "Turned off · its pages are hidden".into();
    }
    if !missing.is_empty() {
        return if m.needs_hint.is_empty() {
            format!("Needs <tt>{}</tt>, which isn't installed", glib::markup_escape_text(&missing.join(", ")))
        } else {
            m.needs_hint.clone()
        };
    }
    match ext::cached_pages(e.id()) {
        Some(p) if !p.is_empty() => format!("{} page{}{version}", p.len(), if p.len() == 1 { "" } else { "s" }),
        _ => format!("No matching devices right now{version}"),
    }
}

/// The icon it has in the catalog, or its first page's.
fn ext_icon(e: &Extension, catalog: &[manage::Entry]) -> String {
    catalog
        .iter()
        .find(|c| c.id == e.id())
        .map(|c| c.icon.clone())
        .or_else(|| ext::cached_pages(e.id()).and_then(|p| p.first().map(|p| p.icon.clone())))
        .filter(|i| !i.is_empty())
        .unwrap_or_else(|| "application-x-addon-symbolic".into())
}

/// The group its pages are in: where the sidebar has them, or where its manifest puts them.
fn ext_group(e: &Extension, sections: &[Section]) -> &'static str {
    let prefix = format!("{}.", e.id());
    sections.iter().find(|s| s.id.starts_with(&prefix)).map(|s| s.group).unwrap_or_else(|| manifest_group(&e.manifest.category))
}

mod loose {
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct Loose;

    #[glib::object_subclass]
    impl ObjectSubclass for Loose {
        const NAME: &'static str = "SettingsSystemLoose";
        type Type = super::Loose;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Loose {
        fn dispose(&self) {
            while let Some(c) = self.obj().first_child() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for Loose {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, o: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(c) = self.obj().first_child() else { return (0, 0, -1, -1) };
            if o == gtk::Orientation::Horizontal {
                let (_, nat, _, _) = c.measure(o, -1);
                return (0, nat, -1, -1);
            }
            let (cmin, _, _, _) = c.measure(gtk::Orientation::Horizontal, -1);
            let width = if for_size < 0 { -1 } else { for_size.max(cmin) };
            let (min, nat, _, _) = c.measure(o, width);
            (min, nat, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(c) = self.obj().first_child() {
                let (cmin, _, _, _) = c.measure(gtk::Orientation::Horizontal, -1);
                c.allocate(width.max(cmin), height, baseline, None);
            }
        }
    }
}

glib::wrapper! {
    /// A holder that asks for no width of its own (its content can still be wider), so
    /// a wide preview never keeps the window from getting narrow enough to put the
    /// preview above the controls.
    pub struct Loose(ObjectSubclass<loose::Loose>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

pub(crate) fn shrinkable(content: &impl IsA<gtk::Widget>) -> Loose {
    let l: Loose = glib::Object::new();
    l.set_overflow(gtk::Overflow::Hidden);
    content.set_parent(&l);
    l
}

/// A heading like the preview's own (`Preview::head`), for content redrawn in place.
pub(crate) fn preview_head(title: &str, line: &str, end: &[gtk::Widget]) -> gtk::Box {
    let head = widgets::hbox(10);
    head.add_css_class("preview-head");
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(title, "preview-title");
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&t);
    if !line.is_empty() {
        let l = widgets::label(line, "preview-line");
        l.set_wrap(true);
        text.append(&l);
    }
    head.append(&text);
    for w in end {
        w.set_valign(gtk::Align::Center);
        head.append(w);
    }
    head
}

/// A small Settings: the sidebar with the group that gains pages marked, that group's
/// pages with the new ones, and a card for each new page.
fn map(e: &Extension, sections: &[Section], on: bool) -> gtk::Box {
    let pages = ext::cached_pages(e.id()).unwrap_or_default();
    let group = ext_group(e, sections);
    let key = colour_key(group);
    let card = widgets::hbox(0);
    card.add_css_class("preview-card");
    card.add_css_class("ex-map");

    let side = widgets::vbox(2);
    side.add_css_class("ex-side");
    let all = groups(sections, e.id());
    for (g, first_icon, _) in &all {
        let r = widgets::hbox(8);
        r.add_css_class("ex-side-row");
        let gains = *g == group;
        if gains {
            r.add_css_class("on");
        }
        r.append(&colour_tile(group_icon(g, first_icon), &colour_key(g)));
        let l = widgets::label(g, "ex-side-name");
        l.set_hexpand(true);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        r.append(&l);
        if gains {
            let plus = widgets::label(&format!("+{}", pages.len()), "ex-plus");
            if !on {
                plus.add_css_class("dim");
            }
            r.append(&plus);
        }
        side.append(&r);
    }
    card.append(&side);

    let right = widgets::vbox(12);
    right.add_css_class("ex-right");
    right.set_hexpand(true);
    let kicker = widgets::label(&group.to_uppercase(), "ex-kicker");
    kicker.add_css_class(&format!("section-{key}"));
    right.append(&kicker);
    // The group's pages: a few before the new ones, then the new ones.
    let strip = gtk::FlowBox::new();
    strip.set_selection_mode(gtk::SelectionMode::None);
    strip.set_column_spacing(14);
    strip.set_row_spacing(6);
    strip.set_max_children_per_line(12);
    strip.add_css_class("ex-strip");
    let theirs: Vec<&str> = all
        .iter()
        .find(|(g, _, _)| *g == group)
        .map(|(_, _, p)| p.iter().filter(|(_, mine)| !mine).map(|(t, _)| *t).collect())
        .unwrap_or_default();
    let skip = theirs.len().saturating_sub(4);
    if skip > 0 {
        strip.insert(&widgets::label("…", "ex-tab"), -1);
    }
    for t in &theirs[skip..] {
        strip.insert(&widgets::label(t, "ex-tab"), -1);
    }
    for p in &pages {
        let t = widgets::hbox(6);
        t.add_css_class("ex-tab-new");
        t.add_css_class(&format!("section-{key}"));
        t.append(&widgets::label(&p.title, "ex-tab-title"));
        t.append(&widgets::tag(if on { "New" } else { "Hidden" }));
        strip.insert(&t, -1);
    }
    right.append(&strip);
    for p in &pages {
        let g = widgets::vbox(6);
        g.add_css_class("ex-glimpse");
        if !on {
            g.add_css_class("off");
        }
        let top = widgets::hbox(10);
        top.append(&colour_tile(&p.icon, &key));
        let name = widgets::vbox(0);
        name.append(&widgets::label(&p.title, "ex-glimpse-title"));
        name.append(&widgets::label(&format!("{group} › {}", p.title), "ex-glimpse-where"));
        top.append(&name);
        g.append(&top);
        if !p.description.is_empty() {
            let d = widgets::label(&p.description, "ex-glimpse-desc");
            d.set_wrap(true);
            g.append(&d);
        }
        right.append(&g);
    }
    card.append(&right);
    card
}

/// The selected extension: where its pages go, what it is, Remove and Update.
fn show(holder: &gtk::Box, e: &Extension, catalog: &[manage::Entry], sections: &[Section], has_update: bool) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let on = ext::enabled(e.id());
    let pages = ext::cached_pages(e.id()).unwrap_or_default();
    let group = ext_group(e, sections);
    let mut line = vec![format!("For {group}")];
    if !pages.is_empty() {
        line.push(format!("adds {} page{}", pages.len(), if pages.len() == 1 { "" } else { "s" }));
    }
    if !e.manifest.version.is_empty() {
        line.push(e.manifest.version.clone());
    }
    let mut end: Vec<gtk::Widget> = Vec::new();
    let e2 = e.clone();
    let remove = widgets::confirm_button("Remove", "Remove?", move |b| {
        let (e3, name) = (e2.clone(), e2.manifest.name.clone());
        let id = e3.id().to_string();
        busy(
            b,
            "Removing…",
            move || manage::remove(&e3),
            move |_| {
                crate::backend::updates::clear_extension(&id);
                SELECTED.with(|s| *s.borrow_mut() = None);
                window::toast(&format!("Removed {name}"));
                after_change();
            },
        );
    });
    remove.add_css_class("flat");
    remove.set_tooltip_text(Some("Delete it and its pages; turning it off keeps it"));
    end.push(remove.upcast());
    if has_update {
        let update = gtk::Button::with_label("Update");
        update.add_css_class("suggested-action");
        let e2 = e.clone();
        update.connect_clicked(move |b| {
            let (e3, name, id) = (e2.clone(), e2.manifest.name.clone(), e2.id().to_string());
            busy(
                b,
                "Updating…",
                move || manage::update(&e3),
                move |changed| {
                    crate::backend::updates::clear_extension(&id);
                    window::toast(&if changed { format!("Updated {name}") } else { format!("{name} is up to date") });
                    after_change();
                },
            );
        });
        end.push(update.upcast());
    }
    holder.append(&preview_head(&e.manifest.name, &line.join(" · "), &end));

    if pages.is_empty() {
        let c = widgets::vbox(8);
        c.add_css_class("preview-card");
        c.add_css_class("ex-none");
        let missing = e.missing();
        let (title, text) = if !on {
            ("Turned off", "Its pages are hidden. Turn it on to see them again.".to_string())
        } else if !missing.is_empty() {
            (
                "Something it needs isn't installed",
                if e.manifest.needs_hint.is_empty() {
                    format!("It needs {}, which isn't installed.", missing.join(", "))
                } else {
                    e.manifest.needs_hint.clone()
                },
            )
        } else {
            ("No pages right now", "None of the devices it works with are connected. Plug one in, then look again.".to_string())
        };
        let t = widgets::label(title, "ex-glimpse-title");
        t.set_wrap(true);
        c.append(&t);
        let l = gtk::Label::new(None);
        widgets::set_markup_or_text(&l, &text);
        l.add_css_class("ex-glimpse-desc");
        l.set_wrap(true);
        l.set_xalign(0.0);
        c.append(&l);
        if on && missing.is_empty() {
            let again = gtk::Button::with_label("Look for devices");
            again.set_halign(gtk::Align::Start);
            again.connect_clicked(rescan);
            c.append(&again);
        }
        holder.append(&c);
    }
    if !pages.is_empty() || !on {
        holder.append(&map(e, sections, on));
    }

    // What it is, from its manifest and the catalog.
    let about = widgets::vbox(6);
    about.add_css_class("preview-card");
    about.append(&widgets::label("ABOUT IT", "ex-kicker"));
    if !e.manifest.description.is_empty() {
        let d = widgets::label(&e.manifest.description, "ex-about");
        d.set_wrap(true);
        about.append(&d);
    }
    if let Some(c) = catalog.iter().find(|c| c.id == e.id()).filter(|c| !c.devices.is_empty()) {
        let d = widgets::label(&format!("Works with {}", c.devices), "ex-glimpse-desc");
        d.set_wrap(true);
        about.append(&d);
    }
    if about.observe_children().n_items() > 1 {
        holder.append(&about);
    }
}

fn rescan(b: &gtk::Button) {
    busy(
        b,
        "Looking…",
        || Ok(ext::refresh_pages()),
        |changed| {
            window::toast(if changed { "Found changes" } else { "Nothing new" });
            after_change();
        },
    );
}

/// Nothing installed: the catalog, to start from.
fn show_catalog(holder: &gtk::Box, available: &[manage::Entry]) {
    holder.append(&preview_head(
        "Pages for more devices and apps",
        "An extension adds pages to Settings, in the section they belong to.",
        &[],
    ));
    let list = widgets::vbox(0);
    for c in available.iter().take(6) {
        list.append(&catalog_row(c));
    }
    // The list takes what height is left and scrolls inside its card.
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(200)
        .vexpand(true)
        .child(&list)
        .build();
    let card = widgets::vbox(0);
    card.add_css_class("settings-card");
    card.set_overflow(gtk::Overflow::Hidden);
    card.append(&scroll);
    holder.append(&card);
}

fn catalog_row(c: &manage::Entry) -> gtk::Box {
    let mut desc = glib::markup_escape_text(&c.description).to_string();
    if !c.devices.is_empty() {
        desc.push_str(&format!("\n<i>{}</i>", glib::markup_escape_text(&c.devices)));
    }
    let missing: Vec<&str> = c.needs.iter().map(String::as_str).filter(|p| !cmd::present(p)).collect();
    if !missing.is_empty() {
        desc.push_str(&format!("\nNeeds <tt>{}</tt>, which isn't installed.", glib::markup_escape_text(&missing.join(", "))));
    }
    let install = gtk::Button::with_label("Install");
    let id = c.id.clone();
    install.connect_clicked(move |b| {
        let id = id.clone();
        let weak = b.downgrade();
        busy(
            b,
            "Installing…",
            move || manage::install(&id),
            move |e| {
                let n = ext::cached_pages(e.id()).map(|p| p.len()).unwrap_or(0);
                window::toast(&if n == 0 {
                    format!("Installed {}. No matching devices found yet.", e.manifest.name)
                } else {
                    format!("Installed {}", e.manifest.name)
                });
                SELECTED.with(|s| *s.borrow_mut() = Some(e.id().to_string()));
                close_from(&weak);
                after_change();
            },
        );
    });
    let r = widgets::row(&c.name, &desc, Some(install.upcast_ref()));
    r.prepend(&widgets::tile(&c.icon));
    r.add_css_class("with-tile");
    r
}

fn available_page(page: &Page, available: &[manage::Entry]) {
    let g = page.group("");
    g.note("From the Nyx catalog. Each one adds its pages in the section they belong to.");
    if available.is_empty() {
        g.add(&widgets::row("Every extension in the catalog is installed", "", None));
    }
    for c in available {
        g.add(&catalog_row(c));
    }
}

fn github_page(page: &Page) {
    let g = page.group("");
    g.note("Any git repository with an <tt>extension.toml</tt>. Only add extensions you trust: they run as you.");
    let repo = gtk::Entry::new();
    repo.set_placeholder_text(Some("https://github.com/user/repo"));
    repo.set_hexpand(true);
    let path = gtk::Entry::new();
    path.set_placeholder_text(Some("folder (optional)"));
    path.set_width_chars(14);
    let add = gtk::Button::with_label("Install");
    add.add_css_class("suggested-action");
    {
        let (repo, path) = (repo.clone(), path.clone());
        add.connect_clicked(move |b| {
            let url = repo.text().trim().to_string();
            if url.is_empty() {
                repo.grab_focus();
                return;
            }
            let sub = path.text().trim().to_string();
            let weak = b.downgrade();
            busy(
                b,
                "Installing…",
                move || manage::install_from(&url, &sub, &[], None),
                move |e| {
                    window::toast(&format!("Installed {}", e.manifest.name));
                    SELECTED.with(|s| *s.borrow_mut() = Some(e.id().to_string()));
                    close_from(&weak);
                    after_change();
                },
            );
        });
    }
    let line = widgets::hbox(8);
    line.append(&repo);
    line.append(&path);
    line.append(&add);
    g.add(&widgets::stacked_row("Repository", "", line.upcast_ref()));
}

pub fn build(page: &Page) {
    let installed = ext::installed();
    let catalog = Rc::new(manage::catalog(false));
    if !FETCHED.with(|f| f.replace(true)) {
        // Fetch the latest catalog; redraw only if it differs from what's shown.
        let shown = (*catalog).clone();
        cmd::background(
            || manage::catalog(true),
            move |fresh| {
                if fresh != shown {
                    window::rebuild("extensions");
                }
            },
        );
    }
    let status = crate::backend::updates::cached();
    let pending = Rc::new(status.extensions.clone());
    let sections = Rc::new(super::all());
    let available: Vec<manage::Entry> = catalog.iter().filter(|c| !installed.iter().any(|e| e.id() == c.id)).cloned().collect();

    // The picture: the selected extension, and where its pages go.
    let pv = page.split();
    let holder = widgets::vbox(14);
    // So the catalog's list can take the height left over.
    pv.fill(&shrinkable(&holder));
    pv.foot("dialog-information-symbolic", "Turning an extension off hides its pages and keeps its settings; Remove deletes it.");
    let chosen = SELECTED
        .with(|s| s.borrow().clone())
        .and_then(|id| installed.iter().find(|e| e.id() == id))
        .or_else(|| installed.iter().find(|e| pending.iter().any(|p| p == e.id())))
        .or_else(|| installed.iter().find(|e| ext::enabled(e.id()) && ext::cached_pages(e.id()).is_some_and(|p| !p.is_empty())))
        .or(installed.first())
        .cloned();
    let show_ext: Rc<dyn Fn(&Extension)> = {
        let (holder, catalog, sections, pending) = (holder.clone(), catalog.clone(), sections.clone(), pending.clone());
        Rc::new(move |e: &Extension| show(&holder, e, &catalog, &sections, pending.iter().any(|p| p == e.id())))
    };
    match &chosen {
        Some(e) => show_ext(e),
        None => show_catalog(&holder, &available),
    }

    if !installed.iter().any(|e| e.id() == "asus") && cmd::present("asusctl") && is_asus_laptop() {
        let b = page
            .banner("<b>ASUS controls are now an extension.</b> Install it to get the ASUS and Aura Lighting pages back.", false);
        let install = gtk::Button::with_label("Install");
        install.add_css_class("suggested-action");
        install.set_valign(gtk::Align::Center);
        install.connect_clicked(|b| {
            busy(
                b,
                "Installing…",
                || manage::install("asus"),
                |e| {
                    window::toast(&format!("Installed {}", e.manifest.name));
                    after_change();
                },
            )
        });
        b.append(&install);
    }

    // ----- Updates -----
    let waiting: Vec<&Extension> = installed.iter().filter(|e| pending.iter().any(|p| p == e.id())).collect();
    let checked = if status.checked == 0 {
        "Not checked yet".to_string()
    } else {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        format!("Checked {}", units::ago(now.saturating_sub(status.checked) as i64))
    };
    let g = page.group("");
    if waiting.is_empty() {
        let check = gtk::Button::with_label("Check");
        check.set_tooltip_text(Some("Settings also checks once a day when it opens"));
        check.connect_clicked(|b| {
            busy(
                b,
                "Checking…",
                || Ok(crate::backend::updates::check(true)),
                |s| {
                    if s.extensions.is_empty() {
                        window::toast("Extensions are up to date");
                    }
                    window::after_update_check(s);
                },
            );
        });
        let title = if installed.is_empty() { "No extensions yet" } else { "Extensions are up to date" };
        g.add(&widgets::icon_row("software-update-available-symbolic", title, &checked, Some(check.upcast_ref())));
    } else {
        let names: Vec<&str> = waiting.iter().map(|e| e.manifest.name.as_str()).collect();
        let update = gtk::Button::with_label("Update");
        update.add_css_class("suggested-action");
        update.connect_clicked(|b| {
            busy(b, "Updating…", crate::backend::updates::update_extensions, |names| {
                window::toast(&format!("Updated {}", names.join(", ")));
                after_change();
                window::rebuild_if_built("about");
            })
        });
        let title = format!("{} update{} · {}", waiting.len(), if waiting.len() == 1 { "" } else { "s" }, names.join(", "));
        g.add(&widgets::icon_row("folder-download-symbolic", &title, &checked, Some(update.upcast_ref())));
    }
    widgets::keywords("check for updates update all new versions");

    // ----- Installed -----
    if !installed.is_empty() {
        let g = page.group("Installed");
        let tools = widgets::hbox(8);
        tools.set_halign(gtk::Align::End);
        tools.add_css_class("group-tools");
        let again = gtk::Button::new();
        let inner = widgets::hbox(6);
        inner.append(&gtk::Image::from_icon_name("view-refresh-symbolic"));
        inner.append(&gtk::Label::new(Some("Look for devices")));
        again.set_child(Some(&inner));
        again.add_css_class("flat");
        again.set_tooltip_text(Some("After plugging something in, ask every extension what it can see"));
        again.connect_clicked(rescan);
        tools.append(&again);
        g.top(&tools);
        let rows: Rc<RefCell<Vec<(String, gtk::Box)>>> = Rc::default();
        for e in installed.clone() {
            let on = gtk::Switch::new();
            on.set_active(ext::enabled(e.id()));
            on.set_valign(gtk::Align::Center);
            on.set_tooltip_text(Some("Turn off to hide its pages without removing it"));
            let id = e.id().to_string();
            on.connect_active_notify(move |s| {
                manage::set_enabled(&id, s.is_active());
                SELECTED.with(|sel| *sel.borrow_mut() = Some(id.clone()));
                // This page is rebuilt with the new state; not from inside the switch's own handler.
                glib::idle_add_local_once(after_change);
            });
            let row = widgets::row(&e.manifest.name, &installed_desc(&e), Some(on.upcast_ref()));
            row.prepend(&colour_tile(&ext_icon(&e, &catalog), &colour_key(ext_group(&e, &sections))));
            row.add_css_class("with-tile");
            if !ext::enabled(e.id()) {
                row.add_css_class("ex-off");
            }
            if pending.iter().any(|p| p == e.id()) {
                widgets::tag_row(&row, "Update");
            }
            {
                let (rows, show_ext, e, row2) = (rows.clone(), show_ext.clone(), e.clone(), row.clone());
                pickable(&row, move || {
                    SELECTED.with(|s| *s.borrow_mut() = Some(e.id().to_string()));
                    mark_picked(rows.borrow().iter().map(|(_, r)| r), Some(&row2));
                    show_ext(&e);
                });
            }
            g.add(&row);
            widgets::keywords("enable disable turn on off hide extension update rescan refresh detect");
            rows.borrow_mut().push((e.id().to_string(), row));
        }
        let picked = chosen.as_ref().and_then(|c| rows.borrow().iter().find(|(id, _)| id == c.id()).map(|(_, r)| r.clone()));
        mark_picked(rows.borrow().iter().map(|(_, r)| r), picked.as_ref());
    }

    // ----- More -----
    let g = page.group("");
    let available = Rc::new(available);
    let count = if available.is_empty() { String::new() } else { available.len().to_string() };
    let (r, _) = widgets::nav_row(Some("application-x-addon-symbolic"), "Get more extensions", "Devices and apps from the catalog", &count, {
        let available = available.clone();
        move || {
            let available = available.clone();
            widgets::subpage("Get more extensions", 600, move |p| {
                available_page(p, &available);
                widgets::forget_rows(&p.body);
            });
        }
    });
    let words: Vec<String> = catalog.iter().map(|c| format!("{} {} {}", c.id, c.name, c.devices)).collect();
    widgets::keywords(&format!("install catalog available {}", words.join(" ")));
    g.add(&r);
    let (r, _) = widgets::nav_row(Some("text-editor-symbolic"), "Add from GitHub", "Any repository with an extension.toml", "", || {
        widgets::subpage("Add from GitHub", 560, |p| {
            github_page(p);
            widgets::forget_rows(&p.body);
        });
    });
    widgets::keywords("url link git clone custom third party repository");
    g.add(&r);
}
