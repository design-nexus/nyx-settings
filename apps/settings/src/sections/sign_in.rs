//! Sign-in & Security: face, fingerprint and security keys from Nyx Biometrics, and where
//! each one signs you in. Everything goes through `nyx-authctl`: it reports the state as
//! JSON, and its commands do the enrolling and the PAM changes, asking for approval in
//! their own prompts. Nothing here edits system files.
//!
//! The ways to sign in are rows on the left; the right side is a table of every place
//! you sign in against every way, ticked where it works.

use gtk::prelude::*;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::accounts::{clear, head, pill, pill_row, roomy, terminal};
use crate::widgets::{self, Page};
use crate::{cmd, window};

const CTL: &str = "nyx-authctl";
const ID: &str = "sign-in";

pub fn available() -> bool {
    cmd::present(CTL)
}

/// Run an `nyx-authctl` command in the background, then refresh the page.
fn ctl(args: Vec<String>, done: &'static str) {
    cmd::background(
        move || {
            let refs: Vec<&str> = std::iter::once(CTL).chain(args.iter().map(String::as_str)).collect();
            cmd::run(&refs).map_err(|e| e.to_string())
        },
        move |r| {
            match r {
                Ok(_) if !done.is_empty() => window::toast(done),
                Ok(_) => {}
                Err(e) => window::toast(e.lines().last().unwrap_or("That didn't work")),
            }
            window::rebuild(ID);
        },
    );
}

/// Commands that need a key touch, a PIN or a diff to read run in a terminal; refresh after.
fn in_terminal(command: &str) {
    terminal(command);
    gtk::glib::timeout_add_seconds_local_once(20, || window::rebuild_if_built(ID));
}

fn fingers(fp: &Value) -> usize {
    fp["readers"].as_array().into_iter().flatten().map(|r| r["fingers"].as_array().map_or(0, Vec::len)).sum()
}

/// What's set up, in a line ("Face, 2 fingerprints. Your password works too."), for
/// the Accounts page's card.
pub fn summary(done: impl FnOnce(String) + 'static) {
    if !available() {
        done("Nyx Biometrics isn't installed.".into());
        return;
    }
    cmd::run_async(&[CTL, "status", "--json"], move |result| {
        let v: Value = result.ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let mut parts = Vec::new();
        if v["face"]["ready"] == true {
            parts.push("Face".to_string());
        }
        let fingers = fingers(&v["fingerprint"]);
        if fingers > 0 {
            parts.push(format!("{fingers} fingerprint{}", if fingers == 1 { "" } else { "s" }));
        }
        done(if parts.is_empty() {
            "Not set up. Your password works everywhere.".into()
        } else {
            format!("{}. Your password works too.", parts.join(", "))
        });
    });
}

/// The ways besides a password that are set up for you: "Face", "Fingerprint", "Security key".
pub fn methods(done: impl FnOnce(Vec<&'static str>) + 'static) {
    cmd::run_async(&[CTL, "status", "--json"], move |result| {
        let v: Value = result.ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        done(METHODS.iter().filter(|(m, _, _)| have(&v, m)).map(|(_, name, _)| *name).collect());
    });
}

/// The ways to sign in besides a password: (policy key, name, icon).
const METHODS: [(&str, &str, &str); 3] = [
    ("face", "Face", "camera-web-symbolic"),
    ("fingerprint", "Fingerprint", "auth-fingerprint-symbolic"),
    ("security-key", "Security key", "media-flash-symbolic"),
];

fn have(status: &Value, m: &str) -> bool {
    match m {
        "face" => status["face"]["ready"] == true,
        "fingerprint" => fingers(&status["fingerprint"]) > 0,
        _ => status["keys"].as_array().is_some_and(|k| !k.is_empty()),
    }
}

/// The places sign-in happens use a picture of their own.
fn surface_icon(key: &str) -> &'static str {
    match key {
        "login" => "avatar-default-symbolic",
        "lock-screen" | "lock" => "system-lock-screen-symbolic",
        "sudo" => "utilities-terminal-symbolic",
        "polkit" => "security-high-symbolic",
        k if k.contains("pass") || k.contains("vault") => "dialog-password-symbolic",
        _ => "dialog-password-symbolic",
    }
}

/// The scan styles, by the name the lock screen reads.
const STYLES: [(&str, &str); 5] = [
    ("halo", "Halo"),
    ("circular", "Circular face scan"),
    ("cyberpunk", "Cyberpunk scanner"),
    ("minimal", "Minimal"),
    ("edge", "Edge glow"),
];

pub fn build(page: &Page) {
    if !available() {
        let g = page.group("Sign-in & Security");
        g.add(&widgets::row(
            "Nyx Biometrics isn't installed",
            "Install it to sign in with your face, a fingerprint or a security key.",
            None,
        ));
        widgets::keywords("biometrics face fingerprint security key");
        return;
    }
    let status: Rc<RefCell<Value>> = Rc::default();

    // The picture: every place against every way.
    let pv = page.split();
    let undo = widgets::labeled_button("edit-undo-symbolic", "Undo last change");
    undo.add_css_class("acc-small");
    undo.set_tooltip_text(Some("Put the sign-in files back the way they were before the last change"));
    undo.connect_clicked(|_| in_terminal("nyx-authctl pam restore"));
    let content = roomy(&pv);
    content.append(&head("What unlocks what", "Each place tries the ways in order. Your password always works too.", &[undo.upcast()]));
    let table = widgets::vbox(0);
    table.add_css_class("preview-card");
    table.add_css_class("sgn-table");
    table.append(&widgets::label("Checking this computer…", "dim"));
    content.append(&table);
    let banners = widgets::vbox(10);
    content.append(&banners);
    let foot = pv.foot(
        "dialog-information-symbolic",
        "Changes to sign-in open a terminal that shows exactly what changes and tests it before keeping it.",
    );

    // The ways to sign in.
    let ways = page.group("Ways to sign in");
    let ways_h = widgets::vbox(0);
    ways_h.append(&widgets::label("Checking this computer…", "dim"));
    ways.add(&ways_h);

    // The rarer things, a level down.
    let more = page.group("More");
    let style = overlay()["lock-screen"]["style"].as_str().unwrap_or("halo").to_string();
    let (r, _) = widgets::nav_row(
        Some("view-reveal-symbolic"),
        "How a scan looks",
        "On the lock screen, prompts and sudo",
        STYLES.iter().find(|(k, _)| *k == style).map_or("Halo", |(_, n)| n),
        || {
            widgets::subpage("How a scan looks", 560, look_group);
        },
    );
    widgets::keywords("overlay style circular cyberpunk minimal edge glow halo animation lock screen look polkit sudo intensity login screen sddm greeter theme");
    more.add(&r);
    let (r, ssh_count) = widgets::nav_row(Some("utilities-terminal-symbolic"), "SSH keys on a security key", "", "", {
        let status = status.clone();
        move || {
            let keys = status.borrow()["ssh_keys"].clone();
            widgets::subpage("SSH keys on a security key", 600, move |p| {
                let g = p.group("SSH keys on a security key");
                g.note(
                    "The private key stays on the security key; each use needs a touch. Keys kept in the vault are Nyx Passwords' to serve.",
                );
                let h = widgets::vbox(0);
                g.add(&h);
                ssh_rows(&h, &keys);
            });
        }
    });
    widgets::keywords("ssh key security key fido2 new create github load resident");
    more.add(&r);
    let (r, hw_value) = widgets::nav_row(Some("computer-symbolic"), "This computer", "Camera, reader and helper", "", {
        let status = status.clone();
        move || {
            let findings = status.borrow()["findings"].clone();
            widgets::subpage("This computer", 600, move |p| {
                let g = p.group("This computer");
                for f in findings.as_array().into_iter().flatten() {
                    let r = widgets::row(f["message"].as_str().unwrap_or_default(), "", None);
                    match f["status"].as_str() {
                        Some("ok") => pill_row(&r, "OK", "ok"),
                        Some("warn" | "missing" | "error") => pill_row(&r, "Check", "warn"),
                        _ => {}
                    }
                    g.add(&r);
                }
            });
        }
    });
    widgets::keywords("camera reader helper hardware check");
    more.add(&r);
    let (r, _) = widgets::nav_row(Some("edit-undo-symbolic"), "If something goes wrong", "Undo the last change, or recover", "", || {
        widgets::subpage("If something goes wrong", 600, recovery);
    });
    widgets::keywords("undo restore rollback pam recovery");
    more.add(&r);

    cmd::run_async(&[CTL, "status", "--json"], move |result| {
        widgets::begin_section(ID);
        clear(&ways_h);
        clear(&table);
        let parsed: Value = match result.map(|t| serde_json::from_str(&t)) {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                ways_h.append(&widgets::row("Couldn't read the status", &e.to_string(), None));
                return;
            }
            Err(e) => {
                ways_h.append(&widgets::row("Couldn't read the status", &e.to_string(), None));
                return;
            }
        };
        *status.borrow_mut() = parsed.clone();
        let s = &parsed;
        if s["helper"] != "ready" {
            let (r, _) = widgets::button_row(
                "Finish installing",
                "Nyx Biometrics needs its small administrator helper to make sign-in changes.",
                "Install…",
                |_| in_terminal("sudo nyx-authctl helper install"),
            );
            ways_h.append(&r);
        }
        let surfaces: Vec<Value> = s["surfaces"].as_array().cloned().unwrap_or_default();
        let places = |m: &str| surfaces.iter().filter(|x| x["methods"].as_array().is_some_and(|ms| ms.iter().any(|v| v == m))).count();
        face_row(&ways_h, &s["face"], places("face"));
        fingerprint_row(&ways_h, &s["fingerprint"], places("fingerprint"));
        keys_row(&ways_h, &s["keys"]);
        let r = widgets::icon_row("dialog-password-symbolic", "Password", "Always works, everywhere", None);
        pill_row(&r, "On", "ok");
        widgets::keywords("password");
        ways_h.append(&r);

        let n_ssh = s["ssh_keys"].as_array().map_or(0, Vec::len);
        ssh_count.set_text(&n_ssh.to_string());
        ssh_count.set_visible(n_ssh > 0);
        let checks = s["findings"].as_array().into_iter().flatten().filter(|f| matches!(f["status"].as_str(), Some("warn" | "missing" | "error"))).count();
        hw_value.set_text(&if checks == 0 { "All good".to_string() } else { format!("{checks} to check") });
        hw_value.remove_css_class(if checks == 0 { "sgn-check" } else { "sgn-good" });
        hw_value.add_css_class(if checks == 0 { "sgn-good" } else { "sgn-check" });
        hw_value.set_visible(s["findings"].as_array().is_some_and(|f| !f.is_empty()));

        unlock_table(&table, s);
        apply_banners(&banners, s);
        if s["autologin"] == true {
            foot.set_text("Login: autologin is on, so this screen only appears after you log out.");
        }
    });
}

/// The table: a column per way, a row per place, a tick where that way works there.
fn unlock_table(table: &gtk::Box, status: &Value) {
    const CELL: i32 = 76;
    let head = widgets::hbox(0);
    head.add_css_class("sgn-head");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    head.append(&spacer);
    let columns: Vec<(&str, &str, &str)> =
        METHODS.iter().copied().chain(std::iter::once(("password", "Password", "dialog-password-symbolic"))).collect();
    for (_, name, icon) in &columns {
        let c = widgets::vbox(4);
        c.set_size_request(CELL, -1);
        c.set_hexpand(false);
        let i = gtk::Image::from_icon_name(icon);
        i.add_css_class("sgn-col-icon");
        c.append(&i);
        let l = gtk::Label::new(Some(name));
        l.add_css_class("sgn-col-name");
        c.append(&l);
        head.append(&c);
    }
    table.append(&head);

    let autologin = status["autologin"] == true;
    for s in status["surfaces"].as_array().into_iter().flatten() {
        let key = s["key"].as_str().unwrap_or_default().to_string();
        let methods: Vec<String> =
            s["methods"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(String::from)).collect();
        let mut order = methods.clone();
        if !order.iter().any(|m| m == "password") {
            order.push("password".into());
        }
        let names: Vec<String> = order
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let name = METHODS.iter().find(|(k, _, _)| k == m).map(|(_, n, _)| *n).unwrap_or("Password");
                if i == 0 { name.to_string() } else { name.to_lowercase() }
            })
            .collect();
        let mut desc: Vec<String> = vec![names.join(", then ")];
        let system = s["system"] == true;
        if let Some(err) = s["in_effect"]["error"].as_str() {
            desc.push(err.to_string());
        }
        if s["check"]["ok"] == false {
            desc.extend(s["check"]["errors"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from));
        }

        let row = widgets::hbox(0);
        row.add_css_class("sgn-row");
        let lead = widgets::hbox(12);
        lead.set_hexpand(true);
        lead.append(&widgets::tile(surface_icon(&key)));
        let text = widgets::vbox(2);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        let line = widgets::hbox(8);
        let title = widgets::label(s["label"].as_str().unwrap_or_default(), "sgn-place");
        title.set_wrap(true);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        line.append(&title);
        if system && s["in_effect"] == "no" {
            line.append(&pill("Not applied", "warn"));
        } else if !system {
            line.append(&pill("App", "accent"));
        }
        text.append(&line);
        let d = widgets::label(&desc.join(". "), "sgn-order");
        d.set_ellipsize(gtk::pango::EllipsizeMode::End);
        d.set_tooltip_text(Some(&desc.join(". ")));
        text.append(&d);
        if !system {
            text.set_tooltip_text(Some("Nyx Passwords and other apps will use this as they add support."));
        } else if key == "login" && autologin {
            text.set_tooltip_text(Some("Autologin is on, so this screen only appears after you log out."));
        }
        lead.append(&text);
        row.append(&lead);

        for (m, name, _) in &columns {
            let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            cell.set_size_request(CELL, -1);
            cell.set_hexpand(false);
            if *m == "password" {
                let lock = widgets::hbox(0);
                lock.add_css_class("sgn-always");
                lock.set_halign(gtk::Align::Center);
                lock.set_valign(gtk::Align::Center);
                lock.set_hexpand(true);
                let icon = gtk::Image::from_icon_name("system-lock-screen-symbolic");
                icon.set_hexpand(true);
                icon.set_halign(gtk::Align::Center);
                lock.append(&icon);
                lock.set_tooltip_text(Some("Your password always works too"));
                cell.append(&lock);
            } else {
                let check = gtk::CheckButton::new();
                check.set_halign(gtk::Align::Center);
                check.set_valign(gtk::Align::Center);
                check.set_hexpand(true);
                check.set_active(methods.iter().any(|x| x == m));
                if have(status, m) {
                    check.set_tooltip_text(Some(&format!("{name} for {}", s["label"].as_str().unwrap_or_default())));
                    let (key, m) = (key.clone(), m.to_string());
                    check.connect_toggled(move |c| {
                        let on = if c.is_active() { "on" } else { "off" };
                        ctl(vec!["policy".into(), "set".into(), key.clone(), m.clone(), on.into()], "");
                    });
                } else {
                    check.set_sensitive(false);
                    check.set_tooltip_text(Some(&format!("Set up {} first", name.to_lowercase())));
                }
                cell.append(&check);
            }
            row.append(&cell);
        }
        table.append(&row);
    }
}

/// One banner per place whose new settings aren't in use yet, with its Apply button.
fn apply_banners(banners: &gtk::Box, status: &Value) {
    clear(banners);
    for s in status["surfaces"].as_array().into_iter().flatten() {
        if s["system"] != true || s["in_effect"] != "no" {
            continue;
        }
        let key = s["key"].as_str().unwrap_or_default().to_string();
        let label = gtk::glib::markup_escape_text(s["label"].as_str().unwrap_or_default());
        let b = widgets::banner(
            &format!(
                "<b>{label} is ready to apply</b>\nA terminal shows exactly what changes and tests it before keeping it."
            ),
            true,
        );
        b.add_css_class("sgn-apply");
        let apply = gtk::Button::with_label("Apply…");
        apply.set_valign(gtk::Align::Center);
        if !(key == "login" && status["autologin"] == true) {
            apply.add_css_class("suggested-action");
        }
        apply.connect_clicked(move |_| in_terminal(&format!("nyx-authctl pam apply {key}")));
        b.append(&apply);
        banners.append(&b);
    }
}

fn recovery(page: &Page) {
    let g = page.group("If something goes wrong");
    let (r, _) = widgets::button_row(
        "Undo the last change",
        "Put the sign-in files back the way they were before the last change.",
        "Undo…",
        |_| in_terminal("nyx-authctl pam restore"),
    );
    widgets::keywords("undo restore rollback pam recovery");
    g.add(&r);
    g.note(
        "If sudo and administrator prompts both stop working, press Ctrl+Alt+F3, sign in with your password, \
         and run <tt>/usr/local/lib/nyx-biometrics/nyx-auth-helper pam-rollback ID</tt> with the newest ID from \
         <tt>/var/lib/nyx-biometrics/pam-backups</tt>.",
    );
}

fn face_row(h: &gtk::Box, face: &Value, places: usize) {
    let controls = widgets::hbox(8);
    let (desc, on, reach) = if face.is_null() {
        ("No camera found".to_string(), false, String::new())
    } else if face["ready"] == true {
        let camera = face["camera"].as_str().unwrap_or("camera");
        let reach = if face["level"] == "one-factor" {
            "Can sign in to sudo, administrator prompts and the lock screen."
        } else {
            "A colour camera only shows someone is there, so it only unlocks the lock screen."
        };
        let b = gtk::Button::with_label("Manage…");
        b.connect_clicked(|_| in_terminal("nyx-authctl face manage"));
        controls.append(&b);
        let backend = face["backend"].as_str().unwrap_or("");
        let mut parts = vec![format!("{backend} · {camera} camera").trim_start_matches(" · ").to_string()];
        if places > 0 {
            parts.push(format!("{places} place{}", if places == 1 { "" } else { "s" }));
        }
        (parts.join(" · "), true, reach.to_string())
    } else if face["installed"] == true {
        let b = gtk::Button::with_label("Set up…");
        b.add_css_class("suggested-action");
        b.connect_clicked(|_| in_terminal("nyx-authctl face enroll"));
        controls.append(&b);
        (format!("{} is installed. Look at the camera to enroll your face.", face["backend"].as_str().unwrap_or("")), false, String::new())
    } else {
        let backend = face["backend"].as_str().unwrap_or("Irlume").to_lowercase();
        let b = gtk::Button::with_label("Install…");
        b.connect_clicked(move |_| in_terminal(&format!("yay -S {backend}")));
        controls.append(&b);
        (face["reason"].as_str().unwrap_or("").to_string(), false, String::new())
    };
    let r = widgets::icon_row("camera-web-symbolic", "Face", &desc, Some(controls.upcast_ref()));
    if !reach.is_empty() {
        r.set_tooltip_text(Some(&reach));
    }
    if on {
        pill_row(&r, "On", "ok");
    }
    widgets::keywords("face recognition irlume gaze camera infrared windows hello");
    h.append(&r);
}

fn fingerprint_row(h: &gtk::Box, fp: &Value, places: usize) {
    let controls = widgets::hbox(8);
    let n = fingers(fp);
    let desc = match fp["state"].as_str() {
        Some("ready") => {
            let readers = fp["readers"].as_array().cloned().unwrap_or_default();
            let name = readers.first().and_then(|r| r["name"].as_str()).unwrap_or("Fingerprint reader").to_string();
            let b = gtk::Button::with_label(if n == 0 { "Set up…" } else { "Add a finger…" });
            b.connect_clicked(|_| in_terminal("nyx-authctl finger enroll"));
            controls.append(&b);
            if n == 0 {
                format!("{name} · no fingers yet")
            } else {
                let mut d = format!("{name} · {n} finger{}", if n == 1 { "" } else { "s" });
                if places > 0 {
                    d.push_str(&format!(" · {places} place{}", if places == 1 { "" } else { "s" }));
                }
                d
            }
        }
        Some("not-installed") => {
            let b = gtk::Button::with_label("Install…");
            b.connect_clicked(|_| in_terminal("sudo pacman -S --needed fprintd libfprint"));
            controls.append(&b);
            "A reader is connected, but fprintd isn't installed".to_string()
        }
        Some("error") => format!("The fingerprint service didn't answer: {}", fp["message"].as_str().unwrap_or("")),
        _ => "No reader found. A supported USB reader works too".to_string(),
    };
    let r = widgets::icon_row("auth-fingerprint-symbolic", "Fingerprint", &desc, Some(controls.upcast_ref()));
    // No reader and nothing set up: nothing to do here until one is plugged in.
    crate::capability::rebuild_on(&["hw:fingerprint"], "sign-in");
    if n == 0 && !matches!(fp["state"].as_str(), Some("ready" | "not-installed" | "error")) {
        crate::capability::bind(&r, &["hw:fingerprint"]);
    }
    if n > 0 {
        pill_row(&r, "On", "ok");
    }
    widgets::keywords("fingerprint fprintd reader finger biometric");
    h.append(&r);
}

/// Names are passed to a shell, so only plain characters are allowed.
fn safe_name(name: &str) -> Option<String> {
    let name = name.trim();
    let ok = !name.is_empty()
        && name.chars().count() <= 48
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'));
    ok.then(|| name.to_string())
}

/// The security keys, as one row that opens their own page.
fn keys_row(h: &gtk::Box, keys: &Value) {
    let keys = keys.as_array().cloned().unwrap_or_default();
    let names: Vec<String> = keys
        .iter()
        .map(|k| {
            let name = k["name"].as_str().unwrap_or("Security key");
            if k["role"] == "backup" { format!("{name} as backup") } else { name.to_string() }
        })
        .collect();
    let desc = match names.len() {
        0 => "Register a YubiKey or another FIDO2 key".to_string(),
        1 => format!("{}. Add a backup too", names[0]),
        _ => names.join(", "),
    };
    let value = if keys.is_empty() { String::new() } else { keys.len().to_string() };
    let (r, _) = widgets::nav_row(Some("media-flash-symbolic"), "Security keys", &desc, &value, move || {
        let keys = keys.clone();
        widgets::subpage("Security keys", 600, move |p| keys_page(p, &keys));
    });
    widgets::keywords("security key yubikey fido2 u2f add register backup remove");
    h.append(&r);
}

fn keys_page(page: &Page, keys: &[Value]) {
    let g = page.group("Security keys");
    for k in keys {
        let id = k["id"].as_str().unwrap_or_default().to_string();
        let role = if k["role"] == "backup" { "Backup" } else { "Primary" };
        let pin = if k["pin"] == true { "asks for its PIN" } else { "touch only" };
        let remove = widgets::confirm_button("Remove", "Remove it", move |_| {
            ctl(vec!["keys".into(), "remove".into(), id.clone()], "Security key removed");
        });
        let r = widgets::icon_row(
            "media-flash-symbolic",
            k["name"].as_str().unwrap_or("Security key"),
            &format!("{role} security key, {pin}."),
            Some(remove.upcast_ref()),
        );
        widgets::keywords("security key yubikey fido2 remove");
        g.add(&r);
    }
    let desc = match keys.len() {
        0 => "Register a YubiKey or another FIDO2 key, then register a backup.".to_string(),
        1 => "Register a backup key too, so losing one doesn't lock you out of anything that needs a key.".to_string(),
        n => format!("{n} keys registered."),
    };
    let ask_pin = Rc::new(Cell::new(false));
    let (r, _) = widgets::switch_row(
        "Ask for its PIN",
        "Each use needs the key's PIN as well as a touch. Off: a touch is enough.",
        false,
        {
            let ask_pin = ask_pin.clone();
            move |on| ask_pin.set(on)
        },
    );
    widgets::keywords("security key yubikey fido2 pin touch");
    g.add(&r);
    let (r, _) = widgets::entry_row("Add a security key", &desc, "", "name, such as Primary YubiKey", move |name| match safe_name(&name) {
        Some(n) => in_terminal(&format!("nyx-authctl keys add '{n}'{}", if ask_pin.get() { " --pin" } else { "" })),
        None => window::toast("Use letters, numbers, spaces, dashes or dots in the name"),
    });
    widgets::keywords("security key yubikey fido2 u2f add register backup pin");
    g.add(&r);
}

/// The plain SSH key names `nyx-authctl ssh-key create` accepts.
fn ssh_name(name: &str) -> Option<String> {
    let name = name.trim();
    let ok = !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| name.to_string())
}

fn ssh_rows(h: &gtk::Box, keys: &Value) {
    clear(h);
    for k in keys.as_array().into_iter().flatten() {
        let r = widgets::row(
            k["comment"].as_str().filter(|c| !c.is_empty()).unwrap_or_else(|| k["file"].as_str().unwrap_or("SSH key")),
            &format!("~/.ssh/{}.pub · {}", k["file"].as_str().unwrap_or_default(), k["kind"].as_str().unwrap_or_default()),
            None,
        );
        widgets::keywords("ssh key security key fido2");
        h.append(&r);
    }
    let (r, _) = widgets::entry_row(
        "New SSH key",
        "Made on the plugged-in security key and kept there, so another computer can fetch it. Each use asks for the key's PIN.",
        "",
        "name, such as github",
        |name| match ssh_name(&name) {
            Some(n) => in_terminal(&format!("nyx-authctl ssh-key create {n} --pin")),
            None => window::toast("Use letters, digits, dashes or underscores in the name"),
        },
    );
    widgets::keywords("ssh key security key fido2 new create github");
    h.append(&r);
    let (r, _) = widgets::button_row(
        "Bring keys to this computer",
        "Fetch the SSH keys kept on a security key into ~/.ssh.",
        "Load…",
        |_| in_terminal("nyx-authctl ssh-key load"),
    );
    widgets::keywords("ssh key security key load resident");
    h.append(&r);
}

#[cfg(test)]
mod tests {
    use super::safe_name;

    #[test]
    fn ssh_names_are_shell_safe() {
        assert_eq!(super::ssh_name(" github "), Some("github".into()));
        assert!(super::ssh_name("a b").is_none());
        assert!(super::ssh_name("x'; rm -rf ~").is_none());
        assert!(super::ssh_name("").is_none());
    }

    #[test]
    fn key_names_are_shell_safe() {
        assert_eq!(safe_name(" Primary YubiKey "), Some("Primary YubiKey".into()));
        assert_eq!(safe_name("Key 2.0-b"), Some("Key 2.0-b".into()));
        assert_eq!(safe_name("x'; rm -rf ~"), None);
        assert_eq!(safe_name(""), None);
        assert_eq!(safe_name(&"a".repeat(49)), None);
    }
}

fn overlay_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("nyx/auth/overlay.json")
}

fn overlay() -> Value {
    std::fs::read_to_string(overlay_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

/// Change one setting for one surface, keeping everything else in the file.
fn set_overlay(surface: &str, key: &str, value: Value) {
    let mut all = overlay();
    if !all.is_object() {
        all = serde_json::json!({});
    }
    let entry = all.as_object_mut().expect("object").entry(surface).or_insert_with(|| serde_json::json!({}));
    if let Some(o) = entry.as_object_mut() {
        o.insert(key.into(), value);
    }
    let path = overlay_path();
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(path.with_extension("json.new"), serde_json::to_string_pretty(&all).unwrap_or_default()))
        .and_then(|_| std::fs::rename(path.with_extension("json.new"), &path));
    if let Err(e) = written {
        window::toast(&format!("Couldn't save: {e}"));
    }
}

fn style_options(with_minimal: bool) -> Vec<(String, String)> {
    STYLES.iter().filter(|(k, _)| with_minimal || *k != "minimal").map(|(k, n)| (k.to_string(), n.to_string())).collect()
}

/// How scanning looks on the lock screen. The lock screen reads the file as it changes.
fn look_group(page: &Page) {
    let g = page.group("Look");
    g.note("How a face scan looks. It only appears while a scan runs.");
    let current = overlay();
    let lock = &current["lock-screen"];
    let style = lock["style"].as_str().unwrap_or("halo").to_string();
    let (r, _) = widgets::choice_row("Lock screen style", "", style_options(true), &style, |id| {
        set_overlay("lock-screen", "style", Value::from(id));
        window::rebuild_if_built(ID);
    });
    widgets::keywords("overlay style circular cyberpunk minimal edge glow animation lock screen look");
    g.add(&r);

    let polkit_style = current["polkit"]["style"].as_str().unwrap_or("halo").to_string();
    let (r, _) = widgets::choice_row(
        "Administrator prompt style",
        "With the Nyx administrator prompt in use.",
        style_options(false),
        &polkit_style,
        |id| set_overlay("polkit", "style", Value::from(id)),
    );
    widgets::keywords("overlay style polkit administrator prompt");
    g.add(&r);

    let sudo_style = current["sudo"]["style"].as_str().unwrap_or("halo").to_string();
    let (r, _) = widgets::choice_row(
        "Terminal (sudo) style",
        "Shown on screen while sudo checks your face in a terminal.",
        style_options(true),
        &sudo_style,
        |id| set_overlay("sudo", "style", Value::from(id)),
    );
    widgets::keywords("overlay style sudo terminal");
    g.add(&r);

    let strength = match lock["intensity"].as_f64() {
        Some(v) if v < 0.55 => "subtle",
        Some(v) if v > 0.85 => "strong",
        _ => "normal",
    };
    let levels = vec![
        ("subtle".to_string(), "Subtle".to_string()),
        ("normal".to_string(), "Normal".to_string()),
        ("strong".to_string(), "Strong".to_string()),
    ];
    let (r, _) = widgets::choice_row("Glow", "How bright the scan and edge glow are.", levels, strength, |id| {
        let v = match id.as_str() {
            "subtle" => 0.4,
            "strong" => 1.0,
            _ => 0.7,
        };
        set_overlay("lock-screen", "intensity", Value::from(v));
        set_overlay("polkit", "intensity", Value::from(v));
        set_overlay("sudo", "intensity", Value::from(v));
    });
    widgets::keywords("overlay intensity glow brightness");
    g.add(&r);

    let (r, _) = widgets::switch_row(
        "Animations",
        "Sweeps, scan lines and pulses while scanning.",
        lock["animate"].as_bool().unwrap_or(true),
        |on| {
            set_overlay("lock-screen", "animate", Value::from(on));
            set_overlay("polkit", "animate", Value::from(on));
            set_overlay("sudo", "animate", Value::from(on));
        },
    );
    widgets::keywords("overlay animation motion");
    g.add(&r);

    // The login screen is a system file, so the switch goes through the helper (an
    // administrator prompt) and shows what SDDM will actually use.
    if std::path::Path::new(LOGIN_THEME_DIR).is_dir() {
        let (r, _) = widgets::switch_row(
            "Login screen with face",
            "After logging out: Omarchy's login screen with Halo. Leave the password empty and press Enter to use your face. Off: Omarchy's own.",
            login_theme_on(),
            |on| {
                ctl(
                    vec!["login-theme".into(), if on { "on" } else { "off" }.into()],
                    if on { "The login screen shows Halo now." } else { "Omarchy's login screen is back." },
                )
            },
        );
        widgets::keywords("login screen sddm greeter theme face halo");
        g.add(&r);
    }
}

const LOGIN_THEME_DIR: &str = "/usr/share/sddm/themes/nyx-biometrics";
const LOGIN_THEME_CONF: &str = "/etc/sddm.conf.d/zz-nyx-biometrics.conf";

fn login_theme_on() -> bool {
    std::fs::read_to_string(LOGIN_THEME_CONF).is_ok_and(|t| t.lines().any(|l| l.trim() == "Current=nyx-biometrics"))
}

/// Whether the login screen takes your face (for Boot & Login's picture of it).
pub fn login_face() -> bool {
    std::path::Path::new(LOGIN_THEME_DIR).is_dir() && login_theme_on()
}
