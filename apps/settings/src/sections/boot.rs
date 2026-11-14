//! The boot screen, login screen, hibernation and snapshots. Omarchy's own
//! scripts do the work (they ask for sudo themselves), so they run in a terminal.
//!
//! The picture is the start-up in order: the boot menu, the start-up screen, the login
//! screen and the desktop, each as it will look, changed by the settings beside it.

use crate::sections::accounts::{head, pill, roomy, terminal};
use crate::widgets::{self, Page};
use crate::{cmd, sketch, window};
use gtk::gdk::prelude::GdkCairoContextExt;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Names a script printed, one per line.
fn lines(program: &str) -> Vec<String> {
    cmd::output(&[program]).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

/// Re-read what's set once a terminal is likely done, even if it was cancelled.
fn in_terminal(command: &str) {
    terminal(command);
    gtk::glib::timeout_add_seconds_local_once(30, || window::rebuild_if_built("boot"));
}

/// Whether the computer starts through the boot menu (`Some(true)`) or straight into
/// Omarchy (`Some(false)`), from the firmware's boot order.
fn boot_menu_shown() -> Option<bool> {
    let text = cmd::output(&["efibootmgr"])?;
    let first = text.lines().find_map(|l| l.strip_prefix("BootOrder:"))?.split(',').next()?.trim().to_string();
    let label = text.lines().find_map(|l| l.strip_prefix(&format!("Boot{first}")))?.trim_start_matches('*').trim().to_lowercase();
    if label.starts_with("limine") {
        Some(true)
    } else if label.starts_with("omarchy") {
        Some(false)
    } else {
        None
    }
}

/// The person the login screen signs in by itself, from SDDM's settings.
fn autologin_user() -> Option<String> {
    let mut files: Vec<_> = std::fs::read_dir("/etc/sddm.conf.d").into_iter().flatten().flatten().map(|e| e.path()).collect();
    files.sort();
    files.insert(0, "/etc/sddm.conf".into());
    let mut user = None;
    for f in files {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        let mut in_section = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_section = line == "[Autologin]";
            } else if in_section && let Some(v) = line.strip_prefix("User=") {
                user = Some(v.trim().to_string()).filter(|v| !v.is_empty());
            }
        }
    }
    user
}

/// Whether the disk is encrypted, so the start-up screen asks for its password.
fn encrypted() -> bool {
    cmd::output(&["lsblk", "-rno", "TYPE"]).is_some_and(|t| t.lines().any(|l| l.trim() == "crypt"))
}

/// A theme's folder (`omarchy-theme-dir`).
fn theme_dir(theme: &str) -> Option<std::path::PathBuf> {
    cmd::output(&["omarchy-theme-dir", theme]).map(std::path::PathBuf::from).filter(|p| p.is_dir())
}

/// A theme's background, text and accent colours, from its `colors.toml`.
fn theme_colours(theme: &str) -> Option<(sketch::Rgb, sketch::Rgb, sketch::Rgb)> {
    let text = std::fs::read_to_string(theme_dir(theme)?.join("colors.toml")).ok()?;
    let get = |key: &str| {
        text.lines().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == key).then(|| crate::fangraph::rgb(v.trim().trim_matches('"')))
        })
    };
    let bg = get("background")?;
    let fg = get("foreground")?;
    Some((bg, fg, get("accent").unwrap_or(fg)))
}

/// The picture of a theme's start-up screen that Omarchy ships with it.
fn unlock_preview(theme: &str) -> Option<std::path::PathBuf> {
    let p = if theme == "default" {
        std::path::PathBuf::from("/usr/share/plymouth/themes/omarchy/preview-unlock.png")
    } else {
        theme_dir(theme)?.join("preview-unlock.png")
    };
    p.is_file().then_some(p)
}

fn nice(theme: &str) -> String {
    if theme == "default" {
        return "Omarchy".into();
    }
    theme
        .split(['-', '_'])
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn build(page: &Page) {
    let styled = cmd::present("omarchy-plymouth-set-by-theme");
    let theme = if styled { cmd::output(&["omarchy-plymouth-current"]).unwrap_or_else(|| "default".into()) } else { String::new() };
    let menu = boot_menu_shown();
    let auto = autologin_user();
    let hib_setup = cmd::present("omarchy-hibernation-setup");
    let hibernates = cmd::present("omarchy-hibernation-available") && cmd::run(&["omarchy-hibernation-available"]).is_ok();

    preview(page, &theme, menu, auto.as_deref(), hibernates);
    start_up(page, menu);
    if styled {
        look(page, &theme);
    }
    signing_in(page, auto.as_deref(), hib_setup, hibernates);
    repairs(page);
}

/// The steps from power-on to the desktop, each drawn as it will look.
fn preview(page: &Page, theme: &str, menu: Option<bool>, auto: Option<&str>, hibernates: bool) {
    let pv = page.split();
    let content = roomy(&pv);
    content.append(&head("From power-on to your desktop", "Each step as it will look.", &[]));
    let steps = widgets::vbox(10);
    steps.add_css_class("bts-steps");
    let nums: Rc<RefCell<Vec<gtk::Widget>>> = Rc::default();
    let colours = theme_colours(if theme == "default" || theme.is_empty() { "" } else { theme });

    // 1. The boot menu.
    let (desc, tag, skipped) = match menu {
        Some(true) => ("Pick a snapshot here to go back to how the system was, or start another system.", Some("Shown"), false),
        Some(false) => ("Skipped: the computer starts Omarchy straight away.", Some("Skipped"), true),
        None => ("Where you can pick a snapshot to go back to, or another system.", None, false),
    };
    let menu_art = widgets::drawing(96, move |cr, p, section, w, h| draw_menu(cr, p, section, w, h, colours));
    steps.append(&step(&nums, "1", Some(menu_art.upcast()), "Boot menu", desc, tag, skipped));

    // 2. The start-up screen.
    if !theme.is_empty() {
        let art = unlock_preview(theme)
            .and_then(|path| gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, 340, 192, true).ok())
            .map(|picture| {
                let area = gtk::DrawingArea::new();
                area.set_draw_func(move |_, cr, w, h| {
                    let (pw, ph) = (picture.width() as f64, picture.height() as f64);
                    let scale = (w as f64 / pw).max(h as f64 / ph);
                    cr.translate((w as f64 - pw * scale) / 2.0, (h as f64 - ph * scale) / 2.0);
                    cr.scale(scale, scale);
                    cr.set_source_pixbuf(&picture, 0.0, 0.0);
                    let _ = cr.paint();
                });
                area.upcast::<gtk::Widget>()
            });
        let desc = if encrypted() {
            format!("{}'s colours and logo, while it asks for the disk password.", nice(theme))
        } else {
            format!("{}'s colours and logo while the computer starts.", nice(theme))
        };
        steps.append(&step(&nums, "2", art, "Start-up screen", &desc, Some(&nice(theme)), false));
    }

    // 3. The login screen.
    let face = super::sign_in::login_face();
    let me = crate::backend::accounts::people().into_iter().find(|u| Some(u.name.as_str()) == auto).map(|u| if u.full_name.is_empty() { u.name } else { u.full_name });
    let desc = match (auto, face) {
        (Some(user), _) => format!("Skipped: {} signs in automatically. It still shows after you log out.", me.clone().unwrap_or(user.to_string())),
        (None, true) => "Leave the password empty and press Enter to use your face.".to_string(),
        (None, false) => "Choose who's signing in and type the password.".to_string(),
    };
    let who = me.unwrap_or_else(crate::backend::accounts::me);
    let login_art = widgets::drawing(96, move |cr, p, _, w, h| draw_login(cr, p, w, h, colours, &who));
    let n = if theme.is_empty() { "2" } else { "3" };
    steps.append(&step(&nums, n, Some(login_art.upcast()), "Login screen", &desc, Some(if auto.is_some() { "Skipped" } else { "Shown" }), auto.is_some()));

    // 4. The desktop.
    let desc = if hibernates {
        "After hibernating, everything opens where you left it."
    } else {
        "Your apps start fresh; set up hibernation to come back to where you left off."
    };
    steps.append(&step(&nums, "", None, "Your desktop", desc, None, false));

    // A line through the step numbers.
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&steps));
    let own: Rc<RefCell<gtk::glib::WeakRef<gtk::DrawingArea>>> = Rc::default();
    let own2 = own.clone();
    let rail = widgets::drawing(0, move |cr, p, _, _, _| {
        let Some(area) = own2.borrow().upgrade() else { return };
        let points: Vec<(f64, f64)> = nums
            .borrow()
            .iter()
            .filter_map(|n| n.compute_bounds(&area))
            .map(|b| (b.x() as f64 + b.width() as f64 / 2.0, b.y() as f64 + b.height() as f64 / 2.0))
            .collect();
        for pair in points.windows(2) {
            sketch::set(cr, p.dim, 0.35);
            cr.set_line_width(1.5);
            cr.move_to(pair[0].0, pair[0].1 + 16.0);
            cr.line_to(pair[1].0, pair[1].1 - 16.0);
            let _ = cr.stroke();
        }
    });
    *own.borrow_mut() = rail.downgrade();
    rail.set_can_target(false);
    overlay.add_overlay(&rail);
    overlay.set_measure_overlay(&rail, false);
    content.append(&overlay);
    pv.foot("dialog-information-symbolic", "Changes here open a terminal, which asks for your password.");
}

/// One step: its number, a small picture, what happens there, and a tag.
fn step(
    nums: &Rc<RefCell<Vec<gtk::Widget>>>,
    n: &str,
    art: Option<gtk::Widget>,
    title: &str,
    desc: &str,
    tag: Option<&str>,
    skipped: bool,
) -> gtk::Box {
    let card = widgets::hbox(16);
    card.add_css_class("preview-card");
    card.add_css_class("bts-step");
    if skipped {
        card.add_css_class("skipped");
    }
    let num = widgets::hbox(0);
    num.add_css_class("bts-num");
    if !n.is_empty() {
        in_section_colour(&num);
    }
    if n.is_empty() {
        num.add_css_class("done");
        num.append(&gtk::Image::from_icon_name("object-select-symbolic"));
    } else {
        let l = gtk::Label::new(Some(n));
        l.set_halign(gtk::Align::Center);
        l.set_width_chars(2);
        num.append(&l);
    }
    num.set_valign(gtk::Align::Center);
    num.set_halign(gtk::Align::Center);
    num.set_hexpand(false);
    card.append(&num);
    nums.borrow_mut().push(num.upcast());
    if let Some(art) = art {
        let frame = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        frame.add_css_class("bts-art");
        frame.set_overflow(gtk::Overflow::Hidden);
        frame.set_size_request(170, 96);
        frame.set_valign(gtk::Align::Center);
        frame.set_hexpand(false);
        art.set_hexpand(false);
        art.set_size_request(170, 96);
        frame.append(&art);
        card.append(&frame);
    }
    let text = widgets::vbox(3);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let line = widgets::hbox(8);
    let t = widgets::label(title, "bts-title");
    t.set_hexpand(true);
    line.append(&t);
    if let Some(tag) = tag {
        let p = pill(tag, if skipped { "muted" } else { "bts-pill" });
        if !skipped {
            in_section_colour(&p);
        }
        line.append(&p);
    }
    text.append(&line);
    let d = widgets::label(desc, "bts-desc");
    d.set_wrap(true);
    text.append(&d);
    card.append(&text);
    card
}

/// Gives `w` the colour of the page it's on (the theme colours `.section-<key>`).
fn in_section_colour(w: &impl IsA<gtk::Widget>) {
    w.connect_realize(|w| {
        let mut up = w.parent();
        while let Some(a) = up {
            if let Some(key) = a.css_classes().iter().find_map(|c| c.strip_prefix("colour-").map(str::to_string)) {
                w.add_css_class(&format!("section-{key}"));
                return;
            }
            up = a.parent();
        }
    });
}

/// Limine's menu, small: the menu's title and Omarchy picked.
fn draw_menu(cr: &gtk::cairo::Context, p: &sketch::Pal, section: sketch::Rgb, w: f64, h: f64, colours: Option<(sketch::Rgb, sketch::Rgb, sketch::Rgb)>) {
    let (bg, fg, accent) = colours.unwrap_or((sketch::mix(p.bg, (0.0, 0.0, 0.0), 0.4), p.text, section));
    sketch::set(cr, bg, 1.0);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();
    cr.select_font_face("monospace", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
    cr.set_font_size(8.0);
    sketch::set(cr, accent, 1.0);
    cr.move_to(16.0, 18.0);
    let _ = cr.show_text("Limine");
    let x = 16.0;
    let bw = w - 32.0;
    sketch::set(cr, accent, 0.9);
    cr.rectangle(x - 2.0, 26.0, bw, 11.0);
    let _ = cr.fill();
    cr.select_font_face("monospace", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
    sketch::set(cr, bg, 1.0);
    cr.move_to(x + 2.0, 34.5);
    let _ = cr.show_text("Omarchy");
    for (i, len) in [0.62, 0.58, 0.4].iter().enumerate() {
        sketch::set(cr, fg, 0.35);
        cr.rectangle(x + 2.0, 44.0 + i as f64 * 12.0, (bw - 8.0) * len, 4.0);
        let _ = cr.fill();
    }
}

/// The login screen, small: who signs in and the password field, in the theme's colours.
fn draw_login(cr: &gtk::cairo::Context, p: &sketch::Pal, w: f64, h: f64, colours: Option<(sketch::Rgb, sketch::Rgb, sketch::Rgb)>, who: &str) {
    let (bg, fg, accent) = colours.unwrap_or((sketch::mix(p.bg, (0.0, 0.0, 0.0), 0.4), p.text, p.accent));
    sketch::set(cr, bg, 1.0);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();
    let cx = w / 2.0;
    sketch::set(cr, super::accounts::swatch_for(who), 1.0);
    cr.arc(cx, 30.0, 13.0, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
    cr.set_font_size(8.5);
    if let Ok(ext) = cr.text_extents(who) {
        sketch::set(cr, fg, 0.95);
        cr.move_to(cx - ext.width() / 2.0 - ext.x_bearing(), 56.0);
        let _ = cr.show_text(who);
    }
    sketch::set(cr, fg, 0.25);
    cr.set_line_width(1.0);
    sketch::rounded(cr, cx - 40.0, 64.0, 80.0, 14.0, 4.0);
    let _ = cr.stroke();
    sketch::set(cr, accent, 0.9);
    cr.arc(cx + 33.0, 71.0, 3.0, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
}

fn start_up(page: &Page, menu: Option<bool>) {
    let any = ["omarchy-snapshot", "omarchy-setup-direct-boot"].iter().any(|c| cmd::present(c));
    if !any {
        return;
    }
    let g = page.group("Start-up");
    if cmd::present("omarchy-setup-direct-boot") {
        let controls = widgets::hbox(10);
        if let Some(shown) = menu {
            controls.append(&widgets::label(if shown { "Shown" } else { "Skipped" }, "row-value"));
        }
        let b = gtk::Button::with_label("Change…");
        b.add_css_class("acc-small");
        b.set_tooltip_text(Some("Adds or removes a firmware boot entry so the computer starts Omarchy without the boot menu"));
        b.connect_clicked(|_| in_terminal("omarchy-setup-direct-boot"));
        controls.append(&b);
        let r = widgets::icon_row("view-list-symbolic", "Boot menu", "Skip it to start right away", Some(controls.upcast_ref()));
        widgets::keywords("boot directly uki efi limine bootloader menu skip direct boot");
        g.add(&r);
    }
    if cmd::present("omarchy-snapshot") {
        let (r, _) = widgets::nav_row(
            Some("document-open-recent-symbolic"),
            "Restore a snapshot",
            "Go back to how the system was. Make one first from About.",
            "",
            || in_terminal("omarchy-snapshot restore"),
        );
        widgets::keywords("snapper rollback undo btrfs backup");
        g.add(&r);
    }
}

fn look(page: &Page, current: &str) {
    let themes = lines("omarchy-plymouth-list");
    if themes.is_empty() {
        return;
    }
    let g = page.group("Look");
    let mut options: Vec<(String, String)> = themes.iter().map(|t| (t.clone(), nice(t))).collect();
    if current == "default" || !themes.iter().any(|t| t == current) {
        options.insert(0, ("default".into(), "Omarchy".into()));
    }
    let selected = if themes.iter().any(|t| t == current) { current.to_string() } else { "default".into() };
    let dd = widgets::dropdown(&options, &selected);
    dd.add_css_class("dfa-dropdown");
    let (opts, themes, current) = (options.clone(), themes.clone(), current.to_string());
    dd.connect_selected_notify(move |d| {
        let Some((t, _)) = opts.get(d.selected() as usize) else { return };
        if *t == current {
            return;
        }
        if t == "default" {
            in_terminal("omarchy-plymouth-reset");
        } else if themes.contains(t) {
            in_terminal(&format!("omarchy-plymouth-set-by-theme {}", cmd::shell_quote(t)));
        }
    });
    let r = widgets::icon_row("preferences-desktop-appearance-symbolic", "Style from a theme", "For start-up and login", Some(dd.upcast_ref()));
    r.set_tooltip_text(Some("Colours and logo for the boot and login screens. A terminal opens and asks for your password."));
    widgets::keywords("plymouth sddm splash boot screen login greeter theme colors");
    g.add(&r);
}

fn signing_in(page: &Page, auto: Option<&str>, hib_setup: bool, hibernates: bool) {
    let g = page.group("Signing in");
    let value = widgets::label(if auto.is_some() { "On" } else { "Off" }, "row-value");
    let r = widgets::icon_row(
        "avatar-default-symbolic",
        "Sign in automatically",
        &match auto {
            Some(user) if encrypted() => format!("As {user}, once the disk is unlocked"),
            Some(user) => format!("As {user}"),
            None => "The login screen asks who's signing in".to_string(),
        },
        Some(value.upcast_ref()),
    );
    widgets::keywords("autologin automatic login sddm");
    g.add(&r);

    if !hib_setup {
        let r = widgets::icon_row("weather-clear-night-symbolic", "Hibernation", "Not available on this system", None);
        widgets::keywords("hibernate swap resume suspend sleep disk");
        g.add(&r);
        return;
    }
    let sw = gtk::Switch::new();
    sw.set_active(hibernates);
    sw.connect_active_notify(move |s| {
        if s.is_active() != hibernates {
            in_terminal(if hibernates { "omarchy-hibernation-remove" } else { "omarchy-hibernation-setup" });
        }
    });
    let r = widgets::icon_row(
        "weather-clear-night-symbolic",
        "Hibernation",
        if hibernates { "Saves everything to disk, then switches off" } else { "Setting it up makes a swap file and changes the boot settings" },
        Some(sw.upcast_ref()),
    );
    widgets::keywords("hibernate swap resume suspend sleep disk");
    g.add(&r);
}

fn repairs(page: &Page) {
    let any = ["omarchy-plymouth-reset", "omarchy-refresh-sddm", "omarchy-refresh-limine"].iter().any(|c| cmd::present(c));
    if !any {
        return;
    }
    let g = page.group("");
    let (r, _) = widgets::nav_row(
        Some("applications-engineering-symbolic"),
        "If something goes wrong",
        "Repairs, and Omarchy's own look",
        "",
        || {
            widgets::subpage("If something goes wrong", 560, |p| {
                let g = p.group("If something goes wrong");
                g.note("Each one opens a terminal and asks for your password.");
                if cmd::present("omarchy-plymouth-reset") {
                    let (r, _) = widgets::button_row(
                        "Back to default",
                        "Restore Omarchy's own boot and login screens.",
                        "Restore…",
                        |_| in_terminal("omarchy-plymouth-reset"),
                    );
                    g.add(&r);
                }
                if cmd::present("omarchy-refresh-sddm") {
                    let (r, _) =
                        widgets::button_row("Repair login screen", "Reinstall the login screen if it looks broken.", "Repair…", |_| {
                            in_terminal("omarchy-refresh-sddm")
                        });
                    g.add(&r);
                }
                if cmd::present("omarchy-refresh-limine") {
                    let (r, _) = widgets::button_row("Repair the boot menu", "Rebuild the boot menu's settings.", "Repair…", |_| {
                        in_terminal("omarchy-refresh-limine")
                    });
                    g.add(&r);
                }
            });
        },
    );
    widgets::keywords("plymouth sddm reset original refresh fix login limine bootloader repair grub");
    g.add(&r);
}
