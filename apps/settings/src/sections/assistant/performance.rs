//! How fast the last answer came, and what the assistant holds in memory. The preview
//! takes the last answer apart step by step and shows what fills the graphics card;
//! the controls are the preset, what's loaded now, and speed tests and the last problem.

use super::{client, size};
use crate::sections::ai as kit;
use crate::sketch::{self, Rgb};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;

/// The last turn and what's loaded: the page redraws after each answer.
pub fn signature() -> String {
    client::json(&["perf", "--json"])
        .map(|v| format!("{} {} {}", v["last_turn"], v["llm"]["loaded"], v["vision"]["loaded"]))
        .unwrap_or_default()
}

/// The timing stages, measured from when you stopped speaking, in plain words.
fn stage_label(name: &str) -> Option<&'static str> {
    Some(match name {
        "STT complete" => "Your words recognised",
        "routing" => "Decided what to do",
        "LLM first token" => "First word of the answer",
        "TTS begins" => "Voice started",
        "audio playback" => "You hear it",
        _ => return None,
    })
}

/// Each step's colour: the section's, then the swatches.
fn stage_colour(i: usize, accent: Rgb) -> Rgb {
    if i == 0 { accent } else { kit::SWATCHES[(i - 1) % kit::SWATCHES.len()] }
}

/// A tidy step for the time scale under `end` milliseconds.
fn tick_step(end: f64) -> f64 {
    [50.0, 100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0, 2500.0, 5000.0, 10000.0]
        .into_iter()
        .find(|s| end / s <= 5.0)
        .unwrap_or(20000.0)
}

fn tick_text(ms: f64, step: f64) -> String {
    if ms == 0.0 {
        "0".into()
    } else if step >= 1000.0 {
        format!("{:.0} s", ms / 1000.0)
    } else if step >= 100.0 {
        format!("{:.1} s", ms / 1000.0)
    } else {
        format!("{ms:.0} ms")
    }
}

/// The time scale above the steps.
fn axis(end: f64) -> gtk::DrawingArea {
    widgets::drawing(16, move |cr, p, _, w, _| {
        let step = tick_step(end);
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
        cr.set_font_size(10.0);
        sketch::set(cr, p.dim, 0.9);
        let mut t = 0.0;
        while t < end {
            let x = w * t / end;
            let text = tick_text(t, step);
            if let Ok(ext) = cr.text_extents(&text) {
                if x + ext.width() < w {
                    cr.move_to(x, 11.0);
                    let _ = cr.show_text(&text);
                }
            }
            t += step;
        }
    })
}

/// One step's track: the time it took, from where the step before ended.
fn track(from: f64, to: f64, end: f64, colour: usize) -> gtk::DrawingArea {
    let area = widgets::drawing(14, move |cr, p, accent, w, h| {
        sketch::set(cr, sketch::mix(p.bg, p.text, 0.07), 1.0);
        sketch::rounded(cr, 0.0, 0.0, w, h, 4.0);
        let _ = cr.fill();
        let step = tick_step(end);
        sketch::set(cr, p.bg, 0.55);
        let mut t = step;
        while t < end {
            cr.rectangle((w * t / end).round(), 0.0, 1.0, h);
            t += step;
        }
        let _ = cr.fill();
        let (x0, x1) = (w * from / end, w * to / end);
        sketch::set(cr, stage_colour(colour, accent), 1.0);
        sketch::rounded(cr, x0, 0.0, (x1 - x0).max(4.0), h, 4.0);
        let _ = cr.fill();
    });
    area.set_valign(gtk::Align::Center);
    area
}

/// A step's coloured dot.
fn dot(colour: usize) -> gtk::DrawingArea {
    let d = widgets::drawing(8, move |cr, _, accent, w, h| {
        sketch::set(cr, stage_colour(colour, accent), 1.0);
        cr.arc(w / 2.0, h / 2.0, w.min(h) / 2.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    });
    d.set_content_width(8);
    d.set_hexpand(false);
    d.set_valign(gtk::Align::Center);
    d
}

/// The last answer, step by step on one time scale, so the slow step stands out.
fn waterfall(stages: &[(&'static str, u64)], gaps: &Value) -> gtk::Box {
    let card = widgets::vbox(12);
    card.add_css_class("preview-card");
    card.add_css_class("aip-wf");
    let total = stages.last().map(|s| s.1).unwrap_or(0);
    let head = widgets::hbox(10);
    let big = widgets::label(&format!("{:.2} s", total as f64 / 1000.0), "aip-big");
    big.set_valign(gtk::Align::Center);
    head.append(&big);
    let what = widgets::label("from when you stopped speaking until you heard the answer", "aip-what");
    what.set_wrap(true);
    what.set_hexpand(true);
    what.set_valign(gtk::Align::Center);
    head.append(&what);
    card.append(&head);

    let end = (total as f64 * 1.07).max(1.0);
    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(7);
    grid.attach(&axis(end), 1, 0, 1, 1);
    let mut from = 0.0;
    let last = stages.len().saturating_sub(1);
    for (i, (label, ms)) in stages.iter().enumerate() {
        let name = widgets::hbox(8);
        name.append(&dot(i));
        let l = widgets::label(label, if i == last { "aip-step-last" } else { "aip-step" });
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.append(&l);
        grid.attach(&name, 0, i as i32 + 1, 1, 1);
        let t = track(from, *ms as f64, end, i);
        t.set_hexpand(true);
        grid.attach(&t, 1, i as i32 + 1, 1, 1);
        let v = widgets::label(&format!("{ms} ms"), if i == last { "aip-ms-last" } else { "aip-ms" });
        v.set_xalign(1.0);
        v.set_width_chars(7);
        grid.attach(&v, 2, i as i32 + 1, 1, 1);
        from = *ms as f64;
    }
    card.append(&grid);

    let mut note = String::new();
    if let Some(count) = gaps["count"].as_u64() {
        note = match (count, gaps["max_ms"].as_u64()) {
            (0, _) => "No pauses between phrases.".to_string(),
            (1, Some(max)) => format!("One pause between phrases, {max} ms."),
            (n, Some(max)) => format!("{n} pauses between phrases, the longest {max} ms."),
            (n, None) => format!("{n} pauses between phrases."),
        };
    }
    if stages.iter().any(|(l, _)| *l == "First word of the answer") {
        if !note.is_empty() {
            note.push(' ');
        }
        note.push_str("The first word waits on the language model, so a smaller model answers sooner.");
    }
    if !note.is_empty() {
        let n = kit::icon_line("info-symbolic", &note, "aip-note");
        card.append(&n);
    }
    card
}

/// What fills the graphics card: the assistant, other apps, and what's free.
fn vram_card(gpu: &Value) -> Option<gtk::Box> {
    let (used, total) = (gpu["used_mib"].as_u64()?, gpu["total_mib"].as_u64()?);
    let mib = |m: u64| m * 1_048_576;
    let mine = gpu["assistant_mib"].as_u64().map(|m| m.min(used));
    let card = widgets::vbox(10);
    card.add_css_class("preview-card");
    card.add_css_class("aip-vram");
    let figure = match mine {
        Some(m) => format!("{} of {} · the assistant uses {}", size(mib(used)), size(mib(total)), size(mib(m))),
        None => format!("{} of {} used", size(mib(used)), size(mib(total))),
    };
    card.append(&kit::bar_head("Graphics memory", &figure));
    let mut parts = Vec::new();
    if let Some(m) = mine {
        parts.push((m as f64, None));
    }
    parts.push(((used - mine.unwrap_or(0)) as f64, Some(kit::GRAPHITE)));
    card.append(&kit::stacked_bar(parts, total as f64, 12));
    let legend = widgets::hbox(16);
    legend.add_css_class("aip-legend");
    if let Some(m) = mine {
        legend.append(&kit::legend_item(kit::Swatch::Section, "The assistant", &size(mib(m))));
    }
    legend.append(&kit::legend_item(kit::Swatch::Of(kit::GRAPHITE), if mine.is_some() { "Other apps" } else { "In use" }, &size(mib(used - mine.unwrap_or(0)))));
    legend.append(&kit::legend_item(kit::Swatch::Free, "Free", &size(mib(total.saturating_sub(used)))));
    card.append(&legend);
    Some(card)
}

/// The machine as the assistant last measured it, in one line.
fn machine_line(perf: &Value) -> String {
    let gpu = &perf["gpu"];
    let mut bits = Vec::new();
    if let Some(battery) = perf["on_battery"].as_bool() {
        bits.push(if battery { "On battery".to_string() } else { "Plugged in".to_string() });
    }
    if let Some(temp) = perf["cpu_temp_c"].as_f64() {
        bits.push(format!("processor {temp:.0} °C"));
    }
    if gpu["asleep"].as_bool().unwrap_or(false) {
        bits.push("the graphics card is asleep".to_string());
    } else if let Some(temp) = gpu["temp_c"].as_f64() {
        let mut card = format!("graphics card {temp:.0} °C");
        if let Some(w) = gpu["power_w"].as_f64() {
            card.push_str(&format!(", {w:.0} W"));
        }
        if let Some(u) = gpu["util_pct"].as_f64() {
            card.push_str(&format!(", {u:.0}% busy"));
        }
        bits.push(card);
    }
    bits.join(" · ")
}

/// Lite, Balanced or Advanced: one choice that sets the model, its context, where speech
/// runs and when the models unload (`nyx-assistant preset`).
fn presets(page: &Page) {
    let Some(p) = client::json(&["preset", "show", "--json"]) else {
        return;
    };
    let g = page.group("Preset");
    g.note("Sets the model, where speech runs and when models unload.");
    let title = |name: &str| {
        let mut c = name.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    let recommended = p["recommended"].as_str().unwrap_or("").to_string();
    let why = p["why"].as_str().unwrap_or("").to_string();
    let current = p["current"].as_str().unwrap_or("custom").to_string();
    let mut options: Vec<(String, String, String)> = p["presets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| {
            let name = x["name"].as_str()?;
            // What it sets, in a line: the model and how much it remembers, where speech
            // runs, when the models leave memory.
            let mut facts = Vec::new();
            match (x["model"].as_str(), x["ctx"].as_u64()) {
                (Some(m), Some(ctx)) => facts.push(format!("{m}, remembering {} tokens.", kit::thousands(ctx))),
                (Some(m), None) => facts.push(format!("{m}.")),
                _ => {}
            }
            match x["speech_device"].as_str() {
                Some("cuda") => facts.push("Speech runs on the graphics card;".into()),
                Some("cpu") => facts.push("Speech runs on the processor;".into()),
                _ => {}
            }
            match x["idle_seconds"].as_u64() {
                Some(0) => facts.push("the models stay loaded.".into()),
                Some(s) if s % 60 == 0 => facts.push(format!("the models leave memory after {} minute{}.", s / 60, if s == 60 { "" } else { "s" })),
                Some(s) => facts.push(format!("the models leave memory after {s} seconds.")),
                None => {}
            }
            let mut desc = facts.join(" ");
            if desc.ends_with(';') {
                desc.pop();
                desc.push('.');
            }
            if desc.is_empty() {
                desc = x["description"].as_str().unwrap_or("").to_string();
            }
            if x["model_installed"] == false {
                desc.push_str(" Its model isn't downloaded yet.");
            }
            if name == recommended && !why.is_empty() {
                desc.push_str(&format!(" Recommended here, for {why}."));
            }
            Some((name.to_string(), desc, x["description"].as_str().unwrap_or("").to_string()))
        })
        .collect();
    if current == "custom" {
        options.push(("custom".into(), "You've changed a setting yourself, so none of the presets matches. Pick one to go back to it.".into(), String::new()));
    }
    let mut first: Option<gtk::CheckButton> = None;
    for (id, desc, about) in options {
        let check = gtk::CheckButton::new();
        check.set_valign(gtk::Align::Center);
        if let Some(f) = &first {
            check.set_group(Some(f));
        } else {
            first = Some(check.clone());
        }
        check.set_active(id == current);
        let r = widgets::row(&title(&id), &glib::markup_escape_text(&desc), None);
        if id == recommended {
            widgets::tag_row(&r, "Recommended");
        }
        r.prepend(&check);
        r.add_css_class("aip-preset");
        if !about.is_empty() {
            r.set_tooltip_text(Some(&about));
            widgets::keywords(&about);
        }
        if id == current {
            r.add_css_class("ai-picked");
        }
        let c = check.clone();
        kit::on_click(&r, move || c.set_active(true));
        let current = current.clone();
        check.connect_toggled(move |c| {
            if !c.is_active() || id == "custom" || id == current {
                return;
            }
            let id = id.clone();
            cmd::background(
                move || client::run(&["preset", &id]),
                |r| {
                    match r {
                        Ok(out) if out.contains("isn't downloaded") => {
                            window::toast("Applied; the preset's model isn't downloaded yet (see Language Model)")
                        }
                        Ok(_) => window::toast("Preset applied; the assistant restarts if it needs to"),
                        Err(e) => window::toast(&format!("Couldn't apply the preset: {e}")),
                    }
                    window::rebuild("assistant-performance");
                },
            );
        });
        g.add(&r);
    }
}

fn yes(v: &Value) -> bool {
    v.as_bool().unwrap_or(false)
}

fn file_name(v: &Value) -> String {
    v.as_str().and_then(|p| p.rsplit('/').next()).unwrap_or("—").to_string()
}

/// What the assistant has in memory, each with its state.
fn loaded(page: &Page, perf: &Value) {
    let g = page.group("Loaded now");
    let llm = &perf["llm"];
    // The recommended model's name, when it's one of them; otherwise from the file.
    let path = llm["model"].as_str().unwrap_or("");
    let list = client::json(&["models", "list", "--json"]);
    let name = list
        .as_ref()
        .and_then(|l| l["curated"].as_array().cloned())
        .into_iter()
        .flatten()
        .find(|m| m["filename"].as_str().is_some_and(|f| path.ends_with(f)))
        .and_then(|m| m["name"].as_str().map(kit::plain_name))
        .unwrap_or_else(|| kit::plain_name(&file_name(&llm["model"])));
    let mut desc = name;
    if let Some(ctx) = llm["ctx"].as_u64() {
        desc.push_str(&format!(" · {} tokens of conversation", kit::thousands(ctx)));
    }
    let state = if yes(&llm["loaded"]) { kit::pill("In memory", "ok") } else { kit::pill("Loads when you speak", "wait") };
    let r = widgets::icon_row("settings-sparkle-symbolic", "Language model", &glib::markup_escape_text(&desc), Some(state.upcast_ref()));
    r.set_tooltip_text(Some(path));
    g.add(&r);
    widgets::keywords("context tokens llm");

    let vision = &perf["vision"];
    let (desc, state) = if vision["projector"].is_null() {
        ("This model can't see the screen".to_string(), kit::pill("Off", "off"))
    } else if yes(&vision["loaded"]) {
        ("Loads when you ask about the screen".to_string(), kit::pill("In memory", "ok"))
    } else {
        ("Loads when you ask about the screen".to_string(), kit::pill("Waiting", "wait"))
    };
    let r = widgets::icon_row("view-reveal-symbolic", "Seeing the screen", &desc, Some(state.upcast_ref()));
    if let Some(p) = vision["projector"].as_str() {
        r.set_tooltip_text(Some(p));
    }
    g.add(&r);
    widgets::keywords("vision projector");

    let stt = &perf["stt"];
    let (desc, state) = match (yes(&stt["streaming"]), stt["load_ms"].as_u64()) {
        (true, Some(ms)) if ms > 0 => (format!("Listens as you talk · loaded in {:.1} s", ms as f64 / 1000.0), kit::pill("Ready", "ok")),
        (true, _) => ("Listens as you talk".to_string(), kit::pill("Loading", "wait")),
        _ => ("Not available: there's no speech model".to_string(), kit::pill("Off", "off")),
    };
    g.add(&widgets::icon_row("audio-input-microphone-symbolic", "Speech recognition", &desc, Some(state.upcast_ref())));
    let device = match perf["tts"]["device"].as_str() {
        Some("cuda") => "on the graphics card".to_string(),
        Some("cpu") => "on the processor".to_string(),
        Some(d) => format!("on {d}"),
        None => String::new(),
    };
    let desc = if device.is_empty() { "Kokoro".to_string() } else { format!("Kokoro, {device}") };
    g.add(&widgets::icon_row("audio-speakers-symbolic", "Voice", &desc, Some(kit::pill("Ready", "ok").upcast_ref())));
}

/// The assistant's state when a turn failed, in words.
fn stage_words(stage: &str) -> String {
    match stage {
        "idle" => "waiting".into(),
        "activating" => "loading its model".into(),
        "listening" => "listening".into(),
        "transcribing" => "recognising speech".into(),
        "thinking" => "thinking".into(),
        "executing" => "carrying out a task".into(),
        "speaking" => "speaking".into(),
        "interrupted" => "being interrupted".into(),
        "unavailable" => "reaching Nyx AI Core".into(),
        "error" => "handling an error".into(),
        other => other.to_string(),
    }
}

/// "Today at 18:02", "Yesterday at 18:02", "12 Oct at 18:02".
fn when_words(ms: u64) -> String {
    let Ok(t) = glib::DateTime::from_unix_local((ms / 1000) as i64) else { return String::new() };
    let Ok(now) = glib::DateTime::now_local() else { return String::new() };
    let days = now.ymd();
    let yesterday = now.add_days(-1).map(|d| d.ymd()).ok();
    let format = if t.ymd() == days {
        "Today at %H:%M"
    } else if Some(t.ymd()) == yesterday {
        "Yesterday at %H:%M"
    } else {
        "%e %b at %H:%M"
    };
    t.format(format).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Speed tests and the last problem, one level down.
fn more(page: &Page, failure: Option<Value>) {
    let g = page.group("");
    let (r, _) = widgets::nav_row(
        Some("power-profile-performance-symbolic"),
        "Speed tests",
        "Measure loading, talking and seeing the screen",
        "",
        || {
            widgets::subpage("Speed tests", 560, |page| {
                let g = page.group("");
                g.note("For repeatable numbers, run one in a terminal. Each loads the models and uses the graphics card while it runs.");
                for (icon, title, desc, command) in [
                    ("document-save-download-symbolic", "Loading", "How long the models take to get ready.", "nyx-assistant bench load"),
                    ("audio-input-microphone-symbolic", "Talking", "Listening and answering at the same time.", "nyx-assistant bench duplex"),
                    ("view-reveal-symbolic", "Seeing the screen", "How long a look at the screen takes.", "nyx-assistant bench vision"),
                    ("input-mouse-symbolic", "Using the computer", "Steps of a task on the desktop.", "nyx-assistant bench computer"),
                ] {
                    let copy = widgets::icon_button("edit-copy-symbolic", "Copy the command");
                    copy.connect_clicked(move |b| {
                        b.clipboard().set_text(command);
                        window::toast("Copied the command");
                    });
                    g.add(&widgets::icon_row(icon, title, &format!("{desc} <tt>{command}</tt>"), Some(copy.upcast_ref())));
                }
            });
        },
    );
    g.add(&r);
    widgets::keywords("benchmark bench load duplex vision computer terminal");
    let Some(failure) = failure else { return };
    let at = failure["at_ms"].as_u64().map(when_words).unwrap_or_default();
    let stage = failure["stage"].as_str().map(stage_words);
    let line = match (at.is_empty(), &stage) {
        (false, Some(s)) => format!("{at}, while {s}"),
        (false, None) => at.clone(),
        (true, Some(s)) => format!("While {s}"),
        (true, None) => String::new(),
    };
    let (r, _) = widgets::nav_row(Some("dialog-warning-symbolic"), "Last problem", &line, "", move || {
        let failure = failure.clone();
        widgets::subpage("Last problem", 520, move |page| {
            let g = page.group("");
            g.note("This record has timing and status only. What you said, prompts and tool arguments are left out.");
            if let Some(ms) = failure["at_ms"].as_u64() {
                g.add(&widgets::info_row("When", &when_words(ms)).0);
            }
            if let Some(s) = failure["stage"].as_str() {
                g.add(&widgets::info_row("What it was doing", &stage_words(s)).0);
            }
            if let Some(c) = failure["category"].as_str() {
                g.add(&widgets::info_row("Kind of problem", &c.replace('_', " ")).0);
            }
            g.add(&widgets::info_row("Turn", &failure["turn_id"].to_string()).0);
            if let Some(n) = failure["registered_tools"].as_u64() {
                g.add(&widgets::info_row("Tools it could use", &n.to_string()).0);
            }
            let tools: Vec<&str> = failure["selected_tools"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            if !tools.is_empty() {
                g.add(&widgets::info_row("Its last steps", &tools.join(", ")).0);
            }
        });
    });
    g.add(&r);
    widgets::keywords("failure error turn stage registered tools");
}

pub fn build(page: &Page) {
    let failure = client::json(&["privacy", "failure"]).filter(|v| v.get("turn_id").is_some());
    let perf = client::json(&["perf", "--json"]);

    // ----- The preview: the last answer, and the graphics card -----
    let pv = page.split();
    let stages: Vec<(&'static str, u64)> = perf
        .as_ref()
        .and_then(|p| p["last_turn"]["stages"].as_array().cloned())
        .map(|a| {
            a.iter()
                .filter_map(|s| Some((stage_label(s["name"].as_str()?)?, s["ms"].as_u64()?)))
                // A stage that didn't happen (a typed request, no speech) reads 0.
                .filter(|(_, ms)| *ms > 0)
                .collect()
        })
        .unwrap_or_default();
    let spoken = stages.iter().any(|(l, _)| *l == "Your words recognised");
    let line = match (&perf, stages.is_empty()) {
        (None, _) => "",
        (Some(_), true) => "Nothing measured yet",
        (Some(_), false) if spoken => "Spoken",
        (Some(_), false) => "Typed",
    };
    pv.head("Last answer", line, &[]);
    match &perf {
        None => pv.add(&kit::empty_card(
            "power-profile-performance-symbolic",
            "The assistant isn't running",
            "Once it's running, each answer shows here step by step: how long it took to recognise your words, think and start speaking.",
        )),
        Some(_) if stages.is_empty() => pv.add(&kit::empty_card(
            "power-profile-performance-symbolic",
            "No answer yet",
            "Ask the assistant something; the timings of its answer show here.",
        )),
        Some(p) => pv.add(&waterfall(&stages, &p["last_turn"]["gaps"])),
    }
    if let Some(card) = perf.as_ref().and_then(|p| vram_card(&p["gpu"])) {
        pv.add(&card);
    }
    if let Some(p) = &perf {
        let line = machine_line(p);
        if !line.is_empty() {
            pv.foot("computer-symbolic", &line);
        }
    }

    // ----- The controls -----
    if perf.is_none() {
        page.banner("The assistant isn't running, or needs updating. Start it on the Assistant page.", false);
    }
    presets(page);
    if let Some(p) = &perf {
        loaded(page, p);
    }
    more(page, failure);
    widgets::keywords(
        "preset lite balanced advanced hardware performance latency speed timing vram gpu memory benchmark temperature power battery gaps",
    );
}
