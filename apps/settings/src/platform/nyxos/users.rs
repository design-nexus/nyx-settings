//! NyxOS's Users page: you (your name and password), the other people on this computer
//! as a list, and the one picked beside it: what they can do, their password and home
//! folder, and removing them. Accounts are part of the system configuration (the System
//! Manager declares them); passwords are set apart and never stored in it.

use super::pending::Pending;
use crate::backend::{accounts as acc, passwd};
use crate::dialog::{Field, ask};
use crate::platform::nyxos::{self, mark_system};
use crate::sections::accounts::password_problem;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use nyx_system_proto::{Change, valid_full_name};
use std::cell::RefCell;
use std::rc::Rc;

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

// ---------- Faces ----------

/// Colours for people's initials, picked by their login so each keeps theirs.
const FACE_COLOURS: [(f64, f64, f64); 8] = [
    (0.93, 0.38, 0.36),
    (0.33, 0.78, 0.42),
    (0.55, 0.45, 0.93),
    (0.96, 0.52, 0.35),
    (0.25, 0.62, 0.93),
    (0.93, 0.70, 0.25),
    (0.20, 0.72, 0.70),
    (0.85, 0.40, 0.70),
];

/// "Sam Rivera" → "SR"; "ken" → "K".
fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let pick = match words.as_slice() {
        [] => String::new(),
        [one] => one.chars().take(1).collect(),
        [first, .., last] => first.chars().take(1).chain(last.chars().take(1)).collect(),
    };
    pick.to_uppercase()
}

/// A person's face: their picture where AccountsService has one, otherwise their
/// initials on a circle of their colour.
pub(super) fn avatar(full_name: &str, login: &str, size: i32) -> gtk::Widget {
    let picture = std::path::Path::new("/var/lib/AccountsService/icons").join(login);
    if picture.is_file() {
        let p = gtk::Picture::for_filename(&picture);
        p.set_content_fit(gtk::ContentFit::Cover);
        p.set_size_request(size, size);
        p.set_overflow(gtk::Overflow::Hidden);
        p.add_css_class("nxu-face");
        p.set_valign(gtk::Align::Center);
        return p.upcast();
    }
    let text = initials(if full_name.is_empty() { login } else { full_name });
    let colour = FACE_COLOURS[login.bytes().fold(7usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize)) % FACE_COLOURS.len()];
    let area = gtk::DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    area.set_valign(gtk::Align::Center);
    area.set_halign(gtk::Align::Start);
    area.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let r = w.min(h) / 2.0;
        cr.set_source_rgb(colour.0, colour.1, colour.2);
        cr.arc(w / 2.0, h / 2.0, r, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
        cr.set_font_size(r * 0.78);
        if let Ok(ext) = cr.text_extents(&text) {
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.move_to(w / 2.0 - ext.width() / 2.0 - ext.x_bearing(), h / 2.0 - ext.height() / 2.0 - ext.y_bearing());
            let _ = cr.show_text(&text);
        }
    });
    area.upcast()
}

// ---------- The page ----------

/// A person on this computer, as the page shows them.
#[derive(Clone)]
struct Person {
    login: String,
    full_name: String,
    home: String,
    admin: bool,
    /// Made by NyxOS (Settings can change them), not set in the configuration by hand.
    managed: bool,
}

impl Person {
    fn title(&self) -> String {
        if self.full_name.is_empty() { self.login.clone() } else { self.full_name.clone() }
    }

    /// "Alex" from "Alex Rivera".
    fn first(&self) -> String {
        self.title().split_whitespace().next().unwrap_or_default().to_string()
    }

    fn role(&self) -> &'static str {
        if self.admin { "Administrator" } else { "Standard" }
    }
}

/// The picked person, drawn in the preview.
struct Detail {
    pane: gtk::Box,
    pending: Rc<Pending>,
    host: String,
    /// The role each person is set to on this page (waiting to be applied, or as is).
    roles: RefCell<std::collections::HashMap<String, bool>>,
}

impl Detail {
    fn show(self: &Rc<Self>, p: &Person) {
        while let Some(c) = self.pane.first_child() {
            self.pane.remove(&c);
        }
        let admin = self.roles.borrow().get(&p.login).copied().unwrap_or(p.admin);
        let is_me = p.login == acc::me();
        let head = widgets::hbox(16);
        head.add_css_class("nxu-head");
        head.append(&avatar(&p.full_name, &p.login, 60));
        let text = widgets::vbox(2);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        let name = widgets::label(&p.title(), "nxu-name");
        name.set_wrap(true);
        text.append(&name);
        let what = match (is_me, p.managed) {
            (true, _) => format!("you · {}", p.role()),
            (false, true) => format!("{} account", p.role()),
            (false, false) => format!("{} · set in this computer's configuration", p.role()),
        };
        let line = widgets::label(&format!("{} · {what} · {}", p.login, p.home), "nxu-line");
        line.set_wrap(true);
        line.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        text.append(&line);
        head.append(&text);
        let back = gtk::Button::new();
        back.add_css_class("flat");
        back.add_css_class("nxu-back");
        back.set_valign(gtk::Align::Start);
        let b = widgets::hbox(4);
        b.append(&gtk::Image::from_icon_name("go-previous-symbolic"));
        b.append(&gtk::Label::new(Some("Accounts")));
        back.set_child(Some(&b));
        back.connect_clicked(|_| window::navigate("online-accounts"));
        head.append(&back);
        self.pane.append(&head);

        let first = p.first();
        self.pane.append(&widgets::label(&format!("What {first} can do").to_uppercase(), "nxu-lab"));
        let choices = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        choices.set_homogeneous(true);
        let host = &self.host;
        let standard = choice(
            "avatar-default-symbolic",
            "Standard",
            "Use the apps, install their own, and change their own settings.",
            None,
        );
        let administrator = choice(
            "security-high-symbolic",
            "Administrator",
            &format!("Also change {host} for everyone: updates, services, other people. Approves with their password."),
            Some(&standard.1),
        );
        standard.1.set_active(!admin);
        administrator.1.set_active(admin);
        for c in [&standard.0, &administrator.0] {
            choices.append(c);
        }
        let can_change = p.managed;
        choices.set_sensitive(can_change);
        if can_change {
            let (me, pending, login, was) = (self.clone(), self.pending.clone(), p.login.clone(), p.admin);
            administrator.1.connect_toggled(move |b| {
                let now = b.is_active();
                me.roles.borrow_mut().insert(login.clone(), now);
                let change = (now != was).then(|| Change::PersonAdmin { name: login.clone(), admin: now });
                let role = |a: bool| if a { "Administrator" } else { "Standard" };
                pending.set_shown(&format!("admin-{login}"), change, role(was), role(now));
            });
        }
        self.pane.append(&choices);
        for (b, c) in [(&standard.1, &standard.0), (&administrator.1, &administrator.0)] {
            let c = c.clone();
            let sync = move |b: &gtk::CheckButton| {
                if b.is_active() {
                    c.add_css_class("on");
                } else {
                    c.remove_css_class("on");
                }
            };
            sync(b);
            b.connect_toggled(sync);
        }

        let card = widgets::vbox(0);
        card.add_css_class("settings-card");
        if p.managed || is_me {
            let login = p.login.clone();
            let (r, _) = widgets::button_row(
                "Password",
                if is_me { "You're asked for the one you have now." } else { "An administrator's password is asked for." },
                if is_me { "Change…" } else { "Set a new one…" },
                move |_| password_dialog(login.clone(), is_me),
            );
            r.prepend(&widgets::tile("changes-prevent-symbolic"));
            r.add_css_class("with-tile");
            card.append(&r);
        }
        if is_me {
            let (r, _) = widgets::nav_row(
                Some("auth-fingerprint-symbolic"),
                "Face and fingerprint",
                "Set up in Sign-in & Security.",
                "",
                || window::navigate("sign-in"),
            );
            card.append(&r);
        } else {
            card.append(&widgets::icon_row(
                "auth-fingerprint-symbolic",
                "Face and fingerprint",
                &format!("{first} sets these up when signed in."),
                None,
            ));
        }
        card.append(&widgets::icon_row("folder-symbolic", "Home folder", &format!("{} · stays if the account is removed", p.home), None));
        widgets::mark_first_rows(card.upcast_ref());
        self.pane.append(&card);

        if p.managed && !is_me {
            let rm = widgets::hbox(12);
            rm.add_css_class("settings-card");
            rm.add_css_class("nxu-remove");
            let icon = gtk::Image::from_icon_name("user-trash-symbolic");
            icon.add_css_class("nxu-danger");
            rm.append(&icon);
            let t = widgets::vbox(2);
            t.set_hexpand(true);
            t.append(&widgets::label(&format!("Remove {first}'s account"), "settings-option-title"));
            let d = widgets::label(
                &format!("{first} can't sign in any more; the home folder stays on {host}."),
                "settings-option-description",
            );
            d.set_wrap(true);
            t.append(&d);
            rm.append(&t);
            let login = p.login.clone();
            let remove = widgets::confirm_button("Remove…", &format!("Remove {login}'s account? Their home folder stays."), move |b| {
                b.set_sensitive(false);
                nyxos::apply(vec![Change::RemovePerson { name: login.clone() }], "Account removed", |_| {
                    window::rebuild("accounts")
                });
            });
            remove.add_css_class("destructive-action");
            remove.set_valign(gtk::Align::Center);
            rm.append(&remove);
            self.pane.append(&rm);
        } else if !p.managed {
            let text = if is_me {
                format!("Your account is set in {host}'s configuration, so what you can do is changed there, not here.")
            } else {
                format!("{first}'s account is set in {host}'s configuration by hand, so it's changed there, not here.")
            };
            let note = widgets::label(&text, "nxu-note");
            note.set_wrap(true);
            self.pane.append(&note);
        }
    }
}

/// One of the two roles, as a card with a radio button.
fn choice(icon: &str, title: &str, desc: &str, group: Option<&gtk::CheckButton>) -> (gtk::Box, gtk::CheckButton) {
    let card = widgets::vbox(4);
    card.add_css_class("nxu-choice");
    let top = widgets::hbox(8);
    let radio = gtk::CheckButton::new();
    radio.set_group(group);
    top.append(&radio);
    let i = gtk::Image::from_icon_name(icon);
    i.add_css_class("nxu-choice-icon");
    top.append(&i);
    top.append(&widgets::label(title, "nxu-choice-title"));
    card.append(&top);
    let d = widgets::label(desc, "settings-option-description");
    d.set_wrap(true);
    d.set_xalign(0.0);
    card.append(&d);
    let click = gtk::GestureClick::new();
    let r = radio.clone();
    click.connect_released(move |_, _, _, _| r.set_active(true));
    card.add_controller(click);
    card.set_cursor_from_name(Some("pointer"));
    (card, radio)
}

pub fn build(page: &Page) {
    let s = nyxos::settings();
    let me = acc::me();
    let groups = passwd::groups();
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_string())
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "this computer".into());
    let people: Vec<Person> = acc::people()
        .into_iter()
        .map(|u| Person {
            admin: s.people.get(&u.name).map(|p| p.admin).unwrap_or_else(|| acc::is_admin(&u, &groups)),
            managed: s.people.contains_key(&u.name),
            login: u.name.clone(),
            full_name: u.full_name.clone(),
            home: u.home.clone(),
        })
        .collect();
    let mine = people.iter().find(|p| p.login == me).cloned().unwrap_or_else(|| Person {
        login: me.clone(),
        full_name: String::new(),
        home: std::env::var("HOME").unwrap_or_default(),
        admin: false,
        managed: false,
    });
    let others: Vec<Person> = people.iter().filter(|p| p.login != me).cloned().collect();
    let pending = Pending::new("accounts");

    // The picture: the person picked, or you when there's nobody else.
    let pv = page.split();
    let holder = widgets::vbox(14);
    pv.add(&nyxos::shrinkable(&holder));
    let detail = Rc::new(Detail { pane: holder, pending: pending.clone(), host: host.clone(), roles: RefCell::default() });
    let picked = others.first().cloned().unwrap_or_else(|| mine.clone());
    detail.show(&picked);

    // You: your face, your name and your password.
    let g = page.group("You");
    let line = widgets::hbox(12);
    line.add_css_class("nxu-me");
    line.append(&avatar(&mine.full_name, &me, 40));
    let t = widgets::vbox(2);
    t.set_valign(gtk::Align::Center);
    t.append(&widgets::label(&mine.title(), "nxu-me-name"));
    t.append(&widgets::label(&format!("Signed in as {me} · {}", mine.role()), "settings-option-description"));
    line.append(&t);
    g.add(&line);
    let (who, now) = (me.clone(), mine.full_name.clone());
    let p = pending.clone();
    let (r, entry) = widgets::entry_row("Full name", "On the sign-in screen.", &mine.full_name, "Your name", move |name| {
        let name = name.trim().to_string();
        if name != now && !valid_full_name(&name) {
            p.set("full-name", None);
            return window::toast("A name is up to 100 characters, without : or commas");
        }
        let change = (name != now).then(|| Change::FullName { user: who.clone(), full_name: name.clone() });
        p.set_shown("full-name", change, &now, &name);
    });
    entry.set_width_chars(14);
    mark_system(&r);
    widgets::keywords("name full name me profile");
    g.add(&r);
    let mine_login = me.clone();
    let (r, _) = widgets::button_row("Password", "For signing in, unlocking and system changes.", "Change…", move |_| {
        password_dialog(mine_login.clone(), true)
    });
    widgets::keywords("password passwd change");
    g.add(&r);

    // Everyone else, and adding someone.
    let g = page.group("Other people");
    if others.is_empty() {
        g.note("Nobody else has an account on this computer.");
    }
    let rows: Rc<RefCell<Vec<gtk::Box>>> = Rc::default();
    for person in &others {
        let desc = if person.managed {
            format!("{} · {}", person.login, person.role())
        } else {
            format!("{} · set in this computer's configuration", person.login)
        };
        let rows2 = rows.clone();
        let (detail, person2) = (detail.clone(), person.clone());
        let row_slot: Rc<RefCell<Option<gtk::Box>>> = Rc::default();
        let slot = row_slot.clone();
        let (r, _) = widgets::nav_row(None, &person.title(), &desc, "", move || {
            for other in rows2.borrow().iter() {
                other.remove_css_class("picked");
            }
            if let Some(row) = slot.borrow().as_ref() {
                row.add_css_class("picked");
            }
            detail.show(&person2);
        });
        *row_slot.borrow_mut() = Some(r.clone());
        r.prepend(&avatar(&person.full_name, &person.login, 34));
        r.add_css_class("nxu-person");
        if !person.managed
            && let Some(chevron) = nyxos::descendant(&r, "chevron").and_downcast::<gtk::Image>()
        {
            chevron.set_icon_name(Some("changes-prevent-symbolic"));
            chevron.set_tooltip_text(Some("Set in this computer's configuration"));
        }
        if person.admin
            && let Some(title) = nyxos::title_line(&r)
        {
            let shield = gtk::Image::from_icon_name("security-high-symbolic");
            shield.add_css_class("nxu-shield");
            shield.set_tooltip_text(Some("Administrator"));
            title.append(&shield);
        }
        if person.login == picked.login {
            r.add_css_class("picked");
        }
        rows.borrow_mut().push(r.clone());
        widgets::keywords(&format!("{} administrator admin password remove", person.login));
        g.add(&r);
    }
    let (add, _) = widgets::nav_row(None, "Add a person", &format!("Someone else who uses {host}."), "", add_dialog);
    add.add_css_class("nxu-add");
    if let Some(chevron) = nyxos::descendant(&add, "chevron") {
        chevron.set_visible(false);
    }
    let plus = gtk::Image::from_icon_name("list-add-symbolic");
    plus.add_css_class("nxu-plus");
    plus.set_valign(gtk::Align::Center);
    add.prepend(&plus);
    widgets::keywords("add user new account person family");
    g.add(&add);

    pv.foot(nyxos::layers_icon(), &format!("Each change is built into {host} with your password; it takes a minute or two."));
    pending.place(page);
}
