//! Sign-in & Security: face, fingerprint and security keys from Nyx Biometrics, and where
//! each one signs you in. Everything goes through `nyx-authctl`: it reports the state as
//! JSON, and its commands do the enrolling and the PAM changes, asking for approval in
//! their own prompts. Nothing here edits system files.

use gtk::prelude::*;
use serde_json::Value;

use super::accounts::terminal;
use crate::widgets::{self, Group, Page};
use crate::{cmd, window};

const CTL: &str = "nyx-authctl";
const ID: &str = "sign-in";

pub fn available() -> bool {
    cmd::present(CTL)
}

fn holder(group: &Group, placeholder: &str) -> gtk::Box {
    let h = widgets::vbox(0);
    if !placeholder.is_empty() {
        h.append(&widgets::label(placeholder, "dim"));
    }
    group.add(&h);
    h
}

fn clear(h: &gtk::Box) {
    widgets::forget_rows(h);
    while let Some(c) = h.first_child() {
        h.remove(&c);
    }
}

fn ok_tag(row: &gtk::Box, text: &str, ok: bool) {
    widgets::tag_row(row, text);
    if let Some(tag) = row.first_child().and_then(|t| t.first_child()).and_then(|l| l.last_child()) {
        tag.add_css_class(if ok { "ok-tag" } else { "warn-tag" });
    }
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

    let ways = page.group("Ways to sign in");
    ways.note("Your password always works too, everywhere.");
    let ways_h = holder(&ways, "Checking this computer…");
    let uses = page.group("Where they're used");
    uses.note("Changes to sign-in open a terminal that shows exactly what changes and tests it before keeping it.");
    let uses_h = holder(&uses, "");
    let hw = page.collapsible("This computer", false);
    let hw_h = holder(&hw, "");

    let undo = page.group("If something goes wrong");
    let (r, _) = widgets::button_row(
        "Undo the last change",
        "Put the sign-in files back the way they were before the last change.",
        "Undo…",
        |_| in_terminal("nyx-authctl pam restore"),
    );
    widgets::keywords("undo restore rollback pam recovery");
    undo.add(&r);
    undo.note(
        "If sudo and administrator prompts both stop working, press Ctrl+Alt+F3, sign in with your password, \
         and run <tt>/usr/local/lib/nyx-biometrics/nyx-auth-helper pam-rollback ID</tt> with the newest ID from \
         <tt>/var/lib/nyx-biometrics/pam-backups</tt>.",
    );

    cmd::run_async(&[CTL, "status", "--json"], move |result| {
        widgets::begin_section(ID);
        clear(&ways_h);
        let status: Value = match result.map(|t| serde_json::from_str(&t)) {
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
        if status["helper"] != "ready" {
            let (r, _) = widgets::button_row(
                "Finish installing",
                "Nyx Biometrics needs its small administrator helper to make sign-in changes.",
                "Install…",
                |_| in_terminal("sudo nyx-authctl helper install"),
            );
            ways_h.append(&r);
        }
        face_row(&ways_h, &status["face"]);
        fingerprint_row(&ways_h, &status["fingerprint"]);
        keys_rows(&ways_h, &status["keys"]);
        let r = widgets::row("Password", "Always works, everywhere.", None);
        ok_tag(&r, "On", true);
        widgets::keywords("password");
        ways_h.append(&r);

        surface_rows(&uses_h, &status);

        for f in status["findings"].as_array().into_iter().flatten() {
            let msg = f["message"].as_str().unwrap_or_default();
            let r = widgets::row(msg, "", None);
            match f["status"].as_str() {
                Some("ok") => ok_tag(&r, "OK", true),
                Some("warn") | Some("missing") | Some("error") => ok_tag(&r, "Check", false),
                _ => {}
            }
            hw_h.append(&r);
        }
    });
}

fn face_row(h: &gtk::Box, face: &Value) {
    let controls = widgets::hbox(8);
    let (title, desc, on) = if face.is_null() {
        ("Face", "No camera found.".to_string(), false)
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
        ("Face", format!("{} on the {camera} camera. {reach}", face["backend"].as_str().unwrap_or("")), true)
    } else if face["installed"] == true {
        let b = gtk::Button::with_label("Set up…");
        b.add_css_class("suggested-action");
        b.connect_clicked(|_| in_terminal("nyx-authctl face enroll"));
        controls.append(&b);
        (
            "Face",
            format!("{} is installed. Look at the camera to enroll your face.", face["backend"].as_str().unwrap_or("")),
            false,
        )
    } else {
        let backend = face["backend"].as_str().unwrap_or("Irlume").to_lowercase();
        let b = gtk::Button::with_label("Install…");
        b.connect_clicked(move |_| in_terminal(&format!("yay -S {backend}")));
        controls.append(&b);
        ("Face", face["reason"].as_str().unwrap_or("").to_string(), false)
    };
    let r = widgets::row(title, &desc, Some(controls.upcast_ref()));
    if on {
        ok_tag(&r, "On", true);
    }
    widgets::keywords("face recognition irlume gaze camera infrared windows hello");
    h.append(&r);
}

fn fingerprint_row(h: &gtk::Box, fp: &Value) {
    let controls = widgets::hbox(8);
    let mut on = false;
    let desc = match fp["state"].as_str() {
        Some("ready") => {
            let readers = fp["readers"].as_array().cloned().unwrap_or_default();
            let fingers: usize = readers.iter().map(|r| r["fingers"].as_array().map_or(0, Vec::len)).sum();
            let name = readers.first().and_then(|r| r["name"].as_str()).unwrap_or("Fingerprint reader").to_string();
            let b = gtk::Button::with_label(if fingers == 0 { "Set up…" } else { "Add a finger…" });
            b.connect_clicked(|_| in_terminal("nyx-authctl finger enroll"));
            controls.append(&b);
            on = fingers > 0;
            if fingers == 0 {
                format!("{name}. No fingers enrolled yet.")
            } else {
                format!("{name}, {fingers} finger(s) enrolled.")
            }
        }
        Some("not-installed") => {
            let b = gtk::Button::with_label("Install…");
            b.connect_clicked(|_| in_terminal("sudo pacman -S --needed fprintd libfprint"));
            controls.append(&b);
            "A reader is connected, but fprintd isn't installed.".to_string()
        }
        Some("error") => format!("The fingerprint service didn't answer: {}", fp["message"].as_str().unwrap_or("")),
        _ => "No fingerprint reader found. A supported USB reader works too.".to_string(),
    };
    let r = widgets::row("Fingerprint", &desc, Some(controls.upcast_ref()));
    if on {
        ok_tag(&r, "On", true);
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

fn keys_rows(h: &gtk::Box, keys: &Value) {
    let keys = keys.as_array().cloned().unwrap_or_default();
    for k in &keys {
        let id = k["id"].as_str().unwrap_or_default().to_string();
        let role = if k["role"] == "backup" { "Backup" } else { "Primary" };
        let pin = if k["pin"] == true { "asks for its PIN" } else { "touch only" };
        let remove = widgets::confirm_button("Remove", "Remove it", move |_| {
            ctl(vec!["keys".into(), "remove".into(), id.clone()], "Security key removed");
        });
        let r = widgets::row(
            k["name"].as_str().unwrap_or("Security key"),
            &format!("{role} security key, {pin}."),
            Some(remove.upcast_ref()),
        );
        widgets::keywords("security key yubikey fido2 remove");
        h.append(&r);
    }
    let desc = match keys.len() {
        0 => "Register a YubiKey or another FIDO2 key, then register a backup.".to_string(),
        1 => "Register a backup key too, so losing one doesn't lock you out of anything that needs a key.".to_string(),
        n => format!("{n} keys registered."),
    };
    let (r, _) =
        widgets::entry_row("Add a security key", &desc, "", "name, such as Primary YubiKey", |name| match safe_name(&name) {
            Some(n) => in_terminal(&format!("nyx-authctl keys add '{n}' --pin")),
            None => window::toast("Use letters, numbers, spaces, dashes or dots in the name"),
        });
    widgets::keywords("security key yubikey fido2 u2f add register backup");
    h.append(&r);
}

const METHODS: [(&str, &str); 3] = [("face", "Face"), ("fingerprint", "Fingerprint"), ("security-key", "Security key")];

fn surface_rows(h: &gtk::Box, status: &Value) {
    clear(h);
    let have = |m: &str| match m {
        "face" => status["face"]["ready"] == true,
        "fingerprint" => status["fingerprint"]["readers"]
            .as_array()
            .is_some_and(|rs| rs.iter().any(|r| r["fingers"].as_array().is_some_and(|f| !f.is_empty()))),
        _ => status["keys"].as_array().is_some_and(|k| !k.is_empty()),
    };
    for s in status["surfaces"].as_array().into_iter().flatten() {
        let key = s["key"].as_str().unwrap_or_default().to_string();
        let methods: Vec<String> =
            s["methods"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(String::from)).collect();
        let names: Vec<String> = methods
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let name = METHODS.iter().find(|(k, _)| k == m).map(|(_, n)| *n).unwrap_or("Password");
                if i == 0 { name.to_string() } else { name.to_lowercase() }
            })
            .collect();
        let mut desc = names.join(", then ");
        let system = s["system"] == true;
        if !system {
            desc.push_str(". Nyx Passwords and other apps will use this as they add support.");
        } else if s["in_effect"] == "no" {
            desc.push_str(". Not applied yet.");
        } else if let Some(err) = s["in_effect"]["error"].as_str() {
            desc = format!("{desc}. {err}");
        }
        if key == "login" && status["autologin"] == true {
            desc.push_str(" Autologin is on, so this screen only appears after you log out.");
        }
        if s["check"]["ok"] == false {
            let errs: Vec<&str> = s["check"]["errors"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            desc = format!("{desc} {}", errs.join(" "));
        }

        let controls = widgets::hbox(6);
        for (m, label) in METHODS {
            if !have(m) {
                continue;
            }
            let pair = widgets::hbox(6);
            pair.append(&widgets::label(label, "dim"));
            let sw = gtk::Switch::new();
            sw.set_valign(gtk::Align::Center);
            sw.set_active(methods.iter().any(|x| x == m));
            let key = key.clone();
            sw.connect_active_notify(move |sw| {
                let on = if sw.is_active() { "on" } else { "off" };
                ctl(vec!["policy".into(), "set".into(), key.clone(), m.into(), on.into()], "");
            });
            pair.append(&sw);
            controls.append(&pair);
        }
        if system && s["in_effect"] == "no" {
            let apply = gtk::Button::with_label("Apply…");
            if !(key == "login" && status["autologin"] == true) {
                apply.add_css_class("suggested-action");
            }
            let key = key.clone();
            apply.connect_clicked(move |_| in_terminal(&format!("nyx-authctl pam apply {key}")));
            controls.append(&apply);
        }
        let r = widgets::row(s["label"].as_str().unwrap_or_default(), &desc, Some(controls.upcast_ref()));
        if system && s["in_effect"] == "yes" && methods.len() > 1 {
            ok_tag(&r, "In use", true);
        }
        widgets::keywords("sudo polkit administrator lock screen login passwords vault pam policy");
        h.append(&r);
    }
}

#[cfg(test)]
mod tests {
    use super::safe_name;

    #[test]
    fn key_names_are_shell_safe() {
        assert_eq!(safe_name(" Primary YubiKey "), Some("Primary YubiKey".into()));
        assert_eq!(safe_name("Key 2.0-b"), Some("Key 2.0-b".into()));
        assert_eq!(safe_name("x'; rm -rf ~"), None);
        assert_eq!(safe_name(""), None);
        assert_eq!(safe_name(&"a".repeat(49)), None);
    }
}
