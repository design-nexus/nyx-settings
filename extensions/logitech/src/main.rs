//! Logitech mice and keyboards for Settings, over HID++ on `/dev/hidraw*`.
//!
//! settings-logitech pages | describe PAGE | set PAGE KEY VALUE | watch
//!
//! The protocol code comes from the Logi app (design-nexus/nexus-logi); saved
//! settings share its `~/.config/logi/devices.toml`.

mod access;
mod devices;
mod extension;
mod files;
mod gearbench;
// Kept identical to Logi's copy, including parts this helper doesn't use (pairing, notifications).
#[allow(dead_code)]
mod hidpp;
mod pane;
mod paths;
mod service;
mod state;
mod watch;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("watch") => watch::watch(),
        _ => extension::run(&args),
    };
    if let Err(e) = result {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
