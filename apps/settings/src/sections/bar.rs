use crate::backend::shell;
use crate::widgets::{self, Page};
use crate::{cmd, paths, window};
use gtk::prelude::*;

fn run(args: &[&str]) {
    let label = args.join(" ");
    cmd::run_async(args, move |r| {
        if let Err(e) = r {
            window::toast(&format!("{label}: {e}"));
        }
    });
}

fn shell_text_size() -> Option<f64> {
    let text = std::fs::read_to_string(paths::omarchy_config().join("shell.toml")).ok()?;
    let t: toml::Table = toml::from_str(&text).ok()?;
    t.get("font")?.get("base-size")?.as_integer().map(|n| n as f64)
}

fn set_shell_text_size(size: i64) -> anyhow::Result<()> {
    let path = paths::omarchy_config().join("shell.toml");
    let mut t: toml::Table = std::fs::read_to_string(&path).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default();
    let font = t.entry("font").or_insert_with(|| toml::Value::Table(Default::default()));
    if let Some(f) = font.as_table_mut() {
        f.insert("base-size".into(), toml::Value::Integer(size));
    }
    cmd::atomic_write(&path, &toml::to_string(&t)?)
}

pub fn build(page: &Page) {
    let g = page.group("Bar");
    let position = shell::get(&["bar", "position"]).and_then(|v| v.as_str().map(String::from)).unwrap_or("top".into());
    use crate::sketch::Sketch;
    let cards = [("top", "Top"), ("bottom", "Bottom"), ("left", "Left"), ("right", "Right")]
        .iter()
        .map(|(id, name)| (id.to_string(), name.to_string(), Sketch::Bar(id)))
        .collect();
    let (r, _, _) = widgets::visual_choice("Position", "Which edge of the screen the bar sits on.", cards, &position, |p| {
        run(&["omarchy-bar", "position", &p])
    });
    g.add(&r);
    let transparent = shell::get(&["bar", "transparent"]).and_then(|v| v.as_bool()).unwrap_or(false);
    let (r, _) = widgets::switch_row("Transparent background", "Let the wallpaper show through the bar.", transparent, |on| {
        run(&["omarchy-bar", "transparent", if on { "true" } else { "false" }]);
    });
    g.add(&r);
    let (r, _) = widgets::button_row("Show or hide the bar", "Also on <tt>Super Shift Space</tt>.", "Toggle", |_| {
        run(&["omarchy-toggle-bar"])
    });
    g.add(&r);

    let g = page.plain_group("Widgets");
    g.note("Drag a widget to reorder it or move it to another section, or use the arrows and the ⋯ menu.");
    let editor = super::barlayout::editor();
    g.add(&editor);
    widgets::keywords("widgets reorder order move arrange layout left center right section plugins drag add remove");
    // Two clicks: this throws away the whole layout.
    let b = widgets::confirm_button("Restore", "Click again to restore", |_| {
        run(&["omarchy-bar", "defaults"]);
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(600), || window::rebuild("bar"));
    });
    b.add_css_class("destructive-action");
    g.add(&widgets::row(
        "Restore Omarchy's default bar",
        "Puts Omarchy's own widgets back in their places and takes every other plugin widget off the bar. \
         The bar's position and transparency reset too.",
        Some(b.upcast_ref()),
    ));

    let g = page.collapsible("Shell text", false);
    let size = shell_text_size().unwrap_or(12.0);
    let (r, _) =
        widgets::slider_row("Text size", "Font size of the bar, menus and panels.", (9.0, 18.0, 1.0), size, 0, " pt", |v| {
            match set_shell_text_size(v.round() as i64) {
                Ok(()) => {}
                Err(e) => window::toast(&format!("Couldn't save text size: {e}")),
            }
        });
    widgets::keywords("font size bigger smaller scale");
    g.add(&r);

    notifications(page);
}

// ----- Notifications -----

fn notifications(page: &Page) {
    use crate::backend::notify;
    use std::cell::Cell;
    use std::rc::Rc;

    let g = page.group("Notifications");
    // `syncing` stops the switch echoing a change it's only mirroring.
    let syncing = Rc::new(Cell::new(false));
    let (r, sw) = widgets::switch_row(
        "Do not disturb",
        "Silence notifications until you turn it back on. Also <tt>Super Ctrl ,</tt>.",
        notify::dnd(),
        {
            let syncing = syncing.clone();
            move |on| {
                if syncing.get() {
                    return;
                }
                cmd::background(
                    move || notify::set_dnd(on),
                    |r| {
                        if let Err(e) = r {
                            window::toast(&format!("Couldn't change Do not disturb: {e}"));
                        }
                    },
                );
            }
        },
    );
    widgets::keywords("dnd silence mute notifications quiet");
    g.add(&r);
    // It can also change from the keyboard shortcut or the bar.
    let weak = sw.downgrade();
    gtk::glib::timeout_add_seconds_local(3, move || {
        let Some(sw) = weak.upgrade() else { return gtk::glib::ControlFlow::Break };
        if sw.is_mapped() {
            let on = notify::dnd();
            if sw.is_active() != on {
                syncing.set(true);
                sw.set_active(on);
                syncing.set(false);
            }
        }
        gtk::glib::ControlFlow::Continue
    });

    let g = page.collapsible("Recent notifications", false);
    g.note("Notifications you've had, newest first.");
    widgets::keywords("notification history centre center recent");
    let list = widgets::vbox(6);
    g.add(&list);
    let fill: Rc<dyn Fn()> = {
        let list = list.clone();
        Rc::new(move || {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let entries = notify::history();
            if entries.is_empty() {
                list.append(&widgets::row("Nothing yet", "Notifications you've had appear here after they close.", None));
                return;
            }
            let now =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
            for e in entries.iter().take(20) {
                let when = crate::units::ago((now - e.timestamp) / 1000);
                let meta = if e.app.is_empty() || e.app == "omarchy-action" { when } else { format!("{} · {when}", e.app) };
                let mut desc = gtk::glib::markup_escape_text(&e.body).to_string();
                if !desc.is_empty() {
                    desc.push('\n');
                }
                desc.push_str(&format!("<small>{}</small>", gtk::glib::markup_escape_text(&meta)));
                let title = if e.summary.is_empty() { e.app.as_str() } else { e.summary.as_str() };
                list.append(&widgets::row(title, &desc, None));
            }
        })
    };
    fill();
    let buttons = widgets::hbox(8);
    buttons.append(&widgets::command_button("Dismiss all", &["omarchy-shell", "notifications", "dismissAll"]));
    let clear = widgets::confirm_button("Clear history", "Click again to clear", move |_| {
        let fill = fill.clone();
        cmd::background(notify::clear_history, move |r| match r {
            // The shell clears in the background; give it a moment.
            Ok(_) => {
                let fill = fill.clone();
                gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || fill());
            }
            Err(e) => window::toast(&format!("Couldn't clear history: {e}")),
        });
    });
    buttons.append(&clear);
    g.add(&widgets::row(
        "On screen and history",
        "Dismiss the notifications showing now, or forget the history.",
        Some(buttons.upcast_ref()),
    ));
}
