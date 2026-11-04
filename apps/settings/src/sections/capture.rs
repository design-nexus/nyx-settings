//! Screenshots, screen recording, reminders, weather, dictation and small tools.
//! Mostly buttons for Omarchy's own commands, with the choices that go with them.

use crate::sections::accounts::terminal;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::process::{Command, Stdio};
use std::rc::Rc;

pub fn build(page: &Page) {
    screenshots(page);
    recording(page);
    reminders(page);
    weather(page);
    dictation(page);
    tools(page);
}

/// Run a command after a short pause, so Settings can get out of the picture first.
fn later(args: Vec<String>) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(700));
        if let Some((program, rest)) = args.split_first() {
            let _ = Command::new(program).args(rest).stdin(Stdio::null()).status();
        }
    });
}

/// Ask the compositor to hide this window for a moment, then run `args`.
fn hide_and_run(args: Vec<String>) {
    if let Some(win) = window::window() {
        win.minimize();
    }
    later(args);
}

// ----- Screenshots -----

fn screenshots(page: &Page) {
    if !cmd::present("omarchy-capture-screenshot") {
        return;
    }
    let g = page.group("Screenshots");
    let mode = Rc::new(RefCell::new("smart".to_string()));
    let save = Rc::new(Cell::new(false));
    let (r, _) = widgets::choice_row(
        "What to capture",
        "Smart picks a window or lets you drag a region. Scroll captures a page longer than the screen.",
        widgets::opts(&[
            ("smart", "Smart"),
            ("region", "A region"),
            ("windows", "A window"),
            ("fullscreen", "The whole screen"),
            ("scroll", "Scrolling capture"),
        ]),
        "smart",
        {
            let mode = mode.clone();
            move |m| *mode.borrow_mut() = m
        },
    );
    g.add(&r);
    let (r, _) = widgets::choice_row(
        "Where it goes",
        "Copy puts it on the clipboard; save writes a file to your Pictures folder.",
        widgets::opts(&[("copy", "Copy to the clipboard"), ("save", "Save to a file")]),
        "copy",
        {
            let save = save.clone();
            move |s| save.set(s == "save")
        },
    );
    g.add(&r);
    let (r, _) = widgets::button_row("Take a screenshot", "Settings steps aside first.", "Capture", move |_| {
        let mut args = vec!["omarchy-capture-screenshot".to_string(), mode.borrow().clone()];
        args.push(if save.get() { "save".into() } else { "copy".into() });
        hide_and_run(args);
    });
    widgets::keywords("screenshot screen capture print region window scroll clipboard");
    g.add(&r);
    if let Some(args) = capture_tool("--text", "omarchy-capture-text") {
        let (r, _) = widgets::button_row(
            "Copy text from the screen",
            "Drag over text in an image or video to copy it.",
            "Select…",
            move |_| hide_and_run(args.clone()),
        );
        widgets::keywords("ocr text recognition extract");
        g.add(&r);
    }
    if let Some(args) = capture_tool("--qr", "omarchy-capture-qr") {
        let (r, _) = widgets::button_row("Read a QR code", "Drag over a QR code on the screen.", "Scan…", move |_| {
            hide_and_run(args.clone())
        });
        widgets::keywords("qr code scan");
        g.add(&r);
    }
    let colour = if cmd::present("nyx-capture") {
        Some(vec!["nyx-capture".to_string(), "--colour".into()])
    } else if cmd::present("hyprpicker") {
        Some(vec!["hyprpicker".to_string(), "-a".into()])
    } else {
        None
    };
    if let Some(args) = colour {
        let (r, _) = widgets::button_row("Pick a colour", "Click any pixel on the screen to copy its colour.", "Pick…", move |_| {
            hide_and_run(args.clone())
        });
        widgets::keywords("colour color picker eyedropper hex");
        g.add(&r);
    }
}

/// Nyx Capture's `flag` when it is installed, else Omarchy's `script`.
fn capture_tool(flag: &str, script: &str) -> Option<Vec<String>> {
    if cmd::present("nyx-capture") {
        Some(vec!["nyx-capture".into(), flag.into()])
    } else if cmd::present(script) {
        Some(vec![script.into()])
    } else {
        None
    }
}

// ----- Recording -----

#[derive(Default)]
struct Rec {
    desktop: bool,
    mic: bool,
    webcam: bool,
    full: bool,
}

/// The recorder's arguments for the chosen options.
fn recording_args(r: &Rec) -> Vec<String> {
    let mut a = vec!["omarchy-capture-screenrecording".to_string()];
    if r.full {
        a.push("--fullscreen".into());
    }
    if r.desktop {
        a.push("--with-desktop-audio".into());
    }
    if r.mic {
        a.push("--with-microphone-audio".into());
    }
    if r.webcam {
        a.push("--with-webcam".into());
    }
    a
}

fn recording(page: &Page) {
    if !cmd::present("omarchy-capture-screenrecording") {
        return;
    }
    let g = page.group("Screen recording");
    let rec = Rc::new(RefCell::new(Rec::default()));
    for (title, desc, set) in [
        ("Record the whole screen", "Skip choosing a region first.", (|r: &mut Rec, v| r.full = v) as fn(&mut Rec, bool)),
        ("Include sound from the computer", "What you hear: music, calls, videos.", |r, v| r.desktop = v),
        ("Include the microphone", "Your voice.", |r, v| r.mic = v),
        ("Show the webcam", "A small camera picture over the recording.", |r, v| r.webcam = v),
    ] {
        let rec = rec.clone();
        let (r, _) = widgets::switch_row(title, desc, false, move |on| set(&mut rec.borrow_mut(), on));
        widgets::keywords("record video screencast");
        g.add(&r);
    }
    let controls = widgets::hbox(8);
    let start = gtk::Button::with_label("Start");
    start.add_css_class("suggested-action");
    start.connect_clicked(move |_| hide_and_run(recording_args(&rec.borrow())));
    let stop = gtk::Button::with_label("Stop");
    stop.connect_clicked(|_| cmd::spawn(&["omarchy-capture-screenrecording", "--stop-recording"]));
    controls.append(&start);
    controls.append(&stop);
    g.add(&widgets::row("Recording", "The recording is saved to your Videos folder.", Some(controls.upcast_ref())));
    widgets::keywords("record video screencast start stop");
}

// ----- Reminders -----

/// `minutes` and an optional message for `omarchy-reminder`.
fn reminder_args(minutes: &str, message: &str) -> Option<Vec<String>> {
    let m: u32 = minutes.trim().parse().ok().filter(|m| (1..=10080).contains(m))?;
    let mut a = vec!["omarchy-reminder".to_string(), m.to_string()];
    // A leading - would be read as an option.
    let msg = message.trim().trim_start_matches('-').trim_start();
    if !msg.is_empty() {
        a.push(msg.to_string());
    }
    Some(a)
}

/// "Check the oven in 12m (14:05)" for each waiting reminder in `omarchy-reminder show --json`.
fn reminder_labels(json: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    v.get("reminders")
        .and_then(|r| r.as_array())
        .map(|list| {
            list.iter()
                .map(|r| {
                    let text = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                    format!("{} in {} ({})", text("label"), text("remaining"), text("atTime"))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn reminders(page: &Page) {
    if !cmd::present("omarchy-reminder") {
        return;
    }
    let g = page.collapsible("Reminders", false);
    let minutes = gtk::Entry::new();
    minutes.set_placeholder_text(Some("Minutes"));
    minutes.set_width_chars(8);
    let message = gtk::Entry::new();
    message.set_placeholder_text(Some("Message (optional)"));
    let set = gtk::Button::with_label("Remind me");
    set.add_css_class("suggested-action");
    let form = widgets::hbox(8);
    form.append(&minutes);
    form.append(&message);
    form.append(&set);
    set.connect_clicked(move |_| match reminder_args(&minutes.text(), &message.text()) {
        Some(args) => {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            cmd::run_async(&refs, |r| {
                match r {
                    Ok(_) => window::toast("Reminder set"),
                    Err(e) => window::toast(&format!("{e}")),
                }
                window::rebuild("capture");
            });
        }
        None => window::toast("Enter the minutes from now, 1 to 10080"),
    });
    g.add(&widgets::stacked_row("Remind me in", "A notification after that many minutes.", form.upcast_ref()));
    widgets::keywords("reminder timer alarm notify later");
    // `show` pops a notification; the JSON form only prints.
    let shown = reminder_labels(&cmd::output(&["omarchy-reminder", "show", "--json"]).unwrap_or_default());
    if !shown.is_empty() {
        let (r, _) = widgets::info_row("Waiting", &shown.join("\n"));
        g.add(&r);
        let (r, _) = widgets::button_row("Cancel reminders", "", "Clear all", |_| {
            cmd::run_async(&["omarchy-reminder", "clear"], |_| window::rebuild("capture"));
        });
        g.add(&r);
    }
}

// ----- Weather -----

fn weather(page: &Page) {
    if !cmd::present("omarchy-weather-location") {
        return;
    }
    let g = page.collapsible("Weather and status", false);
    let (r, _) = widgets::entry_row(
        "Weather location",
        "A town or city name, used by the weather panel and bar. Empty finds it automatically. Press Enter.",
        "",
        "e.g. Austin",
        |place| {
            let place = place.trim().to_string();
            if place.starts_with('-') {
                return;
            }
            let args: Vec<&str> = if place.is_empty() {
                vec!["omarchy-weather-location", "--clear"]
            } else {
                vec!["omarchy-weather-location", "--set", &place]
            };
            cmd::run_async(&args, |r| match r {
                Ok(_) => window::toast("Weather location saved"),
                Err(e) => window::toast(&format!("{e}")),
            });
        },
    );
    widgets::keywords("weather location city forecast");
    g.add(&r);
    let controls = widgets::hbox(8);
    for (script, label) in [
        ("omarchy-notification-time", "Time"),
        ("omarchy-notification-battery", "Battery"),
        ("omarchy-notification-weather", "Weather"),
    ] {
        if cmd::present(script) {
            let b = gtk::Button::with_label(label);
            b.connect_clicked(move |_| cmd::spawn(&[script]));
            controls.append(&b);
        }
    }
    g.add(&widgets::row("Show now", "Pop up the time, battery or weather panel.", Some(controls.upcast_ref())));
    widgets::keywords("notification panel time battery weather popup");
}

// ----- Dictation -----

fn dictation(page: &Page) {
    if !cmd::present("omarchy-voxtype-install") {
        return;
    }
    let g = page.collapsible("Dictation", false);
    let on = cmd::present("voxtype");
    let controls = widgets::hbox(8);
    if on {
        for (script, label) in [("omarchy-voxtype-config", "Settings"), ("omarchy-voxtype-model", "Voice model")] {
            if cmd::present(script) {
                let b = gtk::Button::with_label(label);
                b.connect_clicked(move |_| cmd::spawn(&[script]));
                controls.append(&b);
            }
        }
        if cmd::present("omarchy-voxtype-remove") {
            let b = gtk::Button::with_label("Remove…");
            b.connect_clicked(|_| terminal("omarchy-voxtype-remove"));
            controls.append(&b);
        }
    } else {
        let b = gtk::Button::with_label("Install…");
        b.connect_clicked(|_| terminal("omarchy-voxtype-install"));
        controls.append(&b);
    }
    g.add(&widgets::row(
        "Speech to text",
        if on { "Installed. Type by talking, in any app." } else { "Type by talking, in any app. Runs on this computer." },
        Some(controls.upcast_ref()),
    ));
    widgets::keywords("voxtype dictation voice speech whisper microphone typing");
}

// ----- Tools -----

/// Coding agents Omarchy knows, as (id, name).
const AGENTS: &[(&str, &str)] = &[
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("opencode", "OpenCode"),
    ("pi", "Pi"),
    ("omp", "Oh My Pi"),
    ("ori", "Ori"),
    ("grok", "Grok"),
    ("agy", "Antigravity"),
    ("copilot", "GitHub Copilot"),
    ("crush", "Crush"),
    ("cursor-agent", "Cursor CLI"),
    ("muse", "Muse Code"),
    ("hermes", "Hermes"),
    ("openclaw", "OpenClaw"),
];

fn tools(page: &Page) {
    let g = page.collapsible("Tools", false);
    if cmd::present("omarchy-default-agent") {
        // The choice is a one-line file; asking the script to read it would be the same.
        let current = std::fs::read_to_string(crate::paths::config_home().join("omarchy/defaults/agent"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let mut options = widgets::opts(AGENTS);
        if current.is_empty() {
            options.insert(0, (String::new(), "None chosen".into()));
        } else if !AGENTS.iter().any(|(id, _)| *id == current) {
            options.push((current.clone(), current.clone()));
        }
        let (r, _) = widgets::choice_row(
            "Default coding agent",
            "The assistant the agent shortcut opens. Choosing one installs it if needed, in a terminal, and opens it.",
            options,
            &current,
            {
                let current = current.clone();
                move |id| {
                    if !id.is_empty() && id != current && AGENTS.iter().any(|(a, _)| *a == id) {
                        cmd::spawn(&["omarchy-default-agent", &id]);
                    }
                }
            },
        );
        widgets::keywords("agent ai assistant claude codex opencode cursor copilot coding llm");
        g.add(&r);
    }
    if cmd::present("omarchy-toggle-crash-capture") {
        let off = cmd::run(&["omarchy-toggle-enabled", "crash-capture-off"]).is_ok();
        let (r, _) =
            widgets::switch_row("Tell me when a program crashes", "A notification with a way to look into it.", !off, |_| {
                cmd::run_async(&["omarchy-toggle-crash-capture"], |r| {
                    if let Err(e) = r {
                        window::toast(&format!("{e}"));
                    }
                });
            });
        widgets::keywords("crash coredump notification report debug");
        g.add(&r);
    }
    if cmd::present("omarchy-disk-speedtest") {
        let (r, _) = widgets::button_row("Disk speed test", "Measures how fast the drive reads and writes.", "Run…", |_| {
            terminal("omarchy-disk-speedtest")
        });
        widgets::keywords("ssd nvme benchmark storage");
        g.add(&r);
    }
    let controls = widgets::hbox(8);
    for (script, label) in [("omarchy-restart-audio", "Sound"), ("omarchy-restart-shell", "Bar and menus")] {
        if cmd::present(script) {
            let b = gtk::Button::with_label(label);
            b.connect_clicked(move |b| {
                b.set_sensitive(false);
                let b = b.clone();
                cmd::run_async(&[script], move |r| {
                    b.set_sensitive(true);
                    match r {
                        Ok(_) => window::toast("Restarted"),
                        Err(e) => window::toast(&format!("{e}")),
                    }
                });
            });
            controls.append(&b);
        }
    }
    if controls.first_child().is_some() {
        g.add(&widgets::row("Restart", "Try this when sound stops or the bar misbehaves.", Some(controls.upcast_ref())));
        widgets::keywords("fix reset pipewire audio shell bar");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_arguments() {
        assert_eq!(recording_args(&Rec::default()), ["omarchy-capture-screenrecording"]);
        let r = Rec { desktop: true, mic: true, webcam: false, full: true };
        assert_eq!(
            recording_args(&r),
            ["omarchy-capture-screenrecording", "--fullscreen", "--with-desktop-audio", "--with-microphone-audio"]
        );
    }

    #[test]
    fn waiting_reminders() {
        assert!(reminder_labels(r#"{"count":0,"reminders":[]}"#).is_empty());
        assert!(reminder_labels("not json").is_empty());
        let j = r#"{"count":1,"reminders":[{"label":"Check the oven","remaining":"12m","atTime":"14:05"}]}"#;
        assert_eq!(reminder_labels(j), ["Check the oven in 12m (14:05)"]);
    }

    #[test]
    fn reminders() {
        assert_eq!(reminder_args("5", ""), Some(vec!["omarchy-reminder".into(), "5".into()]));
        assert_eq!(
            reminder_args(" 30 ", " Check the oven "),
            Some(vec!["omarchy-reminder".into(), "30".into(), "Check the oven".into()])
        );
        assert_eq!(reminder_args("5", "--help"), Some(vec!["omarchy-reminder".into(), "5".into(), "help".into()]));
        assert_eq!(reminder_args("0", ""), None);
        assert_eq!(reminder_args("abc", ""), None);
        assert_eq!(reminder_args("99999", ""), None);
    }
}
