//! Hearing (speech recognition, devices), talking over it (duplex, barge-in) and its voice.

use super::{choice, client, save, slider, switch};
use crate::backend::audio;
use crate::widgets::{self, Debounce, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Kokoro's voices: (id, label).
const VOICES: &[(&str, &str)] = &[
    ("af_heart", "Heart (US)"),
    ("af_bella", "Bella (US)"),
    ("af_sarah", "Sarah (US)"),
    ("af_nicole", "Nicole (US)"),
    ("af_sky", "Sky (US)"),
    ("am_adam", "Adam (US)"),
    ("am_michael", "Michael (US)"),
    ("bf_emma", "Emma (UK)"),
    ("bf_isabella", "Isabella (UK)"),
    ("bm_george", "George (UK)"),
    ("bm_lewis", "Lewis (UK)"),
];

fn device_row(cfg: &serde_json::Value, key: &'static str, title: &str, devices: Vec<audio::Device>) -> gtk::Box {
    let mut options = vec![("default".to_string(), "Same as the system".to_string())];
    options.extend(devices.into_iter().map(|d| (d.name, d.description)));
    let current = client::get_str(cfg, key);
    if !options.iter().any(|(id, _)| *id == current) {
        options.push((current.clone(), current.clone()));
    }
    widgets::choice_row(title, "Restarts the assistant.", options, &current, move |id| save(key, client::quoted(&id))).0
}

fn speech_model_row() -> gtk::Box {
    let asr = client::json(&["models", "list", "--json"]).map(|v| v["asr"].clone()).unwrap_or_default();
    if asr["installed"] == true {
        let (r, _) = widgets::info_row("Speech model", "Installed");
        return r;
    }
    let (r, _) = widgets::button_row(
        "Speech model",
        "Not downloaded yet (about 650 MB). Until then the assistant uses Voxtype.",
        "Download",
        |b| {
            b.set_sensitive(false);
            b.set_label("Downloading…");
            cmd::background(
                || client::run(&["models", "fetch", "asr"]),
                |r| {
                    match r {
                        Ok(_) => window::toast("Speech model installed. Restart the assistant to use it."),
                        Err(e) => window::toast(&format!("Couldn't download the speech model: {e}")),
                    }
                    window::rebuild("assistant-voice");
                },
            );
        },
    );
    r
}

pub fn build(page: &Page) {
    let Some(cfg) = super::config_or_banner(page) else { return };

    let g = page.group("Hearing");
    g.add(&choice(
        &cfg,
        "stt.provider",
        "Speech recognition",
        "Restarts the assistant.",
        &[("streaming", "Streaming (transcribes while you speak)"), ("voxtype", "Voxtype (after you stop)")],
    ));
    if client::get_str(&cfg, "stt.provider") == "streaming" {
        g.add(&speech_model_row());
        g.add(&choice(
            &cfg,
            "stt.device",
            "Run recognition on",
            "The graphics card is faster but shares its memory with the language model.",
            &[("cpu", "Processor"), ("cuda", "Graphics card")],
        ));
    }
    g.add(&device_row(&cfg, "audio.source", "Microphone", audio::sources()));
    g.add(&device_row(&cfg, "audio.sink", "Speaker", audio::hardware_sinks()));
    widgets::keywords("stt asr speech recognition transcription microphone mic speaker output device voxtype");

    let g = page.group("Talking over it");
    g.add(&switch(&cfg, "barge_in.enabled", "Interrupt by speaking", "Speaking while it talks stops or lowers its voice."));
    g.add(&switch(
        &cfg,
        "duplex.enabled",
        "Listen while talking",
        "It keeps listening during its answer, so you can say “stop” or add to the request.",
    ));
    g.add(&choice(
        &cfg,
        "duplex.on_speech",
        "When you speak over it",
        "",
        &[("pause", "Pause its voice"), ("duck", "Lower its voice")],
    ));
    g.add(&slider(
        &cfg,
        "duplex.duck_gain",
        "Lowered volume",
        "How loud its voice stays while you talk.",
        (0.0, 1.0, 0.05),
        2,
        "",
    ));
    g.add(&slider(
        &cfg,
        "duplex.min_rms",
        "Quietest speech",
        "Lower hears softer speech, and more background noise.",
        (0.0, 0.1, 0.001),
        3,
        "",
    ));
    g.add(&slider(
        &cfg,
        "barge_in.grace_ms",
        "Ignore the start of its answer for",
        "So its own first words aren't taken for yours.",
        (0.0, 2000.0, 50.0),
        0,
        " ms",
    ));
    widgets::keywords("duplex barge in interrupt duck pause stop sensitivity");

    let g = page.group("Voice");
    let picked =
        Rc::new(RefCell::new((client::get_str(&cfg, "tts.voice"), client::get(&cfg, "tts.speed").as_f64().unwrap_or(1.0))));
    let options: Vec<(String, String)> = VOICES.iter().map(|(id, l)| (id.to_string(), l.to_string())).collect();
    let p = picked.clone();
    let (r, _) = widgets::choice_row("Voice", "", options, &picked.borrow().0, move |id| {
        p.borrow_mut().0 = id.clone();
        save("tts.voice", client::quoted(&id));
    });
    g.add(&r);
    let (p, wait) = (picked.clone(), Debounce::default());
    let (r, _) = widgets::slider_row("Speed", "", (0.5, 2.0, 0.05), picked.borrow().1, 2, "×", move |v| {
        p.borrow_mut().1 = v;
        wait.call(400, move || save("tts.speed", format!("{v:.2}")));
    });
    g.add(&r);
    let (r, _) = widgets::button_row("Hear it", "Says a sentence with this voice and speed.", "Preview", move |b| {
        let (voice, speed) = picked.borrow().clone();
        b.set_sensitive(false);
        let b = b.clone();
        cmd::background(
            move || client::run(&["preview", &voice, &format!("{speed:.2}")]),
            move |r| {
                b.set_sensitive(true);
                if let Err(e) = r {
                    window::toast(&format!("Couldn't preview: {e}"));
                }
            },
        );
    });
    g.add(&r);
    widgets::keywords("tts voice kokoro speed preview");
}
