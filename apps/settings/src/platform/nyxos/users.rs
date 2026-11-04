//! NyxOS's Users page: your name and password, the other people on this computer, and
//! adding someone. Accounts are part of the system configuration (the System Manager
//! declares them); passwords are set apart and never stored in it.

use super::tag_system;
use crate::backend::{accounts as acc, passwd};
use crate::dialog::{Field, ask};
use crate::platform::nyxos;
use crate::sections::accounts::password_problem;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use nyx_system_proto::{Change, valid_full_name};

/// Run `nyx-system ARGS` with `input` on its stdin (a password never goes on a command
/// line), then toast `done` or what went wrong and rebuild Users.
fn with_stdin(args: Vec<String>, input: String, working: &str, done: &'static str) {
    window::toast(working);
    cmd::background(
        move || -> Result<(), String> {
            use std::io::Write;
            let mut child = std::process::Command::new("nyx-system")
                .args(&args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(format!("{input}\n").as_bytes()).map_err(|e| e.to_string())?;
            }
            let out = child.wait_with_output().map_err(|e| e.to_string())?;
            if out.status.success() {
                return Ok(());
            }
            let err = String::from_utf8_lossy(&out.stderr);
            let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it didn't work");
            Err(last.trim().trim_start_matches("nyx-system: ").to_string())
        },
        move |r| {
            match r {
                Ok(()) => window::toast(done),
                Err(e) => window::toast(&e),
            }
            window::rebuild("accounts");
        },
    );
}

/// Set `user`'s password through the System Manager (yours asks for your own).
fn set_password(user: String, password: String) {
    with_stdin(vec!["password".into(), user], password, "Changing the password…", "Password changed");
}

fn password_dialog(user: String, mine: bool) {
    ask(
        if mine { "Change your password" } else { "Set a password" },
        if mine { "You're asked for the one you have now." } else { "An administrator's password is asked for." },
        vec![Field::secret("New password"), Field::secret("New password again")],
        "Change",
        None,
        move |v, _| {
            if let Some(p) = password_problem(&v[0], &v[1]) {
                return Some(p);
            }
            set_password(user.clone(), v[0].clone());
            None
        },
    );
}

fn add_dialog() {
    ask(
        "Add a person",
        "They get their own account and home folder. It's added to this computer's configuration, which is rebuilt.",
        vec![Field::text("Full name"), Field::text("Login name"), Field::secret("Password"), Field::secret("Password again")],
        "Add",
        Some(("Administrator (can make system changes)", false)),
        |v, admin| {
            let (full, name) = (v[0].trim().to_string(), v[1].trim().to_string());
            if !valid_full_name(&full) {
                return Some("A name is up to 100 characters, without : or commas.".into());
            }
            if !passwd::valid_name(&name) || nyx_system_proto::reserved_user(&name) {
                return Some("Use lower-case letters, digits, - and _, starting with a letter.".into());
            }
            if passwd::users().iter().any(|u| u.name == name) {
                return Some(format!("There is already an account called {name}."));
            }
            if let Some(p) = password_problem(&v[2], &v[3]) {
                return Some(p);
            }
            let mut args = vec!["person".to_string(), "add".into(), name, full];
            if admin {
                args.push("--admin".into());
            }
            args.push("--password-stdin".into());
            with_stdin(args, v[2].clone(), "Adding the account. The system is rebuilt; it takes a minute.", "Account added");
            None
        },
    );
}

pub fn build(page: &Page) {
    let s = nyxos::settings();
    let me = acc::me();
    let people = acc::people();

    let g = page.group("You");
    let full = people.iter().find(|u| u.name == me).map(|u| u.full_name.clone()).unwrap_or_default();
    let (who, now) = (me.clone(), full.clone());
    let (r, _) = widgets::entry_row("Full name", &format!("Signed in as {me}"), &full, "Your name", move |name| {
        let name = name.trim().to_string();
        if name == now {
            return;
        }
        if !valid_full_name(&name) {
            return window::toast("A name is up to 100 characters, without : or commas");
        }
        nyxos::apply(vec![Change::FullName { user: who.clone(), full_name: name }], "Name changed", |_| {
            window::rebuild("accounts")
        });
    });
    tag_system(&r);
    widgets::keywords("name full name me profile");
    g.add(&r);
    let mine = me.clone();
    let (r, _) = widgets::button_row("Password", "Used to sign in, unlock and approve system changes.", "Change…", move |_| {
        password_dialog(mine.clone(), true)
    });
    widgets::keywords("password passwd change");
    g.add(&r);

    let g = page.group("Other people");
    let others: Vec<_> = people.iter().filter(|u| u.name != me).collect();
    if others.is_empty() {
        g.note("Nobody else has an account on this computer.");
    }
    for u in others {
        let title = if u.full_name.is_empty() { u.name.clone() } else { u.full_name.clone() };
        match s.people.get(&u.name) {
            Some(p) => {
                let admin = p.admin;
                let name = u.name.clone();
                let switch = gtk::Switch::new();
                switch.set_active(admin);
                switch.set_valign(gtk::Align::Center);
                switch.set_tooltip_text(Some("Administrator: can make system changes"));
                switch.connect_active_notify(move |sw| {
                    if sw.is_active() != admin {
                        let change = Change::PersonAdmin { name: name.clone(), admin: sw.is_active() };
                        nyxos::apply(vec![change], "Saved", |_| window::rebuild("accounts"));
                    }
                });
                let (n1, n2) = (u.name.clone(), u.name.clone());
                let pw = gtk::Button::with_label("Password…");
                pw.connect_clicked(move |_| password_dialog(n1.clone(), false));
                let remove = widgets::confirm_button(
                    "Remove",
                    &format!("Remove {}'s account? Their home folder stays.", u.name),
                    move |b| {
                        b.set_sensitive(false);
                        nyxos::apply(vec![Change::RemovePerson { name: n2.clone() }], "Account removed", |_| {
                            window::rebuild("accounts")
                        });
                    },
                );
                let controls = widgets::hbox(8);
                controls.append(&widgets::label("Administrator", "dim"));
                controls.append(&switch);
                controls.append(&pw);
                controls.append(&remove);
                let r = widgets::row(&title, &u.name, Some(controls.upcast_ref()));
                tag_system(&r);
                g.add(&r);
            }
            None => {
                g.add(&widgets::row(&title, &format!("{} · Set in this computer's configuration", u.name), None));
            }
        }
    }
    let (r, _) = widgets::button_row("Add a person", "Someone else who uses this computer.", "Add…", |_| add_dialog());
    widgets::keywords("add user new account person family");
    g.add(&r);
}
