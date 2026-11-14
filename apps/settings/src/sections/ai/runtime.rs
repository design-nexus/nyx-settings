//! Local AI: whether Nyx AI Core runs, what it holds in memory, how many models stay
//! loaded and for how long, updates, the default models, and coding tools. The preview
//! is who uses it: the apps, Nyx AI Core, and the models it has loaded or apps use.

use super::{art, call, change, has, model_name, size};
use crate::sketch::{self, Rgb};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use serde_json::Value;
use std::rc::Rc;

const PAGE: &str = "local-ai";

fn service(action: &'static str) {
    let units: &[&str] = if action == "start" { &["nyx-aid.socket", "nyx-aid.service"] } else { &["nyx-aid.service"] };
    let mut args = vec!["systemctl", "--user", action];
    args.extend_from_slice(units);
    cmd::run_async(&args, move |r| {
        match r {
            Ok(_) => window::toast(if action == "start" { "Started Nyx AI Core" } else { "Restarted Nyx AI Core" }),
            Err(e) => window::toast(&format!("Couldn't {action} Nyx AI Core: {e}")),
        }
        window::rebuild(PAGE);
    });
}

fn choice(title: &str, desc: &str, options: &[(&str, &str)], current: &str, key: &'static str) -> gtk::Box {
    let options: Vec<(String, String)> = options.iter().map(|(id, l)| (id.to_string(), l.to_string())).collect();
    widgets::choice_row(title, desc, options, current, move |id| change(vec!["service".into(), format!("{key}={id}")], PAGE, "")).0
}

/// The apps that use local models by default: icon, name, the role they use.
const APPS: [(&str, &str, &str); 4] = [
    ("settings-sparkle-symbolic", "Assistant", "assistant"),
    ("chat-symbolic", "Chat", "chat"),
    ("system-search-symbolic", "Search", "embedding"),
    ("code-symbolic", "Coding tools", "coding"),
];

/// How a line in the diagram is drawn.
#[derive(Clone, Copy, PartialEq)]
enum Edge {
    /// A model answering right now.
    Lit,
    /// In use, nothing happening.
    Idle,
    /// Nothing loaded, or switched off.
    Off,
    /// Coding tools: they reach it with the API key.
    Key,
}

/// The lines between a row of `n` nodes and the core, in a box `w`×`h`. `tall`: the
/// nodes are above (`into_core`) or below the core; otherwise left or right of it.
fn draw_edges(cr: &gtk::cairo::Context, p: &sketch::Pal, accent: Rgb, w: f64, h: f64, edges: &[Edge], tall: bool, into_core: bool, core: f64) {
    let n = edges.len();
    if n == 0 {
        return;
    }
    let gap = if tall { 12.0 } else { 10.0 };
    let span = if tall { w } else { h };
    let item = (span - gap * (n as f64 - 1.0)) / n as f64;
    let node_at = |i: usize| i as f64 * (item + gap) + item / 2.0;
    // Where the lines meet the core: spread along its side.
    let mid = span / 2.0;
    let reach = if tall { (core / 2.0 - 34.0).max(0.0) } else { 34.0 };
    let core_at = |i: usize| if n == 1 { mid } else { mid - reach + i as f64 * 2.0 * reach / (n as f64 - 1.0) };
    for (i, e) in edges.iter().enumerate() {
        let (a, b) = (node_at(i), core_at(i));
        // From the node's side to the core's side, as a smooth S.
        let (x1, y1, x2, y2) = match (tall, into_core) {
            (true, true) => (a, 0.0, b, h),
            (true, false) => (b, 0.0, a, h),
            (false, true) => (0.0, a, w, b),
            (false, false) => (0.0, b, w, a),
        };
        if tall {
            cr.move_to(x1, y1);
            cr.curve_to(x1, (y1 + y2) / 2.0, x2, (y1 + y2) / 2.0, x2, y2);
        } else {
            cr.move_to(x1, y1);
            cr.curve_to((x1 + x2) / 2.0, y1, (x1 + x2) / 2.0, y2, x2, y2);
        }
        let violet = super::SWATCHES[1];
        match e {
            Edge::Lit => {
                sketch::set(cr, accent, 0.95);
                cr.set_line_width(2.2);
                cr.set_dash(&[], 0.0);
            }
            Edge::Idle => {
                sketch::set(cr, sketch::mix(p.dim, accent, 0.35), 0.75);
                cr.set_line_width(1.4);
                cr.set_dash(&[], 0.0);
            }
            Edge::Off => {
                sketch::set(cr, p.dim, 0.45);
                cr.set_line_width(1.2);
                cr.set_dash(&[2.0, 4.0], 0.0);
            }
            Edge::Key => {
                sketch::set(cr, violet, 0.9);
                cr.set_line_width(1.6);
                cr.set_dash(&[5.0, 4.0], 0.0);
            }
        }
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        if *e == Edge::Lit {
            sketch::set(cr, accent, 1.0);
            cr.arc((x1 + x2) / 2.0, (y1 + y2) / 2.0, 3.5, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
    }
}

/// A box in the diagram: an icon tile (or a model's tile), a name with an optional
/// badge after it (a pin, a lock) and a line.
fn node(lead: &gtk::Widget, name: &str, line: &str, class: &str, badge: Option<gtk::Image>) -> gtk::Box {
    let b = widgets::hbox(10);
    b.add_css_class("lai-node");
    b.add_css_class(class);
    b.set_valign(gtk::Align::Center);
    lead.set_valign(gtk::Align::Center);
    b.append(lead);
    let text = widgets::vbox(1);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let top = widgets::hbox(5);
    let n = widgets::label(name, "lai-node-name");
    n.set_ellipsize(gtk::pango::EllipsizeMode::End);
    top.append(&n);
    if let Some(badge) = badge {
        badge.set_pixel_size(12);
        badge.set_valign(gtk::Align::Center);
        top.append(&badge);
    }
    text.append(&top);
    let l = widgets::label(line, "lai-node-line");
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&l);
    b.append(&text);
    b
}

/// A node with its tile above its text (in a row of boxes across), or beside it.
fn stack_node(b: &gtk::Box, stacked: bool) {
    b.set_orientation(if stacked { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal });
    b.set_spacing(if stacked { 6 } else { 10 });
    let align = if stacked { gtk::Align::Center } else { gtk::Align::Start };
    if let Some(lead) = b.first_child() {
        lead.set_halign(if stacked { gtk::Align::Center } else { gtk::Align::Start });
    }
    let Some(text) = b.last_child() else { return };
    let mut c = text.first_child();
    while let Some(w) = c {
        w.set_halign(align);
        if let Some(l) = w.downcast_ref::<gtk::Label>() {
            l.set_xalign(if stacked { 0.5 } else { 0.0 });
        }
        c = w.next_sibling();
    }
}

/// A model shown in the diagram: loaded now, or an app's default.
struct Shown {
    name: String,
    line: String,
    edge: Edge,
    kept_by: Vec<String>,
}

pub fn build(page: &Page) {
    let status = match call(&["status"]) {
        Ok(s) => s,
        Err(e) => {
            let b = page.banner(&format!("Nyx AI Core isn't running: {e}"), true);
            let start = gtk::Button::with_label("Start");
            start.set_valign(gtk::Align::Center);
            start.connect_clicked(|b| {
                b.set_sensitive(false);
                service("start");
            });
            b.append(&start);
            return;
        }
    };
    let settings = call(&["service"]).unwrap_or(Value::Null);
    let models = call(&["models"]).unwrap_or(Value::Null);
    let list: Vec<Value> = models.as_array().cloned().unwrap_or_default();
    let worker = &status["worker"];
    let res = &status["resources"];
    let defaults = &settings["defaults"];
    let loaded = res["loaded"].as_array().cloned().unwrap_or_default();
    let name_of = |id: &str| list.iter().find(|m| m["id"].as_str() == Some(id)).map(model_name).unwrap_or_else(|| id.to_string());
    let endpoint = settings["openai_endpoint"].as_str();

    super::watch(page, PAGE, super::signature(&status, &settings));

    let state = worker["state"].as_str().unwrap_or("unknown");
    let release = worker["release"].as_str().unwrap_or("");
    let state_text = match state {
        "ready" => "Running",
        "stopped" => "Waiting for the first request",
        "starting" | "restarting" => "Starting",
        "failed" => "Stopped after repeated crashes",
        "missing" => "llama.cpp isn't installed",
        other => other,
    };

    // ----- The preview: who's using it -----
    let pv = page.split();
    let answering = loaded.iter().filter(|m| m["active"].as_u64().unwrap_or(0) > 0).count();
    let busy = super::pill(&format!("{answering} answering"), "ok");
    busy.set_visible(answering > 0);
    pv.head("Who's using it", "The apps on this computer and the models Nyx AI Core holds for them.", &[busy.upcast_ref()]);

    // The models: loaded ones first, then the ones apps use that aren't loaded.
    let mut shown: Vec<Shown> = loaded
        .iter()
        .map(|m| {
            let id = m["id"].as_str().unwrap_or("");
            let active = m["active"].as_u64().unwrap_or(0) > 0;
            let what = match m["state"].as_str() {
                Some("loading") => "loading",
                _ if active => "answering",
                _ => "idle",
            };
            Shown {
                name: name_of(id),
                line: format!("{} · {what}", size(m["estimate"].as_u64().unwrap_or(0))),
                edge: if active { Edge::Lit } else { Edge::Idle },
                kept_by: m["pinned_by"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect(),
            }
        })
        .collect();
    let loaded_ids: Vec<&str> = loaded.iter().filter_map(|m| m["id"].as_str()).collect();
    for (_, _, role) in APPS {
        let Some(id) = defaults[role].as_str().filter(|id| !id.is_empty()) else { continue };
        let name = name_of(id);
        if loaded_ids.contains(&id) || shown.iter().any(|s| s.name == name) {
            continue;
        }
        let bytes = list.iter().find(|m| m["id"].as_str() == Some(id)).and_then(|m| m["size"].as_u64()).unwrap_or(0);
        shown.push(Shown { name, line: format!("{} · not loaded", size(bytes)), edge: Edge::Off, kept_by: Vec::new() });
    }
    shown.truncate(3);

    let card = pv.card();
    card.add_css_class("lai-diacard");
    let dia = gtk::Box::new(gtk::Orientation::Vertical, 0);
    dia.add_css_class("lai-dia");
    card.append(&dia);

    let apps = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    apps.set_homogeneous(true);
    let mut app_edges = Vec::new();
    let mut app_nodes = Vec::new();
    for (icon, name, role) in APPS {
        let id = defaults[role].as_str().filter(|id| !id.is_empty());
        let in_memory = id.is_some_and(|id| loaded_ids.contains(&id));
        let (line, edge, class) = match (role, id) {
            ("coding", _) if endpoint.is_none() => ("Off".to_string(), Edge::Off, "off"),
            ("coding", Some(id)) => (name_of(id), Edge::Key, "key"),
            ("coding", None) => ("Names its own model".to_string(), Edge::Key, "key"),
            (_, Some(id)) => (name_of(id), if in_memory { Edge::Idle } else { Edge::Off }, if in_memory { "on" } else { "" }),
            (_, None) => ("No model chosen".to_string(), Edge::Off, "off"),
        };
        let lead = widgets::tile(icon);
        let lock = (edge == Edge::Key).then(|| {
            let lock = gtk::Image::from_icon_name("changes-prevent-symbolic");
            lock.add_css_class("lai-key");
            lock.set_tooltip_text(Some("Coding tools need the API key"));
            lock
        });
        let n = node(lead.upcast_ref(), name, &line, "lai-app", lock);
        if !class.is_empty() {
            n.add_css_class(class);
        }
        apps.append(&n);
        app_nodes.push(n);
        app_edges.push(edge);
    }

    let core_w = Rc::new(std::cell::Cell::new(270.0));
    let in_edges = {
        let (e, cw) = (app_edges.clone(), core_w.clone());
        widgets::drawing(52, move |cr, p, accent, w, h| {
            let tall = h < w;
            draw_edges(cr, p, accent, w, h, &e, tall, true, cw.get())
        })
    };
    in_edges.set_hexpand(true);

    // Nyx AI Core itself: its state and where models run.
    let core = widgets::vbox(10);
    core.add_css_class("lai-node");
    core.add_css_class("lai-core");
    core.set_valign(gtk::Align::Center);
    let top = widgets::hbox(10);
    top.append(&widgets::tile("applications-science-symbolic"));
    let text = widgets::vbox(2);
    text.set_valign(gtk::Align::Center);
    text.append(&widgets::label("Nyx AI Core", "lai-core-name"));
    let line = widgets::hbox(6);
    let ok = state == "ready";
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("ai-dot");
    dot.add_css_class(if ok { "ai-dot-ok" } else { "ai-dot-wait" });
    dot.set_valign(gtk::Align::Center);
    line.append(&dot);
    let held = match loaded.len() {
        0 => "nothing loaded".to_string(),
        1 => "1 model loaded".to_string(),
        n => format!("{n} models loaded"),
    };
    let l = widgets::label(&format!("{state_text} · {held}"), "lai-node-line");
    l.set_wrap(true);
    line.append(&l);
    text.append(&line);
    top.append(&text);
    core.append(&top);
    let backends: Vec<String> = res["gpu_backends"].as_array().into_iter().flatten().filter_map(|b| b.as_str().map(str::to_string)).collect();
    let cpu_only = settings["cpu_only"].as_bool().unwrap_or(false);
    let mut runs_on = Vec::new();
    if backends.iter().any(|b| b != "cpu") {
        let names: Vec<String> = backends.iter().filter(|b| *b != "cpu").map(|b| b.to_uppercase()).collect();
        let c = super::chip("video-display-symbolic", "Graphics card", if cpu_only { "" } else { "on" });
        c.set_tooltip_text(Some(&names.join(", ")));
        runs_on.push(c);
    }
    if backends.iter().any(|b| b == "cpu") {
        runs_on.push(super::chip("computer-symbolic", "Processor", if cpu_only || backends.len() == 1 { "on" } else { "" }));
    }
    if !runs_on.is_empty() {
        core.append(&super::chips(runs_on));
    }
    super::tint(&core);

    let model_edges: Vec<Edge> = shown.iter().map(|s| s.edge).collect();
    let out_edges = {
        let (e, cw) = (model_edges.clone(), core_w.clone());
        widgets::drawing(44, move |cr, p, accent, w, h| {
            let tall = h < w;
            draw_edges(cr, p, accent, w, h, &e, tall, false, cw.get())
        })
    };
    out_edges.set_hexpand(true);

    let mods = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    mods.set_homogeneous(true);
    for s in &shown {
        let pin = (!s.kept_by.is_empty()).then(|| {
            let pin = gtk::Image::from_icon_name("view-pin-symbolic");
            pin.add_css_class("lai-pin");
            pin.set_tooltip_text(Some(&format!("Kept loaded by {}", s.kept_by.join(", "))));
            pin
        });
        let n = node(art(&s.name, 30).upcast_ref(), &s.name, &s.line, "lai-model", pin);
        n.add_css_class(match s.edge {
            Edge::Lit => "lit",
            Edge::Off => "off",
            _ => "idle",
        });
        mods.append(&n);
    }
    if shown.is_empty() {
        mods.append(&widgets::label("No model is loaded, and no app has one chosen.", "lai-node-line"));
    }

    for w in [apps.upcast_ref::<gtk::Widget>(), in_edges.upcast_ref(), core.upcast_ref(), out_edges.upcast_ref(), mods.upcast_ref()] {
        dia.append(w);
    }
    // Above and below the core in a wide preview; left to right in a narrow window.
    {
        let (dia, apps, mods, core, ie, oe, cw) = (dia.clone(), apps.clone(), mods.clone(), core.clone(), in_edges.clone(), out_edges.clone(), core_w.clone());
        super::on_narrow(&page.body, move |narrow| {
            use gtk::Orientation::{Horizontal, Vertical};
            dia.set_orientation(if narrow { Horizontal } else { Vertical });
            apps.set_orientation(if narrow { Vertical } else { Horizontal });
            mods.set_orientation(if narrow { Vertical } else { Horizontal });
            apps.set_spacing(if narrow { 10 } else { 12 });
            mods.set_spacing(if narrow { 10 } else { 12 });
            apps.set_size_request(if narrow { 150 } else { -1 }, -1);
            mods.set_size_request(if narrow { 165 } else { -1 }, -1);
            for n in &app_nodes {
                stack_node(n, !narrow);
            }
            let width = if narrow { 240 } else { 280 };
            cw.set(width as f64);
            core.set_size_request(width, -1);
            core.set_halign(gtk::Align::Center);
            for (e, h) in [(&ie, 52), (&oe, 44)] {
                e.set_content_height(if narrow { 1 } else { h });
                e.set_content_width(if narrow { 44 } else { 1 });
                e.set_vexpand(narrow);
                e.queue_draw();
            }
        });
    }

    // Graphics and system memory.
    let mem = pv.card();
    mem.add_css_class("lai-mems");
    let row = widgets::vbox(14);
    let vram_total = res["vram_total"].as_u64().unwrap_or(0);
    let held: u64 = loaded.iter().map(|m| m["estimate"].as_u64().unwrap_or(0)).sum();
    if vram_total > 0 {
        let col = widgets::vbox(8);
        let figure = if held > 0 { format!("About {} of {}", size(held), size(vram_total)) } else { format!("{} · nothing loaded", size(vram_total)) };
        col.append(&super::bar_head("Graphics memory", &figure));
        let parts: Vec<(f64, Option<Rgb>)> = loaded
            .iter()
            .enumerate()
            .map(|(i, m)| (m["estimate"].as_u64().unwrap_or(0) as f64, if i == 0 { None } else { Some(super::SWATCHES[(i - 1) % 4]) }))
            .collect();
        col.append(&super::stacked_bar(parts, vram_total as f64, 8));
        col.set_tooltip_text(Some("What the loaded models hold, as Nyx AI Core estimates it."));
        row.append(&col);
    }
    let (ram_total, ram_free) = (res["ram_total"].as_u64().unwrap_or(0), res["ram_available"].as_u64().unwrap_or(0));
    if ram_total > 0 {
        let col = widgets::vbox(8);
        let used = ram_total.saturating_sub(ram_free);
        col.append(&super::bar_head("System memory", &format!("{} of {} · {} free", size(used), size(ram_total), size(ram_free))));
        col.append(&super::stacked_bar(vec![(used as f64, Some(super::GRAPHITE))], ram_total as f64, 8));
        row.append(&col);
    }
    mem.append(&row);
    let has_mem = row.first_child().is_some();
    mem.set_visible(has_mem);

    let metrics = &worker["metrics"];
    let requests = metrics["requests"].as_u64().unwrap_or(0);
    let foot = if requests > 0 {
        format!(
            "{requests} {} since it started · first word after {:.2} s on average · {:.0} words a second",
            if requests == 1 { "answer" } else { "answers" },
            metrics["first_token_ms_avg"].as_u64().unwrap_or(0) as f64 / 1000.0,
            metrics["tokens_per_second_avg"].as_f64().unwrap_or(0.0)
        )
    } else {
        "No answers since it started.".to_string()
    };
    let foot = pv.foot("power-profile-performance-symbolic", &foot);
    {
        let (mem, foot) = (mem.clone(), foot.clone());
        super::on_narrow(&page.body, move |narrow| {
            mem.set_visible(!narrow && has_mem);
            if let Some(f) = foot.parent() {
                f.set_visible(!narrow);
            }
        });
    }

    // ----- The controls -----
    let g = page.group("");
    let restart = gtk::Button::with_label("Restart");
    restart.set_valign(gtk::Align::Center);
    restart.connect_clicked(|b| {
        b.set_sensitive(false);
        service("restart");
    });
    let mut core_line = format!("{state_text} · llama.cpp {release}");
    if let Some(e) = worker["last_error"].as_str() {
        core_line.push_str(&format!(" · {e}"));
    }
    let r = widgets::icon_row("applications-science-symbolic", "Nyx AI Core", &glib_escape(&core_line), Some(restart.upcast_ref()));
    r.add_css_class("lai-corerow");
    g.add(&r);
    widgets::keywords("status acceleration cuda vulkan cpu memory vram");
    for m in &loaded {
        let id = m["id"].as_str().unwrap_or("").to_string();
        let kept: Vec<&str> = m["pinned_by"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let mut desc = format!(
            "About {} · {} of {} answering",
            size(m["estimate"].as_u64().unwrap_or(0)),
            m["active"].as_u64().unwrap_or(0),
            m["slots"].as_u64().unwrap_or(1)
        );
        if !kept.is_empty() {
            desc.push_str(&format!(" · kept loaded by {}", kept.join(", ")));
        }
        let unload = gtk::Button::with_label("Unload");
        unload.set_valign(gtk::Align::Center);
        let target = id.clone();
        unload.connect_clicked(move |b| {
            b.set_sensitive(false);
            change(vec!["unload".into(), target.clone()], PAGE, "");
        });
        let r = widgets::row(&name_of(&id), &glib_escape(&desc), Some(unload.upcast_ref()));
        r.prepend(&art(&name_of(&id), 30));
        r.add_css_class("with-tile");
        r.set_tooltip_text(Some(&id));
        g.add(&r);
    }

    let g = page.group("Memory");
    let max = settings["max_loaded"].as_u64().unwrap_or(1).to_string();
    let mut counts: Vec<(String, String)> = (1..=4).map(|n| (n.to_string(), n.to_string())).collect();
    if !counts.iter().any(|(id, _)| *id == max) {
        counts.push((max.clone(), max.clone()));
    }
    let seg = widgets::segmented(&counts, &max, |id| change(vec!["service".into(), format!("max_loaded={id}")], PAGE, ""));
    g.add(&widgets::row("Models kept loaded", "More switch faster, use more memory. Every Nyx app shares them.", Some(seg.upcast_ref())));
    g.add(&choice(
        "Unload an unused model after",
        "Unless an app keeps it loaded.",
        &[("-1", "Never"), ("300", "5 minutes"), ("900", "15 minutes"), ("3600", "1 hour")],
        &settings["idle_unload_seconds"].as_i64().unwrap_or(900).to_string(),
        "idle",
    ));
    if has_battery() {
        let on = settings["battery_saver"].as_bool().unwrap_or(false);
        let desc = if cpu_only {
            "On battery, run on the processor: slower, but the battery lasts. Running on battery now."
        } else {
            "On battery, run on the processor: slower, but the battery lasts."
        };
        g.add(&widgets::switch_row("Battery saver", desc, on, |on| {
            change(vec!["service".into(), format!("battery_saver={}", if on { "on" } else { "off" })], PAGE, "")
        })
        .0);
    }

    let g = page.group("Coding tools");
    g.add(&widgets::switch_row(
        "Let coding tools use these models",
        "Tools on this computer that speak the OpenAI API.",
        endpoint.is_some(),
        |on| change(vec!["service".into(), format!("openai={}", if on { "on" } else { "off" })], PAGE, ""),
    )
    .0);
    widgets::keywords("openai compatible api endpoint");
    if let Some(addr) = endpoint {
        let url = format!("http://{addr}/v1");
        let copy = widgets::icon_button("edit-copy-symbolic", "Copy the address");
        let u = url.clone();
        copy.connect_clicked(move |b| {
            b.clipboard().set_text(&u);
            window::toast("Copied the address");
        });
        let r = widgets::row("Address", &format!("<tt>{}</tt>", glib_escape(&url)), Some(copy.upcast_ref()));
        g.add(&r);
        let key = settings["openai_key_file"].as_str().and_then(|f| std::fs::read_to_string(f).ok()).map(|k| k.trim().to_string());
        let copy = gtk::Button::new();
        copy.set_child(Some(&{
            let b = widgets::hbox(6);
            b.append(&gtk::Image::from_icon_name("edit-copy-symbolic"));
            b.append(&gtk::Label::new(Some("Copy key")));
            b
        }));
        copy.set_valign(gtk::Align::Center);
        copy.set_sensitive(key.is_some());
        copy.connect_clicked(move |b| {
            if let Some(key) = &key {
                b.clipboard().set_text(key);
                window::toast("Copied the API key");
            }
        });
        g.add(&widgets::row("API key", "Paste it into the tool as its API key. Only tools that have it can connect.", Some(copy.upcast_ref())));
    }

    let g = page.group("");
    let roles = [
        ("chat", "Chat", "New chats in Chat and other apps that chat.", "chat"),
        ("assistant", "Assistant", "Nyx Assistant's language model.", "chat"),
        ("embedding", "Search and knowledge", "Finds text by meaning: Glimpse, knowledge bases.", "embedding"),
        ("coding", "Coding tools", "What the OpenAI-compatible API answers with when a tool names no model.", "chat"),
    ];
    let set = roles.iter().filter(|(role, ..)| defaults[*role].as_str().is_some_and(|m| !m.is_empty())).count();
    let defaults_now = defaults.clone();
    let (r, _) = widgets::nav_row(Some("view-list-symbolic"), "Which model each app uses", "", &format!("{set} set"), move || {
        let (list, defaults) = (list.clone(), defaults_now.clone());
        widgets::subpage("Which model each app uses", 560, move |page| {
            let g = page.group("Default models");
            g.note("Apps use these when they have no choice of their own.");
            for (role, title, desc, capability) in roles {
                let mut options = vec![(String::new(), "None".to_string())];
                options.extend(
                    list.iter()
                        .filter(|m| has(m, capability) && m["state"].as_str() != Some("missing"))
                        .filter_map(|m| m["id"].as_str().map(|id| (id.to_string(), super::distinct_name(m, &list)))),
                );
                let current = defaults[role].as_str().unwrap_or("");
                let (row, dd) = widgets::choice_row(title, desc, options.clone(), current, move |id| {
                    change(vec!["default".into(), role.into(), id], PAGE, "")
                });
                // The full name of the picked model, as the list shortens it.
                let tip = move |dd: &gtk::DropDown| {
                    let id = options.get(dd.selected() as usize).map(|(id, _)| id.as_str()).unwrap_or("");
                    dd.set_tooltip_text((!id.is_empty()).then_some(id));
                };
                tip(&dd);
                dd.connect_selected_notify(tip);
                g.add(&row);
            }
        });
    });
    g.add(&r);
    widgets::keywords("default models chat assistant search knowledge embedding coding tools");
    let updates = settings["model_updates"].as_str().unwrap_or("auto").to_string();
    let updates_word = match updates.as_str() {
        "notify" => "Tell me",
        "off" => "Off",
        _ => "Automatic",
    };
    let (r, _) = widgets::nav_row(Some("view-refresh-symbolic"), "Model updates", "", updates_word, move || {
        let updates = updates.clone();
        widgets::subpage("Model updates", 520, move |page| {
            let g = page.group("");
            g.add(&choice(
                "Model updates",
                "Newer versions of downloaded models, from Hugging Face. Automatic updates download only while no model is answering and the computer is plugged in.",
                &[("auto", "Automatic"), ("notify", "Tell me"), ("off", "Off")],
                &updates,
                "updates",
            ));
        });
    });
    g.add(&r);
    widgets::keywords("hugging face newer versions automatic");
    widgets::keywords("ai llm llama local model nyx ai core gpu memory vram cuda vulkan unload update default openai api coding tools");
}

fn has_battery() -> bool {
    std::fs::read_dir("/sys/class/power_supply")
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| std::fs::read_to_string(e.path().join("type")).is_ok_and(|t| t.trim() == "Battery"))
}

fn glib_escape(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}
