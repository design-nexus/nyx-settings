//! Nyx AI Core, the shared local model runtime: its status, how it runs, default
//! models, and the models themselves. Everything goes through the `nyx-ai` command
//! (JSON out), so AI Core checks every change and Settings never edits its files.
//! The parts the Assistant & AI pages share (pills, model tiles, memory bars) are here too.

pub mod models;
pub mod runtime;

use crate::sketch::{self, Rgb};
use crate::widgets;
use crate::{cmd, window};
use gtk::prelude::*;
use serde_json::Value;

pub const PROGRAM: &str = "nyx-ai";

/// Run `nyx-ai ARGS… --json`. The error is AI Core's own message.
pub fn call(args: &[&str]) -> Result<Value, String> {
    let out = std::process::Command::new(PROGRAM)
        .args(args)
        .arg("--json")
        .output()
        .map_err(|e| format!("couldn't run {PROGRAM}: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    match serde_json::from_str::<Value>(text.trim()) {
        Ok(v) if v["error"].is_object() => Err(v["error"]["message"].as_str().unwrap_or("Nyx AI Core refused").to_string()),
        Ok(v) if out.status.success() => Ok(v),
        _ if out.status.success() => Ok(Value::Null),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("nyx-ai: ").to_string()),
    }
}

/// Run a change off the main thread, then redraw `page` (or say why it failed).
pub fn change(args: Vec<String>, page: &'static str, done: &'static str) {
    cmd::background(
        move || {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            call(&refs)
        },
        move |r| {
            match r {
                Ok(_) if !done.is_empty() => window::toast(done),
                Ok(_) => {}
                Err(e) => window::toast(&format!("Couldn't change Nyx AI Core: {e}")),
            }
            window::rebuild(page);
        },
    );
}

/// What a page shows of AI Core's state: the worker, what's loaded and by whom, and
/// the default models. Apps load and unload models while a page is open.
pub fn signature(status: &Value, settings: &Value) -> String {
    let loaded: Vec<String> = status["resources"]["loaded"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| format!("{}|{}|{}|{}", m["id"], m["state"], m["active"].as_u64().unwrap_or(0) > 0, m["pinned_by"]))
        .collect();
    format!("{}|{}|{}", status["worker"]["state"], loaded.join(","), settings["defaults"])
}

/// While `page` shows, re-read AI Core every few seconds and redraw page `id` when
/// [`signature`] changes from `seen`.
pub fn watch(page: &widgets::Page, id: &'static str, seen: String) {
    let body = page.body.downgrade();
    let seen = std::rc::Rc::new(std::cell::RefCell::new(seen));
    let busy = std::rc::Rc::new(std::cell::Cell::new(false));
    gtk::glib::timeout_add_seconds_local(3, move || {
        // A redrawn page is taken out of the window; its replacement watches instead.
        let Some(body) = body.upgrade().filter(|b| b.root().is_some()) else { return gtk::glib::ControlFlow::Break };
        if !body.is_mapped() || busy.replace(true) {
            return gtk::glib::ControlFlow::Continue;
        }
        let (seen, busy) = (seen.clone(), busy.clone());
        cmd::background(
            || signature(&call(&["status"]).unwrap_or(Value::Null), &call(&["service"]).unwrap_or(Value::Null)),
            move |now| {
                busy.set(false);
                if *seen.borrow() != now {
                    *seen.borrow_mut() = now;
                    window::rebuild(id);
                }
            },
        );
        gtk::glib::ControlFlow::Continue
    });
}

/// "5.3 GB", "640 MB".
pub fn size(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", bytes as f64 / 1e6) }
}

/// The capabilities AI Core lists, in words.
pub fn capabilities(m: &Value) -> Vec<&'static str> {
    m["capabilities"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| match c.as_str()? {
            "chat" => Some("Chat"),
            "tools" => Some("Tools"),
            "thinking" => Some("Thinking"),
            "vision" => Some("Vision"),
            "audio" => Some("Audio"),
            "embedding" => Some("Embedding"),
            "draft" => Some("Drafter"),
            _ => None,
        })
        .collect()
}

pub fn has(m: &Value, capability: &str) -> bool {
    m["capabilities"].as_array().is_some_and(|c| c.iter().any(|v| v.as_str() == Some(capability)))
}

/// "16,384".
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A quantization code ("Q4_K_M", "IQ3_S", "BF16").
fn is_quant(token: &str) -> bool {
    let t = token.to_ascii_lowercase();
    let rest = ["ptq", "pq", "iq", "q"].iter().find_map(|p| t.strip_prefix(p));
    rest.is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit())) || matches!(t.as_str(), "f16" | "bf16" | "f32" | "mtp")
}

/// A model's name for people, from a file or repository name: "gemma-4-E4B-it-Q4_K_M.gguf"
/// reads "Gemma 4 E4B". Names that already have spaces are kept.
pub fn plain_name(raw: &str) -> String {
    let raw = raw.trim().trim_end_matches(".gguf");
    // "Gemma 4 E4B (Q4_K_M)": the quantization in brackets goes.
    let raw = match raw.rfind(" (") {
        Some(i) if raw.ends_with(')') => &raw[..i],
        _ => raw,
    };
    if raw.contains(' ') {
        return raw.to_string();
    }
    let words: Vec<String> = raw
        .split('-')
        .filter_map(|t| {
            // "E4B_q4_0": the part before a quantization.
            let t = match t.split_once('_') {
                Some((head, tail)) if is_quant(tail) || is_quant(t) => head,
                _ => t,
            };
            let lower = t.to_ascii_lowercase();
            if t.is_empty() || is_quant(t) || matches!(lower.as_str(), "gguf" | "it" | "qat") {
                return None;
            }
            // "gemma" → "Gemma"; "E4B", "v2" and "Qwen3.5" stay as they are.
            let mut c = t.chars();
            Some(match c.next() {
                Some(f) if t.chars().all(|c| c.is_ascii_lowercase()) => f.to_ascii_uppercase().to_string() + c.as_str(),
                _ => t.to_string(),
            })
        })
        .collect();
    if words.is_empty() { raw.to_string() } else { words.join(" ") }
}

/// The name of a model AI Core lists: its own name, or one made from its repository.
pub fn model_name(m: &Value) -> String {
    let name = m["name"].as_str().unwrap_or("");
    let id = m["id"].as_str().unwrap_or("");
    if name.is_empty() || name == "Hf" {
        let repo = id.split(':').next().unwrap_or(id);
        return plain_name(repo.rsplit('/').next().unwrap_or(repo));
    }
    plain_name(name)
}

/// [`model_name`], with the quantization (then the id) added when another model in
/// `all` has the same name, so a list tells them apart.
pub fn distinct_name(m: &Value, all: &[Value]) -> String {
    let name = model_name(m);
    let same: Vec<&Value> = all.iter().filter(|o| model_name(o) == name).collect();
    if same.len() < 2 {
        return name;
    }
    let q = m["quantization"].as_str().unwrap_or("");
    let with_q = format!("{name} ({q})");
    if !q.is_empty() && same.iter().filter(|o| o["quantization"].as_str() == Some(q)).count() < 2 {
        with_q
    } else {
        format!("{name} ({})", m["id"].as_str().unwrap_or(""))
    }
}

/// "27B" → "27 billion parameters".
pub fn parameters(p: &str) -> String {
    let (n, unit) = p.split_at(p.len().saturating_sub(1));
    match unit {
        "B" => format!("{n} billion parameters"),
        "M" => format!("{n} million parameters"),
        "K" => format!("{n} thousand parameters"),
        _ => format!("{p} parameters"),
    }
}

/// How closely a quantization keeps the original model, in a word.
pub fn quality(q: &str) -> String {
    let t = q.to_ascii_lowercase();
    let bits = ["ptq", "pq", "iq", "q"].iter().find_map(|p| t.strip_prefix(p)).and_then(|r| r.chars().next()).and_then(|c| c.to_digit(10));
    let word = match (t.as_str(), bits) {
        ("f16" | "bf16" | "f32", _) => "Full",
        (_, Some(8..)) => "High",
        (_, Some(5..=7)) => "Good",
        (_, Some(4)) => "Medium",
        (_, Some(3)) => "Low",
        (_, Some(_)) => "Lowest",
        _ => return q.to_string(),
    };
    format!("{word} ({q})")
}

// ---------- Shared parts of the Assistant & AI pages ----------

/// The colours a chart's parts take after the section's own: blue, violet, pink, amber.
pub const SWATCHES: [Rgb; 4] = [(0.30, 0.55, 0.96), (0.58, 0.46, 0.96), (0.84, 0.40, 0.75), (0.96, 0.69, 0.26)];
/// Things that aren't this page's (other apps), as a quiet grey.
pub const GRAPHITE: Rgb = (0.58, 0.60, 0.66);

/// Give `w` the colour of the section it's on (class `section-<key>`, which sets
/// `color`), as row tiles have, so CSS can tint it with `currentColor`.
pub fn tint(w: &impl IsA<gtk::Widget>) {
    w.connect_realize(|w| {
        let key = std::iter::successors(w.parent(), |w| w.parent())
            .find_map(|w| w.css_classes().iter().find_map(|c| c.strip_prefix("colour-").map(str::to_string)));
        if let Some(key) = key {
            w.add_css_class(&format!("section-{key}"));
        }
    });
}

/// A small pill: `ok` ("In memory", with a green dot), `wait` (a grey dot), `off`
/// (no dot), `use` (a check, in the section's colour) or `warn`.
pub fn pill(text: &str, kind: &str) -> gtk::Box {
    let b = widgets::hbox(5);
    b.add_css_class("ai-pill");
    b.add_css_class(&format!("ai-pill-{kind}"));
    b.set_valign(gtk::Align::Center);
    b.set_halign(gtk::Align::End);
    match kind {
        "ok" | "wait" => {
            let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            dot.add_css_class("ai-dot");
            dot.set_valign(gtk::Align::Center);
            b.append(&dot);
        }
        "use" => {
            let i = gtk::Image::from_icon_name("object-select-symbolic");
            i.set_pixel_size(12);
            b.append(&i);
            tint(&b);
        }
        "warn" => {
            let i = gtk::Image::from_icon_name("dialog-warning-symbolic");
            i.set_pixel_size(12);
            b.append(&i);
        }
        _ => {}
    }
    b.append(&gtk::Label::new(Some(text)));
    b
}

/// A chip with an icon: what a model is good at, who uses it.
pub fn chip(icon: &str, text: &str, class: &str) -> gtk::Box {
    let b = widgets::hbox(6);
    b.add_css_class("ai-chip");
    if !class.is_empty() {
        b.add_css_class(class);
    }
    if class == "on" {
        tint(&b);
    }
    if !icon.is_empty() {
        let i = gtk::Image::from_icon_name(icon);
        i.set_pixel_size(13);
        b.append(&i);
    }
    b.append(&gtk::Label::new(Some(text)));
    b
}

/// Chips that wrap onto more lines when they don't fit.
pub fn chips(items: Vec<gtk::Box>) -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_column_spacing(6);
    f.set_row_spacing(6);
    f.set_max_children_per_line(8);
    f.set_homogeneous(false);
    f.add_css_class("ai-chips");
    for c in items {
        // As wide as its text, not as its column.
        c.set_halign(gtk::Align::Start);
        f.insert(&c, -1);
    }
    f
}

/// A model's tile: a cube on a gradient picked from its name, so each model keeps its colour.
pub fn art(seed: &str, size: i32) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("ai-art");
    let n = seed.bytes().fold(7u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32)) % 6;
    b.add_css_class(&format!("ai-art-{n}"));
    b.set_size_request(size, size);
    b.set_valign(gtk::Align::Center);
    b.set_halign(gtk::Align::Start);
    b.set_hexpand(false);
    let i = gtk::Image::from_icon_name("package-x-generic-symbolic");
    i.set_pixel_size((size / 2).max(14));
    i.set_hexpand(true);
    i.set_halign(gtk::Align::Center);
    i.set_valign(gtk::Align::Center);
    b.append(&i);
    b
}

/// Run `f` with whether the page is narrow now, and again each time that changes.
pub fn on_narrow(body: &gtk::Box, f: impl Fn(bool) + 'static) {
    f(body.has_css_class("narrow"));
    let last = std::cell::Cell::new(body.has_css_class("narrow"));
    body.connect_notify_local(Some("css-classes"), move |b, _| {
        let now = b.has_css_class("narrow");
        if now != last.replace(now) {
            f(now);
        }
    });
}

/// A bar split into parts (`(amount, colour)`, no colour: the section's) out of
/// `total`; what's left is the track.
pub fn stacked_bar(parts: Vec<(f64, Option<Rgb>)>, total: f64, height: i32) -> gtk::DrawingArea {
    widgets::drawing(height, move |cr, p, accent, w, h| {
        let total = total.max(parts.iter().map(|(v, _)| *v).sum::<f64>()).max(1e-9);
        sketch::set(cr, sketch::mix(p.bg, p.text, 0.10), 1.0);
        sketch::rounded(cr, 0.0, 0.0, w, h, h / 2.0);
        let _ = cr.fill();
        cr.save().ok();
        sketch::rounded(cr, 0.0, 0.0, w, h, h / 2.0);
        cr.clip();
        let mut x = 0.0;
        for (v, c) in &parts {
            let pw = w * v / total;
            if pw <= 0.0 {
                continue;
            }
            sketch::set(cr, c.unwrap_or(accent), 1.0);
            cr.rectangle(x, 0.0, (pw - 2.0).max(1.0), h);
            let _ = cr.fill();
            x += pw;
        }
        cr.restore().ok();
    })
}

/// The colour of a legend's swatch.
#[derive(Clone, Copy)]
pub enum Swatch {
    /// The section's own colour.
    Section,
    /// The empty track: what's free.
    Free,
    Of(Rgb),
}

/// A legend entry: a swatch, a name and a small amount.
pub fn legend_item(colour: Swatch, name: &str, amount: &str) -> gtk::Box {
    let b = widgets::hbox(6);
    b.add_css_class("ai-legend");
    let dot = widgets::drawing(8, move |cr, p, accent, w, h| {
        let c = match colour {
            Swatch::Section => accent,
            Swatch::Free => sketch::mix(p.bg, p.text, 0.22),
            Swatch::Of(c) => c,
        };
        sketch::set(cr, c, 1.0);
        sketch::rounded(cr, 0.0, 0.0, w, h, 2.5);
        let _ = cr.fill();
    });
    dot.set_content_width(8);
    dot.set_hexpand(false);
    dot.set_valign(gtk::Align::Center);
    b.append(&dot);
    b.append(&widgets::label(name, "ai-legend-name"));
    if !amount.is_empty() {
        b.append(&widgets::label(amount, "ai-legend-amount"));
    }
    b
}

/// A title on the left and a figure on the right, over a bar.
pub fn bar_head(title: &str, figure: &str) -> gtk::Box {
    let r = widgets::hbox(10);
    let t = widgets::label(title, "ai-bar-title");
    t.set_hexpand(true);
    r.append(&t);
    let f = widgets::label(figure, "ai-bar-figure");
    f.set_xalign(1.0);
    f.set_wrap(true);
    r.append(&f);
    r
}

/// A short line with an icon before it, inside a card.
pub fn icon_line(icon: &str, text: &str, class: &str) -> gtk::Box {
    let b = widgets::hbox(8);
    b.add_css_class("ai-line");
    if !class.is_empty() {
        b.add_css_class(class);
    }
    let i = gtk::Image::from_icon_name(icon);
    i.set_pixel_size(14);
    i.set_valign(gtk::Align::Start);
    b.append(&i);
    let l = widgets::label(text, "ai-line-text");
    l.set_wrap(true);
    l.set_hexpand(true);
    b.append(&l);
    b
}

/// A preview card standing in for what can't be shown yet.
pub fn empty_card(icon: &str, title: &str, text: &str) -> gtk::Box {
    let c = widgets::vbox(6);
    c.add_css_class("preview-card");
    c.add_css_class("ai-empty");
    let i = gtk::Image::from_icon_name(icon);
    i.set_pixel_size(28);
    c.append(&i);
    let t = widgets::label(title, "ai-empty-title");
    t.set_xalign(0.5);
    t.set_justify(gtk::Justification::Center);
    t.set_wrap(true);
    c.append(&t);
    let l = widgets::label(text, "ai-empty-text");
    l.set_xalign(0.5);
    l.set_justify(gtk::Justification::Center);
    l.set_wrap(true);
    c.append(&l);
    c
}

/// Clicking anywhere on `row` runs `f` (for rows that pick something).
pub fn on_click(row: &impl IsA<gtk::Widget>, f: impl Fn() + 'static) {
    row.set_cursor_from_name(Some("pointer"));
    let click = gtk::GestureClick::new();
    click.connect_released(move |g, _, x, y| {
        // A click on a button inside the row is the button's own.
        let inner = g.widget().and_then(|w| w.pick(x, y, gtk::PickFlags::DEFAULT));
        if inner.is_some_and(|w| std::iter::successors(Some(w), |w| w.parent()).any(|w| w.is::<gtk::Button>() || w.is::<gtk::DropDown>())) {
            return;
        }
        f();
    });
    row.add_controller(click);
}

pub fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        assert_eq!(size(5_300_000_000), "5.3 GB");
        assert_eq!(size(640_000_000), "640 MB");
        let m: Value = serde_json::from_str(r#"{"capabilities":["chat","vision","nope"]}"#).unwrap();
        assert_eq!(capabilities(&m), ["Chat", "Vision"]);
        assert!(has(&m, "vision") && !has(&m, "embedding"));
        assert_eq!(thousands(16384), "16,384");
        assert_eq!(thousands(262144), "262,144");
        assert_eq!(thousands(512), "512");
    }

    #[test]
    fn names() {
        assert_eq!(plain_name("gemma-4-E4B-it-Q4_K_M.gguf"), "Gemma 4 E4B");
        assert_eq!(plain_name("gemma-4-E4B_q4_0-it.gguf"), "Gemma 4 E4B");
        assert_eq!(plain_name("Gemma 4 E4B (Q4_K_M)"), "Gemma 4 E4B");
        assert_eq!(plain_name("Qwen3.5-9B-Q4_K_M.gguf"), "Qwen3.5 9B");
        assert_eq!(plain_name("embeddinggemma-2"), "Embeddinggemma 2");
        let m: Value = serde_json::from_str(r#"{"id":"google/gemma-4-E4B-it-qat-q4_0-gguf:IT","name":"Hf"}"#).unwrap();
        assert_eq!(model_name(&m), "Gemma 4 E4B");
        assert_eq!(parameters("27B"), "27 billion parameters");
        assert_eq!(parameters("271M"), "271 million parameters");
        assert_eq!(quality("Q4_K_M"), "Medium (Q4_K_M)");
        assert_eq!(quality("PQ2_0"), "Lowest (PQ2_0)");
        assert_eq!(quality("BF16"), "Full (BF16)");
    }
}
