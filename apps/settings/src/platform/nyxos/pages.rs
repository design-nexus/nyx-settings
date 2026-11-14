//! NyxOS's system pages: services and groups, software, and the boot and sign-in
//! screens. Changes are collected on the page and applied together through the
//! Nyx System Manager, as one rebuild.

use super::pending::Pending;
use crate::platform::nyxos::{self, tag_system};
use crate::sections::services::{Unit, parse_units};
use crate::sketch::{self, R, Rgb};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::cairo::{self, Context, ImageSurface};
use gtk::prelude::*;
use gtk::{gio, glib, pango};
use nyx_system_proto::{Change, GROUPS, SERVICES, SystemSettings};
use nyx_theme::engine::logo as brand;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::f64::consts::{PI, TAU};
use std::rc::Rc;

fn options(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

// ---------- Parts the NyxOS pages share ----------

/// A page's changes waiting for Apply, each kept under the words the bar shows for
/// it. `item` is what a change is about, so a second change to it replaces the first.
pub(super) struct Staged {
    pending: Rc<Pending>,
    keys: RefCell<HashMap<String, String>>,
}

impl Staged {
    pub(super) fn new(section: &'static str) -> Rc<Staged> {
        Rc::new(Staged { pending: Pending::new(section), keys: RefCell::default() })
    }

    pub(super) fn set(&self, item: &str, change: Option<(String, Change)>) {
        let old = self.keys.borrow_mut().remove(item);
        if let Some(old) = old {
            self.pending.set(&old, None);
        }
        if let Some((words, change)) = change {
            self.pending.set(&words, Some(change));
            self.keys.borrow_mut().insert(item.to_string(), words);
        }
    }

    pub(super) fn waiting(&self, item: &str) -> bool {
        self.keys.borrow().contains_key(item)
    }

    /// The bar, at the foot of the page's controls.
    pub(super) fn place(&self, page: &Page) {
        self.pending.place(page);
    }
}

pub(super) fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// A group whose title has something at its right (a note, a "Changed" pill).
pub(super) fn head_group(page: &Page, title: &str, end: &[&gtk::Widget], card: bool) -> widgets::Group {
    let g = if card { page.group("") } else { page.plain_group("") };
    let head = widgets::hbox(8);
    head.add_css_class("nx2-group-head");
    let l = widgets::label(&title.to_uppercase(), "group-title");
    l.set_hexpand(true);
    l.set_valign(gtk::Align::Center);
    head.append(&l);
    for w in end {
        w.set_valign(gtk::Align::Center);
        head.append(*w);
    }
    g.top(&head);
    g
}

/// A short note at the right of a heading.
pub(super) fn head_note(text: &str) -> gtk::Label {
    widgets::label(text, "nx2-head-note")
}

/// A preview heading that's rebuilt with what it shows: the box, its title and line.
pub(super) fn preview_head(title: &str, line: &str, end: &[&gtk::Widget]) -> (gtk::Box, gtk::Label, gtk::Label) {
    let head = widgets::hbox(10);
    head.add_css_class("preview-head");
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(title, "preview-title");
    t.set_wrap(true);
    text.append(&t);
    let l = widgets::label(line, "preview-line");
    l.set_wrap(true);
    l.set_visible(!line.is_empty());
    text.append(&l);
    head.append(&text);
    for w in end {
        w.set_valign(gtk::Align::Center);
        head.append(*w);
    }
    (head, t, l)
}

/// A small heading inside a preview, with something at its right.
pub(super) fn sub_head(title: &str, end: Option<&gtk::Widget>) -> gtk::Box {
    let h = widgets::hbox(8);
    h.add_css_class("nx2-sub-head");
    let l = widgets::label(title, "nx2-sub");
    l.set_hexpand(true);
    h.append(&l);
    if let Some(e) = end {
        e.set_valign(gtk::Align::Center);
        h.append(e);
    }
    h
}

/// A card of rows, for previews.
pub(super) fn card() -> gtk::Box {
    let c = widgets::vbox(0);
    c.add_css_class("settings-card");
    c.add_css_class("nx2-card");
    c.set_overflow(gtk::Overflow::Hidden);
    c
}

/// A row for a preview: like [`widgets::row`], but not offered to search, since a
/// preview's rows are drawn again whenever what it shows changes.
pub(super) fn plain_row(icon: Option<&str>, title: &str, desc: &str, end: Option<&gtk::Widget>) -> gtk::Box {
    let r = widgets::hbox(14);
    r.add_css_class("settings-option");
    r.add_css_class("card-row");
    if let Some(i) = icon {
        r.append(&widgets::tile(i));
        r.add_css_class("with-tile");
    }
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(title, "settings-option-title");
    t.set_wrap(true);
    text.append(&t);
    if !desc.is_empty() {
        let d = widgets::label(desc, "settings-option-description");
        d.set_wrap(true);
        d.set_wrap_mode(pango::WrapMode::WordChar);
        text.append(&d);
    }
    r.append(&text);
    if let Some(e) = end {
        e.set_valign(gtk::Align::Center);
        r.append(e);
    }
    r
}

/// Small facts in columns: a dim name over its value.
pub(super) fn facts(pairs: &[(&str, String)], columns: usize) -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.add_css_class("preview-card");
    grid.add_css_class("nx2-facts");
    grid.set_column_homogeneous(true);
    grid.set_row_spacing(12);
    grid.set_column_spacing(16);
    for (i, (k, v)) in pairs.iter().enumerate() {
        let cell = widgets::vbox(2);
        cell.append(&widgets::label(k, "nx2-fact-key"));
        let value = widgets::label(v, "nx2-fact-value");
        value.set_wrap(true);
        cell.append(&value);
        grid.attach(&cell, (i % columns) as i32, (i / columns) as i32, 1, 1);
    }
    grid
}

/// A rounded word: "Changed", "Joins", "Running".
pub(super) fn pill(text: &str, kind: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("nx2-pill");
    if !kind.is_empty() {
        l.add_css_class(kind);
    }
    l.set_valign(gtk::Align::Center);
    l
}

/// A button that reads as a link.
pub(super) fn link(label: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("nx2-link");
    b.set_valign(gtk::Align::Center);
    b
}

/// A coloured dot for a state: `ok`, `off` or `wait`.
pub(super) fn dot(kind: &str) -> gtk::Box {
    let d = widgets::hbox(0);
    d.add_css_class("nx2-dot");
    d.add_css_class(kind);
    d.set_valign(gtk::Align::Center);
    d
}

fn set_dot(d: &gtk::Box, kind: &str) {
    for k in ["ok", "off", "wait"] {
        d.remove_css_class(k);
    }
    d.add_css_class(kind);
}

/// A warning in a line: an icon and the words.
pub(super) fn warning(text: &str) -> gtk::Box {
    let b = widgets::hbox(6);
    b.add_css_class("nx2-warn");
    b.append(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
    let l = widgets::label(text, "");
    l.set_wrap(true);
    b.append(&l);
    b
}

/// A row's description label, to change its words after it's made.
pub(super) fn description_of(row: &gtk::Widget) -> Option<gtk::Label> {
    let mut c = row.first_child();
    while let Some(w) = c {
        if let Some(l) = w.downcast_ref::<gtk::Label>()
            && l.has_css_class("settings-option-description")
        {
            return Some(l.clone());
        }
        if let Some(l) = description_of(&w) {
            return Some(l);
        }
        c = w.next_sibling();
    }
    None
}

/// Text drawn in a picture: `align` 0 puts its start at `x`, 0.5 its middle, 1 its end.
#[allow(clippy::too_many_arguments)]
pub(super) fn text(cr: &Context, s: &str, x: f64, y: f64, size: f64, bold: bool, c: Rgb, alpha: f64, align: f64) -> f64 {
    cr.select_font_face(
        "sans-serif",
        cairo::FontSlant::Normal,
        if bold { cairo::FontWeight::Bold } else { cairo::FontWeight::Normal },
    );
    cr.set_font_size(size);
    let Ok(e) = cr.text_extents(s) else { return 0.0 };
    sketch::set(cr, c, alpha);
    cr.move_to(x - e.x_advance() * align, y);
    let _ = cr.show_text(s);
    // Text leaves a current point, which the next arc would draw a line from.
    cr.new_path();
    e.x_advance()
}

/// The accounts the System Manager knows on this computer, signed-in one first:
/// (login, full name).
pub(super) fn people(s: &SystemSettings) -> Vec<(String, String)> {
    let me = std::env::var("USER").unwrap_or_default();
    let mut logins: Vec<String> = vec![me.clone()];
    for l in s.people.keys().chain(s.full_names.keys()).chain(s.groups.values().flatten()) {
        if !logins.contains(l) {
            logins.push(l.clone());
        }
    }
    logins.retain(|l| !l.is_empty());
    logins
        .into_iter()
        .map(|l| {
            let name = s
                .full_names
                .get(&l)
                .cloned()
                .or_else(|| s.people.get(&l).map(|p| p.full_name.clone()))
                .filter(|n| !n.is_empty())
                .or_else(|| {
                    (l == me)
                        .then(|| glib::real_name().to_string_lossy().into_owned())
                        .filter(|n| !n.is_empty() && n != "Unknown")
                })
                .unwrap_or_else(|| l.clone());
            (l, name)
        })
        .collect()
}

pub(super) fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).take(2).flat_map(char::to_uppercase).collect()
}

/// A person's circle: their initials on a colour picked from their name.
pub(super) fn draw_avatar(cr: &Context, name: &str, cx: f64, cy: f64, r: f64) {
    const COLOURS: [Rgb; 6] =
        [(0.91, 0.42, 0.38), (0.36, 0.55, 0.93), (0.62, 0.45, 0.9), (0.3, 0.7, 0.5), (0.93, 0.62, 0.25), (0.84, 0.4, 0.68)];
    let seed = name.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    cr.arc(cx, cy, r, 0.0, TAU);
    sketch::set(cr, COLOURS[seed as usize % COLOURS.len()], 1.0);
    let _ = cr.fill();
    let s = initials(name);
    cr.select_font_face("sans-serif", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(r * 0.78);
    if let Ok(e) = cr.text_extents(&s) {
        sketch::set(cr, (1.0, 1.0, 1.0), 0.95);
        cr.move_to(cx - e.width() / 2.0 - e.x_bearing(), cy - e.height() / 2.0 - e.y_bearing());
        let _ = cr.show_text(&s);
    }
}

pub(super) fn avatar(name: &str, size: i32) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(size);
    a.set_content_height(size);
    a.set_valign(gtk::Align::Center);
    let name = name.to_string();
    a.set_draw_func(move |_, cr, w, h| {
        let r = w.min(h) as f64 / 2.0;
        draw_avatar(cr, &name, w as f64 / 2.0, h as f64 / 2.0, r);
    });
    a
}

/// Space above a group without a title, which would otherwise sit right under the last.
pub(super) fn gap(g: &widgets::Group) {
    g.wrapper.add_css_class("nx2-gap");
}

/// A column for a preview's content.
pub(super) fn preview_column(pv: &widgets::Preview) -> gtk::Box {
    let column = widgets::vbox(14);
    column.set_hexpand(true);
    pv.add(&column);
    column
}

/// Put something in the foot line of a preview, at its right.
pub(super) fn foot_end(foot: &gtk::Label, w: &impl IsA<gtk::Widget>) {
    if let Some(line) = foot.parent().and_downcast::<gtk::Box>() {
        w.set_hexpand(false);
        line.append(w);
    }
}

/// The children of a box, in order.
fn children(b: &gtk::Box) -> Vec<gtk::Widget> {
    std::iter::successors(b.first_child(), |w| w.next_sibling()).collect()
}

// ---------- Services ----------

/// What the Services page says about one of the System Manager's services.
struct Info {
    short: &'static str,
    icon: &'static str,
    line: &'static str,
    /// What turning it on brings: (icon, what, a line about it).
    adds: &'static [(&'static str, &'static str, &'static str)],
    /// The group that lets people use it.
    group: Option<&'static str>,
    /// Units that are there while it's on.
    units: &'static [&'static str],
}

fn info(name: &str) -> Info {
    let (short, icon, line, adds, group, units): (_, _, _, &'static [(&str, &str, &str)], _, &'static [&str]) = match name {
        "bluetooth" => (
            "Bluetooth",
            "bluetooth-symbolic",
            "Connects headphones, keyboards, mice and phones without wires.",
            &[("bluetooth-symbolic", "The Bluetooth service", "BlueZ, so devices can pair with this computer")],
            None,
            &["bluetooth.service"],
        ),
        "printing" => (
            "Printing",
            "printer-symbolic",
            "Prints to printers on your network or plugged in.",
            &[("printer-symbolic", "The printing service", "CUPS, with printers added on the Printers page")],
            None,
            &["cups.service"],
        ),
        "ssh" => (
            "Remote login",
            "utilities-terminal-symbolic",
            "Sign in to this computer from another one, in a terminal.",
            &[
                ("utilities-terminal-symbolic", "The SSH server", "sshd, started with the computer"),
                ("security-high-symbolic", "A port in the firewall", "Port 22, so other computers can reach it"),
            ],
            None,
            &["sshd.service"],
        ),
        "flatpak" => (
            "Flatpak apps",
            "system-software-install-symbolic",
            "Installs apps from Flathub, each in its own sandbox.",
            &[("system-software-install-symbolic", "Flatpak", "Flathub's apps in Nyx Store, updated with Applications")],
            None,
            &["flatpak-system-helper.service"],
        ),
        "docker" => (
            "Docker",
            "package-x-generic-symbolic",
            "Runs containers for development and self-hosted apps.",
            &[
                ("network-server-symbolic", "The Docker service", "dockerd and containerd, started with the computer"),
                ("system-users-symbolic", "A docker group", "People in it run containers without a password"),
                ("network-wired-symbolic", "A network for containers", "docker0, so containers reach each other and the internet"),
            ],
            Some("docker"),
            &["docker.service"],
        ),
        "tailscale" => (
            "Tailscale",
            "network-vpn-symbolic",
            "Joins this computer to your private Tailscale network.",
            &[("network-vpn-symbolic", "The Tailscale service", "tailscaled; tailscale up signs this computer in")],
            None,
            &["tailscaled.service"],
        ),
        "fwupd" => (
            "Firmware updates",
            "media-flash-symbolic",
            "Finds firmware updates for devices that support them.",
            &[("media-flash-symbolic", "The firmware service", "fwupd, with updates from LVFS on the Updates page")],
            None,
            &["fwupd.service"],
        ),
        "avahi" => (
            "Network discovery",
            "network-workgroup-symbolic",
            "Finds printers, shares and other computers on your network.",
            &[("network-workgroup-symbolic", "The discovery service", "Avahi, which finds things on your network by name")],
            None,
            &["avahi-daemon.service"],
        ),
        "podman" => (
            "Podman",
            "package-x-generic-symbolic",
            "Runs containers, each person their own, without a group.",
            &[("package-x-generic-symbolic", "Podman", "Containers that need no password and no administrator")],
            None,
            &["podman.service"],
        ),
        "incus" => (
            "Incus",
            "network-server-symbolic",
            "Runs system containers and virtual machines.",
            &[
                ("network-server-symbolic", "The Incus service", "incusd, started with the computer"),
                ("system-users-symbolic", "An incus-admin group", "People in it manage containers and virtual machines"),
                ("security-high-symbolic", "The nftables firewall", "Incus needs it for its networks"),
            ],
            Some("incus-admin"),
            &["incus.service"],
        ),
        "libvirtd" => (
            "Virtual machines",
            "computer-symbolic",
            "Runs whole other operating systems in windows.",
            &[
                ("network-server-symbolic", "The libvirt service", "libvirtd, started with the computer"),
                ("system-users-symbolic", "A libvirtd group", "People in it manage the system's virtual machines"),
            ],
            Some("libvirtd"),
            &["libvirtd.service"],
        ),
        "ollama" => (
            "Ollama",
            "applications-science-symbolic",
            "Runs AI models on this computer, without the internet.",
            &[("applications-science-symbolic", "The Ollama service", "Serves models to apps on this computer, at localhost:11434")],
            None,
            &["ollama.service"],
        ),
        _ => ("", "emblem-system-symbolic", "", &[], None, &[]),
    };
    Info { short, icon, line, adds, group, units }
}

/// Groups whose members can do anything an administrator can.
fn as_admin(group: &str) -> bool {
    matches!(group, "docker" | "incus-admin")
}

#[derive(Clone, Copy, PartialEq)]
enum Want {
    Default,
    On,
    Off,
}

impl Want {
    fn of(v: Option<bool>) -> Want {
        match v {
            Some(true) => Want::On,
            Some(false) => Want::Off,
            None => Want::Default,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Want::Default => "default",
            Want::On => "on",
            Want::Off => "off",
        }
    }

    fn from_id(id: &str) -> Want {
        match id {
            "on" => Want::On,
            "off" => Want::Off,
            _ => Want::Default,
        }
    }

    fn enabled(self) -> Option<bool> {
        match self {
            Want::On => Some(true),
            Want::Off => Some(false),
            Want::Default => None,
        }
    }
}

struct Svc {
    name: &'static str,
    what: &'static str,
    info: Info,
    /// As it's set now.
    now: Want,
    /// Whether it's on in the running system, when that can be told.
    running: Option<bool>,
    /// As it will be after Apply.
    want: Cell<Want>,
    line: gtk::Label,
    dot: gtk::Box,
}

impl Svc {
    fn on_now(&self) -> Option<bool> {
        match self.now {
            Want::On => Some(true),
            Want::Off => Some(false),
            Want::Default => self.running,
        }
    }

    /// The tile's line: on or off, whether that's NyxOS's default, or what Apply does.
    fn show_state(&self) {
        let want = self.want.get();
        let (main, tail, kind) = if want != self.now {
            let main = match want {
                Want::On => "On",
                Want::Off => "Off",
                Want::Default => "Default",
            };
            (main, " after Apply", "wait")
        } else {
            let default = if self.now == Want::Default && self.running.is_some() { " · default" } else { "" };
            match self.on_now() {
                Some(true) => ("On", default, "ok"),
                Some(false) => ("Off", default, "off"),
                None => ("Default", "", "off"),
            }
        };
        self.line.set_markup(&format!("{main}<span alpha=\"60%\">{tail}</span>"));
        set_dot(&self.dot, kind);
    }
}

type MemberKey = (String, String);

/// The Services page while it's open.
struct Services {
    staged: Rc<Staged>,
    list: Vec<Svc>,
    people: Vec<(String, String)>,
    /// Who's in each group now, and who will be after Apply: (group, login) → member.
    members_now: BTreeMap<MemberKey, bool>,
    members: RefCell<BTreeMap<MemberKey, bool>>,
    /// Every switch showing a membership, with its "Joins" pill, to keep them alike.
    switches: RefCell<Vec<(MemberKey, glib::WeakRef<gtk::Switch>, glib::WeakRef<gtk::Label>)>>,
    detail: gtk::Box,
    foot: gtk::Label,
    picked: Cell<usize>,
}

const UNITS: &[&str] = &["list-units", "--type=service", "--all", "--plain", "--no-legend", "--no-pager"];
const UNIT_FILES: &[&str] = &["list-unit-files", "--type=service", "--plain", "--no-legend", "--no-pager"];

/// systemctl's services (the same reads the services lists make, fetched ahead).
fn units(user: bool) -> (Vec<Unit>, String) {
    let base: &[&str] = if user { &["systemctl", "--user"] } else { &["systemctl"] };
    let read = |extra: &[&str]| cmd::read(&[base, extra].concat()).unwrap_or_default();
    let files = read(UNIT_FILES);
    (parse_units(&read(UNITS), &files), files)
}

pub fn services(page: &Page) {
    let s = nyxos::settings();
    let (mine, _) = units(true);
    let (system, files) = units(false);
    let states: HashMap<&str, &str> = files
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some((f.next()?, f.next()?))
        })
        .collect();
    let people = people(&s);
    let mut members_now = BTreeMap::new();
    for (group, _) in GROUPS {
        let m = s.groups.get(*group);
        for (login, _) in &people {
            members_now.insert((group.to_string(), login.clone()), m.is_some_and(|m| m.contains(login)));
        }
    }

    let pv = page.split();
    let detail = preview_column(&pv);
    let foot = pv.foot("dialog-information-symbolic", "");

    let list = SERVICES
        .iter()
        .map(|(name, what)| {
            let info = info(name);
            let running = (!states.is_empty())
                .then(|| info.units.iter().any(|u| states.get(u).is_some_and(|st| !matches!(*st, "masked" | "disabled"))));
            let now = Want::of(s.services.get(*name).copied());
            let line = widgets::label("", "nx2-svc-line");
            line.set_ellipsize(pango::EllipsizeMode::End);
            Svc { name, what, info, now, running, want: Cell::new(now), line, dot: dot("off") }
        })
        .collect();
    let c = Rc::new(Services {
        staged: Staged::new("services"),
        list,
        people,
        members: RefCell::new(members_now.clone()),
        members_now,
        switches: RefCell::default(),
        detail,
        foot,
        picked: Cell::new(0),
    });

    // The switchboard: a tile for each service, which opens it in the preview.
    let note = head_note("Default is what NyxOS chooses");
    let g = head_group(page, "System services", &[note.upcast_ref()], false);
    let grid = gtk::Grid::new();
    grid.add_css_class("nx2-svc-grid");
    grid.set_column_homogeneous(true);
    grid.set_row_spacing(8);
    grid.set_column_spacing(8);
    let mut first: Option<gtk::ToggleButton> = None;
    for (i, svc) in c.list.iter().enumerate() {
        let b = gtk::ToggleButton::new();
        b.add_css_class("nx2-svc-tile");
        b.set_tooltip_text(Some(svc.what));
        let row = widgets::hbox(10);
        row.append(&widgets::tile(svc.info.icon));
        let text = widgets::vbox(1);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        let name = widgets::label(if svc.info.short.is_empty() { svc.what } else { svc.info.short }, "nx2-svc-name");
        name.set_ellipsize(pango::EllipsizeMode::End);
        text.append(&name);
        let state = widgets::hbox(6);
        state.append(&svc.dot);
        state.append(&svc.line);
        text.append(&state);
        row.append(&text);
        b.set_child(Some(&row));
        match &first {
            Some(f) => b.set_group(Some(f)),
            None => first = Some(b.clone()),
        }
        let w = Rc::downgrade(&c);
        b.connect_toggled(move |b| {
            if b.is_active()
                && let Some(c) = w.upgrade()
            {
                show_service(&c, i);
            }
        });
        svc.show_state();
        grid.attach(&b, (i % 2) as i32, (i / 2) as i32, 1, 1);
    }
    let holder = widgets::stacked_row("", "", grid.upcast_ref());
    holder.add_css_class("bare");
    let words: Vec<String> = SERVICES.iter().map(|(n, w)| format!("{n} {w}")).collect();
    widgets::keywords(&format!("system services default on off {}", words.join(" ")));
    g.add(&holder);
    if let Some(f) = &first {
        f.set_active(true);
    }

    // Your own services, the whole computer's, and the groups, each a level down.
    let g = page.group("");
    gap(&g);
    let running = |l: &[Unit]| l.iter().filter(|u| u.active == "active").count();
    let (r, v) = widgets::nav_row(
        Some("avatar-default-symbolic"),
        "Your services",
        "Run as you, from when you sign in",
        &format!("{} running", running(&mine)),
        || lists_subpage(true),
    );
    v.set_ellipsize(pango::EllipsizeMode::None);
    widgets::keywords("service daemon systemd unit start stop restart enable disable user");
    g.add(&r);
    let (r, _) = widgets::nav_row(
        Some("view-list-symbolic"),
        "Everything running on this computer",
        "The whole computer's services, for reference",
        &running(&system).to_string(),
        || lists_subpage(false),
    );
    widgets::keywords("running on this computer system units");
    g.add(&r);
    let w = Rc::downgrade(&c);
    let (r, _) = widgets::nav_row(
        Some("system-users-symbolic"),
        "Groups",
        "Who may use containers, virtual machines and devices",
        "",
        move || {
            if let Some(c) = w.upgrade() {
                groups_subpage(&c);
            }
        },
    );
    let names: Vec<&str> = GROUPS.iter().map(|(g, _)| *g).collect();
    widgets::keywords(&format!("groups members {}", names.join(" ")));
    g.add(&r);
    c.staged.place(page);
}

/// Your services, or the whole computer's, on a page of their own.
fn lists_subpage(user: bool) {
    widgets::subpage(if user { "Your services" } else { "Everything running on this computer" }, 640, |p| {
        crate::sections::services::user_and_system_lists(p);
        let groups = children(&p.body);
        for (i, w) in groups.iter().enumerate() {
            w.set_visible((i == 0) == user);
        }
        if !user && let Some(w) = groups.get(1) {
            widgets::reveal_for_search(Some(w));
        }
    });
}

/// Every group, and who's in it.
fn groups_subpage(c: &Rc<Services>) {
    let c = c.clone();
    widgets::subpage("Groups", 560, move |p| {
        p.banner(
            "Being in a group lets an account use containers, virtual machines or devices directly. People see a change the next time they sign in.",
            false,
        );
        for (group, what) in GROUPS {
            let g = p.group(group);
            g.note(what);
            for (login, name) in &c.people {
                let (sw, joins) = member_switch(&c, group, login, name);
                let end = widgets::hbox(8);
                end.append(&joins);
                end.append(&sw);
                let r = widgets::row(name, login, Some(end.upcast_ref()));
                r.prepend(&avatar(name, 28));
                r.add_css_class("with-tile");
                g.add(&r);
            }
        }
    });
}

/// A switch for whether `login` is in `group`, kept alike with every other one for it.
fn member_switch(c: &Rc<Services>, group: &str, login: &str, name: &str) -> (gtk::Switch, gtk::Label) {
    let key = (group.to_string(), login.to_string());
    let on = c.members.borrow().get(&key).copied().unwrap_or(false);
    let was = c.members_now.get(&key).copied().unwrap_or(false);
    let sw = gtk::Switch::new();
    sw.set_active(on);
    let joins = pill(if on { "Joins" } else { "Leaves" }, "accent");
    joins.set_visible(on != was);
    {
        let mut list = c.switches.borrow_mut();
        list.retain(|(_, s, _)| s.upgrade().is_some());
        list.push((key.clone(), sw.downgrade(), joins.downgrade()));
    }
    let (w, name) = (Rc::downgrade(c), name.to_string());
    sw.connect_active_notify(move |s| {
        let Some(c) = w.upgrade() else { return };
        let on = s.is_active();
        if c.members.borrow().get(&key).copied() == Some(on) {
            return;
        }
        c.members.borrow_mut().insert(key.clone(), on);
        let (group, user) = &key;
        let words = format!("{name} {} {group}", if on { "joins" } else { "leaves" });
        c.staged.set(
            &format!("group-{group}-{user}"),
            (on != was).then(|| (words, Change::GroupMember { group: group.clone(), user: user.clone(), member: on })),
        );
        let alike: Vec<_> =
            c.switches.borrow().iter().filter(|(k, _, _)| *k == key).map(|(_, s, p)| (s.upgrade(), p.upgrade())).collect();
        for (s2, p2) in alike {
            if let Some(p2) = p2 {
                p2.set_text(if on { "Joins" } else { "Leaves" });
                p2.set_visible(on != was);
            }
            if let Some(s2) = s2 {
                s2.set_active(on);
            }
        }
    });
    (sw, joins)
}

/// Now, or after Apply: a caption, a dot and a word, and a line under them.
fn state_block(caption: &str, word: &str, kind: &str, line: &str) -> gtk::Box {
    let b = widgets::vbox(3);
    b.set_hexpand(true);
    b.append(&widgets::label(&caption.to_uppercase(), "nx2-caps"));
    let w = widgets::hbox(8);
    w.append(&dot(kind));
    w.append(&widgets::label(word, "nx2-big"));
    b.append(&w);
    let l = widgets::label(line, "nx2-dim");
    l.set_wrap(true);
    b.append(&l);
    b
}

/// The preview of service `i`: Default, On or Off, what it is now and after Apply,
/// what it adds, and who may use it.
fn show_service(c: &Rc<Services>, i: usize) {
    c.picked.set(i);
    let svc = &c.list[i];
    clear(&c.detail);
    let w = Rc::downgrade(c);
    let seg = widgets::segmented(&options(&[("default", "Default"), ("on", "On"), ("off", "Off")]), svc.want.get().id(), move |v| {
        if let Some(c) = w.upgrade() {
            choose_service(&c, i, Want::from_id(&v));
        }
    });
    let (head, _, _) = preview_head(svc.what, svc.info.line, &[seg.upcast_ref()]);
    c.detail.append(&head);

    let flip = widgets::hbox(14);
    flip.add_css_class("preview-card");
    flip.add_css_class("nx2-svc-flip");
    let word = |on: Option<bool>| match on {
        Some(true) => ("On", "ok"),
        Some(false) => ("Off", "off"),
        None => ("Default", "off"),
    };
    let (w1, k1) = word(svc.on_now());
    flip.append(&state_block("Now", w1, k1, if svc.now == Want::Default { "NyxOS's default" } else { "Set on this page" }));
    let arrow = widgets::hbox(0);
    arrow.add_css_class("nx2-svc-arrow");
    arrow.set_valign(gtk::Align::Center);
    arrow.append(&gtk::Image::from_icon_name("go-next-symbolic"));
    flip.append(&arrow);
    let want = svc.want.get();
    let after = if want == svc.now {
        let b = state_block("After you apply", w1, k1, "Nothing waiting");
        b.add_css_class("nx2-faded");
        b
    } else {
        match want {
            Want::On => state_block("After you apply", "On", "ok", "Starts with the computer"),
            Want::Off => state_block("After you apply", "Off", "off", "Doesn't start"),
            Want::Default => state_block("After you apply", "Default", "wait", "Whatever NyxOS chooses"),
        }
    };
    flip.append(&after);
    c.detail.append(&flip);

    if !svc.info.adds.is_empty() {
        c.detail.append(&sub_head("Turning it on adds", None));
        let list = card();
        for (icon, t, d) in svc.info.adds {
            list.append(&plain_row(Some(icon), t, d, None));
        }
        c.detail.append(&list);
    }
    if let Some(group) = svc.info.group {
        let warn = as_admin(group).then(|| warning(&format!("Being in {group} is as good as being an administrator")));
        c.detail.append(&sub_head("Who can use it", warn.as_ref().map(|w| w.upcast_ref())));
        let list = card();
        for (login, name) in &c.people {
            let (sw, joins) = member_switch(c, group, login, name);
            let r = widgets::hbox(10);
            r.add_css_class("settings-option");
            r.add_css_class("card-row");
            r.add_css_class("nx2-person");
            r.append(&avatar(name, 26));
            r.append(&widgets::label(name, "settings-option-title"));
            r.append(&joins);
            let gap = widgets::hbox(0);
            gap.set_hexpand(true);
            r.append(&gap);
            sw.set_valign(gtk::Align::Center);
            r.append(&sw);
            list.append(&r);
        }
        c.detail.append(&list);
    }
    c.foot.set_text(match svc.name {
        "docker" => "Podman runs the same containers without a group, if you'd rather.",
        "ssh" => "Away from home, Tailscale is a safer way in.",
        _ => "Changes wait in the bar below the services and are applied together, as one rebuild.",
    });
}

fn choose_service(c: &Rc<Services>, i: usize, want: Want) {
    let svc = &c.list[i];
    if svc.want.get() == want {
        return;
    }
    svc.want.set(want);
    svc.show_state();
    let words = match want {
        Want::On => format!("{} on", svc.what),
        Want::Off => format!("{} off", svc.what),
        Want::Default => format!("{} back to default", svc.what),
    };
    c.staged.set(
        &format!("service-{}", svc.name),
        (want != svc.now).then(|| (words, Change::Service { name: svc.name.to_string(), enabled: want.enabled() })),
    );
    // Drawn again once the segmented control that asked has finished.
    let w = Rc::downgrade(c);
    glib::idle_add_local_once(move || {
        if let Some(c) = w.upgrade()
            && c.picked.get() == i
        {
            show_service(&c, i);
        }
    });
}

// ---------- Software ----------

/// `nyx-system catalog installed --json`.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Installed {
    #[serde(default)]
    for_you: Vec<Mine>,
    /// Per-user software installed for everyone.
    #[serde(default)]
    movable: Vec<Movable>,
}

#[derive(Debug, serde::Deserialize)]
struct Movable {
    id: String,
    name: String,
    #[serde(default)]
    packages: Vec<String>,
    #[serde(default)]
    nyx: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct Mine {
    id: String,
    name: String,
    version: Option<String>,
    from: String,
}

enum Kind {
    /// Yours, by its catalog id.
    Mine(String),
    /// For everyone, and can be moved to you: its catalog id.
    Movable(String),
    /// A nixpkgs attribute for everyone; whether its license is unfree.
    Package(bool),
    /// A Nyx app turned on or off for everyone.
    NyxApp(String, bool),
}

struct Item {
    name: String,
    version: Option<String>,
    from: String,
    kind: Kind,
    /// The app as the desktop knows it, to open it and show its icon.
    app: Option<gio::AppInfo>,
}

impl Item {
    fn line(&self) -> String {
        match &self.kind {
            Kind::Mine(_) => match &self.version {
                Some(v) => format!("{v} · {}", self.from),
                None => self.from.clone(),
            },
            Kind::Movable(_) => "For everyone; can be yours".into(),
            Kind::Package(_) => "From nixpkgs".into(),
            Kind::NyxApp(_, on) => if *on { "Turned on" } else { "Turned off" }.into(),
        }
    }
}

/// The installed app with one of these names (`name.desktop`).
fn find_app(apps: &[gio::AppInfo], names: &[&str]) -> Option<gio::AppInfo> {
    names.iter().find_map(|n| {
        let want = format!("{}.desktop", n.to_lowercase());
        apps.iter().find(|a| a.id().is_some_and(|id| id.to_lowercase() == want)).cloned()
    })
}

fn app_icon(item: &Item, size: i32) -> gtk::Widget {
    if let Some(icon) = item.app.as_ref().and_then(|a| a.icon()) {
        let img = gtk::Image::from_gicon(&icon);
        img.set_pixel_size(size);
        img.set_valign(gtk::Align::Center);
        img.add_css_class("nx2-sft-icon");
        return img.upcast();
    }
    let t = widgets::tile(match item.kind {
        Kind::Package(_) => "package-x-generic-symbolic",
        Kind::NyxApp(..) => "application-x-addon-symbolic",
        _ => "application-x-executable-symbolic",
    });
    if size > 40 {
        t.add_css_class("nx2-big-tile");
    }
    t.upcast()
}

/// The Software page while it's open: the two lists and the picked app's card.
struct Software {
    lists: [Vec<Item>; 2],
    rows: RefCell<Vec<(usize, usize, gtk::Box)>>,
    detail: gtk::Box,
    terminal: gtk::Label,
    which: Cell<usize>,
}

pub fn software(page: &Page) {
    let s = nyxos::settings();
    let installed: Installed = cmd::read(&["nyx-system", "catalog", "installed", "--json"])
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let apps = gio::AppInfo::all();
    let mine: Vec<Item> = installed
        .for_you
        .iter()
        .map(|m| Item {
            name: m.name.clone(),
            version: m.version.clone(),
            from: m.from.clone(),
            kind: Kind::Mine(m.id.clone()),
            app: find_app(&apps, &[&m.id, &m.name]),
        })
        .collect();
    let mut everyone: Vec<Item> = installed
        .movable
        .iter()
        .map(|m| Item {
            name: m.name.clone(),
            version: None,
            from: "For everyone".into(),
            kind: Kind::Movable(m.id.clone()),
            app: find_app(&apps, &[&m.id, &m.name]),
        })
        .collect();
    let moving = |attr: &str| installed.movable.iter().any(|m| m.packages.iter().any(|p| p == attr));
    for attr in s.packages.iter().filter(|a| !moving(a)) {
        everyone.push(Item {
            name: attr.clone(),
            version: None,
            from: "nixpkgs".into(),
            kind: Kind::Package(s.unfree.contains(attr)),
            app: find_app(&apps, &[attr]),
        });
    }
    for (app, on) in s.nyx_apps.iter().filter(|(app, _)| !installed.movable.iter().any(|m| m.nyx.contains(*app))) {
        everyone.push(Item {
            name: format!("Nyx {app}"),
            version: None,
            from: "NyxOS".into(),
            kind: Kind::NyxApp(app.clone(), *on),
            app: find_app(&apps, &[&format!("nyx-{app}")]),
        });
    }

    // The preview: the picked app's card, where apps come from, and where to go next.
    let pv = page.split();
    let column = preview_column(&pv);
    let detail = widgets::vbox(14);
    column.append(&detail);
    column.append(&sub_head("Where apps come from", None));
    let sources = widgets::hbox(10);
    sources.set_homogeneous(true);
    sources.add_css_class("nx2-sft-sources");
    let source = |icon: &str, title: &str, line: &gtk::Widget, end: Option<&gtk::Widget>| {
        let c = widgets::vbox(6);
        c.add_css_class("preview-card");
        c.add_css_class("nx2-sft-source");
        let t = widgets::tile(icon);
        t.set_halign(gtk::Align::Start);
        c.append(&t);
        c.append(&widgets::label(title, "nx2-sft-source-title"));
        c.append(line);
        if let Some(e) = end {
            let gap = widgets::vbox(0);
            gap.set_vexpand(true);
            c.append(&gap);
            e.set_halign(gtk::Align::Start);
            c.append(e);
        }
        c
    };
    let words = |t: &str| {
        let l = widgets::label(t, "nx2-dim");
        l.set_wrap(true);
        l
    };
    let store = link("Open Store");
    store.connect_clicked(|_| {
        let _ = std::process::Command::new("nyx-store").spawn();
    });
    sources.append(&source(
        "system-software-install-symbolic",
        "Nyx Store",
        words("The NyxOS catalog, nixpkgs and Flathub").upcast_ref(),
        Some(store.upcast_ref()),
    ));
    let welcome = crate::capability::program("nyx-welcome").then(|| {
        let b = link("Open Welcome");
        b.connect_clicked(|_| {
            let _ = std::process::Command::new("nyx-welcome").spawn();
        });
        b
    });
    sources.append(&source(
        "starred-symbolic",
        "Nyx Welcome",
        words("The apps you picked when you first signed in").upcast_ref(),
        welcome.as_ref().map(|b| b.upcast_ref()),
    ));
    let terminal = widgets::label("nyx install", "nx2-mono");
    terminal.set_wrap(true);
    terminal.set_wrap_mode(pango::WrapMode::WordChar);
    let lines = widgets::vbox(4);
    lines.append(&terminal);
    lines.append(&words("Same place, same updates"));
    sources.append(&source("utilities-terminal-symbolic", "In a terminal", lines.upcast_ref(), None));
    column.append(&sources);
    let links = card();
    let (r, _) = widgets::nav_row(
        Some("view-app-grid-symbolic"),
        "Default apps",
        "Which app opens each kind of file",
        "Default Apps",
        || window::navigate("apps"),
    );
    links.append(&r);
    let (r, _) = widgets::nav_row(
        Some("software-update-available-symbolic"),
        "Updates",
        "What's new for the system, your apps and tools, and firmware",
        "Updates",
        || window::navigate("updates"),
    );
    widgets::keywords("update upgrade new versions");
    links.append(&r);
    column.append(&links);
    let foot = pv.foot("dialog-information-symbolic", "Other accounts on this computer install their own.");
    foot_end(&foot, &super::updates::undo_button("software"));

    let sw = Rc::new(Software {
        lists: [mine, everyone],
        rows: RefCell::default(),
        detail,
        terminal,
        which: Cell::new(0),
    });

    // The controls: yours or everyone's, a search, the list, and where to get more.
    let g = page.plain_group("");
    let lists: [gtk::Box; 2] = [card(), card()];
    let ls = lists.clone();
    let w = Rc::downgrade(&sw);
    let seg = widgets::segmented(
        &[
            ("mine".to_string(), format!("For you · {}", sw.lists[0].len())),
            ("all".to_string(), format!("For everyone · {}", sw.lists[1].len())),
        ],
        "mine",
        move |v| {
            let which = usize::from(v == "all");
            ls[0].set_visible(which == 0);
            ls[1].set_visible(which == 1);
            if let Some(sw) = w.upgrade() {
                sw.which.set(which);
                show_item(&sw, which, (!sw.lists[which].is_empty()).then_some(0));
            }
        },
    );
    seg.add_css_class("nx2-wide-seg");
    let mut b = seg.first_child();
    while let Some(c) = b {
        c.set_hexpand(true);
        b = c.next_sibling();
    }
    g.add(&seg);
    let find = gtk::SearchEntry::new();
    find.set_placeholder_text(Some("Find an installed app"));
    find.add_css_class("nx2-find");
    g.add(&find);
    for (which, list) in lists.iter().enumerate() {
        if sw.lists[which].is_empty() {
            let empty = widgets::label(
                if which == 0 {
                    "Nothing yet. Nyx Store, Nyx Welcome and nyx install add software here."
                } else {
                    "Nothing beyond what NyxOS includes. Software that needs the system (services, drivers) is added here."
                },
                "nx2-empty",
            );
            empty.set_wrap(true);
            list.append(&empty);
        }
        for (i, item) in sw.lists[which].iter().enumerate() {
            let r = widgets::row(&item.name, &item.line(), None);
            if let Kind::Package(true) = item.kind {
                widgets::tag_row(&r, "Unfree license");
            }
            r.prepend(&app_icon(item, 32));
            r.add_css_class("with-tile");
            r.add_css_class("nav-row");
            r.add_css_class("nx2-pick-row");
            r.set_cursor_from_name(Some("pointer"));
            let click = gtk::GestureClick::new();
            let w = Rc::downgrade(&sw);
            click.connect_released(move |_, _, _, _| {
                if let Some(sw) = w.upgrade() {
                    show_item(&sw, which, Some(i));
                }
            });
            r.add_controller(click);
            list.append(&r);
            sw.rows.borrow_mut().push((which, i, r));
        }
        widgets::keywords(if which == 0 {
            "installed my apps profile remove uninstall"
        } else {
            "move my account per user everyone system wide reset nixpkgs"
        });
        list.set_visible(which == 0);
        g.add(list);
    }
    {
        let sw = sw.clone();
        find.connect_search_changed(move |e| {
            let q = e.text().to_lowercase();
            for (which, i, r) in sw.rows.borrow().iter() {
                r.set_visible(q.is_empty() || sw.lists[*which][*i].name.to_lowercase().contains(&q));
            }
        });
    }
    let g = page.group("");
    gap(&g);
    let open = gtk::Button::with_label("Open Store");
    open.connect_clicked(|_| {
        let _ = std::process::Command::new("nyx-store").spawn();
    });
    g.add(&widgets::icon_row(
        "system-software-install-symbolic",
        "Get more apps",
        "Nyx Store has the NyxOS catalog, nixpkgs and Flathub",
        Some(open.upcast_ref()),
    ));
    widgets::keywords("nyx store install apps catalog flathub nixpkgs welcome");

    show_item(&sw, 0, (!sw.lists[0].is_empty()).then_some(0));
}

/// The picked app's card, or what the list is for when it's empty.
fn show_item(sw: &Rc<Software>, which: usize, i: Option<usize>) {
    clear(&sw.detail);
    for (w, n, r) in sw.rows.borrow().iter() {
        if Some((*w, *n)) == i.map(|i| (which, i)) {
            r.add_css_class("nx2-picked");
        } else {
            r.remove_css_class("nx2-picked");
        }
    }
    let hero = widgets::hbox(16);
    hero.add_css_class("preview-card");
    hero.add_css_class("nx2-sft-hero");
    let text = widgets::vbox(4);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let buttons = widgets::vbox(8);
    buttons.set_valign(gtk::Align::Center);
    let Some(item) = i.and_then(|i| sw.lists[which].get(i)) else {
        let t = widgets::tile("system-software-install-symbolic");
        t.add_css_class("nx2-big-tile");
        hero.append(&t);
        let (title, line) = if which == 0 {
            (
                "Your apps live here",
                "Apps from Nyx Store, Nyx Welcome and nyx install. Installing and removing your own needs no password, and other accounts have their own.",
            )
        } else {
            (
                "Nothing installed for everyone",
                "Beyond what NyxOS includes. What the system has to provide (services, drivers) is installed for everyone.",
            )
        };
        text.append(&widgets::label(title, "nx2-sft-name"));
        let l = widgets::label(line, "nx2-dim");
        l.set_wrap(true);
        text.append(&l);
        hero.append(&text);
        let store = gtk::Button::with_label("Open Store");
        store.add_css_class("suggested-action");
        store.connect_clicked(|_| {
            let _ = std::process::Command::new("nyx-store").spawn();
        });
        buttons.append(&store);
        hero.append(&buttons);
        sw.detail.append(&hero);
        sw.terminal.set_text("nyx install <app>");
        return;
    };

    hero.append(&app_icon(item, 64));
    let name = widgets::label(&item.name, "nx2-sft-name");
    name.set_wrap(true);
    text.append(&name);
    let about = item.app.as_ref().and_then(|a| a.description()).map(|d| d.to_string()).unwrap_or_else(|| item.line());
    let l = widgets::label(&about, "nx2-dim");
    l.set_wrap(true);
    text.append(&l);
    let chips = widgets::hbox(6);
    chips.add_css_class("nx2-chips");
    let yours = matches!(item.kind, Kind::Mine(_));
    chips.append(&pill(if yours { "Yours" } else { "For everyone" }, if yours { "accent" } else { "" }));
    chips.append(&pill(&item.from, ""));
    if let Some(v) = &item.version {
        chips.append(&pill(v, ""));
    }
    if let Kind::Package(true) = item.kind {
        chips.append(&pill("Unfree license", "warn"));
    }
    text.append(&chips);
    hero.append(&text);

    if let Some(app) = item.app.clone() {
        let open = gtk::Button::with_label("Open");
        open.add_css_class("suggested-action");
        open.connect_clicked(move |_| {
            if let Err(e) = app.launch(&[], None::<&gio::AppLaunchContext>) {
                window::toast(&format!("It couldn't be opened: {e}"));
            }
        });
        buttons.append(&open);
    }
    let action = match &item.kind {
        Kind::Mine(id) => {
            let (id, name) = (id.clone(), item.name.clone());
            widgets::confirm_button("Remove", &format!("Remove {name}?"), move |b| {
                b.set_sensitive(false);
                let what = name.clone();
                cmd::run_async(&["nyx-system", "catalog", "remove", "--or-profile", "--yes", &id], move |r| {
                    window::toast(&match r {
                        Ok(_) => format!("Removed {what}"),
                        Err(e) => format!("{what} couldn't be removed: {e:#}"),
                    });
                    window::rebuild("software");
                });
            })
        }
        Kind::Movable(id) => {
            let (id, what) = (id.clone(), item.name.clone());
            let mv = widgets::confirm_button("Move to My Account", &format!("Move {what}? Others lose it."), move |b| {
                b.set_sensitive(false);
                window::toast(&format!("Moving {what} to your account…"));
                let what = what.clone();
                cmd::run_async(&["nyx-system", "catalog", "move", "--yes", &id], move |r| {
                    window::toast(&match r {
                        Ok(_) => format!("{what} is yours now"),
                        Err(e) => format!("{what} couldn't be moved: {e:#}"),
                    });
                    window::rebuild("software");
                });
            });
            mv.set_sensitive(!nyxos::busy());
            mv
        }
        Kind::Package(_) => {
            let a = item.name.clone();
            widgets::confirm_button("Remove", &format!("Remove {a} for everyone?"), move |b| {
                b.set_sensitive(false);
                nyxos::apply(vec![Change::RemovePackages { packages: vec![a.clone()] }], "Removed", |_| window::rebuild("software"));
            })
        }
        Kind::NyxApp(app, _) => {
            let a = app.clone();
            widgets::confirm_button("Reset", &format!("Give nyx-{app} back to NyxOS's default?"), move |b| {
                b.set_sensitive(false);
                nyxos::apply(vec![Change::NyxApp { name: a.clone(), enabled: None }], "Done", |_| window::rebuild("software"));
            })
        }
    };
    action.add_css_class("nx2-quiet-danger");
    buttons.append(&action);
    hero.append(&buttons);
    sw.detail.append(&hero);

    let rebuild = "A rebuild, with an administrator's password";
    let pairs: Vec<(&str, String)> = match &item.kind {
        Kind::Mine(_) => vec![
            ("Installed for", "You only".into()),
            ("Comes from", item.from.clone()),
            ("Updates", "With Applications".into()),
            ("Lives in", "Your Nix profile".into()),
            ("Version", item.version.clone().unwrap_or_else(|| "Not known".into())),
            ("Removing it", "Needs no password".into()),
        ],
        Kind::Movable(_) => vec![
            ("Installed for", "Everyone on this computer".into()),
            ("Updates", "With the system".into()),
            ("Moving it to you", rebuild.into()),
            ("After the move", "Others install their own".into()),
        ],
        Kind::Package(unfree) => vec![
            ("Installed for", "Everyone on this computer".into()),
            ("Comes from", "nixpkgs".into()),
            ("Updates", "With the system".into()),
            ("License", if *unfree { "Unfree, accepted for this computer".into() } else { "Free".into() }),
            ("Removing it", rebuild.into()),
        ],
        Kind::NyxApp(_, on) => vec![
            ("Installed for", "Everyone on this computer".into()),
            ("Comes from", "NyxOS".into()),
            ("Here", if *on { "Turned on".into() } else { "Turned off".into() }),
            ("Resetting it", "Back to NyxOS's default, in a rebuild".into()),
        ],
    };
    sw.detail.append(&facts(&pairs, 3));
    sw.terminal.set_text(&match &item.kind {
        Kind::Mine(id) | Kind::Movable(id) => format!("nyx install {id}"),
        _ => "nyx install <app>".into(),
    });
}

// ---------- Boot & Sign-in ----------

/// A theme as the boot and sign-in screens draw it.
#[derive(Clone)]
struct Look {
    id: String,
    name: String,
    bg: Rgb,
    surface: Rgb,
    text: Rgb,
    dim: Rgb,
    accent: Rgb,
    second: Rgb,
    accent_hex: String,
}

fn look_of(id: &str, name: &str, colours: [&str; 6]) -> Look {
    let c = |i: usize| crate::fangraph::rgb(colours[i]);
    Look {
        id: id.into(),
        name: name.into(),
        bg: c(0),
        surface: c(1),
        text: c(2),
        dim: c(3),
        accent: c(4),
        second: c(5),
        accent_hex: colours[4].into(),
    }
}

/// The screens before anyone signs in, as the page shows them.
struct Boot {
    looks: Vec<Look>,
    /// The system theme's look, for "Same as the system theme".
    system: Look,
    boot: RefCell<String>,
    login: RefCell<String>,
    logo: RefCell<brand::Logo>,
    pictures: RefCell<HashMap<String, ImageSurface>>,
    me: String,
    others: Vec<String>,
    keyboard: String,
    generations: Vec<(u32, String, bool)>,
}

impl Boot {
    fn look(&self, id: &str) -> &Look {
        self.looks.iter().find(|l| l.id == id).unwrap_or(&self.system)
    }

    /// The logo `px` across in `colour`, drawn once.
    fn logo(&self, colour: &str, px: i32) -> Option<ImageSurface> {
        let key = format!("{colour}/{px}");
        if let Some(s) = self.pictures.borrow().get(&key) {
            return Some(s.clone());
        }
        let surface = svg_surface(&self.logo.borrow().svg(colour, px as u32), px)?;
        self.pictures.borrow_mut().insert(key, surface.clone());
        Some(surface)
    }
}

/// An SVG drawn into a Cairo surface `px` across.
fn svg_surface(svg: &str, px: i32) -> Option<ImageSurface> {
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(svg.as_bytes().to_vec()));
    let pb = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(&stream, px, px, true, gio::Cancellable::NONE).ok()?;
    let (w, h) = (pb.width(), pb.height());
    let mut surface = ImageSurface::create(cairo::Format::ARgb32, w, h).ok()?;
    let stride = surface.stride() as usize;
    let (n, rowstride, alpha) = (pb.n_channels() as usize, pb.rowstride() as usize, pb.has_alpha());
    let src = pb.read_pixel_bytes();
    {
        let mut d = surface.data().ok()?;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let i = y * rowstride + x * n;
                if i + n > src.len() {
                    continue;
                }
                let a = if alpha { src[i + 3] as u32 } else { 255 };
                let o = y * stride + x * 4;
                d[o] = (src[i + 2] as u32 * a / 255) as u8;
                d[o + 1] = (src[i + 1] as u32 * a / 255) as u8;
                d[o + 2] = (src[i] as u32 * a / 255) as u8;
                d[o + 3] = a as u8;
            }
        }
    }
    Some(surface)
}

/// Paint `s` `size` across with its middle at (`cx`, `cy`).
fn paint_centred(cr: &Context, s: &ImageSurface, cx: f64, cy: f64, size: f64) {
    let scale = size / s.width().max(s.height()).max(1) as f64;
    cr.save().ok();
    cr.translate(cx - s.width() as f64 * scale / 2.0, cy - s.height() as f64 * scale / 2.0);
    cr.scale(scale, scale);
    let _ = cr.set_source_surface(s, 0.0, 0.0);
    let _ = cr.paint();
    cr.restore().ok();
}

/// The largest 16:10 screen that fits in `w`×`h`, in the middle.
fn screen_rect(w: f64, h: f64) -> R {
    let sw = w.min(h * 1.6);
    let sh = sw / 1.6;
    R::new((w - sw) / 2.0, (h - sh) / 2.0, sw, sh)
}

fn backdrop(cr: &Context, l: &Look, r: R) {
    sketch::rounded(cr, r.x, r.y, r.w, r.h, 10.0);
    let g = cairo::LinearGradient::new(r.x, r.y, r.x + r.w, r.y + r.h);
    let a = sketch::mix(l.bg, l.accent, 0.16);
    let b = sketch::mix(l.bg, l.second, 0.08);
    g.add_color_stop_rgb(0.0, a.0, a.1, a.2);
    g.add_color_stop_rgb(0.55, l.bg.0, l.bg.1, l.bg.2);
    g.add_color_stop_rgb(1.0, b.0, b.1, b.2);
    let _ = cr.set_source(&g);
    let _ = cr.fill_preserve();
    sketch::set(cr, l.text, 0.12);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
}

fn draw_boot_screen(cr: &Context, b: &Boot, l: &Look, r: R) {
    backdrop(cr, l, r);
    let size = (r.h * 0.2).round();
    let glow = cairo::RadialGradient::new(r.cx(), r.cy() - r.h * 0.05, 0.0, r.cx(), r.cy() - r.h * 0.05, r.h * 0.4);
    glow.add_color_stop_rgba(0.0, l.accent.0, l.accent.1, l.accent.2, 0.14);
    glow.add_color_stop_rgba(1.0, l.accent.0, l.accent.1, l.accent.2, 0.0);
    sketch::rounded(cr, r.x, r.y, r.w, r.h, 10.0);
    let _ = cr.set_source(&glow);
    let _ = cr.fill();
    if let Some(s) = b.logo(&l.accent_hex, (size * 2.0) as i32) {
        paint_centred(cr, &s, r.cx(), r.cy() - r.h * 0.06, size);
    }
    let (cx, cy, rad) = (r.cx(), r.cy() + r.h * 0.16, (r.h * 0.03).max(6.0));
    cr.set_line_width(2.0);
    sketch::set(cr, l.dim, 0.3);
    cr.arc(cx, cy, rad, 0.0, TAU);
    let _ = cr.stroke();
    sketch::set(cr, l.accent, 1.0);
    cr.arc(cx, cy, rad, -PI / 2.0, PI * 0.3);
    let _ = cr.stroke();
}

fn draw_sign_in(cr: &Context, b: &Boot, l: &Look, r: R) {
    backdrop(cr, l, r);
    let u = r.h / 100.0;
    if let Some(s) = b.logo(&l.accent_hex, (u * 14.0) as i32) {
        paint_centred(cr, &s, r.x + u * 6.0, r.y + u * 6.0, u * 6.0);
    }
    let now = glib::DateTime::now_local().ok();
    let clock = now.as_ref().and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default();
    let day = now.as_ref().and_then(|d| d.format("%A %e %B").ok()).map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
    text(cr, &clock, r.cx(), r.y + u * 30.0, u * 17.0, false, l.text, 0.95, 0.5);
    text(cr, &day, r.cx(), r.y + u * 37.0, u * 4.2, false, l.text, 0.75, 0.5);
    let (ay, ar) = (r.y + u * 52.0, u * 7.5);
    cr.arc(r.cx(), ay, ar + 2.0, 0.0, TAU);
    sketch::set(cr, l.text, 0.85);
    let _ = cr.fill();
    draw_avatar(cr, &b.me, r.cx(), ay, ar);
    text(cr, &b.me, r.cx(), ay + ar + u * 6.0, u * 4.2, true, l.text, 0.95, 0.5);
    let (fw, fh) = (r.w * 0.3, u * 7.0);
    let (fx, fy) = (r.cx() - fw / 2.0, ay + ar + u * 9.5);
    sketch::rounded(cr, fx, fy, fw, fh, fh / 2.0);
    sketch::set(cr, sketch::mix(l.surface, l.text, 0.1), 0.75);
    let _ = cr.fill_preserve();
    sketch::set(cr, l.text, 0.18);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    for i in 0..8 {
        cr.arc(fx + fh * 0.7 + i as f64 * u * 2.4, fy + fh / 2.0, u * 0.7, 0.0, TAU);
        sketch::set(cr, l.text, 0.85);
        let _ = cr.fill();
    }
    let (gx, gy) = (fx + fw - fh / 2.0, fy + fh / 2.0);
    cr.arc(gx, gy, fh * 0.36, 0.0, TAU);
    sketch::set(cr, l.accent, 1.0);
    let _ = cr.fill();
    sketch::set(cr, l.bg, 0.9);
    cr.set_line_width(1.6);
    cr.move_to(gx - fh * 0.06, gy - fh * 0.14);
    cr.line_to(gx + fh * 0.08, gy);
    cr.line_to(gx - fh * 0.06, gy + fh * 0.14);
    let _ = cr.stroke();
    // At the foot: the other people on this computer, the keyboard layout and power.
    let by = r.y + r.h - u * 7.0;
    let mut x = r.x + u * 6.0;
    for name in b.others.iter().take(3) {
        draw_avatar(cr, name, x, by, u * 2.6);
        x += u * 4.0;
    }
    if !b.others.is_empty() {
        text(cr, "Other people", x + u * 0.5, by + u * 1.3, u * 3.2, false, l.text, 0.75, 0.0);
    }
    let px = r.x + r.w - u * 6.0;
    cr.arc(px, by, u * 1.9, -PI * 0.3, PI * 1.3);
    sketch::set(cr, l.text, 0.75);
    cr.set_line_width(1.4);
    let _ = cr.stroke();
    cr.move_to(px, by - u * 2.6);
    cr.line_to(px, by - u * 0.6);
    let _ = cr.stroke();
    if !b.keyboard.is_empty() {
        text(cr, &b.keyboard, px - u * 5.0, by + u * 1.3, u * 3.2, false, l.text, 0.75, 1.0);
    }
}

fn draw_boot_menu(cr: &Context, b: &Boot, p: &sketch::Pal, r: R) {
    let bg = (0.06, 0.065, 0.08);
    let fg = (0.86, 0.87, 0.9);
    sketch::rounded(cr, r.x, r.y, r.w, r.h, 10.0);
    sketch::set(cr, bg, 1.0);
    let _ = cr.fill_preserve();
    sketch::set(cr, fg, 0.12);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    let u = r.h / 100.0;
    let (x0, x1) = (r.x + r.w * 0.12, r.x + r.w * 0.88);
    text(cr, "Choose a generation to start", x0, r.y + u * 14.0, u * 4.2, true, fg, 0.9, 0.0);
    if b.generations.is_empty() {
        text(cr, "No generations were found on this computer.", r.cx(), r.cy(), u * 4.0, false, fg, 0.6, 0.5);
        return;
    }
    let shown = b.generations.len().min(7);
    let rh = u * 8.0;
    for (i, (n, date, running)) in b.generations.iter().take(shown).enumerate() {
        let y = r.y + u * 22.0 + i as f64 * rh;
        if i == 0 {
            sketch::rounded(cr, x0 - u * 2.0, y, x1 - x0 + u * 4.0, rh - u, 4.0);
            sketch::set(cr, p.accent, 0.28);
            let _ = cr.fill();
        }
        let name = format!("NyxOS · generation {n}{}", if *running { " (running)" } else { "" });
        cr.select_font_face("monospace", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
        cr.set_font_size(u * 3.6);
        sketch::set(cr, fg, if i == 0 { 1.0 } else { 0.75 });
        cr.move_to(x0, y + rh * 0.62);
        let _ = cr.show_text(&name);
        if let Ok(e) = cr.text_extents(date) {
            sketch::set(cr, fg, 0.5);
            cr.move_to(x1 - e.x_advance(), y + rh * 0.62);
            let _ = cr.show_text(date);
        }
    }
    let more = b.generations.len() - shown;
    let y = r.y + u * 22.0 + shown as f64 * rh + rh * 0.62;
    if more > 0 {
        text(cr, &format!("{more} more"), x0, y, u * 3.6, false, fg, 0.45, 0.0);
    }
    text(cr, "↑ ↓ to choose · Enter to start", r.cx(), r.y + r.h - u * 8.0, u * 3.4, false, fg, 0.5, 0.5);
}

/// A boot screen choice: the theme's background with its two colours.
fn draw_swatch(cr: &Context, l: &Look, w: f64, h: f64) {
    let r = R::new(0.5, 0.5, w - 1.0, h - 1.0);
    sketch::rounded(cr, r.x, r.y, r.w, r.h, 6.0);
    let g = cairo::LinearGradient::new(0.0, 0.0, w, h);
    let a = sketch::mix(l.bg, l.accent, 0.22);
    g.add_color_stop_rgb(0.0, l.bg.0, l.bg.1, l.bg.2);
    g.add_color_stop_rgb(1.0, a.0, a.1, a.2);
    let _ = cr.set_source(&g);
    let _ = cr.fill_preserve();
    sketch::set(cr, l.text, 0.12);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    for (i, c) in [l.accent, l.second].iter().enumerate() {
        cr.arc(w - 26.0 + i as f64 * 11.0, h - 11.0, 4.0, 0.0, TAU);
        sketch::set(cr, *c, 1.0);
        let _ = cr.fill();
    }
}

/// What the preview shows: the boot menu, the boot screen or the sign-in screen.
const VIEWS: [(&str, &str, &str, &str); 3] = [
    ("menu", "Boot menu", "Every generation the computer can start.", "If a change goes wrong, start the generation before it. Your files stay."),
    ("boot", "Boot screen", "What shows while the computer starts.", "The logo takes the theme's colour."),
    ("signin", "Sign-in", "What everyone sees before signing in.", "Each person's own theme takes over once they've signed in."),
];

pub fn boot(page: &Page) {
    let s = nyxos::settings();
    let staged = Staged::new("boot");
    let looks: Vec<Look> = nyx_theme::all()
        .iter()
        .map(|t| {
            let p = &t.palette;
            look_of(&t.id, &t.name, [&p.bg, &p.surface, &p.text, &p.dim_text, &p.accent, &p.highlight])
        })
        .collect();
    let suite_theme = nyx_theme::read_suite().map(|s| s.theme).unwrap_or_default();
    let system = looks.iter().find(|l| l.id == suite_theme).cloned().unwrap_or_else(|| {
        let t = nyx_theme::current_theme();
        let p = &t.palette;
        look_of(&t.id, &t.name, [&p.bg, &p.surface, &p.text, &p.dim_text, &p.accent, &p.highlight])
    });
    let people = people(&s);
    let me = people.first().map(|(_, n)| n.clone()).unwrap_or_default();
    let others = people.iter().skip(1).map(|(_, n)| n.clone()).collect();
    let generations = nyxos::generations()
        .iter()
        .map(|g| {
            let date = glib::DateTime::from_unix_local(g.created)
                .and_then(|d| d.format("%e %b %Y, %H:%M"))
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            (g.number, date, g.current)
        })
        .collect::<Vec<_>>();
    let now_boot = s.theme.boot.clone().unwrap_or_default();
    let now_login = s.theme.login.clone().unwrap_or_default();
    let b = Rc::new(Boot {
        looks,
        system,
        boot: RefCell::new(now_boot.clone()),
        login: RefCell::new(now_login.clone()),
        logo: RefCell::new(brand::current()),
        pictures: RefCell::default(),
        me,
        others,
        keyboard: s.keyboard_layout.clone().unwrap_or_default(),
        generations,
    });

    // The preview: one of the three screens, as it'll look after Apply.
    let pv = page.split();
    let view = Rc::new(Cell::new(2usize));
    let column = preview_column(&pv);
    column.parent().inspect(|s| s.set_vexpand(true));
    let (head, title, line) = preview_head("Sign-in screen", VIEWS[2].2, &[]);
    column.append(&head);
    let stage = {
        let (b, view) = (b.clone(), view.clone());
        widgets::drawing(330, move |cr, p, _, w, h| {
            let r = screen_rect(w, h);
            match view.get() {
                0 => draw_boot_menu(cr, &b, p, r),
                1 => draw_boot_screen(cr, &b, b.look(&b.boot.borrow()), r),
                _ => draw_sign_in(cr, &b, b.look(&b.login.borrow()), r),
            }
        })
    };
    stage.set_vexpand(true);
    stage.add_css_class("nx2-bt-stage");
    column.append(&stage);
    let foot = pv.foot("view-reveal-symbolic", VIEWS[2].3);
    let seg = {
        let (view, stage, title, line, foot) = (view.clone(), stage.clone(), title.clone(), line.clone(), foot.clone());
        widgets::segmented(
            &VIEWS.iter().map(|(id, name, _, _)| (id.to_string(), name.to_string())).collect::<Vec<_>>(),
            "signin",
            move |v| {
                let i = VIEWS.iter().position(|(id, ..)| *id == v).unwrap_or(2);
                view.set(i);
                title.set_text(if i == 2 { "Sign-in screen" } else { VIEWS[i].1 });
                line.set_text(VIEWS[i].2);
                foot.set_text(VIEWS[i].3);
                stage.queue_draw();
            },
        )
    };
    seg.set_valign(gtk::Align::Center);
    head.append(&seg);
    let show = {
        let seg = seg.clone();
        move |i: usize| {
            if let Some(t) = children(&seg).get(i).and_then(|w| w.downcast_ref::<gtk::ToggleButton>()) {
                t.set_active(true);
            }
        }
    };

    // The boot screen: a swatch for each theme.
    let changed_boot = pill("Changed", "accent");
    changed_boot.set_visible(false);
    let g = head_group(page, "Boot screen", &[changed_boot.upcast_ref()], true);
    let grid = gtk::Grid::new();
    grid.add_css_class("nx2-bt-swatches");
    grid.set_column_homogeneous(true);
    grid.set_row_spacing(8);
    grid.set_column_spacing(8);
    // The system's, the one chosen now, then the rest; the first eight show.
    let mut choices = vec![("".to_string(), "System".to_string(), b.system.clone())];
    let mut rest: Vec<&Look> = b.looks.iter().collect();
    rest.sort_by_key(|l| l.id != now_boot);
    choices.extend(rest.into_iter().map(|l| (l.id.clone(), l.name.clone(), l.clone())));
    const SHOWN: usize = 8;
    let hidden: Rc<RefCell<Vec<gtk::ToggleButton>>> = Rc::default();
    let total = choices.len();
    let mut first: Option<gtk::ToggleButton> = None;
    for (i, (id, name, look)) in choices.into_iter().enumerate() {
        let t = gtk::ToggleButton::new();
        t.add_css_class("nx2-bt-swatch");
        t.set_tooltip_text(Some(if id.is_empty() { "Same as the system theme" } else { &name }));
        let col = widgets::vbox(4);
        let sw = gtk::DrawingArea::new();
        sw.set_content_height(38);
        sw.set_hexpand(true);
        sw.set_draw_func(move |_, cr, w, h| draw_swatch(cr, &look, w as f64, h as f64));
        col.append(&sw);
        let l = widgets::label(&name, "nx2-bt-swatch-name");
        l.set_ellipsize(pango::EllipsizeMode::End);
        col.append(&l);
        t.set_child(Some(&col));
        match &first {
            Some(f) => t.set_group(Some(f)),
            None => first = Some(t.clone()),
        }
        t.set_active(id == now_boot);
        let (b, staged, stage, pill, now, show) =
            (b.clone(), staged.clone(), stage.clone(), changed_boot.clone(), now_boot.clone(), show.clone());
        t.connect_toggled(move |t| {
            if !t.is_active() {
                return;
            }
            *b.boot.borrow_mut() = id.clone();
            let change = (id != now).then(|| {
                let name = if id.is_empty() { "same as the system".to_string() } else { name.clone() };
                (format!("Boot screen: {name}"), Change::BootTheme { value: (!id.is_empty()).then(|| id.clone()) })
            });
            staged.set("boot", change);
            pill.set_visible(staged.waiting("boot"));
            show(1);
            stage.queue_draw();
        });
        if i >= SHOWN {
            t.set_visible(false);
            hidden.borrow_mut().push(t.clone());
        }
        grid.attach(&t, (i % 4) as i32, (i / 4) as i32, 1, 1);
    }
    let picker = widgets::vbox(6);
    picker.append(&grid);
    if total > SHOWN {
        let more = link(&format!("All {total} themes"));
        more.set_halign(gtk::Align::Start);
        more.connect_clicked(move |b| {
            let open = !hidden.borrow().first().is_some_and(|t| t.is_visible());
            for t in hidden.borrow().iter() {
                t.set_visible(open);
            }
            b.set_label(&if open { "Fewer themes".to_string() } else { format!("All {total} themes") });
        });
        picker.append(&more);
    }
    let holder = widgets::stacked_row("", "", picker.upcast_ref());
    holder.add_css_class("nx2-bt-pick");
    widgets::keywords("boot screen plymouth splash theme look");
    g.add(&holder);

    // The sign-in screen's look.
    let changed_login = pill("Changed", "accent");
    changed_login.set_visible(false);
    let g = head_group(page, "Sign-in screen", &[changed_login.upcast_ref()], true);
    let mut themes = vec![("".to_string(), "Same as the system".to_string())];
    themes.extend(b.looks.iter().map(|l| (l.id.clone(), l.name.clone())));
    let dd = widgets::dropdown(&themes, &now_login);
    {
        let (b, staged, stage, pill, show) = (b.clone(), staged.clone(), stage.clone(), changed_login.clone(), show.clone());
        dd.connect_selected_notify(move |d| {
            let Some((id, name)) = themes.get(d.selected() as usize).cloned() else { return };
            *b.login.borrow_mut() = id.clone();
            let change = (id != now_login).then(|| {
                let name = if id.is_empty() { "same as the system".to_string() } else { name };
                (format!("Sign-in screen: {name}"), Change::LoginTheme { value: (!id.is_empty()).then(|| id.clone()) })
            });
            staged.set("login", change);
            pill.set_visible(staged.waiting("login"));
            show(2);
            stage.queue_draw();
        });
    }
    g.add(&widgets::icon_row("avatar-default-symbolic", "Look", "Follows the system unless set", Some(dd.upcast_ref())));
    widgets::keywords("sign-in screen greeter login theme look");

    // The logo, on both screens.
    let g = page.group("System logo");
    let stage_logo = {
        let (b, staged, stage, show) = (b.clone(), staged.clone(), stage.clone(), show.clone());
        Rc::new(move |change: Option<Change>| {
            let logo = match &change {
                Some(Change::Logo { value: Some(l) }) => match l.format {
                    nyx_system_proto::LogoFormat::Svg => Some(brand::Logo {
                        image: brand::Image::Svg(l.data.clone()),
                        source: brand::Source::System,
                        path: None,
                    }),
                    nyx_system_proto::LogoFormat::Png => Some(brand::Logo {
                        image: brand::Image::Svg(format!(
                            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1 1\"><image width=\"1\" height=\"1\" href=\"data:image/png;base64,{}\"/></svg>",
                            l.data
                        )),
                        source: brand::Source::System,
                        path: None,
                    }),
                },
                Some(Change::Logo { value: None }) => Some(brand::builtin()),
                _ => None,
            };
            *b.logo.borrow_mut() = logo.unwrap_or_else(brand::current);
            b.pictures.borrow_mut().clear();
            let words = match &change {
                Some(Change::Logo { value: None }) => "The Nyx mark as the logo",
                _ => "A new system logo",
            };
            staged.set("logo", change.map(|c| (words.to_string(), c)));
            show(1);
            stage.queue_draw();
        }) as Rc<dyn Fn(Option<Change>)>
    };
    let r = crate::sections::themes::system_logo_row(s.logo.as_ref(), stage_logo);
    if let Some(d) = description_of(r.upcast_ref()) {
        d.set_text("On both screens, and in apps");
    }
    r.set_tooltip_text(Some("An SVG drawn in currentColor takes each theme's colour; a PNG is shown as it is."));
    widgets::keywords("logo brand mark emblem picture svg png boot sign-in");
    g.add(&r);

    let g = page.group("");
    gap(&g);
    let n = b.generations.len();
    let (r, _) = widgets::nav_row(
        Some("document-open-recent-symbolic"),
        "Earlier generations",
        &if n > 0 { format!("{n} in the boot menu") } else { "The boot menu lists the system's earlier generations".into() },
        "Updates",
        || window::navigate("updates"),
    );
    widgets::keywords("recovery earlier generations boot menu rollback");
    g.add(&r);
    staged.place(page);
}

/// The Network page's firewall: on or off, and the ports opened through Settings.
pub fn firewall(page: &Page) {
    let s = nyxos::settings();
    let g = page.collapsible("Firewall", false);
    g.note("Other computers can only reach the ports you allow here, and the ones services you turn on need (like remote login). Changes are system settings, applied with a rebuild.");
    let on = cmd::output(&["systemctl", "is-active", "firewall.service"]).is_some_and(|x| x == "active");
    let (r, _) = widgets::switch_row("Block incoming connections", "NyxOS's firewall.", on, move |now| {
        if now != on {
            nyxos::apply(
                vec![Change::Firewall { enabled: Some(now) }],
                if now { "Firewall is on" } else { "Firewall is off" },
                |_| window::rebuild("network"),
            );
        }
    });
    tag_system(&r);
    widgets::keywords("firewall block ports security");
    g.add(&r);
    for (protocol, ports) in [("tcp", &s.firewall.tcp), ("udp", &s.firewall.udp)] {
        for port in ports.iter().copied() {
            let remove = widgets::confirm_button("Remove", "Close this port?", move |b| {
                b.set_sensitive(false);
                let change = Change::FirewallPort { port, protocol: protocol.into(), open: false };
                nyxos::apply(vec![change], "Port closed", |_| window::rebuild("network"));
            });
            g.add(&widgets::row(&format!("{port}/{protocol}"), "Allowed in", Some(remove.upcast_ref())));
        }
    }
    let (r, _) = widgets::entry_row(
        "Allow a port",
        "A port number, optionally with /tcp or /udp, e.g. 8080/tcp. Press Enter.",
        "",
        "8080/tcp",
        |t| {
            let t = t.trim().to_string();
            if t.is_empty() {
                return;
            }
            let Some((port, proto)) = crate::sections::network::parse_rule(&t) else {
                return window::toast("Use a port like 8080 or 8080/tcp");
            };
            let Ok(port) = port.parse::<u16>() else { return window::toast("Use a port from 1 to 65535") };
            let protocols: &[&str] = match proto.as_str() {
                "tcp" => &["tcp"],
                "udp" => &["udp"],
                _ => &["tcp", "udp"],
            };
            let changes = protocols.iter().map(|p| Change::FirewallPort { port, protocol: p.to_string(), open: true }).collect();
            nyxos::apply(changes, "Port allowed", |_| window::rebuild("network"));
        },
    );
    tag_system(&r);
    widgets::keywords("open port allow rule");
    g.add(&r);
}

/// Disks' Clean up on NyxOS: old generations and the packages only they used, and old
/// system logs, through the System Manager.
pub fn cleanup_rows(g: &widgets::Group) {
    const DAYS: u32 = 14;
    let cutoff = gtk::glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0) - i64::from(DAYS) * 86400;
    let old = nyxos::generations().iter().filter(|x| !x.current && x.created < cutoff).count();
    let desc = format!(
        "Removes generations older than two weeks ({old} now; never the one running) and the packages only they used. The boot menu keeps the rest."
    );
    let (r, b) = widgets::button_row("Old generations and unused packages", &desc, "Clean", |b| {
        b.set_sensitive(false);
        nyxos::clean(DAYS, |_| window::rebuild_if_built("disks"));
    });
    b.set_sensitive(!nyxos::busy());
    widgets::keywords("garbage collect nix store generations old space free");
    g.add(&r);
    crate::sections::cleanup::cleanup_row(
        g,
        "Old system logs",
        "Keeps the newest 200 MB of the system journal.",
        || crate::backend::cleanup::journal_extra(200 << 20),
        |b| {
            nyxos::clean_logs(200, move |ok| {
                if !ok {
                    b.set_sensitive(true);
                }
                window::rebuild_if_built("disks");
            })
        },
    );
}
