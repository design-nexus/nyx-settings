use crate::backend::{appearance, chroma};
use crate::widgets::{self, Page};
use crate::{cmd, paths, prefs, theme};
use gtk::prelude::*;
use gtk::{gdk, gdk_pixbuf, glib};
use std::path::PathBuf;

/// Omarchy theme directory for a display name ("Catppuccin Latte" -> catppuccin-latte).
fn theme_dir(name: &str) -> Option<PathBuf> {
    let slug = name.to_lowercase().replace(' ', "-");
    [paths::omarchy_config().join("themes").join(&slug), PathBuf::from("/usr/share/omarchy/themes").join(&slug)]
        .into_iter()
        .find(|p| p.exists())
}

/// The picture that best shows a theme: its preview, else a screenshot or its first wallpaper.
fn theme_image(dir: &std::path::Path) -> Option<PathBuf> {
    for name in ["preview.png", "theme.png", "preview.jpg", "theme.jpg"] {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    let mut backgrounds: Vec<PathBuf> = std::fs::read_dir(dir.join("backgrounds"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| ["png", "jpg", "jpeg", "webp"].contains(&e.to_lowercase().as_str()))
        })
        .collect();
    backgrounds.sort();
    backgrounds.into_iter().next()
}

/// A strip of the theme's colours, for a theme with no picture at all.
fn colour_strip(dir: &std::path::Path) -> Option<gtk::Box> {
    let text = std::fs::read_to_string(dir.join("colors.toml")).ok()?;
    let table: toml::Table = toml::from_str(&text).ok()?;
    let keys = ["background", "foreground", "accent", "color1", "color2", "color3", "color4", "color5", "color6"];
    let colours: Vec<String> = keys
        .iter()
        .filter_map(|k| table.get(*k).and_then(|v| v.as_str()))
        // Only plain hex colours go into the stylesheet.
        .filter(|c| {
            c.len() <= 9 && c.strip_prefix('#').is_some_and(|h| !h.is_empty() && h.chars().all(|ch| ch.is_ascii_hexdigit()))
        })
        .map(String::from)
        .collect();
    if colours.is_empty() {
        return None;
    }
    let strip = widgets::hbox(0);
    strip.add_css_class("theme-strip");
    strip.set_homogeneous(true);
    for c in colours {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
        b.set_vexpand(true);
        widgets::paint(&b, &c);
        strip.append(&b);
    }
    Some(strip)
}

fn load_thumbnail(picture: &gtk::Picture, path: PathBuf) {
    let picture = picture.clone();
    cmd::background(
        move || {
            let pb = gdk_pixbuf::Pixbuf::from_file_at_scale(&path, 360, 202, true).ok()?;
            Some((pb.read_pixel_bytes(), pb.width(), pb.height(), pb.rowstride(), pb.has_alpha()))
        },
        move |result| {
            if let Some((bytes, w, h, stride, alpha)) = result {
                let format = if alpha { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
                let texture = gdk::MemoryTexture::new(w, h, format, &bytes, stride as usize);
                picture.set_paintable(Some(&texture));
            }
        },
    );
}

pub fn build(page: &Page) {
    if super::themes::omarchy_chooses() {
        omarchy_group(page);
    } else {
        super::themes::system_group(page);
    }
    rest(page);
}

/// The desktop theme where Omarchy chooses it: Omarchy's themes, set with its commands.
fn omarchy_group(page: &Page) {
    let g = page.plain_group("Theme");
    g.note("Applies to the whole desktop: terminal, bar, editor, lock screen and wallpaper.");
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_max_children_per_line(4);
    flow.set_min_children_per_line(3);
    flow.set_row_spacing(10);
    flow.set_column_spacing(10);
    flow.set_homogeneous(true);
    let current = cmd::output(&["omarchy-theme-current"]).unwrap_or_default();
    let names: Vec<String> = cmd::output(&["omarchy-theme-list"])
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    // The first few, and the current one wherever it is; the rest wait behind "Show all".
    const FIRST: usize = 8;
    let current_at = names.iter().position(|n| *n == current).unwrap_or(0);
    let cards: std::rc::Rc<std::cell::RefCell<Vec<(String, gtk::Button)>>> = Default::default();
    let mut hidden: Vec<gtk::Widget> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let card = gtk::Button::new();
        card.add_css_class("theme-card");
        card.set_tooltip_text(Some(name));
        let v = widgets::vbox(0);
        let art = gtk::Overlay::new();
        let pic = gtk::Picture::new();
        pic.set_content_fit(gtk::ContentFit::Cover);
        pic.set_size_request(120, 68);
        pic.set_can_shrink(true);
        pic.set_hexpand(true);
        art.set_child(Some(&pic));
        if let Some(dir) = theme_dir(name) {
            match theme_image(&dir) {
                Some(image) => load_thumbnail(&pic, image),
                None => {
                    if let Some(strip) = colour_strip(&dir) {
                        art.add_overlay(&strip);
                    }
                }
            }
        }
        // Marks the theme in use.
        let tick = gtk::Image::from_icon_name("object-select-symbolic");
        tick.add_css_class("theme-tick");
        tick.set_halign(gtk::Align::End);
        tick.set_valign(gtk::Align::Start);
        art.add_overlay(&tick);
        v.append(&art);
        let l = widgets::label(name, "theme-card-name");
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        v.append(&l);
        card.set_child(Some(&v));
        if *name == current {
            card.add_css_class("selected");
        }
        let n = name.clone();
        let cards2 = cards.clone();
        card.connect_clicked(move |_| {
            for (other, b) in cards2.borrow().iter() {
                if *other == n {
                    b.add_css_class("selected");
                } else {
                    b.remove_css_class("selected");
                }
            }
            let n2 = n.clone();
            cmd::run_async(&["omarchy-theme-set", &n], move |r| match r {
                Ok(_) => crate::window::toast(&format!("Theme set to {n2}")),
                Err(e) => crate::window::toast(&format!("Couldn't set theme: {e}")),
            });
        });
        cards.borrow_mut().push((name.clone(), card.clone()));
        flow.insert(&card, -1);
        if i >= FIRST
            && i != current_at
            && let Some(child) = flow.last_child()
        {
            child.set_visible(false);
            hidden.push(child);
        }
    }
    // No title of its own: the group's heading says it.
    let content = widgets::vbox(10);
    content.append(&flow);
    if !hidden.is_empty() {
        let total = names.len();
        let more = gtk::Button::with_label(&format!("Show all {total} themes"));
        more.add_css_class("show-all");
        more.set_halign(gtk::Align::Center);
        more.connect_clicked(move |b| {
            let show = !hidden.first().is_some_and(|w| w.is_visible());
            for w in &hidden {
                w.set_visible(show);
            }
            b.set_label(&if show { "Show fewer".to_string() } else { format!("Show all {total} themes") });
        });
        content.append(&more);
    }
    let theme_row = widgets::stacked_row("", "", content.upcast_ref());
    theme_row.add_css_class("bare");
    widgets::keywords(&format!("theme {}", names.join(" ")));
    g.add(&theme_row);
}

fn rest(page: &Page) {
    // ----- Wallpaper -----
    let g = page.group("Wallpaper");
    let buttons = widgets::hbox(8);
    buttons.append(&widgets::command_button("Next wallpaper", &["omarchy-theme-bg-next"]));
    buttons.append(&widgets::command_button("Choose…", &["omarchy-menu", "toggle", "background"]));
    g.add(&widgets::row("Background", "Cycle through the current theme's wallpapers or pick one.", Some(buttons.upcast_ref())));

    // ----- Fonts and icons -----
    let g = page.group("Fonts & icons");
    let fonts: Vec<(String, String)> = cmd::output(&["omarchy-font-list"])
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|f| (f.to_string(), f.to_string()))
        .collect();
    let current_font = cmd::output(&["omarchy-font-current"]).unwrap_or_default();
    g.add(&interface_font_row());
    let (r, _) =
        widgets::choice_row("Monospace font", "Used by terminals, the bar and the lock screen.", fonts, &current_font, |font| {
            cmd::run_async(&["omarchy-font-set", &font], move |r| {
                if let Err(e) = r {
                    crate::window::toast(&format!("Couldn't set font: {e}"));
                }
            });
        });
    g.add(&r);
    icons_row(&g);

    // On NyxOS the logo is a system setting, under Boot & Sign-in.
    if crate::platform::PLATFORM != crate::platform::Platform::NyxOS {
        super::themes::logo_group(page);
    }
    super::themes::library_group(page);
    super::themes::install_group(page);
    super::themes::apps_group(page);
    apps_group(page);

    // ----- Settings window -----
    let g = page.collapsible("Settings window", false);
    let p = prefs::get();
    let system = theme::engine::resolve_effective_theme(None, &theme::engine::suite(), &theme::A11y::default()).meta.name;
    let mut options = vec![(String::new(), format!("Follow system ({system})"))];
    options.extend(theme::engine::list_themes().into_iter().map(|m| (m.id, m.name)));
    let (theme_row, _) = widgets::choice_row(
        "Settings theme",
        "Settings follows the system theme unless you give it one of its own. Add your own themes in <tt>~/.config/nyx/themes</tt>.",
        options,
        &theme::app_override().unwrap_or_default(),
        |id| {
            if let Err(e) = theme::set_app_theme((!id.is_empty()).then_some(id.as_str())) {
                crate::window::toast(&e);
            }
        },
    );
    g.add(&theme_row);

    g.add(&nyx_ui::widgets::accent_row("settings"));
    widgets::keywords("accent colour color swatch highlight");

    let (r, _) =
        widgets::switch_row("Reduce motion", "Turn off transitions and animations in this window.", p.reduce_motion, |on| {
            prefs::update(|p| p.reduce_motion = on);
            theme::apply();
        });
    g.add(&r);
    let unit = if prefs::get().temp_unit == prefs::TempUnit::Fahrenheit { "f" } else { "c" };
    let (r, _) = widgets::choice_row(
        "Temperature unit",
        "How temperatures are shown, such as fan curves and GPU limits.",
        widgets::opts(&[("c", "Celsius (°C)"), ("f", "Fahrenheit (°F)")]),
        unit,
        |v| {
            prefs::update(|p| p.temp_unit = if v == "f" { prefs::TempUnit::Fahrenheit } else { prefs::TempUnit::Celsius });
            crate::window::rebuild_if_built("asus");
        },
    );
    widgets::keywords("fahrenheit celsius degrees units temperature");
    g.add(&r);
    // Shared by every Nyx app, like the theme.
    g.add(&nyx_ui::widgets::start_row());
    widgets::keywords("start startup launch first page home remember last open on");

    let g = page.group("Defaults");
    super::themes::reset_group(&g);
}

// ----- Apps follow the theme (hyprchroma) -----

const HYPRCHROMA_URL: &str = "https://github.com/NobleDoodle/hyprchroma";

/// "Synced for <i>Catppuccin</i> · 2 min ago"
fn status_text(st: &chroma::State) -> String {
    if st.theme.is_empty() {
        return "Not synced yet.".into();
    }
    let ago = chroma::ago(&st.last_sync);
    let name = glib::markup_escape_text(&st.theme);
    if ago.is_empty() { format!("Synced for <i>{name}</i>") } else { format!("Synced for <i>{name}</i> · {ago}") }
}

/// The description label of a `widgets::row`.
fn row_desc(row: &gtk::Box) -> Option<gtk::Label> {
    row.first_child()?.last_child()?.downcast::<gtk::Label>().ok()
}

fn apps_group(page: &Page) {
    let g = page.collapsible("Apps follow the theme", false);
    widgets::keywords("gtk qt kde libadwaita apps windows dialogs popups consistent dark reader hyprchroma omarchroma");
    if !chroma::installed() {
        g.add(&widgets::banner(
            "Install <b>hyprchroma</b> to give GTK, libadwaita, Qt and KDE apps the Omarchy theme's colours, \
             so every window and dialog matches.",
            false,
        ));
        let (r, _) = widgets::button_row("hyprchroma", "Not installed.", "Install instructions…", |_| {
            cmd::spawn(&["xdg-open", HYPRCHROMA_URL]);
        });
        g.add(&r);
        return;
    }
    g.note("Windows and dialogs in other apps use the Omarchy theme's colors, and change with it. Powered by hyprchroma.");

    let st = chroma::state();
    let status_row = widgets::row("Status", &status_text(&st), None);
    let refresh = {
        let status_row = status_row.clone();
        move || {
            if let Some(l) = row_desc(&status_row) {
                l.set_markup(&status_text(&chroma::state()));
            }
        }
    };
    let sync = gtk::Button::with_label("Sync now");
    sync.set_valign(gtk::Align::Center);
    {
        let refresh = refresh.clone();
        sync.connect_clicked(move |b| {
            b.set_sensitive(false);
            let (b, refresh) = (b.clone(), refresh.clone());
            cmd::background(chroma::sync_now, move |r| {
                b.set_sensitive(true);
                refresh();
                match r {
                    Ok(_) => crate::window::toast("Apps re-themed"),
                    Err(e) => crate::window::toast(&format!("Couldn't sync: {e}")),
                }
            });
        });
    }
    status_row.append(&sync);
    g.add(&status_row);

    for (key, on) in &st.frameworks {
        let Some(t) = chroma::target(key) else { continue };
        let refresh = refresh.clone();
        let (r, _) = widgets::switch_row(t.title, t.desc, *on, move |on| {
            let refresh = refresh.clone();
            cmd::background(
                move || chroma::set_enabled(t, on),
                move |r| {
                    refresh();
                    if let Err(e) = r {
                        crate::window::toast(&format!("Couldn't change {}: {e}", t.title));
                    }
                },
            );
        });
        if st.status.iter().any(|(k, v)| k == key && v == "not-installed") {
            widgets::tag_row(&r, "App not installed");
        }
        g.add(&r);
    }

    let removed: Vec<&'static chroma::Target> = st.removed.iter().filter_map(|k| chroma::target(k)).collect();
    if !removed.is_empty() {
        let names = removed.iter().map(|t| t.title).collect::<Vec<_>>().join(", ");
        let (r, _) = widgets::button_row("Hidden", &format!("{names}: removed from hyprchroma."), "Restore", move |b| {
            b.set_sensitive(false);
            let removed = removed.clone();
            cmd::background(
                move || removed.iter().try_for_each(|t| chroma::restore_framework(t).map(|_| ())),
                |r| match r {
                    Ok(()) => crate::window::rebuild("theme"),
                    Err(e) => crate::window::toast(&format!("Couldn't restore: {e}")),
                },
            );
        });
        g.add(&r);
    }

    let (r, _) = widgets::switch_row(
        "Automatic sync",
        "Re-theme apps in the background whenever the Omarchy theme or font changes.",
        chroma::service_active(),
        |on| {
            cmd::background(
                move || chroma::set_service(on),
                |r| {
                    if let Err(e) = r {
                        crate::window::toast(&format!("Couldn't change automatic sync: {e}"));
                    }
                },
            );
        },
    );
    g.add(&r);

    // Apps that need a restart; filled in once hyprchroma answers.
    let stale = widgets::vbox(6);
    stale.set_visible(false);
    g.add(&stale);
    {
        let stale = stale.clone();
        cmd::background(chroma::stale_apps, move |apps| {
            for app in &apps {
                stale.append(&widgets::row(app, "Still showing the old theme. Restart it to update.", None));
            }
            stale.set_visible(!apps.is_empty());
        });
    }

    if chroma::plugin_installed() {
        let b = widgets::confirm_button("Remove", "Click again to remove", |b| {
            b.set_sensitive(false);
            cmd::run_async(&["omarchy-plugin-remove", chroma::PLUGIN, "--yes"], |r| match r {
                Ok(_) => {
                    crate::window::toast("Omarchroma removed");
                    crate::window::rebuild("theme");
                }
                Err(e) => crate::window::toast(&format!("Couldn't remove Omarchroma: {e}")),
            });
        });
        b.add_css_class("destructive-action");
        g.add(&widgets::row(
            "Omarchroma bar widget",
            "Settings covers everything it does. Removing it keeps hyprchroma and automatic sync.",
            Some(b.upcast_ref()),
        ));
    }

    let (adv, content) = widgets::disclosure("Advanced", "");
    let b = widgets::confirm_button("Restore", "Click again to restore", move |b| {
        b.set_sensitive(false);
        let (refresh, b) = (refresh.clone(), b.clone());
        cmd::background(chroma::restore_stock, move |r| {
            refresh();
            b.set_sensitive(true);
            match r {
                Ok(_) => crate::window::toast("Stock look restored"),
                Err(e) => crate::window::toast(&format!("Couldn't restore: {e}")),
            }
        });
    });
    b.add_css_class("destructive-action");
    content.append(&widgets::row(
        "Restore stock look",
        "Undo hyprchroma's changes to GTK and KDE files and put back the default styling. Sync again to re-theme.",
        Some(b.upcast_ref()),
    ));
    let source = widgets::row("Colors from", "", None);
    content.append(&source);
    cmd::background(chroma::palette_source, move |src| {
        if let Some(t) = source.first_child()
            && let Ok(t) = t.downcast::<gtk::Box>()
        {
            let d = widgets::label(if src.is_empty() { "Unknown" } else { &src }, "settings-option-description");
            t.append(&d);
        }
    });
    g.add(&adv);
}

// ----- Icons and interface font -----

const FOLLOW: &str = "";

fn icons_row(g: &widgets::Group) {
    let cfg = appearance::load();
    let follow_label = match appearance::omarchy_icon_theme() {
        Some(t) => format!("Follow the theme ({t})"),
        None => "Follow the theme".into(),
    };
    let mut options = vec![(FOLLOW.to_string(), follow_label)];
    options.extend(appearance::icon_themes().into_iter().map(|t| (t.clone(), t)));
    let current = appearance::chosen(&cfg).unwrap_or_default();

    // Umbra's look, shown while Umbra is the icon theme.
    let look = appearance::umbra_look().unwrap_or_else(|| "tile".into());
    let looks: Vec<(String, String)> =
        appearance::UMBRA_LOOKS.iter().map(|(id, label, _)| (id.to_string(), label.to_string())).collect();
    let (look_row, _) = widgets::choice_row(
        "Look",
        "Umbra's folders and files: satin tiles, solid folders and pages, or coloured lines.",
        looks,
        &look,
        |id| {
            cmd::background(
                move || appearance::use_umbra(&id),
                |r: anyhow::Result<()>| {
                    if let Err(e) = r {
                        crate::window::toast(&format!("Couldn't change the look: {e}"));
                    }
                },
            );
        },
    );
    look_row.set_visible(current == appearance::UMBRA);

    let lr = look_row.clone();
    let (r, _) = widgets::choice_row(
        "Icons",
        "Icon theme for apps and file choosers. Omarchy themes pick one; choose here to keep your own.",
        options,
        &current,
        move |id| {
            let umbra = id == appearance::UMBRA;
            lr.set_visible(umbra);
            let c = appearance::Config { icon_theme: (id != FOLLOW).then_some(id) };
            cmd::background(
                move || {
                    appearance::save(&c)?;
                    crate::backend::themehook::ensure();
                    if umbra {
                        let look = appearance::umbra_look().unwrap_or_else(|| "tile".into());
                        appearance::use_umbra(&look)
                    } else {
                        appearance::leave_umbra();
                        appearance::apply_icons(&c)
                    }
                },
                |r: anyhow::Result<()>| {
                    if let Err(e) = r {
                        crate::window::toast(&format!("Couldn't set icons: {e}"));
                    }
                },
            );
        },
    );
    widgets::keywords("icon theme icons yaru papirus adwaita umbra look tile solid line");
    g.add(&r);
    if appearance::umbra_installed() || current == appearance::UMBRA {
        widgets::keywords("umbra icon look tile solid line");
        g.add(&look_row);
    }
}

fn interface_font_row() -> gtk::Box {
    let button = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    button.set_font_desc(&gtk::pango::FontDescription::from_string(&appearance::interface_font()));
    button.set_use_font(true);
    button.set_use_size(true);
    button.connect_font_desc_notify(|b| {
        let Some(desc) = b.font_desc() else { return };
        let desc = desc.to_string();
        if desc == appearance::interface_font() {
            return;
        }
        cmd::background(
            move || appearance::set_interface_font(&desc),
            |r| {
                if let Err(e) = r {
                    crate::window::toast(&format!("Couldn't set font: {e}"));
                }
            },
        );
    });
    widgets::keywords("interface font ui text gtk qt size");
    widgets::row("Interface font", "Text in app windows, menus and dialogs (GTK and Qt).", Some(button.upcast_ref()))
}
