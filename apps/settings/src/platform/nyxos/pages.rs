//! NyxOS's system pages: time and language, services and groups, software and updates,
//! and the boot and sign-in screens. Changes are collected on the page and applied
//! together through the Nyx System Manager, as one rebuild.

use crate::platform::nyxos::{self, tag_system};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use nyx_system_proto::{Change, GROUPS, SERVICES, valid_hostname};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// A page's changes waiting to be applied, and the button that applies them.
struct Pending {
    changes: RefCell<BTreeMap<String, Change>>,
    apply: gtk::Button,
}

impl Pending {
    /// Changes for `section`, which is rebuilt after they're applied; [`Pending::place`]
    /// puts the Apply row on the page.
    fn new(section: &'static str) -> Rc<Pending> {
        let apply = gtk::Button::with_label("Apply");
        apply.add_css_class("suggested-action");
        apply.set_sensitive(false);
        let p = Rc::new(Pending { changes: RefCell::default(), apply: apply.clone() });
        let weak = Rc::downgrade(&p);
        apply.connect_clicked(move |b| {
            let Some(p) = weak.upgrade() else { return };
            let changes: Vec<Change> = p.changes.borrow().values().cloned().collect();
            b.set_sensitive(false);
            nyxos::apply(changes, "Applied", move |_| window::rebuild(section));
        });
        p
    }

    /// The Apply row, in its own group where the page is now.
    fn place(&self, page: &Page) {
        let g = page.plain_group("");
        g.add(&widgets::row(
            "Apply system changes",
            "System settings are built into a new generation and switched to when the build works; it takes a minute or two. An administrator's password is asked for.",
            Some(self.apply.upcast_ref()),
        ));
    }

    fn set(&self, key: &str, change: Option<Change>) {
        let mut c = self.changes.borrow_mut();
        match change {
            Some(ch) => c.insert(key.to_string(), ch),
            None => c.remove(key),
        };
        self.apply.set_sensitive(!c.is_empty() && !nyxos::busy());
    }
}

fn options(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

const LOCALES: &[(&str, &str)] = &[
    ("en_US.UTF-8", "English (United States)"),
    ("en_GB.UTF-8", "English (United Kingdom)"),
    ("en_CA.UTF-8", "English (Canada)"),
    ("en_AU.UTF-8", "English (Australia)"),
    ("de_DE.UTF-8", "Deutsch (Deutschland)"),
    ("fr_FR.UTF-8", "Français (France)"),
    ("es_ES.UTF-8", "Español (España)"),
    ("es_MX.UTF-8", "Español (México)"),
    ("it_IT.UTF-8", "Italiano"),
    ("pt_PT.UTF-8", "Português (Portugal)"),
    ("pt_BR.UTF-8", "Português (Brasil)"),
    ("nl_NL.UTF-8", "Nederlands"),
    ("sv_SE.UTF-8", "Svenska"),
    ("nb_NO.UTF-8", "Norsk bokmål"),
    ("da_DK.UTF-8", "Dansk"),
    ("fi_FI.UTF-8", "Suomi"),
    ("pl_PL.UTF-8", "Polski"),
    ("cs_CZ.UTF-8", "Čeština"),
    ("tr_TR.UTF-8", "Türkçe"),
    ("ru_RU.UTF-8", "Русский"),
    ("uk_UA.UTF-8", "Українська"),
    ("ja_JP.UTF-8", "日本語"),
    ("ko_KR.UTF-8", "한국어"),
    ("zh_CN.UTF-8", "中文 (简体)"),
];

const KEYBOARDS: &[(&str, &str)] = &[
    ("us", "English (US)"),
    ("gb", "English (UK)"),
    ("ca", "Canadian"),
    ("de", "German"),
    ("ch", "Swiss"),
    ("fr", "French"),
    ("be", "Belgian"),
    ("es", "Spanish"),
    ("latam", "Spanish (Latin America)"),
    ("pt", "Portuguese"),
    ("br", "Portuguese (Brazil)"),
    ("it", "Italian"),
    ("nl", "Dutch"),
    ("se", "Swedish"),
    ("no", "Norwegian"),
    ("dk", "Danish"),
    ("fi", "Finnish"),
    ("pl", "Polish"),
    ("cz", "Czech"),
    ("tr", "Turkish"),
    ("ru", "Russian"),
    ("ua", "Ukrainian"),
    ("jp", "Japanese"),
    ("kr", "Korean"),
];

/// `options` with `current` in it, when it isn't one of them already.
fn with_current(mut list: Vec<(String, String)>, current: &str) -> Vec<(String, String)> {
    if !current.is_empty() && !list.iter().any(|(id, _)| id == current) {
        list.insert(0, (current.to_string(), current.to_string()));
    }
    list
}

pub fn region(page: &Page) {
    let s = nyxos::settings();
    let pending = Pending::new("region");
    let set = {
        let p = pending.clone();
        move |key: &str, change: Option<Change>| p.set(key, change)
    };

    let g = page.group("Date and time");
    let tz_now =
        cmd::output(&["timedatectl", "show", "-p", "Timezone", "--value"]).map(|t| t.trim().to_string()).unwrap_or_default();
    let zones: Vec<(String, String)> = cmd::output(&["timedatectl", "list-timezones"])
        .unwrap_or_default()
        .lines()
        .map(|z| (z.to_string(), z.replace('_', " ")))
        .collect();
    let set1 = set.clone();
    let now = tz_now.clone();
    let (r, dd) = widgets::choice_row("Time zone", "", with_current(zones, &tz_now), &tz_now, move |z| {
        set1("time-zone", (z != now).then_some(Change::TimeZone { value: Some(z) }));
    });
    dd.set_enable_search(true);
    dd.set_expression(Some(gtk::PropertyExpression::new(gtk::StringObject::static_type(), None::<gtk::Expression>, "string")));
    tag_system(&r);
    widgets::keywords("timezone zone clock city");
    g.add(&r);
    let clock = cmd::output(&["date", "+%A %e %B %Y, %H:%M"]).unwrap_or_default();
    let (r, _) = widgets::info_row("Now", clock.trim());
    g.add(&r);

    let g = page.group("Language and keyboard");
    let lang = s.locale.clone().or_else(|| std::env::var("LANG").ok()).unwrap_or_else(|| "en_US.UTF-8".into());
    let set2 = set.clone();
    let now = lang.clone();
    let (r, _) = widgets::choice_row(
        "Language and formats",
        "For the whole computer: menus where apps are translated, dates, numbers and currency.",
        with_current(options(LOCALES), &lang),
        &lang,
        move |l| set2("locale", (l != now).then_some(Change::Locale { value: Some(l) })),
    );
    tag_system(&r);
    widgets::keywords("language locale format region translation");
    g.add(&r);
    let kb = s.keyboard_layout.clone().unwrap_or_else(|| "us".into());
    let set3 = set.clone();
    let now = kb.clone();
    let (r, _) = widgets::choice_row(
        "Keyboard layout",
        "On the desktop and the console. Settings → Keyboard has more layouts to switch between.",
        with_current(options(KEYBOARDS), &kb),
        &kb,
        move |k| set3("keyboard", (k != now).then_some(Change::KeyboardLayout { value: Some(k) })),
    );
    tag_system(&r);
    widgets::keywords("keyboard layout xkb keymap");
    g.add(&r);

    let g = page.group("This computer");
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname").map(|h| h.trim().to_string()).unwrap_or_default();
    let set4 = set.clone();
    let now = host.clone();
    let (r, _) = widgets::entry_row("Name", "How this computer shows on your network.", &host, "nyxos", move |name| {
        let name = name.trim().to_string();
        if name != now && !valid_hostname(&name) {
            window::toast("A name is letters, digits and dashes, not at either end, up to 63");
            set4("hostname", None);
            return;
        }
        set4("hostname", (name != now).then_some(Change::Hostname { value: Some(name) }));
    });
    tag_system(&r);
    widgets::keywords("hostname computer name network");
    g.add(&r);

    pending.place(page);
}

pub fn services(page: &Page) {
    let s = nyxos::settings();
    let pending = Pending::new("services");
    let g = page.group("System services");
    g.note(
        "Services NyxOS turns on and off for the whole computer. Default is what NyxOS and this computer's configuration choose.",
    );
    for (name, what) in SERVICES {
        let current = match s.services.get(*name) {
            Some(true) => "on",
            Some(false) => "off",
            None => "default",
        };
        let p = pending.clone();
        let (n, now) = (name.to_string(), current.to_string());
        let (r, _) =
            widgets::choice_row(what, "", options(&[("default", "Default"), ("on", "On"), ("off", "Off")]), current, move |v| {
                let enabled = match v.as_str() {
                    "on" => Some(true),
                    "off" => Some(false),
                    _ => None,
                };
                p.set(&format!("service-{n}"), (v != now).then(|| Change::Service { name: n.clone(), enabled }));
            });
        tag_system(&r);
        widgets::keywords(name);
        g.add(&r);
    }

    let g = page.group("Groups");
    g.note("Being in one of these lets an account use containers, virtual machines or devices directly. Some, like docker, are as good as an administrator.");
    let me = std::env::var("USER").unwrap_or_default();
    for (group, what) in GROUPS {
        let members = s.groups.get(*group).cloned().unwrap_or_default();
        let mine = members.contains(&me);
        let others: Vec<String> = members.iter().filter(|m| **m != me).cloned().collect();
        let desc = if others.is_empty() { what.to_string() } else { format!("{what}. Also: {}", others.join(", ")) };
        let p = pending.clone();
        let (g2, u) = (group.to_string(), me.clone());
        let (r, _) = widgets::switch_row(group, &desc, mine, move |on| {
            p.set(
                &format!("group-{g2}"),
                (on != mine).then(|| Change::GroupMember { group: g2.clone(), user: u.clone(), member: on }),
            );
        });
        tag_system(&r);
        g.add(&r);
    }
    pending.place(page);
    crate::sections::services::user_and_system_lists(page);
}

fn date(secs: i64) -> String {
    gtk::glib::DateTime::from_unix_local(secs)
        .and_then(|d| d.format("%e %b %Y, %H:%M"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub fn software(page: &Page) {
    let s = nyxos::settings();
    nyxos::updates::group(page);
    nyxos::updates::history(page);
    let g = page.group("Store");
    let (r, _) = widgets::button_row(
        "Nyx Store",
        "Nyx Store installs apps from the NyxOS catalog, nixpkgs and Flathub, and updates Flathub apps.",
        "Open Store",
        |_| {
            let _ = std::process::Command::new("nyx-store").spawn();
        },
    );
    g.add(&r);

    let g = page.group("Generations");
    let list = nyxos::generations();
    g.note("Every system change makes a new generation. Going back undoes the last change, software and system settings with it; your files stay. Earlier ones are also in the boot menu.");
    let previous = list.iter().find(|x| !x.current && list.iter().any(|c| c.current && c.number > x.number)).map(|x| x.number);
    for g8 in list.iter().take(8) {
        let r = widgets::row(&format!("Generation {}", g8.number), &date(g8.created), None);
        if g8.current {
            widgets::tag_row(&r, "Current");
        }
        g.add(&r);
    }
    if let Some(n) = previous {
        let back = widgets::confirm_button("Go Back", &format!("Go back to generation {n}?"), |b| {
            b.set_sensitive(false);
            nyxos::rollback(|_| window::rebuild("software"));
        });
        back.set_sensitive(!nyxos::busy());
        g.add(&widgets::row(
            &format!("Go back to generation {n}"),
            "Switches to it now; it stays until the next change.",
            Some(back.upcast_ref()),
        ));
        widgets::keywords("rollback undo previous generation revert");
    }

    let g = page.group("Installed for everyone");
    if s.packages.is_empty() && s.nyx_apps.is_empty() {
        g.note("Nothing beyond what NyxOS includes. Nyx Store and Nyx Welcome add software here.");
    }
    for attr in &s.packages {
        let a = attr.clone();
        let remove = widgets::confirm_button("Remove", &format!("Remove {attr} for everyone?"), move |b| {
            b.set_sensitive(false);
            nyxos::apply(vec![Change::RemovePackages { packages: vec![a.clone()] }], "Removed", |_| window::rebuild("software"));
        });
        let r = widgets::row(attr, "From nixpkgs", Some(remove.upcast_ref()));
        if s.unfree.contains(attr) {
            widgets::tag_row(&r, "Unfree license");
        }
        g.add(&r);
    }
    for (app, on) in &s.nyx_apps {
        let a = app.clone();
        let reset = widgets::confirm_button("Reset", &format!("Give nyx-{app} back to NyxOS's default?"), move |b| {
            b.set_sensitive(false);
            nyxos::apply(vec![Change::NyxApp { name: a.clone(), enabled: None }], "Done", |_| window::rebuild("software"));
        });
        let r = widgets::row(&format!("Nyx {app}"), if *on { "Turned on" } else { "Turned off" }, Some(reset.upcast_ref()));
        g.add(&r);
    }
    nyxos::updates::settings(page);
}

pub fn boot(page: &Page) {
    let s = nyxos::settings();
    let pending = Pending::new("boot");
    let g = page.group("Boot and sign-in screens");
    g.note("How the screens before you sign in look, for everyone. Unless set here, they follow the system theme.");
    let mut themes = vec![("".to_string(), "Same as the system theme".to_string())];
    themes.extend(nyx_theme::all().into_iter().map(|t| (t.id, t.name)));
    for (key, title, current) in [
        ("boot", "Boot screen", s.theme.boot.clone().unwrap_or_default()),
        ("login", "Sign-in screen", s.theme.login.clone().unwrap_or_default()),
    ] {
        let p = pending.clone();
        let now = current.clone();
        let (r, _) = widgets::choice_row(title, "", themes.clone(), &current, move |t| {
            let value = (!t.is_empty()).then(|| t.clone());
            let change = if key == "boot" { Change::BootTheme { value } } else { Change::LoginTheme { value } };
            p.set(key, (t != now).then_some(change));
        });
        tag_system(&r);
        widgets::keywords("plymouth greeter login screen boot splash theme");
        g.add(&r);
    }
    let p = pending.clone();
    let r = crate::sections::themes::system_logo_row(s.logo.as_ref(), Rc::new(move |change| p.set("logo", change)));
    tag_system(&r);
    widgets::keywords("logo brand mark emblem picture svg png boot sign-in");
    g.add(&r);
    pending.place(page);
    let g = page.group("Recovery");
    let (r, _) = widgets::button_row(
        "Earlier generations",
        "The boot menu lists the system's earlier generations, to start one that worked if a change didn't.",
        "Software & Updates",
        |_| window::navigate("software"),
    );
    g.add(&r);
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
