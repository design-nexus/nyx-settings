//! Input › Trackpad: tapping, clicking and scrolling on the left; on the right the
//! trackpad with three or four fingers on it and every swipe and pinch labelled with
//! what it does.

use super::keyboard::{fit, flow, flow_add, hold_controls, more_card, more_page, slim_slider, tame_head, tiled};
use crate::backend::gestures::{self, FingerSet, Gestures};
use crate::backend::store;
use crate::sketch::{self, Pal};
use crate::widgets::{self, Page, hypr_choice, hypr_choice_typed, hypr_slider, hypr_switch, opts};
use crate::{cmd, window};
use gtk::cairo::Context;
use gtk::gio;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;

fn finger_set(g: &mut Gestures, fingers: u8) -> &mut FingerSet {
    if fingers == 3 { &mut g.three } else { &mut g.four }
}

fn edit(fingers: u8, f: impl FnOnce(&mut FingerSet)) {
    store::update(true, |s| f(finger_set(&mut s.gestures, fingers)));
}

fn read_set(fingers: u8) -> FingerSet {
    store::read(|s| if fingers == 3 { s.gestures.three.clone() } else { s.gestures.four.clone() })
}

fn action_options(dir: &str, glimpse: bool) -> Vec<(String, String)> {
    gestures::ACTIONS
        .iter()
        .filter(|(id, _)| !gestures::horizontal_only(id) || dir == "left" || dir == "right")
        .filter(|(id, _)| glimpse || *id != "glimpse")
        .map(|(id, label)| (id.to_string(), label.to_string()))
        .collect()
}

/// (desktop id, name) of every app the launcher shows, by name.
fn installed_apps() -> Vec<(String, String)> {
    let mut apps: Vec<(String, String)> = gio::AppInfo::all()
        .into_iter()
        .filter(|a| a.should_show())
        .filter_map(|a| Some((a.id()?.to_string(), a.name().to_string())))
        .collect();
    apps.sort_by(|a, b| a.0.cmp(&b.0));
    apps.dedup_by(|a, b| a.0 == b.0);
    apps.sort_by_key(|a| a.1.to_lowercase());
    apps
}

/// App picker choices; a chosen app that's since been uninstalled stays listed
/// so the row shows what the gesture is set to.
fn app_options(installed: &[(String, String)], current: Option<&String>) -> Vec<(String, String)> {
    let mut options = vec![(String::new(), "Choose an app…".to_string())];
    if let Some(id) = current.filter(|id| !id.is_empty() && !installed.iter().any(|(i, _)| i == *id)) {
        options.push((id.clone(), id.trim_end_matches(".desktop").to_string()));
    }
    options.extend_from_slice(installed);
    options
}

/// The configured direction that fires when the fingers move toward `place`
/// (with Reverse on, swiping left does what's set for right).
fn configured_at(set: &FingerSet, place: &str) -> &'static str {
    gestures::DIRECTIONS.iter().copied().find(|d| gestures::physical(set, d) == place).unwrap_or("left")
}

/// The trackpad with the fingers on it and an arrow for every swipe and pinch.
fn draw_pad(cr: &Context, p: &Pal, w: f64, h: f64, fingers: u8, set: &FingerSet) {
    let pw = (w - 8.0).min((h - 8.0) * 1.65);
    let ph = pw / 1.65;
    let (x0, y0) = ((w - pw) / 2.0, (h - ph) / 2.0);
    sketch::rounded(cr, x0, y0, pw, ph, ph * 0.1);
    sketch::set(cr, sketch::mix(p.bg, p.text, 0.06), 1.0);
    let _ = cr.fill_preserve();
    sketch::set(cr, sketch::mix(p.bg, p.text, 0.22), 1.0);
    cr.set_line_width(1.5);
    let _ = cr.stroke();
    let (cx, cy) = (w / 2.0, h / 2.0);
    let u = ph / 224.0;
    let colour = p.accent;
    let arrow = |cr: &Context, (x1, y1): (f64, f64), (x2, y2): (f64, f64), dashed: bool, on: bool| {
        let a = if on { 1.0 } else { 0.35 };
        sketch::set(cr, colour, a);
        cr.set_line_width(if dashed { 2.4 } else { 3.6 } * u.max(0.6));
        cr.set_dash(if dashed { &[5.0, 4.0][..] } else { &[][..] }, 0.0);
        cr.move_to(cx + x1 * u, cy + y1 * u);
        cr.line_to(cx + x2 * u, cy + y2 * u);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        let ang = (y2 - y1).atan2(x2 - x1);
        let l = 14.0 * u.max(0.6);
        let (tx, ty) = (cx + x2 * u, cy + y2 * u);
        cr.move_to(tx + 2.0 * ang.cos(), ty + 2.0 * ang.sin());
        cr.line_to(tx - l * (ang - 0.45).cos(), ty - l * (ang - 0.45).sin());
        cr.line_to(tx - l * (ang + 0.45).cos(), ty - l * (ang + 0.45).sin());
        cr.close_path();
        let _ = cr.fill();
    };
    let on = |place: &str| set.action(configured_at(set, place)) != "none";
    arrow(cr, (0.0, -34.0), (0.0, -92.0), false, on("up"));
    arrow(cr, (0.0, 34.0), (0.0, 92.0), false, on("down"));
    arrow(cr, (-80.0, 0.0), (-150.0, 0.0), false, on("left"));
    arrow(cr, (80.0, 0.0), (150.0, 0.0), false, on("right"));
    let pin = on("pinchin");
    arrow(cr, (-120.0, -80.0), (-82.0, -50.0), true, pin);
    arrow(cr, (120.0, 80.0), (82.0, 50.0), true, pin);
    let pout = on("pinchout");
    arrow(cr, (82.0, -50.0), (120.0, -80.0), true, pout);
    arrow(cr, (-82.0, 50.0), (-120.0, 80.0), true, pout);
    let (xs, ys): (&[f64], &[f64]) = if fingers == 3 { (&[-34.0, 0.0, 34.0], &[6.0, -6.0, 4.0]) } else { (&[-51.0, -17.0, 17.0, 51.0], &[10.0, -4.0, -8.0, 6.0]) };
    for (x, y) in xs.iter().zip(ys) {
        cr.arc(cx + x * u, cy + y * u, 14.0 * u, 0.0, 2.0 * PI);
        sketch::set(cr, sketch::mix(p.bg, colour, 0.35), 1.0);
        let _ = cr.fill_preserve();
        sketch::set(cr, colour, 1.0);
        cr.set_line_width(1.6);
        let _ = cr.stroke();
    }
}

/// Everything the gesture labels share while one finger count is shown.
struct Stage {
    fingers: Cell<u8>,
    glimpse: bool,
    installed: Vec<(String, String)>,
    /// The action drop-down for each configured direction now on show.
    dropdowns: RefCell<Vec<(String, gtk::DropDown)>>,
    pad: gtk::DrawingArea,
    cells: [gtk::Box; 6],
    follow: gtk::Box,
}

/// Where each label goes: top row pinch in, swipe up, pinch out; bottom row left, down, right.
const PLACES: [&str; 6] = ["pinchin", "up", "pinchout", "left", "down", "right"];

/// Fill the six labels for the finger count on show.
fn fill_stage(st: &Rc<Stage>) {
    let fingers = st.fingers.get();
    let set = read_set(fingers);
    // Both sideways swipes following the fingers read as one: "Swipe left or right".
    let together = set.action("left") == "workspace-follow" && set.action("right") == "workspace-follow";
    st.dropdowns.borrow_mut().clear();
    for (cell, place) in st.cells.iter().zip(PLACES) {
        super::keyboard::clear(cell);
        let dir = configured_at(&set, place).to_string();
        if together && place == "right" {
            let note = widgets::hbox(6);
            note.add_css_class("tp-follow");
            note.append(&gtk::Image::from_icon_name("object-flip-horizontal-symbolic"));
            let l = widgets::label("Workspaces slide with your fingers", "tp-follow-text");
            l.set_wrap(true);
            l.set_max_width_chars(16);
            note.append(&l);
            note.set_valign(gtk::Align::End);
            cell.set_valign(gtk::Align::End);
            cell.append(&note);
            continue;
        }
        cell.set_valign(gtk::Align::Fill);
        let caption = widgets::label(if together && place == "left" { "Swipe left or right" } else { gestures::direction_label(place) }, "tp-caption");
        cell.append(&caption);
        let options = action_options(&dir, st.glimpse);
        let dd = widgets::dropdown(&options, set.action(&dir));
        dd.set_factory(Some(&short_label(if matches!(place, "left" | "right") && !together { 12 } else { 16 })));
        dd.add_css_class("tp-action");
        cell.append(&dd);

        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some("Command to run"));
        entry.add_css_class("mono");
        entry.set_width_chars(10);
        entry.set_text(set.commands.get(&dir).map(String::as_str).unwrap_or(""));
        entry.set_visible(set.action(&dir) == "command");
        cell.append(&entry);
        let apps = app_options(&st.installed, set.apps.get(&dir));
        let picker = widgets::dropdown(&apps, set.apps.get(&dir).map(String::as_str).unwrap_or(""));
        picker.set_visible(set.action(&dir) == "app");
        cell.append(&picker);

        let d = dir.clone();
        picker.connect_selected_notify(move |p| {
            let Some((id, _)) = apps.get(p.selected() as usize).cloned() else { return };
            let d = d.clone();
            edit(fingers, move |s| {
                s.apps.insert(d, id);
            });
        });
        let d = dir.clone();
        entry.connect_changed(move |e| {
            let text = e.text().to_string();
            let d = d.clone();
            edit(fingers, move |s| {
                s.commands.insert(d, text);
            });
        });
        let d = dir.clone();
        let (e2, p2, st2) = (entry.clone(), picker.clone(), st.clone());
        dd.connect_selected_notify(move |dd| {
            let Some((id, _)) = options.get(dd.selected() as usize).cloned() else { return };
            e2.set_visible(id == "command");
            p2.set_visible(id == "app");
            if WidgetExt::is_visible(&e2) {
                e2.grab_focus();
            }
            let partner = match d.as_str() {
                "left" => Some("right"),
                "right" => Some("left"),
                _ => None,
            };
            // "Follows fingers" covers both horizontal directions at once.
            if let Some(p) = partner {
                let partner_is_follow = read_set(fingers).action(p) == "workspace-follow";
                if id == "workspace-follow" || partner_is_follow {
                    let partner_id = if id == "workspace-follow" { "workspace-follow" } else { "none" };
                    edit(fingers, move |s| {
                        s.actions.insert(p.to_string(), partner_id.to_string());
                    });
                }
            }
            let dd2 = d.clone();
            edit(fingers, move |s| {
                s.actions.insert(dd2, id);
            });
            st2.pad.queue_draw();
            show_follow(&st2);
            if partner.is_some() {
                // The sideways labels may join or part: lay them out again once this signal is done.
                let st3 = st2.clone();
                gtk::glib::idle_add_local_once(move || fill_stage(&st3));
            }
        });
        st.dropdowns.borrow_mut().push((dir, dd));
    }
    show_follow(st);
    st.pad.queue_draw();
}

/// The closed drop-down's label: short, so three fit across the trackpad. The note
/// under the picture says which actions follow the fingers.
fn short_label(chars: i32) -> gtk::SignalListItemFactory {
    let f = gtk::SignalListItemFactory::new();
    f.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        let l = gtk::Label::new(None);
        l.set_xalign(0.0);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_width_chars(6);
        l.set_max_width_chars(chars);
        item.set_child(Some(&l));
    });
    f.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        let (Some(s), Some(l)) = (item.item().and_downcast::<gtk::StringObject>(), item.child().and_downcast::<gtk::Label>()) else { return };
        let text = s.string();
        l.set_text(text.trim_end_matches(" (follows fingers)"));
        l.set_tooltip_text(Some(&text));
    });
    f
}

/// The note that workspaces slide with the fingers, when a sideways swipe does that.
fn show_follow(st: &Stage) {
    let set = read_set(st.fingers.get());
    let together = set.action("left") == "workspace-follow" && set.action("right") == "workspace-follow";
    st.follow.set_visible(!together && ["left", "right"].iter().any(|d| set.action(d) == "workspace-follow"));
}

/// A switch for an Omarchy device toggle (`omarchy-toggle-touchpad on|off`). The device
/// is off while Omarchy keeps its name in `toggles/hypr/<kind>-disabled-name`.
fn device_switch(g: &crate::widgets::Group, title: &str, desc: &str, script: &'static str, keywords: &str, icon: &str) {
    let kind = script.trim_start_matches("omarchy-toggle-");
    let on = !crate::paths::home().join(format!(".local/state/omarchy/toggles/hypr/{kind}-disabled-name")).exists();
    let (r, _) = widgets::switch_row(title, desc, on, move |now| {
        cmd::run_async(&[script, if now { "on" } else { "off" }], move |r| {
            if let Err(e) = r {
                window::toast(&format!("{e}"));
            }
        })
    });
    g.add(&tiled(r, icon));
    widgets::keywords(keywords);
}

pub fn build(page: &Page) {
    let enabled = store::read(|s| s.gestures.enabled);

    // ----- The picture: every gesture on one trackpad -----
    let pv = page.split();
    let seg_slot = widgets::hbox(10);
    let fingers_seg = widgets::hbox(0);
    seg_slot.append(&fingers_seg);
    let manage = gtk::Switch::new();
    manage.set_active(enabled);
    manage.set_valign(gtk::Align::Center);
    manage.set_tooltip_text(Some("Manage swipe gestures here"));
    seg_slot.append(&manage);
    let head = pv.head("Gestures", "Pick what each swipe and pinch does.", &[fit(&seg_slot).upcast_ref()]);
    tame_head(&head, 20);

    let foreign = gestures::foreign_sources();
    if !foreign.is_empty() {
        let names: Vec<String> = foreign.iter().map(|(p, n)| format!("<tt>{}</tt> ({n})", crate::paths::pretty(p))).collect();
        let b = widgets::banner(
            &format!(
                "<b>Other gestures are defined in</b> {}. They load first, so they win over the ones here for the same \
                 fingers and direction.",
                names.join(", ")
            ),
            true,
        );
        let off = gtk::Button::with_label("Turn those off");
        off.set_valign(gtk::Align::Center);
        off.set_tooltip_text(Some("Comments out their hl.gesture lines. Each file is backed up first."));
        off.connect_clicked(|_| match gestures::disable_foreign() {
            Ok(n) => {
                crate::backend::hypr::reload();
                crate::window::toast(&format!("Turned off {n} gestures from other files"));
                crate::window::rebuild("trackpad");
            }
            Err(e) => crate::window::toast(&format!("{e}")),
        });
        b.append(&off);
        pv.add(&b);
    }

    let stage_card = widgets::vbox(6);
    stage_card.add_css_class("preview-card");
    stage_card.add_css_class("tp-stage");
    let cells: [gtk::Box; 6] = std::array::from_fn(|i| {
        let c = widgets::vbox(4);
        c.add_css_class("tp-cell");
        c.set_halign(match i % 3 {
            0 => gtk::Align::Start,
            1 => gtk::Align::Center,
            _ => gtk::Align::End,
        });
        c
    });
    // The middle label sits over the middle; the sides take the room it leaves.
    let row_of = |cs: &[gtk::Box]| {
        let r = gtk::CenterBox::new();
        r.set_start_widget(Some(&cs[0]));
        r.set_center_widget(Some(&cs[1]));
        r.set_end_widget(Some(&cs[2]));
        r
    };
    let top = row_of(&cells[..3]);
    let bottom = row_of(&cells[3..]);
    let follow = widgets::hbox(6);
    follow.add_css_class("tp-follow");
    follow.append(&gtk::Image::from_icon_name("object-flip-horizontal-symbolic"));
    let fl = widgets::label("Workspaces slide with your fingers", "tp-follow-text");
    fl.set_wrap(true);
    follow.append(&fl);
    let shown = Rc::new(Cell::new(3u8));
    let pad = {
        let shown = shown.clone();
        widgets::drawing(210, move |cr, p, _, w, h| {
            let f = shown.get();
            draw_pad(cr, p, w, h, f, &read_set(f))
        })
    };
    pad.set_vexpand(true);
    stage_card.append(&top);
    stage_card.append(&pad);
    stage_card.append(&bottom);
    stage_card.append(&follow);
    pv.add(&fit(&stage_card));
    stage_card.set_sensitive(enabled);

    let st = Rc::new(Stage {
        fingers: Cell::new(3),
        glimpse: cmd::present("nyx-glimpse") || store::read(|s| gestures::DIRECTIONS.iter().any(|d| s.gestures.three.action(d) == "glimpse" || s.gestures.four.action(d) == "glimpse")),
        installed: installed_apps(),
        dropdowns: RefCell::default(),
        pad: pad.clone(),
        cells,
        follow,
    });
    fill_stage(&st);
    widgets::keywords("swipe left right up down pinch three four 3 4 fingers gesture");
    {
        let stage_card = stage_card.clone();
        manage.connect_active_notify(move |sw| {
            let on = sw.is_active();
            stage_card.set_sensitive(on);
            store::update(true, |s| s.gestures.enabled = on);
        });
    }

    // Reverse chips and the way to Swipe feel.
    let opts_line = widgets::hbox(8);
    opts_line.add_css_class("tp-opts");
    let chips = flow();
    chips.set_column_spacing(8);
    chips.set_hexpand(true);
    chips.set_valign(gtk::Align::Center);
    let rev_h = gtk::ToggleButton::new();
    let rev_v = gtk::ToggleButton::new();
    for (b, text, icon) in [
        (&rev_h, "Reverse left and right", "object-flip-horizontal-symbolic"),
        (&rev_v, "Reverse up and down", "object-flip-vertical-symbolic"),
    ] {
        let c = widgets::hbox(6);
        c.append(&gtk::Image::from_icon_name(icon));
        c.append(&gtk::Label::new(Some(text)));
        b.set_child(Some(&c));
        b.add_css_class("tp-chip");
        flow_add(&chips, b);
    }
    opts_line.append(&chips);
    let (feel_box, open_feel) = more_page("Swipe feel", 480);
    let feel_link = gtk::Button::new();
    let fc = widgets::hbox(4);
    fc.append(&gtk::Label::new(Some("Swipe feel")));
    fc.append(&gtk::Image::from_icon_name("go-next-symbolic"));
    feel_link.set_child(Some(&fc));
    feel_link.add_css_class("flat");
    feel_link.add_css_class("tp-link");
    feel_link.connect_clicked(move |_| open_feel());
    feel_link.set_halign(gtk::Align::End);
    feel_link.set_valign(gtk::Align::Center);
    opts_line.append(&feel_link);
    pv.add(&fit(&opts_line));
    opts_line.set_sensitive(enabled);
    {
        let opts_line = opts_line.clone();
        manage.connect_active_notify(move |sw| opts_line.set_sensitive(sw.is_active()));
    }
    let syncing = Rc::new(Cell::new(false));
    let reverse_sync: Rc<dyn Fn(&Stage)> = {
        let (rev_h, rev_v, syncing) = (rev_h.clone(), rev_v.clone(), syncing.clone());
        Rc::new(move |st: &Stage| {
            let set = read_set(st.fingers.get());
            syncing.set(true);
            rev_h.set_active(set.reverse_horizontal);
            rev_v.set_active(set.reverse_vertical);
            syncing.set(false);
        })
    };
    reverse_sync(&st);
    {
        let (st, shown, reverse_sync) = (st.clone(), shown.clone(), reverse_sync.clone());
        fingers_seg.append(&widgets::segmented(&opts(&[("3", "3 fingers"), ("4", "4 fingers")]), "3", move |id| {
            let f = if id == "4" { 4 } else { 3 };
            st.fingers.set(f);
            shown.set(f);
            fill_stage(&st);
            reverse_sync(&st);
        }));
    }
    for (b, vertical) in [(&rev_h, false), (&rev_v, true)] {
        let (st, syncing) = (st.clone(), syncing.clone());
        b.connect_toggled(move |b| {
            if syncing.get() {
                return;
            }
            let on = b.is_active();
            edit(st.fingers.get(), move |s| {
                if vertical {
                    s.reverse_vertical = on;
                } else {
                    s.reverse_horizontal = on;
                }
            });
            fill_stage(&st);
        });
    }
    widgets::keywords("reverse invert swipe direction");
    pv.foot("dialog-information-symbolic", "Two fingers scroll, and two fingers sideways go back and forward in browsers.");

    // ----- Tapping and clicking -----
    let g = page.group("Tapping and clicking");
    g.add(&tiled(hypr_switch("input.touchpad.tap_to_click", "Tap to click", "A light tap with one finger."), "input-touchpad-symbolic"));
    g.add(&tiled(
        hypr_choice(
            "input.touchpad.tap_button_map",
            "Tap with two or three fingers",
            "",
            opts(&[("lrm", "2 = right-click"), ("lmr", "2 = middle-click")]),
        ),
        "input-touchpad-symbolic",
    ));
    widgets::keywords("tap right click middle click two finger three finger 3 fingers middle-click");
    g.add(&tiled(hypr_switch("input.touchpad.tap_and_drag", "Tap and drag", "Tap, then touch again and move."), "view-fullscreen-symbolic"));
    g.add(&tiled(
        hypr_choice_typed(
            "input.touchpad.clickfinger_behavior",
            "Right-click by",
            "Pressing with",
            opts(&[("1", "Two fingers"), ("0", "The bottom corner")]),
            true,
        ),
        "input-mouse-symbolic",
    ));
    widgets::keywords("right click bottom right corner two fingers clickfinger");

    // The rarer ones, a level down.
    let (more_box, open_more) = more_page("More tapping and clicking", 480);
    let card = more_card(&more_box, "");
    card.append(&tiled(
        hypr_choice_typed(
            "input.touchpad.drag_lock",
            "Drag lock",
            "Keep dragging after lifting your finger.",
            opts(&[("0", "Off"), ("1", "Until a short timeout"), ("2", "Until you tap again")]),
            true,
        ),
        "view-fullscreen-symbolic",
    ));
    card.append(&tiled(
        hypr_switch("input.touchpad.middle_button_emulation", "Middle-click with both buttons", "Pressing left and right together is a middle click."),
        "input-mouse-symbolic",
    ));
    card.append(&tiled(
        hypr_choice_typed(
            "input.touchpad.drag_3fg",
            "Three-finger drag",
            "Move windows and selections by dragging with three fingers, no click needed.",
            opts(&[("0", "Off"), ("1", "Three fingers"), ("2", "Four fingers")]),
            true,
        ),
        "input-touchpad-symbolic",
    ));
    let note = widgets::label(
        "A four-finger tap can't be given an action: trackpads report taps with up to three fingers, and only as clicks.",
        "group-note",
    );
    note.set_wrap(true);
    note.add_css_class("tp-more-note");
    more_box.append(&note);
    let (r, _) = widgets::nav_row(Some("view-more-symbolic"), "More", "Drag lock, middle-click, three-finger drag", "", move || open_more());
    g.add(&r);
    widgets::keywords("drag lock middle click both buttons three finger drag tap four fingers 4");

    // ----- Scrolling and typing -----
    let g = page.group("Scrolling and typing");
    g.add(&tiled(
        hypr_switch("input.touchpad.natural_scroll", "Natural scrolling", "Content follows your fingers."),
        "view-continuous-symbolic",
    ));
    widgets::keywords("reverse invert direction two finger swipe scroll");
    g.add(&tiled(
        slim_slider(hypr_slider("input.touchpad.scroll_factor", "Scroll speed", "", (0.1, 2.0, 0.05), 2, "×", false), 120),
        "input-touchpad-symbolic",
    ));
    g.add(&tiled(
        hypr_switch("input.touchpad.disable_while_typing", "Ignore the trackpad while typing", "Stops accidental palm touches."),
        "input-keyboard-symbolic",
    ));
    widgets::keywords("palm typing disable while typing");

    devices(page);

    // ----- Swipe feel, a level down -----
    swipe_feel(&feel_box);
    hold_controls(page, 22);
}

/// Switching the trackpad or touchscreen off, where Omarchy can.
fn devices(page: &Page) {
    if !cmd::present("omarchy-toggle-touchpad") && !cmd::present("omarchy-toggle-touchscreen") {
        return;
    }
    let pad_here = cmd::present("omarchy-toggle-touchpad") && cmd::output(&["omarchy-hw-touchpad"]).is_some_and(|d| !d.is_empty());
    let screen_here = cmd::present("omarchy-toggle-touchscreen") && cmd::output(&["omarchy-hw-touchscreen"]).is_some_and(|d| !d.is_empty());
    if !pad_here && !screen_here {
        return;
    }
    let g = page.group("Devices");
    if pad_here {
        device_switch(
            &g,
            "Trackpad",
            "Switch it off while using a mouse.",
            "omarchy-toggle-touchpad",
            "disable enable touchpad trackpad off",
            "input-touchpad-symbolic",
        );
    }
    if screen_here {
        device_switch(
            &g,
            "Touchscreen",
            "Switch touch on the screen off or on.",
            "omarchy-toggle-touchscreen",
            "touch screen tablet pen disable enable",
            "video-display-symbolic",
        );
    }
}

/// How swiping between workspaces behaves: loop, how many, how far, and past the ends.
fn swipe_feel(content: &gtk::Box) {
    let card = more_card(content, "");
    let (distance, create, forever, looping, loop_count) = store::read(|s| {
        (
            s.gestures.swipe_distance,
            s.gestures.create_new_workspace,
            s.gestures.swipe_forever,
            s.gestures.loop_workspaces,
            s.gestures.loop_count,
        )
    });
    // Swipes the loop takes over (going past the last workspace).
    let past_last = widgets::vbox(0);
    past_last.set_sensitive(!looping);
    let (count_row, _) = widgets::slider_row(
        "Workspaces to swipe through",
        "1 to this many always stay open, like the bar shows them, empty ones too. Swipes never go past the last \
         one (with Loop around, they wrap back to the first).",
        (2.0, 10.0, 1.0),
        loop_count.max(2) as f64,
        0,
        "",
        |v| store::update(true, |s| s.gestures.loop_count = v.round() as u32),
    );
    let (r, _) = {
        let past_last = past_last.clone();
        widgets::switch_row(
            "Loop around",
            "Past the last workspace a swipe slides on to the first, and back from the first to the last.",
            looping,
            move |on| {
                past_last.set_sensitive(!on);
                store::update(true, |s| s.gestures.loop_workspaces = on);
            },
        )
    };
    card.append(&tiled(r, "media-playlist-repeat-symbolic"));
    widgets::keywords("wrap cycle circular loop around first last swipe feel");
    card.append(&tiled(slim_slider(count_row, 120), "view-grid-symbolic"));
    let (r, _) = widgets::slider_row(
        "Swipe distance",
        "How far to swipe for one workspace.",
        (100.0, 1000.0, 10.0),
        distance.unwrap_or(300) as f64,
        0,
        " px",
        |v| store::update(true, |s| s.gestures.swipe_distance = Some(v.round() as i64)),
    );
    card.append(&tiled(slim_slider(r, 120), "object-flip-horizontal-symbolic"));
    let (r, _) = widgets::switch_row(
        "Keep swiping past neighbours",
        "Swipe through several workspaces in one go.",
        forever.unwrap_or(false),
        |on| store::update(true, |s| s.gestures.swipe_forever = Some(on)),
    );
    card.append(&tiled(r, "go-next-symbolic"));
    let (r, _) = widgets::switch_row(
        "Make a workspace past the last one",
        "Off while Loop around is on.",
        create.unwrap_or(false),
        |on| store::update(true, |s| s.gestures.create_new_workspace = Some(on)),
    );
    past_last.append(&tiled(r, "list-add-symbolic"));
    card.append(&past_last);
    let note = widgets::label("These apply to swipes that follow your fingers; Next workspace jumps one at a time.", "group-note");
    note.set_wrap(true);
    note.add_css_class("tp-more-note");
    content.append(&note);
}
