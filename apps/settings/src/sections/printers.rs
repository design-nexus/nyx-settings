//! Printers through CUPS. Adding and removing needs the root helper; picking
//! your default and printing a test page don't.

use crate::dialog::{Field, ask};
use crate::sections::accounts::{require_helper, run_admin};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use std::process::{Command, Stdio};

#[derive(Debug, PartialEq)]
struct Printer {
    name: String,
    state: String,
    uri: String,
}

/// `lpstat -p` ("printer NAME is idle.") joined with `lpstat -v` ("device for NAME: URI").
fn parse_printers(status: &str, devices: &str) -> Vec<Printer> {
    status
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("printer ")?;
            let (name, state) = rest.split_once(' ')?;
            let uri = devices.lines().find_map(|d| d.strip_prefix(&format!("device for {name}: "))).unwrap_or("").to_string();
            let state = state.trim_start_matches("is ").split('.').next().unwrap_or("").to_string();
            Some(Printer { name: name.to_string(), state, uri })
        })
        .collect()
}

/// `system default destination: NAME`
fn parse_default(text: &str) -> Option<String> {
    text.strip_prefix("system default destination: ").map(|n| n.trim().to_string())
}

/// A found printer: (display name, ipp address).
type Found = (String, String);

/// `avahi-browse -rtp _ipp._tcp` resolved lines: `=;iface;proto;name;type;domain;host;addr;port;"txt"`.
fn parse_found(text: &str) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    for line in text.lines().filter(|l| l.starts_with('=')) {
        let f: Vec<&str> = line.split(';').collect();
        if f.len() < 10 || f[2] != "IPv4" {
            continue;
        }
        let rp = f[9].split('"').find_map(|t| t.strip_prefix("rp=")).unwrap_or("ipp/print");
        let uri = format!("ipp://{}:{}/{}", f[7], f[8], rp.trim_start_matches('/'));
        if !found.iter().any(|(_, u)| *u == uri) {
            found.push((f[3].replace("\\032", " "), uri));
        }
    }
    found
}

/// A queue name CUPS accepts, made from a display name.
fn queue_name(display: &str) -> String {
    let mut s: String = display.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    s = s.trim_matches('_').to_string();
    if s.is_empty() { "Printer".into() } else { s.chars().take(60).collect() }
}

pub fn build(page: &Page) {
    if !cmd::present("lpstat") {
        page.banner("Printing isn't installed. Install cups to use printers: sudo pacman -S cups", true);
        return;
    }
    let can_edit = require_helper(page, "printers", "adding and removing printers");
    let g = page.group("Printing");
    let on = cmd::output(&["systemctl", "is-active", "cups.service"]).is_some_and(|s| s == "active");
    let (r, sw) = widgets::switch_row(
        "Printing service",
        "CUPS, which sends documents to printers. Turn it on to add or use printers.",
        on,
        move |now| {
            if now != on {
                run_admin(
                    &["service", if now { "enable-now" } else { "disable-now" }, "cups.service"],
                    None,
                    if now { "Printing is on" } else { "Printing is off" },
                    "printers",
                );
            }
        },
    );
    sw.set_sensitive(can_edit);
    widgets::keywords("cups print spooler service");
    g.add(&r);
    if !on {
        return;
    }

    // Asking CUPS can take a moment the first time, so the page opens at once and
    // the printers appear when it answers.
    let g = page.group("Printers");
    let holder = widgets::vbox(6);
    holder.append(&widgets::label("Looking for printers…", "dim"));
    g.add(&holder);
    cmd::background(
        || {
            // One call: each lpstat run costs about a second.
            // lpstat exits non-zero when there are no printers, so read stdout whatever the status.
            let all = Command::new("lpstat")
                .args(["-d", "-p", "-v"])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            (parse_printers(&all, &all), all.lines().find_map(parse_default))
        },
        move |(printers, default)| {
            // These rows are added after the page was built, so tell search where they belong.
            widgets::begin_section("printers");
            fill(&g, &holder, can_edit, printers, default);
        },
    );
}

fn fill(g: &widgets::Group, holder: &gtk::Box, can_edit: bool, printers: Vec<Printer>, default: Option<String>) {
    widgets::forget_rows(holder);
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    if printers.is_empty() {
        holder.append(&widgets::label("No printers yet. Find one on your network below, or enter its address.", "dim"));
    }
    for p in &printers {
        let controls = widgets::hbox(6);
        let is_default = default.as_deref() == Some(&p.name);
        if !is_default {
            let name = p.name.clone();
            let b = gtk::Button::with_label("Make default");
            b.connect_clicked(move |_| {
                cmd::run_async(&["lpoptions", "-d", &name], |r| {
                    match r {
                        Ok(_) => window::toast("Default printer changed"),
                        Err(e) => window::toast(&format!("{e}")),
                    }
                    window::rebuild("printers");
                });
            });
            controls.append(&b);
        }
        let name = p.name.clone();
        let test = gtk::Button::with_label("Test page");
        test.connect_clicked(move |_| {
            cmd::run_async(&["lp", "-d", &name, "/usr/share/cups/data/default-testpage.pdf"], |r| match r {
                Ok(_) => window::toast("Test page sent"),
                Err(e) => window::toast(&format!("{e}")),
            });
        });
        controls.append(&test);
        if can_edit {
            let edit = gtk::Button::from_icon_name("document-edit-symbolic");
            edit.add_css_class("flat");
            edit.set_tooltip_text(Some("Edit"));
            let (name, uri) = (p.name.clone(), p.uri.clone());
            edit.connect_clicked(move |_| {
                let name = name.clone();
                ask(
                    "Edit printer address",
                    "Where the printer is on the network.",
                    vec![Field::text_with("ipp://…", &uri)],
                    "Save",
                    None,
                    move |v, _| {
                        let uri = v[0].trim().to_string();
                        if uri.is_empty() {
                            return Some("Enter the printer's address.".into());
                        }
                        run_admin(&["printer-set-uri", &name, &uri], None, "Printer updated", "printers");
                        None
                    },
                );
            });
            controls.append(&edit);
            let name = p.name.clone();
            controls.append(&widgets::confirm_button("Remove", "Remove printer?", move |_| {
                run_admin(&["printer-remove", &name], None, "Printer removed", "printers")
            }));
        }
        let r = widgets::row(
            &p.name.replace('_', " "),
            &gtk::glib::markup_escape_text(&format!("{} · {}", p.state, p.uri)),
            Some(controls.upcast_ref()),
        );
        if is_default {
            widgets::tag_row(&r, "Default");
        }
        widgets::keywords("printer print queue default test page");
        holder.append(&r);
    }
    if !can_edit {
        return;
    }
    let results = widgets::vbox(6);
    let (r, _) =
        widgets::button_row("Find printers", "Looks for printers on your network that don't need a driver.", "Search", {
            let results = results.clone();
            move |b| {
                b.set_sensitive(false);
                b.set_label("Searching…");
                let (b, results) = (b.clone(), results.clone());
                cmd::background(
                    || {
                        Command::new("timeout")
                            .args(["6", "avahi-browse", "-rtp", "_ipp._tcp"])
                            .stdin(Stdio::null())
                            .output()
                            .map(|o| parse_found(&String::from_utf8_lossy(&o.stdout)))
                            .unwrap_or_default()
                    },
                    move |found| {
                        b.set_sensitive(true);
                        b.set_label("Search");
                        widgets::forget_rows(&results);
                        widgets::begin_section("printers");
                        while let Some(c) = results.first_child() {
                            results.remove(&c);
                        }
                        if found.is_empty() {
                            results.append(&widgets::label("No printers found.", "dim"));
                        }
                        for (name, uri) in found {
                            let add = gtk::Button::with_label("Add");
                            let (queue, u) = (queue_name(&name), uri.clone());
                            add.connect_clicked(move |b| {
                                b.set_sensitive(false);
                                run_admin(&["printer-add", &queue, &u], None, "Printer added", "printers");
                            });
                            results.append(&widgets::row(&name, &gtk::glib::markup_escape_text(&uri), Some(add.upcast_ref())));
                        }
                    },
                );
            }
        });
    widgets::keywords("add discover scan network ipp airprint avahi");
    if !cmd::present("avahi-browse") {
        r.set_visible(false);
    }
    g.add(&r);
    g.add(&results);

    let (r, _) = widgets::entry_row("Add by address", "Like ipp://192.168.1.50/ipp/print. Press Enter.", "", "ipp://…", |uri| {
        let uri = uri.trim().to_string();
        if uri.is_empty() {
            return;
        }
        let host = uri.split("//").nth(1).and_then(|r| r.split(['/', ':']).next()).unwrap_or("Printer").to_string();
        run_admin(&["printer-add", &queue_name(&host), &uri], None, "Printer added", "printers");
    });
    widgets::keywords("add printer address uri url ip");
    g.add(&r);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printers_and_default() {
        let p = parse_printers(
            "printer HP_Laser is idle.  enabled since Mon\nprinter Brother disabled since x\n",
            "device for HP_Laser: ipp://h/ipp/print\ndevice for Brother: usb://B\n",
        );
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], Printer { name: "HP_Laser".into(), state: "idle".into(), uri: "ipp://h/ipp/print".into() });
        assert_eq!(p[1].uri, "usb://B");
        assert_eq!(parse_default("system default destination: HP_Laser"), Some("HP_Laser".into()));
        assert_eq!(parse_default("no system default destination"), None);
    }

    #[test]
    fn found_printers() {
        let t = "+;eth0;IPv4;HP\\032LaserJet;_ipp._tcp;local\n=;eth0;IPv4;HP\\032LaserJet;_ipp._tcp;local;hp.local;192.168.1.50;631;\"rp=ipp/print\" \"ty=HP\"\n=;eth0;IPv6;HP;_ipp._tcp;local;hp.local;fe80::1;631;\"rp=ipp/print\"\n";
        let f = parse_found(t);
        assert_eq!(f, [("HP LaserJet".to_string(), "ipp://192.168.1.50:631/ipp/print".to_string())]);
    }

    #[test]
    fn queue_names() {
        assert_eq!(queue_name("HP LaserJet 400"), "HP_LaserJet_400");
        assert_eq!(queue_name("!!"), "Printer");
        assert_eq!(queue_name("192.168.1.5"), "192_168_1_5");
    }
}
