//! Switcher: Nyx Switcher's settings (`~/.config/nyx/switcher.toml` over NyxOS's
//! defaults). The switcher reads them each
//! time it opens; a changed key is bound in Hyprland by `nyx-switcher --integrate`,
//! which refuses a key another binding uses (its message is shown).

use super::desktop::{self, Kind, desk, on_narrow, text, text_width, tint, window};
use crate::backend::hypr;
use crate::sketch::{self, Pal, R, Rgb, mix, rounded};
use crate::widgets::{self, Page};
use crate::{cmd, window as win};
use gtk::cairo::Context;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// What the switcher uses: NyxOS's defaults and an administrator's under the user's file.
fn read() -> toml::Table {
    nyx_config::Layers::standard().merged("switcher").unwrap_or_default()
}

/// Change one key (in `[keys]` when `table` is given) in the user's file, keeping the rest.
fn set(table: Option<&str>, key: &str, value: toml::Value) -> bool {
    let key = match table {
        Some(name) => format!("{name}.{key}"),
        None => key.to_string(),
    };
    match nyx_config::Layers::standard().set(nyx_config::Layer::User, "switcher", &key, value) {
        Ok(()) => true,
        Err(e) => {
            win::toast(&format!("Couldn't save: {e:#}"));
            false
        }
    }
}

const KEYS: &[(&str, &str, &str)] = &[
    ("windows", "Switch windows", "ALT + TAB"),
    ("windows_reverse", "Switch windows backwards", "ALT + SHIFT + TAB"),
    ("apps", "Switch apps", "SUPER + TAB"),
    ("apps_reverse", "Switch apps backwards", "SUPER + SHIFT + TAB"),
    ("same_app", "Windows of this app", "ALT + grave"),
    ("same_app_reverse", "Windows of this app backwards", "ALT + SHIFT + grave"),
    ("overview", "Show all windows", "SUPER + ALT + W"),
];

const GESTURES: &[(&str, &str)] =
    &[("", "No gesture"), ("4 pinchout", "Spread four fingers"), ("5 up", "Five fingers up"), ("5 down", "Five fingers down")];

/// The keys bound to `key` (from the file or the default), as Hyprland writes them.
fn bound(t: &toml::Table, key: &str) -> String {
    let default = KEYS.iter().find(|(k, _, _)| *k == key).map(|(_, _, d)| *d).unwrap_or("");
    t.get("keys").and_then(|v| v.as_table()).and_then(|k| k.get(key)).and_then(|v| v.as_str()).unwrap_or(default).to_string()
}

/// "ALT + SHIFT + grave" as keycaps: ["Alt", "Shift", "`"].
fn caps(keys: &str) -> Vec<String> {
    keys.split('+')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(|k| match k.to_lowercase().as_str() {
            "grave" => "`".to_string(),
            "super" | "mod4" => "Super".to_string(),
            "ctrl" | "control" => "Ctrl".to_string(),
            l => {
                let mut c = l.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
            }
        })
        .collect()
}

fn keys_title(keys: &str) -> String {
    let c = caps(keys);
    if c.is_empty() { "Not bound".to_string() } else { c.join(" + ") }
}

/// A window as the switcher lists it.
#[derive(Clone)]
struct Win {
    class: String,
    title: String,
    workspace: i64,
    tabs: usize,
}

/// The open windows, newest first, as `scope` and the settings pick them.
fn windows(t: &toml::Table) -> Vec<Win> {
    let Some(serde_json::Value::Array(list)) = hypr::json(&["clients"]) else { return Vec::new() };
    let scope = t.get("scope").and_then(|v| v.as_str()).unwrap_or("all");
    let agents = t.get("show_agent_workspaces").and_then(|v| v.as_bool()).unwrap_or(false);
    let collapse = t.get("collapse_groups").and_then(|v| v.as_bool()).unwrap_or(true);
    let mut wins: Vec<&serde_json::Value> = list
        .iter()
        .filter(|c| c["mapped"].as_bool().unwrap_or(true) && !c["hidden"].as_bool().unwrap_or(false))
        .filter(|c| c["workspace"]["id"].as_i64().unwrap_or(0) > 0)
        .filter(|c| agents || !c["workspace"]["name"].as_str().unwrap_or_default().starts_with("nyx-agent"))
        .collect();
    wins.sort_by_key(|c| c["focusHistoryID"].as_i64().unwrap_or(i64::MAX));
    if let Some(focused) = wins.first().copied() {
        let (mon, ws) = (focused["monitor"].clone(), focused["workspace"]["id"].clone());
        match scope {
            "monitor" => wins.retain(|c| c["monitor"] == mon),
            "workspace" => wins.retain(|c| c["workspace"]["id"] == ws),
            _ => {}
        }
    }
    let mut seen_groups: Vec<serde_json::Value> = Vec::new();
    let mut out = Vec::new();
    for c in wins {
        let group = c["grouped"].as_array().cloned().unwrap_or_default();
        if collapse && group.len() > 1 {
            let key = serde_json::Value::Array(group.clone());
            if seen_groups.contains(&key) {
                continue;
            }
            seen_groups.push(key);
        }
        out.push(Win {
            class: c["class"].as_str().unwrap_or_default().to_string(),
            title: c["title"].as_str().unwrap_or_default().to_string(),
            workspace: c["workspace"]["id"].as_i64().unwrap_or(0),
            tabs: if collapse && group.len() > 1 { group.len() } else { 0 },
        });
    }
    out
}

/// What a window probably shows, from its app, for its drawing.
fn kind_of(class: &str) -> Kind {
    let c = class.to_lowercase();
    if ["term", "alacritty", "kitty", "foot", "ghostty", "wezterm", "konsole"].iter().any(|k| c.contains(k)) {
        Kind::Term
    } else if ["browser", "firefox", "chrom", "brave", "zen", "epiphany", "web"].iter().any(|k| c.contains(k)) {
        Kind::Web
    } else if ["files", "nautilus", "thunar", "dolphin", "nemo"].iter().any(|k| c.contains(k)) {
        Kind::Files
    } else {
        Kind::Doc
    }
}

fn app_name(class: &str) -> String {
    let last = class.rsplit('.').next().unwrap_or(class);
    let mut c = last.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// The switcher's picture as it looks: windows, apps or every window.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Windows,
    Apps,
    All,
}

/// A thumbnail: the window drawn, its workspace, and its icon and title under it.
fn thumb(w: &Win, icon: &gio::Icon, width: i32, on: bool, numbers: bool) -> gtk::Box {
    let b = widgets::vbox(5);
    b.add_css_class("sw-thumb");
    if on {
        b.add_css_class("on");
    }
    tint(&b);
    let (kind, ws, tabs) = (kind_of(&w.class), w.workspace, w.tabs);
    let area = widgets::drawing((width as f64 * 0.62) as i32, move |cr, p, sec, wd, ht| {
        window(cr, p, R::new(0.5, 0.5, wd - 1.0, ht - 1.0), kind, false);
        if numbers {
            rounded(cr, 3.0, ht - 15.0, 14.0, 12.0, 3.0);
            sketch::set(cr, p.bg, 0.85);
            let _ = cr.fill();
            let s = ws.to_string();
            let tw = text_width(cr, &s, 8.5, true);
            text(cr, &s, 10.0 - tw / 2.0, ht - 6.0, 8.5, true, p.text, 0.95);
        }
        if tabs > 1 {
            let s = format!("{tabs} tabs");
            let tw = text_width(cr, &s, 8.0, true) + 10.0;
            rounded(cr, wd - tw - 3.0, ht - 15.0, tw, 12.0, 6.0);
            sketch::set(cr, sec, 0.95);
            let _ = cr.fill();
            text(cr, &s, wd - tw + 2.0, ht - 6.0, 8.0, true, (1.0, 1.0, 1.0), 1.0);
        }
    });
    area.remove_css_class("preview-drawing");
    area.set_content_width(width);
    area.set_hexpand(false);
    b.append(&area);
    let cap = widgets::hbox(4);
    let i = gtk::Image::from_gicon(icon);
    i.set_pixel_size(13);
    cap.append(&i);
    let l = widgets::label(&w.title, "sw-title");
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(1);
    l.set_hexpand(true);
    l.set_xalign(0.0);
    cap.append(&l);
    cap.set_size_request(width, -1);
    b.append(&cap);
    b.set_tooltip_text(Some(&w.title));
    b
}

/// An app as Super + Tab lists it: its icon, and how many windows it has.
fn app_entry(name: &str, icon: &gio::Icon, count: usize, on: bool) -> gtk::Box {
    let b = widgets::vbox(4);
    b.add_css_class("sw-app");
    if on {
        b.add_css_class("on");
    }
    tint(&b);
    let o = gtk::Overlay::new();
    let i = gtk::Image::from_gicon(icon);
    i.set_pixel_size(34);
    o.set_child(Some(&i));
    if count > 1 {
        let c = widgets::label(&count.to_string(), "sw-count");
        c.set_halign(gtk::Align::End);
        c.set_valign(gtk::Align::Start);
        o.add_overlay(&c);
    }
    b.append(&o);
    let l = widgets::label(name, "sw-title");
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(8);
    b.append(&l);
    b
}

/// Keycaps with words between them, for the hint under the switcher.
fn key_hint(parts: &[(Vec<String>, &str)]) -> gtk::Box {
    let h = widgets::hbox(6);
    h.add_css_class("sw-keyhint");
    for (i, (keys, words)) in parts.iter().enumerate() {
        if i > 0 {
            h.append(&widgets::label("·", "sw-hint-text"));
        }
        let k: Vec<&str> = keys.iter().map(String::as_str).collect();
        h.append(&widgets::key_caps(&k));
        h.append(&widgets::label(words, "sw-hint-text"));
    }
    h
}

/// "Show after" as a line: a tap on the left of the mark, a hold on its right.
fn draw_timeline(cr: &Context, p: &Pal, sec: Rgb, w: f64, _h: f64, ms: f64) {
    let (ty, th) = (2.0, 22.0);
    let x = (ms / 400.0).clamp(0.0, 1.0) * (w - 2.0) + 1.0;
    rounded(cr, 1.0, ty, w - 2.0, th, 5.0);
    sketch::set(cr, p.text, 0.07);
    let _ = cr.fill();
    cr.save().ok();
    rounded(cr, 1.0, ty, w - 2.0, th, 5.0);
    cr.clip();
    cr.rectangle(1.0, ty, x - 1.0, th);
    sketch::set(cr, sec, 0.38);
    let _ = cr.fill();
    cr.restore().ok();
    if x > 40.0 {
        text(cr, "Tap", 9.0, ty + 15.0, 10.5, true, p.text, 0.95);
    }
    let hold = "Hold: the switcher shows, and Tab moves along";
    let room = w - x - 18.0;
    let mut s = hold.to_string();
    while !s.is_empty() && text_width(cr, &s, 10.5, false) > room {
        s = s.rsplit_once(' ').map(|(a, _)| a.to_string()).unwrap_or_default();
    }
    if !s.is_empty() {
        text(cr, &s, x + 10.0, ty + 15.0, 10.5, false, p.dim, 1.0);
    }
    sketch::set(cr, sec, 1.0);
    cr.rectangle(x - 1.0, ty - 2.0, 2.0, th + 4.0);
    let _ = cr.fill();
    cr.arc(x, ty, 6.0, 0.0, std::f64::consts::TAU);
    sketch::set(cr, mix(sec, (1.0, 1.0, 1.0), 0.85), 1.0);
    let _ = cr.fill_preserve();
    sketch::set(cr, sec, 1.0);
    cr.set_line_width(2.0);
    let _ = cr.stroke();
    for (i, l) in ["0", "100", "200", "300", "400 ms"].iter().enumerate() {
        let tx = 1.0 + (w - 2.0) * i as f64 / 4.0;
        let lw = text_width(cr, l, 10.0, false);
        text(cr, l, (tx - lw / 2.0).min(w - lw).max(0.0), ty + th + 16.0, 10.0, false, p.dim, 1.0);
    }
}

/// A dimmed desktop behind the switcher.
fn draw_dim_desk(cr: &Context, p: &Pal, sec: Rgb, w: f64, h: f64) {
    let area = desk(cr, p, sec, R::new(0.5, 0.5, w - 1.0, h - 1.0));
    let room = area.inset(9.0);
    let (left, right) = room.hsplit(0.56, 9.0);
    let (top, bottom) = right.vsplit(0.5, 9.0);
    window(cr, p, left, Kind::Files, false);
    window(cr, p, top, Kind::Term, true);
    window(cr, p, bottom, Kind::Doc, false);
    rounded(cr, area.x, area.y, area.w, area.h, 0.0);
    sketch::set(cr, (0.0, 0.0, 0.0), 0.42);
    let _ = cr.fill();
}

pub fn build(page: &Page) {
    let t = read();
    let s = |k: &str, d: &str| t.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string();
    let b = |k: &str, d: bool| t.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
    let opts = |o: &[(&str, &str)]| o.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
    let icons = Rc::new(desktop::app_icons());
    let mode = Rc::new(Cell::new(Mode::Windows));
    let narrow = Rc::new(Cell::new(false));
    let delay = Rc::new(Cell::new(t.get("show_delay_ms").and_then(|v| v.as_integer()).unwrap_or(80) as f64));
    let save_delay = widgets::Debounce::default();

    // ---- The picture: the switcher over the desk, as the keys open it.
    let pv = page.split();
    let line = widgets::label("", "preview-line");
    line.set_wrap(true);
    line.set_xalign(0.0);
    let mode_seg =
        widgets::segmented(&opts(&[("windows", "Windows"), ("apps", "Apps"), ("all", "All windows")]), "windows", |_| {});
    let head = pv.head("", "", &[mode_seg.upcast_ref()]);
    let title = head.first_child().and_then(|t| t.first_child()).and_downcast::<gtk::Label>();
    if let Some(text) = head.first_child().and_downcast::<gtk::Box>() {
        text.append(&line);
    }
    let stage = gtk::Overlay::new();
    stage.add_css_class("sw-stage");
    stage.set_size_request(desktop::PICTURE_MIN, -1);
    stage.set_overflow(gtk::Overflow::Hidden);
    let back = widgets::drawing(290, draw_dim_desk);
    back.set_vexpand(true);
    stage.set_child(Some(&back));
    let panel_slot = widgets::hbox(0);
    panel_slot.set_halign(gtk::Align::Center);
    panel_slot.set_valign(gtk::Align::Center);
    panel_slot.set_margin_top(10);
    stage.add_overlay(&panel_slot);
    let hint_slot = widgets::hbox(0);
    hint_slot.set_halign(gtk::Align::Center);
    hint_slot.set_valign(gtk::Align::End);
    hint_slot.set_margin_bottom(12);
    stage.add_overlay(&hint_slot);
    pv.add(&stage);

    let refresh: Rc<dyn Fn()> = {
        let (mode, narrow, icons, panel_slot, hint_slot, line) =
            (mode.clone(), narrow.clone(), icons.clone(), panel_slot.clone(), hint_slot.clone(), line.clone());
        Rc::new(move || {
            let t = read();
            let thumbs = t.get("thumbnails").and_then(|v| v.as_bool()).unwrap_or(true);
            let collapse = t.get("collapse_groups").and_then(|v| v.as_bool()).unwrap_or(true);
            let size = t.get("thumbnail_size").and_then(|v| v.as_str()).unwrap_or("medium").to_string();
            let scope = t.get("scope").and_then(|v| v.as_str()).unwrap_or("all").to_string();
            let width = match size.as_str() {
                "small" => 62,
                "large" => 108,
                _ => 84,
            } - if narrow.get() { 10 } else { 0 };
            let most = match size.as_str() {
                "large" => 4,
                _ => 5,
            };
            while let Some(c) = panel_slot.first_child() {
                panel_slot.remove(&c);
            }
            while let Some(c) = hint_slot.first_child() {
                hint_slot.remove(&c);
            }
            let panel = widgets::hbox(8);
            panel.add_css_class("sw-panel");
            let wins = windows(&t);
            let icon_of = |class: &str| desktop::app_icon(&icons, class);
            let (keys, heading, about) = match mode.get() {
                Mode::Windows => (
                    bound(&t, "windows"),
                    keys_title(&bound(&t, "windows")),
                    match scope.as_str() {
                        "monitor" => "Windows on this screen, newest first.",
                        "workspace" => "Windows on this workspace, newest first.",
                        _ => "Windows from every workspace, newest first.",
                    },
                ),
                Mode::Apps => (bound(&t, "apps"), keys_title(&bound(&t, "apps")), "Apps with a window open, one entry each."),
                Mode::All => (
                    bound(&t, "overview"),
                    keys_title(&bound(&t, "overview")),
                    "Every window on every workspace, to pick with the mouse or the arrows.",
                ),
            };
            if let Some(title) = &title {
                title.set_text(&heading);
            }
            line.set_text(about);
            if wins.is_empty() {
                panel.append(&widgets::label("No windows open", "sw-empty"));
            }
            match mode.get() {
                Mode::Windows if thumbs => {
                    for (i, w) in wins.iter().take(most).enumerate() {
                        panel.append(&thumb(w, &icon_of(&w.class), width, i == 1, true));
                    }
                }
                Mode::Windows => {
                    for (i, w) in wins.iter().take(7).enumerate() {
                        panel.append(&app_entry(&app_name(&w.class), &icon_of(&w.class), 1, i == 1));
                    }
                }
                Mode::Apps => {
                    let mut apps: Vec<(String, usize)> = Vec::new();
                    for w in &wins {
                        match apps.iter_mut().find(|(c, _)| *c == w.class) {
                            Some(a) => a.1 += 1,
                            None => apps.push((w.class.clone(), 1)),
                        }
                    }
                    for (i, (class, n)) in apps.iter().take(7).enumerate() {
                        panel.append(&app_entry(&app_name(class), &icon_of(class), *n, i == 1));
                    }
                }
                Mode::All => {
                    let grid = gtk::Grid::new();
                    grid.set_column_spacing(8);
                    grid.set_row_spacing(8);
                    let per_row = 4;
                    let all = windows(&{
                        let mut t = t.clone();
                        t.insert("scope".into(), toml::Value::String("all".into()));
                        t.insert("collapse_groups".into(), toml::Value::Boolean(false));
                        t
                    });
                    for (i, w) in all.iter().take(8).enumerate() {
                        grid.attach(&thumb(w, &icon_of(&w.class), 72, i == 0, true), (i % per_row) as i32, (i / per_row) as i32, 1, 1);
                    }
                    panel.append(&grid);
                }
            }
            panel_slot.append(&panel);
            let k = caps(&keys);
            let hint = match mode.get() {
                Mode::Windows if collapse => key_hint(&[(k, "next"), (vec!["Shift".into()], "back"), (vec!["↓".into()], "tabs")]),
                Mode::Windows | Mode::Apps => key_hint(&[(k, "next"), (vec!["Shift".into()], "back")]),
                Mode::All => key_hint(&[(k, "opens it"), (vec!["Esc".into()], "closes it")]),
            };
            hint_slot.append(&hint);
        })
    };
    refresh();
    {
        let mut c = mode_seg.first_child();
        let mut i = 0;
        while let Some(child) = c {
            if let Some(btn) = child.downcast_ref::<gtk::ToggleButton>() {
                let m = [Mode::Windows, Mode::Apps, Mode::All][i.min(2)];
                let (mode, refresh) = (mode.clone(), refresh.clone());
                btn.connect_toggled(move |btn| {
                    if btn.is_active() {
                        mode.set(m);
                        refresh();
                    }
                });
            }
            i += 1;
            c = child.next_sibling();
        }
    }

    // "Show after", as a line you drag.
    let timing = pv.card();
    timing.add_css_class("sw-timing");
    let th = widgets::hbox(8);
    th.append(&widgets::label("Show after", "sw-timing-title"));
    let tsub = widgets::label("How long you hold the keys before it appears.", "sw-timing-sub");
    tsub.set_hexpand(true);
    tsub.set_xalign(0.0);
    tsub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    th.append(&tsub);
    let readout = widgets::label(&format!("{} ms", delay.get().round()), "sw-readout");
    tint(&readout);
    th.append(&readout);
    timing.append(&th);
    let tl = {
        let delay = delay.clone();
        widgets::drawing(48, move |cr, p, sec, w, h| draw_timeline(cr, p, sec, w, h, delay.get()))
    };
    tl.remove_css_class("preview-drawing");
    tl.set_cursor_from_name(Some("pointer"));
    timing.append(&tl);
    let tnote = widgets::label("A quick tap switches straight to the last window without showing anything.", "sw-timing-sub");
    tnote.set_wrap(true);
    tnote.set_xalign(0.0);
    timing.append(&tnote);
    let narrow_slider: Rc<RefCell<Option<gtk::Scale>>> = Rc::default();
    let set_delay: Rc<dyn Fn(f64)> = {
        let (delay, tl, readout, narrow_slider) = (delay.clone(), tl.clone(), readout.clone(), narrow_slider.clone());
        Rc::new(move |v: f64| {
            let v = ((v / 10.0).round() * 10.0).clamp(0.0, 400.0);
            if v == delay.get() {
                return;
            }
            delay.set(v);
            tl.queue_draw();
            readout.set_text(&format!("{v} ms"));
            if let Some(sc) = narrow_slider.borrow().as_ref()
                && sc.value() != v
            {
                sc.set_value(v);
            }
            save_delay.call(300, move || {
                set(None, "show_delay_ms", toml::Value::Integer(v as i64));
            });
        })
    };
    {
        let drag = gtk::GestureDrag::new();
        let start = Rc::new(Cell::new(0.0));
        let (st, sd, a) = (start.clone(), set_delay.clone(), tl.clone());
        drag.connect_drag_begin(move |_, x, _| {
            st.set(x);
            sd(x / a.width().max(1) as f64 * 400.0);
        });
        let (sd, a) = (set_delay.clone(), tl.clone());
        drag.connect_drag_update(move |_, dx, _| sd((start.get() + dx) / a.width().max(1) as f64 * 400.0));
        tl.add_controller(drag);
    }
    let foot = pv.foot("dialog-information-symbolic", "Alt + Tab lists windows; Super + Tab lists apps, one entry each.");

    // ---- The controls.
    let g = page.group("What it lists");
    let (r, _) = {
        let refresh = refresh.clone();
        widgets::choice_row(
            "Windows from",
            "",
            opts(&[("all", "Every workspace"), ("monitor", "This monitor"), ("workspace", "This workspace")]),
            &s("scope", "all"),
            move |v| {
                set(None, "scope", toml::Value::String(v));
                refresh();
            },
        )
    };
    r.prepend(&widgets::tile("view-paged-symbolic"));
    r.add_css_class("with-tile");
    g.add(&r);
    widgets::keywords("scope workspace monitor which windows overview always shows them all");
    let switch = |icon: &str, title: &str, desc: &str, key: &'static str, default: bool, refresh: Option<Rc<dyn Fn()>>| {
        let (r, _) = widgets::switch_row(title, desc, b(key, default), move |on| {
            set(None, key, toml::Value::Boolean(on));
            if let Some(f) = &refresh {
                f();
            }
        });
        r.prepend(&widgets::tile(icon));
        r.add_css_class("with-tile");
        r
    };
    g.add(&switch("view-dual-symbolic", "Tabbed windows once", "↓ shows a group's tabs.", "collapse_groups", true, Some(refresh.clone())));
    widgets::keywords("one entry per group tabbed windows tabs collapse");
    g.add(&switch("starred-symbolic", "Assistant workspaces", "Windows assistants work in.", "show_agent_workspaces", false, Some(refresh.clone())));
    widgets::keywords("agent workspaces nyx-agent assistants");

    let g = page.group("How it looks");
    g.add(&switch("image-x-generic-symbolic", "Window thumbnails", "Off: app icons only.", "thumbnails", true, Some(refresh.clone())));
    widgets::keywords("thumbnails previews icons");
    let size_seg = {
        let refresh = refresh.clone();
        widgets::segmented(&opts(&[("small", "Small"), ("medium", "Medium"), ("large", "Large")]), &s("thumbnail_size", "medium"), move |v| {
            set(None, "thumbnail_size", toml::Value::String(v));
            refresh();
        })
    };
    g.add(&widgets::icon_row("view-fullscreen-symbolic", "Thumbnail size", "", Some(size_seg.upcast_ref())));
    widgets::keywords("thumbnail size small medium large");
    g.add(&switch("emblem-system-symbolic", "Animation", "Fade in, slide the selection.", "animation", true, None));
    widgets::keywords("animation fade slide motion");

    // In a narrow window "Show after" leaves the picture and comes here.
    let g = page.group("Timing");
    let (r, sl) = {
        let sd = set_delay.clone();
        widgets::slider_row("Show after", "A quicker tap switches without showing it.", (0.0, 400.0, 10.0), delay.get(), 0, " ms", move |v| sd(v))
    };
    sl.scale.set_width_request(120);
    *narrow_slider.borrow_mut() = Some(sl.scale.clone());
    r.prepend(&widgets::tile("alarm-symbolic"));
    r.add_css_class("with-tile");
    g.add(&r);
    widgets::forget_rows(&g.wrapper);
    let timing_group = g.wrapper.clone();

    let g = page.group("Keys and gestures");
    let key_count = KEYS.len().saturating_sub(2);
    let (r, _) = {
        let refresh = refresh.clone();
        widgets::nav_row(
            Some("input-keyboard-symbolic"),
            "Keys",
            &format!("{}, {} and {key_count} more", keys_title(&bound(&t, "windows")), keys_title(&bound(&t, "apps"))),
            "",
            move || {
                let refresh = refresh.clone();
                let d = widgets::subpage("Keys", 560, |sp| keys_page(sp));
                d.connect_destroy(move |_| refresh());
            },
        )
    };
    g.add(&r);
    widgets::keywords(
        "keys shortcut keybinding super tab alt grave overview conflict switch windows apps backwards windows of this app show all windows",
    );
    let current = s("gesture", "");
    let label_of = |id: &str| GESTURES.iter().find(|(g, _)| *g == id).map(|(_, l)| l.to_string()).unwrap_or_else(|| id.to_string());
    let value_slot: Rc<RefCell<Option<gtk::Label>>> = Rc::default();
    let vs = value_slot.clone();
    let (r, value) = widgets::nav_row(Some("input-touchpad-symbolic"), "Touchpad gesture", "Opens Show all windows", &label_of(&current), move || {
        if let Some(v) = vs.borrow().clone() {
            widgets::subpage("Touchpad gesture", 520, move |sp| gesture_page(sp, v));
        }
    });
    *value_slot.borrow_mut() = Some(value);
    g.add(&r);
    widgets::keywords("gesture swipe touchpad trackpad fingers overview spread");

    let foot_box = foot.parent();
    on_narrow(page, move |n| {
        timing.set_visible(!n);
        if let Some(f) = &foot_box {
            f.set_visible(!n);
        }
        timing_group.set_visible(n);
        if narrow.get() != n {
            narrow.set(n);
            refresh();
        }
    });
}

/// The keys, one level down: typed as Hyprland writes them.
fn keys_page(sp: &Page) {
    let t = read();
    let g = sp.group("");
    g.note("Keys as Hyprland writes them, like ALT + TAB; leave one empty to unbind it. A key another binding uses isn't taken. Omarchy's Alt+Tab and Super+Tab come back when the switcher is removed.");
    for (key, title, _) in KEYS {
        let current = bound(&t, key);
        let k = *key;
        let bound_now = Rc::new(RefCell::new(current.clone()));
        let field: Rc<RefCell<Option<glib::WeakRef<gtk::Entry>>>> = Rc::default();
        let f = field.clone();
        let (r, entry) = widgets::entry_row(title, "", &current, "Not bound", move |v| {
            let v = v.trim().to_string();
            let old = bound_now.replace(v.clone());
            if !set(Some("keys"), k, toml::Value::String(v)) {
                return;
            }
            let (bound_now, f) = (bound_now.clone(), f.clone());
            bind(move || {
                // Refused: the file and the field go back to the key that's bound.
                set(Some("keys"), k, toml::Value::String(old.clone()));
                bound_now.replace(old.clone());
                if let Some(e) = f.borrow().as_ref().and_then(|w| w.upgrade()) {
                    e.set_text(&old);
                }
            });
        });
        *field.borrow_mut() = Some(entry.downgrade());
        g.add(&r);
    }
}

/// The touchpad gesture for Show all windows, one level down.
fn gesture_page(sp: &Page, value: gtk::Label) {
    let t = read();
    let current = t.get("gesture").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let prev = Rc::new(RefCell::new(current.clone()));
    let g = sp.group("");
    let (r, _) = widgets::choice_row(
        "Show all windows",
        "A touchpad gesture that opens the overview. One another setting already uses isn't taken.",
        GESTURES.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        &current,
        move |v| {
            let old = prev.replace(v.clone());
            let label = GESTURES.iter().find(|(g, _)| *g == v).map(|(_, l)| *l).unwrap_or("");
            value.set_text(label);
            value.set_visible(true);
            if set(None, "gesture", toml::Value::String(v)) {
                let (prev, value) = (prev.clone(), value.clone());
                bind(move || {
                    set(None, "gesture", toml::Value::String(old.clone()));
                    prev.replace(old.clone());
                    value.set_text(GESTURES.iter().find(|(g, _)| *g == old).map(|(_, l)| *l).unwrap_or(""));
                    win::rebuild("switcher");
                });
            }
        },
    );
    g.add(&r);
}

/// Bind the keys from the file; show why not when Hyprland or a conflict says no,
/// and run `undo`.
fn bind(undo: impl FnOnce() + 'static) {
    cmd::background(
        || std::process::Command::new("nyx-switcher").arg("--integrate").output(),
        |r| match r {
            Ok(out) if out.status.success() => win::toast("Keys updated"),
            Ok(out) => {
                let msg = String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("nyx-switcher: ").to_string();
                win::toast(&format!("Keys not changed: {msg}"));
                undo();
            }
            Err(e) => {
                win::toast(&format!("nyx-switcher: {e}"));
                undo();
            }
        },
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn keys_as_caps() {
        assert_eq!(super::caps("ALT + SHIFT + grave"), vec!["Alt", "Shift", "`"]);
        assert_eq!(super::keys_title("SUPER + TAB"), "Super + Tab");
        assert_eq!(super::keys_title(""), "Not bound");
    }
}
