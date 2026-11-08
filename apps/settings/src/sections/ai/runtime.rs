//! Local AI: whether Nyx AI Core runs, what it holds in memory, how many models stay
//! loaded and for how long, updates, the default models, and coding tools.

use super::{call, change, has, size};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use serde_json::Value;

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
    let worker = &status["worker"];
    let res = &status["resources"];

    let g = page.group("Status");
    let state = worker["state"].as_str().unwrap_or("unknown");
    let release = worker["release"].as_str().unwrap_or("");
    let restart = gtk::Button::with_label("Restart");
    restart.set_valign(gtk::Align::Center);
    restart.connect_clicked(|b| {
        b.set_sensitive(false);
        service("restart");
    });
    let state_text = match state {
        "ready" => "Running",
        "stopped" => "Waiting for the first request",
        "starting" | "restarting" => "Starting",
        "failed" => "Stopped after repeated crashes",
        "missing" => "llama.cpp isn't installed",
        other => other,
    };
    g.add(&widgets::row(
        "Nyx AI Core",
        &glib_escape(&format!("{state_text} · llama.cpp {release}{}", worker["last_error"].as_str().map(|e| format!(" · {e}")).unwrap_or_default())),
        Some(restart.upcast_ref()),
    ));
    let backends: Vec<String> = res["gpu_backends"].as_array().into_iter().flatten().filter_map(|b| b.as_str().map(str::to_uppercase)).collect();
    g.add(&widgets::info_row("Acceleration", &backends.join(", ")).0);
    g.add(&widgets::info_row(
        "Memory",
        &format!(
            "{} graphics memory, {} system memory ({} free)",
            size(res["vram_total"].as_u64().unwrap_or(0)),
            size(res["ram_total"].as_u64().unwrap_or(0)),
            size(res["ram_available"].as_u64().unwrap_or(0))
        ),
    )
    .0);
    let loaded = res["loaded"].as_array().cloned().unwrap_or_default();
    if loaded.is_empty() {
        g.note("No model is loaded.");
    }
    for m in loaded {
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
        g.add(&widgets::row(&id, &glib_escape(&desc), Some(unload.upcast_ref())));
    }
    let metrics = &worker["metrics"];
    if metrics["requests"].as_u64().unwrap_or(0) > 0 {
        g.note(&format!(
        "{} requests since it started · first word after {} ms on average · {} words per second",
        metrics["requests"].as_u64().unwrap_or(0),
        metrics["first_token_ms_avg"].as_u64().unwrap_or(0),
        metrics["tokens_per_second_avg"].as_f64().unwrap_or(0.0)
        ));
    }

    let g = page.group("Memory and updates");
    g.add(&choice(
        "Models kept loaded",
        "More switch faster and use more memory. Every Nyx app shares them.",
        &[("1", "One"), ("2", "Two"), ("3", "Three"), ("4", "Four")],
        &settings["max_loaded"].as_u64().unwrap_or(1).to_string(),
        "max_loaded",
    ));
    g.add(&choice(
        "Unload after",
        "An unused model leaves memory after this long. One an app keeps loaded stays.",
        &[("-1", "Never"), ("300", "5 minutes"), ("900", "15 minutes"), ("3600", "1 hour")],
        &settings["idle_unload_seconds"].as_i64().unwrap_or(900).to_string(),
        "idle",
    ));
    g.add(&choice(
        "Model updates",
        "Newer versions of downloaded models, from Hugging Face. Automatic updates download only while no model is answering and the computer is plugged in.",
        &[("auto", "Automatic"), ("notify", "Tell me"), ("off", "Off")],
        settings["model_updates"].as_str().unwrap_or("auto"),
        "updates",
    ));
    if has_battery() {
        let on = settings["battery_saver"].as_bool().unwrap_or(false);
        let desc = if settings["cpu_only"].as_bool().unwrap_or(false) {
            "On battery, models run on the processor instead of the graphics card: slower, but the battery lasts longer. Running on battery now."
        } else {
            "On battery, models run on the processor instead of the graphics card: slower, but the battery lasts longer."
        };
        g.add(&widgets::switch_row("Battery saver", desc, on, |on| {
            change(vec!["service".into(), format!("battery_saver={}", if on { "on" } else { "off" })], PAGE, "")
        }).0);
    }

    let g = page.group("Default models");
    g.note("Apps use these when they have no choice of their own.");
    let list: Vec<Value> = models.as_array().cloned().unwrap_or_default();
    let defaults = &settings["defaults"];
    for (role, title, desc, capability) in [
        ("chat", "Chat", "New chats in Chat and other apps that chat.", "chat"),
        ("assistant", "Assistant", "Nyx Assistant's language model.", "chat"),
        ("embedding", "Search and knowledge", "Finds text by meaning: Glimpse, knowledge bases.", "embedding"),
        ("coding", "Coding tools", "What the OpenAI-compatible API answers with when a tool names no model.", "chat"),
    ] {
        let mut options = vec![(String::new(), "None".to_string())];
        options.extend(
            list.iter()
                .filter(|m| has(m, capability) && m["state"].as_str() != Some("missing"))
                .filter_map(|m| m["id"].as_str().map(|id| (id.to_string(), id.to_string()))),
        );
        let current = defaults[role].as_str().unwrap_or("");
        let row = widgets::choice_row(title, desc, options, current, move |id| {
            change(vec!["default".into(), role.into(), id], PAGE, "")
        })
        .0;
        g.add(&row);
    }

    let g = page.group("Coding tools");
    match settings["openai_endpoint"].as_str() {
        Some(addr) => {
            g.add(&widgets::info_row("OpenAI-compatible API", &format!("http://{addr}/v1")).0);
            g.note("Point a coding tool at this address with any placeholder key. Only this computer can reach it; it starts Nyx AI Core when called.");
        }
        None => g.note("The OpenAI-compatible API is off (openai_endpoint in ~/.config/nyx-ai/settings.toml)."),
    }
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
