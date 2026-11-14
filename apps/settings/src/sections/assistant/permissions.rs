//! What the assistant may do: allowed, asks first, or never. The controls are a column
//! of three-way switches beside a map of everything it can reach.

use super::{Again, chip_flow, client, flow_add, more, row_text, save, seg_pick, seg_row, swatch, when};
use crate::sketch::{self, R};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const HELPER: &str = "/usr/local/lib/nyx-assistant/nyx-assistant-helper";
const HELPER_INSTALL: &str = "sudo nyx-assistant install-helper";

/// One thing it can do.
struct Cap {
    key: &'static str,
    icon: &'static str,
    title: &'static str,
    desc: &'static str,
    /// More about it, when you point at its row.
    tip: &'static str,
    /// The audit log's name for it.
    capability: &'static str,
    /// Only asks first or never (it always needs your password).
    ask_only: bool,
    words: &'static str,
    /// Where it sits on the map, 0–1 across and down.
    at: (f64, f64),
}

const CAPS: &[Cap] = &[
    Cap {
        key: "permissions.launch_app",
        icon: "application-x-executable-symbolic",
        title: "Open apps",
        desc: "",
        tip: "",
        capability: "app_launch",
        ask_only: false,
        words: "launch start apps",
        at: (0.5, 0.09),
    },
    Cap {
        key: "permissions.volume",
        icon: "audio-volume-high-symbolic",
        title: "Volume",
        desc: "",
        tip: "",
        capability: "audio",
        ask_only: false,
        words: "change the volume sound mute",
        at: (0.83, 0.22),
    },
    Cap {
        key: "permissions.files_read",
        icon: "x-office-document-symbolic",
        title: "Read files",
        desc: "",
        tip: "Look inside your files.",
        capability: "file_read",
        ask_only: false,
        words: "files read look inside",
        at: (0.9, 0.54),
    },
    Cap {
        key: "permissions.files_write",
        icon: "document-edit-symbolic",
        title: "Change files",
        desc: "",
        tip: "Create, edit, move and delete files.",
        capability: "file_write",
        ask_only: false,
        words: "files write edit move delete",
        at: (0.77, 0.86),
    },
    Cap {
        key: "permissions.shutdown",
        icon: "system-shutdown-symbolic",
        title: "Shut down",
        desc: "",
        tip: "Shut down or restart the computer.",
        capability: "shutdown",
        ask_only: false,
        words: "shut down and restart reboot power off",
        at: (0.23, 0.86),
    },
    Cap {
        key: "permissions.admin",
        icon: "system-software-install-symbolic",
        title: "Software",
        desc: "Asks for your password too.",
        tip: "Install, remove and update software, and start or stop system services.",
        capability: "privileged",
        ask_only: true,
        words: "install remove and update software admin sudo services password",
        at: (0.1, 0.54),
    },
    Cap {
        key: "permissions.web",
        icon: "web-browser-symbolic",
        title: "Search the web",
        desc: "Your question goes out.",
        tip: "",
        capability: "web",
        ask_only: false,
        words: "web search internet browser",
        at: (0.17, 0.22),
    },
];

/// The config spells "asks first" as confirm or ask.
fn decision(cfg: &Value, key: &str) -> String {
    match client::get_str(cfg, key).as_str() {
        "ask" => "confirm".into(),
        "allow" => "allow".into(),
        "deny" => "deny".into(),
        _ => "confirm".into(),
    }
}

fn decisions(ask_only: bool) -> Vec<(String, String)> {
    let mut v = vec![];
    if !ask_only {
        v.push(("allow".to_string(), "Allowed".to_string()));
    }
    v.push(("confirm".into(), "Asks first".into()));
    v.push(("deny".into(), "Never".into()));
    v
}

fn decision_word(d: &str) -> &'static str {
    match d {
        "allow" => "Allowed",
        "deny" => "Never",
        _ => "Asks first",
    }
}

/// Allowed, asks first and never as the theme draws them.
fn decision_rgb(d: &str) -> sketch::Rgb {
    let p = sketch::pal();
    match d {
        "allow" => swatch("green", p.accent),
        "deny" => p.danger,
        _ => swatch("amber", p.accent),
    }
}

// ---------- The map ----------

struct Map {
    name: String,
    states: RefCell<Vec<String>>,
    hover: Cell<Option<usize>>,
}

/// Where each thing's box is on a map `w` × `h`.
fn node_rects(cr: &gtk::cairo::Context, w: f64, h: f64) -> Vec<R> {
    CAPS.iter()
        .map(|c| {
            let tw = super::text_width(cr, c.title, 12.0, true).max(super::text_width(cr, "Asks first", 11.0, false));
            let (bw, bh) = (tw + 50.0, 40.0);
            let x = (c.at.0 * w - bw / 2.0).min(w - bw - 2.0).max(2.0);
            let y = (c.at.1 * h - bh / 2.0).min(h - bh - 2.0).max(2.0);
            R::new(x, y, bw, bh)
        })
        .collect()
}

fn draw_map(cr: &gtk::cairo::Context, p: &sketch::Pal, c: sketch::Rgb, w: f64, h: f64, map: &Map) {
    use super::text_at;
    let states = map.states.borrow();
    let rects = node_rects(cr, w, h);
    let (cx, cy) = (w * 0.5, h * 0.52);
    // The lines: solid for allowed, dashed for asks first, dotted for never.
    for (i, r) in rects.iter().enumerate() {
        let d = states[i].as_str();
        let col = decision_rgb(d);
        let hot = map.hover.get() == Some(i);
        sketch::set(cr, col, if hot { 1.0 } else { 0.8 });
        cr.set_line_width(if hot { 2.4 } else { 1.6 });
        match d {
            "allow" => cr.set_dash(&[], 0.0),
            "deny" => cr.set_dash(&[1.5, 4.0], 0.0),
            _ => cr.set_dash(&[6.0, 5.0], 0.0),
        }
        cr.move_to(cx, cy);
        cr.line_to(r.cx(), r.cy());
        let _ = cr.stroke();
    }
    cr.set_dash(&[], 0.0);
    // The assistant in the middle.
    super::draw_orb(cr, p, c, cx, cy, 30.0, true);
    sketch::set(cr, p.text, 1.0);
    let nw = super::text_width(cr, &map.name, 12.0, true);
    text_at(cr, &map.name, cx - nw / 2.0, cy + 40.0, 12.0, true, false);
    // The things it can reach.
    for (i, (r, cap)) in rects.iter().zip(CAPS).enumerate() {
        let d = states[i].as_str();
        let col = decision_rgb(d);
        let hot = map.hover.get() == Some(i);
        sketch::rounded(cr, r.x, r.y, r.w, r.h, 8.0);
        sketch::set(cr, sketch::mix(p.bg, col, if hot { 0.16 } else { 0.08 }), 1.0);
        let _ = cr.fill_preserve();
        sketch::set(cr, col, if hot { 1.0 } else { 0.75 });
        cr.set_line_width(if hot { 2.0 } else { 1.2 });
        let _ = cr.stroke();
        // A tile for its icon, then its name and state.
        sketch::rounded(cr, r.x + 7.0, r.y + 8.0, 24.0, 24.0, 6.0);
        sketch::set(cr, sketch::mix(p.bg, p.text, 0.1), 1.0);
        let _ = cr.fill();
        sketch::set(cr, col, 1.0);
        cr.arc(r.x + 19.0, r.y + 20.0, 4.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        sketch::set(cr, p.text, 1.0);
        text_at(cr, cap.title, r.x + 39.0, r.y + 14.0, 12.0, true, false);
        sketch::set(cr, col, 1.0);
        text_at(cr, decision_word(d), r.x + 39.0, r.y + 28.0, 11.0, false, false);
    }
}

/// The node under (x, y), if any.
fn hit(area: &gtk::DrawingArea, x: f64, y: f64) -> Option<usize> {
    let (w, h) = (area.width() as f64, area.height() as f64);
    let surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 1, 1).ok()?;
    let cr = gtk::cairo::Context::new(&surface).ok()?;
    node_rects(&cr, w, h).iter().position(|r| x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h).map(|i| i.min(CAPS.len() - 1))
}

/// The newest audit record for each capability: (when, tool, how it was decided).
fn last_uses() -> Vec<(String, u64, String, String)> {
    let rows = client::json(&["audit", "--limit", "30", "--json"]).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let mut out: Vec<(String, u64, String, String)> = Vec::new();
    for r in rows {
        let cap = r["capability"].as_str().unwrap_or("").to_string();
        let at = r["at_ms"].as_u64().unwrap_or(0);
        let tool = r["tool"].as_str().unwrap_or("").to_string();
        let how = match (r["decision"].as_str().unwrap_or(""), r["approval_id"].is_string()) {
            ("deny", _) => "not allowed".to_string(),
            (_, true) => "after asking you".to_string(),
            _ => "allowed by your settings".to_string(),
        };
        match out.iter_mut().find(|(c, ..)| *c == cap) {
            Some(e) if e.1 < at => *e = (cap, at, tool, how),
            Some(_) => {}
            None => out.push((cap, at, tool, how)),
        }
    }
    out
}

/// A popover at a node: its three choices, and when it was last used.
fn node_popover(area: &gtk::DrawingArea, i: usize, map: &Rc<Map>, segs: &Rc<RefCell<Vec<gtk::Box>>>, uses: &Rc<Vec<(String, u64, String, String)>>) {
    let cap = &CAPS[i];
    let (w, h) = (area.width() as f64, area.height() as f64);
    let Ok(surface) = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 1, 1) else { return };
    let Ok(cr) = gtk::cairo::Context::new(&surface) else { return };
    let r = node_rects(&cr, w, h)[i];
    let pop = gtk::Popover::new();
    pop.set_parent(area);
    pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(r.x as i32, r.y as i32, r.w as i32, r.h as i32)));
    let col = widgets::vbox(8);
    col.add_css_class("asst-pop");
    col.append(&widgets::label(&format!("{} · {}", cap.title, decision_word(&map.states.borrow()[i]).to_lowercase()), "asst-card-title"));
    let used = uses.iter().find(|(c, ..)| c == cap.capability);
    let line = match used {
        Some((_, at, tool, how)) => format!("Last used {} ({tool}), {how}.", when(*at)),
        None => "Not among its last 30 actions.".to_string(),
    };
    let l = widgets::label(&line, "asst-card-line");
    l.set_wrap(true);
    l.set_max_width_chars(34);
    col.append(&l);
    let opts: Vec<(&str, &str)> =
        if cap.ask_only { vec![("confirm", "Ask"), ("deny", "Never")] } else { vec![("allow", "Allow"), ("confirm", "Ask"), ("deny", "Never")] };
    let current = map.states.borrow()[i].clone();
    let segs = segs.clone();
    let pop2 = pop.clone();
    let opts2: Vec<String> = opts.iter().map(|(id, _)| id.to_string()).collect();
    let seg = widgets::segmented(&widgets::opts(&opts), &current, move |id| {
        if let (Some(s), Some(n)) = (segs.borrow().get(i), opts2.iter().position(|o| *o == id)) {
            seg_pick(s, n);
        }
        pop2.popdown();
    });
    seg.set_halign(gtk::Align::Start);
    col.append(&seg);
    pop.set_child(Some(&col));
    pop.connect_closed(|p| {
        let p = p.clone();
        gtk::glib::idle_add_local_once(move || p.unparent());
    });
    pop.popup();
}

// ---------- Lists of words ----------

/// A list setting as chips: click one to take it off, or add another.
fn word_chips(cfg: &Value, key: &'static str, add_label: &str, ask: (&'static str, &'static str)) -> gtk::FlowBox {
    let items: Vec<String> = client::get(cfg, key)
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let flow = chip_flow();
    let items = Rc::new(RefCell::new(items));
    fill_chips(&flow, key, &items, add_label.to_string(), ask, false);
    flow
}

fn fill_chips(
    flow: &gtk::FlowBox,
    key: &'static str,
    items: &Rc<RefCell<Vec<String>>>,
    add_label: String,
    ask: (&'static str, &'static str),
    all: bool,
) {
    while let Some(c) = flow.first_child() {
        flow.remove(&c);
    }
    const SHOWN: usize = 8;
    let list = items.borrow().clone();
    let refill = {
        let (flow, items, add_label) = (flow.downgrade(), items.clone(), add_label.clone());
        move |all: bool| {
            if let Some(f) = flow.upgrade() {
                fill_chips(&f, key, &items, add_label.clone(), ask, all);
            }
        }
    };
    for (n, word) in list.iter().enumerate() {
        if !all && n >= SHOWN {
            let more = gtk::Button::with_label(&format!("+{} more", list.len() - SHOWN));
            more.add_css_class("asst-chip-button");
            more.add_css_class("asst-more-chip");
            let refill = refill.clone();
            more.connect_clicked(move |_| refill(true));
            flow_add(flow, &more);
            break;
        }
        let b = gtk::Button::with_label(word);
        b.add_css_class("asst-chip-button");
        b.set_tooltip_text(Some(&format!("Take “{word}” off the list")));
        let (items, word, refill) = (items.clone(), word.clone(), refill.clone());
        b.connect_clicked(move |_| {
            items.borrow_mut().retain(|w| *w != word);
            save(key, client::list(&items.borrow()));
            refill(all);
        });
        flow_add(flow, &b);
    }
    let add = gtk::Button::new();
    let inner = widgets::hbox(3);
    inner.append(&gtk::Image::from_icon_name("list-add-symbolic"));
    inner.append(&gtk::Label::new(Some(&add_label)));
    add.set_child(Some(&inner));
    add.add_css_class("asst-chip-button");
    add.add_css_class("asst-add-chip");
    let items = items.clone();
    add.connect_clicked(move |_| {
        let (items, refill) = (items.clone(), refill.clone());
        widgets::ask_text(ask.0, ask.1, "", "Add", move |text| {
            let text = text.trim().to_string();
            if !text.is_empty() && !items.borrow().contains(&text) {
                items.borrow_mut().push(text);
                save(key, client::list(&items.borrow()));
                refill(all);
            }
        });
    });
    flow_add(flow, &add);
}

// ---------- Pages one level down ----------

fn rule_row(tool: String, current: &str, again: &Again) -> gtk::Box {
    let dd = widgets::dropdown(&decisions(false), current);
    let key = format!("mcp.permissions.{tool}");
    let opts = decisions(false);
    dd.connect_selected_notify(move |d| {
        if let Some((id, _)) = opts.get(d.selected() as usize) {
            save(&key, client::quoted(id));
        }
    });
    let remove = widgets::icon_button("user-trash-symbolic", "Remove this rule");
    let (t, again) = (tool.clone(), again.clone());
    remove.connect_clicked(move |_| {
        let key = format!("mcp.permissions.{t}");
        let again = again.clone();
        cmd::background(
            move || client::run(&["config", "unset", &key]),
            move |r| {
                if let Err(e) = r {
                    window::toast(&format!("Couldn't remove the rule: {e}"));
                }
                cmd::forget_ahead();
                window::rebuild("assistant-permissions");
                again();
            },
        );
    });
    let controls = widgets::hbox(6);
    controls.append(&dd);
    controls.append(&remove);
    widgets::row(&tool, "", Some(controls.upcast_ref()))
}

/// Pick a tool and a decision, then Add.
fn add_rule_form(tools: Vec<String>, again: &Again) -> gtk::Box {
    let form = widgets::hbox(8);
    let names: Vec<(String, String)> = tools.iter().map(|t| (t.clone(), t.clone())).collect();
    let tool = widgets::dropdown(&names, "");
    tool.set_enable_search(true);
    tool.set_hexpand(true);
    let choice = widgets::dropdown(&decisions(false), "confirm");
    let add = gtk::Button::with_label("Add");
    add.add_css_class("suggested-action");
    let opts = decisions(false);
    let (t, c, again) = (tool.clone(), choice.clone(), again.clone());
    add.connect_clicked(move |_| {
        let (Some(name), Some((d, _))) = (tools.get(t.selected() as usize).cloned(), opts.get(c.selected() as usize).cloned())
        else {
            window::toast("Pick a tool first.");
            return;
        };
        let key = format!("mcp.permissions.{name}");
        let again = again.clone();
        cmd::background(
            move || client::set(&key, &client::quoted(&d)),
            move |r| {
                if let Err(e) = r {
                    window::toast(&format!("Couldn't add the rule: {e}"));
                }
                window::rebuild("assistant-permissions");
                again();
            },
        );
    });
    form.append(&tool);
    form.append(&choice);
    form.append(&add);
    form
}

fn rules_page(page: &Page, again: &Again) {
    let cfg = client::config();
    let status = client::status();
    let g = page.group("Rules for single tools");
    g.note("A rule for one tool wins over the general setting.");
    let rules = client::get(&cfg, "mcp.permissions").as_object().cloned().unwrap_or_default();
    for (tool, d) in rules {
        let d = match d.as_str().unwrap_or("confirm") {
            "ask" => "confirm",
            d => d,
        };
        g.add(&rule_row(tool, d, again));
    }
    let tools: Vec<String> = status
        .as_ref()
        .and_then(|s| s["mcp_tools"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();
    if tools.is_empty() {
        g.note("Start the assistant to add rules for its tools.");
    } else {
        g.add(&widgets::form_disclosure("Add a rule", "", &add_rule_form(tools, again)));
    }

    let g = page.group("Every tool it has");
    let Some(status) = status else {
        g.note("Start the assistant to see each tool and what it may do.");
        return;
    };
    let (search_row, search) = widgets::entry_row("Find a tool", "By its name.", "", "Search tools", |_| {});
    g.add(&search_row);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    let mut rows = Vec::new();
    for item in status["capability_health"].as_array().into_iter().flatten() {
        let name = item["name"].as_str().unwrap_or("");
        let permission = item["permission"].as_str().unwrap_or("unknown");
        let reason = item["reason"].as_str().unwrap_or("");
        let row = widgets::row(name, &gtk::glib::markup_escape_text(&format!("{} · {reason}", decision_word(permission))), None);
        list.append(&row);
        rows.push((name.to_lowercase(), row));
    }
    search.connect_changed(move |entry| {
        let query = entry.text().to_lowercase();
        for (name, row) in &rows {
            row.set_visible(query.is_empty() || name.contains(&query));
        }
    });
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_max_content_height(360);
    scroll.set_propagate_natural_height(true);
    scroll.set_child(Some(&list));
    g.add(&scroll);
    for error in status["mcp_errors"].as_array().into_iter().flatten() {
        g.note(&gtk::glib::markup_escape_text(&format!("Unavailable: {error}")));
    }
}

fn helper_page(page: &Page, _: &Again) {
    let g = page.group("Administrator helper");
    g.note("It lets the assistant change software and system services, after you type your password.");
    if std::path::Path::new(HELPER).exists() {
        let (r, _) = widgets::info_row("Helper", "Installed");
        g.add(&r);
    } else {
        let (r, _) = widgets::button_row(
            "Not installed",
            &format!("Software and service changes need it. Run <tt>{HELPER_INSTALL}</tt> in a terminal."),
            "Copy",
            |b| {
                b.clipboard().set_text(HELPER_INSTALL);
                window::toast("Copied the command.");
            },
        );
        g.add(&r);
    }
}

fn summary_markup(states: &[String]) -> String {
    let count = |d: &str| states.iter().filter(|s| *s == d).count();
    format!("<b>{}</b> allowed · <b>{}</b> ask first · <b>{}</b> never", count("allow"), count("confirm"), count("deny"))
}

pub fn build(page: &Page) {
    let Some(cfg) = super::config_or_banner(page) else { return };
    let map = Rc::new(Map {
        name: client::get_str(&cfg, "assistant.name"),
        states: RefCell::new(CAPS.iter().map(|c| decision(&cfg, c.key)).collect()),
        hover: Cell::new(None),
    });
    let segs: Rc<RefCell<Vec<gtk::Box>>> = Rc::default();
    let uses = Rc::new(last_uses());

    // The picture: everything it can reach around it, each line in its state.
    let pv = page.split();
    let summary = gtk::Label::new(None);
    summary.add_css_class("asst-card-line");
    summary.set_markup(&summary_markup(&map.states.borrow()));
    pv.head(&format!("What {} can reach", map.name), "Click one to change it.", &[summary.upcast_ref()]);
    let area = {
        let map = map.clone();
        pv.drawing(400, move |cr, p, c, w, h| draw_map(cr, p, c, w, h, &map))
    };
    super::screen_shaped(&area, 0.72);
    area.add_css_class("asst-map");
    let motion = gtk::EventControllerMotion::new();
    {
        let (m, a) = (map.clone(), area.clone());
        motion.connect_motion(move |_, x, y| {
            let h = hit(&a, x, y);
            if h != m.hover.get() {
                m.hover.set(h);
                a.set_cursor_from_name(if h.is_some() { Some("pointer") } else { None });
                a.queue_draw();
            }
        });
        let (map, a) = (map.clone(), area.clone());
        motion.connect_leave(move |_| {
            map.hover.set(None);
            a.queue_draw();
        });
    }
    area.add_controller(motion);
    let click = gtk::GestureClick::new();
    {
        let (map, a, segs, uses) = (map.clone(), area.clone(), segs.clone(), uses.clone());
        click.connect_released(move |_, _, x, y| {
            if let Some(i) = hit(&a, x, y) {
                node_popover(&a, i, &map, &segs, &uses);
            }
        });
    }
    area.add_controller(click);
    let legend = widgets::hbox(14);
    legend.add_css_class("asst-map-legend");
    for (d, text) in [("allow", "Allowed"), ("confirm", "Asks first"), ("deny", "Never")] {
        let key = widgets::hbox(6);
        let line = widgets::drawing(10, move |cr, _, _, w, h| {
            sketch::set(cr, decision_rgb(d), 1.0);
            cr.set_line_width(2.0);
            match d {
                "allow" => cr.set_dash(&[], 0.0),
                "deny" => cr.set_dash(&[1.5, 3.0], 0.0),
                _ => cr.set_dash(&[4.0, 3.0], 0.0),
            }
            cr.move_to(0.0, h / 2.0);
            cr.line_to(w, h / 2.0);
            let _ = cr.stroke();
        });
        line.set_content_width(22);
        line.set_hexpand(false);
        line.set_valign(gtk::Align::Center);
        key.append(&line);
        key.append(&widgets::label(text, "asst-card-line"));
        legend.append(&key);
    }
    let note = widgets::label("Commands it writes itself always ask first.", "asst-card-line");
    note.set_hexpand(true);
    note.set_xalign(1.0);
    note.set_wrap(true);
    legend.append(&note);
    pv.add(&legend);

    // The controls: each thing it can do, as a three-way switch.
    let g = page.group("What it may do");
    for (i, cap) in CAPS.iter().enumerate() {
        let opts: Vec<(&str, &str)> =
            if cap.ask_only { vec![("confirm", "Ask"), ("deny", "Never")] } else { vec![("allow", "Allow"), ("confirm", "Ask"), ("deny", "Never")] };
        let current = map.states.borrow()[i].clone();
        let (map2, area2, summary2) = (map.clone(), area.clone(), summary.clone());
        let (r, seg) = seg_row(cap.icon, cap.title, cap.desc, &opts, &current, move |id| {
            save(cap.key, client::quoted(&id));
            map2.states.borrow_mut()[i] = id;
            summary2.set_markup(&summary_markup(&map2.states.borrow()));
            area2.queue_draw();
        });
        if !cap.tip.is_empty() {
            r.set_tooltip_text(Some(cap.tip));
        }
        widgets::keywords(&format!("permission allow ask confirm deny never {}", cap.words));
        segs.borrow_mut().push(seg);
        g.add(&r);
    }
    g.note("Commands the assistant writes itself always ask first.");

    let g = page.group("Using apps for you");
    let r = widgets::icon_row("view-conceal-symbolic", "Never touch", "", None);
    if let Some(text) = row_text(&r) {
        text.append(&word_chips(&cfg, "computer.deny_classes", "App", (
            "Never touch an app",
            "Its window class, like keepassxc. Password managers, terminals and lock screens are never touched anyway.",
        )));
    }
    r.set_tooltip_text(Some("Password managers, terminals and lock screens are never touched anyway."));
    g.add(&r);
    widgets::keywords("never touch these apps window classes computer use deny");
    let r = widgets::icon_row("input-mouse-symbolic", "Always ask before clicking", "", None);
    if let Some(text) = row_text(&r) {
        text.append(&word_chips(&cfg, "computer.confirm_words", "Word", (
            "Always ask before clicking",
            "Buttons with this word ask first, even during an approved task.",
        )));
    }
    g.add(&r);
    widgets::keywords("always ask before clicking confirm words send buy delete computer use click type");

    let g = page.group("");
    let rules = client::get(&cfg, "mcp.permissions").as_object().map(|m| m.len()).unwrap_or(0);
    let count = match rules {
        0 => "None".to_string(),
        1 => "1 rule".to_string(),
        n => format!("{n} rules"),
    };
    let (r, _) = widgets::nav_row(Some("preferences-system-symbolic"), "Rules for single tools", "", &count, || {
        more("Rules for single tools", rules_page)
    });
    widgets::keywords("tool rule mcp capability health tools");
    g.add(&r);
    let installed = std::path::Path::new(HELPER).exists();
    let (r, _) = widgets::nav_row(
        Some("security-high-symbolic"),
        "Administrator helper",
        "",
        if installed { "Installed" } else { "Not installed" },
        || more("Administrator helper", helper_page),
    );
    widgets::keywords("admin helper polkit sudo install-helper");
    g.add(&r);
}
