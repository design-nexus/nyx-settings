use super::arrange;
use crate::backend::state::Monitor;
use crate::backend::{hypr, store};
use crate::widgets::{self, Page, opts};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

const SCALES: &[f64] = &[1.0, 1.25, 1.333333, 1.5, 1.6, 1.666667, 1.75, 2.0, 2.5, 3.0];

fn scale_label(s: f64) -> String {
    format!("{}%", (s * 100.0).round() as i64)
}

/// After a display change, ask to keep it; put the old layout back if nobody answers.
fn confirm_or_revert(previous: BTreeMap<String, Monitor>) {
    let Some(win) = window::window() else { return };
    let dialog = gtk::Window::builder().transient_for(&win).modal(true).title("Keep these display settings?").build();
    dialog.add_css_class("settings-window");
    dialog.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    let card = widgets::vbox(14);
    card.add_css_class("dialog-card");
    card.append(&widgets::label("Keep these display settings?", "section-title"));
    let msg = widgets::label("Reverting in 15 seconds.", "dim");
    card.append(&msg);
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    let revert = gtk::Button::with_label("Revert");
    let keep = gtk::Button::with_label("Keep");
    keep.add_css_class("suggested-action");
    buttons.append(&revert);
    buttons.append(&keep);
    card.append(&buttons);
    dialog.set_child(Some(&card));

    let done = Rc::new(Cell::new(false));
    let do_revert = {
        let dialog = dialog.clone();
        let done = done.clone();
        let previous = previous.clone();
        move || {
            if done.replace(true) {
                return;
            }
            let p = previous.clone();
            store::update(true, move |s| s.monitors = p);
            store::flush();
            dialog.close();
            glib::timeout_add_local_once(std::time::Duration::from_millis(900), || window::rebuild("displays"));
        }
    };
    let r1 = do_revert.clone();
    revert.connect_clicked(move |_| r1());
    let d2 = dialog.clone();
    let done2 = done.clone();
    keep.connect_clicked(move |_| {
        done2.set(true);
        d2.close();
    });
    let keys = gtk::EventControllerKey::new();
    let revert_on_escape = do_revert.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            revert_on_escape();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    dialog.add_controller(keys);
    let left = Rc::new(Cell::new(15));
    let r2 = do_revert.clone();
    let done3 = done.clone();
    glib::timeout_add_seconds_local(1, move || {
        if done3.get() {
            return glib::ControlFlow::Break;
        }
        let n = left.get() - 1;
        left.set(n);
        msg.set_text(&format!("Reverting in {n} seconds."));
        if n <= 0 {
            r2();
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    dialog.present();
}

fn change(output: &str, f: impl FnOnce(&mut Monitor)) {
    let previous = store::read(|s| s.monitors.clone());
    let current = live_monitor(output);
    store::update(true, |s| {
        let m = s.monitors.entry(output.to_string()).or_insert_with(|| current.unwrap_or_default());
        f(m);
    });
    store::flush();
    confirm_or_revert(previous);
}

/// The monitor as it runs now, as a starting point for an override.
fn live_monitor(name: &str) -> Option<Monitor> {
    let mons = hypr::json(&["monitors", "all"])?;
    let m = mons.as_array()?.iter().find(|m| m.get("name").and_then(|n| n.as_str()) == Some(name))?;
    Some(Monitor {
        mode: Some(format!(
            "{}x{}@{:.2}",
            m.get("width")?.as_i64()?,
            m.get("height")?.as_i64()?,
            m.get("refreshRate")?.as_f64()?
        )),
        position: Some(format!("{}x{}", m.get("x")?.as_i64()?, m.get("y")?.as_i64()?)),
        scale: m.get("scale").and_then(|v| v.as_f64()),
        transform: m.get("transform").and_then(|v| v.as_i64()),
        disabled: m.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false),
    })
}

pub fn build(page: &Page) {
    monitors(page);
    rendering(page);
    laptop_screen(page);
}

fn rendering(page: &Page) {
    let g = page.group("Rendering");
    g.note("Applies to all displays. Actual VRR and cursor use are shown above for each screen.");
    g.add(&widgets::hypr_choice_typed(
        "misc.vrr",
        "Variable refresh rate (VRR)",
        "Adaptive Sync on supported displays. Fullscreen modes activate only while eligible content is fullscreen.",
        opts(&[("0", "Off"), ("1", "On"), ("2", "Fullscreen only"), ("3", "Fullscreen video / games")]),
        true,
    ));
    widgets::keywords("adaptive sync freesync gsync vrr power gpu");
    g.add(&widgets::hypr_choice_typed(
        "cursor.no_hardware_cursors",
        "Hardware cursors",
        "Use a hardware cursor when possible to reduce compositor work during pointer movement. Automatic lets Hyprland decide.",
        opts(&[("0", "Enabled when possible"), ("1", "Disabled"), ("2", "Automatic")]),
        true,
    ));
    widgets::keywords("hardware software cursor pointer igpu gpu performance");
    g.add(&widgets::hypr_choice_typed(
        "render.direct_scanout",
        "Direct scanout",
        "Lets eligible fullscreen apps bypass composition. Automatic applies to games. Turn off if fullscreen graphics glitch.",
        opts(&[("0", "Off"), ("1", "On"), ("2", "Automatic for games")]),
        true,
    ));
    widgets::keywords("direct scanout fullscreen video game compositor igpu gpu performance");
}

/// The built-in screen: off while an external monitor is in use, or mirrored onto it.
fn laptop_screen(page: &Page) {
    if !cmd::present("omarchy-hyprland-monitor-internal") || cmd::run(&["omarchy-hw-laptop"]).is_err() {
        return;
    }
    let g = page.group("Laptop screen");
    let run = |args: &'static [&'static str]| {
        cmd::run_async(args, |r| {
            if let Err(e) = r {
                window::toast(&format!("{e}"));
            }
            glib::timeout_add_local_once(std::time::Duration::from_millis(900), || window::rebuild("displays"));
        })
    };
    // Omarchy keeps each of these as a flag file while it's in effect.
    let flag = |name: &str| crate::paths::home().join(format!(".local/state/omarchy/toggles/hypr/{name}.lua")).exists();
    let (r, _) = widgets::switch_row(
        "Built-in screen",
        "Turn the laptop's own screen off, for example with the lid closed on a desk.",
        !flag("internal-monitor-disable"),
        move |on| {
            run(if on { &["omarchy-hyprland-monitor-internal", "on"] } else { &["omarchy-hyprland-monitor-internal", "off"] })
        },
    );
    g.add(&r);
    widgets::keywords("internal laptop display eDP clamshell off disable");
    if cmd::present("omarchy-hyprland-monitor-internal-mirror") {
        let (r, _) = widgets::switch_row(
            "Mirror to an external screen",
            "Show the laptop's screen on the connected monitor too, for presenting.",
            flag("internal-monitor-mirror"),
            move |on| {
                run(if on {
                    &["omarchy-hyprland-monitor-internal-mirror", "on"]
                } else {
                    &["omarchy-hyprland-monitor-internal-mirror", "off"]
                })
            },
        );
        g.add(&r);
        widgets::keywords("duplicate projector presentation external monitor");
    }
    let (r, _) = widgets::button_row(
        "Recover the built-in screen",
        "Use this if the laptop screen stays dark after unplugging a monitor.",
        "Recover",
        move |_| run(&["omarchy-hyprland-monitor-internal", "recover"]),
    );
    widgets::keywords("black dark blank fix");
    g.add(&r);
}

/// Two or more displays: drag them into place, and see which is which.
fn arrangement(page: &Page, monitors: &[Value]) {
    let num = |m: &Value, k: &str| m.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let rects: Vec<arrange::Rect> = monitors
        .iter()
        .filter(|m| !m.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false))
        .map(|m| {
            let (w, h) = arrange::logical_size(
                num(m, "width"),
                num(m, "height"),
                num(m, "scale"),
                m.get("transform").and_then(|v| v.as_i64()).unwrap_or(0),
            );
            arrange::Rect {
                name: m.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                x: num(m, "x"),
                y: num(m, "y"),
                w,
                h,
            }
        })
        .collect();
    if rects.len() < 2 {
        return;
    }
    let g = page.group("Arrangement");
    g.note("Drag a display to where it sits on your desk. It snaps to the edge of the one next to it.");
    let canvas = arrange::canvas(rects, |positions| {
        let previous = store::read(|s| s.monitors.clone());
        let live: Vec<(String, Option<Monitor>)> = positions.iter().map(|(n, _, _)| (n.clone(), live_monitor(n))).collect();
        store::update(true, move |s| {
            for ((name, x, y), (_, current)) in positions.iter().zip(live) {
                let m = s.monitors.entry(name.clone()).or_insert_with(|| current.unwrap_or_default());
                m.position = Some(format!("{x}x{y}"));
            }
        });
        store::flush();
        confirm_or_revert(previous);
    });
    let card = widgets::stacked_row("", "", canvas.upcast_ref());
    card.add_css_class("bare");
    widgets::keywords("arrange arrangement position layout drag left right above below multiple monitors");
    g.add(&card);
    let (r, _) =
        widgets::button_row("Identify", "Shows each display's name on the display itself.", "Identify", |_| arrange::identify());
    widgets::keywords("which monitor name");
    g.add(&r);
}

fn monitors(page: &Page) {
    let monitors: Vec<Value> = hypr::json(&["monitors", "all"]).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    if monitors.is_empty() {
        page.banner("Couldn't read the displays from Hyprland.", true);
        return;
    }
    let brightness = crate::platform::brightness();
    arrangement(page, &monitors);

    for m in &monitors {
        let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let desc = m.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let g = page.group(&name);
        g.note(&glib::markup_escape_text(&desc));

        if !m.get("disabled").and_then(Value::as_bool).unwrap_or(false) {
            for (title, field) in [("VRR active", "vrr"), ("Hardware cursors in use", "hardwareCursorsInUse")] {
                let status = match m.get(field).and_then(Value::as_bool) {
                    Some(true) => "Yes",
                    Some(false) => "No",
                    None => "Unknown",
                };
                let (r, _) = widgets::info_row(title, status);
                g.add(&r);
            }
        }

        if let Some(b) = brightness.filter(|_| m.get("focused").and_then(|f| f.as_bool()).unwrap_or(false)) {
            let n = name.clone();
            let (r, _) = widgets::slider_row("Brightness", "", (1.0, 100.0, 1.0), b, 0, "%", move |v| {
                // Omarchy's script reaches external screens too (DDC); elsewhere the backlight.
                if crate::capability::program("omarchy-brightness-display") {
                    cmd::spawn(&["omarchy-brightness-display", "--no-osd", "--monitor", &n, &format!("{}%", v.round() as i64)]);
                } else {
                    crate::platform::set_brightness(v.round() as i64);
                }
            });
            g.add(&r);
        }

        let modes: Vec<String> = m
            .get("availableModes")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.trim_end_matches("Hz").to_string())).collect())
            .unwrap_or_default();
        let current_mode = format!(
            "{}x{}@{:.2}",
            m.get("width").and_then(|v| v.as_i64()).unwrap_or(0),
            m.get("height").and_then(|v| v.as_i64()).unwrap_or(0),
            m.get("refreshRate").and_then(|v| v.as_f64()).unwrap_or(0.0)
        );
        let mode_opts: Vec<(String, String)> = modes
            .iter()
            .map(|md| {
                let (res, hz) = md.split_once('@').unwrap_or((md, ""));
                let hz = hz.parse::<f64>().map(|h| format!("{} Hz", h.round())).unwrap_or_default();
                (md.clone(), format!("{} · {hz}", res.replace('x', " × ")))
            })
            .collect();
        let n = name.clone();
        let (r, _) = widgets::choice_row("Resolution & refresh rate", "", mode_opts, &current_mode, move |mode| {
            change(&n, |mon| mon.mode = Some(mode));
        });
        g.add(&r);

        let scale = m.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0);
        let mut scale_opts: Vec<(String, String)> = SCALES.iter().map(|s| (format!("{s}"), scale_label(*s))).collect();
        let current_scale = SCALES.iter().find(|s| (*s - scale).abs() < 0.01).map(|s| format!("{s}")).unwrap_or_else(|| {
            scale_opts.push((format!("{scale}"), scale_label(scale)));
            format!("{scale}")
        });
        let n = name.clone();
        let (r, _) = widgets::choice_row(
            "Scale",
            "Makes everything bigger on high-resolution screens.",
            scale_opts,
            &current_scale,
            move |s| {
                let v: f64 = s.parse().unwrap_or(1.0);
                change(&n, |mon| mon.scale = Some(v));
            },
        );
        widgets::keywords("hidpi zoom size");
        g.add(&r);

        let transform = m.get("transform").and_then(|v| v.as_i64()).unwrap_or(0);
        let n = name.clone();
        let (r, _) = widgets::choice_row(
            "Rotation",
            "",
            opts(&[
                ("0", "Normal"),
                ("1", "90°"),
                ("2", "180°"),
                ("3", "270°"),
                ("4", "Flipped"),
                ("5", "Flipped 90°"),
                ("6", "Flipped 180°"),
                ("7", "Flipped 270°"),
            ]),
            &transform.to_string(),
            move |t| {
                let v: i64 = t.parse().unwrap_or(0);
                change(&n, |mon| mon.transform = Some(v));
            },
        );
        widgets::keywords("rotate portrait transform");
        g.add(&r);

        if monitors.len() > 1 {
            let pos = format!(
                "{}x{}",
                m.get("x").and_then(|v| v.as_i64()).unwrap_or(0),
                m.get("y").and_then(|v| v.as_i64()).unwrap_or(0)
            );
            let n = name.clone();
            let (r, _) = widgets::entry_row(
                "Position",
                "Top-left corner in the layout, like <tt>2560x0</tt>, or <tt>auto-right</tt>.",
                &pos,
                "auto",
                move |p| change(&n, |mon| mon.position = Some(p.trim().to_string())),
            );
            g.add(&r);
            let disabled = m.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false);
            let n = name.clone();
            let (r, _) = widgets::switch_row("Use this display", "", !disabled, move |on| change(&n, |mon| mon.disabled = !on));
            g.add(&r);
        }

        let managed = store::read(|s| s.monitors.contains_key(&name));
        if managed {
            let n = name.clone();
            let (r, _) = widgets::button_row(
                "Settings manages this display",
                "Hand it back to <tt>monitors.lua</tt>.",
                "Reset",
                move |_| {
                    let previous = store::read(|s| s.monitors.clone());
                    let n = n.clone();
                    store::update(true, move |s| {
                        s.monitors.remove(&n);
                    });
                    store::flush();
                    confirm_or_revert(previous);
                },
            );
            g.add(&r);
        }
    }
}
