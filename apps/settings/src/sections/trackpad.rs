use crate::backend::gestures::{self, FingerSet, Gestures};
use crate::backend::store;
use crate::widgets::{self, Page, hypr_choice, hypr_choice_typed, hypr_slider, hypr_switch, opts};
use crate::{cmd, window};
use gtk::gio;
use gtk::prelude::*;
use std::rc::Rc;

fn finger_set(g: &mut Gestures, fingers: u8) -> &mut FingerSet {
    if fingers == 3 { &mut g.three } else { &mut g.four }
}

fn edit(fingers: u8, f: impl FnOnce(&mut FingerSet)) {
    store::update(true, |s| f(finger_set(&mut s.gestures, fingers)));
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

/// The swipe/pinch editor for one finger count.
fn finger_card(fingers: u8) -> gtk::Box {
    let set = store::read(|s| if fingers == 3 { s.gestures.three.clone() } else { s.gestures.four.clone() });
    let card = widgets::vbox(10);
    card.add_css_class("gesture-grid");

    let grid = gtk::Grid::new();
    grid.set_row_spacing(8);
    grid.set_column_spacing(14);
    let dropdowns: Rc<std::cell::RefCell<Vec<(String, gtk::DropDown)>>> = Default::default();
    let glimpse = cmd::present("nyx-glimpse") || gestures::DIRECTIONS.iter().any(|d| set.action(d) == "glimpse");
    let installed = installed_apps();

    for (i, dir) in gestures::DIRECTIONS.iter().enumerate() {
        let dir = dir.to_string();
        let l = widgets::label(gestures::direction_label(&dir), "direction");
        l.set_width_chars(12);
        grid.attach(&l, 0, i as i32, 1, 1);

        let options = action_options(&dir, glimpse);
        let dd = widgets::dropdown(&options, set.action(&dir));
        dd.set_hexpand(true);
        grid.attach(&dd, 1, i as i32, 1, 1);

        let extra = widgets::hbox(0);
        extra.set_hexpand(true);
        grid.attach(&extra, 2, i as i32, 1, 1);

        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some("Command to run"));
        entry.add_css_class("mono");
        entry.set_text(set.commands.get(&dir).map(String::as_str).unwrap_or(""));
        entry.set_visible(set.action(&dir) == "command");
        entry.set_hexpand(true);
        extra.append(&entry);

        let apps = app_options(&installed, set.apps.get(&dir));
        let picker = widgets::dropdown(&apps, set.apps.get(&dir).map(String::as_str).unwrap_or(""));
        picker.set_visible(set.action(&dir) == "app");
        picker.set_hexpand(true);
        extra.append(&picker);

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
        let e2 = entry.clone();
        let p2 = picker.clone();
        let all = dropdowns.clone();
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
                let partner_is_follow = store::read(|s| {
                    let set = if fingers == 3 { &s.gestures.three } else { &s.gestures.four };
                    set.action(p) == "workspace-follow"
                });
                if id == "workspace-follow" || partner_is_follow {
                    let partner_id = if id == "workspace-follow" { "workspace-follow".to_string() } else { "none".to_string() };
                    for (dir, other) in all.borrow().iter() {
                        if dir == p {
                            let o = action_options(p, glimpse);
                            if let Some(pos) = o.iter().position(|(x, _)| *x == partner_id)
                                && other.selected() != pos as u32
                            {
                                other.set_selected(pos as u32);
                            }
                        }
                    }
                }
            }
            let d = d.clone();
            edit(fingers, move |s| {
                s.actions.insert(d, id);
            });
        });
        dropdowns.borrow_mut().push((dir, dd));
    }
    card.append(&grid);

    let reverse = widgets::hbox(24);
    for (label, vertical) in [("Reverse left / right", false), ("Reverse up / down", true)] {
        let bx = widgets::hbox(10);
        let sw = gtk::Switch::new();
        sw.set_active(if vertical { set.reverse_vertical } else { set.reverse_horizontal });
        sw.connect_active_notify(move |sw| {
            let on = sw.is_active();
            edit(fingers, move |s| {
                if vertical {
                    s.reverse_vertical = on;
                } else {
                    s.reverse_horizontal = on;
                }
            });
        });
        bx.append(&sw);
        bx.append(&widgets::label(label, ""));
        reverse.append(&bx);
    }
    card.append(&reverse);
    card
}

/// A switch for an Omarchy device toggle (`omarchy-toggle-touchpad on|off`). The device
/// is off while Omarchy keeps its name in `toggles/hypr/<kind>-disabled-name`.
fn device_switch(g: &crate::widgets::Group, title: &str, desc: &str, script: &'static str, keywords: &str) {
    let kind = script.trim_start_matches("omarchy-toggle-");
    let on = !crate::paths::home().join(format!(".local/state/omarchy/toggles/hypr/{kind}-disabled-name")).exists();
    let (r, _) = widgets::switch_row(title, desc, on, move |now| {
        cmd::run_async(&[script, if now { "on" } else { "off" }], move |r| {
            if let Err(e) = r {
                window::toast(&format!("{e}"));
            }
        })
    });
    g.add(&r);
    widgets::keywords(keywords);
}

pub fn build(page: &Page) {
    if cmd::present("omarchy-toggle-touchpad") || cmd::present("omarchy-toggle-touchscreen") {
        let g = page.group("Devices");
        if cmd::present("omarchy-toggle-touchpad") && cmd::output(&["omarchy-hw-touchpad"]).is_some_and(|d| !d.is_empty()) {
            device_switch(
                &g,
                "Touchpad",
                "Switch the touchpad off, for example while using a mouse.",
                "omarchy-toggle-touchpad",
                "disable enable touchpad trackpad off",
            );
        }
        if cmd::present("omarchy-toggle-touchscreen") && cmd::output(&["omarchy-hw-touchscreen"]).is_some_and(|d| !d.is_empty()) {
            device_switch(
                &g,
                "Touchscreen",
                "Switch touch on the screen off or on.",
                "omarchy-toggle-touchscreen",
                "touch screen tablet pen disable enable",
            );
        }
    }
    // ----- Tapping -----
    let g = page.group("Tapping");
    g.add(&hypr_switch("input.touchpad.tap_to_click", "Tap to click", "A light one-finger tap clicks."));
    g.add(&hypr_choice(
        "input.touchpad.tap_button_map",
        "Two- and three-finger taps",
        "Which button a multi-finger tap presses.",
        opts(&[
            ("lrm", "2 fingers: right-click · 3 fingers: middle-click"),
            ("lmr", "2 fingers: middle-click · 3 fingers: right-click"),
        ]),
    ));
    widgets::keywords("tap right click middle click two finger three finger");
    g.add(&hypr_switch("input.touchpad.tap_and_drag", "Tap and drag", "Tap, then touch again and move to drag."));
    g.add(&hypr_choice_typed(
        "input.touchpad.drag_lock",
        "Drag lock",
        "Keep dragging after lifting your finger.",
        opts(&[("0", "Off"), ("1", "Until a short timeout"), ("2", "Until you tap again")]),
        true,
    ));

    // ----- Clicking -----
    let g = page.collapsible("Clicking", false);
    g.add(&hypr_choice_typed(
        "input.touchpad.clickfinger_behavior",
        "Right-click by",
        "",
        opts(&[("1", "Pressing with two fingers"), ("0", "Pressing the bottom-right corner")]),
        true,
    ));
    g.add(&hypr_switch(
        "input.touchpad.middle_button_emulation",
        "Middle-click with both buttons",
        "Pressing left and right together is a middle click.",
    ));
    g.add(&hypr_choice_typed(
        "input.touchpad.drag_3fg",
        "Three-finger drag",
        "Move windows and selections by dragging with three fingers, no click needed.",
        opts(&[("0", "Off"), ("1", "Three fingers"), ("2", "Four fingers")]),
        true,
    ));
    g.add(&hypr_switch(
        "input.touchpad.disable_while_typing",
        "Ignore the trackpad while typing",
        "Stops accidental palm touches.",
    ));

    // ----- Two fingers -----
    let g = page.group("Scrolling");
    g.note("Two-finger movement is scrolling. Swiping two fingers sideways goes back and forward in browsers and file managers.");
    g.add(&hypr_switch(
        "input.touchpad.natural_scroll",
        "Natural scrolling",
        "Content follows your fingers. Turn off to reverse the scroll direction.",
    ));
    widgets::keywords("reverse invert direction two finger swipe");
    g.add(&hypr_slider("input.touchpad.scroll_factor", "Scroll speed", "", (0.1, 2.0, 0.05), 2, "×", false));

    // ----- Swipes -----
    let enabled = store::read(|s| s.gestures.enabled);
    let g = page.group("Gestures");
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
        g.top(&b);
    }
    let cards = widgets::vbox(10);
    cards.set_sensitive(enabled);
    let (r, _) = {
        let cards = cards.clone();
        widgets::switch_row(
            "Manage swipe gestures here",
            "Settings becomes the only place gestures are defined. Omarchy has none by default.",
            enabled,
            move |on| {
                cards.set_sensitive(on);
                store::update(true, |s| s.gestures.enabled = on);
            },
        )
    };
    g.add(&r);

    let three = widgets::stacked_row(
        "Three fingers",
        "Pick what each swipe or pinch does. <b>Follows fingers</b> actions track your movement smoothly.",
        finger_card(3).upcast_ref(),
    );
    widgets::keywords("swipe left right up down pinch reverse three 3");
    cards.append(&three);
    let four = widgets::stacked_row("Four fingers", "", finger_card(4).upcast_ref());
    widgets::keywords("swipe left right up down pinch reverse four 4");
    cards.append(&four);
    g.add(&cards);

    // ----- Feel -----
    let g = page.collapsible("Swipe feel", false);
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
    let past_last = widgets::vbox(6);
    past_last.set_sensitive(!looping);
    let (count_row, _) = widgets::slider_row(
        "Workspaces to swipe through",
        "1 to this many, like the bar shows, including empty ones. Swipes never go past the last one \
         (with Loop around, they wrap back to 1).",
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
            "Swiping past the last workspace slides on and lands on the first, and back from the first lands on the \
             last. Swipes stay smooth everywhere. Follows the reverse setting above.",
            looping,
            move |on| {
                past_last.set_sensitive(!on);
                store::update(true, |s| s.gestures.loop_workspaces = on);
            },
        )
    };
    widgets::keywords("wrap cycle circular loop around first last");
    g.add(&r);
    g.add(&count_row);
    let (r, _) = widgets::slider_row(
        "Swipe distance",
        "How far to swipe for a full workspace change.",
        (100.0, 1000.0, 10.0),
        distance.unwrap_or(300) as f64,
        0,
        " px",
        |v| store::update(true, |s| s.gestures.swipe_distance = Some(v.round() as i64)),
    );
    g.add(&r);
    let (r, _) = widgets::switch_row(
        "Create a workspace past the last one",
        "Off keeps swipes to the workspaces above.",
        create.unwrap_or(false),
        |on| store::update(true, |s| s.gestures.create_new_workspace = Some(on)),
    );
    past_last.append(&r);
    g.add(&past_last);
    let (r, _) = widgets::switch_row(
        "Keep swiping past neighbors",
        "Swipe through several workspaces in one go.",
        forever.unwrap_or(false),
        |on| store::update(true, |s| s.gestures.swipe_forever = Some(on)),
    );
    g.add(&r);

    // ----- Taps with more fingers -----
    let g = page.collapsible("Four-finger tap", false);
    g.add(&widgets::row(
        "Not available",
        "Trackpads report taps with one to three fingers only, and only as mouse clicks, so a four-finger tap can't be \
         bound without a helper that reads the trackpad directly (and needs extra permissions). Use a four-finger pinch or \
         swipe above instead.",
        None,
    ));
    widgets::keywords("tap four fingers 4");
}
