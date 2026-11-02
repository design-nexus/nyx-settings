//! Users, groups, sudo, SSH and sign-in security.
//!
//! Reading is free. Changing accounts goes through `settings-helper` (root, via
//! pkexec, see `backend::accounts`); the Omarchy security scripts are
//! interactive, so they open in a floating terminal like the updater does.

use crate::backend::accounts as acc;
use crate::backend::passwd::{self, Group, User};
use crate::dialog::{Field, ask};
use crate::widgets::{self, Page};
use crate::{cmd, paths, window};
use gtk::prelude::*;
use std::cell::RefCell;
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

pub fn terminal(command: &str) {
    cmd::spawn(&["omarchy-launch-floating-terminal-with-presentation", command]);
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
fn password_problem(a: &str, b: &str) -> Option<String> {
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
        "Add a user",
        "The new user gets their own home folder.",
        vec![Field::text("User name"), Field::secret("Password"), Field::secret("Password again")],
        "Add user",
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
fn password_free_group(page: &Page, can_edit: bool) {
    let g = page.group("Passwords for system changes");
    g.note(
        "Changing disks, mounts and the package cache asks for your password once, then remembers it for a few minutes. \
         Users, passwords and the firewall always ask.",
    );
    let syncing = Rc::new(std::cell::Cell::new(false));
    let s2 = syncing.clone();
    let (r, sw) = widgets::switch_row(
        "Don't ask for disks, mounts and cleanup",
        "No password while you're signed in at this computer. Any program you run could then format a drive or change mounts \
         without asking, so leave it off if you're unsure.",
        crate::prefs::get().password_free,
        move |_| {},
    );
    sw.set_sensitive(can_edit);
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
    widgets::keywords("password polkit sudo ask prompt authentication disks mounts");
    g.add(&r);
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
            "You can't rename the account you're signed in with."
        } else {
            "Renaming only works while the user is signed out."
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

// ----- Page -----

pub fn build(page: &Page) {
    let can_edit = require_helper(page, "accounts", "changing users and groups");

    let me = acc::me();
    let users = acc::people();
    let groups = passwd::groups();
    let shells = passwd::shells();

    // ----- You -----
    // Your own account, its password and shell; other people are listed below.
    if let Some(mine) = users.iter().find(|u| u.name == me) {
        let g = page.group("You");
        let member_of = acc::groups_of(mine, &groups);
        g.add(&user_row(mine, &me, &groups, can_edit, &member_of));
        widgets::keywords("passwd login signed in groups");
        if can_edit && !shells.is_empty() {
            let opts: Vec<(String, String)> =
                shells.iter().map(|s| (s.clone(), s.rsplit('/').next().unwrap_or(s).to_string())).collect();
            let name = me.clone();
            let (r, _) = widgets::choice_row(
                "Login shell",
                "The shell new terminals start with. Applies to new terminals.",
                opts,
                &mine.shell,
                move |sh| apply(args(&["set-shell", &name, &sh]), None, "Shell changed"),
            );
            widgets::keywords("bash zsh fish chsh");
            g.add(&r);
        }
    }

    password_free_group(page, can_edit);

    // ----- Users -----
    let g = page.group("Other users");
    let others: Vec<&User> = users.iter().filter(|u| u.name != me).collect();
    g.note(if others.is_empty() {
        "Nobody else has an account on this computer."
    } else {
        "Deleting a user keeps their home folder, so no files are lost."
    });
    for u in others {
        g.add(&user_row(u, &me, &groups, can_edit, &[]));
    }
    if can_edit {
        let shell = users
            .iter()
            .find(|u| u.name == me)
            .map(|u| u.shell.clone())
            .filter(|s| shells.contains(s))
            .unwrap_or_else(|| "/bin/bash".into());
        let (r, _) = widgets::button_row("Add a user", "Create another account on this computer.", "Add user…", move |_| {
            add_user_dialog(shell.clone())
        });
        widgets::keywords("new account create");
        g.add(&r);
    }

    // ----- Groups -----
    groups_section(page, &users, &groups, can_edit);

    // ----- Sudo -----
    if cmd::present("omarchy-sudo-passwordless") {
        let g = page.collapsible("Administrator access", false);
        g.note("Use sudo without typing your password, for a while.");
        let (r, _) = widgets::button_row(
            "Passwordless sudo",
            "Turn off the sudo password prompt for a limited time. Run it again to turn it back on early.",
            "Open…",
            |_| terminal("omarchy-sudo-passwordless"),
        );
        widgets::keywords("sudo password nopasswd root admin");
        g.add(&r);
    }

    ssh_section(page);
    sign_in_section(page);
}

/// A user with their actions. `member_of` lists groups to show under the name (yours).
fn user_row(u: &User, me: &str, groups: &[Group], can_edit: bool, member_of: &[String]) -> gtk::Box {
    let is_me = u.name == me;
    let admin = acc::is_admin(u, groups);
    let locked = acc::is_locked(&u.name);
    let mut bits = Vec::new();
    if !u.full_name.is_empty() {
        bits.push(u.full_name.clone());
    }
    bits.push(u.home.clone());
    if !member_of.is_empty() {
        bits.push(format!("member of {}", member_of.join(", ")));
    }
    let controls = widgets::hbox(8);
    if can_edit {
        if !is_me {
            let sw = gtk::Switch::new();
            sw.set_active(admin);
            sw.set_valign(gtk::Align::Center);
            sw.set_tooltip_text(Some("Administrator"));
            let name = u.name.clone();
            sw.connect_state_set(move |s, on| {
                if on != admin {
                    s.set_sensitive(false);
                    apply(
                        args(&[if on { "add-member" } else { "remove-member" }, &name, "wheel"]),
                        None,
                        if on { "Now an administrator" } else { "No longer an administrator" },
                    );
                }
                gtk::glib::Propagation::Proceed
            });
            controls.append(&widgets::label("Admin", "dim"));
            controls.append(&sw);
            // Whether another user is locked can only be read as root, so offer
            // both actions when it isn't known.
            let actions: &[bool] = match locked {
                Some(true) => &[false],
                Some(false) => &[true],
                None => &[true, false],
            };
            for &lock_it in actions {
                let name = u.name.clone();
                let b = gtk::Button::with_label(if lock_it { "Lock" } else { "Unlock" });
                b.set_tooltip_text(Some(if lock_it { "Stop this user from signing in" } else { "Let this user sign in again" }));
                b.connect_clicked(move |b| {
                    b.set_sensitive(false);
                    apply(
                        args(&[if lock_it { "lock-user" } else { "unlock-user" }, &name]),
                        None,
                        if lock_it { "Account locked" } else { "Account unlocked" },
                    );
                });
                controls.append(&b);
            }
        }
        let edit = gtk::Button::from_icon_name("document-edit-symbolic");
        edit.add_css_class("flat");
        edit.set_tooltip_text(Some(if is_me { "Edit your full name and shell" } else { "Edit name, username and shell" }));
        let user = u.clone();
        edit.connect_clicked(move |_| edit_user(&user, is_me));
        controls.append(&edit);
        let name = u.name.clone();
        let pw = gtk::Button::with_label(if is_me { "Change password" } else { "Password" });
        pw.set_tooltip_text(Some(if is_me {
            "The password you sign in and use sudo with"
        } else {
            "Set a new password for this user"
        }));
        pw.connect_clicked(move |_| change_password(&name));
        controls.append(&pw);
        if !is_me {
            let name = u.name.clone();
            controls.append(&widgets::confirm_button("Delete", "Delete user?", move |b| {
                b.set_sensitive(false);
                apply(args(&["delete-user", &name, "1"]), None, "User deleted");
            }));
        }
    }
    let r = widgets::row(&u.name, &gtk::glib::markup_escape_text(&bits.join(" · ")), Some(controls.upcast_ref()));
    if is_me {
        widgets::tag_row(&r, "You");
    } else if admin {
        widgets::tag_row(&r, "Administrator");
    }
    if locked == Some(true) {
        widgets::tag_row(&r, "Locked");
    }
    widgets::keywords("user account login admin sudo wheel");
    r
}

fn groups_section(page: &Page, users: &[User], groups: &[Group], can_edit: bool) {
    let g = page.collapsible("Groups", false);
    g.note("A group gives its members access to something. Switch a user on to add them.");
    let user_names: Vec<String> = users.iter().map(|u| u.name.clone()).collect();
    let mut shown: Vec<(&Group, String, bool)> = Vec::new();
    for (name, desc) in NOTABLE {
        if let Some(gr) = groups.iter().find(|g| g.name == *name) {
            shown.push((gr, desc.to_string(), false));
        }
    }
    // Groups someone made: not a system group and not a user's own group.
    for gr in
        groups.iter().filter(|g| !g.is_system() && !user_names.contains(&g.name) && !NOTABLE.iter().any(|(n, _)| *n == g.name))
    {
        shown.push((gr, "A group you made.".into(), true));
    }
    for (gr, desc, custom) in shown {
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
            "Make a group, then add users to it above. Press Enter to create.",
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
    let g = page.collapsible("SSH", false);
    let active = acc::sshd_active();
    if cmd::present("omarchy-setup-security-sshd") {
        let (title, label, command) = if active {
            ("Remote sign-in", "Turn off…", "omarchy-remove-security-sshd")
        } else {
            ("Remote sign-in", "Turn on…", "omarchy-setup-security-sshd")
        };
        let (r, _) = widgets::button_row(
            title,
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
    }

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

fn sign_in_section(page: &Page) {
    let items: &[(&str, &str, &str, &str, &str, &str)] = &[
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
    // With Nyx Biometrics, face, fingerprint and keys live on their own page; Omarchy's
    // scripts would wire the same PAM files a second way.
    let biometrics = super::sign_in::available();
    let items: Vec<_> = items
        .iter()
        .filter(|i| cmd::present(i.0))
        .filter(|i| !biometrics || i.0 == "omarchy-setup-security-sudoless-docker")
        .collect();
    let drive = cmd::present("omarchy-drive-password");
    if items.is_empty() && !drive && !biometrics {
        return;
    }
    let g = page.collapsible("Sign-in security", biometrics);
    if biometrics {
        let (r, _) = widgets::button_row(
            "Face, fingerprint and security keys",
            "Sign in to sudo, administrator prompts and the lock screen without typing.",
            "Open…",
            |_| window::navigate("sign-in"),
        );
        widgets::keywords("face fingerprint security key fido2 yubikey biometric");
        g.add(&r);
    }
    for (_, title, desc, setup, remove, kw) in items {
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
