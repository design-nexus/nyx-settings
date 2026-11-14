//! Users, groups, sudo, SSH and sign-in security.
//!
//! Reading is free. Changing accounts goes through `settings-helper` (root, via
//! pkexec, see `backend::accounts`); the Omarchy security scripts are
//! interactive, so they open in a floating terminal like the updater does.
//!
//! The page is the people on the computer beside the chosen person's card: who they
//! are and what they can do, as switches for the groups behind each ability.

use crate::backend::accounts as acc;
use crate::backend::passwd::{self, Group, User};
use crate::dialog::{Field, ask};
use crate::widgets::{self, Page};
use crate::{cmd, paths, sketch, theme, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::rc::Rc;

pub fn files() -> Vec<PathBuf> {
    vec![authorized_keys()]
}

fn authorized_keys() -> PathBuf {
    paths::home().join(".ssh/authorized_keys")
}

/// Groups worth showing even though they're system groups.
const NOTABLE: &[(&str, &str)] = &[
    ("wheel", "Administrators: can use sudo."),
    (
        "docker",
        "Can run Docker without sudo. This is the same as having root: any program running as a member can take over the computer.",
    ),
    ("video", "Can use cameras and the screen directly."),
    ("audio", "Can use sound devices directly."),
    ("input", "Can read keyboards and mice directly."),
    ("storage", "Can mount and unmount drives."),
    ("kvm", "Can run virtual machines with hardware speed-up."),
    ("libvirt", "Can manage virtual machines."),
    ("uucp", "Can use serial ports."),
    ("lp", "Can use printers."),
];

/// What a group lets its members do, as a person's card shows it: (group, icon, title, line).
const ABILITIES: &[(&str, &str, &str, &str)] = &[
    ("wheel", "security-high-symbolic", "Change the system", "Install apps, use sudo"),
    ("lp", "printer-symbolic", "Use printers", "Print and scan"),
    ("storage", "drive-removable-media-symbolic", "Mount drives", "USB sticks and disks"),
    ("video", "camera-web-symbolic", "Cameras and the screen", "Calls and screen capture"),
    ("kvm", "computer-symbolic", "Virtual machines", "At full speed"),
    ("docker", "package-x-generic-symbolic", "Run Docker", "Same as administrator"),
];

pub fn terminal(command: &str) {
    crate::platform::in_terminal(command);
}

/// Run a helper request off the main thread, then say how it went and redraw.
fn apply(args: Vec<String>, stdin: Option<String>, done: &'static str) {
    cmd::background(
        move || {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            acc::helper(&refs, stdin.as_deref()).map_err(|e| e.to_string())
        },
        move |r| {
            match r {
                Ok(_) => window::toast(done),
                Err(e) => window::toast(&e),
            }
            window::rebuild("accounts");
        },
    );
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// Check a new password typed twice.
pub(crate) fn password_problem(a: &str, b: &str) -> Option<String> {
    if a.is_empty() {
        Some("Enter a password.".into())
    } else if a != b {
        Some("The two passwords don't match.".into())
    } else {
        None
    }
}

fn change_password(name: &str) {
    let name = name.to_string();
    ask(
        &format!("Change password for {name}"),
        "",
        vec![Field::secret("New password"), Field::secret("New password again")],
        "Change password",
        None,
        move |v, _| {
            if let Some(p) = password_problem(&v[0], &v[1]) {
                return Some(p);
            }
            apply(args(&["set-password", &name]), Some(format!("{}\n", v[0])), "Password changed");
            None
        },
    );
}

fn add_user_dialog(default_shell: String) {
    ask(
        "Add a person",
        "They get their own home folder.",
        vec![Field::text("User name"), Field::secret("Password"), Field::secret("Password again")],
        "Add",
        Some(("Administrator (can use sudo)", false)),
        move |v, admin| {
            let name = v[0].trim();
            if !passwd::valid_name(name) {
                return Some("Use lower-case letters, digits, - and _, starting with a letter.".into());
            }
            if passwd::users().iter().any(|u| u.name == name) {
                return Some(format!("There is already a user called {name}."));
            }
            if let Some(p) = password_problem(&v[1], &v[2]) {
                return Some(p);
            }
            SELECTED.with(|s| *s.borrow_mut() = name.to_string());
            apply(
                args(&["add-user", name, &default_shell, if admin { "1" } else { "0" }]),
                Some(format!("{}\n", v[1])),
                "User added",
            );
            None
        },
    );
}

// ----- Root helper -----

/// Show how to turn on the root helper when it's missing or old, at the top of
/// the page. Returns whether changes can be made now. Shared by every page that
/// changes the system.
pub fn require_helper(page: &Page, section: &'static str, what: &str) -> bool {
    let (ok, notice) = helper_notice(section, what);
    if let Some(n) = notice {
        page.body.append(&n);
    }
    ok
}

/// The same notice, for a page to place itself (e.g. inside the group it affects).
pub fn helper_notice(section: &'static str, what: &str) -> (bool, Option<gtk::Box>) {
    let state = acc::helper_state();
    if state == acc::HelperState::Ready {
        return (true, None);
    }
    let (button, text) = if state == acc::HelperState::Missing {
        (
            "Set up…",
            format!(
                "<b>Allow system changes.</b> {} needs a small helper that runs as root. Setting it up asks for your password in a terminal, once.",
                capitalise(what)
            ),
        )
    } else {
        (
            "Update…",
            "<b>Update the system helper.</b> This version of Settings has a newer one than the one installed.".to_string(),
        )
    };
    let b = widgets::banner(&text, false);
    let set_up = gtk::Button::with_label(button);
    set_up.set_valign(gtk::Align::Center);
    set_up.set_tooltip_text(Some("Installs the helper to /usr/local/lib/settings"));
    set_up.connect_clicked(move |_| {
        let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "settings".into());
        terminal(&format!("sudo {} --install-helper", cmd::shell_quote(&exe)));
        gtk::glib::timeout_add_seconds_local_once(20, move || window::rebuild(section));
    });
    b.append(&set_up);
    // An outdated helper still works for what it already did.
    (state == acc::HelperState::Outdated, Some(b))
}

/// Whether disks, mounts and cleanup ask for a password. Off: once, then
/// remembered for a few minutes. On: never, for you at this computer.
fn password_free_row(can_edit: bool) -> gtk::Box {
    let syncing = Rc::new(std::cell::Cell::new(false));
    let sw = gtk::Switch::new();
    sw.set_active(crate::prefs::get().password_free);
    sw.set_sensitive(can_edit);
    let s2 = syncing.clone();
    let sw2 = sw.clone();
    sw.connect_active_notify(move |s| {
        if s2.get() {
            return;
        }
        let on = s.is_active();
        s.set_sensitive(false);
        let (sw, syncing) = (sw2.clone(), s2.clone());
        cmd::background(
            move || acc::helper(&["password-free", if on { "on" } else { "off" }], None).map_err(|e| e.to_string()),
            move |r| {
                sw.set_sensitive(true);
                match r {
                    Ok(_) => {
                        crate::prefs::update(|p| p.password_free = on);
                        window::toast(if on {
                            "Disks, mounts and cleanup won't ask for your password."
                        } else {
                            "They'll ask for your password again."
                        });
                    }
                    Err(e) => {
                        // Put the switch back as it was.
                        syncing.set(true);
                        sw.set_active(!on);
                        syncing.set(false);
                        window::toast(&e);
                    }
                }
            },
        );
    });
    let r = widgets::icon_row(
        "drive-harddisk-symbolic",
        "Skip the password for disks",
        "Users and the firewall still ask",
        Some(sw.upcast_ref()),
    );
    r.set_tooltip_text(Some(
        "Changing disks, mounts and the package cache asks for your password once, then remembers it for a few minutes. \
         With this on it doesn't ask while you're signed in at this computer, so any program you run could format a drive \
         or change mounts without asking. Leave it off if you're unsure.",
    ));
    widgets::keywords("password polkit sudo ask prompt authentication disks mounts cleanup passwords for system changes");
    r
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Run a system change through the helper off the main thread, toast the result and redraw.
pub fn run_admin(args: &[&str], stdin: Option<&str>, done: &str, section: &'static str) {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let stdin = stdin.map(str::to_string);
    let done = done.to_string();
    cmd::background(
        move || {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            acc::helper(&refs, stdin.as_deref()).map_err(|e| e.to_string())
        },
        move |r| {
            match r {
                Ok(_) => window::toast(&done),
                Err(e) => window::toast(&e),
            }
            window::rebuild(section);
        },
    );
}

/// Run several helper requests one after another (stopping at the first failure),
/// then say how it went and redraw.
pub fn run_admin_steps(steps: Vec<Vec<String>>, done: &str, section: &'static str) {
    if steps.is_empty() {
        return;
    }
    let done = done.to_string();
    cmd::background(
        move || {
            for step in steps {
                let refs: Vec<&str> = step.iter().map(String::as_str).collect();
                acc::helper(&refs, None).map_err(|e| e.to_string())?;
            }
            Ok::<(), String>(())
        },
        move |r| {
            match r {
                Ok(()) => window::toast(&done),
                Err(e) => window::toast(&e),
            }
            window::rebuild(section);
        },
    );
}

/// Change a user's full name, shell and (for other users) login name.
fn edit_user(u: &User, is_me: bool) {
    let shells = passwd::shells();
    let shell_options: Vec<(String, String)> =
        shells.iter().map(|s| (s.clone(), s.rsplit('/').next().unwrap_or(s).to_string())).collect();
    let mut fields = vec![Field::text_with("Full name", &u.full_name)];
    if !is_me {
        fields.push(Field::text_with("User name", &u.name));
    }
    let has_shell = shells.contains(&u.shell);
    if has_shell {
        fields.push(Field::choice(shell_options, &u.shell));
    }
    let (old, user) = (u.name.clone(), u.clone());
    ask(
        &format!("Edit {}", u.name),
        if is_me {
            "You can't rename the account you're signed in with. The shell is what new terminals start with."
        } else {
            "Renaming only works while they're signed out."
        },
        fields,
        "Save",
        if is_me { None } else { Some(("Also rename the home folder", true)) },
        move |v, move_home| {
            let mut i = 0;
            let full = v[i].trim().to_string();
            i += 1;
            let name = if is_me {
                old.clone()
            } else {
                i += 1;
                v[i - 1].trim().to_string()
            };
            let shell = if has_shell { v[i].clone() } else { user.shell.clone() };
            if !is_me && !passwd::valid_name(&name) {
                return Some("Use lower-case letters, digits, - and _, starting with a letter.".into());
            }
            if full.contains([':', ',']) {
                return Some("The full name can't contain : or ,".into());
            }
            let mut steps = Vec::new();
            if full != user.full_name {
                steps.push(args(&["set-fullname", &old, &full]));
            }
            if shell != user.shell {
                steps.push(args(&["set-shell", &old, &shell]));
            }
            if name != old {
                steps.push(args(&["rename-user", &old, &name, if move_home { "1" } else { "0" }]));
                SELECTED.with(|s| *s.borrow_mut() = name.clone());
            }
            if steps.is_empty() {
                return None;
            }
            run_admin_steps(steps, "User updated", "accounts");
            None
        },
    );
}

fn rename_group(old: &str) {
    let old = old.to_string();
    ask(&format!("Rename {old}"), "", vec![Field::text_with("Group name", &old)], "Save", None, {
        let old = old.clone();
        move |v, _| {
            let new = v[0].trim().to_string();
            if new == old {
                return None;
            }
            if !passwd::valid_name(&new) {
                return Some("Use lower-case letters, digits, - and _, starting with a letter.".into());
            }
            apply(args(&["rename-group", &old, &new]), None, "Group renamed");
            None
        }
    });
}

// ----- Parts the account and system pages share -----

/// The swatches a picture or a provider can take, without the greys.
fn bright_swatches() -> Vec<sketch::Rgb> {
    let all: Vec<sketch::Rgb> = theme::swatches()
        .into_iter()
        .filter(|(n, _)| !matches!(n.as_str(), "graphite" | "grey" | "gray" | "slate" | "silver"))
        .map(|(_, hex)| crate::fangraph::rgb(&hex))
        .collect();
    if all.is_empty() { vec![sketch::pal().accent] } else { all }
}

/// A swatch picked by `key`, the same one every time.
pub(crate) fn swatch_for(key: &str) -> sketch::Rgb {
    let s = bright_swatches();
    let n = key.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    s[n as usize % s.len()]
}

fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| c.is_whitespace() || matches!(c, '-' | '_' | '.')).filter(|w| !w.is_empty()).collect();
    let first = |w: &str| w.chars().next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    match words.as_slice() {
        [] => "?".into(),
        [one] => first(one),
        [a, .., b] => first(a) + &first(b),
    }
}

/// A round picture with someone's initials, in a swatch picked by their name.
pub(crate) fn avatar(name: &str, size: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    area.set_halign(gtk::Align::Center);
    area.set_valign(gtk::Align::Center);
    area.add_css_class("acc-avatar");
    let (key, text) = (name.to_string(), initials(name));
    area.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let r = w.min(h) / 2.0;
        cr.arc(w / 2.0, h / 2.0, r, 0.0, std::f64::consts::TAU);
        sketch::set(cr, swatch_for(&key), 1.0);
        let _ = cr.fill();
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
        cr.set_font_size(r * if text.chars().count() > 1 { 0.72 } else { 0.86 });
        if let Ok(ext) = cr.text_extents(&text) {
            sketch::set(cr, (1.0, 1.0, 1.0), 0.97);
            cr.move_to(w / 2.0 - ext.width() / 2.0 - ext.x_bearing(), h / 2.0 - ext.height() / 2.0 - ext.y_bearing());
            let _ = cr.show_text(&text);
        }
    });
    let weak = area.downgrade();
    theme::subscribe(&area, move || {
        if let Some(a) = weak.upgrade() {
            a.queue_draw();
        }
    });
    area
}

/// A small coloured pill: `kind` is "ok", "warn", "accent", "danger" or "muted".
pub(crate) fn pill(text: &str, kind: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("acc-pill");
    l.add_css_class(kind);
    l.set_valign(gtk::Align::Center);
    l.set_halign(gtk::Align::Start);
    l
}

/// Put a [`pill`] after a row's title.
pub(crate) fn pill_row(row: &gtk::Box, text: &str, kind: &str) {
    let mut c = row.first_child();
    while let Some(w) = c.as_ref().filter(|w| !w.has_css_class("row-text")) {
        c = w.next_sibling();
    }
    let Some(texts) = c.and_downcast::<gtk::Box>() else { return };
    let Some(title) = texts.first_child().and_downcast::<gtk::Label>() else { return };
    let line = widgets::hbox(8);
    texts.insert_child_after(&line, Some(&title));
    texts.remove(&title);
    title.set_wrap(false);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    line.append(&title);
    line.append(&pill(text, kind));
}

/// Small facts side by side: a caption over each value. They wrap onto more lines when
/// narrow. Returns the value labels too, in order, for facts filled in later.
pub(crate) fn facts(items: &[(&str, String)]) -> (gtk::FlowBox, Vec<gtk::Label>) {
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_homogeneous(false);
    flow.set_column_spacing(22);
    flow.set_row_spacing(10);
    flow.set_max_children_per_line(items.len().max(1) as u32);
    flow.set_min_children_per_line(2.min(items.len().max(1)) as u32);
    flow.add_css_class("acc-facts");
    let mut values = Vec::new();
    for (k, v) in items {
        let b = widgets::vbox(2);
        b.append(&widgets::label(k, "acc-fact-key"));
        let value = widgets::label(v, "acc-fact-value");
        value.set_selectable(true);
        value.set_can_focus(false);
        value.set_wrap(true);
        value.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        b.append(&value);
        values.push(value);
        flow.append(&b);
    }
    let mut c = flow.first_child();
    while let Some(w) = c {
        w.set_focusable(false);
        c = w.next_sibling();
    }
    (flow, values)
}

/// A heading inside a preview: a bold title and a dim line beside it.
pub(crate) fn subhead(title: &str, line: &str) -> gtk::Box {
    let b = widgets::hbox(8);
    b.add_css_class("acc-subhead");
    b.append(&widgets::label(title, "acc-subhead-title"));
    if !line.is_empty() {
        let l = widgets::label(line, "acc-subhead-line");
        l.set_wrap(true);
        l.set_hexpand(true);
        b.append(&l);
    }
    b
}

/// Where a preview's content goes: a column as wide as the preview, under the
/// headings it makes itself (see [`head`]).
pub(crate) fn roomy(pv: &widgets::Preview) -> gtk::Box {
    let inner = widgets::vbox(14);
    pv.add(&inner);
    inner
}

/// A preview's heading made in place: a title, a line, and widgets at the right.
pub(crate) fn head(title: &str, line: &str, end: &[gtk::Widget]) -> gtk::Box {
    let head = widgets::hbox(10);
    head.add_css_class("preview-head");
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(title, "preview-title");
    t.set_wrap(true);
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

/// Something at the right of a group's title (a count, a note, a small button).
pub(crate) fn title_end(group: &widgets::Group, end: &impl IsA<gtk::Widget>) {
    let Some(head) = group.wrapper.first_child().and_downcast::<gtk::Box>() else { return };
    let Some(title) = head.first_child().and_downcast::<gtk::Label>().filter(|l| l.has_css_class("group-title")) else {
        return;
    };
    let line = widgets::hbox(8);
    line.add_css_class("acc-title-line");
    head.remove(&title);
    title.set_hexpand(true);
    line.append(&title);
    let end = end.as_ref();
    end.set_valign(gtk::Align::Center);
    end.add_css_class("acc-title-end");
    line.append(end);
    head.prepend(&line);
}

/// Make a row a button: a click or Enter calls `f`.
pub(crate) fn clickable(row: &impl IsA<gtk::Widget>, f: impl Fn() + 'static) {
    let row = row.as_ref();
    row.add_css_class("acc-clickable");
    row.set_cursor_from_name(Some("pointer"));
    row.set_focusable(true);
    let f = Rc::new(f);
    let click = gtk::GestureClick::new();
    let f2 = f.clone();
    click.connect_released(move |_, _, _, _| f2());
    row.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::space) {
            f();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    row.add_controller(keys);
}

/// A row that adds something: a plus and its title in the accent colour.
pub(crate) fn add_row(title: &str, f: impl Fn() + 'static) -> gtk::Box {
    let r = widgets::row(title, "", None);
    let plus = gtk::Image::from_icon_name("list-add-symbolic");
    plus.add_css_class("acc-add-icon");
    r.prepend(&plus);
    r.add_css_class("acc-add-row");
    r.add_css_class("with-tile");
    clickable(&r, f);
    r
}

/// Remove everything in `b` (and forget its rows for search).
pub(crate) fn clear(b: &gtk::Box) {
    widgets::forget_rows(b);
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// How a line between two things looks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Link {
    /// In use: solid, in the section's colour.
    On,
    /// Not in use: faint and dashed.
    Off,
    /// The one being looked at: brighter and thicker.
    Hot,
}

pub(crate) type Links = Rc<RefCell<Vec<(gtk::Widget, gtk::Widget, Link)>>>;

/// `content` with curves drawn from each link's first widget (its right edge) to its
/// second (its left edge). Call `queue_draw` on the returned area after changing links.
pub(crate) fn linked(content: &impl IsA<gtk::Widget>, links: Links) -> (gtk::Overlay, gtk::DrawingArea) {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(content));
    let own: Rc<RefCell<glib::WeakRef<gtk::DrawingArea>>> = Rc::default();
    let own2 = own.clone();
    let area = widgets::drawing(0, move |cr, p, section, _, _| {
        let Some(area) = own2.borrow().upgrade() else { return };
        for (from, to, kind) in links.borrow().iter() {
            let (Some(a), Some(b)) = (from.compute_bounds(&area), to.compute_bounds(&area)) else { continue };
            if !from.is_drawable() || !to.is_drawable() {
                continue;
            }
            let (x0, y0) = (a.x() as f64 + a.width() as f64, a.y() as f64 + a.height() as f64 / 2.0);
            let (x1, y1) = (b.x() as f64, b.y() as f64 + b.height() as f64 / 2.0);
            let mid = (x0 + x1) / 2.0;
            cr.move_to(x0, y0);
            cr.curve_to(mid, y0, mid, y1, x1, y1);
            match kind {
                Link::On => {
                    sketch::set(cr, section, 0.75);
                    cr.set_line_width(1.6);
                    cr.set_dash(&[], 0.0);
                }
                Link::Hot => {
                    sketch::set(cr, section, 1.0);
                    cr.set_line_width(2.6);
                    cr.set_dash(&[], 0.0);
                }
                Link::Off => {
                    sketch::set(cr, p.dim, 0.45);
                    cr.set_line_width(1.2);
                    cr.set_dash(&[3.0, 4.0], 0.0);
                }
            }
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
        }
    });
    *own.borrow_mut() = area.downgrade();
    area.set_can_target(false);
    area.set_vexpand(true);
    overlay.add_overlay(&area);
    overlay.set_measure_overlay(&area, false);
    (overlay, area)
}

// ----- Who signed in when -----

/// The people signed in right now (`loginctl`).
fn signed_in_now() -> Vec<String> {
    cmd::output(&["loginctl", "list-users", "--no-legend"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1).map(str::to_string))
        .collect()
}

/// When each person last signed in, from `lastlog2`, as "Today at 21:02".
fn last_sign_ins() -> HashMap<String, String> {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let text = if cmd::present("lastlog2") { cmd::output(&["lastlog2"]).unwrap_or_default() } else { String::new() };
    let now = glib::DateTime::now_local().ok();
    text.lines()
        .skip(1)
        .filter(|l| !l.contains("Never logged in"))
        .filter_map(|l| {
            let t: Vec<&str> = l.split_whitespace().collect();
            let n = t.len();
            if n < 7 {
                return None;
            }
            let (mon, day, time, tz, year) = (t[n - 5], t[n - 4], t[n - 3], t[n - 2], t[n - 1]);
            let month = MONTHS.iter().position(|m| *m == mon)? + 1;
            if tz.len() != 5 {
                return None;
            }
            let iso = format!("{year}-{month:02}-{day:0>2}T{time}{}:{}", &tz[..3], &tz[3..]);
            let when = glib::DateTime::from_iso8601(&iso, None).ok()?.to_local().ok()?;
            Some((t[0].to_string(), friendly_when(&when, now.as_ref()?)))
        })
        .collect()
}

/// "Today at 21:02", "Yesterday at 9:12" or "3 Nov 2026".
pub(crate) fn friendly_when(when: &glib::DateTime, now: &glib::DateTime) -> String {
    let day = |d: &glib::DateTime| (d.year(), d.day_of_year());
    let yesterday = now.add_days(-1).ok();
    let time = when.format("%H:%M").map(|s| s.to_string()).unwrap_or_default();
    if day(when) == day(now) {
        format!("Today at {time}")
    } else if yesterday.as_ref().is_some_and(|y| day(y) == day(when)) {
        format!("Yesterday at {time}")
    } else {
        when.format("%-d %b %Y").map(|s| s.to_string()).unwrap_or_default()
    }
}

// ----- Page -----

thread_local! {
    /// The person whose card is showing, kept across redraws.
    static SELECTED: RefCell<String> = const { RefCell::new(String::new()) };
}

/// What a person's card needs to know about everyone.
struct People {
    me: String,
    users: Vec<User>,
    groups: Vec<Group>,
    can_edit: bool,
    signed_in: Vec<String>,
    last: HashMap<String, String>,
    locked: HashMap<String, Option<bool>>,
}

impl People {
    fn display_name(u: &User) -> String {
        if u.full_name.trim().is_empty() { u.name.clone() } else { u.full_name.clone() }
    }

    fn role(&self, u: &User) -> String {
        let admin = acc::is_admin(u, &self.groups);
        let mut role = if self.locked.get(&u.name).copied().flatten() == Some(true) {
            "Locked · can't sign in".to_string()
        } else if admin {
            "Administrator".to_string()
        } else {
            "Standard".to_string()
        };
        if u.name == self.me {
            role.push_str(" · you");
        }
        role
    }

    fn when(&self, u: &User) -> String {
        if self.signed_in.contains(&u.name) {
            "Signed in now".into()
        } else {
            self.last.get(&u.name).cloned().unwrap_or_default()
        }
    }

    fn default_shell(&self) -> String {
        let shells = passwd::shells();
        self.users
            .iter()
            .find(|u| u.name == self.me)
            .map(|u| u.shell.clone())
            .filter(|s| shells.contains(s))
            .unwrap_or_else(|| "/bin/bash".into())
    }
}

pub fn build(page: &Page) {
    let can_edit = require_helper(page, "accounts", "changing users and groups");
    let me = acc::me();
    let users = acc::people();
    let groups = passwd::groups();
    let locked = users.iter().map(|u| (u.name.clone(), acc::is_locked(&u.name))).collect();
    let people = Rc::new(People { me: me.clone(), users, groups, can_edit, signed_in: signed_in_now(), last: last_sign_ins(), locked });

    let mut chosen = SELECTED.with(|s| s.borrow().clone());
    if !people.users.iter().any(|u| u.name == chosen) {
        chosen = me.clone();
    }

    // The picture: the chosen person's card.
    let pv = page.split();
    let card = roomy(&pv);
    card.add_css_class("acc-person-preview");
    let foot = pv.foot("dialog-information-symbolic", "Deleting someone keeps their home folder, so no files are lost.");
    let foot_actions = widgets::hbox(8);
    foot_actions.set_valign(gtk::Align::Center);
    if let Some(line) = foot.parent().and_downcast::<gtk::Box>() {
        line.add_css_class("acc-foot");
        line.append(&foot_actions);
    }

    // The people, with a way to add one.
    let host = cmd::output(&["hostnamectl", "hostname"]).filter(|h| !h.is_empty()).unwrap_or_else(|| "this computer".into());
    let g = page.group(&format!("People on {host}"));
    title_end(&g, &widgets::label(&people.users.len().to_string(), "acc-count"));
    let rows: Rc<RefCell<Vec<(String, gtk::Box)>>> = Rc::default();
    let show: Rc<dyn Fn(&str)> = {
        let (people, card, foot_actions, rows) = (people.clone(), card.clone(), foot_actions.clone(), rows.clone());
        Rc::new(move |name: &str| {
            SELECTED.with(|s| *s.borrow_mut() = name.to_string());
            for (n, r) in rows.borrow().iter() {
                if n == name {
                    r.add_css_class("selected");
                } else {
                    r.remove_css_class("selected");
                }
            }
            if let Some(u) = people.users.iter().find(|u| u.name == name) {
                person_card(&card, &foot_actions, &people, u);
            }
        })
    };
    for u in &people.users {
        let r = person_row(&people, u);
        let (name, show) = (u.name.clone(), show.clone());
        clickable(&r, move || show(&name));
        rows.borrow_mut().push((u.name.clone(), r.clone()));
        g.add(&r);
    }
    if can_edit {
        let shell = people.default_shell();
        g.add(&add_row("Add a person", move || add_user_dialog(shell.clone())));
        widgets::keywords("new account create add user");
    }
    show(&chosen);

    // Settings for everyone, and the rarer tools a level down.
    let g = page.group("For everyone");
    g.add(&password_free_row(can_edit));

    let shown = shown_groups(&people.users, &people.groups);
    let (r, _) = widgets::nav_row(Some("system-users-symbolic"), "Groups", "", &shown.len().to_string(), {
        let people = people.clone();
        move || {
            let people = people.clone();
            widgets::subpage("Groups", 600, move |p| groups_section(p, &people.users, &people.groups, people.can_edit));
        }
    });
    widgets::keywords("group groups member wheel docker video audio input storage kvm libvirt uucp lp new rename");
    g.add(&r);

    let active = acc::sshd_active();
    let (r, _) = widgets::nav_row(
        Some("utilities-terminal-symbolic"),
        "Remote sign-in",
        "Let other computers sign in with SSH",
        if active { "On" } else { "Off" },
        || {
            widgets::subpage("Remote sign-in", 600, ssh_section);
        },
    );
    widgets::keywords("ssh sshd openssh server remote firewall authorized keys public github agent passphrase");
    g.add(&r);

    if cmd::present("omarchy-sudo-passwordless") {
        let (r, _) = widgets::nav_row(
            Some("alarm-symbolic"),
            "Pause the sudo password",
            "For a few minutes, then it asks again",
            "",
            || terminal("omarchy-sudo-passwordless"),
        );
        widgets::keywords("sudo password nopasswd root admin administrator access passwordless");
        g.add(&r);
    }

    security_rows(&g);
}

/// A person in the list: their picture, name and role, and when they last signed in.
fn person_row(people: &People, u: &User) -> gtk::Box {
    let r = widgets::row(&People::display_name(u), &people.role(u), None);
    r.add_css_class("acc-person");
    r.prepend(&avatar(&People::display_name(u), 34));
    let when = widgets::label(&people.when(u), "row-value");
    when.set_valign(gtk::Align::Center);
    r.append(&when);
    if u.name == people.me {
        pill_row(&r, "You", "ok");
        widgets::keywords("passwd login signed in groups you me shell bash zsh fish chsh");
    }
    widgets::keywords(&format!("{} user account login admin sudo wheel", u.name));
    r
}

/// The chosen person's card, what they can do, and how they sign in.
fn person_card(card: &gtk::Box, foot_actions: &gtk::Box, people: &People, u: &User) {
    clear(card);
    clear(foot_actions);
    let is_me = u.name == people.me;
    let name = People::display_name(u);
    let admin = acc::is_admin(u, &people.groups);
    let locked = people.locked.get(&u.name).copied().flatten();

    // Who they are.
    let top = widgets::hbox(18);
    top.add_css_class("preview-card");
    top.add_css_class("acc-card");
    top.append(&avatar(&name, 72));
    let text = widgets::vbox(10);
    text.set_hexpand(true);
    let line = widgets::hbox(8);
    let title = widgets::label(&name, "acc-card-name");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    line.append(&title);
    if locked == Some(true) {
        line.append(&pill("Locked", "muted"));
    } else if admin {
        line.append(&pill("Administrator", "ok"));
    } else {
        line.append(&pill("Standard", "accent"));
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    line.append(&spacer);
    if people.can_edit {
        let edit = widgets::labeled_button("document-edit-symbolic", "Edit");
        edit.add_css_class("acc-small");
        edit.set_tooltip_text(Some(if is_me { "Your full name and shell" } else { "Name, user name and shell" }));
        let user = u.clone();
        edit.connect_clicked(move |_| edit_user(&user, is_me));
        line.append(&edit);
        let pw = widgets::labeled_button("dialog-password-symbolic", if is_me { "Change password" } else { "Set a password" });
        pw.add_css_class("acc-small");
        pw.set_tooltip_text(Some(if is_me { "The password you sign in and use sudo with" } else { "Set a new password for them" }));
        let n = u.name.clone();
        pw.connect_clicked(move |_| change_password(&n));
        line.append(&pw);
    }
    text.append(&line);
    let mut items = vec![
        ("User name", u.name.clone()),
        ("Home folder", u.home.clone()),
        ("Shell", u.shell.rsplit('/').next().unwrap_or(&u.shell).to_string()),
    ];
    let when = people.when(u);
    if !when.is_empty() {
        items.push((if when == "Signed in now" { "Signed in" } else { "Last signed in" }, if when == "Signed in now" { "Now".into() } else { when }));
    }
    text.append(&facts(&items).0);
    top.append(&text);
    card.append(&top);

    // What they can do, as the groups behind each ability.
    let first = name.split_whitespace().next().unwrap_or(&name).to_string();
    let abilities: Vec<_> = ABILITIES.iter().filter_map(|a| people.groups.iter().find(|g| g.name == a.0).map(|g| (a, g))).collect();
    if !abilities.is_empty() {
        card.append(&subhead(
            &if is_me { "What you can do".to_string() } else { format!("What {first} can do") },
            &if is_me { "Each switch adds you to the group behind it.".to_string() } else { format!("Each switch adds {first} to the group behind it.") },
        ));
        let grid = gtk::Grid::new();
        grid.set_column_spacing(10);
        grid.set_row_spacing(10);
        grid.set_column_homogeneous(true);
        grid.add_css_class("acc-abilities");
        for (i, ((group, icon, title, line), gr)) in abilities.into_iter().enumerate() {
            let tile = ability_tile(people, u, gr, icon, title, line, *group == "docker");
            grid.attach(&tile, (i % 2) as i32, (i / 2) as i32, 1, 1);
        }
        card.append(&grid);
    }

    // Signing in.
    card.append(&subhead("Signing in", ""));
    let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    box_.add_css_class("preview-card");
    box_.add_css_class("acc-signin");
    let chips = widgets::hbox(6);
    chips.append(&chip("dialog-password-symbolic", "Password"));
    let mut items: Vec<(&str, gtk::Widget)> = vec![("Signs in with", chips.clone().upcast())];
    if is_me && super::sign_in::available() {
        let chips = chips.clone();
        super::sign_in::methods(move |methods| {
            for m in methods {
                let icon = match m {
                    "Face" => "camera-web-symbolic",
                    "Fingerprint" => "auth-fingerprint-symbolic",
                    _ => "media-flash-symbolic",
                };
                chips.append(&chip(icon, m));
            }
        });
    }
    let member: Vec<String> = acc::groups_of(u, &people.groups).into_iter().filter(|g| *g != u.name).collect();
    if !member.is_empty() {
        let shown = if member.len() > 4 { format!("{} and {} more", member[..3].join(", "), member.len() - 3) } else { member.join(", ") };
        let l = widgets::label(&shown, "acc-fact-value");
        l.set_tooltip_text(Some(&member.join(", ")));
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        items.push(("Member of", l.upcast()));
    }
    match locked {
        Some(false) => items.push(("Account", status_line("emblem-ok-symbolic", "Can sign in", "ok"))),
        Some(true) => items.push(("Account", status_line("system-lock-screen-symbolic", "Locked", "muted"))),
        None => {}
    }
    for (i, (k, v)) in items.into_iter().enumerate() {
        let col = widgets::vbox(6);
        col.set_hexpand(true);
        if i > 0 {
            col.add_css_class("acc-signin-col");
        }
        col.append(&widgets::label(k, "acc-fact-key"));
        v.set_halign(gtk::Align::Start);
        col.append(&v);
        box_.append(&col);
    }
    card.append(&box_);

    // Lock and delete, at the foot, for other people.
    if people.can_edit && !is_me {
        // Whether another user is locked can only be read as root, so offer
        // both actions when it isn't known.
        let actions: &[bool] = match locked {
            Some(true) => &[false],
            Some(false) => &[true],
            None => &[true, false],
        };
        for &lock_it in actions {
            let n = u.name.clone();
            let b = widgets::labeled_button(
                if lock_it { "system-lock-screen-symbolic" } else { "changes-allow-symbolic" },
                if lock_it { "Lock" } else { "Unlock" },
            );
            b.set_tooltip_text(Some(if lock_it { "Stop them signing in; their files stay" } else { "Let them sign in again" }));
            b.add_css_class("acc-small");
            b.connect_clicked(move |b| {
                b.set_sensitive(false);
                apply(
                    args(&[if lock_it { "lock-user" } else { "unlock-user" }, &n]),
                    None,
                    if lock_it { "Account locked" } else { "Account unlocked" },
                );
            });
            foot_actions.append(&b);
        }
        let n = u.name.clone();
        let del = widgets::confirm_button("Delete", "Delete them?", move |b| {
            b.set_sensitive(false);
            SELECTED.with(|s| s.borrow_mut().clear());
            apply(args(&["delete-user", &n, "1"]), None, "User deleted");
        });
        del.add_css_class("acc-danger-text");
        del.add_css_class("acc-small");
        foot_actions.append(&del);
    }
}

/// An icon and a word on a soft pill ("Password", "Fingerprint").
fn chip(icon: &str, text: &str) -> gtk::Box {
    let b = widgets::hbox(5);
    b.add_css_class("acc-chip");
    b.append(&gtk::Image::from_icon_name(icon));
    b.append(&gtk::Label::new(Some(text)));
    b
}

fn status_line(icon: &str, text: &str, kind: &str) -> gtk::Widget {
    let b = widgets::hbox(6);
    b.add_css_class("acc-status");
    b.add_css_class(kind);
    b.append(&gtk::Image::from_icon_name(icon));
    b.append(&gtk::Label::new(Some(text)));
    b.upcast()
}

/// One ability: its tile, what it means, and a switch that adds the person to its group.
fn ability_tile(people: &People, u: &User, gr: &Group, icon: &str, title: &str, line: &str, warn: bool) -> gtk::Box {
    let b = widgets::hbox(12);
    b.add_css_class("acc-ability");
    b.append(&widgets::tile(icon));
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(title, "acc-ability-title");
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&t);
    let l = widgets::label(line, if warn { "acc-ability-warn" } else { "acc-ability-line" });
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&l);
    b.append(&text);
    let fixed = u.gid == gr.gid;
    let on = gr.members.contains(&u.name) || fixed;
    if on {
        b.add_css_class("on");
    }
    let sw = gtk::Switch::new();
    sw.set_active(on);
    sw.set_valign(gtk::Align::Center);
    let is_me = u.name == people.me;
    let locked_out = fixed || (gr.name == "wheel" && is_me);
    sw.set_sensitive(people.can_edit && !locked_out);
    b.set_tooltip_text(Some(&if gr.name == "wheel" && is_me {
        format!("{title}: {line}. You can't take this away from the account you're signed in with.")
    } else if warn {
        "Run Docker: Docker access is the same as root. Only add people you'd give the administrator password to.".to_string()
    } else {
        format!("{title}: {line}. The {} group.", gr.name)
    }));
    let (name, group) = (u.name.clone(), gr.name.clone());
    sw.connect_active_notify(move |s| {
        let add = s.is_active();
        s.set_sensitive(false);
        let done = if group == "wheel" {
            if add { "Now an administrator" } else { "No longer an administrator" }
        } else {
            "Group updated"
        };
        apply(args(&[if add { "add-member" } else { "remove-member" }, &name, &group]), None, done);
    });
    b.append(&sw);
    b
}

/// The groups the Groups page lists: the notable system ones, then ones someone made.
fn shown_groups<'a>(users: &[User], groups: &'a [Group]) -> Vec<(&'a Group, String, bool)> {
    let user_names: Vec<&str> = users.iter().map(|u| u.name.as_str()).collect();
    let mut shown: Vec<(&Group, String, bool)> = Vec::new();
    for (name, desc) in NOTABLE {
        if let Some(gr) = groups.iter().find(|g| g.name == *name) {
            shown.push((gr, desc.to_string(), false));
        }
    }
    // Groups someone made: not a system group and not a user's own group.
    for gr in groups
        .iter()
        .filter(|g| !g.is_system() && !user_names.contains(&g.name.as_str()) && !NOTABLE.iter().any(|(n, _)| *n == g.name))
    {
        shown.push((gr, "A group you made.".into(), true));
    }
    shown
}

fn groups_section(page: &Page, users: &[User], groups: &[Group], can_edit: bool) {
    let g = page.group("Groups");
    g.note("A group gives its members access to something. Switch someone on to add them.");
    let user_names: Vec<String> = users.iter().map(|u| u.name.clone()).collect();
    for (gr, desc, custom) in shown_groups(users, groups) {
        let members =
            if gr.members.is_empty() { "No members.".to_string() } else { format!("Members: {}.", gr.members.join(", ")) };
        let (wrapper, content) = widgets::disclosure(&gr.name, &format!("{desc} {members}"));
        widgets::keywords(&format!("group {}", gr.name));
        if gr.name == "docker" {
            content.append(&widgets::banner(
                "Docker access is the same as root. Only add users you'd give the administrator password to.",
                true,
            ));
        }
        if can_edit {
            let states: Vec<bool> = users.iter().map(|u| gr.members.contains(&u.name) || (u.gid == gr.gid)).collect();
            let fixed: Vec<bool> = users.iter().map(|u| u.gid == gr.gid).collect();
            let labels: Vec<&str> = user_names.iter().map(String::as_str).collect();
            let before = Rc::new(RefCell::new(states.clone()));
            let (names, group) = (user_names.clone(), gr.name.clone());
            let chips = widgets::chip_toggles(&labels, &states, move |now| {
                let prev = before.replace(now.clone());
                let mut jobs: Vec<Vec<String>> = Vec::new();
                for i in 0..now.len() {
                    if now[i] != prev[i] && !fixed[i] {
                        jobs.push(args(&[if now[i] { "add-member" } else { "remove-member" }, &names[i], &group]));
                    }
                }
                if jobs.is_empty() {
                    return;
                }
                cmd::background(
                    move || {
                        for j in jobs {
                            let refs: Vec<&str> = j.iter().map(String::as_str).collect();
                            acc::helper(&refs, None).map_err(|e| e.to_string())?;
                        }
                        Ok::<(), String>(())
                    },
                    |r| {
                        match r {
                            Ok(()) => window::toast("Group updated"),
                            Err(e) => window::toast(&e),
                        }
                        window::rebuild("accounts");
                    },
                );
            });
            content.append(&chips);
            if custom {
                let name = gr.name.clone();
                let rename = gtk::Button::with_label("Rename");
                rename.set_halign(gtk::Align::Start);
                let n = name.clone();
                rename.connect_clicked(move |_| rename_group(&n));
                content.append(&rename);
                content.append(&widgets::confirm_button("Delete group", "Delete group?", move |_| {
                    apply(args(&["delete-group", &name]), None, "Group deleted")
                }));
            }
        }
        g.add(&wrapper);
    }
    if can_edit {
        let (r, _) = widgets::entry_row(
            "New group",
            "Make a group, then add people to it above. Press Enter to create.",
            "",
            "group name",
            |name| {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return;
                }
                if !passwd::valid_name(&name) {
                    window::toast("Use lower-case letters, digits, - and _, starting with a letter");
                    return;
                }
                apply(args(&["add-group", &name]), None, "Group created");
            },
        );
        widgets::keywords("create add group");
        g.add(&r);
    }
}

fn ssh_section(page: &Page) {
    let g = page.group("Remote sign-in");
    let active = acc::sshd_active();
    if cmd::present("omarchy-setup-security-sshd") {
        let (label, command) =
            if active { ("Turn off…", "omarchy-remove-security-sshd") } else { ("Turn on…", "omarchy-setup-security-sshd") };
        let (r, _) = widgets::button_row(
            "Remote sign-in",
            if active {
                "Other computers can sign in with SSH. Turning it off also closes the firewall port."
            } else {
                "Let other computers sign in with SSH. Turning it on opens the firewall port and asks which key to allow."
            },
            label,
            move |_| {
                terminal(command);
                gtk::glib::timeout_add_seconds_local_once(30, || window::rebuild_if_built("accounts"));
            },
        );
        widgets::keywords("sshd openssh server remote ssh firewall ufw");
        g.add(&r);
    } else {
        let (r, _) = widgets::info_row("Remote sign-in", if active { "On" } else { "Off" });
        g.add(&r);
    }

    let g = page.group("Keys allowed to sign in");
    let path = authorized_keys();
    let keys = acc_keys(&path);
    for (kind, comment, line) in &keys {
        let title = if comment.is_empty() { kind.clone() } else { comment.clone() };
        let remove = {
            let (path, line) = (path.clone(), line.clone());
            widgets::confirm_button("Remove", "Remove key?", move |_| {
                match remove_key(&path, &line) {
                    Ok(()) => window::toast("Key removed"),
                    Err(e) => window::toast(&format!("Couldn't remove the key: {e}")),
                }
                window::rebuild("accounts");
            })
        };
        let edit = gtk::Button::from_icon_name("document-edit-symbolic");
        edit.add_css_class("flat");
        edit.set_tooltip_text(Some("Edit"));
        {
            let (path, line) = (path.clone(), line.clone());
            edit.connect_clicked(move |_| {
                let (path, old) = (path.clone(), line.clone());
                ask(
                    "Edit key",
                    "The whole public key line, including its comment.",
                    vec![Field::text_with("ssh-ed25519 AAAA…", &old)],
                    "Save",
                    None,
                    move |v, _| {
                        let new = v[0].trim().to_string();
                        if acc::parse_keys(&new).len() != 1 || new.contains('\n') {
                            return Some("That doesn't look like one public key.".into());
                        }
                        match replace_key(&path, &old, &new) {
                            Ok(()) => window::toast("Key updated"),
                            Err(e) => return Some(format!("Couldn't save the key: {e}")),
                        }
                        window::rebuild("accounts");
                        None
                    },
                );
            });
        }
        let controls = widgets::hbox(6);
        controls.append(&edit);
        controls.append(&remove);
        let r = widgets::row(
            &title,
            &gtk::glib::markup_escape_text(&format!("Allowed to sign in · {kind}")),
            Some(controls.upcast_ref()),
        );
        widgets::keywords("authorized key public ssh");
        g.add(&r);
    }
    if keys.is_empty() {
        g.note("No keys are allowed to sign in yet.");
    }
    let (r, _) = widgets::entry_row(
        "Allow a key",
        "Paste a public key (starts with ssh-ed25519 or ssh-rsa) and press Enter.",
        "",
        "ssh-ed25519 AAAA…",
        |text| {
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            if acc::parse_keys(&text).len() != 1 || text.contains('\n') {
                window::toast("That doesn't look like one public key");
                return;
            }
            match add_key(&authorized_keys(), &text) {
                Ok(()) => window::toast("Key added"),
                Err(e) => window::toast(&format!("Couldn't add the key: {e}")),
            }
            window::rebuild("accounts");
        },
    );
    widgets::keywords("authorized key public ssh paste");
    g.add(&r);
    if cmd::present("omarchy-setup-security-sshd") {
        let (r, _) = widgets::entry_row(
            "Allow a GitHub user's keys",
            "Fetch the public keys of a GitHub account, then press Enter.",
            "",
            "github username",
            |user| {
                let user = user.trim().to_string();
                if user.is_empty() {
                    return;
                }
                if !user.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') || user.starts_with('-') {
                    window::toast("That isn't a GitHub user name");
                    return;
                }
                terminal(&format!("omarchy-setup-security-sshd --gh-keys {user}"));
                gtk::glib::timeout_add_seconds_local_once(30, || window::rebuild_if_built("accounts"));
            },
        );
        widgets::keywords("github gh keys import");
        g.add(&r);
    }
    if cmd::present("omarchy-setup-security-ssh-agent") {
        let (r, _) = widgets::button_row(
            "SSH agent",
            "Ask for key passphrases in a dialog and remember them in the keyring.",
            "Turn on…",
            |_| terminal("omarchy-setup-security-ssh-agent"),
        );
        widgets::keywords("agent gcr passphrase keyring");
        g.add(&r);
    }
}

fn acc_keys(path: &std::path::Path) -> Vec<(String, String, String)> {
    acc::parse_keys(&std::fs::read_to_string(path).unwrap_or_default())
}

fn add_key(path: &std::path::Path, line: &str) -> anyhow::Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == line) {
        return Ok(());
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
    write_keys(path, &text)
}

fn replace_key(path: &std::path::Path, old: &str, new: &str) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = text.lines().map(|l| if l.trim() == old { new } else { l }).collect();
    write_keys(path, &format!("{}\n", lines.join("\n")))
}

fn remove_key(path: &std::path::Path, line: &str) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let kept: Vec<&str> = text.lines().filter(|l| l.trim() != line).collect();
    write_keys(path, &format!("{}\n", kept.join("\n")))
}

/// authorized_keys must be private or sshd ignores it.
fn write_keys(path: &std::path::Path, text: &str) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    cmd::atomic_write(path, text)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

/// Omarchy's sign-in security scripts: (script that shows it's there, title, line, set up, remove, search words).
type Security = (&'static str, &'static str, &'static str, &'static str, &'static str, &'static str);
const SECURITY: &[Security] = &[
    (
        "omarchy-setup-security-fido2",
        "Security key (FIDO2)",
        "Use a hardware key to unlock sudo and sign in.",
        "omarchy-setup-security-fido2",
        "omarchy-remove-security-fido2",
        "fido2 yubikey security key u2f",
    ),
    (
        "omarchy-setup-security-fingerprint",
        "Fingerprint",
        "Use a fingerprint to unlock sudo and the lock screen.",
        "omarchy-setup-security-fingerprint",
        "omarchy-remove-security-fingerprint",
        "fingerprint fprintd biometric",
    ),
    (
        "omarchy-setup-security-sudoless-docker",
        "Docker without sudo",
        "Add your user to the docker group. This is the same as giving it root. Takes effect after a restart.",
        "omarchy-setup-security-sudoless-docker",
        "omarchy-remove-security-sudoless-docker",
        "docker group sudoless containers",
    ),
];

/// Sign-in security: Nyx Biometrics' page when it's there, and Omarchy's scripts a level down.
fn security_rows(g: &widgets::Group) {
    // With Nyx Biometrics, face, fingerprint and keys live on their own page; Omarchy's
    // scripts would wire the same PAM files a second way.
    let biometrics = super::sign_in::available();
    let items: Vec<&'static Security> = SECURITY
        .iter()
        .filter(|i| cmd::present(i.0))
        .filter(|i| !biometrics || i.0 == "omarchy-setup-security-sudoless-docker")
        .collect();
    let drive = cmd::present("omarchy-drive-password");
    if biometrics {
        let (r, _) = widgets::nav_row(
            Some("security-high-symbolic"),
            "Face, fingerprint and security keys",
            "On Sign-in & Security",
            "",
            || window::navigate("sign-in"),
        );
        widgets::keywords("face fingerprint security key fido2 yubikey biometric");
        g.add(&r);
    }
    if items.is_empty() && !drive {
        return;
    }
    let mut names: Vec<&str> = items.iter().map(|i| i.1).collect();
    if drive {
        names.push("Disk encryption password");
    }
    let desc = names.join(", ");
    let words: Vec<&str> = items.iter().map(|i| i.5).collect();
    let (r, _) = widgets::nav_row(Some("dialog-password-symbolic"), "Sign-in security", &desc, "", move || {
        let items = items.clone();
        widgets::subpage("Sign-in security", 600, move |page| security_page(page, &items, drive));
    });
    widgets::keywords(&format!("{} luks encryption drive disk unlock boot passphrase", words.join(" ")));
    g.add(&r);
}

fn security_page(page: &Page, items: &[&'static Security], drive: bool) {
    let g = page.group("Sign-in security");
    for (_, title, desc, setup, remove, kw) in items.iter().copied() {
        let controls = widgets::hbox(8);
        let (setup, remove) = (*setup, *remove);
        let on = gtk::Button::with_label("Set up…");
        on.connect_clicked(move |_| terminal(setup));
        controls.append(&on);
        if cmd::present(remove) {
            let off = gtk::Button::with_label("Remove…");
            off.connect_clicked(move |_| terminal(remove));
            controls.append(&off);
        }
        let r = widgets::row(title, desc, Some(controls.upcast_ref()));
        widgets::keywords(kw);
        g.add(&r);
    }
    if drive {
        let (r, _) = widgets::button_row(
            "Disk encryption password",
            "Change the password that unlocks the encrypted drive when the computer starts.",
            "Change…",
            |_| terminal("omarchy-drive-password"),
        );
        widgets::keywords("luks encryption drive disk unlock boot passphrase");
        g.add(&r);
    }
}
