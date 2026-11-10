//! Displays › Virtual monitors: one wide screen split into two or three monitors by
//! Nyx's Hyprland, each with its own workspaces. The layouts live in
//! `virtual-monitors.toml` (nyx-desktop's `virtual_monitors`, shared with
//! `nyx display virtual`); a change takes effect at once, reverts inside Hyprland unless
//! it's kept, and is saved only once kept.

use super::displays::keep_or_revert;
use crate::widgets::{self, Page};
use crate::{theme, window};
use gtk::glib;
use gtk::prelude::*;
use nyx_desktop::hypr::Instance;
use nyx_desktop::virtual_monitors::{self as vm, Display, Fullscreen, Layout, Paths, Preset, Settings, Target};
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;

/// A monitor's virtual monitor role in `hyprctl -j monitors all`: `region`, `host` or `""`.
pub fn role(m: &Value) -> &str {
    m.get("virtual").and_then(|v| v.get("role")).and_then(Value::as_str).unwrap_or("")
}

fn text<'a>(m: &'a Value, key: &str) -> &'a str {
    m.get(key).and_then(Value::as_str).unwrap_or("")
}

fn stable_id(m: &Value) -> String {
    let (make, model, serial) = (text(m, "make").trim(), text(m, "model").trim(), text(m, "serial").trim());
    if make.is_empty() && model.is_empty() { text(m, "name").to_string() } else { format!("{make}|{model}|{serial}") }
}

fn rebuild_soon() {
    glib::timeout_add_local_once(std::time::Duration::from_millis(700), || window::rebuild("displays"));
}

pub fn build(page: &Page, monitors: &[Value]) {
    // Only Nyx's Hyprland has virtual monitors; on any other the section isn't shown.
    let Ok(hypr) = Instance::find() else { return };
    let Ok(status) = vm::status(&hypr) else { return };
    let Ok(paths) = Paths::default_paths() else { return };

    let g = page.group("Virtual monitors");
    widgets::keywords("virtual monitors split ultrawide wide screen regions divide three two");
    let settings = match Settings::load(&paths) {
        Ok(s) => s,
        Err(e) => {
            g.note(&glib::markup_escape_text(&format!(
                "Couldn't read the virtual monitor settings ({e:#}). Fix or remove {} to change them here.",
                paths.user.display()
            )));
            return;
        }
    };
    g.note(
        "Split one wide screen into two or three monitors side by side, each with its own workspaces, windows and fullscreen.",
    );

    let h = hypr.clone();
    let (r, _) = widgets::switch_row("Virtual monitors", "Off gives every screen back whole.", settings.enabled, move |on| {
        set_enabled(&h, on)
    });
    g.add(&r);
    if status.killed {
        g.note("Turned off for this session with the recovery command. They come back when Hyprland's config is next reloaded.");
    }
    if !settings.enabled {
        recovery_note(&g);
        return;
    }

    let (r, _) = widgets::choice_row(
        "Workspace shortcuts act on",
        "Which virtual monitor switching workspaces and swiping change.",
        widgets::opts(&[("focus", "The one with the focused window"), ("pointer", "The one under the pointer")]),
        match settings.workspace_target {
            Target::Focus => "focus",
            Target::Pointer => "pointer",
        },
        |v| update_options(move |s| s.workspace_target = if v == "pointer" { Target::Pointer } else { Target::Focus }),
    );
    g.add(&r);
    let (r, _) = widgets::choice_row(
        "Fullscreen",
        "What fullscreen does on a virtual monitor. The whole screen joins the regions back until fullscreen ends.",
        widgets::opts(&[("region", "Fills the virtual monitor"), ("physical", "Fills the whole screen")]),
        match settings.fullscreen {
            Fullscreen::Region => "region",
            Fullscreen::Physical => "physical",
        },
        |v| update_options(move |s| s.fullscreen = if v == "physical" { Fullscreen::Physical } else { Fullscreen::Region }),
    );
    widgets::keywords("fullscreen game movie whole screen");
    g.add(&r);
    recovery_note(&g);

    // Each physical screen: an ordinary monitor, or the screen behind split regions.
    for m in monitors.iter().filter(|m| role(m) != "region" && !m.get("disabled").and_then(Value::as_bool).unwrap_or(false)) {
        screen(page, &hypr, &settings, &status, m);
    }
}

fn recovery_note(g: &widgets::Group) {
    g.note("If a layout ever leaves you stuck, <tt>nyx display virtual disable --all</tt> in a terminal gives every screen back whole.");
}

fn set_enabled(hypr: &Instance, on: bool) {
    let Ok(paths) = Paths::default_paths() else { return };
    let Ok(mut settings) = Settings::load(&paths) else { return };
    let previous = settings.enabled;
    settings.enabled = on;
    let apply = |s: &Settings| -> anyhow::Result<()> {
        vm::apply_options(hypr, s)?;
        for d in &s.displays {
            vm::apply(hypr, s, d, None)?;
        }
        Ok(())
    };
    if let Err(e) = apply(&settings) {
        window::toast(&format!("{e:#}"));
        return;
    }
    // Turning them off is always safe; turning them on splits any screen set up before.
    if !on || !settings.displays.iter().any(|d| d.enabled && d.layout.regions() > 1) {
        save(&settings, &paths);
        return;
    }
    let (h, s, p) = (hypr.clone(), settings.clone(), paths.clone());
    let back = Rc::new(RefCell::new(settings));
    keep_or_revert(
        DEFAULT_SECONDS,
        move || save(&s, &p),
        move || {
            let mut s = back.borrow().clone();
            s.enabled = previous;
            if let Err(e) = vm::apply_options(&h, &s) {
                window::toast(&format!("{e:#}"));
            }
            rebuild_soon();
        },
    );
}

const DEFAULT_SECONDS: u32 = vm::DEFAULT_CONFIRM_SECONDS;

fn save(settings: &Settings, paths: &Paths) {
    if let Err(e) = settings.save(paths) {
        window::toast(&format!("Couldn't save the virtual monitor settings: {e:#}"));
    }
    rebuild_soon();
}

/// Change the global options: they take effect and are saved at once (not risky).
fn update_options(f: impl FnOnce(&mut Settings)) {
    let Ok(paths) = Paths::default_paths() else { return };
    let Ok(mut settings) = Settings::load(&paths) else { return };
    f(&mut settings);
    if let Ok(hypr) = Instance::find()
        && let Err(e) = vm::apply_options(&hypr, &settings)
    {
        window::toast(&format!("{e:#}"));
    }
    if let Err(e) = settings.save(&paths) {
        window::toast(&format!("Couldn't save the virtual monitor settings: {e:#}"));
    }
}

/// Put a screen's new layout into effect. Risky changes wait to be kept; one that isn't
/// kept goes back to how it was, and only a kept one is saved.
fn commit(hypr: &Instance, display: Display, risky: bool) {
    let Ok(paths) = Paths::default_paths() else { return };
    let Ok(mut settings) = Settings::load(&paths) else { return };
    let previous = settings.display(&display.id, &display.connector).cloned();
    *settings.entry(&display.id, &display.connector, &display.description) = display.clone();
    let seconds = risky.then_some(settings.confirm_seconds.max(5));
    if let Err(e) = vm::apply(hypr, &settings, &display, seconds) {
        window::toast(&format!("{e:#}"));
        return;
    }
    let Some(seconds) = seconds else {
        save(&settings, &paths);
        return;
    };
    let (h, s, p) = (hypr.clone(), settings.clone(), paths.clone());
    let h2 = hypr.clone();
    keep_or_revert(
        seconds,
        move || {
            if let Err(e) = vm::confirm(&h) {
                window::toast(&format!("{e:#}"));
            }
            save(&s, &p);
        },
        move || {
            // Hyprland reverts on its own when the time runs out; this is for Revert.
            let mut s = settings.clone();
            let old = previous.clone().unwrap_or(Display { enabled: false, ..display.clone() });
            *s.entry(&old.id, &old.connector, &old.description) = old.clone();
            if let Err(e) = vm::apply(&h2, &s, &old, None) {
                window::toast(&format!("{e:#}"));
            }
            rebuild_soon();
        },
    );
}

fn layout_id(l: &Layout) -> String {
    match l.preset {
        Preset::Full => "full".into(),
        Preset::Equal => format!("equal-{}", l.count.clamp(1, 3)),
        Preset::CenterFocused => "center".into(),
        Preset::Custom => format!("custom-{}", l.ratios.len().clamp(2, 3)),
    }
}

fn layout_from_id(id: &str, current: &Layout) -> Layout {
    match id {
        "full" => Layout::full(),
        "equal-2" => Layout::equal(2),
        "equal-3" => Layout::equal(3),
        "center" => Layout::center_focused(),
        "custom-2" | "custom-3" => {
            let n = if id == "custom-3" { 3 } else { 2 };
            let start = current.proportions().ok().filter(|r| r.len() == n).unwrap_or_else(|| vec![1.0; n]);
            let total: f64 = start.iter().sum();
            Layout::custom(start.iter().map(|r| (r / total * 100.0).round()).collect())
        }
        _ => current.clone(),
    }
}

fn screen(page: &Page, hypr: &Instance, settings: &Settings, status: &vm::Status, m: &Value) {
    let name = text(m, "name").to_string();
    let id = stable_id(m);
    let description = text(m, "description").to_string();
    let width = m.get("width").and_then(Value::as_i64).unwrap_or(0) as i32;
    let scale = m.get("scale").and_then(Value::as_f64).unwrap_or(1.0);
    let rotated = m.get("transform").and_then(Value::as_i64).unwrap_or(0) % 2 == 1;
    let display = settings.display(&id, &name).cloned().unwrap_or(Display {
        id: id.clone(),
        connector: name.clone(),
        description: description.clone(),
        enabled: false,
        ..Display::default()
    });
    let live = status.groups.iter().find(|g| g.host == name);

    let g = page.group(&format!("{name} · virtual monitors"));
    g.note(&glib::markup_escape_text(&description));
    if rotated {
        g.note("Rotated screens can't be split yet.");
        return;
    }

    let (h, d) = (hypr.clone(), display.clone());
    let (r, _) = widgets::switch_row("Split this screen", "", display.enabled, move |on| {
        commit(&h, Display { enabled: on, ..d.clone() }, on)
    });
    g.add(&r);

    let ratios = Rc::new(RefCell::new(display.layout.proportions().unwrap_or_else(|_| vec![1.0])));
    let preview = preview(ratios.clone(), width, scale);
    let card = widgets::stacked_row("", "", preview.upcast_ref());
    card.add_css_class("bare");
    g.add(&card);
    if !display.enabled {
        return;
    }

    let (h, d) = (hypr.clone(), display.clone());
    let (r, _) = widgets::choice_row(
        "Layout",
        "",
        widgets::opts(&[
            ("full", "The whole screen"),
            ("equal-2", "Two equal"),
            ("equal-3", "Three equal"),
            ("center", "Three, larger in the middle"),
            ("custom-2", "Two, your proportions"),
            ("custom-3", "Three, your proportions"),
        ]),
        &layout_id(&display.layout),
        move |v| {
            let layout = layout_from_id(&v, &d.layout);
            if let Err(e) = vm::validate(&layout, width, scale) {
                window::toast(&format!("{e:#}"));
                return;
            }
            commit(&h, Display { layout, profile: String::new(), ..d.clone() }, true);
        },
    );
    widgets::keywords("layout proportions equal center focused custom");
    g.add(&r);

    if display.layout.preset == Preset::Custom {
        let n = display.layout.ratios.len();
        let total: f64 = display.layout.ratios.iter().sum();
        let percents = Rc::new(RefCell::new(display.layout.ratios.iter().map(|r| r / total * 100.0).collect::<Vec<f64>>()));
        for i in 0..n - 1 {
            let (p, r2, area) = (percents.clone(), ratios.clone(), preview.clone());
            let value = percents.borrow()[i];
            let (r, _) = widgets::slider_row(&format!("Region {}", i + 1), "", (10.0, 80.0, 1.0), value, 0, "%", move |v| {
                let mut p = p.borrow_mut();
                p[i] = v;
                let rest: f64 = p[..n - 1].iter().sum();
                p[n - 1] = (100.0 - rest).max(1.0);
                *r2.borrow_mut() = p.clone();
                area.queue_draw();
            });
            g.add(&r);
        }
        let (h, d, p) = (hypr.clone(), display.clone(), percents.clone());
        let (r, _) = widgets::button_row("Use these proportions", "The last region takes what's left.", "Apply", move |_| {
            let layout = Layout::custom(p.borrow().iter().map(|v| v.round()).collect());
            if let Err(e) = vm::validate(&layout, width, scale) {
                window::toast(&format!("{e:#}"));
                return;
            }
            commit(&h, Display { layout, profile: String::new(), ..d.clone() }, true);
        });
        g.add(&r);
    }

    // Saved layouts.
    let mut names: Vec<(String, String)> = vec![(String::new(), "None".into())];
    names.extend(settings.profiles.keys().map(|k| (k.clone(), k.clone())));
    let (h, d, profiles) = (hypr.clone(), display.clone(), settings.profiles.clone());
    let (r, _) = widgets::choice_row("Saved layout", "", names, &display.profile, move |v| {
        let Some(layout) = profiles.get(&v).cloned() else { return };
        if let Err(e) = vm::validate(&layout, width, scale) {
            window::toast(&format!("{e:#}"));
            return;
        }
        commit(&h, Display { layout, profile: v, ..d.clone() }, true);
    });
    widgets::keywords("profile saved layout");
    g.add(&r);
    let d = display.clone();
    let (r, _) = widgets::entry_row(
        "Save this layout as",
        "A name to pick it by later, here or with nyx display virtual.",
        "",
        "Name",
        move |name| {
            let name = name.trim().to_string();
            if name.is_empty() {
                return;
            }
            let Ok(paths) = Paths::default_paths() else { return };
            let Ok(mut s) = Settings::load(&paths) else { return };
            s.profiles.insert(name.clone(), d.layout.clone());
            s.entry(&d.id, &d.connector, &d.description).profile = name;
            save(&s, &paths);
        },
    );
    g.add(&r);

    let (h, d) = (hypr.clone(), display.clone());
    let counts: Vec<(String, String)> = (1..=10).map(|n| (n.to_string(), n.to_string())).collect();
    let (r, _) = widgets::choice_row(
        "Workspaces on each region",
        "The leftmost region keeps your usual workspaces; the others get their own, numbered from 1.",
        counts,
        &display.workspaces.to_string(),
        move |v| commit(&h, Display { workspaces: v.parse().unwrap_or(10), ..d.clone() }, false),
    );
    g.add(&r);

    if let Some(group) = live {
        let n = name.clone();
        let h = hypr.clone();
        let merged = group.merged();
        let (r, _) = widgets::button_row(
            if merged { "Using the whole screen" } else { "Use the whole screen for now" },
            if merged {
                "The regions are joined until you split them again; their workspaces come back where they were."
            } else {
                "Joins the regions into one, for a game, a film or a big canvas. Nothing closes."
            },
            if merged { "Split again" } else { "Join" },
            move |_| {
                let r = if merged { vm::restore(&h, &n) } else { vm::merge(&h, &n) };
                if let Err(e) = r {
                    window::toast(&format!("{e:#}"));
                }
                rebuild_soon();
            },
        );
        widgets::keywords("merge join full width whole restore split again temporary");
        g.add(&r);
    }
}

/// The screen drawn with its regions, as Hyprland will split it.
fn preview(ratios: Rc<RefCell<Vec<f64>>>, width: i32, scale: f64) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_height(110);
    area.set_hexpand(true);
    area.add_css_class("arrange-canvas");
    area.set_draw_func(move |_, cr, w, h| {
        let pal = theme::current_palette();
        let rgb = |c: &str| crate::fangraph::rgb(c);
        let pad = 16.0;
        let (sw, sh) = ((w as f64 - 2.0 * pad).max(1.0), (h as f64 - 2.0 * pad).max(1.0));
        let (a, b, c) = rgb(&pal.accent);
        let (t1, t2, t3) = rgb(&pal.text);
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
        cr.set_font_size(12.0);
        match vm::partition(width as i64, scale, &ratios.borrow(), vm::MIN_REGION_WIDTH) {
            Ok(spans) => {
                for (i, s) in spans.iter().enumerate() {
                    let x = pad + sw * s.x as f64 / width.max(1) as f64;
                    let rw = sw * s.width as f64 / width.max(1) as f64;
                    cr.set_source_rgba(a, b, c, 0.16);
                    cr.rectangle(x + 2.0, pad, rw - 4.0, sh);
                    let _ = cr.fill_preserve();
                    cr.set_source_rgba(a, b, c, 0.9);
                    cr.set_line_width(1.5);
                    let _ = cr.stroke();
                    let label = format!("{} · {} px", i + 1, (s.width as f64 / scale).round());
                    cr.set_source_rgb(t1, t2, t3);
                    if let Ok(ext) = cr.text_extents(&label) {
                        cr.move_to(x + (rw - ext.width()) / 2.0, pad + (sh + ext.height()) / 2.0);
                        let _ = cr.show_text(&label);
                    }
                }
            }
            Err(e) => {
                cr.set_source_rgb(t1, t2, t3);
                cr.move_to(pad, pad + sh / 2.0);
                let _ = cr.show_text(&format!("{e:#}"));
            }
        }
    });
    area
}
