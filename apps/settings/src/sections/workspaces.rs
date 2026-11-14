//! Tiling & Workspaces: the layout and focus beside a tiled workspace, and the rest
//! (workspaces and screens, workspaces' own settings, window controls, the scratchpad)
//! one level down.

use super::desktop::{Kind, desk, on_narrow, pill, text, text_width, window};
use super::workspace_rules;
use crate::backend::{hypr, store};
use crate::sketch::{self, Pal, R, Rgb, Sketch, rounded};
use crate::widgets::{self, Page, hypr_choice_typed, hypr_slider, hypr_switch, hypr_visual_choice};
use gtk::cairo::Context;
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// ---------------------------------------------------------------- One level down

thread_local! {
    /// The page open one level down: its window, title and how it's built.
    static SUB: RefCell<Option<(gtk::Window, &'static str, fn(&Page))>> = const { RefCell::new(None) };
}

/// Open a page one level down. Closing it brings this page up to date.
fn open_sub(title: &'static str, build: fn(&Page)) {
    let w = widgets::subpage(title, 660, build);
    w.connect_close_request(|_| {
        glib::idle_add_local_once(|| crate::window::rebuild("workspaces"));
        glib::Propagation::Proceed
    });
    SUB.with(|s| *s.borrow_mut() = Some((w, title, build)));
}

/// Build the page open one level down again, after a change that alters its rows.
pub(super) fn refresh_sub() {
    match SUB.with(|s| s.borrow_mut().take()) {
        Some((w, title, build)) => {
            w.close();
            open_sub(title, build);
        }
        None => crate::window::rebuild("workspaces"),
    }
}

fn window_controls_page(page: &Page) {
    super::window_controls::build(page);
    // Omarchy's titlebar and the Dock's hover buttons: what NyxOS's decorations replace.
    if !crate::backend::window_controls::decorations() {
        super::own_titlebars::build(page);
    }
}

// ---------------------------------------------------------------- The picture

const KEYS: &[&str] =
    &["general.layout", "general.gaps_in", "general.gaps_out", "master.mfact", "master.new_status", "scrolling.column_width"];

type Vals = Rc<RefCell<HashMap<&'static str, Value>>>;

fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        // Gaps can be "5" or "5 10 5 10": the first is enough here.
        Value::String(s) => s.split_whitespace().next()?.parse().ok(),
        Value::Object(o) => o.values().find_map(|x| number(Some(x))),
        _ => None,
    }
}

fn current(key: &str) -> Option<Value> {
    store::option(key).or_else(|| hypr::get_option(key))
}

fn get(vals: &Vals, key: &str, default: f64) -> f64 {
    number(vals.borrow().get(key)).unwrap_or(default)
}

fn get_str(vals: &Vals, key: &str, default: &str) -> String {
    vals.borrow().get(key).and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| default.to_string())
}

/// Read the picture's options now, and keep them up to date while `owner` lives.
fn watch(owner: &impl IsA<gtk::Widget>, changed: Rc<dyn Fn()>) -> Vals {
    let vals: Vals = Rc::default();
    for k in KEYS {
        if let Some(v) = current(k) {
            vals.borrow_mut().insert(k, v);
        }
    }
    for k in KEYS {
        let (vals, changed) = (vals.clone(), changed.clone());
        store::subscribe(owner, k, move |key, value| {
            let Some(&k) = KEYS.iter().find(|k| **k == key) else { return };
            match value {
                Some(v) => {
                    vals.borrow_mut().insert(k, v.clone());
                    changed();
                }
                // Reset: the value comes back from the user's config after a reload.
                None => {
                    let (vals, changed) = (vals.clone(), changed.clone());
                    glib::timeout_add_local_once(std::time::Duration::from_millis(750), move || {
                        match hypr::get_option(k) {
                            Some(v) => vals.borrow_mut().insert(k, v),
                            None => vals.borrow_mut().remove(k),
                        };
                        changed();
                    });
                }
            }
        });
    }
    vals
}

fn layout_name(id: &str) -> String {
    match id {
        "dwindle" => "Dwindle".into(),
        "master" => "Master & stack".into(),
        "scrolling" => "Scrolling".into(),
        other => {
            let mut c = other.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
    }
}

/// Where the next window opens: a dashed box.
fn ghost(cr: &Context, p: &Pal, sec: Rgb, r: R) {
    rounded(cr, r.x, r.y, r.w, r.h, 6.0);
    sketch::set(cr, sec, 0.16);
    let _ = cr.fill_preserve();
    cr.set_dash(&[5.0, 4.0], 0.0);
    cr.set_line_width(1.3);
    sketch::set(cr, p.text, 0.75);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
    let label = "+  Next window";
    let tw = text_width(cr, label, 11.0, true);
    if r.w > tw + 12.0 && r.h > 20.0 {
        text(cr, label, r.cx() - tw / 2.0, r.cy() + 4.0, 11.0, true, p.text, 0.95);
    }
}

/// A label on a pill in the section's colour, centred on `cx`.
fn tag(cr: &Context, sec: Rgb, s: &str, cx: f64, y: f64) {
    let w = text_width(cr, s, 10.0, true) + 14.0;
    pill(cr, s, cx - w / 2.0, y, 10.0, sec, 0.95, (1.0, 1.0, 1.0));
}

/// One workspace tiled with the layout, its gaps marked, the window under the pointer
/// focused and a dashed box where the next window opens.
fn draw_tiled(cr: &Context, p: &Pal, sec: Rgb, w: f64, h: f64, vals: &Vals) {
    let area = desk(cr, p, sec, R::new(0.5, 0.5, w - 1.0, h - 1.0));
    let gin = get(vals, "general.gaps_in", 5.0).max(0.0);
    let gout = get(vals, "general.gaps_out", 10.0).max(0.0);
    // A little larger than life, so the gaps show.
    let gi = (gin * 1.3).clamp(1.5, 14.0);
    let go = (gout * 1.3).clamp(3.0, 34.0);
    let inner = area.inset(go);
    let g = gi * 2.0;
    let layout = get_str(vals, "general.layout", "dwindle");
    let focus;
    let gap_at;
    match layout.as_str() {
        "master" => {
            let mfact = get(vals, "master.mfact", 0.55).clamp(0.1, 0.9);
            let status = get_str(vals, "master.new_status", "slave");
            let (m, stack) = inner.hsplit(mfact, g);
            let (a, rest) = stack.vsplit(1.0 / 3.0, g);
            let (b, c) = rest.vsplit(0.5, g);
            if status == "master" {
                ghost(cr, p, sec, m);
                window(cr, p, a, Kind::Web, true);
                window(cr, p, b, Kind::Term, false);
                window(cr, p, c, Kind::Doc, false);
                focus = a;
            } else {
                window(cr, p, m, Kind::Web, true);
                window(cr, p, a, Kind::Term, false);
                window(cr, p, b, Kind::Doc, false);
                ghost(cr, p, sec, c);
                focus = m;
            }
            gap_at = (m.x + m.w + g / 2.0, m.y + m.h * 0.3);
        }
        "scrolling" => {
            let cw = get(vals, "scrolling.column_width", 0.5).clamp(0.1, 1.0);
            let col = inner.w * cw;
            let x0 = inner.cx() - col / 2.0;
            cr.save().ok();
            cr.rectangle(area.x, area.y, area.w, area.h);
            cr.clip();
            for i in -2i32..=2 {
                let r = R::new(x0 + i as f64 * (col + g), inner.y, col, inner.h);
                match i {
                    0 => window(cr, p, r, Kind::Doc, true),
                    1 => ghost(cr, p, sec, r),
                    -1 => window(cr, p, r, Kind::Files, false),
                    _ => window(cr, p, r, Kind::Term, false),
                }
            }
            cr.restore().ok();
            focus = R::new(x0, inner.y, col, inner.h);
            gap_at = (x0 - g / 2.0, inner.y + inner.h * 0.3);
            let hint = "Scroll, or Super + arrows, for the next column";
            let tw = text_width(cr, hint, 10.0, true) + 14.0;
            if tw < area.w - 20.0 {
                pill(cr, hint, area.cx() - tw / 2.0, area.y + area.h - go - 14.0, 10.0, (0.0, 0.0, 0.0), 0.6, (1.0, 1.0, 1.0));
            }
        }
        _ => {
            let (a, rest) = inner.hsplit(0.5, g);
            let (b, rest) = rest.vsplit(0.5, g);
            let (c, d) = rest.hsplit(0.5, g);
            window(cr, p, a, Kind::Files, false);
            window(cr, p, b, Kind::Term, false);
            window(cr, p, c, Kind::Doc, true);
            ghost(cr, p, sec, d);
            focus = c;
            gap_at = (a.x + a.w + g / 2.0, a.y + a.h * 0.3);
        }
    }
    sketch::pointer(cr, p, focus.cx() + focus.w * 0.1, focus.cy() + focus.h * 0.1);
    tag(cr, sec, &format!("{} px", gin.round()), gap_at.0, gap_at.1);
    tag(cr, sec, &format!("{} px at the edges", gout.round()), inner.x + inner.w * 0.25, area.y + go / 2.0);
}

// ---------------------------------------------------------------- The page

/// A picture of what a row does, shown when the pointer rests on it.
fn picture_tip(row: &gtk::Box, s: impl Fn() -> Sketch + 'static) {
    row.set_has_tooltip(true);
    row.connect_query_tooltip(move |_, _, _, _, tip| {
        let a = sketch::area(s(), 132, 80, Rc::new(std::cell::Cell::new(true)));
        tip.set_custom(Some(&a));
        true
    });
}

fn tipped(row: gtk::Box, s: Sketch) -> gtk::Box {
    picture_tip(&row, move || s);
    row
}

/// Layout cards as in the mockup: a sketch, the name in bold and a line under it.
fn restyle_layout_cards(outer: &gtk::Box) {
    fn walk(w: &gtk::Widget, f: &dyn Fn(&gtk::Widget)) {
        f(w);
        let mut c = w.first_child();
        while let Some(ch) = c {
            walk(&ch, f);
            c = ch.next_sibling();
        }
    }
    walk(outer.upcast_ref(), &|w| {
        if let Some(l) = w.downcast_ref::<gtk::Label>() {
            if l.has_css_class("picture-card-label") {
                let name = l.text().to_string();
                let desc = match name.as_str() {
                    "Dwindle" => "Each new window splits the last in half",
                    "Master & stack" => "One big window, the rest beside it",
                    "Scrolling" => "Columns side by side; scroll through",
                    _ => "",
                };
                l.set_markup(&format!(
                    "<b>{}</b>\n<span size=\"small\">{}</span>",
                    glib::markup_escape_text(&name),
                    glib::markup_escape_text(desc)
                ));
                l.set_xalign(0.0);
                l.set_justify(gtk::Justification::Left);
                l.set_max_width_chars(16);
                l.set_halign(gtk::Align::Fill);
                l.add_css_class("ws-lay-label");
            }
        } else if let Some(f) = w.downcast_ref::<gtk::FlowBox>() {
            f.set_min_children_per_line(3);
            f.set_max_children_per_line(3);
        } else if let Some(a) = w.downcast_ref::<gtk::DrawingArea>() {
            a.set_content_width(80);
            a.set_content_height(50);
            a.set_halign(gtk::Align::Fill);
        }
    });
    // The group is titled "Layout" already.
    if let Some(t) = outer
        .first_child()
        .and_then(|head| head.first_child())
        .and_then(|text| text.first_child())
        .and_downcast::<gtk::Label>()
    {
        t.set_visible(false);
    }
    outer.add_css_class("ws-layouts");
}

pub fn build(page: &Page) {
    // ---- The picture: a workspace tiled with the layout.
    let pv = page.split();
    let line = widgets::label("", "preview-line");
    line.set_wrap(true);
    line.set_xalign(0.0);
    let gaps = gtk::Button::new();
    gaps.add_css_class("ws-gaps-button");
    let gl = widgets::hbox(6);
    gl.append(&gtk::Image::from_icon_name("preferences-desktop-appearance-symbolic"));
    gl.append(&gtk::Label::new(Some("Gaps on Window Style")));
    gaps.set_child(Some(&gl));
    gaps.connect_clicked(|_| crate::window::navigate("look"));
    let head = pv.head("", "", &[gaps.upcast_ref()]);
    let title = head.first_child().and_then(|t| t.first_child()).and_downcast::<gtk::Label>();
    if let Some(text) = head.first_child().and_downcast::<gtk::Box>() {
        text.append(&line);
    }
    let vals_slot: Rc<RefCell<Option<Vals>>> = Rc::default();
    let area = {
        let vs = vals_slot.clone();
        widgets::drawing(300, move |cr, p, sec, w, h| {
            if let Some(v) = vs.borrow().as_ref() {
                draw_tiled(cr, p, sec, w, h, v);
            }
        })
    };
    // A screen's proportions, however tall the controls beside it.
    area.connect_resize(|a, w, _| {
        let h = (w as f64 * 0.62).round() as i32;
        if a.content_height() != h {
            let a = a.clone();
            glib::idle_add_local_once(move || a.set_content_height(h));
        }
    });
    area.set_size_request(super::desktop::PICTURE_MIN, -1);
    area.add_css_class("ws-desk");
    pv.add(&area);
    let active = hypr::json(&["activeworkspace"]).and_then(|v| v.get("id").and_then(|i| i.as_i64()));
    let describe: Rc<dyn Fn()> = {
        let (vs, area, title, line) = (vals_slot.clone(), area.clone(), title.clone(), line.clone());
        Rc::new(move || {
            if let Some(v) = vs.borrow().as_ref() {
                if let Some(t) = &title {
                    t.set_text(&layout_name(&get_str(v, "general.layout", "dwindle")));
                }
                line.set_text(&match active {
                    Some(n) => format!("Workspace {n}. The dashed box is where the next window opens."),
                    None => "The dashed box is where the next window opens.".to_string(),
                });
            }
            area.queue_draw();
        })
    };
    let vals = watch(&area, describe.clone());
    *vals_slot.borrow_mut() = Some(vals.clone());
    describe();
    let strip = workspace_rules::strip_card(|| open_sub("Workspaces and screens", workspace_rules::screens_page));
    pv.add(&strip);

    // ---- The controls: the layout and focus; the rest one level down.
    let g = page.group("Layout");
    let cards = hypr_visual_choice(
        "general.layout",
        "Layout",
        "How windows share the screen as you open more of them.",
        vec![
            ("dwindle".into(), "Dwindle".into(), Sketch::Layout("dwindle")),
            ("master".into(), "Master & stack".into(), Sketch::Layout("master")),
            ("scrolling".into(), "Scrolling".into(), Sketch::Layout("scrolling")),
        ],
        false,
    );
    widgets::keywords("tiling dwindle master stack scrolling how windows share the screen as you open more");
    restyle_layout_cards(&cards);
    g.add(&cards);
    // Each layout's own settings, under it while it's the one in use.
    let dwindle = widgets::vbox(0);
    dwindle.append(&tipped(
        hypr_switch("dwindle.smart_split", "Split toward the pointer", "A new window opens on the side the pointer is on."),
        Sketch::SplitTowardCursor,
    ));
    widgets::keywords("smart split cursor dwindle");
    dwindle.append(&tipped(
        hypr_switch("dwindle.preserve_split", "Keep split direction", "Splits stay as they are when windows move or close."),
        Sketch::PreserveSplit,
    ));
    widgets::keywords("preserve split direction dwindle");
    let master = widgets::vbox(0);
    master.append(&hypr_visual_choice(
        "master.new_status",
        "New windows",
        "Where a window you open goes.",
        vec![
            ("slave".into(), "Join the stack".into(), Sketch::NewWindow("slave")),
            ("master".into(), "Become the big one".into(), Sketch::NewWindow("master")),
            ("inherit".into(), "Next to the focused one".into(), Sketch::NewWindow("inherit")),
        ],
        false,
    ));
    widgets::keywords("master new status stack");
    master.append(&hypr_slider("master.mfact", "Master width", "How much of the screen the big window takes.", (0.2, 0.8, 0.01), 2, "", false));
    widgets::keywords("master width mfact ratio");
    let scrolling = widgets::vbox(0);
    scrolling.append(&hypr_slider(
        "scrolling.column_width",
        "Column width",
        "How wide a new column is, as a share of the screen.",
        (0.2, 1.0, 0.05),
        2,
        "",
        false,
    ));
    widgets::keywords("scrolling column width");
    for b in [&dwindle, &master, &scrolling] {
        g.add(b);
    }
    let show_for: Rc<dyn Fn()> = {
        let (vals, dwindle, master, scrolling) = (vals.clone(), dwindle.clone(), master.clone(), scrolling.clone());
        Rc::new(move || {
            let l = get_str(&vals, "general.layout", "dwindle");
            master.set_visible(l == "master");
            scrolling.set_visible(l == "scrolling");
            dwindle.set_visible(l != "master" && l != "scrolling");
        })
    };
    show_for();
    {
        let show_for = show_for.clone();
        store::subscribe(&dwindle, "general.layout", move |_, _| {
            let f = show_for.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || f());
            show_for();
        });
    }

    let g = page.group("Focus");
    let typing = hypr_choice_typed(
        "input.follow_mouse",
        "Typing goes to",
        "",
        vec![
            ("1".into(), "The window under the pointer".into()),
            ("0".into(), "The window you click".into()),
            ("2".into(), "The window you click; hover shows".into()),
            ("3".into(), "Only the window the keys pick".into()),
        ],
        true,
    );
    picture_tip(&typing, || Sketch::Focus(number(current("input.follow_mouse").as_ref()).unwrap_or(1.0).clamp(0.0, 3.0) as u8));
    g.add(&typing);
    widgets::keywords("focus follows mouse pointer click to focus hover keys never follow");
    g.add(&tipped(hypr_switch("misc.focus_on_activate", "Apps that ask come forward", "Like a link opened from elsewhere."), Sketch::FocusOnActivate));
    widgets::keywords("focus on activate attention front");
    g.add(&tipped(hypr_switch("cursor.no_warps", "Keep the pointer still", "Moving focus with the keys leaves the pointer where it is."), Sketch::NoWarps));
    widgets::keywords("no warps cursor jump pointer");

    let g = page.group("");
    let n = workspace_rules::persistent();
    let (r, _) = widgets::nav_row(
        Some("view-grid-symbolic"),
        "Workspaces and screens",
        "",
        &match n {
            0 => String::new(),
            1 => "1 always there".into(),
            n => format!("1–{n} always there"),
        },
        || open_sub("Workspaces and screens", workspace_rules::screens_page),
    );
    g.add(&r);
    widgets::keywords("persistent empty always workspaces count monitor screen pin assign display press again back and forth previous");
    let own = workspace_rules::own_count();
    let (r, _) = widgets::nav_row(
        Some("view-paged-symbolic"),
        "Workspaces with their own settings",
        "",
        &if own == 0 { String::new() } else { own.to_string() },
        || open_sub("Workspaces with their own settings", workspace_rules::own_page),
    );
    g.add(&r);
    widgets::keywords("workspace rule layout name gaps decorations floating scrolling override");
    let wc_on = crate::backend::window_controls::enabled(&crate::backend::window_controls::settings());
    let (r, _) = widgets::nav_row(
        Some("window-new-symbolic"),
        "Window controls",
        "",
        if wc_on { "Adaptive" } else { "Off" },
        || open_sub("Window controls", window_controls_page),
    );
    g.add(&r);
    widgets::keywords(
        "window controls close button hover delay popover headerbar titlebar decorations terminal chromeless corner double click maximize minimize float tile per app own titlebar hyprbars",
    );
    let (r, _) = widgets::nav_row(
        Some("starred-symbolic"),
        "Scratchpad and small habits",
        "Swallowing, middle-click paste",
        "",
        || open_sub("Scratchpad and small habits", workspace_rules::scratchpad_page),
    );
    g.add(&r);
    widgets::keywords("scratchpad special workspace hide empty terminal swallowing swallow middle click paste");
    shell(page);

    cap_wrapping(page.body.upcast_ref());

    on_narrow(page, move |narrow| {
        strip.set_visible(!narrow);
        gaps.set_visible(!narrow);
    });
}

/// The split measures the controls' width for their height, and a column of rows only
/// shares out that height to children that expand: wrapped text and the layout cards
/// would get one line's height and ask for the whole line's width, holding the controls
/// column wider than on the other pages. Letting them take the spare height (inside their
/// own column only) keeps it at its width.
fn cap_wrapping(w: &gtk::Widget) {
    if w.is::<gtk::Popover>() {
        return;
    }
    let wraps = w.downcast_ref::<gtk::Label>().is_some_and(|l| l.wraps()) || w.is::<gtk::FlowBox>();
    if wraps
        && let Some(parent) = w.parent().and_downcast::<gtk::Box>()
        && parent.orientation() == gtk::Orientation::Vertical
        && !parent.is_vexpand_set()
    {
        parent.set_vexpand(false);
        w.set_vexpand(true);
        w.set_valign(gtk::Align::Start);
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        cap_wrapping(&ch);
        c = ch.next_sibling();
    }
}

/// Nyx Shell Core's own settings, through it (it owns them): shown when it's installed,
/// saying why when it isn't running.
fn shell(page: &Page) {
    use crate::backend::shell_core;
    if !crate::capability::state("daemon:shell-core").exists() {
        return;
    }
    let g = page.group("Shell");
    if !shell_core::available() {
        let why = crate::capability::reason("daemon:shell-core").unwrap_or_else(|| "Shell Core isn't running".into());
        g.note(&format!("{why}. Its settings show here when it is."));
        return;
    }
    let (config, problems) = match shell_core::config() {
        Ok(c) => c,
        Err(e) => {
            g.note(&gtk::glib::markup_escape_text(&e));
            return;
        }
    };
    for p in problems {
        g.note(&gtk::glib::markup_escape_text(&format!("Shell Core set aside a setting it couldn't use: {p}")));
    }
    let restore = config.pointer("/focus/restore").and_then(serde_json::Value::as_bool).unwrap_or(true);
    let (r, _) = crate::widgets::switch_row(
        "Give focus back when a panel closes",
        "The window you were in takes the keyboard again after the Control Center, Glimpse or another panel closes.",
        restore,
        |on| {
            if let Err(e) = shell_core::set("focus.restore", serde_json::json!(on)) {
                crate::window::toast(&e);
            }
        },
    );
    crate::widgets::keywords("focus restore keyboard panel overlay shell core");
    g.add(&r);
    let yours = nyx_config::Layers::standard()
        .read(nyx_config::Layer::User, "shell")
        .ok()
        .is_some_and(|t| t.get("focus").and_then(|f| f.get("restore")).is_some());
    if yours {
        let (r, _) = crate::widgets::button_row("Focus after panels", "Back to NyxOS's default.", "Reset", |_| {
            match shell_core::reset("focus.restore") {
                Ok(()) => crate::window::rebuild("workspaces"),
                Err(e) => crate::window::toast(&e),
            }
        });
        g.add(&r);
    }
}

#[cfg(test)]
mod tests {
    use super::number;
    use serde_json::json;

    #[test]
    fn reads_numbers_from_hyprland_values() {
        assert_eq!(number(Some(&json!(5))), Some(5.0));
        assert_eq!(number(Some(&json!("5 10 5 10"))), Some(5.0));
        assert_eq!(number(Some(&json!({"custom": "3 3 3 3"}))), Some(3.0));
        assert_eq!(number(None), None);
    }
}
