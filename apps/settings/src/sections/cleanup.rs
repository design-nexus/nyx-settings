//! Disks, last group: how full each filesystem is, what's taking space in your
//! home folder, and cleanups that each say what they'd free.

use crate::backend::accounts;
use crate::backend::cleanup;
use crate::backend::sysinfo::bytes_text;
use crate::backend::udisks;
use crate::widgets::{self, Page};
use crate::{cmd, paths, window};
use gtk::prelude::*;

/// One cleanup: a row whose size is worked out in the background.
pub(crate) fn cleanup_row(
    g: &widgets::Group,
    title: &str,
    desc: &str,
    measure: impl FnOnce() -> u64 + Send + 'static,
    clean: impl Fn(gtk::Button) + 'static,
) {
    let b = gtk::Button::with_label("Clean");
    b.set_sensitive(false);
    let size = widgets::label("Measuring…", "dim");
    let control = widgets::hbox(12);
    control.append(&size);
    control.append(&b);
    g.add(&widgets::row(title, desc, Some(control.upcast_ref())));
    let (b2, size2) = (b.clone(), size.clone());
    cmd::background(measure, move |bytes| {
        if bytes < (1 << 20) {
            size2.set_text("Nothing to free");
        } else {
            size2.set_text(&format!("Frees {}", bytes_text(bytes)));
            b2.set_sensitive(true);
        }
    });
    b.connect_clicked(move |b| {
        b.set_sensitive(false);
        clean(b.clone());
    });
}

/// Run a root helper cleanup, then say how it went.
fn helper(args: &'static [&'static str], done: &'static str, button: gtk::Button) {
    if !accounts::helper_installed() {
        window::toast("Settings' system helper isn't installed. Reinstall Settings to add it.");
        button.set_sensitive(true);
        return;
    }
    cmd::background(
        move || accounts::helper(args, None),
        move |r| match r {
            Ok(_) => {
                window::toast(done);
                window::rebuild_if_built("disks");
            }
            Err(e) => {
                window::toast(&format!("{e:#}"));
                button.set_sensitive(true);
            }
        },
    );
}

pub fn build(page: &Page) {
    let g = page.group("Space");
    // Each filesystem that's mounted, once.
    let mut seen = Vec::new();
    for d in udisks::load().unwrap_or_default() {
        for p in &d.parts {
            let c = p.content();
            let Some(m) = c.mounts.first() else { continue };
            if seen.contains(&c.path) {
                continue;
            }
            seen.push(c.path.clone());
            let Some((used, size)) = udisks::used(m) else { continue };
            if size == 0 {
                continue;
            }
            let bar = gtk::LevelBar::for_interval(0.0, 1.0);
            bar.set_value(used as f64 / size as f64);
            bar.set_hexpand(true);
            bar.set_valign(gtk::Align::Center);
            bar.add_css_class("part-usage");
            bar.set_size_request(220, -1);
            let free = widgets::label(&format!("{} free of {}", bytes_text(size.saturating_sub(used)), bytes_text(size)), "dim");
            let control = widgets::vbox(4);
            control.append(&bar);
            control.append(&free);
            let title = if m == "/" { "System".to_string() } else { p.title() };
            g.add(&widgets::row(&title, &format!("{} · {}", udisks::fs_name(&c.fs), glib_escape(m)), Some(control.upcast_ref())));
        }
    }

    let g = page.collapsible("Biggest folders in your home", false);
    let list = widgets::vbox(0);
    list.append(&widgets::row("Measuring…", "This can take a minute on a big home folder.", None));
    g.add(&list);
    let home = paths::home();
    let l2 = list.clone();
    cmd::background(
        move || cleanup::largest(&home, 10),
        move |found| {
            while let Some(c) = l2.first_child() {
                l2.remove(&c);
            }
            let top = found.first().map(|f| f.1).unwrap_or(1).max(1);
            for (path, size) in found {
                let name =
                    std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(path.clone());
                let bar = gtk::LevelBar::for_interval(0.0, 1.0);
                bar.set_value(size as f64 / top as f64);
                bar.set_size_request(160, -1);
                bar.set_valign(gtk::Align::Center);
                bar.add_css_class("part-usage");
                let open = gtk::Button::from_icon_name("folder-open-symbolic");
                open.add_css_class("flat");
                open.set_tooltip_text(Some("Open"));
                let p = path.clone();
                open.connect_clicked(move |_| cmd::spawn(&["xdg-open", &p]));
                let control = widgets::hbox(10);
                control.append(&widgets::label(&bytes_text(size), "dim"));
                control.append(&bar);
                control.append(&open);
                l2.append(&widgets::row(&name, &glib_escape(&path), Some(control.upcast_ref())));
            }
        },
    );
    widgets::keywords("disk usage space largest biggest folders du");

    let g = page.group("Clean up");
    g.note("Each one says how much it frees before you click.");
    crate::platform::system_cleanup(&g);
    let trash = paths::home().join(".local/share/Trash");
    let t2 = trash.clone();
    cleanup_row(
        &g,
        "Trash",
        "Files you deleted in Files.",
        move || cleanup::folder_size(&t2),
        move |b| {
            let t = trash.clone();
            cmd::background(
                move || {
                    for sub in ["files", "info", "expunged"] {
                        let dir = t.join(sub);
                        let _ = std::fs::remove_dir_all(&dir);
                        let _ = std::fs::create_dir_all(&dir);
                    }
                },
                move |_| {
                    window::toast("Trash emptied.");
                    b.set_sensitive(false);
                },
            );
        },
    );
    let thumbs = paths::home().join(".cache/thumbnails");
    let th2 = thumbs.clone();
    cleanup_row(
        &g,
        "Thumbnail cache",
        "Small pictures of your files; they're made again as needed.",
        move || cleanup::folder_size(&th2),
        move |b| {
            let t = thumbs.clone();
            cmd::background(
                move || std::fs::remove_dir_all(&t),
                move |_| {
                    window::toast("Thumbnails cleared.");
                    b.set_sensitive(false);
                },
            );
        },
    );
    let (r, _) =
        widgets::button_row("Old snapshots", "Snapshots of the system, on Boot &amp; Sign-in.", "Open", |_| window::navigate("boot"));
    g.add(&r);
    widgets::keywords("clean cleanup free space cache pacman orphans journal logs trash thumbnails snapshots");
}

fn glib_escape(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}

/// Omarchy's system cleanup: pacman's cache, orphaned packages and old logs, through the
/// root helper.
pub(crate) fn omarchy_rows(g: &widgets::Group) {
    if let (_, Some(notice)) = super::accounts::helper_notice("disks", "cleaning the package cache and logs") {
        g.top(&notice);
    }
    cleanup_row(
        g,
        "Package cache",
        "Downloaded copies of packages: keeps the newest of each, for going back after a bad update.",
        cleanup::package_cache,
        |b| helper(&["clean-pkg-cache"], "Package cache cleaned.", b),
    );
    cleanup_row(
        g,
        "Packages nothing needs",
        "Installed to support other packages that are gone now.",
        || cleanup::orphans().1,
        |b| {
            let names = cleanup::orphans().0;
            if names.is_empty() {
                window::toast("There are none now.");
                return;
            }
            let shown = names.iter().take(12).cloned().collect::<Vec<_>>().join(", ");
            let more = if names.len() > 12 { format!(" and {} more", names.len() - 12) } else { String::new() };
            let b2 = b.clone();
            window::toast_action(&format!("Remove {shown}{more}?"), "Remove", move || {
                helper(&["remove-orphans"], "Removed.", b2.clone())
            });
            b.set_sensitive(true);
        },
    );
    cleanup_row(
        g,
        "Old system logs",
        "Keeps the newest 200 MB of the system journal.",
        || cleanup::journal_extra(200 << 20),
        |b| helper(&["vacuum-journal", "200M"], "Old logs removed.", b),
    );
}
