//! Mounts: drives mounted at startup, shares from a NAS or another computer
//! (SMB, NFS), and folders on another computer over SSH. Drives go into fstab
//! through UDisks; shares through Settings' root helper, which checks every
//! part of the line; SSH folders are the user's own services.

use super::disks::form;
use crate::backend::mounts::{self, Kind, Mount};
use crate::backend::udisks;
use crate::widgets::{self, Page};
use crate::{cmd, paths, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

fn refresh() {
    window::rebuild_if_built("mounts");
    window::rebuild_if_built("disks");
}

fn my_ids() -> (String, String) {
    let id = |flag: &str| cmd::output(&["id", flag]).map(|s| s.trim().to_string()).unwrap_or_else(|| "1000".into());
    (id("-u"), id("-g"))
}

fn need(program: &str, package: &'static str) -> bool {
    cmd::present(program) || crate::platform::provide_tool(program, package)
}

fn entry(placeholder: &str, text: &str) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.set_placeholder_text(Some(placeholder));
    e.set_text(text);
    e
}

fn labelled(body: &gtk::Box, label: &str, w: &impl IsA<gtk::Widget>) {
    let l = widgets::label(label, "dim");
    l.set_xalign(0.0);
    body.append(&l);
    body.append(w);
}

fn on_demand_switch(body: &gtk::Box, on: bool) -> gtk::Switch {
    let (r, sw) = widgets::switch_row(
        "Mount when first opened",
        "It's mounted the first time something opens the folder, so a missing drive or network never holds up starting.",
        on,
        |_| {},
    );
    body.append(&r);
    sw
}

fn read_only_switch(body: &gtk::Box) -> gtk::Switch {
    let (r, sw) = widgets::switch_row("Read-only", "Files can be opened but not changed.", false, |_| {});
    body.append(&r);
    sw
}

// ---------- Drives at startup ----------

/// The fstab type for a filesystem UDisks names.
fn fstab_type(fs: &str) -> &str {
    if fs == "ntfs" { "ntfs3" } else { fs }
}

/// Mount a partition at a folder every time the computer starts. With `None`,
/// pick the partition first.
pub fn startup_dialog(part: Option<udisks::Part>) {
    let candidates: Vec<udisks::Part> = udisks::load()
        .unwrap_or_default()
        .iter()
        .flat_map(|d| d.parts.iter().map(|p| p.content().clone()))
        .filter(|p| p.filesystem && !p.system && !p.fstab && !p.fs.is_empty())
        .collect();
    let body = widgets::vbox(8);
    let chooser = if part.is_none() {
        if candidates.is_empty() {
            window::toast("There's no partition to add: every one is the system's or already mounts at startup.");
            return;
        }
        let options: Vec<(String, String)> = candidates
            .iter()
            .map(|p| {
                (
                    p.path.clone(),
                    format!("{} · {} · {}", p.title(), udisks::fs_name(&p.fs), crate::backend::sysinfo::bytes_text(p.size)),
                )
            })
            .collect();
        let dd = widgets::dropdown(&options, &options[0].0);
        labelled(&body, "Partition", &dd);
        Some(dd)
    } else {
        None
    };
    let first = part.clone().or_else(|| candidates.first().cloned()).unwrap_or_default();
    let folder = entry("/mnt/name", &format!("/mnt/{}", mounts::slug(&first.title())));
    labelled(&body, "Folder", &folder);
    let on_demand = on_demand_switch(&body, false);
    let ro = read_only_switch(&body);
    if let Some(dd) = &chooser {
        let (c, f) = (candidates.clone(), folder.clone());
        dd.connect_selected_notify(move |d| {
            if let Some(p) = c.get(d.selected() as usize) {
                f.set_text(&format!("/mnt/{}", mounts::slug(&p.title())));
            }
        });
    }
    form(
        "Mount a drive at startup",
        "It mounts at the folder every time the computer starts. If the drive is missing, starting carries on without it.",
        &body,
        "Add",
        false,
        move || {
            let p = match (&part, &chooser) {
                (Some(p), _) => p.clone(),
                (None, Some(dd)) => match candidates.get(dd.selected() as usize) {
                    Some(p) => p.clone(),
                    None => return Some("Pick a partition.".into()),
                },
                _ => return Some("Pick a partition.".into()),
            };
            let dir = folder.text().trim().to_string();
            if !mounts::valid_dir(&dir, &paths::home()) {
                return Some("Use a folder inside /mnt, /media or your home folder.".into());
            }
            if crate::platform::fstab().iter().any(|m| m.dir == dir) {
                return Some(format!("Something already mounts at {dir}."));
            }
            let mut opts = vec!["nofail".to_string(), "x-gvfs-show".to_string(), "x-udisks-auth".to_string()];
            if on_demand.is_active() {
                opts.push("x-systemd.automount".into());
            }
            if ro.is_active() {
                opts.push("ro".into());
            }
            if ["vfat", "exfat", "ntfs"].contains(&p.fs.as_str()) {
                let (uid, gid) = my_ids();
                opts.extend([format!("uid={uid}"), format!("gid={gid}"), "umask=022".into()]);
            }
            if p.uuid.is_empty() {
                return Some("This partition has no filesystem id to mount it by.".into());
            }
            if !crate::platform::system_ready() {
                return Some("Settings' system helper isn't installed. Reinstall Settings to add it.".into());
            }
            // One helper call makes the folder, writes the line and mounts it,
            // so it asks for a password once at most.
            let id = unique_id(&mounts::slug(
                std::path::Path::new(&dir).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default().as_str(),
            ));
            let (uuid, fs, options) = (p.uuid.clone(), fstab_type(&p.fs).to_string(), opts.join(","));
            cmd::background(
                move || crate::platform::mount_add_drive(&id, &uuid, &dir, &fs, &options).map(|_| dir),
                |r| {
                    match r {
                        Ok(dir) => window::toast(&format!("It mounts at {dir} from now on.")),
                        Err(e) => window::toast(&format!("{e:#}")),
                    }
                    refresh();
                },
            );
            None
        },
    );
}

// ---------- Network shares ----------

fn smb_dialog() {
    if !need("mount.cifs", "cifs-utils") {
        return;
    }
    let body = widgets::vbox(8);
    let name = entry("Name, e.g. Photos on NAS", "");
    labelled(&body, "Name", &name);
    let server = entry("Server: nas.local or 192.168.1.20", "");
    labelled(&body, "Server", &server);
    let share = entry("Share: Photos", "");
    labelled(&body, "Share", &share);
    let user = entry("User name (blank for a guest share)", "");
    labelled(&body, "Sign in", &user);
    let pass = gtk::PasswordEntry::new();
    pass.set_placeholder_text(Some("Password"));
    pass.set_show_peek_icon(true);
    body.append(&pass);
    let version = widgets::dropdown(
        &[
            ("", "Version: automatic"),
            ("3.1.1", "SMB 3.1.1"),
            ("3.0", "SMB 3.0"),
            ("2.1", "SMB 2.1"),
            ("1.0", "SMB 1 (old devices)"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect::<Vec<_>>(),
        "",
    );
    body.append(&version);
    let folder = entry("/mnt/name", "");
    labelled(&body, "Folder", &folder);
    let n2 = name.clone();
    let f2 = folder.clone();
    let touched = Rc::new(Cell::new(false));
    let t2 = touched.clone();
    folder.connect_changed(move |e| t2.set(e.has_focus()));
    name.connect_changed(move |e| {
        if !touched.get() {
            f2.set_text(&format!("/mnt/{}", mounts::slug(&e.text())));
        }
    });
    let on_demand = on_demand_switch(&body, true);
    let ro = read_only_switch(&body);
    let test = gtk::Button::with_label("Test connection");
    test.set_halign(gtk::Align::Start);
    body.append(&test);

    let read = {
        let (server, share, user, pass, version, folder, on_demand, ro, name) = (
            server.clone(),
            share.clone(),
            user.clone(),
            pass.clone(),
            version.clone(),
            folder.clone(),
            on_demand.clone(),
            ro.clone(),
            n2.clone(),
        );
        Rc::new(move || -> Result<(String, String, String, Vec<String>, String), String> {
            let source = mounts::network_source("cifs", server.text().trim(), share.text().trim())
                .ok_or("Check the server and share: letters, digits, dots and dashes.")?;
            let mut opts = vec!["nofail".to_string(), "_netdev".to_string()];
            if on_demand.is_active() {
                opts.extend([
                    "x-systemd.automount".to_string(),
                    "x-systemd.idle-timeout=10min".to_string(),
                    "x-systemd.mount-timeout=20".to_string(),
                ]);
            }
            if ro.is_active() {
                opts.push("ro".into());
            }
            let v = ["", "3.1.1", "3.0", "2.1", "1.0"].get(version.selected() as usize).copied().unwrap_or("");
            if !v.is_empty() {
                opts.push(format!("vers={v}"));
            }
            let (uid, gid) = my_ids();
            opts.extend([format!("uid={uid}"), format!("gid={gid}"), "iocharset=utf8".into()]);
            let login = if user.text().trim().is_empty() {
                opts.push("guest".into());
                String::new()
            } else {
                format!("{}\n{}\n\n", user.text().trim(), pass.text())
            };
            let id =
                mounts::slug(&if name.text().trim().is_empty() { share.text().to_string() } else { name.text().to_string() });
            Ok((id, source, folder.text().trim().to_string(), opts, login))
        })
    };
    let r2 = read.clone();
    test.connect_clicked(move |b| {
        let (_, source, _, opts, login) = match r2() {
            Ok(x) => x,
            Err(e) => return window::toast(&e),
        };
        b.set_sensitive(false);
        b.set_label("Testing…");
        let b2 = b.clone();
        cmd::background(
            move || {
                let opts = opts.join(",");
                crate::platform::mount_test("cifs", &source, &opts, &login)
            },
            move |r| {
                b2.set_sensitive(true);
                b2.set_label("Test connection");
                match r {
                    Ok(_) => window::toast("Connected: the share works."),
                    Err(e) => window::toast(&format!("Couldn't connect: {e:#}")),
                }
            },
        );
    });
    form(
        "Add a Windows or NAS share (SMB)",
        "The password is kept in a file only the system can read.",
        &body,
        "Add",
        false,
        move || {
            let (id, source, dir, opts, login) = match read() {
                Ok(x) => x,
                Err(e) => return Some(e),
            };
            if !mounts::valid_dir(&dir, &paths::home()) {
                return Some("Use a folder inside /mnt, /media or your home folder.".into());
            }
            add_network(id, "cifs", source, dir, opts, login);
            None
        },
    );
}

fn nfs_dialog() {
    if !need("mount.nfs", "nfs-utils") {
        return;
    }
    let body = widgets::vbox(8);
    let name = entry("Name, e.g. Media", "");
    labelled(&body, "Name", &name);
    let server = entry("Server: nas.local or 192.168.1.20", "");
    labelled(&body, "Server", &server);
    let export = entry("Exported folder: /srv/media", "");
    labelled(&body, "Folder on the server", &export);
    let version = widgets::dropdown(
        &[("", "Version: automatic"), ("4.2", "NFS 4.2"), ("4", "NFS 4"), ("3", "NFS 3")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect::<Vec<_>>(),
        "",
    );
    body.append(&version);
    let folder = entry("/mnt/name", "");
    labelled(&body, "Folder here", &folder);
    let f2 = folder.clone();
    name.connect_changed(move |e| f2.set_text(&format!("/mnt/{}", mounts::slug(&e.text()))));
    let on_demand = on_demand_switch(&body, true);
    let ro = read_only_switch(&body);
    form("Add an NFS share", "", &body, "Add", false, move || {
        let Some(source) = mounts::network_source("nfs", server.text().trim(), export.text().trim()) else {
            return Some("Check the server and folder.".into());
        };
        let dir = folder.text().trim().to_string();
        if !mounts::valid_dir(&dir, &paths::home()) {
            return Some("Use a folder inside /mnt, /media or your home folder.".into());
        }
        let mut opts = vec!["nofail".to_string(), "_netdev".to_string(), "soft".to_string()];
        if on_demand.is_active() {
            opts.extend([
                "x-systemd.automount".to_string(),
                "x-systemd.idle-timeout=10min".to_string(),
                "x-systemd.mount-timeout=20".to_string(),
            ]);
        }
        if ro.is_active() {
            opts.push("ro".into());
        }
        if let Some(v) = ["", "4.2", "4", "3"].get(version.selected() as usize).filter(|v| !v.is_empty()) {
            opts.push(format!("nfsvers={v}"));
        }
        let id = mounts::slug(&if name.text().trim().is_empty() { export.text().to_string() } else { name.text().to_string() });
        add_network(id, "nfs", source, dir, opts, String::new());
        None
    });
}

/// `id`, or `id-2`, `id-3`… if a mount already has it.
fn unique_id(id: &str) -> String {
    let taken: Vec<String> = crate::platform::fstab().into_iter().map(|m| m.id).collect();
    let mut out = id.to_string();
    let mut n = 2;
    while taken.contains(&out) {
        out = format!("{id}-{n}");
        n += 1;
    }
    out
}

fn add_network(id: String, fstype: &'static str, source: String, dir: String, opts: Vec<String>, login: String) {
    if !crate::platform::system_ready() {
        window::toast("Settings' system helper isn't installed. Reinstall Settings to add it.");
        return;
    }
    let id2 = unique_id(&id);
    cmd::background(
        move || {
            let opts = if opts.is_empty() { "-".to_string() } else { opts.join(",") };
            crate::platform::mount_add_network(&id2, fstype, &source, &dir, &opts, &login)
        },
        |r| {
            match r {
                Ok(_) => window::toast("Added. It's in the list below."),
                Err(e) => window::toast(&format!("{e:#}")),
            }
            refresh();
        },
    );
}

// ---------- SSH folders ----------

fn ssh_dialog() {
    if !need("sshfs", "sshfs") {
        return;
    }
    let body = widgets::vbox(8);
    let name = entry("Name, e.g. Server", "");
    labelled(&body, "Name", &name);
    let target = entry("user@computer", &format!("{}@", std::env::var("USER").unwrap_or_default()));
    labelled(&body, "Sign in as", &target);
    let remote = entry("Folder there (blank: their home folder)", "");
    labelled(&body, "Folder on that computer", &remote);
    let home = paths::home();
    let folder = entry("~/Remote/name", "");
    labelled(&body, "Folder here", &folder);
    let (f2, h2) = (folder.clone(), home.clone());
    name.connect_changed(move |e| f2.set_text(&h2.join("Remote").join(e.text().trim()).to_string_lossy()));
    let note =
        widgets::label("It signs in with your SSH key, so the other computer needs your public key (Users → SSH keys).", "dim");
    note.set_wrap(true);
    note.set_xalign(0.0);
    body.append(&note);
    let test = gtk::Button::with_label("Test connection");
    test.set_halign(gtk::Align::Start);
    body.append(&test);
    let t2 = target.clone();
    test.connect_clicked(move |b| {
        let who = t2.text().trim().to_string();
        b.set_sensitive(false);
        let b2 = b.clone();
        cmd::background(
            move || cmd::run(&["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=6", "--", &who, "true"]),
            move |r| {
                b2.set_sensitive(true);
                match r {
                    Ok(_) => window::toast("Connected with your key."),
                    Err(e) => window::toast(&format!("Couldn't sign in with your key: {e:#}")),
                }
            },
        );
    });
    form("Add a folder over SSH", "", &body, "Add", false, move || {
        let who = target.text().trim().to_string();
        let Some((user, host)) = who.split_once('@') else {
            return Some("Write it as user@computer.".into());
        };
        let label = name.text().trim().to_string();
        if label.is_empty() {
            return Some("Give it a name.".into());
        }
        let dir = folder.text().trim().to_string();
        if !mounts::valid_dir(&dir, &home) || !dir.starts_with(&*home.to_string_lossy()) {
            return Some("Use a folder inside your home folder.".into());
        }
        let unit = match mounts::ssh_unit(&label, user, host, remote.text().trim(), &dir) {
            Ok(u) => u,
            Err(e) => return Some(e),
        };
        let id = mounts::slug(&label);
        let path = mounts::ssh_unit_dir(&home).join(mounts::ssh_unit_name(&id));
        if path.exists() {
            return Some("There's already an SSH folder with that name.".into());
        }
        if let Err(e) = std::fs::create_dir_all(mounts::ssh_unit_dir(&home)).and_then(|_| std::fs::write(&path, unit)) {
            return Some(format!("Couldn't save it: {e}"));
        }
        let name = mounts::ssh_unit_name(&id);
        cmd::background(
            move || {
                cmd::run(&["systemctl", "--user", "daemon-reload"])?;
                cmd::run(&["systemctl", "--user", "enable", "--now", &name])
            },
            |r| {
                match r {
                    Ok(_) => window::toast("Added: it's mounted now and at every sign-in."),
                    Err(e) => window::toast(&format!("Saved, but it didn't start: {e:#}")),
                }
                refresh();
            },
        );
        None
    });
}

fn remove_ssh(id: String) {
    let home = paths::home();
    let name = mounts::ssh_unit_name(&id);
    let path = mounts::ssh_unit_dir(&home).join(&name);
    cmd::background(
        move || {
            let _ = cmd::run(&["systemctl", "--user", "disable", "--now", &name]);
            let _ = std::fs::remove_file(&path);
            cmd::run(&["systemctl", "--user", "daemon-reload"])
        },
        |_| {
            window::toast("Removed.");
            refresh();
        },
    );
}

// ---------- The page ----------

fn kind_icon(k: &Kind) -> &'static str {
    match k {
        Kind::Drive => "drive-harddisk-symbolic",
        Kind::Smb => "folder-remote-symbolic",
        Kind::Nfs => "network-workgroup-symbolic",
        Kind::Ssh => "utilities-terminal-symbolic",
        Kind::Other => "folder-symbolic",
    }
}

fn kind_name(k: &Kind) -> &'static str {
    match k {
        Kind::Drive => "A drive at startup",
        Kind::Smb => "A Windows or NAS share",
        Kind::Nfs => "An NFS share",
        Kind::Ssh => "A folder over SSH",
        Kind::Other => "A mount",
    }
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Mounted,
    /// Waiting to be mounted the first time something opens it.
    Waiting,
    Not,
}

fn status(dir: &str, now: &[(String, String)]) -> State {
    let here: Vec<&String> = now.iter().filter(|(d, _)| d == dir).map(|(_, k)| k).collect();
    if here.iter().any(|k| *k != "autofs") {
        State::Mounted
    } else if !here.is_empty() {
        State::Waiting
    } else {
        State::Not
    }
}

fn state_chip(s: State) -> gtk::Box {
    let (text, class) = match s {
        State::Mounted => ("Mounted", "ok"),
        State::Waiting => ("Mounts when opened", "accent"),
        State::Not => ("Not mounted", ""),
    };
    let l = widgets::label(text, "cn-chip");
    if !class.is_empty() {
        l.add_css_class(class);
    }
    // In a box, so the card's padding for labels in rows doesn't reach it.
    let b = widgets::hbox(0);
    b.set_valign(gtk::Align::Center);
    b.append(&l);
    b
}

/// How a mount is removed: through the helper (a line Settings added to
/// fstab) or the user's services (an SSH folder). The user's own fstab lines
/// have none.
#[derive(Clone)]
enum Remove {
    Helper(String),
    Ssh(String),
}

fn remove(how: &Remove) {
    match how {
        Remove::Helper(id) => {
            let id = id.clone();
            cmd::background(
                move || crate::platform::mount_remove(&id),
                |r| {
                    match r {
                        Ok(_) => window::toast("Removed."),
                        Err(e) => window::toast(&format!("{e:#}")),
                    }
                    refresh();
                },
            );
        }
        Remove::Ssh(id) => remove_ssh(id.clone()),
    }
}

/// A mount on the list: an fstab line or an SSH folder.
#[derive(Clone)]
struct Item {
    m: Mount,
    how: Option<Remove>,
}

fn items() -> Vec<Item> {
    // Settings removes only the lines it added; the rest are yours.
    let mut out: Vec<Item> = crate::platform::fstab()
        .into_iter()
        .filter(|m| !m.system)
        .map(|m| {
            let how = m.ours().then(|| Remove::Helper(m.id.clone()));
            Item { m, how }
        })
        .collect();
    for (id, target, dir) in mounts::ssh_folders(&paths::home()) {
        out.push(Item {
            m: Mount {
                id: id.clone(),
                kind: Kind::Ssh,
                source: target,
                dir,
                fstype: "fuse.sshfs".into(),
                options: String::new(),
                system: false,
            },
            how: Some(Remove::Ssh(id)),
        });
    }
    out
}

fn name_of(m: &Mount) -> String {
    std::path::Path::new(&m.dir).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(m.dir.clone())
}

/// A folder with the home folder written as ~.
fn short_dir(dir: &str) -> String {
    let home = paths::home().to_string_lossy().to_string();
    match dir.strip_prefix(&home) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
        _ => dir.to_string(),
    }
}

fn in_home(dir: &str) -> bool {
    short_dir(dir).starts_with('~')
}

fn option<'a>(m: &'a Mount, key: &str) -> Option<&'a str> {
    m.options.split(',').find_map(|o| o.strip_prefix(key)?.strip_prefix('='))
}

fn has_option(m: &Mount, key: &str) -> bool {
    m.options.split(',').any(|o| o == key)
}

/// Where a mount's files come from: the machine or drive, what of it, and how
/// it connects.
struct Route {
    icon: &'static str,
    from: String,
    what: String,
    how: String,
}

fn route(m: &Mount, part: Option<&udisks::Part>) -> Route {
    match m.kind {
        Kind::Smb => {
            let rest = m.source.trim_start_matches('/');
            let (server, share) = rest.split_once('/').unwrap_or((rest, ""));
            let mut how = option(m, "vers").map_or("SMB".to_string(), |v| format!("SMB {v}"));
            if has_option(m, "guest") {
                how.push_str(" · as a guest");
            } else if let Some(u) = option(m, "username").or(option(m, "user")) {
                how.push_str(&format!(" · signed in as {u}"));
            } else if option(m, "credentials").is_some() {
                how.push_str(" · with a saved sign-in");
            }
            Route {
                icon: "network-server-symbolic",
                from: server.to_string(),
                what: if share.trim_matches('/').is_empty() { "The whole server".into() } else { format!("Share: {share}") },
                how,
            }
        }
        Kind::Nfs => {
            let (server, path) = m.source.split_once(':').unwrap_or((&m.source, ""));
            Route {
                icon: "network-server-symbolic",
                from: server.to_string(),
                what: format!("Folder: {path}"),
                how: option(m, "nfsvers").or(option(m, "vers")).map_or("NFS".to_string(), |v| format!("NFS {v}")),
            }
        }
        Kind::Ssh => {
            let (who, path) = m.source.split_once(':').unwrap_or((&m.source, ""));
            let (user, host) = who.split_once('@').unwrap_or(("", who));
            Route {
                icon: "computer-symbolic",
                from: host.to_string(),
                what: if path.is_empty() { "Their home folder".into() } else { format!("Folder: {path}") },
                how: if user.is_empty() { "SSH, with your key".into() } else { format!("SSH · signed in as {user} with your key") },
            }
        }
        Kind::Drive => match part {
            Some(p) => Route {
                icon: "drive-harddisk-symbolic",
                from: p.title(),
                what: format!("{} partition", crate::backend::sysinfo::bytes_text(p.size)),
                how: udisks::fs_name(&p.fs),
            },
            None => Route {
                icon: "drive-harddisk-symbolic",
                from: m.source.trim_start_matches("UUID=").chars().take(13).collect(),
                what: "A partition".into(),
                how: udisks::fs_name(&m.fstype),
            },
        },
        Kind::Other => Route { icon: "folder-symbolic", from: m.source.clone(), what: m.fstype.clone(), how: String::new() },
    }
}

/// The partition a drive line mounts, found through UDisks.
fn find_part(source: &str) -> Option<udisks::Part> {
    let uuid = source.strip_prefix("UUID=")?;
    udisks::load()
        .ok()?
        .iter()
        .flat_map(|d| d.parts.iter())
        .find(|p| p.uuid == uuid || p.content().uuid == uuid)
        .map(|p| p.content().clone())
}

/// (size, used, free) in bytes of the filesystem mounted at `dir`. Only ask once
/// it's mounted: asking about a folder that mounts when opened would mount it.
fn space(dir: &str) -> Option<(u64, u64, u64)> {
    let text = cmd::output(&["timeout", "4", "df", "-B1", "--output=size,used,avail", "--", dir])?;
    let n: Vec<u64> = text.lines().nth(1)?.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    (n.len() == 3 && n[0] > 0).then(|| (n[0], n[1], n[2]))
}

thread_local! {
    /// The folder of the mount shown on the right, kept while the page is redrawn.
    static PICKED: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

struct Ui {
    items: Vec<Item>,
    now: Vec<(String, String)>,
    body: gtk::Box,
    removal: gtk::Box,
    foot: gtk::Label,
    rows: std::cell::RefCell<Vec<(String, gtk::Box)>>,
}

pub fn build(page: &Page) {
    if let (_, Some(notice)) = crate::platform::system_notice("mounts", "adding a network share") {
        page.body.append(&notice);
    }
    widgets::keywords("mount fstab nas samba smb cifs windows share nfs sshfs ssh network drive automount startup");
    let pv = page.split();
    super::connectivity::steady(&pv);
    let body = widgets::vbox(14);
    pv.add(&body);
    let foot = pv.foot("dialog-information-symbolic", "");
    let removal = widgets::hbox(4);
    if let Some(row) = foot.parent().and_downcast::<gtk::Box>() {
        row.append(&removal);
    }
    let items = items();
    let picked = PICKED
        .with(|p| p.borrow().clone())
        .filter(|d| items.iter().any(|i| &i.m.dir == d))
        .or_else(|| items.first().map(|i| i.m.dir.clone()));
    PICKED.with(|p| *p.borrow_mut() = picked);
    let ui = Rc::new(Ui { items, now: mounts::mounted(), body, removal, foot, rows: Default::default() });
    list(page, &ui);
    add_kinds(page);
    system_own(page);
    show(&ui, None);
}

fn list(page: &Page, ui: &Rc<Ui>) {
    let g = page.group("Your mounts");
    if ui.items.is_empty() {
        g.add(&widgets::icon_row(
            "folder-remote-symbolic",
            "None yet",
            "Mounts you add show here, with whether they're mounted now.",
            None,
        ));
        return;
    }
    for item in &ui.items {
        let m = &item.m;
        let r = widgets::icon_row(
            kind_icon(&m.kind),
            &glib::markup_escape_text(&name_of(m)),
            &glib::markup_escape_text(&format!("In {}", short_dir(&m.dir))),
            Some(state_chip(status(&m.dir, &ui.now)).upcast_ref()),
        );
        r.add_css_class("cn-click");
        r.set_cursor_from_name(Some("pointer"));
        r.set_focusable(true);
        let (ui2, dir) = (ui.clone(), m.dir.clone());
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            PICKED.with(|p| *p.borrow_mut() = Some(dir.clone()));
            show(&ui2, None);
        });
        r.add_controller(click);
        ui.rows.borrow_mut().push((m.dir.clone(), r.clone()));
        g.add(&r);
    }
}

fn add_kinds(page: &Page) {
    let g = page.plain_group("Add a mount");
    g.note(crate::platform::mounts_note());
    let grid = gtk::Grid::new();
    grid.set_column_spacing(8);
    grid.set_row_spacing(8);
    grid.set_column_homogeneous(true);
    let kinds: [(&str, &str, &str, fn()); 4] = [
        ("drive-harddisk-symbolic", "Drive at startup", "A partition, mounted at a folder every time", || startup_dialog(None)),
        ("folder-remote-symbolic", "Windows or NAS share", "SMB: shared folders from a NAS or a Windows PC", smb_dialog),
        ("network-workgroup-symbolic", "NFS share", "Shared folders from a Linux server or NAS", nfs_dialog),
        ("utilities-terminal-symbolic", "Folder over SSH", "A folder on another computer you can sign in to", ssh_dialog),
    ];
    for (i, (icon, title, tip, open)) in kinds.into_iter().enumerate() {
        let b = gtk::Button::new();
        b.add_css_class("cn-mt-kind");
        b.set_tooltip_text(Some(tip));
        let c = widgets::hbox(10);
        c.append(&widgets::tile(icon));
        let t = widgets::label(title, "cn-strong");
        t.set_wrap(true);
        t.set_xalign(0.0);
        c.append(&t);
        b.set_child(Some(&c));
        b.connect_clicked(move |_| open());
        grid.attach(&b, (i % 2) as i32, (i / 2) as i32, 1, 1);
    }
    g.add(&grid);
}

fn system_own(page: &Page) {
    let system: Vec<Mount> = crate::platform::fstab().into_iter().filter(|m| m.system).collect();
    if system.is_empty() {
        return;
    }
    let g = page.group("");
    widgets::keywords("system root boot home swap partitions fstab");
    let n = system.len().to_string();
    let (r, _) = widgets::nav_row(
        Some("system-lock-screen-symbolic"),
        "The system's own",
        "Where Linux itself is mounted; Settings leaves these alone",
        &n,
        move || {
            let system = system.clone();
            widgets::subpage("The system's own", 560, move |page| {
                let g = page.group("");
                g.note("Where the system itself is mounted. Settings leaves these alone.");
                for m in system {
                    let (r, _) = widgets::info_row(&m.dir, &format!("{} · {}", m.fstype, m.source));
                    g.add(&r);
                }
            });
        },
    );
    g.add(&r);
}

// ----- The picked mount -----

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// A row in the preview: a tile, its text and a value at the right.
fn fact_row(icon: &str, title: &str, desc: &str, value: &str) -> gtk::Box {
    let v = widgets::label(value, "row-value");
    v.set_valign(gtk::Align::Center);
    widgets::icon_row(icon, title, desc, Some(v.upcast_ref()))
}

fn show(ui: &Rc<Ui>, part: Option<udisks::Part>) {
    let picked = PICKED.with(|p| p.borrow().clone());
    for (dir, row) in ui.rows.borrow().iter() {
        if picked.as_deref() == Some(dir.as_str()) {
            row.add_css_class("cn-picked");
        } else {
            row.remove_css_class("cn-picked");
        }
    }
    clear(&ui.body);
    clear(&ui.removal);
    let Some(item) = picked.and_then(|d| ui.items.iter().find(|i| i.m.dir == d)) else {
        ui.foot.set_text("Settings adds each one so a missing drive or network never holds up starting.");
        let empty = nyx_ui::widgets::empty_state(
            "folder-remote-symbolic",
            "No mounts yet",
            "Add a drive, a share from a NAS or Windows PC, or a folder over SSH on the left.",
            None,
        );
        empty.add_css_class("cn-empty");
        ui.body.append(&empty);
        return;
    };
    let m = &item.m;
    let state = status(&m.dir, &ui.now);
    if m.kind == Kind::Drive && part.is_none() {
        let (ui2, source, dir) = (ui.clone(), m.source.clone(), m.dir.clone());
        cmd::background(
            move || find_part(&source),
            move |p| {
                if p.is_some() && PICKED.with(|x| x.borrow().as_deref() == Some(dir.as_str())) {
                    show(&ui2, p);
                }
            },
        );
    }

    // The heading: its name, its kind, and opening it.
    let head = widgets::hbox(8);
    head.add_css_class("preview-head");
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(&name_of(m), "preview-title");
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&t);
    let origin = match (&m.kind, &item.how) {
        (Kind::Ssh, _) => "kept mounted while you're signed in",
        (_, Some(_)) => "added in Settings",
        (_, None) => "your own line in /etc/fstab",
    };
    let l = widgets::label(&format!("{} · {origin}", kind_name(&m.kind)), "preview-line");
    l.set_wrap(true);
    l.set_xalign(0.0);
    text.append(&l);
    head.append(&text);
    if state != State::Not {
        let open = nyx_ui::widgets::labeled_button("folder-open-symbolic", "Open folder");
        open.set_valign(gtk::Align::Center);
        let d = m.dir.clone();
        open.connect_clicked(move |_| cmd::spawn(&["xdg-open", &d]));
        head.append(&open);
    }
    if m.kind == Kind::Ssh {
        let unit = mounts::ssh_unit_name(&m.id);
        let (label, verb) = if state == State::Mounted { ("Unmount", "stop") } else { ("Mount", "start") };
        let b = gtk::Button::with_label(label);
        b.set_valign(gtk::Align::Center);
        b.connect_clicked(move |b| {
            b.set_sensitive(false);
            cmd::run_async(&["systemctl", "--user", verb, &unit], |r| {
                if let Err(e) = r {
                    window::toast(&format!("{e:#}"));
                }
                refresh();
            });
        });
        head.append(&b);
    }
    ui.body.append(&head);

    // The path: where the files come from, how, and the folder they appear as here.
    let rt = route(m, part.as_ref());
    let card = widgets::vbox(0);
    card.add_css_class("preview-card");
    card.add_css_class("cn-mt-route");
    let line = widgets::hbox(8);
    let end = |icon: &str, title: &str, desc: &str| {
        let b = widgets::vbox(4);
        b.add_css_class("cn-mt-end");
        b.set_size_request(130, -1);
        let tile = widgets::tile(icon);
        tile.add_css_class("cn-mt-big");
        b.append(&tile);
        let t = widgets::label(title, "cn-strong");
        t.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        t.set_max_width_chars(20);
        t.set_tooltip_text(Some(title));
        b.append(&t);
        let d = widgets::label(desc, "cn-note");
        d.set_wrap(true);
        d.set_justify(gtk::Justification::Center);
        d.set_max_width_chars(20);
        b.append(&d);
        b
    };
    line.append(&end(rt.icon, &rt.from, &rt.what));
    let mid = widgets::vbox(2);
    mid.set_hexpand(true);
    mid.set_valign(gtk::Align::Center);
    let how = widgets::label(&rt.how, "cn-note");
    how.set_wrap(true);
    how.set_justify(gtk::Justification::Center);
    mid.append(&how);
    let live = state == State::Mounted;
    let arrow = widgets::drawing(14, move |cr, p, accent, w, h| {
        let y = h / 2.0;
        let c = if live { accent } else { p.dim };
        crate::sketch::set(cr, c, if live { 1.0 } else { 0.6 });
        cr.set_line_width(2.0);
        if !live {
            cr.set_dash(&[5.0, 4.0], 0.0);
        }
        cr.move_to(2.0, y);
        cr.line_to(w - 8.0, y);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        cr.move_to(w - 2.0, y);
        cr.line_to(w - 10.0, y - 5.0);
        cr.line_to(w - 10.0, y + 5.0);
        cr.close_path();
        let _ = cr.fill();
    });
    mid.append(&arrow);
    line.append(&mid);
    let here = if in_home(&m.dir) { "A folder in your home" } else { "A folder for everyone here" };
    line.append(&end("folder-symbolic", &short_dir(&m.dir), here));
    card.append(&line);
    ui.body.append(&card);

    // How full it is.
    let fill = widgets::vbox(8);
    fill.add_css_class("preview-card");
    fill.add_css_class("cn-mt-space");
    let top = widgets::hbox(8);
    let used = widgets::label("", "cn-strong");
    used.set_hexpand(true);
    used.set_xalign(0.0);
    used.set_wrap(true);
    let free = widgets::label("", "cn-note");
    top.append(&used);
    top.append(&free);
    fill.append(&top);
    let share = Rc::new(std::cell::Cell::new(None::<f64>));
    let bar = {
        let share = share.clone();
        widgets::drawing(8, move |cr, p, accent, w, h| {
            crate::sketch::set(cr, p.text, 0.08);
            crate::sketch::rounded(cr, 0.0, 0.0, w, h, h / 2.0);
            let _ = cr.fill();
            if let Some(f) = share.get() {
                crate::sketch::set(cr, accent, 1.0);
                crate::sketch::rounded(cr, 0.0, 0.0, (w * f).max(h), h, h / 2.0);
                let _ = cr.fill();
            }
        })
    };
    bar.set_visible(state == State::Mounted);
    fill.append(&bar);
    let place = match m.kind {
        Kind::Drive => "on the drive",
        Kind::Ssh => "on that computer",
        Kind::Smb | Kind::Nfs => "on the server",
        Kind::Other => "there",
    };
    if state != State::Mounted {
        used.remove_css_class("cn-strong");
    }
    match state {
        State::Mounted => {
            used.set_text("Reading how full it is…");
            let (dir, place) = (m.dir.clone(), place.to_string());
            cmd::background(
                move || space(&dir),
                move |s| match s {
                    Some((size, u, f)) => {
                        let b = crate::backend::sysinfo::bytes_text;
                        used.set_text(&format!("{} used of {}", b(u), b(size)));
                        free.set_text(&format!("{} free {place}", b(f)));
                        share.set(Some(u as f64 / size as f64));
                        bar.queue_draw();
                    }
                    None => used.set_text("It didn't say how full it is."),
                },
            );
        }
        State::Waiting => used.set_text("How full it is shows once it's mounted: opening the folder mounts it."),
        State::Not => used.set_text("It isn't mounted now."),
    }
    ui.body.append(&fill);

    // How it's mounted.
    let opts = widgets::vbox(0);
    opts.add_css_class("settings-card");
    let on_off = |on: bool| if on { "On" } else { "Off" };
    let missing = match m.kind {
        Kind::Drive => "A missing drive never holds up starting",
        _ => "A missing server never holds up starting",
    };
    if m.kind == Kind::Ssh {
        let who = m.source.split_once(':').map_or(m.source.as_str(), |(w, _)| w);
        opts.append(&fact_row("system-users-symbolic", "Signed in as", "With your SSH key (Users → SSH keys)", who));
    } else {
        opts.append(&fact_row("alarm-symbolic", "Mount when first opened", missing, on_off(m.on_demand())));
        opts.append(&fact_row("view-reveal-symbolic", "Read-only", "Files open but can't be changed", on_off(has_option(m, "ro"))));
    }
    if m.kind == Kind::Smb {
        let (who, desc) = if has_option(m, "guest") {
            ("A guest".to_string(), "The share asks for no password")
        } else if let Some(u) = option(m, "username").or(option(m, "user")) {
            (u.to_string(), "The password is kept where only the system can read it")
        } else if option(m, "credentials").is_some() {
            ("A saved sign-in".to_string(), "Kept where only the system can read it")
        } else {
            ("Nobody yet".to_string(), "It mounts without signing in")
        };
        opts.append(&fact_row("system-users-symbolic", "Signed in as", desc, &who));
    }
    ui.body.append(&opts);

    match &item.how {
        Some(how) => {
            ui.foot.set_text(&format!("Stopping it leaves every file {place} where it is."));
            let how = how.clone();
            let b = widgets::confirm_button("Stop mounting it", "Stop mounting it?", move |b| {
                b.set_sensitive(false);
                remove(&how);
            });
            b.add_css_class("flat");
            b.add_css_class("cn-danger");
            ui.removal.append(&b);
        }
        None => ui.foot.set_text("It's your own line in /etc/fstab, so Settings leaves it as it is."),
    }
}
