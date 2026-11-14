use crate::backend::sysinfo;
use crate::backend::updates;
use crate::sketch;
use crate::widgets::{self, Page};
use crate::{cmd, prefs, units, window};
use gtk::prelude::*;
use std::cell::RefCell;

thread_local! {
    /// The version just installed, until Settings restarts into it.
    static INSTALLED: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn checked_text(s: &updates::Status) -> String {
    if s.checked == 0 {
        return "not checked yet".into();
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("checked {}", units::ago(now.saturating_sub(s.checked) as i64))
}

fn check_now(b: &gtk::Button) {
    b.set_sensitive(false);
    b.set_label("Checking…");
    cmd::background(
        || updates::check(true),
        |s| {
            if !s.settings_update() && s.extensions.is_empty() {
                window::toast("Everything is up to date");
            }
            window::after_update_check(s);
        },
    );
}

/// A row made by one of the `widgets` row functions, with an icon on a tile before it.
fn with_tile(r: gtk::Box, icon: &str) -> gtk::Box {
    r.prepend(&widgets::tile(icon));
    r.add_css_class("with-tile");
    r
}

// ---------- This computer ----------

/// What the picture shows, read once when the page is built.
struct Specs {
    host: String,
    model: String,
    cpu: String,
    cores: usize,
    ghz: Option<f64>,
    memory: sysinfo::Memory,
    /// Each graphics chip's name, and whether it's built into the processor.
    gpus: Vec<(String, bool)>,
    /// The system disk: size and used.
    disk: Option<(u64, u64)>,
    display: Option<Display>,
    screens: usize,
    battery: Option<sysinfo::Battery>,
    system: (String, String),
    kernel: String,
    hyprland: String,
    uptime: u64,
}

struct Display {
    width: i64,
    height: i64,
    hz: f64,
    /// The diagonal in inches, when the screen reports its size.
    inches: Option<f64>,
}

fn gpus() -> Vec<(String, bool)> {
    let text = cmd::output(&["lspci", "-mm"]).unwrap_or_default();
    parse_gpus(&text)
}

/// `lspci -mm`: graphics chips by name (`Intel Arc Pro 130T/140T`), and whether each is
/// on the processor's own bus (built in).
fn parse_gpus(text: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let (slot, rest) = line.split_once(' ').unwrap_or((line, ""));
        let fields: Vec<&str> = rest.split('"').map(str::trim).filter(|f| !f.is_empty()).collect();
        let [class, vendor, device, ..] = fields[..] else { continue };
        if !["VGA compatible controller", "3D controller", "Display controller"].contains(&class) {
            continue;
        }
        let maker = if vendor.contains("NVIDIA") {
            "NVIDIA"
        } else if vendor.contains("Intel") {
            "Intel"
        } else if vendor.contains("AMD") || vendor.contains("ATI") {
            "AMD"
        } else {
            vendor.split_whitespace().next().unwrap_or(vendor)
        };
        // "GB203M / GN22-X9 [GeForce RTX 5080 Max-Q / Mobile]": the marketing name is in brackets.
        let name = match (device.rfind('['), device.rfind(']')) {
            (Some(a), Some(b)) if b > a => device[a + 1..b].to_string(),
            _ => device.to_string(),
        };
        let name = name.trim_end_matches(" / Mobile").trim_end_matches(" Mobile").to_string();
        let name = if name.starts_with(maker) { name } else { format!("{maker} {name}") };
        out.push((name, slot.starts_with("00:")));
    }
    out
}

fn display() -> (Option<Display>, usize) {
    let list = cmd::output(&["hyprctl", "monitors", "-j"])
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|m| !m["name"].as_str().unwrap_or("").starts_with("HEADLESS"))
        .collect::<Vec<_>>();
    let first = list.iter().find(|m| m["name"].as_str().is_some_and(|n| n.starts_with("eDP"))).or(list.first());
    let d = first.map(|m| {
        let (pw, ph) = (m["physicalWidth"].as_f64().unwrap_or(0.0), m["physicalHeight"].as_f64().unwrap_or(0.0));
        Display {
            width: m["width"].as_i64().unwrap_or(0),
            height: m["height"].as_i64().unwrap_or(0),
            hz: m["refreshRate"].as_f64().unwrap_or(0.0),
            inches: (pw > 0.0 && ph > 0.0).then(|| (pw * pw + ph * ph).sqrt() / 25.4),
        }
    });
    (d, list.len())
}

/// The system's name and version: Omarchy's own, or the distribution's.
fn system() -> (String, String) {
    if let Some(v) = cmd::output(&["omarchy-version"]) {
        return ("Omarchy".into(), v.trim().to_string());
    }
    let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let field = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .map(|v| v.trim_matches('"').to_string())
            .unwrap_or_default()
    };
    let name = field("NAME");
    let version = [field("VERSION_ID"), field("BUILD_ID")].into_iter().find(|v| !v.is_empty()).unwrap_or_default();
    (if name.is_empty() { "System".into() } else { name }, version)
}

fn read_specs() -> Specs {
    let cores = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default().lines().filter(|l| l.starts_with("processor")).count();
    let ghz = std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
        .ok()
        .and_then(|t| t.trim().parse::<f64>().ok())
        .map(|khz| khz / 1_000_000.0);
    let disk = sysinfo::disks().into_iter().find(|d| d.mount == "/").map(|d| (d.size, d.used));
    let (display, screens) = display();
    let hyprland = cmd::output(&["hyprctl", "version", "-j"])
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["version"].as_str().map(str::to_string))
        .unwrap_or_default();
    Specs {
        host: sysinfo::hostname(),
        model: sysinfo::model(),
        cpu: sysinfo::cpu_model(),
        cores,
        ghz,
        memory: sysinfo::memory(),
        gpus: if cmd::present("lspci") { gpus() } else { Vec::new() },
        disk,
        display,
        screens,
        battery: sysinfo::battery(),
        system: system(),
        kernel: std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string(),
        hyprland,
        uptime: sysinfo::uptime_secs(),
    }
}

/// One fact beside the picture: a small heading, the value, a line under it and,
/// for what fills up, how full it is.
struct Fact {
    icon: &'static str,
    label: &'static str,
    value: String,
    sub: String,
    fill: Option<f64>,
}

fn facts(s: &Specs) -> (Vec<Fact>, Vec<Fact>) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    if !s.cpu.is_empty() {
        let mut sub = format!("{} cores", s.cores);
        if let Some(g) = s.ghz {
            sub.push_str(&format!(" · up to {g:.1} GHz"));
        }
        left.push(Fact { icon: "power-profile-performance-symbolic", label: "Processor", value: s.cpu.clone(), sub, fill: None });
    }
    if s.memory.total > 0 {
        left.push(Fact {
            icon: "view-grid-symbolic",
            label: "Memory",
            value: memory_size(s.memory.total),
            sub: format!("{} in use", sysinfo::bytes_text(s.memory.used)),
            fill: Some(s.memory.used as f64 / s.memory.total as f64),
        });
    }
    let main = s.gpus.iter().find(|g| !g.1).or(s.gpus.first());
    if let Some((name, built_in)) = main {
        let others: Vec<&str> = s.gpus.iter().filter(|g| g.0 != *name).map(|g| g.0.as_str()).collect();
        let sub = match (others.first(), built_in) {
            (Some(o), _) => format!("And {o}, built in"),
            (None, true) => "Built in".into(),
            (None, false) => String::new(),
        };
        left.push(Fact { icon: "applications-graphics-symbolic", label: "Graphics", value: name.clone(), sub, fill: None });
    }
    if let Some((size, used)) = s.disk {
        right.push(Fact {
            icon: "drive-harddisk-symbolic",
            label: "Storage",
            value: sysinfo::bytes_text(size),
            sub: format!("{} free", sysinfo::bytes_text(size.saturating_sub(used))),
            fill: Some(used as f64 / size.max(1) as f64),
        });
    }
    if let Some(d) = &s.display {
        let value = match d.inches {
            Some(i) => format!("{i:.0}″ · {} × {}", d.width, d.height),
            None => format!("{} × {}", d.width, d.height),
        };
        let mut sub = format!("{:.0} Hz", d.hz);
        if s.screens > 1 {
            sub.push_str(&format!(" · and {} more", s.screens - 1));
        }
        right.push(Fact { icon: "preferences-desktop-display-symbolic", label: "Display", value, sub, fill: None });
    }
    if let Some(b) = &s.battery {
        let (value, fill) = match b.health {
            Some(h) => (format!("{h}% health"), Some(h as f64 / 100.0)),
            None => (format!("{}% charged", b.percent), Some(b.percent as f64 / 100.0)),
        };
        let sub = match b.cycles {
            Some(c) => format!("{c} charges"),
            None => format!("{}% now", b.percent),
        };
        right.push(Fact { icon: "battery-good-symbolic", label: "Battery", value, sub, fill });
    }
    (left, right)
}

/// Memory as it's sold: 31.1 GB usable reads as 32 GB.
fn memory_size(bytes: u64) -> String {
    let gb = bytes as f64 / (1u64 << 30) as f64;
    let sold = [2.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0, 96.0, 128.0, 192.0, 256.0];
    match sold.iter().find(|s| **s >= gb && **s - gb < **s * 0.12) {
        Some(s) => format!("{s:.0} GB"),
        None => sysinfo::bytes_text(bytes),
    }
}

fn fact_widget(f: &Fact) -> gtk::Box {
    let b = widgets::hbox(10);
    b.add_css_class("ab-fact");
    let t = widgets::tile(f.icon);
    t.set_valign(gtk::Align::Start);
    b.append(&t);
    let text = widgets::vbox(1);
    text.set_hexpand(true);
    text.append(&widgets::label(&f.label.to_uppercase(), "ab-fact-label"));
    let v = widgets::label(&f.value, "ab-fact-value");
    v.set_wrap(true);
    text.append(&v);
    if !f.sub.is_empty() {
        let s = widgets::label(&f.sub, "ab-fact-sub");
        s.set_wrap(true);
        text.append(&s);
    }
    if let Some(fill) = f.fill {
        let bar = gtk::LevelBar::for_interval(0.0, 1.0);
        bar.set_value(fill.clamp(0.0, 1.0));
        bar.add_css_class("part-usage");
        bar.add_css_class("ab-fact-bar");
        text.append(&bar);
    }
    b.append(&text);
    b
}

/// The computer: a laptop when it has a battery, otherwise a screen on a stand, with
/// the desktop on it.
fn draw_computer(cr: &gtk::cairo::Context, p: &sketch::Pal, w: f64, h: f64, laptop: bool) {
    let body = sketch::mix(p.bg, p.text, 0.16);
    let edge = sketch::mix(p.bg, p.text, 0.28);
    if laptop {
        let sw = (w * 0.78).min((h - 16.0) * 1.55);
        let sh = sw / 1.55;
        let x = (w - sw) / 2.0;
        let y = (h - sh - 14.0) / 2.0;
        sketch::rounded(cr, x, y, sw, sh, 8.0);
        sketch::set(cr, body, 1.0);
        let _ = cr.fill_preserve();
        sketch::set(cr, edge, 1.0);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        sketch::desktop(cr, p, sketch::R::new(x + 6.0, y + 6.0, sw - 12.0, sh - 12.0));
        let dw = sw * 1.16;
        let dx = (w - dw) / 2.0;
        sketch::rounded(cr, dx, y + sh, dw, 9.0, 4.0);
        sketch::set(cr, edge, 1.0);
        let _ = cr.fill();
        sketch::rounded(cr, w / 2.0 - sw * 0.1, y + sh, sw * 0.2, 3.5, 1.7);
        sketch::set(cr, body, 1.0);
        let _ = cr.fill();
    } else {
        let sw = (w * 0.8).min((h - 30.0) * 1.6);
        let sh = sw / 1.6;
        let x = (w - sw) / 2.0;
        let y = 4.0;
        sketch::rounded(cr, x, y, sw, sh, 8.0);
        sketch::set(cr, body, 1.0);
        let _ = cr.fill_preserve();
        sketch::set(cr, edge, 1.0);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        sketch::desktop(cr, p, sketch::R::new(x + 6.0, y + 6.0, sw - 12.0, sh - 12.0));
        sketch::set(cr, edge, 1.0);
        cr.rectangle(w / 2.0 - 7.0, y + sh, 14.0, 16.0);
        let _ = cr.fill();
        sketch::rounded(cr, w / 2.0 - sw * 0.18, y + sh + 15.0, sw * 0.36, 6.0, 3.0);
        let _ = cr.fill();
    }
}

fn details_text(s: &Specs, left: &[Fact], right: &[Fact]) -> String {
    let mut out = vec![s.host.clone()];
    if !s.model.is_empty() {
        out.push(s.model.clone());
    }
    for f in left.iter().chain(right) {
        let mut line = format!("{}: {}", f.label, f.value);
        if !f.sub.is_empty() {
            line.push_str(&format!(" ({})", f.sub));
        }
        out.push(line);
    }
    out.push(format!("{} {}", s.system.0, s.system.1).trim().to_string());
    out.push(format!("Linux {}", s.kernel));
    if !s.hyprland.is_empty() {
        out.push(format!("Hyprland {}", s.hyprland));
    }
    out.push(format!("Settings {}", env!("CARGO_PKG_VERSION")));
    out.push(format!("Up for {}", sysinfo::duration_text(s.uptime)));
    out.join("\n")
}

fn computer_preview(page: &Page) {
    let s = read_specs();
    let (left, right) = facts(&s);
    let pv = page.split();
    let copy = gtk::Button::new();
    let inner = widgets::hbox(6);
    inner.append(&gtk::Image::from_icon_name("edit-copy-symbolic"));
    inner.append(&gtk::Label::new(Some("Copy details")));
    copy.set_child(Some(&inner));
    let text = details_text(&s, &left, &right);
    copy.connect_clicked(move |b| {
        b.clipboard().set_text(&text);
        window::toast("Copied this computer's details");
    });
    let title = if s.host.is_empty() { "This computer" } else { s.host.as_str() };
    pv.head(title, &s.model, &[copy.upcast_ref()]);

    let stage = widgets::hbox(12);
    stage.add_css_class("preview-card");
    stage.add_css_class("ab-stage");
    stage.set_vexpand(true);
    let column = |facts: &[Fact]| {
        let c = widgets::vbox(18);
        c.set_valign(gtk::Align::Center);
        c.set_hexpand(true);
        c.set_size_request(140, -1);
        for f in facts {
            c.append(&fact_widget(f));
        }
        c
    };
    stage.append(&column(&left));
    let mid = widgets::vbox(6);
    mid.set_valign(gtk::Align::Center);
    let laptop = s.battery.is_some();
    let pic = widgets::drawing(124, move |cr, p, _, w, h| draw_computer(cr, p, w, h, laptop));
    pic.set_size_request(176, 124);
    pic.set_hexpand(false);
    mid.append(&pic);
    if !s.model.is_empty() {
        let m = widgets::label(&s.model, "ab-model");
        m.set_wrap(true);
        m.set_justify(gtk::Justification::Center);
        m.set_xalign(0.5);
        m.set_max_width_chars(20);
        mid.append(&m);
    }
    stage.append(&mid);
    stage.append(&column(&right));
    pv.add(&stage);

    let os = widgets::hbox(0);
    os.add_css_class("preview-card");
    os.add_css_class("ab-os");
    os.set_homogeneous(true);
    let mut cells = vec![(s.system.0.clone(), s.system.1.clone()), ("Linux".to_string(), s.kernel.clone())];
    if !s.hyprland.is_empty() {
        cells.push(("Hyprland".into(), s.hyprland.clone()));
    }
    cells.push(("Up for".into(), sysinfo::duration_text(s.uptime)));
    for (k, v) in cells {
        let c = widgets::vbox(2);
        c.append(&widgets::label(&k, "ab-os-key"));
        let l = widgets::label(&v, "ab-os-value");
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        c.append(&l);
        os.append(&c);
    }
    pv.add(&os);
    pv.foot("dialog-information-symbolic", "Copy details puts all of this on the clipboard, for a bug report or a forum post.");
}

/// The computer's name: changed here through the root helper, or on Time & Language
/// where the system keeps it with its other settings.
fn this_computer(page: &Page) {
    let g = page.group("This computer");
    let host = cmd::output(&["hostnamectl", "hostname"]).unwrap_or_else(sysinfo::hostname);
    let editable = crate::platform::PLATFORM == crate::platform::Platform::Omarchy && crate::platform::system_ready();
    if editable {
        let (r, e) = widgets::entry_row("Name", "Shown on the network and to other devices.", &host, "name", {
            let host = host.clone();
            move |n| {
                let n = n.trim().to_string();
                if !n.is_empty() && n != host {
                    crate::sections::accounts::run_admin(&["set-hostname", &n], None, "Name changed", "about");
                }
            }
        });
        e.set_width_chars(16);
        g.add(&with_tile(r, "computer-symbolic"));
    } else {
        let (r, _) = widgets::nav_row(Some("computer-symbolic"), "Name", "Shown on the network and to other devices.", &host, || {
            window::navigate("region")
        });
        g.add(&r);
    }
    widgets::keywords("hostname computer name network");
}

// ---------- Versions ----------

/// Settings' own version and updates, and extension updates.
fn settings_group(page: &Page) {
    let g = page.group("Versions");
    let s = updates::cached();
    let current = env!("CARGO_PKG_VERSION");
    if let Some(v) = INSTALLED.with(|i| i.borrow().clone()) {
        let (r, _) = widgets::button_row(
            &format!("Settings {v} is installed"),
            "Restart Settings to start using it.",
            "Restart Settings",
            |_| window::restart("about"),
        );
        g.add(&with_tile(r, "preferences-system-symbolic"));
    } else if s.settings_update() {
        let line = widgets::hbox(8);
        if !s.url.is_empty() {
            let notes = gtk::Button::with_label("What's new");
            let url = s.url.clone();
            notes.connect_clicked(move |_| cmd::spawn(&["xdg-open", &url]));
            line.append(&notes);
        }
        let update = gtk::Button::with_label("Update");
        update.add_css_class("suggested-action");
        let can = updates::self_updatable();
        update.set_sensitive(can);
        let latest = s.latest.clone();
        update.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_label("Updating…");
            let (b, latest) = (b.clone(), latest.clone());
            cmd::background(
                || updates::update_settings().map_err(|e| format!("{e:#}")),
                move |r| match r {
                    Ok(()) => {
                        INSTALLED.with(|i| *i.borrow_mut() = Some(latest.clone()));
                        window::toast(&format!("Settings {latest} is installed. Restart to use it."));
                        window::rebuild("about");
                    }
                    Err(e) => {
                        b.set_sensitive(true);
                        b.set_label("Update");
                        window::toast(&format!("Couldn't update: {e}"));
                    }
                },
            );
        });
        line.append(&update);
        let desc = if can {
            format!("You have {current}. Updating takes a few seconds; then restart Settings.")
        } else {
            format!("You have {current}, a development build: update it from its source.")
        };
        let r = widgets::row(&format!("Settings {} is available", s.latest), &desc, Some(line.upcast_ref()));
        g.add(&with_tile(r, "software-update-available-symbolic"));
    } else {
        let (r, _) =
            widgets::button_row(&format!("Settings {current}"), &format!("Up to date · {}", checked_text(&s)), "Check now", check_now);
        g.add(&with_tile(r, "preferences-system-symbolic"));
    }
    widgets::keywords("update upgrade version new release settings app check");

    // Only extensions still installed (the list may predate a removal).
    let names: Vec<String> = crate::ext::installed()
        .into_iter()
        .filter(|e| s.extensions.iter().any(|id| id == e.id()))
        .map(|e| e.manifest.name)
        .collect();
    if !names.is_empty() {
        let (r, _) = widgets::button_row(
            &format!("Extension updates ({})", names.len()),
            &glib_escape(&names.join(", ")),
            "Update all",
            |b| {
                b.set_sensitive(false);
                b.set_label("Updating…");
                let b = b.clone();
                cmd::background(
                    || updates::update_extensions().map_err(|e| format!("{e:#}")),
                    move |r| {
                        b.set_sensitive(true);
                        match r {
                            Ok(n) => window::toast(&format!("Updated {}", n.join(", "))),
                            Err(e) => window::toast(&format!("Couldn't update: {e}")),
                        }
                        window::reload_sections(false);
                        window::rebuild("about");
                        window::rebuild_if_built("extensions");
                    },
                );
            },
        );
        g.add(&with_tile(r, "folder-download-symbolic"));
    }
    let (r, _) = widgets::switch_row(
        "Install extension updates by themselves",
        "Settings looks once a day when it opens. Settings itself always asks first.",
        prefs::get().auto_update_extensions,
        |on| prefs::update(|p| p.auto_update_extensions = on),
    );
    widgets::keywords("automatic auto update extensions");
    g.add(&with_tile(r, "application-x-addon-symbolic"));
}

fn glib_escape(t: &str) -> String {
    gtk::glib::markup_escape_text(t).to_string()
}

// ---------- Your settings ----------

/// What Settings changed: Hyprland options by the page their row is on (page id, the
/// options' keys and titles), and the other things it manages (page id, what).
struct Changes {
    options: Vec<(String, Vec<(String, String)>)>,
    others: Vec<(&'static str, String)>,
    total: usize,
}

fn read_changes() -> Changes {
    let st = crate::backend::store::read(|s| s.clone());
    let mut options: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for key in st.options.keys() {
        let (title, section) = widgets::option_row(key).unwrap_or_else(|| (key.clone(), String::new()));
        match options.iter_mut().find(|(s, _)| *s == section) {
            Some((_, list)) => list.push((key.clone(), title)),
            None => options.push((section, vec![(key.clone(), title)])),
        }
    }
    let mut others: Vec<(&str, String)> = Vec::new();
    if !st.binds.is_empty() || !st.unbinds.is_empty() {
        others.push(("keybindings", format!("{} of your own, {} turned off", st.binds.len(), st.unbinds.len())));
    }
    if !st.monitors.is_empty() {
        others.push((
            "displays",
            format!("{} display{} set up here", st.monitors.len(), if st.monitors.len() == 1 { "" } else { "s" }),
        ));
    }
    let n_startup =
        st.autostart.len() + crate::backend::autostart::list().into_iter().filter(|i| i.enabled && !i.is_system).count();
    if !st.window_rules.is_empty() || !st.layer_rules.is_empty() || n_startup > 0 {
        others.push((
            "rules",
            format!(
                "{n_startup} startup program{}, {} rules",
                if n_startup == 1 { "" } else { "s" },
                st.window_rules.len() + st.layer_rules.len()
            ),
        ));
    }
    if !st.devices.is_empty() {
        others
            .push(("mouse", format!("Settings for {} device{}", st.devices.len(), if st.devices.len() == 1 { "" } else { "s" })));
    }
    if st.gestures != Default::default() {
        others.push(("trackpad", "Swipe and pinch gestures".to_string()));
    }
    if st.max_volume.is_some_and(|v| v != 100) {
        others.push(("audio", "Maximum volume".to_string()));
    }
    Changes { total: st.options.len(), options, others }
}

/// Everything changed here, by page, with a way to see it and to undo it.
fn changes_page(page: &Page, c: &Changes) {
    let g = page.group("");
    g.note(&match (c.total, c.others.is_empty()) {
        (0, true) => "Nothing changed yet: everything uses your own config files and Omarchy's defaults.".to_string(),
        (0, false) => "What Settings manages for you, by page.".to_string(),
        (n, _) => format!(
            "{n} option{} changed here. Reset hands them back to your own config or Omarchy's defaults.",
            if n == 1 { "" } else { "s" }
        ),
    });
    let close = |b: &gtk::Button| {
        if let Some(w) = b.root().and_downcast::<gtk::Window>() {
            w.close();
        }
    };
    for (section, list) in &c.options {
        let page_title = crate::window::section_title(section).unwrap_or("Other options").to_string();
        let names: Vec<String> = list.iter().map(|(_, t)| glib_escape(t)).collect();
        let controls = widgets::hbox(8);
        if !section.is_empty() {
            let show = gtk::Button::with_label("Show");
            let (sec, first) = (section.clone(), list[0].1.clone());
            show.connect_clicked(move |b| {
                close(b);
                crate::window::show_option(&sec, &first);
            });
            controls.append(&show);
        }
        let keys: Vec<String> = list.iter().map(|(k, _)| k.clone()).collect();
        let sec = section.clone();
        let row = widgets::row(&page_title, &names.join(", "), Some(controls.upcast_ref()));
        let r2 = row.clone();
        let reset = widgets::confirm_button("Reset", "Reset all?", move |_| {
            let keys = keys.clone();
            crate::backend::store::update(true, move |s| {
                for k in &keys {
                    s.options.remove(k);
                }
            });
            crate::backend::store::flush();
            window::toast("Back to your own config and Omarchy's defaults");
            r2.set_visible(false);
            // Its page shows the old values until redrawn.
            if let Some(id) = window::section_id(&sec) {
                window::rebuild_if_built(id);
            }
            window::rebuild("about");
        });
        reset.set_tooltip_text(Some("Reset every option on this page that was changed here"));
        controls.append(&reset);
        g.add(&row);
    }
    for (id, desc) in &c.others {
        let id = *id;
        let (r, _) = widgets::button_row(window::section_title(id).unwrap_or(id), desc, "Show", move |b| {
            close(b);
            window::navigate(id);
        });
        g.add(&r);
    }
}

/// Export everything to a file, or bring a backup in.
fn export() {
    use crate::backend::backup;
    let dialog = gtk::FileDialog::builder().title("Export settings").initial_name("omarchy-settings.json").modal(true).build();
    dialog.save(window::window().as_ref(), gtk::gio::Cancellable::NONE, |res| {
        let Ok(file) = res else { return };
        let Some(path) = file.path() else { return };
        match backup::export().and_then(|text| cmd::atomic_write(&path, &text)) {
            Ok(()) => window::toast(&format!("Saved {}", path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default())),
            Err(e) => window::toast(&format!("Couldn't export: {e:#}")),
        }
    });
}

fn import() {
    use crate::backend::backup;
    let dialog = gtk::FileDialog::builder().title("Import settings").modal(true).build();
    dialog.open(window::window().as_ref(), gtk::gio::Cancellable::NONE, |res| {
        let Ok(file) = res else { return };
        let Some(path) = file.path() else { return };
        let files = match std::fs::read_to_string(&path).map_err(anyhow::Error::from).and_then(|t| backup::read_backup(&t)) {
            Ok(f) => f,
            Err(e) => return window::toast(&format!("Couldn't import: {e:#}")),
        };
        let names: Vec<String> = files.iter().map(|(n, _)| n.clone()).collect();
        crate::dialog::ask(
            "Import these settings?",
            &format!("This replaces {} here. Settings restarts afterwards.", names.join(", ")),
            vec![],
            "Import",
            None,
            move |_, _| {
                if let Err(e) = backup::import(&files) {
                    return Some(format!("{e:#}"));
                }
                restored();
                None
            },
        );
    });
}

fn your_settings(page: &Page) {
    let g = page.group("Your settings");
    let c = std::rc::Rc::new(read_changes());
    let pages = c.options.len() + c.others.len();
    let desc = match (c.total, pages) {
        (0, 0) => "Nothing changed yet".to_string(),
        (0, p) => format!("Managed here, on {p} page{}", if p == 1 { "" } else { "s" }),
        (n, p) => format!("{n} setting{} changed here, on {p} page{}", if n == 1 { "" } else { "s" }, if p == 1 { "" } else { "s" }),
    };
    let value = if c.total > 0 { c.total.to_string() } else { String::new() };
    let (r, _) = widgets::nav_row(Some("edit-undo-history-symbolic"), "Your changes", &desc, &value, move || {
        let c = c.clone();
        widgets::subpage("Your changes", 560, move |p| {
            changes_page(p, &c);
            widgets::forget_rows(&p.body);
        });
    });
    widgets::keywords("changed modified reset defaults");
    g.add(&r);

    let buttons = widgets::hbox(8);
    let e = gtk::Button::with_label("Export…");
    e.set_tooltip_text(Some("Save a backup file to keep or to bring to another computer"));
    e.connect_clicked(|_| export());
    let i = gtk::Button::with_label("Import…");
    i.set_tooltip_text(Some("Replace these settings with a backup's. Settings restarts to use them."));
    i.connect_clicked(|_| import());
    buttons.append(&e);
    buttons.append(&i);
    let r = widgets::row(
        "Move your settings",
        "Preferences, Hyprland options, shortcuts, displays, sound and lighting, in one file.",
        Some(buttons.upcast_ref()),
    );
    widgets::keywords("export backup save copy transfer migrate import restore load");
    g.add(&with_tile(r, "document-send-symbolic"));
}

/// Take a restored backup into use, then restart for the rest.
fn restored() {
    use crate::backend::{audio, state::State, store};
    let fresh = State::load(&crate::paths::state_file());
    store::update(true, move |s| *s = fresh);
    store::flush();
    crate::prefs::reload();
    crate::theme::apply();
    cmd::background(
        || {
            let eq = audio::load();
            if eq.enabled { audio::install_and_start(&eq).map_err(|e| format!("{e:#}")) } else { Ok(()) }
        },
        |r| {
            if let Err(e) = r {
                window::toast(&format!("Couldn't apply the sound settings: {e}"));
            }
            window::restart("about");
        },
    );
}

pub fn build(page: &Page) {
    computer_preview(page);
    this_computer(page);
    settings_group(page);
    crate::platform::about(page);
    your_settings(page);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphics_from_lspci() {
        let t = "00:02.0 \"VGA compatible controller\" \"Intel Corporation\" \"Arrow Lake-P [Arc Pro 130T/140T]\" -r03 -p00 \"ASUSTeK Computer Inc.\" \"Device 3778\"\n\
                 00:14.0 \"USB controller\" \"Intel Corporation\" \"Device 7f6e\"\n\
                 01:00.0 \"VGA compatible controller\" \"NVIDIA Corporation\" \"GB203M / GN22-X9 [GeForce RTX 5080 Max-Q / Mobile]\" -ra1 -p00 \"ASUSTeK Computer Inc.\" \"Device 3a18\"\n";
        assert_eq!(
            parse_gpus(t),
            vec![("Intel Arc Pro 130T/140T".to_string(), true), ("NVIDIA GeForce RTX 5080 Max-Q".to_string(), false)]
        );
    }

    #[test]
    fn memory_as_sold() {
        assert_eq!(memory_size(31_100_000_000), "32 GB");
        assert_eq!(memory_size(15 * (1 << 30)), "16 GB");
    }
}
