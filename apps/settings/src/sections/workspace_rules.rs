//! Tiling & Workspaces, second half: which screen each workspace is on, how
//! many always exist, a workspace's own layout, name, gaps and decorations,
//! and the scratchpad.

use crate::backend::state::WorkspaceRule;
use crate::backend::{hypr, store};
use crate::dialog::{Field, ask};
use crate::sketch::{self, R, Sketch};
use crate::widgets::{self, Page};
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone)]
struct Screen {
    name: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn screens() -> Vec<Screen> {
    let Some(serde_json::Value::Array(list)) = hypr::json(&["monitors"]) else { return Vec::new() };
    list.iter()
        .filter_map(|m| {
            let scale = m.get("scale")?.as_f64().unwrap_or(1.0).max(0.1);
            Some(Screen {
                name: m.get("name")?.as_str()?.to_string(),
                x: m.get("x")?.as_f64()?,
                y: m.get("y")?.as_f64()?,
                w: m.get("width")?.as_f64()? / scale,
                h: m.get("height")?.as_f64()? / scale,
            })
        })
        .collect()
}

fn rules() -> Vec<WorkspaceRule> {
    store::read(|s| s.workspaces.clone())
}

/// Change workspace `id`'s rule (made if missing, dropped once empty).
fn change(id: u32, f: impl FnOnce(&mut WorkspaceRule)) {
    store::update(true, |s| {
        let i = match s.workspaces.iter().position(|r| r.workspace == id) {
            Some(i) => i,
            None => {
                s.workspaces.push(WorkspaceRule { workspace: id, ..Default::default() });
                s.workspaces.len() - 1
            }
        };
        f(&mut s.workspaces[i]);
        s.workspaces.retain(|r| !r.is_empty());
        s.workspaces.sort_by_key(|r| r.workspace);
    });
}

/// The screens as they're arranged, each with the workspaces pinned to it,
/// and the rest under "Any screen".
fn screen_strip(screens: Vec<Screen>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_height(120);
    area.set_hexpand(true);
    area.add_css_class("workspace-strip");
    area.set_draw_func(move |_, cr, w, h| {
        let p = sketch::pal();
        let (w, h) = (w as f64, h as f64);
        let rules = rules();
        let persistent = store::read(|s| s.persistent_workspaces);
        let pinned = |name: &str| -> Vec<u32> {
            rules.iter().filter(|r| r.monitor.as_deref() == Some(name)).map(|r| r.workspace).collect()
        };
        let free: Vec<u32> = (1..=10)
            .filter(|id| {
                !rules
                    .iter()
                    .any(|r| r.workspace == *id && r.monitor.as_deref().is_some_and(|m| screens.iter().any(|s| s.name == m)))
            })
            .collect();
        // Fit the arrangement into the left of the area, "Any screen" on the right.
        let side = 150.0f64.min(w * 0.3);
        let room = R::new(4.0, 4.0, w - side - 16.0, h - 8.0);
        let (minx, miny) = screens.iter().fold((f64::MAX, f64::MAX), |(a, b), s| (a.min(s.x), b.min(s.y)));
        let (maxx, maxy) = screens.iter().fold((f64::MIN, f64::MIN), |(a, b), s| (a.max(s.x + s.w), b.max(s.y + s.h)));
        let k = if screens.is_empty() { 1.0 } else { (room.w / (maxx - minx)).min(room.h / (maxy - miny)) };
        for s in &screens {
            let r = R::new(room.x + (s.x - minx) * k, room.y + (s.y - miny) * k, s.w * k - 4.0, s.h * k - 4.0);
            sketch::rounded(cr, r.x, r.y, r.w, r.h, 6.0);
            sketch::set(cr, sketch::mix(p.bg, p.text, 0.06), 1.0);
            let _ = cr.fill_preserve();
            sketch::set(cr, p.dim, 0.5);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            text(cr, p.dim, &s.name, r.x + 8.0, r.y + 16.0, 11.0);
            chips(cr, &p, &pinned(&s.name), persistent, R::new(r.x + 8.0, r.y + 26.0, r.w - 16.0, r.h - 30.0));
        }
        let any = R::new(w - side - 4.0, 4.0, side, h - 8.0);
        text(cr, p.dim, "Any screen", any.x, any.y + 12.0, 11.0);
        chips(cr, &p, &free, persistent, R::new(any.x, any.y + 22.0, any.w, any.h - 22.0));
    });
    area
}

fn text(cr: &gtk::cairo::Context, c: sketch::Rgb, s: &str, x: f64, y: f64, size: f64) {
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
    cr.set_font_size(size);
    sketch::set(cr, c, 1.0);
    cr.move_to(x, y);
    let _ = cr.show_text(s);
}

/// Numbered chips; ones that always exist are filled.
fn chips(cr: &gtk::cairo::Context, p: &sketch::Pal, ids: &[u32], persistent: u32, r: R) {
    let (cw, ch, gap) = (22.0, 18.0, 5.0);
    let per_row = ((r.w + gap) / (cw + gap)).floor().max(1.0) as usize;
    for (i, id) in ids.iter().enumerate() {
        let x = r.x + (i % per_row) as f64 * (cw + gap);
        let y = r.y + (i / per_row) as f64 * (ch + gap);
        if y + ch > r.y + r.h + 2.0 {
            break;
        }
        sketch::rounded(cr, x, y, cw, ch, 5.0);
        let always = *id <= persistent;
        sketch::set(cr, if always { p.accent } else { sketch::mix(p.surface, p.text, 0.1) }, 1.0);
        let _ = cr.fill();
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
        cr.set_font_size(10.0);
        let label = id.to_string();
        if let Ok(e) = cr.text_extents(&label) {
            sketch::set(cr, if always { p.bg } else { p.text }, 1.0);
            cr.move_to(x + cw / 2.0 - e.width() / 2.0 - e.x_bearing(), y + ch / 2.0 - e.height() / 2.0 - e.y_bearing());
            let _ = cr.show_text(&label);
        }
    }
}

const MODES: &[(&str, &str)] = &[
    ("", "Same as everywhere"),
    ("dwindle", "Dwindle"),
    ("master", "Master & stack"),
    ("scrolling", "Scrolling"),
    ("floating", "Floating"),
];
/// (id, label, inner and outer gaps).
type Gap = (&'static str, &'static str, Option<(i64, i64)>);
type Refresh = Rc<dyn Fn()>;

const GAPS: &[Gap] = &[
    ("", "Same as everywhere", None),
    ("none", "No gaps", Some((0, 0))),
    ("small", "Small (2 / 4)", Some((2, 4))),
    ("medium", "Medium (5 / 10)", Some((5, 10))),
    ("large", "Large (10 / 20)", Some((10, 20))),
];
const DECOR: &[(&str, &str)] =
    &[("", "Same as everywhere"), ("flat", "No borders, rounding or shadows"), ("noborder", "No borders")];

fn describe(r: &WorkspaceRule) -> String {
    let mut parts = Vec::new();
    if let Some(m) = r.mode.as_deref() {
        parts.push(MODES.iter().find(|(id, _)| *id == m).map(|(_, l)| l.to_string()).unwrap_or_else(|| m.to_string()));
    }
    if let Some(g) = r.gaps {
        parts.push(
            GAPS.iter()
                .find(|(_, _, v)| *v == Some(g))
                .map(|(_, l, _)| l.to_string())
                .unwrap_or_else(|| format!("gaps {} / {}", g.0, g.1)),
        );
    }
    if r.no_border && r.no_rounding && r.no_shadow {
        parts.push("No borders, rounding or shadows".into());
    } else if r.no_border {
        parts.push("No borders".into());
    }
    if let Some(m) = &r.monitor {
        parts.push(format!("on {m}"));
    }
    parts.join(" · ")
}

/// The add/edit form for workspace `id` (or a new one).
fn edit(id: Option<u32>, refresh: Refresh) {
    let r = id.and_then(|id| rules().into_iter().find(|r| r.workspace == id)).unwrap_or_default();
    let ws_options: Vec<(String, String)> = (1..=10).map(|i| (i.to_string(), format!("Workspace {i}"))).collect();
    let gaps_now = GAPS.iter().find(|(_, _, v)| *v == r.gaps).map(|(id, _, _)| *id).unwrap_or("");
    let decor_now = if r.no_border && r.no_rounding && r.no_shadow {
        "flat"
    } else if r.no_border {
        "noborder"
    } else {
        ""
    };
    let fields = vec![
        Field::choice(ws_options, &id.unwrap_or(1).to_string()),
        Field::text_with("Name (optional), e.g. Web", r.name.as_deref().unwrap_or("")),
        Field::choice(
            MODES
                .iter()
                // Floating is Omarchy's own mode; NyxOS has Hyprland's layouts only.
                .filter(|(a, _)| *a != "floating" || crate::platform::PLATFORM == crate::platform::Platform::Omarchy)
                .map(|(a, b)| (a.to_string(), format!("Layout: {b}")))
                .collect(),
            r.mode.as_deref().unwrap_or(""),
        ),
        Field::choice(GAPS.iter().map(|(a, b, _)| (a.to_string(), format!("Gaps: {b}"))).collect(), gaps_now),
        Field::choice(DECOR.iter().map(|(a, b)| (a.to_string(), format!("Windows: {b}"))).collect(), decor_now),
    ];
    let title = if id.is_some() { "Change a workspace" } else { "Give a workspace its own settings" };
    ask(title, "Anything left as \"Same as everywhere\" follows the rest of this page.", fields, "Save", None, move |v, _| {
        let ws: u32 = v[0].parse().unwrap_or(1);
        let name = v[1].trim().to_string();
        if name.chars().count() > 32 {
            return Some("Keep the name to 32 characters.".into());
        }
        let mode = (!v[2].is_empty()).then(|| v[2].clone());
        let gaps = GAPS.iter().find(|(gid, _, _)| *gid == v[3]).and_then(|(_, _, g)| *g);
        let decor = v[4].clone();
        if let Some(old) = id.filter(|old| *old != ws) {
            // Moved to another number: the old one keeps only its screen.
            change(old, |r| {
                r.name = None;
                r.mode = None;
                r.gaps = None;
                r.no_border = false;
                r.no_rounding = false;
                r.no_shadow = false;
            });
        }
        change(ws, |r| {
            r.name = (!name.is_empty()).then_some(name);
            r.mode = mode;
            r.gaps = gaps;
            r.no_border = decor == "flat" || decor == "noborder";
            r.no_rounding = decor == "flat";
            r.no_shadow = decor == "flat";
        });
        refresh();
        None
    });
}

/// The keys for a binding by its description, as Settings shows keys.
fn keys_for(description: &str) -> Option<String> {
    let serde_json::Value::Array(list) = hypr::json(&["binds"])? else { return None };
    list.iter().find(|b| b.get("description").and_then(|d| d.as_str()) == Some(description)).map(|b| {
        super::keybindings::keys_string(
            b.get("modmask").and_then(|m| m.as_u64()).unwrap_or(0),
            b.get("key").and_then(|k| k.as_str()).unwrap_or(""),
        )
    })
}

pub fn build(page: &Page) {
    let screens = screens();
    let g = page.group("Workspaces and screens");
    let strip = screen_strip(screens.clone());
    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("preview-card");
    card.append(&strip);
    g.add(&card);
    let persistent = store::read(|s| s.persistent_workspaces);
    let s2 = strip.clone();
    let (r, _) = widgets::slider_row(
        "Workspaces that always exist",
        "Filled in above. They stay in the bar even when empty; the rest come and go as you use them. 0 leaves this to Hyprland.",
        (0.0, 10.0, 1.0),
        persistent as f64,
        0,
        "",
        move |v| {
            let n = v.round() as u32;
            store::update(true, |s| s.persistent_workspaces = n);
            s2.queue_draw();
        },
    );
    widgets::keywords("persistent empty always workspaces count");
    g.add(&r);
    if screens.len() > 1 {
        let options: Vec<(String, String)> = std::iter::once((String::new(), "Any screen".to_string()))
            .chain(screens.iter().map(|s| (s.name.clone(), s.name.clone())))
            .collect();
        for id in 1..=10u32 {
            let now = rules().into_iter().find(|r| r.workspace == id).and_then(|r| r.monitor).unwrap_or_default();
            let s2 = strip.clone();
            let (r, _) = widgets::choice_row(&format!("Workspace {id}"), "", options.clone(), &now, move |m| {
                change(id, |r| r.monitor = (!m.is_empty()).then_some(m));
                s2.queue_draw();
            });
            g.add(&r);
        }
        widgets::keywords("monitor screen pin assign workspace display");
    } else {
        g.note("With a second screen plugged in, you can choose which workspaces open on it here.");
    }

    let g = page.group("Workspaces with their own settings");
    let list = widgets::vbox(0);
    g.add(&list);
    // The list redraws itself after a change, so it holds a (weak) handle on itself.
    let refresh: Refresh = {
        let (list, strip) = (list.clone(), strip.clone());
        let me: Rc<std::cell::RefCell<std::rc::Weak<dyn Fn()>>> =
            Rc::new(std::cell::RefCell::new(std::rc::Weak::<Box<dyn Fn()>>::new()));
        let me2 = me.clone();
        let f: Refresh = Rc::new(move || {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let refresh = me2.borrow().upgrade();
            let own: Vec<WorkspaceRule> =
                rules().into_iter().filter(|r| r.name.is_some() || r.mode.is_some() || r.gaps.is_some() || r.no_border).collect();
            if own.is_empty() {
                list.append(&widgets::row("None yet", "Give one workspace its own layout, name, gaps or bare windows: a scrolling workspace for writing, a floating one for tools.", None));
            }
            for r in own {
                let id = r.workspace;
                let title = match &r.name {
                    Some(n) => format!("Workspace {id} · {n}"),
                    None => format!("Workspace {id}"),
                };
                let buttons = widgets::hbox(6);
                let e = gtk::Button::with_label("Change");
                let rf = refresh.clone();
                e.connect_clicked(move |_| {
                    if let Some(rf) = rf.clone() {
                        edit(Some(id), rf);
                    }
                });
                let x = gtk::Button::from_icon_name("user-trash-symbolic");
                x.set_tooltip_text(Some("Back to the same as everywhere"));
                x.add_css_class("flat");
                let rf = refresh.clone();
                x.connect_clicked(move |_| {
                    change(id, |r| {
                        r.name = None;
                        r.mode = None;
                        r.gaps = None;
                        r.no_border = false;
                        r.no_rounding = false;
                        r.no_shadow = false;
                    });
                    if let Some(rf) = rf.clone() {
                        rf();
                    }
                });
                buttons.append(&e);
                buttons.append(&x);
                list.append(&widgets::row(&title, &describe(&r), Some(buttons.upcast_ref())));
            }
            strip.queue_draw();
        });
        *me.borrow_mut() = Rc::downgrade(&f);
        f
    };
    refresh();
    let rf = refresh.clone();
    let (r, _) = widgets::button_row(
        "Add a workspace's own settings",
        "Layout, name, gaps and window decorations for one workspace.",
        "Add…",
        move |_| edit(None, rf.clone()),
    );
    widgets::keywords("workspace rule layout name gaps decorations floating scrolling override");
    g.add(&r);

    let g = page.group("Scratchpad");
    let card = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    card.add_css_class("settings-option");
    card.add_css_class("card-row");
    let a = sketch::area(Sketch::Scratchpad, 132, 80, Rc::new(Cell::new(true)));
    a.set_valign(gtk::Align::Center);
    card.append(&a);
    let toggle = keys_for("Toggle scratchpad").unwrap_or_else(|| "its key".into());
    let send = keys_for("Move window to scratchpad");
    let mut text = format!(
        "A hidden workspace that slides over the one you're on: a terminal, notes or music one key away. \
         Show or hide it with <b>{}</b>",
        glib_escape(&toggle)
    );
    match send {
        Some(k) => text.push_str(&format!("; send a window there with <b>{}</b>.", glib_escape(&k))),
        None => text.push('.'),
    }
    let l = gtk::Label::new(None);
    l.set_markup(&text);
    l.set_wrap(true);
    l.set_xalign(0.0);
    l.set_hexpand(true);
    l.add_css_class("settings-option-description");
    card.append(&l);
    g.add(&card);
    let r = widgets::hypr_switch(
        "misc.close_special_on_empty",
        "Hide it when it's empty",
        "Closing its last window hides the scratchpad.",
    );
    widgets::sketched(&r, Sketch::Scratchpad);
    g.add(&r);
}

fn glib_escape(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}
