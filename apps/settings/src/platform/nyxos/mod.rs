//! NyxOS: Settings on the NixOS-based NyxOS, where the Nyx System Manager makes system
//! changes. They're declarative: a change is built into a new system generation and
//! switched to only when it builds, so pages collect what changed and apply it together.
//! Settings asks the manager through `nyx-system`, never runs privileged commands itself.

pub mod accounts;
pub mod layout;
pub mod lock;
pub mod mounts;
pub mod pages;
pub mod pending;
pub mod region;
pub mod power;
pub mod printers;
pub mod updates;
pub mod users;

use crate::sections::home::Hero;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use nyx_system_proto::{Change, SystemSettings};
use std::cell::Cell;
use std::path::Path;

const PROFILES: &str = "/nix/var/nix/profiles";

/// The System Manager's settings (empty when it can't be reached).
pub fn settings() -> SystemSettings {
    cmd::output(&["nyx-system", "settings"]).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

thread_local! {
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

pub fn busy() -> bool {
    BUSY.with(Cell::get)
}

fn set_busy(on: bool) {
    BUSY.with(|b| b.set(on));
}

/// Run `nyx-system ARGS`: a toast while it builds, another when it's done, then `then`
/// with whether it worked.
fn run(args: Vec<String>, working: &str, done: &'static str, then: impl FnOnce(bool) + 'static) {
    if busy() {
        window::toast("Another system change is still running.");
        return;
    }
    window::toast(working);
    run_then(args, move |r| {
        match &r {
            Ok(()) => window::toast(done),
            Err(e) => window::toast(e),
        }
        then(r.is_ok());
    });
}

/// Run `nyx-system ARGS` without toasts, then `then` with what went wrong, if anything
/// (the last line nyx-system printed).
fn run_then(args: Vec<String>, then: impl FnOnce(Result<(), String>) + 'static) {
    set_busy(true);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    cmd::run_async(&refs, move |r| {
        set_busy(false);
        then(r.map(|_| ()).map_err(|e| {
            let text = format!("{e:#}");
            let last = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(&text);
            last.trim().trim_start_matches("nyx-system: ").to_string()
        }));
    });
}

/// Apply system changes in one rebuild, saying nothing: the caller shows how it went.
pub fn apply_quiet(changes: Vec<Change>, then: impl FnOnce(Result<(), String>) + 'static) {
    if changes.is_empty() {
        return;
    }
    if busy() {
        then(Err("Another system change is still running.".into()));
        return;
    }
    let json = serde_json::to_string(&changes).unwrap_or_default();
    run_then(vec!["nyx-system".into(), "apply".into(), json], then);
}

/// Apply system changes in one rebuild.
pub fn apply(changes: Vec<Change>, done: &'static str, then: impl FnOnce(bool) + 'static) {
    if changes.is_empty() {
        return;
    }
    let json = serde_json::to_string(&changes).unwrap_or_default();
    run(
        vec!["nyx-system".into(), "apply".into(), json],
        "Building the new system. It's switched to when the build works.",
        done,
        then,
    );
}

pub fn rollback(then: impl FnOnce(bool) + 'static) {
    run(
        vec!["nyx-system".into(), "rollback".into()],
        "Going back to the previous generation…",
        "Back to the previous generation",
        then,
    );
}

/// Remove generations older than `days` and what nothing needs any more.
pub fn clean(days: u32, then: impl FnOnce(bool) + 'static) {
    run(
        vec!["nyx-system".into(), "clean".into(), days.to_string()],
        "Removing old generations and unused packages…",
        "Cleaned up",
        then,
    );
}

/// Shrink the system log to `megabytes`.
pub fn clean_logs(megabytes: u32, then: impl FnOnce(bool) + 'static) {
    run(vec!["nyx-system".into(), "clean-logs".into(), megabytes.to_string()], "Removing old logs…", "Old logs removed", then);
}

/// One system generation.
pub struct Generation {
    pub number: u32,
    /// Seconds since the epoch.
    pub created: i64,
    pub current: bool,
}

/// The generation running now.
pub fn current_generation() -> Option<Generation> {
    generations().into_iter().find(|g| g.current)
}

/// The number the next build gets: one past the newest, even when an older one runs.
pub fn next_generation() -> Option<u32> {
    generations().first().map(|g| g.number + 1)
}

/// The generation going back would switch to: the newest one before the current.
pub fn previous_generation() -> Option<u32> {
    let list = generations();
    let current = list.iter().find(|g| g.current)?.number;
    list.iter().map(|g| g.number).filter(|n| *n < current).max()
}

/// When something happened, the way people say it: "today", "yesterday", "Tuesday",
/// "2 November".
pub fn when(secs: i64) -> String {
    let Some(then) = gtk::glib::DateTime::from_unix_local(secs).ok() else { return String::new() };
    let Some(now) = gtk::glib::DateTime::now_local().ok() else { return String::new() };
    let days = if now.year() == then.year() { now.day_of_year() - then.day_of_year() } else { 8 };
    match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        2..=6 => then.format("%A").map(|s| s.to_string()).unwrap_or_default(),
        _ => then.format("%e %B").map(|s| s.trim().to_string()).unwrap_or_default(),
    }
}

/// The system's generations, newest first.
pub fn generations() -> Vec<Generation> {
    let current: Option<u32> = std::fs::read_link(Path::new(PROFILES).join("system"))
        .ok()
        .and_then(|l| l.to_string_lossy().strip_prefix("system-")?.strip_suffix("-link")?.parse().ok());
    let mut list: Vec<Generation> = std::fs::read_dir(PROFILES)
        .map(|d| {
            d.flatten()
                .filter_map(|e| {
                    let number: u32 =
                        e.file_name().to_string_lossy().strip_prefix("system-")?.strip_suffix("-link")?.parse().ok()?;
                    let created =
                        e.path().symlink_metadata().ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs()
                            as i64;
                    Some(Generation { number, created, current: Some(number) == current })
                })
                .collect()
        })
        .unwrap_or_default();
    list.sort_by_key(|g| std::cmp::Reverse(g.number));
    list
}

/// `NAME VERSION` from os-release.
pub fn version() -> String {
    let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let field = |k: &str| {
        text.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).map(|v| v.trim_matches('"').to_string()).unwrap_or_default()
    };
    let name = field("NAME");
    let version = field("VERSION");
    format!("{} {}", if name.is_empty() { "NyxOS".into() } else { name }, version).trim().to_string()
}

/// Turn one of the System Manager's services on or off.
pub fn set_service(name: &str, on: bool, done: &'static str, section: &'static str) {
    apply(vec![Change::Service { name: name.into(), enabled: Some(on) }], done, move |_| window::rebuild(section));
}

pub fn set_power(key: &str, action: &str, section: &'static str) {
    let change = Change::Power { key: key.into(), action: Some(action.into()) };
    apply(vec![change], "Saved", move |_| window::rebuild(section));
}

pub fn set_hibernate_delay(secs: &str, section: &'static str) {
    let Ok(seconds) = secs.parse::<u32>() else { return };
    apply(vec![Change::HibernateDelay { seconds: Some(seconds) }], "Saved", move |_| window::rebuild(section));
}

/// Whether the kernel can hibernate and there's swap to hibernate to.
pub fn hibernation_available() -> bool {
    let disk = std::fs::read_to_string("/sys/power/state").is_ok_and(|s| s.split_whitespace().any(|w| w == "disk"));
    let swap = std::fs::read_to_string("/proc/swaps").is_ok_and(|s| s.lines().count() > 1);
    disk && swap
}

/// Mark a row as a system setting, applied with a rebuild.
pub fn tag_system(row: &gtk::Box) {
    widgets::tag_row(row, "System");
}

/// The icon for a system generation, and for settings that are built into one: stacked
/// layers (Adwaita has none, so it's written where GTK's icon lookup finds it).
pub fn layers_icon() -> &'static str {
    const NAME: &str = "nyx-layers-symbolic";
    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#2e3436" d="M8 1l7 3.5L8 8 1 4.5zM1.6 7.1L8 10.3l6.4-3.2.6 1.3L8 11.9 1 8.4zM1.6 10.6L8 13.8l6.4-3.2.6 1.3L8 15.4 1 11.9z"/></svg>"##;
    thread_local! {
        static DONE: Cell<bool> = const { Cell::new(false) };
    }
    if !DONE.with(|d| d.replace(true))
        && let Some(display) = gtk::gdk::Display::default()
    {
        let dir = crate::paths::cache_dir().join("nyxos-icons");
        let path = dir.join(format!("{NAME}.svg"));
        if std::fs::create_dir_all(&dir).is_ok() {
            if std::fs::read_to_string(&path).ok().as_deref() != Some(SVG) {
                let _ = std::fs::write(&path, SVG);
            }
            gtk::IconTheme::for_display(&display).add_search_path(&dir);
        }
    }
    NAME
}

/// Mark a row as a system setting, applied with a rebuild: a small icon after its title
/// that says so when pointed at (the "System" tag, where every row on a page is one).
pub fn mark_system(row: &gtk::Box) {
    if let Some(line) = title_line(row) {
        let i = gtk::Image::from_icon_name(layers_icon());
        i.add_css_class("nx-sys");
        i.set_valign(gtk::Align::Center);
        i.set_tooltip_text(Some("A system setting, applied with a rebuild"));
        line.append(&i);
    }
}

/// `w` as wide as it likes when there's room, but with no minimum width: a preview's
/// parts mustn't hold the window wider than the page's narrow layout needs, or the page
/// never gets narrow enough to put the preview above the controls.
pub(crate) fn shrinkable(w: &impl IsA<gtk::Widget>) -> Shrink {
    let s: Shrink = gtk::glib::Object::new();
    s.set_overflow(gtk::Overflow::Hidden);
    w.set_parent(&s);
    s
}

/// `w` asking for no more than `max` pixels of width, though never less than it needs
/// (a drop-down whose longest choice would crowd its row's title).
pub(crate) fn capped(w: &impl IsA<gtk::Widget>, max: i32) -> Shrink {
    use gtk::subclass::prelude::ObjectSubclassIsExt;
    let s: Shrink = gtk::glib::Object::new();
    s.imp().cap.set(max);
    w.set_parent(&s);
    s
}

mod shrink {
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct Shrink {
        /// The most natural width it asks for, when set: then it keeps its child's minimum.
        pub cap: std::cell::Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Shrink {
        const NAME: &'static str = "NyxSettingsShrink";
        type Type = super::Shrink;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Shrink {
        fn dispose(&self) {
            while let Some(c) = self.obj().first_child() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for Shrink {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(child) = self.obj().first_child() else { return (0, 0, -1, -1) };
            if orientation == gtk::Orientation::Horizontal {
                let (min, natural, _, _) = child.measure(orientation, for_size);
                let cap = self.cap.get();
                if cap > 0 {
                    return (min, natural.min(cap).max(min), -1, -1);
                }
                return (0, natural, -1, -1);
            }
            // Never ask the child about a width under its own minimum.
            let least = child.measure(gtk::Orientation::Horizontal, -1).0;
            let width = if for_size < 0 { -1 } else { for_size.max(least) };
            let (min, natural, _, _) = child.measure(orientation, width);
            (min, natural, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.obj().first_child() {
                let width = width.max(child.measure(gtk::Orientation::Horizontal, -1).0);
                let height = height.max(child.measure(gtk::Orientation::Vertical, width).0);
                child.allocate(width, height, baseline, None);
            }
        }
    }
}

gtk::glib::wrapper! {
    /// See [`shrinkable`].
    pub struct Shrink(ObjectSubclass<shrink::Shrink>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// The first widget under `w` with the CSS class `class`.
pub(crate) fn descendant(w: &impl IsA<gtk::Widget>, class: &str) -> Option<gtk::Widget> {
    let mut c = w.as_ref().first_child();
    while let Some(child) = c {
        if child.has_css_class(class) {
            return Some(child);
        }
        if let Some(found) = descendant(&child, class) {
            return Some(found);
        }
        c = child.next_sibling();
    }
    None
}

/// A row's title in a line of its own, so marks can sit beside it (a dot before it, the
/// system mark after it).
pub(crate) fn title_line(row: &gtk::Box) -> Option<gtk::Box> {
    let title = descendant(row, "settings-option-title")?;
    // A title squeezed by its row's control breaks between words, never inside one.
    if let Some(l) = title.downcast_ref::<gtk::Label>() {
        l.set_wrap_mode(gtk::pango::WrapMode::Word);
    }
    let parent = title.parent().and_downcast::<gtk::Box>()?;
    if parent.has_css_class("nx-title-line") {
        return Some(parent);
    }
    let line = widgets::hbox(6);
    line.add_css_class("nx-title-line");
    parent.insert_child_after(&line, Some(&title));
    parent.remove(&title);
    title.set_hexpand(false);
    line.append(&title);
    Some(line)
}

/// `nyx-system update status --json`, the parts Home needs.
#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateStatus {
    #[serde(default)]
    targets: Vec<UpdateTarget>,
    #[serde(default)]
    last_checked: i64,
    /// Each kind's update setting; `off` isn't checked.
    #[serde(default)]
    policies: std::collections::BTreeMap<String, String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateTarget {
    name: String,
    category: String,
    #[serde(default)]
    provider: String,
    available: Option<String>,
}

/// "Firefox", "Firefox and Zed", "Firefox, Zed and 3 more".
fn names(list: &[&str]) -> String {
    match list {
        [] => String::new(),
        [a] => a.to_string(),
        [a, b] => format!("{a} and {b}"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
    }
}

/// "What needs you" on Home: rows that come in as their checks answer, kept in one
/// order (updates, firmware, accounts, the disk) and counted beside the title.
struct Needs {
    group: widgets::Group,
    count: gtk::Label,
}

impl Needs {
    fn add(&self, rank: u8, row: &gtk::Box) {
        row.set_widget_name(&format!("nx-need-{rank}"));
        let rank_of = |w: &gtk::Widget| w.widget_name().strip_prefix("nx-need-").and_then(|n| n.parse::<u8>().ok());
        let mut after: Option<gtk::Widget> = None;
        let mut c = self.group.list.first_child();
        while let Some(child) = c {
            if rank_of(&child).is_some_and(|r| r <= rank) {
                after = Some(child.clone());
            }
            c = child.next_sibling();
        }
        self.group.list.insert_child_after(row, after.as_ref());
        self.group.list.set_visible(true);
        widgets::mark_first_rows(self.group.list.upcast_ref());
        let mut n = 0;
        let mut c = self.group.list.first_child();
        while let Some(child) = c {
            n += usize::from(rank_of(&child).is_some());
            c = child.next_sibling();
        }
        self.count.set_text(&n.to_string());
        self.group.wrapper.set_visible(n > 0);
        window::set_nav_badge("home", Some(&n.to_string()));
    }
}

/// Home: the generation in the badge, and what needs you (updates waiting, an account
/// to sign in to again, a nearly full disk) above the generation that's running.
pub fn home_updates(page: &Page, hero: &Hero) {
    let current = current_generation();
    hero.count.set_text(&current.as_ref().map(|g| g.number.to_string()).unwrap_or_else(|| "—".into()));
    hero.caption.set_text("Generation");
    hero.badge.set_tooltip_text(Some("Updates and earlier generations"));
    hero.badge.connect_clicked(|_| window::navigate("updates"));

    // What needs you: filled in as the checks answer, hidden while there's nothing.
    let group = page.group("Needs you");
    group.wrapper.add_css_class("nx-needs");
    group.wrapper.set_visible(false);
    let count = widgets::label("", "nx-count");
    if let Some(head) = group.wrapper.first_child().and_downcast::<gtk::Box>()
        && let Some(title) = head.first_child()
    {
        let line = widgets::hbox(8);
        head.insert_child_after(&line, Some(&title));
        head.remove(&title);
        line.append(&title);
        count.set_valign(gtk::Align::Center);
        line.append(&count);
    }
    // What needs you leads the column, wherever Home asks for it.
    let top = group.wrapper.clone();
    gtk::glib::idle_add_local_once(move || {
        if let Some(parent) = top.parent().and_downcast::<gtk::Box>() {
            parent.reorder_child_after(&top, None::<&gtk::Widget>);
        }
    });
    let needs = std::rc::Rc::new(Needs { group, count });
    window::set_nav_badge("home", None);

    let g = page.group("NyxOS");
    let earlier = generations().iter().filter(|x| !x.current).count();
    let (title, desc) = match &current {
        Some(c) => (
            format!("Generation {}", c.number),
            match earlier {
                0 => format!("Running since {}.", when(c.created)),
                1 => format!("Running since {}; 1 earlier one is in the boot menu.", when(c.created)),
                n => format!("Running since {}; {n} earlier ones are in the boot menu.", when(c.created)),
            },
        ),
        None => (version(), "Updates has what's new, the update settings and earlier generations.".to_string()),
    };
    let (r, value) = widgets::nav_row(Some(layers_icon()), &title, &desc, &version(), || window::navigate("updates"));
    widgets::keywords("update upgrade generation rollback version nixos");
    g.add(&r);

    let n = needs.clone();
    cmd::run_async(&["nyx-system", "update", "status", "--json"], move |r| {
        let needs = n;
        let Some(s) = r.ok().and_then(|t| serde_json::from_str::<UpdateStatus>(&t).ok()) else { return };
        let checked = |p: &str| s.policies.get(p).is_none_or(|v| v != "off");
        let ready: Vec<&UpdateTarget> = s
            .targets
            .iter()
            .filter(|t| t.available.is_some() && checked(&t.category))
            // Apps that come with the system update with it.
            .filter(|t| t.provider != "channel" || t.category == "system")
            .collect();
        let (firmware, rest): (Vec<&UpdateTarget>, Vec<&UpdateTarget>) = ready.into_iter().partition(|t| t.category == "firmware");
        let system = rest.iter().find(|t| t.category == "system");
        let others: Vec<&str> = rest.iter().filter(|t| t.category != "system").map(|t| t.name.as_str()).collect();
        let total = rest.len() + firmware.len();
        let ago = if s.last_checked > 0 { format!("Checked {}.", when(s.last_checked)) } else { String::new() };
        value.set_text(&match total {
            0 if s.last_checked > 0 => "Up to date".to_string(),
            0 => version(),
            1 => "1 update".to_string(),
            n => format!("{n} updates"),
        });
        widgets::begin_section("home");
        if !rest.is_empty() {
            let title = match (system, rest.len()) {
                (Some(t), _) => format!("{} {} is ready", t.name, t.available.as_deref().unwrap_or_default()),
                (None, 1) => format!("{} {} is ready", rest[0].name, rest[0].available.as_deref().unwrap_or_default()),
                (None, n) => format!("{n} updates are ready"),
            };
            let next = next_generation();
            let desc = match (system.is_some(), others.is_empty(), next) {
                (true, true, Some(n)) => format!("Builds generation {n}, switched to when the build works."),
                (true, true, None) => "Builds a new generation, switched to when the build works.".to_string(),
                (true, false, Some(n)) => format!("With {} · builds generation {n}.", names(&others)),
                (true, false, None) => format!("With {}. The system part builds a new generation.", names(&others)),
                (false, _, _) => format!("{}. {ago}", names(&others)),
            };
            let b = gtk::Button::with_label("Update");
            b.add_css_class("suggested-action");
            b.set_valign(gtk::Align::Center);
            b.set_sensitive(!busy());
            b.connect_clicked(|b| {
                b.set_sensitive(false);
                b.set_label("Updating…");
                updates::update(&[], "Updating everything. This can take a while.");
            });
            let r = widgets::icon_row("software-update-available-symbolic", &title, desc.trim(), Some(b.upcast_ref()));
            widgets::keywords("update upgrade install new version");
            needs.add(0, &r);
        }
        if !firmware.is_empty() {
            let devices: Vec<&str> = firmware.iter().map(|t| t.name.as_str()).collect();
            let (r, _) = widgets::nav_row(
                Some("system-run-symbolic"),
                if firmware.len() == 1 { "A firmware update is ready" } else { "Firmware updates are ready" },
                &format!("For {}. Firmware is only updated when you choose; it can't be undone.", names(&devices)),
                "",
                || window::navigate("updates"),
            );
            widgets::keywords("firmware fwupd update");
            needs.add(1, &r);
        }
    });

    // Online accounts that stopped syncing until you sign in again.
    let n = needs.clone();
    cmd::background(
        || nyx_accounts_native::client::Client::connect().and_then(|c| c.list()).unwrap_or_default(),
        move |accounts| {
            widgets::begin_section("home");
            for a in accounts.iter().filter(|a| a.status == "needs_sign_in") {
                let name = if a.label.trim().is_empty() { a.identity.as_str() } else { a.label.as_str() };
                let b = gtk::Button::with_label("Sign in");
                b.set_valign(gtk::Align::Center);
                b.connect_clicked(|_| window::navigate("online-accounts"));
                let desc = a.error.clone().filter(|e| !e.trim().is_empty()).unwrap_or_else(|| {
                    "Its mail, calendar and files wait until you do.".to_string()
                });
                let r = widgets::icon_row("mail-unread-symbolic", &format!("{name} needs you to sign in"), &desc, Some(b.upcast_ref()));
                widgets::keywords("account sign in again expired online accounts");
                n.add(2, &r);
            }
        },
    );

    // The system's disk, once it's nearly full.
    let n = needs;
    cmd::background(
        || crate::backend::sysinfo::disks().into_iter().find(|d| d.mount == "/"),
        move |disk| {
            let Some(d) = disk else { return };
            let used = d.used as f64 / d.size as f64;
            if used < 0.85 {
                return;
            }
            widgets::begin_section("home");
            let free = crate::backend::sysinfo::bytes_text(d.size.saturating_sub(d.used));
            let old = generations().iter().filter(|g| !g.current).count();
            let desc = match old {
                0 => format!("{free} free."),
                1 => format!("{free} free · 1 earlier generation is kept."),
                n => format!("{free} free · {n} earlier generations are kept."),
            };
            let b = gtk::Button::with_label("Clean up");
            b.set_valign(gtk::Align::Center);
            b.connect_clicked(|_| window::navigate("disks"));
            let r = widgets::icon_row(
                "drive-harddisk-symbolic",
                &format!("The disk is {}% full", (used * 100.0).round() as u32),
                &desc,
                Some(b.upcast_ref()),
            );
            widgets::keywords("disk full space storage clean up");
            n.add(3, &r);
        },
    );
}

/// About: NyxOS's version and generation; updates and going back are on Updates.
pub fn about(page: &Page) {
    let g = page.group("NyxOS");
    let (r, _) = widgets::info_row("Version", &version());
    g.add(&r);
    if let Some(current) = generations().into_iter().find(|x| x.current) {
        let (r, _) = widgets::info_row("Generation", &current.number.to_string());
        g.add(&r);
    }
    let (r, _) =
        widgets::button_row("Updates and generations", "Update NyxOS, or go back to an earlier generation.", "Updates", |_| {
            window::navigate("updates")
        });
    widgets::keywords("update upgrade rollback generation version nixos");
    g.add(&r);
}
