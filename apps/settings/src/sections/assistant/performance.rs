//! How fast the last answer came, and what the assistant holds in memory.

use super::{client, size};
use crate::widgets::{self, Page};
use crate::{cmd, window};
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

/// One bar from "you stopped" to "you hear it", a segment per stage.
fn timing_bar(stages: Vec<(&'static str, u64)>) -> gtk::DrawingArea {
    use crate::sketch::{self, mix, rounded, set};
    let area = gtk::DrawingArea::new();
    area.set_content_height(22);
    area.set_hexpand(true);
    area.set_draw_func(move |_, cr, w, h| {
        let p = sketch::pal();
        let end = stages.last().map(|s| s.1).unwrap_or(1).max(1) as f64;
        let (w, h) = (w as f64, h as f64);
        set(cr, p.dim, 0.25);
        rounded(cr, 0.0, 0.0, w, h, 6.0);
        let _ = cr.fill();
        let mut from = 0.0;
        for (i, (_, ms)) in stages.iter().enumerate() {
            let to = w * (*ms as f64 / end);
            set(cr, mix(p.accent, p.surface, i as f64 * 0.15), 1.0);
            rounded(cr, from, 0.0, (to - from - 2.0).max(1.0), h, 4.0);
            let _ = cr.fill();
            from = to;
        }
    });
    area
}

/// Lite, Balanced or Advanced: one choice that sets the model, its context, where speech
/// runs and when the models unload (`nyx-assistant preset`).
fn presets(page: &Page) {
    let Some(p) = client::json(&["preset", "show", "--json"]) else {
        return;
    };
    let g = page.group("Hardware preset");
    let title = |name: &str| {
        let mut c = name.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    let recommended = p["recommended"].as_str().unwrap_or("");
    let mut options: Vec<(String, String)> = p["presets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| {
            let name = x["name"].as_str()?;
            Some((name.to_string(), if name == recommended { format!("{} (recommended)", title(name)) } else { title(name) }))
        })
        .collect();
    let current = p["current"].as_str().unwrap_or("custom").to_string();
    if current == "custom" {
        options.insert(0, ("custom".into(), "Custom".into()));
    }
    let why = p["why"].as_str().unwrap_or("");
    let (r, _) = widgets::choice_row(
        "Preset",
        &format!("Recommended here: {} ({why}). Changing a setting afterwards makes it Custom.", title(recommended)),
        options,
        &current,
        move |id| {
            if id == "custom" {
                return;
            }
            cmd::background(
                move || client::run(&["preset", &id]),
                |r| {
                    match r {
                        Ok(out) if out.contains("isn't downloaded") => {
                            window::toast("Applied; the preset's model isn't downloaded yet (see Models)")
                        }
                        Ok(_) => window::toast("Preset applied; the assistant restarts if it needs to"),
                        Err(e) => window::toast(&format!("Couldn't apply the preset: {e}")),
                    }
                    window::rebuild("assistant-performance");
                },
            );
        },
    );
    g.add(&r);
    if let Some(x) = p["presets"].as_array().into_iter().flatten().find(|x| x["name"] == current.as_str()) {
        g.note(x["description"].as_str().unwrap_or(""));
    }
}

fn yes(v: &Value) -> bool {
    v.as_bool().unwrap_or(false)
}

fn file_name(v: &Value) -> String {
    v.as_str().and_then(|p| p.rsplit('/').next()).unwrap_or("—").to_string()
}

pub fn build(page: &Page) {
    if let Some(failure) = client::json(&["privacy", "failure"]).filter(|v| v.get("turn_id").is_some()) {
        let g = page.group("Last failure");
        g.note(&format!(
            "Turn {} · stage {} · {} registered tools",
            failure["turn_id"], failure["stage"], failure["registered_tools"]
        ));
        g.note("This record contains timing and status only. Utterances, prompts and tool arguments are excluded.");
    }

    let Some(perf) = client::json(&["perf", "--json"]) else {
        page.banner("The assistant isn't running, or needs updating. Start it on the Assistant page.", false);
        return;
    };

    presets(page);

    let g = page.group("Last answer");
    let stages: Vec<(&'static str, u64)> = perf["last_turn"]["stages"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| Some((stage_label(s["name"].as_str()?)?, s["ms"].as_u64()?)))
                // A stage that didn't happen (a typed request, no speech) reads 0.
                .filter(|(_, ms)| *ms > 0)
                .collect()
        })
        .unwrap_or_default();
    if stages.is_empty() {
        g.note("Ask the assistant something; the timings of its answer show here.");
    } else {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("preview-card");
        card.append(&timing_bar(stages.clone()));
        g.add(&card);
        for (label, ms) in stages {
            g.add(&widgets::info_row(label, &format!("{ms} ms")).0);
        }
        let gaps = &perf["last_turn"]["gaps"];
        if let Some(count) = gaps["count"].as_u64() {
            let text = match (count, gaps["max_ms"].as_u64()) {
                (0, _) => "None".to_string(),
                (n, Some(max)) => format!("{n}, the longest {max} ms"),
                (n, None) => n.to_string(),
            };
            g.add(&widgets::info_row("Pauses between phrases", &text).0);
        }
        g.note("Measured from when you stopped speaking.");
    }

    let g = page.group("Loaded now");
    let llm = &perf["llm"];
    let state = if yes(&llm["loaded"]) { "in memory" } else { "loads when you speak" };
    g.add(&widgets::info_row("Language model", &format!("{} · {state}", file_name(&llm["model"]))).0);
    if let Some(ctx) = llm["ctx"].as_u64() {
        g.add(&widgets::info_row("Context", &format!("{ctx} tokens")).0);
    }
    let vision = &perf["vision"];
    let projector = if vision["projector"].is_null() {
        "This model can't see the screen".to_string()
    } else {
        format!(
            "{} · {}",
            file_name(&vision["projector"]),
            if yes(&vision["loaded"]) { "in memory" } else { "loads when needed" }
        )
    };
    g.add(&widgets::info_row("Vision", &projector).0);
    let stt = &perf["stt"];
    let speech = match (yes(&stt["streaming"]), stt["load_ms"].as_u64()) {
        (true, Some(ms)) if ms > 0 => format!("Streaming · loaded in {ms} ms"),
        (true, _) => "Streaming · loading".to_string(),
        _ => "Voxtype".to_string(),
    };
    g.add(&widgets::info_row("Speech recognition", &speech).0);
    let tts = &perf["tts"];
    g.add(&widgets::info_row("Voice", &format!("Kokoro on {}", tts["device"].as_str().unwrap_or("?"))).0);
    let gpu = &perf["gpu"];
    if let (Some(used), Some(total)) = (gpu["used_mib"].as_u64(), gpu["total_mib"].as_u64()) {
        let mine = gpu["assistant_mib"].as_u64().map(|m| format!(", the assistant {}", size(m * 1_048_576))).unwrap_or_default();
        g.add(
            &widgets::info_row(
                "Graphics memory",
                &format!("{} of {} used{mine}", size(used * 1_048_576), size(total * 1_048_576)),
            )
            .0,
        );
    }

    let g = page.group("Machine");
    if yes(&gpu["asleep"]) {
        g.add(&widgets::info_row("Graphics card", "Asleep").0);
    } else if let Some(temp) = gpu["temp_c"].as_f64() {
        let power = gpu["power_w"].as_f64().map(|w| format!(" · {w:.0} W")).unwrap_or_default();
        let load = gpu["util_pct"].as_f64().map(|u| format!(" · {u:.0}% busy")).unwrap_or_default();
        g.add(&widgets::info_row("Graphics card", &format!("{temp:.0} °C{power}{load}")).0);
    }
    if let Some(temp) = perf["cpu_temp_c"].as_f64() {
        g.add(&widgets::info_row("Processor", &format!("{temp:.0} °C")).0);
    }
    if let Some(battery) = perf["on_battery"].as_bool() {
        g.add(&widgets::info_row("Power", if battery { "Battery" } else { "Plugged in" }).0);
    }
    g.note("For repeatable numbers run <tt>nyx-assistant bench load</tt>, <tt>bench duplex</tt>, <tt>bench vision</tt> or <tt>bench computer</tt> in a terminal.");
    widgets::keywords(
        "preset lite balanced advanced hardware performance latency speed timing vram gpu memory benchmark temperature power battery gaps",
    );
}
