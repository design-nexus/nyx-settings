//! Whether the assistant runs, how you call it and how a conversation goes, beside a
//! picture of its overlay and of how long it listens. Tasks, privacy and history, the
//! personal dictionary and its helpers each open a page of their own.

use super::{Again, chip, chip_flow, choice, client, flow_add, icon_slider, icon_switch, more, orb, row_text, row_title, run_then, save, slider, switch};
use crate::sketch::{self, R};
use crate::widgets::{self, Page};
use crate::{cmd, paths, window};
use gtk::prelude::*;
use serde_json::Value;
use std::cell::Cell;
use std::rc::Rc;

/// The model's name: its curated name when it's one of those, else its file name.
fn model_name(path: &str) -> String {
    let curated = client::json(&["models", "list", "--json"])
        .and_then(|v| v["curated"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .find(|m| m["path"].as_str() == Some(path))
        .and_then(|m| m["name"].as_str().map(str::to_string));
    curated.unwrap_or_else(|| {
        let file = path.rsplit('/').next().unwrap_or(path);
        file.strip_suffix(".gguf").unwrap_or(file).to_string()
    })
}

fn health(status: Option<&Value>, cfg: &Value, name: &str) -> String {
    let Some(s) = status else {
        if client::running() {
            return "Starting…".into();
        }
        let model = client::get_str(cfg, "llm.model");
        return if model.is_empty() {
            format!("Stopped. Start it to talk to {name}.")
        } else {
            format!("Stopped · {}", model_name(&model))
        };
    };
    let model = s["model"].as_str().map(model_name).unwrap_or_else(|| "No model".into());
    let state = if s["model_loaded"] == true { "ready" } else { "loads when you speak" };
    let muted = if s["muted"] == true { " · microphone off" } else { "" };
    format!("{model} on this computer · {state}{muted}")
}

/// Start, stop or restart the service, then redraw the page.
fn service_button(label: &str, action: &'static str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.connect_clicked(move |b| {
        b.set_sensitive(false);
        cmd::background(
            move || client::service(action),
            move |r| {
                if let Err(e) = r {
                    window::toast(&format!("Couldn't {action} the assistant: {e}"));
                }
                window::rebuild("assistant");
            },
        );
    });
    b
}

/// Run `nyx-assistant ARGS…` off the main thread; redraw the page when it's done.
fn run_then_redraw(args: Vec<String>, failed: &'static str) {
    run_then(args, failed, || window::rebuild("assistant"));
}

/// [`run_then_redraw`], and build the page one level down again too.
fn run_then_both(args: Vec<String>, failed: &'static str, again: &Again) {
    let again = again.clone();
    run_then(args, failed, move || {
        window::rebuild("assistant");
        again();
    });
}

/// (id, name) of each saved persona.
fn personas() -> Vec<(String, String)> {
    let dir = paths::config_home().join("nyx-assistant/personas");
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(String, String)> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension()? != "toml" {
                return None;
            }
            let id = p.file_stem()?.to_string_lossy().to_string();
            let name = std::fs::read_to_string(&p)
                .ok()
                .and_then(|t| t.parse::<toml::Table>().ok())
                .and_then(|t| t.get("name").and_then(|n| n.as_str()).map(str::to_string))
                .unwrap_or_else(|| id.clone());
            Some((id, name))
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// A key name as it's printed on the key.
fn key_name(k: &str) -> String {
    match k.to_uppercase().as_str() {
        "SPACE" => "Space".into(),
        "RETURN" => "Enter".into(),
        "ESCAPE" => "Esc".into(),
        "ALT_R" => "Right Alt".into(),
        "ALT_L" => "Left Alt".into(),
        _ if k.chars().count() == 1 => k.to_uppercase(),
        _ => k.to_string(),
    }
}

/// The keys bound to the assistant, as Hyprland reports them: (what, keys), one per
/// key combination (pressing and releasing push-to-talk are one entry).
fn binds() -> Vec<(String, Vec<String>)> {
    let Ok(text) = cmd::run(&["hyprctl", "binds", "-j"]) else { return Vec::new() };
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    let mods = |m: u64| {
        [(64, "Super"), (4, "Ctrl"), (8, "Alt"), (1, "Shift")]
            .iter()
            .filter(|(bit, _)| m & bit != 0)
            .map(|(_, n)| n.to_string())
            .collect::<Vec<_>>()
    };
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for b in v.as_array().into_iter().flatten() {
        let Some(d) = b["description"].as_str() else { continue };
        let low = d.to_lowercase();
        if !(low.contains("assistant") || low.contains("dictation") || low.contains("ptt")) {
            continue;
        }
        let mut keys = mods(b["modmask"].as_u64().unwrap_or(0));
        keys.push(key_name(b["key"].as_str().unwrap_or("")));
        if !out.iter().any(|(_, k)| *k == keys) {
            out.push((d.to_string(), keys));
        }
    }
    out
}

fn key_caps(keys: &[String]) -> gtk::Box {
    let b = widgets::hbox(3);
    b.add_css_class("asst-keys");
    for k in keys {
        b.append(&widgets::label(k, "key-cap"));
    }
    b
}

const MODES: &[(&str, &str)] = &[("autonomous", "Everything allowed"), ("assisted", "Small changes only"), ("manual", "Asks before any change")];
const AGENTS: &[(&str, &str)] = &[("agy", "Antigravity"), ("claude", "Claude Code"), ("codex", "Codex"), ("cursor", "Cursor")];

/// "8 s", "2 min", "1 min 30 s".
fn span(secs: f64) -> String {
    let s = secs.round() as i64;
    match (s / 60, s % 60) {
        (0, s) => format!("{s} s"),
        (m, 0) => format!("{m} min"),
        (m, s) => format!("{m} min {s} s"),
    }
}

/// Saved dictionary words: (word, place or word) → (spelling, hearing variants).
type Words = std::collections::BTreeMap<(String, String), (String, Vec<String>)>;

fn dictionary_words() -> Words {
    let corrections = client::json(&["dictionary", "list"]).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let mut words: Words = Default::default();
    for correction in corrections {
        let word = correction["corrected"].as_str().unwrap_or("").to_string();
        let scope = correction["context"].as_str().unwrap_or("word").to_string();
        let heard = correction["wrong"].as_str().unwrap_or("").to_string();
        let (_, aliases) = words.entry((word.to_lowercase(), scope)).or_insert_with(|| (word.clone(), Vec::new()));
        if !heard.eq_ignore_ascii_case(&word) {
            aliases.push(heard);
        }
    }
    words
}

// ---------- The preview ----------

/// What the overlay shows in the picture.
#[derive(Clone, Copy, PartialEq)]
enum Shown {
    Listening,
    Answering,
    Asking,
}

/// What the picture needs to know, shared with the controls that change it.
struct Look {
    name: String,
    shown: Cell<Shown>,
    overlay: Cell<bool>,
    wake: Cell<bool>,
    conversation: Cell<bool>,
    follow_up: Cell<f64>,
    idle: Cell<f64>,
}

/// A few bars of a voice, `n` of them across `w`.
fn wave(cr: &gtk::cairo::Context, c: sketch::Rgb, x: f64, cy: f64, w: f64, h: f64, n: usize, seed: u32) {
    let step = w / n as f64;
    for i in 0..n {
        let k = ((i as u32).wrapping_mul(2654435761).wrapping_add(seed.wrapping_mul(40503)) >> 7) % 100;
        let bh = h * (0.18 + k as f64 / 100.0 * 0.82);
        sketch::set(cr, c, 0.9);
        sketch::rounded(cr, x + i as f64 * step, cy - bh / 2.0, (step * 0.5).max(1.5), bh, 1.0);
        let _ = cr.fill();
    }
}

/// A card of the overlay, drawn over the desktop.
fn panel(cr: &gtk::cairo::Context, p: &sketch::Pal, r: R) {
    sketch::set(cr, (0.0, 0.0, 0.0), 0.25);
    sketch::rounded(cr, r.x, r.y + 3.0, r.w, r.h, 10.0);
    let _ = cr.fill();
    sketch::set(cr, sketch::mix(p.bg, p.surface, 0.5), 0.94);
    sketch::rounded(cr, r.x, r.y, r.w, r.h, 10.0);
    let _ = cr.fill_preserve();
    sketch::set(cr, p.text, 0.12);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
}

/// A small microphone glyph centred at (x, y).
fn mic(cr: &gtk::cairo::Context, c: sketch::Rgb, x: f64, y: f64) {
    sketch::set(cr, c, 1.0);
    sketch::rounded(cr, x - 2.5, y - 6.0, 5.0, 8.0, 2.5);
    let _ = cr.fill();
    cr.set_line_width(1.3);
    cr.arc(x, y - 1.0, 4.5, 0.0, std::f64::consts::PI);
    let _ = cr.stroke();
    cr.move_to(x, y + 3.5);
    cr.line_to(x, y + 6.0);
    let _ = cr.stroke();
}

fn draw_overlay(cr: &gtk::cairo::Context, p: &sketch::Pal, c: sketch::Rgb, w: f64, h: f64, look: &Look) {
    use super::{draw_orb, text_at, text_width};
    let area = R::new(1.0, 1.0, w - 2.0, h - 2.0);
    sketch::desktop(cr, p, area);
    if !look.overlay.get() {
        let text = "The overlay is off";
        let tw = text_width(cr, text, 12.0, false) + 24.0;
        panel(cr, p, R::new(area.cx() - tw / 2.0, area.y + area.h - 46.0, tw, 28.0));
        sketch::set(cr, p.dim, 1.0);
        text_at(cr, text, area.cx() - tw / 2.0 + 12.0, area.y + area.h - 32.0, 12.0, false, false);
        return;
    }
    let card_w = (area.w * 0.62).clamp(220.0, 400.0);
    let x = area.cx() - card_w / 2.0;
    let bottom = area.y + area.h - 16.0;
    match look.shown.get() {
        Shown::Listening => {
            let r = R::new(area.cx() - 90.0, bottom - 40.0, 180.0, 40.0);
            panel(cr, p, r);
            draw_orb(cr, p, c, r.x + 22.0, r.cy(), 12.0, true);
            sketch::set(cr, p.text, 1.0);
            text_at(cr, "Listening…", r.x + 42.0, r.cy(), 12.5, true, false);
            wave(cr, c, r.x + 120.0, r.cy(), 48.0, 18.0, 9, 3);
        }
        Shown::Answering => {
            let ans = R::new(x, bottom - 64.0, card_w, 64.0);
            let you = "“Set a timer for ten minutes”";
            let yw = text_width(cr, you, 12.0, false) + 40.0;
            let yr = R::new(area.cx() - yw / 2.0, ans.y - 38.0, yw, 28.0);
            panel(cr, p, yr);
            mic(cr, c, yr.x + 16.0, yr.cy());
            sketch::set(cr, p.text, 0.92);
            text_at(cr, you, yr.x + 28.0, yr.cy(), 12.0, false, false);
            panel(cr, p, ans);
            sketch::set(cr, c, 0.9);
            cr.set_line_width(1.5);
            sketch::rounded(cr, ans.x, ans.y, ans.w, ans.h, 10.0);
            let _ = cr.stroke();
            draw_orb(cr, p, c, ans.x + 26.0, ans.cy(), 14.0, true);
            sketch::set(cr, p.text, 1.0);
            text_at(cr, "Timer set", ans.x + 50.0, ans.cy() - 9.0, 13.0, true, false);
            sketch::set(cr, p.dim, 1.0);
            text_at(cr, "Ten minutes, starting now.", ans.x + 50.0, ans.cy() + 10.0, 12.0, false, false);
            if ans.w > 300.0 {
                wave(cr, c, ans.x + ans.w - 74.0, ans.cy(), 58.0, 26.0, 14, 11);
            }
        }
        Shown::Asking => {
            let r = R::new(x, bottom - 92.0, card_w, 92.0);
            panel(cr, p, r);
            sketch::set(cr, sketch::mix(p.accent, (1.0, 0.75, 0.2), 0.6), 0.9);
            cr.set_line_width(1.5);
            sketch::rounded(cr, r.x, r.y, r.w, r.h, 10.0);
            let _ = cr.stroke();
            draw_orb(cr, p, c, r.x + 26.0, r.y + 28.0, 14.0, true);
            sketch::set(cr, p.text, 1.0);
            text_at(cr, "Open Music?", r.x + 50.0, r.y + 20.0, 13.0, true, false);
            sketch::set(cr, p.dim, 1.0);
            text_at(cr, &format!("{} asks before it does this.", look.name), r.x + 50.0, r.y + 38.0, 12.0, false, false);
            let mut bx = r.x + r.w - 14.0;
            for (label, primary) in [("Allow", true), ("Deny", false)] {
                let bw = text_width(cr, label, 12.0, true) + 24.0;
                bx -= bw;
                sketch::rounded(cr, bx, r.y + r.h - 34.0, bw, 24.0, 6.0);
                sketch::set(cr, if primary { p.accent } else { sketch::mix(p.surface, p.text, 0.12) }, 1.0);
                let _ = cr.fill();
                sketch::set(cr, if primary { p.bg } else { p.text }, 1.0);
                text_at(cr, label, bx + 12.0, r.y + r.h - 22.0, 12.0, true, false);
                bx -= 8.0;
            }
        }
    }
}

/// One conversation as a bar: its name, your question, its answer, listening for a
/// reply and the quiet before it ends, sized by the settings.
fn draw_timeline(cr: &gtk::cairo::Context, p: &sketch::Pal, c: sketch::Rgb, w: f64, h: f64, look: &Look) {
    use super::{text_at, text_width};
    let follow = if look.conversation.get() { look.follow_up.get() } else { 0.0 };
    let idle = look.idle.get();
    let wake = if look.wake.get() { format!("“{}”", look.name) } else { "Key".to_string() };
    let mut parts: Vec<(String, &str, f64, u8)> =
        vec![(wake, "", 1.1, 0), ("Your question".into(), "You", 2.2, 1), ("Its answer".into(), "Answer", 2.6, 2)];
    if follow > 0.0 {
        parts.push(("Listens for a reply".into(), "Listens", 2.0 + (follow / 60.0).min(1.0) * 2.4, 3));
    }
    if look.conversation.get() {
        parts.push(("Quiet".into(), "", 1.4 + (idle / 600.0).min(1.0) * 2.6, 4));
    }
    let total: f64 = parts.iter().map(|p| p.2).sum();
    let (bar_y, bar_h, gap) = (2.0, 28.0, 3.0);
    let usable = w - gap * (parts.len() as f64 - 1.0);
    let mut x = 0.0;
    let mut reply_mid = None;
    for (label, short, flex, kind) in &parts {
        let pw = usable * flex / total;
        sketch::rounded(cr, x, bar_y, pw, bar_h, 5.0);
        let (fill, alpha) = match kind {
            0 => (c, 0.55),
            1 => (sketch::mix(p.accent, p.surface, 0.35), 0.55),
            2 => (sketch::mix(c, p.accent, 0.5), 0.45),
            3 => (c, 0.18),
            _ => (p.surface, 0.9),
        };
        sketch::set(cr, sketch::mix(fill, p.bg, 0.15), alpha);
        let _ = cr.fill_preserve();
        if *kind == 3 {
            cr.save().ok();
            cr.clip_preserve();
            sketch::set(cr, c, 0.35);
            cr.set_line_width(5.0);
            let mut sx = x - bar_h;
            while sx < x + pw {
                cr.move_to(sx, bar_y + bar_h);
                cr.line_to(sx + bar_h, bar_y);
                sx += 11.0;
            }
            let _ = cr.stroke();
            cr.restore().ok();
            sketch::rounded(cr, x, bar_y, pw, bar_h, 5.0);
            sketch::set(cr, c, 0.9);
            cr.set_line_width(1.2);
            let _ = cr.stroke();
            reply_mid = Some(x + pw / 2.0);
        } else {
            cr.new_path();
            if *kind == 4 {
                sketch::rounded(cr, x + 0.5, bar_y + 0.5, pw - 1.0, bar_h - 1.0, 5.0);
                sketch::set(cr, p.dim, 0.35);
                cr.set_line_width(1.0);
                let _ = cr.stroke();
            }
        }
        // The label, or its short form, shortened to fit.
        let mut text = label.clone();
        if !short.is_empty() && text_width(cr, &text, 11.0, false) > pw - 10.0 {
            text = short.to_string();
        }
        while text.chars().count() > 3 && text_width(cr, &text, 11.0, false) > pw - 10.0 {
            let mut chars: Vec<char> = text.trim_end_matches('…').chars().collect();
            chars.pop();
            text = format!("{}…", chars.into_iter().collect::<String>().trim_end());
        }
        if text_width(cr, &text, 11.0, false) <= pw - 6.0 {
            sketch::set(cr, p.text, if *kind == 4 { 0.6 } else { 0.92 });
            let tw = text_width(cr, &text, 11.0, false);
            text_at(cr, &text, x + (pw - tw) / 2.0, bar_y + bar_h / 2.0, 11.0, false, false);
        }
        x += pw + gap;
    }
    sketch::set(cr, p.dim, 1.0);
    let ty = bar_y + bar_h + 12.0;
    let end = if look.conversation.get() { format!("Ends after {} of quiet", span(idle)) } else { "Ends after its answer".to_string() };
    let end_w = text_at(cr, &end, w, ty, 11.0, false, true);
    if let Some(mid) = reply_mid {
        let t = format!("{} for a reply", span(follow));
        let tw = text_width(cr, &t, 11.0, false);
        let x = (mid - tw / 2.0).min(w - end_w - tw - 14.0).max(0.0);
        if x + tw < w - end_w - 8.0 {
            sketch::set(cr, c, 1.0);
            text_at(cr, &t, x, ty, 11.0, false, false);
        }
    }
    let _ = h;
}

// ---------- Pages one level down ----------

fn who_it_is(page: &Page, again: &Again) {
    let cfg = client::config();
    let name = client::get_str(&cfg, "assistant.name");
    let g = page.group("Who it is");
    let ps = personas();
    if !ps.is_empty() {
        let current = client::get_str(&cfg, "assistant.persona_id");
        let pick = ps
            .iter()
            .find(|(id, n)| *id == current || n.eq_ignore_ascii_case(&name))
            .map(|(id, _)| id.clone())
            .unwrap_or_default();
        let again = again.clone();
        let (r, _) = widgets::choice_row("Persona", "Brings its own name, voice, wake words and manner.", ps, &pick, move |id| {
            run_then_both(vec!["persona".into(), "set".into(), id], "Couldn't switch persona", &again);
        });
        g.add(&r);
    }
    let (r, _) = widgets::entry_row("Name", "What it calls itself.", &name, "Echo", |text| {
        if !text.trim().is_empty() {
            save("assistant.name", client::quoted(text.trim()));
            window::rebuild("assistant");
        }
    });
    g.add(&r);
    let phrases: Vec<String> = client::get(&cfg, "wake_word.phrases")
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let (r, e) = widgets::entry_row(
        "Answers to",
        "Names it listens for, comma-separated. Spellings a speech model might hear help.",
        &phrases.join(", "),
        "echo, ekko",
        |text| {
            let items: Vec<String> = text.split(',').map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect();
            save("wake_word.phrases", client::list(&items));
            window::rebuild("assistant");
        },
    );
    e.set_width_chars(24);
    g.add(&r);
}

fn keys_page(page: &Page, _: &Again) {
    let g = page.group("Its keys");
    g.note("Change them on Keyboard Shortcuts.");
    for (what, keys) in binds() {
        g.add(&widgets::row(&what, "", Some(key_caps(&keys).upcast_ref())));
    }
}

fn tasks_page(page: &Page, _: &Again) {
    let cfg = client::config();
    let g = page.group("Tasks");
    g.note("Tasks are requests that take several steps, like “tidy my Downloads folder”.");
    g.add(&choice(&cfg, "agent.mode", "Works on its own", "How much the assistant does without asking first.", MODES));
    g.add(&slider(&cfg, "agent.max_calls", "Steps per task", "The most actions one request may take.", (1.0, 32.0, 1.0), 0, ""));
    g.add(&slider(
        &cfg,
        "agent.max_active_secs",
        "Time per task",
        "Waiting for your approval doesn't count.",
        (10.0, 600.0, 10.0),
        0,
        " s",
    ));
    g.note("What it may do is set on the Permissions page.");
}

fn privacy_page(page: &Page, again: &Again) {
    let cfg = client::config();
    let g = page.group("What leaves this computer");
    g.add(&switch(
        &cfg,
        "location.enabled",
        "Use my location",
        "For the weather: your public IP address goes to ipapi.co to find your city. No GPS.",
    ));
    let web = match client::get_str(&cfg, "permissions.web").as_str() {
        "allow" => "allow",
        "deny" => "deny",
        _ => "confirm",
    };
    let (r, _) = widgets::choice_row(
        "Search the web",
        "Your question goes to the search engine and the sites it finds.",
        vec![("confirm".into(), "Ask each time".into()), ("allow".into(), "Allow".into()), ("deny".into(), "Off".into())],
        web,
        |id| save("permissions.web", client::quoted(&id)),
    );
    g.add(&r);

    let g = page.group("History");
    g.note("Normal logs keep timings and status, never what you said or what it answered.");
    g.add(&switch(
        &cfg,
        "privacy.save_history",
        "Save conversations",
        "So you can search them later. Kept on this computer. Task records, undo copies and learned words are kept separately.",
    ));
    g.add(&slider(
        &cfg,
        "privacy.retention_days",
        "Keep for",
        "Conversations, finished task records and undo copies.",
        (1.0, 365.0, 1.0),
        0,
        " days",
    ));
    g.add(&slider(
        &cfg,
        "privacy.backup_limit_mb",
        "Room for undo copies",
        "Copies of files it changed, so you can undo. The oldest go first.",
        (16.0, 2048.0, 16.0),
        0,
        " MiB",
    ));
    let (row, search) =
        widgets::entry_row("Search saved conversations", "Only conversations saved while history is on.", "", "Search history", |_| {});
    let results = gtk::Label::new(None);
    results.set_wrap(true);
    results.set_xalign(0.0);
    results.set_selectable(true);
    results.add_css_class("asst-results");
    let button = gtk::Button::with_label("Search");
    let out = results.clone();
    button.connect_clicked(move |_| {
        let query = search.text().to_string();
        let out = out.clone();
        cmd::background(
            move || client::json(&["history", &query]).ok_or_else(|| anyhow::anyhow!("History unavailable")),
            move |result| {
                let text = result
                    .ok()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default()
                    .iter()
                    .rev()
                    .take(10)
                    .map(|v| format!("{}\n{}", v["user"].as_str().unwrap_or(""), v["assistant"].as_str().unwrap_or("")))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                out.set_text(if text.is_empty() { "No saved conversations found." } else { &text });
            },
        );
    });
    row.append(&button);
    g.add(&row);
    g.add(&results);

    let g = page.group("Clearing up");
    let buttons = widgets::hbox(8);
    for (label, args) in [("Clear expired", vec!["privacy", "prune"]), ("Record for 10 minutes", vec!["privacy", "diagnostics", "on"]), ("Stop recording", vec!["privacy", "diagnostics", "off"])] {
        let b = gtk::Button::with_label(label);
        let args: Vec<String> = args.into_iter().map(String::from).collect();
        let again = again.clone();
        b.connect_clicked(move |_| run_then_both(args.clone(), "Couldn't update privacy controls", &again));
        buttons.append(&b);
    }
    g.add(&widgets::stacked_row(
        "Look into a problem",
        "Records what it hears and says for 10 minutes, then stops on its own. Clearing history keeps your personal dictionary.",
        buttons.upcast_ref(),
    ));
    let again2 = again.clone();
    let clear = widgets::confirm_button("Delete…", "Delete history and undo copies", move |_| {
        run_then_both(vec!["privacy".into(), "clear".into()], "Couldn't clear stored history", &again2);
    });
    g.add(&widgets::row(
        "Delete everything it has kept",
        "Saved conversations, finished task records and undo copies. The system journal isn't erased.",
        Some(clear.upcast_ref()),
    ));
}

fn dictionary_page(page: &Page, again: &Again) {
    let g = page.group("Add a word");
    g.note("Words and hearing corrections stay on this computer. Strong matches are used straight away; unsure ones get a quick check. Place words are for place names.");
    let (row, word) = widgets::entry_row(
        "Word or name",
        "The right spelling. Add it again with another way it's misheard.",
        "",
        "Calera",
        |_| {},
    );
    word.set_max_length(80);
    g.add(&row);
    let (row, heard) = widgets::entry_row("Heard as", "Optional: what it hears by mistake.", "", "Chalira", |_| {});
    heard.set_max_length(80);
    g.add(&row);
    let context = Rc::new(std::cell::RefCell::new("word".to_string()));
    let context_change = context.clone();
    let (row, _) = widgets::choice_row(
        "Use for",
        "Keep place names apart from everyday words.",
        vec![("word".into(), "Everyday words".into()), ("place".into(), "Place names".into())],
        "word",
        move |id| *context_change.borrow_mut() = id,
    );
    g.add(&row);
    let add = gtk::Button::with_label("Add");
    add.add_css_class("suggested-action");
    let again2 = again.clone();
    add.connect_clicked(move |_| {
        let word = word.text().trim().to_string();
        let heard = heard.text().trim().to_string();
        if word.is_empty() {
            window::toast("Enter a word first.");
            return;
        }
        let mut args = vec!["dictionary".into(), "add".into(), word, "--context".into(), context.borrow().clone()];
        if !heard.is_empty() {
            args.extend(["--heard".into(), heard]);
        }
        run_then_both(args, "Couldn't save dictionary word", &again2);
    });
    g.add(&widgets::row("Save the word", "Used straight away; no restart needed.", Some(add.upcast_ref())));

    let words = dictionary_words();
    let g = page.group("Your words");
    if words.is_empty() {
        g.note("No words yet.");
        return;
    }
    let (search_row, search) = widgets::entry_row("Find a word", "Saved words and how they're misheard.", "", "Search", |_| {});
    g.add(&search_row);
    let mut search_rows = Vec::new();
    for ((_, scope), (word, aliases)) in &words {
        let delete = widgets::icon_button("user-trash-symbolic", "Delete the word and how it's misheard");
        let (w, s, a) = (word.clone(), scope.clone(), again.clone());
        delete.connect_clicked(move |_| run_then_both(vec!["dictionary".into(), "remove".into(), w.clone(), s.clone()], "Couldn't delete word", &a));
        let detail = if aliases.is_empty() {
            if scope == "place" { "Place name".to_string() } else { "Everyday word".to_string() }
        } else {
            format!("{} · heard as {}", if scope == "place" { "Place name" } else { "Everyday word" }, aliases.join(", "))
        };
        let row = widgets::row(word, &gtk::glib::markup_escape_text(&detail), Some(delete.upcast_ref()));
        search_rows.push((row.clone(), format!("{word} {}", aliases.join(" ")).to_lowercase()));
        g.add(&row);
        for alias in aliases {
            let forget = widgets::icon_button("edit-clear-symbolic", "Forget this way it's misheard");
            let (al, s, a) = (alias.clone(), scope.clone(), again.clone());
            forget.connect_clicked(move |_| {
                run_then_both(vec!["corrections".into(), "delete".into(), al.clone(), s.clone()], "Couldn't forget hearing variant", &a)
            });
            let row = widgets::row(&format!("{alias} → {word}"), "Learned", Some(forget.upcast_ref()));
            row.add_css_class("asst-sub-row");
            search_rows.push((row.clone(), format!("{word} {alias}").to_lowercase()));
            g.add(&row);
        }
    }
    search.connect_changed(move |entry| {
        let text = entry.text().to_lowercase();
        for (row, words) in &search_rows {
            row.set_visible(words.contains(&text));
        }
    });
    let again2 = again.clone();
    let clear = widgets::confirm_button("Forget all", "Forget every word", move |_| {
        run_then_both(vec!["corrections".into(), "clear".into()], "Couldn't clear dictionary", &again2);
    });
    g.add(&widgets::row("Forget all words", "Later requests won't use them.", Some(clear.upcast_ref())));
}

fn helpers_page(page: &Page, again: &Again) {
    let cfg = client::config();
    let g = page.group("Helpers");
    g.add(&choice(&cfg, "agent.default_agent", "Coding agent", "Which one it starts when you ask for coding help.", AGENTS));
    let search = if client::get_str(&cfg, "browser.search_tool").is_empty() { "builtin" } else { "duckduckgo" };
    let again = again.clone();
    let (r, _) = widgets::choice_row(
        "Web search",
        "Where it looks up current facts. Neither needs an account.",
        vec![("builtin".into(), "Built in".into()), ("duckduckgo".into(), "DuckDuckGo".into())],
        search,
        move |id| run_then_both(vec!["search-provider".into(), id], "Couldn't change web search", &again),
    );
    g.add(&r);
    let (r, _) = widgets::entry_row(
        "City",
        "Set one to use it instead of the city found from your internet address.",
        &client::get_str(&cfg, "location.city"),
        "Automatic",
        |text| {
            save("location.city", client::quoted(text.trim()));
            window::rebuild("assistant");
        },
    );
    g.add(&r);
}

/// A card that opens a page one level down: its icon, name and current value.
fn more_tile(icon: &str, title: &str, value: &str, words: &str, open: impl Fn() + 'static) -> gtk::Box {
    let card = widgets::vbox(0);
    card.add_css_class("settings-card");
    card.add_css_class("asst-more");
    card.set_overflow(gtk::Overflow::Hidden);
    let (row, _) = widgets::nav_row(Some(icon), title, value, "", open);
    widgets::keywords(words);
    card.append(&row);
    card
}

pub fn build(page: &Page) {
    let Some(cfg) = super::config_or_banner(page) else { return };
    let status = client::status();
    let running = client::running();
    let name = client::get_str(&cfg, "assistant.name");
    let follow = client::get(&cfg, "conversation.follow_up_secs").as_f64().unwrap_or(0.0);
    let idle = client::get(&cfg, "conversation.idle_secs").as_f64().unwrap_or(60.0);
    let look = Rc::new(Look {
        name: name.clone(),
        shown: Cell::new(Shown::Answering),
        overlay: Cell::new(client::get(&cfg, "ui.enabled").as_bool().unwrap_or(false)),
        wake: Cell::new(client::get(&cfg, "wake_word.enabled").as_bool().unwrap_or(false)),
        conversation: Cell::new(client::get(&cfg, "conversation.enabled").as_bool().unwrap_or(false)),
        follow_up: Cell::new(follow),
        idle: Cell::new(idle),
    });

    // The picture: the overlay mid-answer on a small desktop, and one conversation.
    let pv = page.split();
    let overlay: Rc<std::cell::RefCell<Option<gtk::DrawingArea>>> = Rc::default();
    let states = {
        let (look, overlay) = (look.clone(), overlay.clone());
        widgets::segmented(
            &widgets::opts(&[("listening", "Listening"), ("answering", "Answering"), ("asking", "Asking")]),
            "answering",
            move |id| {
                look.shown.set(match id.as_str() {
                    "listening" => Shown::Listening,
                    "asking" => Shown::Asking,
                    _ => Shown::Answering,
                });
                if let Some(a) = overlay.borrow().as_ref() {
                    a.queue_draw();
                }
            },
        )
    };
    pv.head(&format!("Talking to {name}"), "What you'll see on screen, and how long it listens.", &[states.upcast_ref()]);
    let screen = {
        let look = look.clone();
        pv.drawing(300, move |cr, p, c, w, h| draw_overlay(cr, p, c, w, h, &look))
    };
    super::screen_shaped(&screen, 0.58);
    screen.add_css_class("asst-screen");
    *overlay.borrow_mut() = Some(screen.clone());
    let card = widgets::vbox(10);
    card.add_css_class("preview-card");
    let head = widgets::hbox(8);
    head.append(&widgets::label("One conversation", "asst-card-title"));
    let sub = widgets::label("How long it listens, with the settings on the left", "asst-card-line");
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    head.append(&sub);
    card.append(&head);
    let timeline = {
        let look = look.clone();
        widgets::drawing(48, move |cr, p, c, w, h| draw_timeline(cr, p, c, w, h, &look))
    };
    card.append(&timeline);
    pv.add(&card);
    let city = client::get_str(&cfg, "location.city");
    let place = if !client::get(&cfg, "location.enabled").as_bool().unwrap_or(false) {
        ""
    } else if city.is_empty() {
        " The weather uses your city, found from your internet address."
    } else {
        " The weather uses the city you set."
    };
    pv.foot("changes-prevent-symbolic", &format!("What it hears stays on this computer.{place}"));
    let redraw = {
        let (screen, timeline) = (screen.clone(), timeline.clone());
        move || {
            screen.queue_draw();
            timeline.queue_draw();
        }
    };

    // Its state: running or not, the model, and the microphone.
    let g = page.group("");
    let controls = widgets::hbox(6);
    if running {
        controls.append(&service_button("Restart", "restart"));
        let stop = service_button("Stop", "stop");
        stop.add_css_class("flat");
        controls.append(&stop);
    } else {
        let start = service_button("Start", "start");
        start.add_css_class("suggested-action");
        controls.append(&start);
    }
    let r = widgets::row(&name, &health(status.as_ref(), &cfg, &name), Some(controls.upcast_ref()));
    r.add_css_class("asst-status");
    r.prepend(&orb(40, running && status.is_some()));
    if let Some(t) = row_title(&r)
        && let Some(parent) = t.parent().and_downcast::<gtk::Box>()
    {
        let line = widgets::hbox(8);
        parent.insert_child_after(&line, Some(&t));
        parent.remove(&t);
        line.append(&t);
        let (text, class) = match (running, status.is_some()) {
            (true, true) => ("Running", "asst-ok"),
            (true, false) => ("Starting", "asst-wait"),
            _ => ("Stopped", ""),
        };
        line.append(&chip(text, class));
    }
    g.add(&r);
    widgets::keywords("start stop restart service");
    let waiting: Vec<String> = status
        .as_ref()
        .and_then(|s| s["restart_required"].as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if !waiting.is_empty() {
        g.note(&format!("Restart to use the new {}.", gtk::glib::markup_escape_text(&waiting.join(", "))));
    }
    if let Some(s) = &status {
        let sw = gtk::Switch::new();
        sw.set_active(s["muted"] != true);
        sw.connect_active_notify(|sw| {
            run_then_redraw(vec![if sw.is_active() { "unmute" } else { "mute" }.into()], "Couldn't change the microphone");
        });
        g.add(&widgets::icon_row("audio-input-microphone-symbolic", "Microphone", "Off: it doesn't listen, even for its name.", Some(sw.upcast_ref())));
        widgets::keywords("mute unmute");
    }
    let check = gtk::Button::with_label("Check");
    check.connect_clicked(|_| {
        cmd::spawn(&["xdg-terminal-exec", "bash", "-c", "nyx-assistant doctor; echo; read -rp 'Press Enter to close'"]);
    });
    g.add(&widgets::icon_row("emblem-ok-symbolic", "Run a check-up", "Models, sound, browser and search, in a terminal.", Some(check.upcast_ref())));
    widgets::keywords("check everything doctor diagnostics");

    // How you call it.
    let g = page.group("Calling it");
    let (r, _) = {
        let (look, redraw) = (look.clone(), redraw.clone());
        icon_switch(&cfg, "wake_word.enabled", "view-reveal-symbolic", "Listen for its name", "", move |on| {
            look.wake.set(on);
            redraw();
        })
    };
    let phrases: Vec<String> = client::get(&cfg, "wake_word.phrases")
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if let Some(text) = row_text(&r) {
        let line = widgets::hbox(6);
        let lead = widgets::label("Answers to", "settings-option-description");
        lead.set_valign(gtk::Align::Center);
        line.append(&lead);
        let chips = chip_flow();
        chips.set_hexpand(true);
        for p in phrases.iter().take(3) {
            flow_add(&chips, &chip(p, ""));
        }
        if phrases.len() > 3 {
            flow_add(&chips, &chip(&format!("+{}", phrases.len() - 3), "asst-more-chip"));
        }
        line.append(&chips);
        text.append(&line);
    }
    r.set_tooltip_text(Some(
        "The microphone stays on while it waits for its name; what it hears stays on this computer. Off: it listens only while you hold the push-to-talk key.",
    ));
    g.add(&r);
    widgets::keywords("wake word hey listening privacy microphone");
    let (r, _) = widgets::nav_row(Some("avatar-default-symbolic"), "Name and persona", "", &name, || more("Name and persona", who_it_is));
    widgets::keywords("persona name wake phrases aliases answers to");
    g.add(&r);
    let keys = binds();
    if !keys.is_empty() {
        let ptt = keys
            .iter()
            .find(|(d, _)| {
                let d = d.to_lowercase();
                d.contains("assistant") && (d.contains("ptt") || d.contains("push"))
            })
            .or_else(|| keys.iter().find(|(d, _)| d.to_lowercase().contains("assistant")))
            .or(keys.first());
        let end = widgets::hbox(8);
        if let Some((what, combo)) = ptt {
            let caps = key_caps(combo);
            caps.set_tooltip_text(Some(what));
            end.append(&caps);
        }
        let chevron = gtk::Image::from_icon_name("go-next-symbolic");
        chevron.add_css_class("chevron");
        end.append(&chevron);
        let desc = if keys.len() > 1 { format!("Hold to talk. {} more keys.", keys.len() - 1) } else { "Hold to talk.".to_string() };
        let r = widgets::icon_row("input-keyboard-symbolic", &format!("Talk to {name}"), &desc, Some(end.upcast_ref()));
        r.add_css_class("nav-row");
        r.set_cursor_from_name(Some("pointer"));
        let click = gtk::GestureClick::new();
        click.connect_released(|_, _, _, _| more("Its keys", keys_page));
        r.add_controller(click);
        g.add(&r);
        widgets::keywords("shortcut hotkey push to talk keybinding keys dictation");
    }

    // How a conversation goes.
    let g = page.group("Conversation");
    let (r, _) = {
        let (look, redraw) = (look.clone(), redraw.clone());
        icon_switch(
            &cfg,
            "conversation.enabled",
            "user-available-symbolic",
            "Keep the conversation going",
            "Keep listening after an answer.",
            move |on| {
                look.conversation.set(on);
                redraw();
            },
        )
    };
    g.add(&r);
    widgets::keywords("conversation follow up");
    let r = {
        let (look, redraw) = (look.clone(), redraw.clone());
        icon_slider(
            &cfg,
            "conversation.follow_up_secs",
            "view-refresh-symbolic",
            "Listen for a reply",
            "Answer back without its name.",
            (0.0, 60.0, 1.0),
            0,
            " s",
            move |v, _| {
                look.follow_up.set(v);
                redraw();
            },
        )
    };
    g.add(&r);
    let mut ends: Vec<(String, String)> =
        [30, 60, 120, 300, 600].iter().map(|s| (s.to_string(), format!("{} of quiet", span(*s as f64)))).collect();
    let now = (idle.round() as i64).to_string();
    if !ends.iter().any(|(id, _)| *id == now) {
        ends.push((now.clone(), format!("{} of quiet", span(idle))));
        ends.sort_by_key(|(id, _)| id.parse::<i64>().unwrap_or(0));
    }
    let (r, _) = {
        let (look, redraw) = (look.clone(), redraw.clone());
        widgets::choice_row("End a conversation after", "", ends, &now, move |id| {
            look.idle.set(id.parse().unwrap_or(60.0));
            redraw();
            save("conversation.idle_secs", id);
        })
    };
    r.prepend(&widgets::tile("alarm-symbolic"));
    r.add_css_class("with-tile");
    g.add(&r);
    widgets::keywords("idle quiet ends timeout");
    let (r, _) = {
        let (look, redraw) = (look.clone(), redraw.clone());
        icon_switch(&cfg, "ui.enabled", "focus-windows-symbolic", "Show the overlay", "What it hears, says and asks.", move |on| {
            look.overlay.set(on);
            redraw();
        })
    };
    g.add(&r);
    widgets::keywords("hud overlay on screen");

    // The rest, a page each.
    let g = page.plain_group("");
    let mode = client::get_str(&cfg, "agent.mode");
    let mode = MODES.iter().find(|(id, _)| *id == mode).map(|(_, l)| *l).unwrap_or("Everything allowed");
    let history = if client::get(&cfg, "privacy.save_history").as_bool().unwrap_or(false) {
        format!("History kept {} days", client::get(&cfg, "privacy.retention_days").as_i64().unwrap_or(30))
    } else {
        "History off".to_string()
    };
    let words = dictionary_words().len();
    let words = match words {
        0 => "No words yet".to_string(),
        1 => "1 word".to_string(),
        n => format!("{n} words"),
    };
    let agent = client::get_str(&cfg, "agent.default_agent");
    let agent = AGENTS.iter().find(|(id, _)| *id == agent).map(|(_, l)| l.to_string()).unwrap_or(agent);
    let tiles = [
        more_tile("emblem-ok-symbolic", "Tasks", mode, "autonomy autonomous assisted manual agent budget steps time per task works on its own", || {
            more("Tasks", tasks_page)
        }),
        more_tile(
            "security-high-symbolic",
            "Privacy and history",
            &history,
            "privacy location ip web search internet retention history diagnostics content logging clear delete storage undo backups save conversations",
            || more("Privacy and history", privacy_page),
        ),
        more_tile(
            "accessories-dictionary-symbolic",
            "Personal dictionary",
            &words,
            "dictionary vocabulary speech understanding spelling learned corrections forget memory did you mean best guess",
            || more("Personal dictionary", dictionary_page),
        ),
        more_tile("starred-symbolic", "Helpers", &agent, "coding agent claude codex cursor antigravity search duckduckgo location city weather", || {
            more("Helpers", helpers_page)
        }),
    ];
    // Rows of two boxes rather than a grid: a grid measures its height at its narrowest,
    // where every word of a tile wraps.
    let grid = widgets::vbox(10);
    for pair in tiles.chunks(2) {
        let line = widgets::hbox(10);
        line.set_homogeneous(true);
        for t in pair {
            line.append(t);
        }
        grid.append(&line);
    }
    g.add(&grid);
}
