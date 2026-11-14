//! The bar every NyxOS system page applies its changes with: they're collected as
//! they're made and built into one new generation through the Nyx System Manager.
//! It sits at the foot of the page: what's waiting (each change as before → after),
//! then the build while it runs, then whether it worked and a way back.

use crate::platform::nyxos;
use crate::widgets::{self, Page};
use crate::window;
use gtk::prelude::*;
use nyx_system_proto::Change;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

thread_local! {
    /// What the last apply on each page changed, shown once the page is rebuilt with it.
    static APPLIED: RefCell<HashMap<&'static str, Vec<String>>> = RefCell::default();
}

/// A change waiting: the change itself, and what it was and will be, in words.
struct Waiting {
    change: Change,
    before: String,
    after: String,
    /// When it was first made, so the bar lists changes in the order they were made.
    made: usize,
}

/// A page's changes waiting to be applied, and the bar that applies them.
pub struct Pending {
    section: &'static str,
    changes: RefCell<BTreeMap<String, Waiting>>,
    bar: widgets::ChangesBar,
    icon: gtk::Image,
    spinner: gtk::Spinner,
    chips: gtk::FlowBox,
    progress: gtk::ProgressBar,
    back: gtk::Button,
    done: gtk::Button,
    /// The page's body, whose settings wait while a build runs.
    body: RefCell<Option<gtk::Box>>,
    /// Holds the bar under the page, shown while the page is.
    holder: gtk::Box,
}

impl Pending {
    /// Changes for `section`, which is rebuilt after they're applied or discarded;
    /// [`Pending::place`] puts the bar on the page.
    pub fn new(section: &'static str) -> Rc<Pending> {
        let bar = widgets::changes_bar();
        bar.root.add_css_class("nxa-bar");
        bar.apply.set_label("Apply…");
        bar.apply.set_tooltip_text(Some(
            "Built into a new generation and switched to when the build works; it takes a minute or two. An administrator's password is asked for.",
        ));
        // The bar's own icon gives way to a tile that can also show the build running.
        if let Some(old) = bar.root.first_child() {
            old.set_visible(false);
        }
        let tile = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tile.add_css_class("nxa-tile");
        tile.set_valign(gtk::Align::Center);
        tile.set_hexpand(false);
        let icon = gtk::Image::from_icon_name(nyxos::layers_icon());
        icon.set_hexpand(true);
        let spinner = gtk::Spinner::new();
        spinner.set_hexpand(true);
        spinner.set_visible(false);
        tile.append(&icon);
        tile.append(&spinner);
        bar.root.prepend(&tile);

        let chips = gtk::FlowBox::new();
        chips.add_css_class("nxa-chips");
        chips.set_selection_mode(gtk::SelectionMode::None);
        chips.set_column_spacing(6);
        chips.set_row_spacing(6);
        chips.set_max_children_per_line(4);
        chips.set_homogeneous(false);
        let progress = gtk::ProgressBar::new();
        progress.add_css_class("nxa-progress");
        progress.set_visible(false);
        if let Some(text) = bar.label.parent().and_downcast::<gtk::Box>() {
            text.append(&chips);
            text.append(&progress);
        }

        let back = gtk::Button::new();
        back.add_css_class("flat");
        back.set_visible(false);
        let done = gtk::Button::with_label("Done");
        done.set_visible(false);
        for b in [&back, &done] {
            b.set_valign(gtk::Align::Center);
            bar.root.append(b);
        }

        let p = Rc::new(Pending {
            section,
            changes: RefCell::default(),
            bar,
            icon,
            spinner,
            chips,
            progress,
            back,
            done,
            body: RefCell::default(),
            holder: gtk::Box::new(gtk::Orientation::Vertical, 0),
        });
        p.holder.add_css_class("nxa-holder");
        p.holder.append(&p.bar.root);
        let weak = Rc::downgrade(&p);
        p.bar.apply.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.apply();
            }
        });
        p.bar.discard.connect_clicked(move |_| window::rebuild(section));
        let root = p.bar.root.clone();
        p.done.connect_clicked(move |_| root.set_visible(false));
        p.back.connect_clicked(move |b| {
            b.set_sensitive(false);
            nyxos::rollback(move |_| window::rebuild(section));
        });
        if let Some(what) = APPLIED.with(|a| a.borrow_mut().remove(section)) {
            p.show_applied(&what);
        }
        p
    }

    /// The bar, pinned at the foot of the page under its controls and preview, so it
    /// stays in view however far the page is scrolled.
    pub fn place(&self, page: &Page) {
        let body = page.body.clone();
        *self.body.borrow_mut() = Some(body.clone());
        let (holder, root) = (self.holder.clone(), page.root.clone());
        // Once the page is in the window: under the pages, beside nothing else.
        gtk::glib::idle_add_local_once(move || {
            if holder.parent().is_some() {
                return;
            }
            let Some((stack, column)) = root
                .parent()
                .and_then(|stack| Some((stack.clone(), stack.parent()?.downcast::<gtk::Box>().ok()?)))
                .filter(|(stack, column)| stack.is::<gtk::Stack>() && column.orientation() == gtk::Orientation::Vertical)
            else {
                body.append(&holder);
                return;
            };
            column.insert_child_after(&holder, Some(&stack));
            holder.set_visible(root.is_mapped());
            let h = holder.clone();
            root.connect_map(move |_| h.set_visible(true));
            let h = holder.clone();
            root.connect_unmap(move |_| h.set_visible(false));
            // A page rebuilt or gone takes its bar with it.
            root.connect_notify_local(Some("parent"), move |r, _| {
                if r.parent().is_none()
                    && let Some(column) = holder.parent().and_downcast::<gtk::Box>()
                {
                    column.remove(&holder);
                }
            });
        });
    }

    pub fn set(&self, key: &str, change: Option<Change>) {
        let after = change.as_ref().map(after_text).unwrap_or_default();
        self.set_shown(key, change, "", &after);
    }

    /// [`Pending::set`], with what the setting was and will be, for the bar to show as
    /// before → after.
    pub fn set_shown(&self, key: &str, change: Option<Change>, before: &str, after: &str) {
        {
            let mut c = self.changes.borrow_mut();
            let made = c.get(key).map_or_else(|| c.values().map(|w| w.made + 1).max().unwrap_or(0), |w| w.made);
            match change {
                Some(change) => c.insert(key.to_string(), Waiting { change, before: before.into(), after: after.into(), made }),
                None => c.remove(key),
            };
        }
        self.show_waiting();
    }

    fn show_waiting(&self) {
        let c = self.changes.borrow();
        self.bar.set(c.len(), "");
        self.bar.label.set_text(&match c.len() {
            1 => "1 system change waiting".to_string(),
            n => format!("{n} system changes waiting"),
        });
        self.bar.root.remove_css_class("failed");
        self.bar.apply.set_label("Apply…");
        self.icon.set_icon_name(Some(nyxos::layers_icon()));
        while let Some(chip) = self.chips.first_child() {
            self.chips.remove(&chip);
        }
        let mut list: Vec<(&String, &Waiting)> = c.iter().collect();
        list.sort_by_key(|(_, w)| w.made);
        for (key, w) in list {
            self.chips.insert(&chip(key, &w.before, &w.after), -1);
        }
        self.chips.set_visible(!c.is_empty());
        self.bar.apply.set_sensitive(!c.is_empty() && !nyxos::busy());
    }

    fn apply(self: &Rc<Self>) {
        let changes: Vec<Change> = self.changes.borrow().values().map(|w| w.change.clone()).collect();
        let what: Vec<String> = {
            let c = self.changes.borrow();
            let mut list: Vec<(&String, &Waiting)> = c.iter().collect();
            list.sort_by_key(|(_, w)| w.made);
            list.into_iter().map(|(k, _)| change_name(k)).collect()
        };
        self.show_building();
        let section = self.section;
        let weak = Rc::downgrade(self);
        let root = self.bar.root.clone();
        nyxos::apply_quiet(changes, move |r| {
            let here = root.is_mapped();
            match r {
                Ok(()) => {
                    if !here {
                        window::toast("System changes applied");
                    }
                    APPLIED.with(|a| a.borrow_mut().insert(section, what));
                    window::rebuild(section);
                }
                Err(why) => {
                    if !here {
                        window::toast(&format!("The system changes didn't build, so nothing changed. {why}"));
                    }
                    if let Some(p) = weak.upgrade() {
                        p.show_failed(&why);
                    }
                }
            }
        });
    }

    /// The page's settings wait while a build runs (`on` false), and are back after (`on`).
    fn usable(&self, on: bool) {
        let Some(body) = self.body.borrow().clone() else { return };
        let mut c = body.first_child();
        while let Some(child) = c {
            if child.has_css_class("split-page") {
                if let Some(controls) = nyxos::descendant(&child, "split-controls") {
                    controls.set_sensitive(on);
                }
            } else if child != self.holder {
                child.set_sensitive(on);
            }
            c = child.next_sibling();
        }
    }

    fn show_building(&self) {
        let b = &self.bar;
        b.root.set_visible(true);
        b.root.add_css_class("busy");
        b.label.set_text(&match nyxos::next_generation() {
            Some(n) => format!("Building generation {n}"),
            None => "Building the new system".to_string(),
        });
        b.detail.set_text("It's switched to when the build works. Settings here wait until it's done.");
        b.detail.set_visible(true);
        for w in [b.discard.upcast_ref::<gtk::Widget>(), b.apply.upcast_ref(), self.chips.upcast_ref(), self.icon.upcast_ref()] {
            w.set_visible(false);
        }
        self.spinner.set_visible(true);
        self.spinner.start();
        self.progress.set_visible(true);
        self.usable(false);
        let progress = self.progress.downgrade();
        gtk::glib::timeout_add_local(std::time::Duration::from_millis(120), move || {
            let Some(p) = progress.upgrade() else { return gtk::glib::ControlFlow::Break };
            if !p.is_visible() {
                return gtk::glib::ControlFlow::Break;
            }
            p.pulse();
            gtk::glib::ControlFlow::Continue
        });
    }

    fn end_building(&self) {
        self.bar.root.remove_css_class("busy");
        self.spinner.stop();
        self.spinner.set_visible(false);
        self.progress.set_visible(false);
        self.icon.set_visible(true);
        self.usable(true);
    }

    /// It didn't build: nothing changed, and the changes are still waiting.
    fn show_failed(&self, why: &str) {
        self.end_building();
        self.show_waiting();
        let b = &self.bar;
        b.root.add_css_class("failed");
        self.icon.set_icon_name(Some("dialog-warning-symbolic"));
        b.label.set_text("The build didn't work, so nothing changed");
        let why = why.trim().trim_end_matches('.');
        b.detail.set_text(&if why.is_empty() {
            "Your changes are still waiting.".to_string()
        } else {
            format!("{why}. Your changes are still waiting.")
        });
        b.detail.set_visible(true);
        b.discard.set_visible(true);
        b.apply.set_visible(true);
        b.apply.set_label("Try again");
    }

    /// The page was rebuilt after an apply that worked: say so, with a way back.
    fn show_applied(&self, what: &[String]) {
        let b = &self.bar;
        b.root.set_visible(true);
        b.root.add_css_class("ok");
        self.icon.set_icon_name(Some("object-select-symbolic"));
        let host = std::fs::read_to_string("/proc/sys/kernel/hostname").map(|h| h.trim().to_string()).unwrap_or_default();
        let current = nyxos::current_generation();
        b.label.set_text(&match (&current, host.is_empty()) {
            (Some(g), false) => format!("Applied. {host} is on generation {}.", g.number),
            (Some(g), true) => format!("Applied. This computer is on generation {}.", g.number),
            (None, _) => "Applied.".to_string(),
        });
        let mut detail = match what {
            [] => String::new(),
            [one] => format!("{one} changed."),
            [rest @ .., last] => format!("{} and {} changed.", rest.join(", "), last.to_lowercase()),
        };
        if what.iter().any(|w| *w == change_name("locale")) {
            detail.push_str(" Apps use the new language the next time they open.");
        }
        b.detail.set_text(detail.trim());
        b.detail.set_visible(!detail.is_empty());
        for w in [b.discard.upcast_ref::<gtk::Widget>(), b.apply.upcast_ref(), self.chips.upcast_ref()] {
            w.set_visible(false);
        }
        if let Some(n) = nyxos::previous_generation() {
            self.back.set_child(Some(&labelled_icon("document-open-recent-symbolic", &format!("Go back to {n}"))));
            self.back.set_tooltip_text(Some("Switches to the generation before; it stays until the next change."));
            self.back.set_visible(true);
        }
        self.done.set_visible(true);
    }
}

/// A row that shows when its setting has changed and is waiting to be applied: a dot
/// before its title, and what it was in place of its description.
pub struct RowMark {
    dot: gtk::Box,
    desc: gtk::Label,
    normal: String,
}

impl RowMark {
    pub fn new(row: &gtk::Box) -> RowMark {
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("nxa-dot");
        dot.set_valign(gtk::Align::Center);
        dot.set_visible(false);
        let line = nyxos::title_line(row);
        if let Some(line) = &line {
            line.prepend(&dot);
        }
        let desc = nyxos::descendant(row, "settings-option-description").and_downcast::<gtk::Label>().unwrap_or_else(|| {
            let d = widgets::label("", "settings-option-description");
            d.set_wrap(true);
            d.set_visible(false);
            if let Some(text) = line.as_ref().and_then(|l| l.parent()).and_downcast::<gtk::Box>() {
                text.append(&d);
            }
            d
        });
        let normal = desc.text().to_string();
        RowMark { dot, desc, normal }
    }

    /// Changed from `was`, or back as it is (`None`).
    pub fn set(&self, was: Option<&str>) {
        self.dot.set_visible(was.is_some());
        let text = match was {
            Some(w) => format!("Was {w}"),
            None => self.normal.clone(),
        };
        self.desc.set_text(&text);
        self.desc.set_visible(!text.is_empty());
    }
}

fn labelled_icon(icon: &str, text: &str) -> gtk::Box {
    let b = widgets::hbox(6);
    b.append(&gtk::Image::from_icon_name(icon));
    b.append(&gtk::Label::new(Some(text)));
    b
}

/// One waiting change, as its name and before → after.
fn chip(key: &str, before: &str, after: &str) -> gtk::Box {
    let c = widgets::hbox(6);
    c.add_css_class("nxa-chip");
    c.set_halign(gtk::Align::Start);
    let i = gtk::Image::from_icon_name(change_icon(key));
    c.append(&i);
    c.append(&gtk::Label::new(Some(&change_name(key))));
    if !before.is_empty() {
        let was = widgets::label(before, "nxa-was");
        c.append(&was);
    }
    if !after.is_empty() {
        let arrow = gtk::Image::from_icon_name("go-next-symbolic");
        arrow.add_css_class("nxa-arrow");
        c.append(&arrow);
        c.append(&widgets::label(after, "nxa-now"));
    }
    c.set_tooltip_text(Some(&match (before.is_empty(), after.is_empty()) {
        (false, false) => format!("{}: {before} → {after}", change_name(key)),
        _ => change_name(key),
    }));
    c
}

/// What a waiting change is, in a word or two, from its key.
fn change_name(key: &str) -> String {
    match key {
        "time-zone" => "Time zone".into(),
        "locale" => "Language and formats".into(),
        "keyboard" => "Keyboard layout".into(),
        "hostname" => "This computer's name".into(),
        "boot" => "Boot screen".into(),
        "login" => "Sign-in screen".into(),
        "logo" => "Logo".into(),
        "full-name" => "Your name".into(),
        k => match k.strip_prefix("admin-") {
            Some(user) => format!("{user}'s account"),
            None => k.strip_prefix("service-").or_else(|| k.strip_prefix("group-")).unwrap_or(k).to_string(),
        },
    }
}

fn change_icon(key: &str) -> &'static str {
    match key {
        "time-zone" => "web-browser-symbolic",
        "locale" => "preferences-desktop-locale-symbolic",
        "keyboard" => "input-keyboard-symbolic",
        "hostname" => "computer-symbolic",
        "boot" | "login" | "logo" => "preferences-desktop-wallpaper-symbolic",
        k if k.starts_with("group-") || k.starts_with("admin-") => "system-users-symbolic",
        "full-name" => "avatar-default-symbolic",
        _ => "emblem-system-symbolic",
    }
}

/// What a change sets, in a word, for the bar when the page didn't say.
fn after_text(c: &Change) -> String {
    let on = |v: &Option<bool>| match v {
        Some(true) => "On",
        Some(false) => "Off",
        None => "Default",
    };
    match c {
        Change::Hostname { value }
        | Change::TimeZone { value }
        | Change::Locale { value }
        | Change::KeyboardLayout { value } => value.clone().unwrap_or_else(|| "Default".into()),
        Change::BootTheme { value } | Change::LoginTheme { value } => {
            value.clone().unwrap_or_else(|| "Same as the system theme".into())
        }
        Change::Service { enabled, .. } | Change::NyxApp { enabled, .. } | Change::Firewall { enabled } => on(enabled).into(),
        Change::GroupMember { member, .. } => if *member { "Joined" } else { "Left" }.into(),
        Change::PersonAdmin { admin, .. } => if *admin { "Administrator" } else { "Standard" }.into(),
        Change::FullName { full_name, .. } => full_name.clone(),
        _ => String::new(),
    }
}
