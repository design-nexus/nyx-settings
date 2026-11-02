//! Settings — a control panel for Omarchy.

mod backend;
mod charts;
use nyx_ui::clamp;
mod cmd;
mod control;
mod dialog;
mod ext;
mod fangraph;
mod live;
mod paths;
mod prefs;
mod search;
mod preview;
mod sections;
mod sketch;
mod theme;
mod units;
mod widgets;
mod window;

use gtk::prelude::*;
use gtk::{gio, glib};

const APP_ID: &str = "io.github.design_nexus.nyx.Settings";

const USAGE: &str = "Usage: nyx-settings [--section ID] [--volume raise|lower|+N|-N] [--eq on|off|toggle|status] [--apply]\n\
\n\
  --section ID   open (or switch the open window) to a page, e.g. audio, trackpad\n\
  --volume STEP  change the output volume, allowing past 100% up to the maximum set in Sound\n\
  --eq ACTION    turn the preamp/equalizer on or off, or print whether it's running\n\
  --kbd-timeout S  turn the keyboard backlight off after S idle seconds (off to disable)\n\
  --ext list|install ID|URL [PATH]|update [ID]|enable ID|disable ID|remove ID  manage device extensions\n\
  --update [--check]  update Settings and its extensions (--check only reports)\n\
  --theme-sync   (internal) re-apply the icon theme you chose and tell extensions; run by the theme-set hook\n\
  --kbd-idle     (internal) turn the keyboard backlight off when idle; run by the settings-kbd-idle service\n\
  --remove-old-panels [--dry-run]  remove the settings panels this app replaces (backed up first)\n\
  --install-helper  (run with sudo) install the root helper that system changes (users, firewall, time, services, printers) need\n\
  --apply        rewrite ~/.config/hypr/settings.lua from saved state and exit\n";

fn main() -> glib::ExitCode {
    nyx_core::app::init(nyx_core::app::App { slug: "nyx-settings", name: "Settings" });
    nyx_core::migrate::from(&["settings"]);
    let args: Vec<String> = std::env::args().collect();

    // Headless commands never touch GTK.
    if let Some(i) = args.iter().position(|a| a == "--volume") {
        let step = args.get(i + 1).map(String::as_str).unwrap_or("raise");
        let max = backend::state::State::load(&paths::state_file()).max_volume.unwrap_or(100);
        return match backend::audio::step_volume(step, max) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nyx-settings: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if let Some(i) = args.iter().position(|a| a == "--kbd-timeout") {
        let value = args.get(i + 1).map(String::as_str).unwrap_or("");
        let secs = match value {
            "off" | "never" | "0" => Some(0),
            v => v.trim_end_matches('s').parse::<u32>().ok(),
        };
        let Some(timeout_secs) = secs else {
            eprintln!("usage: settings --kbd-timeout <seconds>|off");
            return glib::ExitCode::FAILURE;
        };
        return match backend::kbdidle::apply(&backend::kbdidle::Config { timeout_secs }) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nyx-settings: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if args.iter().any(|a| a == "--install-helper") {
        return match backend::accounts::install_helper() {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nyx-settings: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if let Some(i) = args.iter().position(|a| a == "--update") {
        return match backend::updates::cli(&args[i + 1..]) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nyx-settings: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if let Some(i) = args.iter().position(|a| a == "--ext") {
        return match ext::manage::cli(&args[i + 1..]) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nyx-settings: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    // --aura-sync is the name older hooks used.
    if args.iter().any(|a| a == "--theme-sync" || a == "--aura-sync") {
        return match backend::themehook::sync() {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nyx-settings: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if args.iter().any(|a| a == "--kbd-idle") {
        return match backend::kbdidle::run_daemon() {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nyx-settings: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if args.iter().any(|a| a == "--kbd-idle-restore") {
        let _ = backend::kbdidle::restore();
        return glib::ExitCode::SUCCESS;
    }
    if let Some(i) = args.iter().position(|a| a == "--eq") {
        use backend::audio;
        let action = args.get(i + 1).map(String::as_str).unwrap_or("status");
        let mut eq = audio::load();
        let on = match action {
            "on" => true,
            "off" => false,
            "toggle" => !audio::running(),
            _ => {
                println!("{}", if audio::running() { "on" } else { "off" });
                return glib::ExitCode::SUCCESS;
            }
        };
        eq.enabled = on;
        let result = if on { audio::install_and_start(&eq) } else { audio::save(&eq).and_then(|_| audio::stop(&eq)) };
        return match result {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nyx-settings: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if args.iter().any(|a| a == "--apply") {
        let state = backend::state::State::load(&paths::state_file());
        return match backend::hypr::write(&state) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nyx-settings: {e:#}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return glib::ExitCode::SUCCESS;
    }

    let mut flags = gio::ApplicationFlags::HANDLES_COMMAND_LINE;
    // Snapshots render in their own process, so they never hand off to an open window.
    if std::env::var_os("SETTINGS_SNAPSHOT").is_some() {
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }
    // NYX_SETTINGS_APP_ID runs a test copy under its own ID, beside the real one.
    let id = std::env::var("NYX_SETTINGS_APP_ID").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| APP_ID.to_string());
    // Windows without an application, like dialogs, take their Wayland app ID from this.
    glib::set_prgname(Some(id.as_str()));
    let app = gtk::Application::builder().application_id(id.as_str()).flags(flags).build();
    // Nyx Assistant lists and shows pages through this; it changes nothing (primary instance only).
    app.connect_startup(|app| nyx_core::control::serve(app, control::tools(), control::handle));
    app.connect_command_line(|app, cl| {
        let argv: Vec<String> = cl.arguments().iter().map(|a| a.to_string_lossy().to_string()).collect();
        let section = argv.iter().position(|a| a == "--section").and_then(|i| argv.get(i + 1)).cloned();
        window::present(app, section.as_deref().map(|id| if id == "cloud-storage" {"online-accounts"} else {id}));
        glib::ExitCode::SUCCESS
    });
    app.run()
}
