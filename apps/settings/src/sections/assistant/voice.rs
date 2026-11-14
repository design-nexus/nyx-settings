//! Its voice first, then hearing (speech recognition, devices) and talking over it
//! (duplex, barge-in), beside a short exchange in the chosen voice and a picture of
//! what talking over it does.

use super::{chip, client, icon_slider, icon_switch, more, row_text, save, seg_row, slider};
use crate::backend::audio;
use crate::sketch;
use crate::widgets::{self, Debounce, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Kokoro's voices: (id, name). The id's first letter says the accent.
const VOICES: &[(&str, &str)] = &[
    ("af_heart", "Heart"),
    ("af_bella", "Bella"),
    ("af_sarah", "Sarah"),
    ("af_nicole", "Nicole"),
    ("af_sky", "Sky"),
    ("am_adam", "Adam"),
    ("am_michael", "Michael"),
    ("bf_emma", "Emma"),
    ("bf_isabella", "Isabella"),
    ("bm_george", "George"),
    ("bm_lewis", "Lewis"),
];

/// What Kokoro says for a preview (the assistant's own sentence).
const SAMPLE: &str = "Hello. This is a preview of your selected voice.";

fn voice_name(id: &str) -> String {
    VOICES.iter().find(|(v, _)| *v == id).map(|(_, n)| n.to_string()).unwrap_or_else(|| id.to_string())
}

fn accent(id: &str) -> &'static str {
    match id.chars().next() {
        Some('a') => "American",
        Some('b') => "British",
        _ => "",
    }
}

fn device_row(cfg: &serde_json::Value, key: &'static str, icon: &str, title: &str, devices: Vec<audio::Device>) -> gtk::Box {
    let mut options = vec![("default".to_string(), "Like the system".to_string())];
    options.extend(devices.into_iter().map(|d| (d.name, d.description)));
    let current = client::get_str(cfg, key);
    if !options.iter().any(|(id, _)| *id == current) {
        options.push((current.clone(), current.clone()));
    }
    let (r, _) = widgets::choice_row(title, "Changing it restarts the assistant.", options, &current, move |id| save(key, client::quoted(&id)));
    r.prepend(&widgets::tile(icon));
    r.add_css_class("with-tile");
    r
}

fn speech_model_row() -> gtk::Box {
    let asr = client::json(&["models", "list", "--json"]).map(|v| v["asr"].clone()).unwrap_or_default();
    if asr["installed"] == true {
        return widgets::icon_row("audio-input-microphone-symbolic", "Speech to text", "On this computer · ready", Some(chip("Ready", "asst-ok").upcast_ref()));
    }
    let b = gtk::Button::with_label("Download");
    b.connect_clicked(|b| {
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
    });
    widgets::icon_row(
        "audio-input-microphone-symbolic",
        "Speech model",
        "Not downloaded yet (about 650 MB). Until then the assistant can't hear you.",
        Some(b.upcast_ref()),
    )
}

/// What the "talking over it" picture shows.
struct Over {
    name: String,
    barge: Cell<bool>,
    duck: Cell<bool>,
    gain: Cell<f64>,
    grace_ms: Cell<f64>,
}

/// Bar heights for a voice track, 0–1, the same every time for a `seed`.
fn bars(n: usize, seed: u32) -> Vec<f64> {
    (0..n).map(|i| 0.18 + (((i as u32).wrapping_mul(2654435761).wrapping_add(seed.wrapping_mul(97531)) >> 9) % 100) as f64 / 100.0 * 0.82).collect()
}

/// Its voice and yours on two tracks: its voice dropping (or pausing) while you speak,
/// and the start of its answer it doesn't listen to.
fn draw_over(cr: &gtk::cairo::Context, p: &sketch::Pal, c: sketch::Rgb, w: f64, h: f64, o: &Over) {
    use super::{text_at, text_width};
    let label_w = 48.0;
    let (x0, tw) = (label_w, w - label_w - 2.0);
    let n = 64;
    let step = tw / n as f64;
    let (talk_from, talk_to) = (30, 46);
    let track_h = (h - 40.0) / 2.0;
    let (echo_y, you_y) = (20.0 + track_h / 2.0, 20.0 + track_h * 1.5 + 4.0);
    sketch::set(cr, p.dim, 1.0);
    text_at(cr, &o.name, 0.0, echo_y, 11.0, true, false);
    text_at(cr, "You", 0.0, you_y, 11.0, true, false);
    let you = sketch::mix(p.accent, (0.3, 0.5, 1.0), 0.3);
    let lowered = o.barge.get();
    for (i, v) in bars(n, 7).into_iter().enumerate() {
        let inside = (talk_from..talk_to).contains(&i);
        let k = if inside && lowered { if o.duck.get() { o.gain.get().max(0.04) } else { 0.04 } } else { 1.0 };
        let bh = (track_h - 4.0) * v * k;
        sketch::set(cr, c, if inside && lowered { 0.55 } else { 0.85 });
        sketch::rounded(cr, x0 + i as f64 * step, echo_y - bh.max(2.0) / 2.0, (step * 0.55).max(1.5), bh.max(2.0), 1.0);
        let _ = cr.fill();
    }
    for (i, v) in bars(n, 19).into_iter().enumerate() {
        let x = x0 + i as f64 * step;
        if (talk_from..talk_to).contains(&i) {
            let bh = (track_h - 4.0) * v;
            sketch::set(cr, you, 0.9);
            sketch::rounded(cr, x, you_y - bh / 2.0, (step * 0.55).max(1.5), bh, 1.0);
            let _ = cr.fill();
        } else {
            sketch::set(cr, p.dim, 0.35);
            cr.rectangle(x, you_y - 0.75, step * 0.55, 1.5);
            let _ = cr.fill();
        }
    }
    // The start of its answer it doesn't take for yours: 2 s is a fifth of the track.
    let grace = o.grace_ms.get();
    let mut grace_end = x0;
    if grace > 0.0 {
        let gw = (grace / 2000.0 * 0.2 * tw).max(4.0);
        cr.save().ok();
        cr.rectangle(x0, echo_y - track_h / 2.0, gw, track_h);
        cr.clip();
        sketch::set(cr, p.text, 0.18);
        cr.set_line_width(3.0);
        let mut sx = x0 - track_h;
        while sx < x0 + gw {
            cr.move_to(sx, echo_y + track_h / 2.0);
            cr.line_to(sx + track_h, echo_y - track_h / 2.0);
            sx += 7.0;
        }
        let _ = cr.stroke();
        cr.restore().ok();
        sketch::set(cr, p.dim, 1.0);
        grace_end = x0 + text_at(cr, &format!("first {:.1} s ignored", grace / 1000.0), x0, 8.0, 10.5, false, false);
    }
    let (mx, mw) = (x0 + talk_from as f64 * step - 3.0, (talk_to - talk_from) as f64 * step + 2.0);
    sketch::set(cr, c, 0.9);
    cr.set_line_width(1.2);
    sketch::rounded(cr, mx, echo_y - track_h / 2.0, mw, track_h, 4.0);
    let _ = cr.stroke();
    let note = match (lowered, o.duck.get()) {
        (false, _) => "Keeps talking".to_string(),
        (true, true) => format!("Lowered to {}%", (o.gain.get() * 100.0).round() as i64),
        (true, false) => "Paused".to_string(),
    };
    let nw = text_width(cr, &note, 10.5, true);
    let nx = (mx + mw / 2.0 - nw / 2.0).max(grace_end + 10.0);
    if nx + nw <= w {
        sketch::set(cr, c, 1.0);
        text_at(cr, &note, nx, 8.0, 10.5, true, false);
    } else {
        // No room beside the first words' note: under the tracks instead.
        sketch::set(cr, c, 1.0);
        text_at(cr, &note, (mx + mw / 2.0 - nw / 2.0).min(w - nw).max(x0), h - 5.0, 10.5, true, false);
    }
}

/// A few bars of its voice, drawn under the answer.
fn draw_wave(cr: &gtk::cairo::Context, c: sketch::Rgb, w: f64, h: f64, playing: f64) {
    let n = ((w / 5.0) as usize).max(8);
    let step = w / n as f64;
    for (i, v) in bars(n, 3).into_iter().enumerate() {
        let bh = (h - 2.0) * v;
        let played = (i as f64 / n as f64) < playing;
        sketch::set(cr, c, if played { 1.0 } else { 0.45 });
        sketch::rounded(cr, i as f64 * step, (h - bh) / 2.0, (step * 0.55).max(1.5), bh, 1.0);
        let _ = cr.fill();
    }
}

fn fine_tuning(page: &Page, _: &super::Again) {
    let cfg = client::config();
    let g = page.group("Fine-tuning");
    g.add(&slider(&cfg, "duplex.min_rms", "Quietest speech", "Lower hears softer speech, and more background noise.", (0.0, 0.1, 0.001), 3, ""));
    g.add(&slider(
        &cfg,
        "barge_in.grace_ms",
        "Ignore the start of its answer for",
        "So its own first words aren't taken for yours.",
        (0.0, 2000.0, 50.0),
        0,
        " ms",
    ));
}

pub fn build(page: &Page) {
    let Some(cfg) = super::config_or_banner(page) else { return };
    let name = client::get_str(&cfg, "assistant.name");
    let picked = Rc::new(RefCell::new((client::get_str(&cfg, "tts.voice"), client::get(&cfg, "tts.speed").as_f64().unwrap_or(1.0))));
    let over = Rc::new(Over {
        name: name.clone(),
        barge: Cell::new(client::get(&cfg, "barge_in.enabled").as_bool().unwrap_or(false)),
        duck: Cell::new(client::get_str(&cfg, "duplex.on_speech") != "pause"),
        gain: Cell::new(client::get(&cfg, "duplex.duck_gain").as_f64().unwrap_or(0.25)),
        grace_ms: Cell::new(client::get(&cfg, "barge_in.grace_ms").as_f64().unwrap_or(0.0)),
    });

    // The picture: a short exchange in its voice, and talking over it.
    let pv = page.split();
    let now = chip("", "asst-voice-chip");
    let show_now = {
        let now = now.clone();
        move |voice: &str, speed: f64| now.set_text(&format!("{} · {speed:.2}×", voice_name(voice)).replace(".00×", "×"))
    };
    show_now(&picked.borrow().0, picked.borrow().1);
    pv.head("Hear it", "A short exchange in the voice and speed on the left.", &[now.upcast_ref()]);
    let chat = widgets::vbox(14);
    chat.add_css_class("preview-card");
    chat.add_css_class("asst-chat");
    let me = widgets::label(&format!("Hey {name}, how do you sound?"), "asst-bubble-me");
    me.set_wrap(true);
    me.set_halign(gtk::Align::End);
    me.set_max_width_chars(36);
    chat.append(&me);
    let line = widgets::hbox(10);
    line.append(&super::orb(30, true));
    let bubble = widgets::vbox(8);
    bubble.add_css_class("asst-bubble-it");
    bubble.set_hexpand(true);
    let said = widgets::label(SAMPLE, "asst-bubble-text");
    said.set_wrap(true);
    bubble.append(&said);
    let play_line = widgets::hbox(10);
    let play = widgets::icon_button("media-playback-start-symbolic", "Hear it");
    play.add_css_class("asst-play");
    play_line.append(&play);
    let progress = Rc::new(Cell::new(0.0));
    let wave = {
        let progress = progress.clone();
        widgets::drawing(26, move |cr, _, c, w, h| draw_wave(cr, c, w, h, progress.get()))
    };
    wave.set_valign(gtk::Align::Center);
    play_line.append(&wave);
    bubble.append(&play_line);
    line.append(&bubble);
    chat.append(&line);
    if !client::running() {
        let l = widgets::label("Start the assistant on the Assistant page to hear it.", "asst-card-line");
        l.set_wrap(true);
        chat.append(&l);
    }
    {
        let (picked, progress, wave) = (picked.clone(), progress.clone(), wave.clone());
        play.connect_clicked(move |b| {
            let (voice, speed) = picked.borrow().clone();
            b.set_sensitive(false);
            b.set_icon_name("media-playback-pause-symbolic");
            // Light the bars along while it talks (about three seconds at 1×).
            let started = std::time::Instant::now();
            let (p2, w2, b2) = (progress.clone(), wave.clone(), b.clone());
            gtk::glib::timeout_add_local(std::time::Duration::from_millis(60), move || {
                if b2.is_sensitive() {
                    p2.set(0.0);
                    w2.queue_draw();
                    return gtk::glib::ControlFlow::Break;
                }
                p2.set((started.elapsed().as_secs_f64() * speed / 3.2).min(1.0));
                w2.queue_draw();
                gtk::glib::ControlFlow::Continue
            });
            let b = b.clone();
            cmd::background(
                move || client::run(&["preview", &voice, &format!("{speed:.2}")]),
                move |r| {
                    b.set_sensitive(true);
                    b.set_icon_name("media-playback-start-symbolic");
                    if let Err(e) = r {
                        window::toast(&format!("Couldn't preview: {e}"));
                    }
                },
            );
        });
    }
    pv.add(&chat);
    let card = widgets::vbox(10);
    card.add_css_class("preview-card");
    let head = widgets::hbox(8);
    head.append(&widgets::label("Talking over it", "asst-card-title"));
    let sub = widgets::label("Its voice drops while you speak, then carries on.", "asst-card-line");
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    head.append(&sub);
    card.append(&head);
    let tracks = {
        let over = over.clone();
        widgets::drawing(110, move |cr, p, c, w, h| draw_over(cr, p, c, w, h, &over))
    };
    card.append(&tracks);
    pv.add(&card);
    pv.foot("changes-prevent-symbolic", "Speech to text and its voice both run on this computer.");

    // Its voice.
    let g = page.group("Its voice");
    let mut options: Vec<(String, String)> = VOICES.iter().map(|(id, l)| (id.to_string(), l.to_string())).collect();
    let current = picked.borrow().0.clone();
    if !options.iter().any(|(id, _)| *id == current) {
        options.push((current.clone(), current.clone()));
    }
    let (r, _) = {
        let (p, show_now) = (picked.clone(), show_now.clone());
        widgets::choice_row("Voice", accent(&current), options, &current, move |id| {
            p.borrow_mut().0 = id.clone();
            show_now(&id, p.borrow().1);
            save("tts.voice", client::quoted(&id));
        })
    };
    r.prepend(&widgets::tile("audio-speakers-symbolic"));
    r.add_css_class("with-tile");
    if let Some(desc) = super::row_text(&r).and_then(|t| t.last_child()).and_downcast::<gtk::Label>()
        && desc.has_css_class("settings-option-description")
        && let Some(dd) = r.last_child().and_downcast::<gtk::DropDown>()
    {
        let opts: Vec<String> = VOICES.iter().map(|(id, _)| id.to_string()).collect();
        dd.connect_selected_notify(move |d| {
            if let Some(id) = opts.get(d.selected() as usize) {
                desc.set_text(accent(id));
            }
        });
    }
    g.add(&r);
    widgets::keywords("tts voice kokoro hear it preview");
    let (b, s) = widgets::slider(0.5, 2.0, 0.05, picked.borrow().1, 2, "×");
    s.scale.set_width_request(120);
    {
        let (p, wait, show_now) = (picked.clone(), Debounce::default(), show_now.clone());
        s.scale.connect_value_changed(move |sc| {
            let v = sc.value();
            p.borrow_mut().1 = v;
            show_now(&p.borrow().0, v);
            wait.call(400, move || save("tts.speed", format!("{v:.2}")));
        });
    }
    g.add(&widgets::icon_row("media-playback-start-symbolic", "Speed", "", Some(b.upcast_ref())));
    widgets::keywords("tts speed rate");

    // Nyx Voice's streaming recognizer: words while you speak.
    let g = page.group("Hearing");
    g.add(&speech_model_row());
    widgets::keywords("stt asr speech recognition transcription nyx voice");
    let seg = widgets::segmented(
        &widgets::opts(&[("cpu", "Processor"), ("cuda", "Graphics card")]),
        &client::get_str(&cfg, "stt.device"),
        |id| save("stt.device", client::quoted(&id)),
    );
    let r = widgets::icon_row("system-run-symbolic", "Runs on", "The graphics card is faster, but shares memory with the model.", None);
    if let Some(text) = row_text(&r) {
        seg.set_halign(gtk::Align::Start);
        seg.set_margin_top(6);
        text.append(&seg);
    }
    g.add(&r);
    widgets::keywords("run recognition on processor cpu graphics card gpu cuda");
    g.add(&device_row(&cfg, "audio.source", "audio-input-microphone-symbolic", "Microphone", audio::sources()));
    widgets::keywords("mic input device");
    g.add(&device_row(&cfg, "audio.sink", "audio-volume-high-symbolic", "Speaker", audio::hardware_sinks()));
    widgets::keywords("speaker output device");

    let g = page.group("Talking over it");
    let (r, _) = {
        let (over, tracks) = (over.clone(), tracks.clone());
        icon_switch(
            &cfg,
            "barge_in.enabled",
            "user-available-symbolic",
            "Stop or lower its voice",
            "When you start talking during an answer.",
            move |on| {
                over.barge.set(on);
                tracks.queue_draw();
            },
        )
    };
    g.add(&r);
    widgets::keywords("interrupt by speaking barge in");
    let (r, _) = icon_switch(
        &cfg,
        "duplex.enabled",
        "audio-input-microphone-symbolic",
        "Keep listening while it talks",
        "So you can say “stop” or add to the request.",
        |_| {},
    );
    g.add(&r);
    widgets::keywords("duplex listen while talking");
    let (r, _) = {
        let (over, tracks) = (over.clone(), tracks.clone());
        seg_row(
            "audio-volume-low-symbolic",
            "When you talk over it",
            "",
            &[("pause", "Pause"), ("duck", "Lower")],
            &client::get_str(&cfg, "duplex.on_speech"),
            move |id| {
                over.duck.set(id == "duck");
                tracks.queue_draw();
                save("duplex.on_speech", client::quoted(&id));
            },
        )
    };
    g.add(&r);
    widgets::keywords("pause duck lower its voice");
    let r = {
        let (over, tracks) = (over.clone(), tracks.clone());
        icon_slider(&cfg, "duplex.duck_gain", "preferences-system-symbolic", "Lowered to", "", (0.0, 1.0, 0.05), 2, "", move |v, l| {
            l.set_text(&format!("{}%", (v * 100.0).round() as i64));
            over.gain.set(v);
            tracks.queue_draw();
        })
    };
    g.add(&r);
    widgets::keywords("lowered volume duck gain");
    let g = page.group("");
    let (r, _) = widgets::nav_row(Some("emblem-system-symbolic"), "Fine-tuning", "", "Quietest speech, its first words", || {
        more("Fine-tuning", fine_tuning)
    });
    widgets::keywords("quietest speech sensitivity min rms ignore the start of its answer grace");
    g.add(&r);
}
