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
        let fingers: usize = v["fingerprint"]["readers"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| r["fingers"].as_array().map_or(0, Vec::len))
            .sum();
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
    let ssh = page.group("SSH keys on a security key");
    ssh.note(
        "The private key stays on the security key; each use needs a touch. Keys kept in the vault are Nyx Passwords' to serve.",
    );
    let ssh_h = holder(&ssh, "");
    let hw = page.collapsible("This computer", false);
    let hw_h = holder(&hw, "");

    look_group(page);

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
        ssh_rows(&ssh_h, &status["ssh_keys"]);

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

/// How scanning looks on the lock screen. The lock screen reads the file as it changes.
fn look_group(page: &Page) {
    let g = page.group("Look");
    g.note("How a face scan looks. It only appears while a scan runs.");
    let current = overlay();
    let lock = &current["lock-screen"];
    let style = lock["style"].as_str().unwrap_or("halo").to_string();
    let options = vec![
        ("halo".to_string(), "Halo".to_string()),
        ("circular".to_string(), "Circular face scan".to_string()),
        ("cyberpunk".to_string(), "Cyberpunk scanner".to_string()),
        ("minimal".to_string(), "Minimal".to_string()),
        ("edge".to_string(), "Edge glow".to_string()),
    ];
    let (r, _) = widgets::choice_row("Lock screen style", "", options, &style, |id| {
        set_overlay("lock-screen", "style", Value::from(id));
    });
    widgets::keywords("overlay style circular cyberpunk minimal edge glow animation lock screen look");
    g.add(&r);

    let polkit_style = current["polkit"]["style"].as_str().unwrap_or("halo").to_string();
    let options = vec![
        ("halo".to_string(), "Halo".to_string()),
        ("circular".to_string(), "Circular face scan".to_string()),
        ("cyberpunk".to_string(), "Cyberpunk scanner".to_string()),
        ("edge".to_string(), "Edge glow".to_string()),
    ];
    let (r, _) = widgets::choice_row(
        "Administrator prompt style",
        "With the Nyx administrator prompt in use.",
        options,
        &polkit_style,
        |id| set_overlay("polkit", "style", Value::from(id)),
    );
    widgets::keywords("overlay style polkit administrator prompt");
    g.add(&r);

    let sudo_style = current["sudo"]["style"].as_str().unwrap_or("halo").to_string();
    let options = vec![
        ("halo".to_string(), "Halo".to_string()),
        ("circular".to_string(), "Circular face scan".to_string()),
        ("cyberpunk".to_string(), "Cyberpunk scanner".to_string()),
        ("minimal".to_string(), "Minimal".to_string()),
        ("edge".to_string(), "Edge glow".to_string()),
    ];
    let (r, _) = widgets::choice_row(
        "Terminal (sudo) style",
        "Shown on screen while sudo checks your face in a terminal.",
        options,
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
