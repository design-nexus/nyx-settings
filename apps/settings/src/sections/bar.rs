//! Bar & Notifications (Omarchy's bar): where the bar sits and how it looks, beside
//! your screen with the bar drawn from its real layout and its sections as chips to
//! drag; notifications under the bar settings.

use crate::backend::shell;
use crate::sketch::{self, R, Sketch, mix, rounded, set};
use crate::widgets::{self, Page};
use crate::{cmd, paths, window};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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

/// Key caps under a row's title, for its shortcut.
pub(crate) fn add_keys(row: &gtk::Box, keys: &[&str]) {
    let mut c = row.first_child();
    while let Some(child) = c {
        if child.has_css_class("row-text")
            && let Some(text) = child.downcast_ref::<gtk::Box>()
        {
            let caps = nyx_ui::widgets::key_caps(keys);
            caps.add_css_class("ob-keys");
            caps.set_halign(gtk::Align::Start);
            text.append(&caps);
            return;
        }
        c = child.next_sibling();
    }
}

/// The screen behind the bar: the wallpaper and two windows beside the bar's edge.
fn draw_screen(cr: &gtk::cairo::Context, p: &sketch::Pal, w: f64, h: f64, position: &str, thick: f64) {
    let r = R::new(0.5, 0.5, w - 1.0, h - 1.0);
    cr.save().ok();
    rounded(cr, r.x, r.y, r.w, r.h, 10.0);
    cr.clip();
    let g = gtk::cairo::LinearGradient::new(0.0, 0.0, w, h);
    let (a, b) = (mix(p.bg, p.accent, 0.16), mix(p.bg, p.danger, 0.22));
    g.add_color_stop_rgb(0.0, a.0, a.1, a.2);
    g.add_color_stop_rgb(1.0, b.0, b.1, b.2);
    let _ = cr.set_source(&g);
    let _ = cr.paint();
    let area = match position {
        "bottom" => R::new(r.x, r.y, r.w, r.h - thick),
        "left" => R::new(r.x + thick, r.y, r.w - thick, r.h),
        "right" => R::new(r.x, r.y, r.w - thick, r.h),
        _ => R::new(r.x, r.y + thick, r.w, r.h - thick),
    }
    .inset(8.0);
    let (left, right) = area.hsplit(0.5, 8.0);
    for (i, win) in [left, right].into_iter().enumerate() {
        rounded(cr, win.x, win.y, win.w, win.h, 6.0);
        set(cr, mix(p.surface, p.text, 0.05), 0.95);
        let _ = cr.fill_preserve();
        if i == 0 {
            set(cr, p.accent, 0.9);
            cr.set_line_width(1.5);
        } else {
            set(cr, p.dim, 0.4);
            cr.set_line_width(1.0);
        }
        let _ = cr.stroke();
        let x = win.x + 14.0;
        for (j, (wf, a)) in [(0.45, 0.6), (0.6, 0.3), (0.35, 0.3)].iter().enumerate() {
            set(cr, p.text, *a);
            rounded(cr, x, win.y + 16.0 + j as f64 * 11.0, (win.w - 28.0) * wf, 4.0, 2.0);
            let _ = cr.fill();
        }
        for j in 0..2 {
            set(cr, mix(p.surface, p.accent, 0.25), 0.8);
            rounded(cr, x, win.y + 60.0 + j as f64 * 24.0, win.w - 28.0, 16.0, 3.0);
            let _ = cr.fill();
        }
    }
    cr.restore().ok();
}

/// Put the bar on its edge of the screen.
fn place(strip: &gtk::Box, position: &str) {
    let (h, v) = match position {
        "bottom" => (gtk::Align::Fill, gtk::Align::End),
        "left" => (gtk::Align::Start, gtk::Align::Fill),
        "right" => (gtk::Align::End, gtk::Align::Fill),
        _ => (gtk::Align::Fill, gtk::Align::Start),
    };
    strip.set_halign(h);
    strip.set_valign(v);
}

/// Closes up the gaps between the picture's widgets until the bar fits `width`, as the
/// real bar would squeeze them, rather than cutting off its right end.
fn fit_strip(strip: &gtk::Box, width: i32) {
    const GAP: i32 = 7;
    if width <= 0 || strip.orientation() == gtk::Orientation::Vertical {
        return;
    }
    let parts: Vec<gtk::Box> =
        std::iter::successors(strip.first_child(), |c| c.next_sibling()).filter_map(|c| c.downcast::<gtk::Box>().ok()).collect();
    for p in &parts {
        p.set_spacing(GAP);
    }
    let (_, natural, _, _) = strip.measure(gtk::Orientation::Horizontal, -1);
    let gaps: i32 = parts.iter().map(|p| (std::iter::successors(p.first_child(), |c| c.next_sibling()).count() as i32 - 1).max(0)).sum();
    if natural > width && gaps > 0 {
        let gap = (GAP - (natural - width + gaps - 1) / gaps).max(2);
        for p in &parts {
            p.set_spacing(gap);
        }
    }
}

/// Where the bar sits, as four small screens.
fn position_cards(position: &str, on_change: impl Fn(String) + 'static) -> gtk::Box {
    let row = widgets::hbox(8);
    row.set_homogeneous(true);
    row.add_css_class("ob-positions");
    let on_change = Rc::new(on_change);
    let mut first: Option<gtk::ToggleButton> = None;
    for (id, name) in [("top", "Top"), ("bottom", "Bottom"), ("left", "Left"), ("right", "Right")] {
        let b = gtk::ToggleButton::new();
        b.add_css_class("ob-position");
        let v = widgets::vbox(4);
        let a = sketch::area(Sketch::Bar(id), 64, 40, Rc::new(Cell::new(true)));
        a.set_halign(gtk::Align::Center);
        v.append(&a);
        let l = gtk::Label::new(Some(name));
        l.add_css_class("ob-position-name");
        v.append(&l);
        b.set_child(Some(&v));
        match &first {
            Some(g) => b.set_group(Some(g)),
            None => first = Some(b.clone()),
        }
        b.set_active(id == position);
        let f = on_change.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                f(id.to_string());
            }
        });
        row.append(&b);
    }
    row
}

pub fn build(page: &Page) {
    let position = shell::get(&["bar", "position"]).and_then(|v| v.as_str().map(String::from)).unwrap_or("top".into());
    let transparent = shell::get(&["bar", "transparent"]).and_then(|v| v.as_bool()).unwrap_or(false);

    // The preview: your screen with the bar, then its sections as chips.
    let pv = page.split();
    // Two clicks: this throws away the whole layout.
    let restore = widgets::confirm_button("Restore defaults", "Click again to restore", |_| {
        run(&["omarchy-bar", "defaults"]);
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(600), || window::rebuild("bar"));
    });
    restore.add_css_class("flat");
    restore.set_tooltip_text(Some(
        "Restore Omarchy's bar: puts Omarchy's own widgets back in their places and takes every other plugin widget off the bar. The bar's position and transparency reset too.",
    ));
    let head = pv.head("Your bar", "Drag a widget to reorder it or move it to another section; click one for more.", &[restore.upcast_ref()]);
    if let Some(title) = head.first_child().and_then(|t| t.first_child()).and_downcast::<gtk::Label>() {
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    }

    let view = super::barlayout::view(&position);
    let pos = Rc::new(RefCell::new(position.clone()));
    let screen = gtk::DrawingArea::new();
    screen.set_hexpand(true);
    {
        let pos = pos.clone();
        screen.set_draw_func(move |_, cr, w, h| draw_screen(cr, &sketch::pal(), w as f64, h as f64, &pos.borrow(), 30.0));
    }
    // A picture of the top of the screen: it takes the height left over, from 170.
    screen.set_content_height(170);
    screen.set_vexpand(true);
    let stage = gtk::Overlay::new();
    stage.add_css_class("ob-stage");
    stage.set_child(Some(&screen));
    stage.add_overlay(&view.strip);
    stage.set_overflow(gtk::Overflow::Hidden);
    place(&view.strip, &position);
    if transparent {
        view.strip.add_css_class("transparent");
    }
    {
        let strip = view.strip.clone();
        screen.connect_resize(move |_, w, _| {
            let strip = strip.clone();
            gtk::glib::idle_add_local_once(move || fit_strip(&strip, w));
        });
    }
    pv.add(&stage);
    pv.add(&view.zones);
    pv.add(&view.add);
    widgets::keywords("widgets reorder order move arrange layout left center right section plugins drag add remove spacer restore default bar");
    {
        let (screen, strip) = (screen.clone(), view.strip.clone());
        view.on_change(move || {
            screen.queue_draw();
            fit_strip(&strip, screen.width());
        });
    }
    let view = Rc::new(view);

    // Text size: the bar in the picture takes it too.
    let size = shell_text_size().unwrap_or(12.0);
    let text_css = gtk::CssProvider::new();
    #[allow(deprecated)]
    view.strip.style_context().add_provider(&text_css, gtk::STYLE_PROVIDER_PRIORITY_USER);
    let show_size = {
        let text_css = text_css.clone();
        move |pt: f64| text_css.load_from_string(&format!(".ob-strip {{ font-size: {}pt; }}", (pt * 0.75).round()))
    };
    show_size(size);

    // ----- The controls -----
    let g = page.plain_group("Position");
    let cards = {
        let (view, screen, pos, strip) = (view.clone(), screen.clone(), pos.clone(), view.strip.clone());
        position_cards(&position, move |p| {
            run(&["omarchy-bar", "position", &p]);
            *pos.borrow_mut() = p.clone();
            place(&strip, &p);
            view.set_position(&p);
            screen.queue_draw();
        })
    };
    let r = widgets::stacked_row("", "", cards.upcast_ref());
    r.add_css_class("bare");
    widgets::keywords("position edge top bottom left right where");
    g.add(&r);

    let g = page.group("");
    let strip = view.strip.clone();
    let (r, _) = widgets::switch_row("Transparent background", "Let the wallpaper show through", transparent, move |on| {
        run(&["omarchy-bar", "transparent", if on { "true" } else { "false" }]);
        if on {
            strip.add_css_class("transparent");
        } else {
            strip.remove_css_class("transparent");
        }
    });
    widgets::keywords("see-through");
    g.add(&r);
    let (r, s) = widgets::slider_row("Text size", "The bar, menus and panels", (9.0, 18.0, 1.0), size, 0, " pt", move |v| {
        show_size(v);
        if let Err(e) = set_shell_text_size(v.round() as i64) {
            window::toast(&format!("Couldn't save text size: {e}"));
        }
    });
    s.scale.set_width_request(110);
    widgets::keywords("font size bigger smaller scale shell text");
    g.add(&r);
    let (r, _) = widgets::button_row("Show or hide the bar", "", "Toggle", |_| run(&["omarchy-toggle-bar"]));
    add_keys(&r, &["Super", "Shift", "Space"]);
    g.add(&r);

    notifications(page);
}

// ----- Notifications -----

fn notifications(page: &Page) {
    use crate::backend::notify;

    let g = page.group("Notifications");
    // `syncing` stops the switch echoing a change it's only mirroring.
    let syncing = Rc::new(Cell::new(false));
    let sw = gtk::Switch::new();
    sw.set_active(notify::dnd());
    {
        let syncing = syncing.clone();
        sw.connect_active_notify(move |s| {
            if syncing.get() {
                return;
            }
            let on = s.is_active();
            cmd::background(
                move || notify::set_dnd(on),
                |r| {
                    if let Err(e) = r {
                        window::toast(&format!("Couldn't change Do not disturb: {e}"));
                    }
                },
            );
        });
    }
    let r = widgets::icon_row("notifications-disabled-symbolic", "Do not disturb", "", Some(sw.upcast_ref()));
    add_keys(&r, &["Super", "Ctrl", ","]);
    widgets::keywords("dnd silence mute notifications quiet until you turn it back on");
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

    let dismiss = widgets::command_button("Dismiss all", &["omarchy-shell", "notifications", "dismissAll"]);
    g.add(&widgets::icon_row("preferences-system-notifications-symbolic", "Showing now", "Clear the ones on screen", Some(dismiss.upcast_ref())));

    let count = notify::history().len();
    // The list opens one level down; clearing it there updates the count here.
    let slot: Rc<RefCell<Option<gtk::Label>>> = Rc::default();
    let (r, value) = {
        let slot = slot.clone();
        widgets::nav_row(Some("document-open-recent-symbolic"), "Recent notifications", "", &count.to_string(), move || {
            let Some(value) = slot.borrow().clone() else { return };
            let d = widgets::subpage("Recent notifications", 560, move |p| history_page(p, value));
            widgets::forget_rows(&d);
        })
    };
    *slot.borrow_mut() = Some(value);
    widgets::keywords("notification history centre center recent clear forget");
    g.add(&r);
}

fn history_page(page: &Page, count: gtk::Label) {
    use crate::backend::notify;
    let g = page.group("Recent notifications");
    g.note("Notifications you've had, newest first.");
    let list = widgets::vbox(0);
    g.add(&list);
    let fill: Rc<dyn Fn()> = {
        let list = list.clone();
        Rc::new(move || {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let entries = notify::history();
            count.set_text(&entries.len().to_string());
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
    let g = page.group("");
    g.add(&widgets::row("Forget the history", "", Some(clear.upcast_ref())));
}
