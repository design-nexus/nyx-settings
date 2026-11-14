use crate::backend::kbdidle;
use crate::sketch;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

fn state_file(source: &str) -> std::path::PathBuf {
    crate::platform::power_profile_state_dir().join(source)
}

/// What Omarchy has saved for "ac" or "battery", if anything.
fn read_saved(source: &str) -> Option<String> {
    std::fs::read_to_string(state_file(source)).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// The profile Omarchy will use for a power source: what's saved, else its own default
/// (Performance on AC when available, otherwise Balanced).
fn effective_profile(saved: Option<&str>, source: &str, profiles: &[String]) -> String {
    if let Some(s) = saved.filter(|s| profiles.iter().any(|p| p == s)) {
        return s.to_string();
    }
    if source == "ac" && profiles.iter().any(|p| p == "performance") { "performance".into() } else { "balanced".into() }
}

fn on_battery() -> bool {
    cmd::output(&[
        "busctl",
        "get-property",
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
        "OnBattery",
    ])
    .is_some_and(|s| s.trim() == "b true")
}

/// Save the profile for a power source. Omarchy's own command also switches to it right away, which
/// is only wanted when that source is the one in use, so for the other source just record the choice.
fn remember(source: &str, profile: &str, in_use: bool) -> anyhow::Result<()> {
    crate::platform::set_power_profile(source, profile, in_use)
}

/// The Status and Time remaining lines, from `omarchy-battery-status --shell`
/// (tab-separated fields). Empty fields are left out.
fn battery_lines(shell: &str) -> (String, String) {
    let get = |k: &str| {
        shell
            .lines()
            .find_map(|l| l.split_once('\t').filter(|(key, _)| *key == k).map(|(_, v)| v.trim().to_string()))
            .unwrap_or_default()
    };
    let (state, time, rate, size) = (get("state"), get("time"), get("rate"), get("size"));
    let word = match state.as_str() {
        "charging" => "Charging".to_string(),
        "discharging" => "On battery".to_string(),
        "fully-charged" => "Fully charged".to_string(),
        "holding" => match get("threshold") {
            t if t.is_empty() => "Holding charge".to_string(),
            t => format!("Holding at {t}"),
        },
        "pending-charge" => "Plugged in, not charging".to_string(),
        _ => String::new(),
    };
    let drawing = rate.trim_end_matches('W').parse::<f64>().is_ok_and(|r| r > 0.0);
    let draw = match (drawing, size.is_empty()) {
        (true, false) => format!("{rate} / {size}"),
        (true, true) => rate,
        (false, _) => size,
    };
    let status = [get("percentage"), word, draw].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join("  ·  ");
    let remaining = match state.as_str() {
        "discharging" | "charging" if time.is_empty() => "Estimating…".to_string(),
        "discharging" => format!("{time} left"),
        "charging" => format!("{time} to full"),
        "" => String::new(),
        _ => "Plugged in".to_string(),
    };
    (status, remaining)
}

/// One `key\tvalue` field of the battery's shell lines.
fn field(shell: &str, k: &str) -> String {
    shell.lines().find_map(|l| l.split_once('\t').filter(|(key, _)| *key == k).map(|(_, v)| v.trim().to_string())).unwrap_or_default()
}

/// The battery's own folder in sysfs, for what the status lines don't say.
fn battery_sysfs() -> Option<std::path::PathBuf> {
    std::fs::read_dir("/sys/class/power_supply")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Battery"))
}

/// How much it holds now, against when it was new, in percent.
fn health() -> Option<f64> {
    let dir = battery_sysfs()?;
    let read = |n: &str| std::fs::read_to_string(dir.join(n)).ok()?.trim().parse::<f64>().ok();
    let (full, design) = read("energy_full").zip(read("energy_full_design")).or_else(|| read("charge_full").zip(read("charge_full_design")))?;
    (design > 0.0).then(|| (full / design * 100.0).min(100.0))
}

/// "Hades · ROG Zephyrus G16": this computer's name and model.
fn computer_line() -> String {
    let host = gtk::glib::host_name().to_string();
    let model = std::fs::read_to_string("/sys/class/dmi/id/product_name").unwrap_or_default();
    let model = model.trim().split('_').next().unwrap_or("").trim().to_string();
    [host, model].into_iter().filter(|s| !s.is_empty() && s != "System Product Name").collect::<Vec<_>>().join(" · ")
}

/// What the page asks while being built, for [`cmd::fetch_ahead`].
pub const READS: &[&[&str]] = &[&["powerprofilesctl", "list"], &["powerprofilesctl", "get"]];

fn profile_label(p: &str) -> String {
    match p {
        "power-saver" => "Power saver".to_string(),
        "balanced" => "Balanced".to_string(),
        "performance" => "Performance".to_string(),
        other => other.to_string(),
    }
}

/// A profile's icon and what it's for.
fn profile_about(p: &str) -> (&'static str, &'static str) {
    match p {
        "power-saver" => ("power-profile-power-saver-symbolic", "Longest battery"),
        "performance" => ("power-profile-performance-symbolic", "Fastest, warmest"),
        _ => ("power-profile-balanced-symbolic", "For most things"),
    }
}

pub fn build(page: &Page) {
    // The rows for a battery, screen brightness and a keyboard light come and go with them.
    crate::capability::rebuild_on(&["hw:battery", "hw:backlight", "hw:keyboard-backlight"], "power");
    let pv = page.split();
    battery_preview(&pv);

    // ----- Profile -----
    let profiles = crate::platform::power_profiles();
    let current = cmd::read(&["powerprofilesctl", "get"]).unwrap_or_default();
    let g = page.plain_group("Power profile");
    let cards = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    cards.set_homogeneous(true);
    cards.add_css_class("pwr-profiles");
    let mut first: Option<gtk::ToggleButton> = None;
    for p in &profiles {
        let (icon, about) = profile_about(p);
        let b = gtk::ToggleButton::new();
        b.add_css_class("pwr-profile");
        super::displays::tint(&b);
        if let Some(f) = &first {
            b.set_group(Some(f));
        } else {
            first = Some(b.clone());
        }
        let col = widgets::vbox(2);
        let i = gtk::Image::from_icon_name(icon);
        i.set_halign(gtk::Align::Start);
        i.add_css_class("pwr-profile-icon");
        col.append(&i);
        let name = widgets::label(&profile_label(p), "pwr-profile-name");
        let d = widgets::label(about, "pwr-profile-about");
        for l in [&name, &d] {
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            col.append(l);
        }
        b.set_child(Some(&col));
        b.set_active(*p == current);
        b.set_tooltip_text(Some(&format!("Use {} right now", profile_label(p).to_lowercase())));
        let p2 = p.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                cmd::run_async(&["powerprofilesctl", "set", &p2], |r| {
                    if let Err(e) = r {
                        window::toast(&format!("{e}"));
                    }
                });
            }
        });
        cards.append(&b);
    }
    // Search finds the cards by this row, which stands for them.
    let r = widgets::stacked_row("", "", cards.upcast_ref());
    r.add_css_class("bare");
    widgets::keywords("right now power profile power saver balanced performance");
    g.add(&r);

    let g = page.group("");
    let options: Vec<(String, String)> = profiles.iter().map(|p| (p.clone(), profile_label(p))).collect();
    let on_battery = on_battery();
    for (when, title, icon) in [("ac", "When plugged in", "ac-adapter-symbolic"), ("battery", "On battery", "battery-level-80-symbolic")] {
        let in_use = (when == "battery") == on_battery;
        let desc = if in_use { "In use now" } else { "" };
        let saved = effective_profile(read_saved(when).as_deref(), when, &profiles);
        let (r, _) = widgets::choice_row(title, desc, options.clone(), &saved, move |p| {
            cmd::background(
                move || remember(when, &p, in_use).map_err(|e| format!("{e:#}")),
                move |r| match r {
                    // The profile in use changed, so the cards are stale.
                    Ok(()) if in_use => window::rebuild("power"),
                    Ok(()) => {}
                    Err(e) => window::toast(&e),
                },
            );
        });
        r.prepend(&widgets::tile(icon));
        r.add_css_class("with-tile");
        widgets::keywords("switched automatically when power changes plugged unplugged battery ac");
        g.add(&r);
    }

    // ----- Brightness -----
    let g = page.group("Brightness");
    if let Some(b) = crate::platform::brightness() {
        let (r, slider) = widgets::slider_row("Screen", "", (1.0, 100.0, 1.0), b, 0, "%", |v| {
            crate::platform::set_brightness(v.round() as i64);
        });
        slider.scale.set_width_request(130);
        r.prepend(&widgets::tile("display-brightness-symbolic"));
        r.add_css_class("with-tile");
        widgets::keywords("display brightness backlight focused screen");
        g.add(&r);
    }
    if cmd::present("brightnessctl") {
        let kbd = cmd::output(&["brightnessctl", "-l", "-c", "leds", "-m"]).unwrap_or_default();
        if let Some(line) = kbd.lines().find(|l| l.contains("kbd_backlight")) {
            let parts: Vec<&str> = line.split(',').collect();
            let dev = parts.first().unwrap_or(&"").to_string();
            let max: f64 = parts.get(4).and_then(|m| m.parse().ok()).unwrap_or(3.0);
            let cur: f64 = parts.get(2).and_then(|m| m.parse().ok()).unwrap_or(0.0);
            let (r, slider) = widgets::slider_row("Keyboard light", "", (0.0, max, 1.0), cur, 0, "", move |v| {
                cmd::spawn(&["brightnessctl", "-d", &dev, "set", &(v.round() as i64).to_string()]);
            });
            let steps = move |v: f64| if v < 0.5 { "Off".to_string() } else { format!("{} of {}", v.round(), max.round()) };
            slider.readout.set_text(&steps(cur));
            slider.scale.set_width_request(130);
            let readout = slider.readout.clone();
            slider.scale.connect_value_changed(move |s| readout.set_text(&steps(s.value())));
            r.prepend(&widgets::tile("keyboard-brightness-symbolic"));
            r.add_css_class("with-tile");
            widgets::keywords("keyboard backlight light backlit");
            g.add(&r);
            if let Some(t) = timeout_row() {
                t.prepend(&widgets::tile("preferences-system-time-symbolic"));
                t.add_css_class("with-tile");
                g.add(&t);
            }
        }
    }
    if g.list.first_child().is_none() {
        g.wrapper.set_visible(false);
    }

    // ----- One level down -----
    let g = page.group("");
    let lid = parse_logind(logind_files().iter().map(String::as_str));
    let laptop = std::path::Path::new("/proc/acpi/button/lid").exists();
    let (key, fallback) = if laptop { ("HandleLidSwitch", "suspend") } else { ("HandlePowerKey", "poweroff") };
    let value = action_label(lid.get(key).map(String::as_str).unwrap_or(fallback));
    let (r, _) = widgets::nav_row(
        Some(if laptop { "computer-laptop-symbolic" } else { "system-shutdown-symbolic" }),
        if laptop { "Lid and power button" } else { "Power button" },
        "",
        &value,
        || {
            widgets::subpage("Lid and power button", 600, buttons_group);
        },
    );
    widgets::keywords("lid close laptop button power key suspend hibernate ignore logind sleep delay hibernate timer battery");
    g.add(&r);
    let (r, _) = widgets::nav_row(Some("system-lock-screen-symbolic"), "Lock, sleep and shut down", "", "", || {
        widgets::subpage("Lock, sleep and shut down", 600, session_group);
    });
    widgets::keywords("shut down shutdown power off restart reboot log out logout suspend sleep hibernate lock session");
    g.add(&r);
    if cmd::present("supergfxctl") && cmd::run(&["omarchy-hw-hybrid-gpu"]).is_ok() {
        let mode = cmd::output(&["supergfxctl", "-g"]).unwrap_or_else(|| "Unknown".into());
        let (r, _) = widgets::nav_row(Some("applications-engineering-symbolic"), "Graphics", "", &mode, || {
            widgets::subpage("Graphics", 560, graphics_group);
        });
        widgets::keywords("gpu nvidia hybrid integrated dedicated supergfx asus battery graphics mode");
        g.add(&r);
    }
}

fn action_label(v: &str) -> String {
    match v {
        "ignore" => "Do nothing",
        "suspend" => "Suspend",
        "hibernate" => "Hibernate",
        "suspend-then-hibernate" => "Suspend, then hibernate",
        "lock" => "Lock",
        "poweroff" => "Power off",
        other => other,
    }
    .to_string()
}

/// The preview: the battery as a gauge, with the time left and what it's using, then
/// its health and charge limit. A computer without one says so.
fn battery_preview(pv: &widgets::Preview) {
    let present = crate::platform::battery_present();
    let shell = if present { crate::platform::battery_shell() } else { String::new() };
    let pill = widgets::label("", "pwr-state");
    pill.set_valign(gtk::Align::Center);
    super::displays::tint(&pill);
    pv.head(if present { "Battery" } else { "Power" }, &computer_line(), &[pill.upcast_ref()]);
    if !present {
        pill.set_visible(false);
        let card = pv.card();
        card.add_css_class("pwr-none");
        let r = widgets::icon_row("ac-adapter-symbolic", "No battery", "This computer runs on mains power. The profiles still choose between quiet and fast.", None);
        card.append(&r);
        pv.foot("emblem-ok-symbolic", "Power saver, Balanced and Performance change how fast the processor may run.");
        return;
    }

    let level = Rc::new(Cell::new(0.0));
    let charging = Rc::new(Cell::new(false));
    let hero = pv.card();
    hero.add_css_class("pwr-hero");
    let row = widgets::hbox(24);
    hero.append(&row);
    let gauge = {
        let (level, charging) = (level.clone(), charging.clone());
        widgets::drawing(110, move |cr, p, _accent, w, h| draw_battery(cr, p, w, h, level.get(), charging.get()))
    };
    gauge.set_hexpand(false);
    gauge.set_content_width(150);
    row.append(&gauge);
    let text = widgets::vbox(3);
    text.set_valign(gtk::Align::Center);
    let pct = widgets::label("", "pwr-percent");
    let left = widgets::label("", "pwr-left");
    let using = widgets::label("", "pwr-using");
    for l in [&pct, &left, &using] {
        l.set_xalign(0.0);
        l.set_wrap(true);
        text.append(l);
    }
    row.append(&text);

    // Health, the charge limit, charge cycles and size, where the battery says.
    let facts = pv.card();
    facts.add_css_class("pwr-facts");
    let facts_row = gtk::FlowBox::new();
    facts_row.set_selection_mode(gtk::SelectionMode::None);
    facts_row.set_homogeneous(true);
    facts_row.set_min_children_per_line(2);
    facts_row.set_max_children_per_line(4);
    facts_row.set_column_spacing(12);
    facts_row.set_row_spacing(10);
    facts_row.set_can_target(false);
    facts.append(&facts_row);
    let fact = |k: &str, v: String| {
        let col = widgets::vbox(2);
        let a = widgets::label(k, "pwr-fact-key");
        let b = widgets::label(&v, "pwr-fact-value");
        a.set_xalign(0.0);
        b.set_xalign(0.0);
        col.append(&a);
        col.append(&b);
        col
    };
    if let Some(h) = health() {
        facts_row.append(&fact("Holds", format!("{}% of new", h.round())));
    }
    let threshold = field(&shell, "threshold");
    if let Some(top) = threshold.rsplit('-').next().filter(|t| !t.is_empty()) {
        facts_row.append(&fact("Charge limit", format!("Stops at {top}")));
    }
    let cycles = field(&shell, "cycles");
    if cycles.parse::<u32>().is_ok_and(|c| c > 0) {
        facts_row.append(&fact("Charged", format!("{cycles} times")));
    }
    let size = field(&shell, "size");
    if !size.is_empty() {
        facts_row.append(&fact("Size", size.replace("Wh", " Wh")));
    }
    facts.set_visible(facts_row.first_child().is_some());

    let show = {
        let (pill, pct, left, using, gauge) = (pill.clone(), pct.clone(), left.clone(), using.clone(), gauge.clone());
        move |shell: &str| {
            let state = field(shell, "state");
            let percent = field(shell, "percentage");
            let value = percent.trim_end_matches('%').parse::<f64>().unwrap_or(0.0);
            level.set(value / 100.0);
            charging.set(state == "charging");
            gauge.queue_draw();
            pct.set_text(if percent.is_empty() { "Unknown" } else { &percent });
            let (_, line) = battery_lines(shell);
            let time = field(shell, "time");
            pill.set_text(match state.as_str() {
                "discharging" => "On battery",
                "charging" => "Charging",
                "" => "",
                _ => "Plugged in",
            });
            pill.set_visible(!state.is_empty());
            left.set_text(&match state.as_str() {
                "discharging" if !time.is_empty() => format!("About {time} left"),
                "charging" if !time.is_empty() => format!("Charging, {time} to full"),
                "fully-charged" => "Fully charged".to_string(),
                "holding" => match field(shell, "threshold") {
                    t if t.is_empty() => "Holding its charge".to_string(),
                    t => format!("Holding at {}", t.rsplit('-').next().unwrap_or(&t)),
                },
                "pending-charge" => "Plugged in, not charging".to_string(),
                _ => line,
            });
            let rate = field(shell, "rate");
            let drawing = rate.trim_end_matches('W').parse::<f64>().is_ok_and(|r| r > 0.0);
            using.set_text(&if drawing && state == "discharging" {
                format!("Using {} right now", rate.replace('W', " W"))
            } else if drawing && state == "charging" {
                format!("Charging at {}", rate.replace('W', " W"))
            } else {
                String::new()
            });
            using.set_visible(drawing);
        }
    };
    show(&shell);
    // Keep it current while the page exists.
    let weak = pct.downgrade();
    gtk::glib::timeout_add_seconds_local(10, move || match weak.upgrade() {
        Some(l) => {
            if l.is_mapped() {
                show(&crate::platform::battery_shell());
            }
            gtk::glib::ControlFlow::Continue
        }
        None => gtk::glib::ControlFlow::Break,
    });

    pv.foot(
        "emblem-ok-symbolic",
        if threshold.is_empty() {
            "Profiles switch on their own when you plug in or unplug."
        } else {
            "Stopping short of full while plugged in helps the battery last for years."
        },
    );
}

/// A battery drawn `level` full (0–1), with a bolt while charging.
fn draw_battery(cr: &gtk::cairo::Context, p: &sketch::Pal, w: f64, h: f64, level: f64, charging: bool) {
    let nub = 8.0;
    let (bw, bh) = ((w - nub - 4.0).min(h * 1.75), h - 8.0);
    let (x, y) = (2.0, 4.0);
    sketch::rounded(cr, x, y, bw, bh, 12.0);
    sketch::set(cr, p.dim, 0.55);
    cr.set_line_width(2.5);
    let _ = cr.stroke();
    sketch::rounded(cr, x + bw + 3.0, y + bh * 0.33, nub - 2.0, bh * 0.34, 3.0);
    sketch::set(cr, p.dim, 0.55);
    let _ = cr.fill();
    let inset = 7.0;
    let fw = (bw - 2.0 * inset) * level.clamp(0.0, 1.0);
    let low = level <= 0.15 && !charging;
    let good = (0.30, 0.78, 0.40);
    let c = if low { p.danger } else { good };
    if fw > 0.5 {
        let g = gtk::cairo::LinearGradient::new(0.0, y, 0.0, y + bh);
        let top = sketch::mix(c, (1.0, 1.0, 1.0), 0.12);
        g.add_color_stop_rgb(0.0, top.0, top.1, top.2);
        g.add_color_stop_rgb(1.0, c.0, c.1, c.2);
        sketch::rounded(cr, x + inset, y + inset, fw, bh - 2.0 * inset, 7.0);
        let _ = cr.set_source(&g);
        let _ = cr.fill();
    }
    if charging {
        let (cx, cy, s) = (x + bw / 2.0, y + bh / 2.0, bh * 0.3);
        cr.move_to(cx + s * 0.15, cy - s);
        cr.line_to(cx - s * 0.55, cy + s * 0.12);
        cr.line_to(cx - s * 0.02, cy + s * 0.12);
        cr.line_to(cx - s * 0.15, cy + s);
        cr.line_to(cx + s * 0.55, cy - s * 0.12);
        cr.line_to(cx + s * 0.02, cy - s * 0.12);
        cr.close_path();
        sketch::set(cr, (1.0, 1.0, 1.0), 0.95);
        let _ = cr.fill_preserve();
        sketch::set(cr, p.bg, 0.5);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    }
}

/// Lock, suspend and hibernate now; and the end of the session.
fn session_group(page: &Page) {
    let g = page.group("Right now");
    let now = widgets::hbox(8);
    now.append(&widgets::command_button("Lock", crate::platform::lock_command()));
    // Omarchy hides these when suspend is switched off or hibernation isn't set up.
    if cmd::run(&["omarchy-toggle-enabled", "suspend-off"]).is_err() {
        now.append(&widgets::command_button("Suspend", &["systemctl", "suspend"]));
    }
    if crate::platform::hibernation_available() {
        now.append(&widgets::command_button("Hibernate", &["systemctl", "hibernate"]));
    }
    g.add(&widgets::icon_row("system-lock-screen-symbolic", "Lock or sleep", "", Some(now.upcast_ref())));
    let end = widgets::hbox(8);
    for (label, confirm, what) in
        [("Log out", "Click again to log out", "logout"), ("Restart", "Click again to restart", "reboot"), ("Shut down", "Click again to shut down", "shutdown")]
    {
        let b = widgets::confirm_button(label, confirm, move |_| cmd::spawn(crate::platform::session_command(what)));
        b.add_css_class("destructive-action");
        end.append(&b);
    }
    let r = widgets::stacked_row("End the session", "Open apps are closed. Each needs a second click.", end.upcast_ref());
    g.add(&r);
    widgets::keywords("shut down shutdown power off restart reboot log out logout suspend sleep hibernate lock");
}

/// Integrated or dedicated graphics on laptops with both.
fn graphics_group(page: &Page) {
    let g = page.group("Graphics");
    let mode = cmd::output(&["supergfxctl", "-g"]).unwrap_or_else(|| "unknown".into());
    let (r, _) = widgets::button_row(
        "Graphics mode",
        &format!("Now: {mode}. Switching between integrated (longer battery) and dedicated (faster) graphics logs you out."),
        "Switch…",
        |_| crate::sections::accounts::terminal("omarchy-toggle-hybrid-gpu"),
    );
    widgets::keywords("gpu nvidia hybrid integrated dedicated supergfx asus battery");
    g.add(&r);
}

/// What logind does for each lid or button event, from `logind.conf` and its
/// drop-ins (a later file wins). Only `Key=Value` lines matter here.
fn parse_logind<'a>(files: impl IntoIterator<Item = &'a str>) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    for text in files {
        for line in text.lines().map(str::trim).filter(|l| !l.starts_with('#') && !l.starts_with(';')) {
            if let Some((k, v)) = line.split_once('=') {
                m.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    m
}

/// Every logind config file in the order systemd reads them.
fn logind_files() -> Vec<String> {
    let mut paths: Vec<std::path::PathBuf> = vec!["/usr/lib/systemd/logind.conf".into(), "/etc/systemd/logind.conf".into()];
    let mut dropins: Vec<std::path::PathBuf> = Vec::new();
    for dir in ["/usr/lib/systemd/logind.conf.d", "/etc/systemd/logind.conf.d"] {
        dropins.extend(
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "conf")),
        );
    }
    // Drop-ins are sorted by file name, whichever directory holds them.
    dropins.sort_by_key(|p| p.file_name().map(|n| n.to_os_string()));
    paths.extend(dropins);
    paths.iter().filter_map(|p| std::fs::read_to_string(p).ok()).collect()
}

fn hibernate_delay_secs() -> u32 {
    let mut secs = 7200;
    for dir in ["/usr/lib/systemd/sleep.conf.d", "/etc/systemd/sleep.conf.d"] {
        let mut files: Vec<std::path::PathBuf> =
            std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files {
            let text = std::fs::read_to_string(f).unwrap_or_default();
            if let Some(v) = parse_logind([text.as_str()]).get("HibernateDelaySec") {
                secs = time_span_secs(v).unwrap_or(secs);
            }
        }
    }
    secs
}

/// A systemd time span such as `7200`, `90min`, `2h` or `1h 30min`, in seconds.
fn time_span_secs(text: &str) -> Option<u32> {
    let mut total: u32 = 0;
    let mut any = false;
    let mut rest = text.trim();
    while !rest.is_empty() {
        let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        let n: u32 = rest[..digits].parse().ok()?;
        rest = rest[digits..].trim_start();
        let unit_len = rest.find(|c: char| c.is_ascii_digit() || c.is_whitespace()).unwrap_or(rest.len());
        let scale = match &rest[..unit_len] {
            "" | "s" | "sec" | "second" | "seconds" => 1,
            "m" | "min" | "minute" | "minutes" => 60,
            "h" | "hr" | "hour" | "hours" => 3600,
            "d" | "day" | "days" => 86400,
            _ => return None,
        };
        total = total.checked_add(n.checked_mul(scale)?)?;
        any = true;
        rest = rest[unit_len..].trim_start();
    }
    any.then_some(total)
}

/// Lid, power button and sleep delay: system settings, made the platform's way.
fn buttons_group(page: &Page) {
    let g = page.group("What happens when");
    let (can_edit, notice) = crate::platform::system_notice("power", "changing what the lid and power button do");
    if let Some(n) = notice {
        g.top(&n);
    }
    let conf = parse_logind(logind_files().iter().map(String::as_str));
    let hibernate = crate::platform::hibernation_available();
    let mut actions = vec![("ignore", "Do nothing"), ("suspend", "Suspend")];
    if hibernate {
        actions.extend([("hibernate", "Hibernate"), ("suspend-then-hibernate", "Suspend, then hibernate")]);
    }
    actions.extend([("lock", "Lock"), ("poweroff", "Power off")]);

    let laptop = std::path::Path::new("/proc/acpi/button/lid").exists();
    let mut rows: Vec<(&'static str, &str, &str, &str)> = Vec::new();
    if laptop {
        rows.push(("HandleLidSwitch", "When the lid closes", "On battery.", "suspend"));
        rows.push((
            "HandleLidSwitchExternalPower",
            "When the lid closes on power",
            "Plugged in. Follows the setting above unless you change it.",
            "suspend",
        ));
        rows.push(("HandleLidSwitchDocked", "When the lid closes while docked", "With an external screen connected.", "ignore"));
    }
    rows.push(("HandlePowerKey", "When you press the power button", "", "poweroff"));
    for (key, title, desc, default) in rows {
        let default = if key == "HandleLidSwitchExternalPower" {
            conf.get("HandleLidSwitch").map(String::as_str).unwrap_or(default)
        } else {
            default
        };
        let current = conf.get(key).map(String::as_str).unwrap_or(default).to_string();
        let mut options: Vec<(String, String)> = actions.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect();
        if !options.iter().any(|(v, _)| *v == current) {
            options.push((current.clone(), current.clone()));
        }
        let (r, dd) = widgets::choice_row(title, desc, options, &current, {
            let current = current.clone();
            move |v| {
                if v != current {
                    crate::platform::set_power(key, &v, "power");
                }
            }
        });
        dd.set_sensitive(can_edit);
        widgets::keywords("lid close laptop button power key suspend hibernate ignore logind");
        g.add(&r);
    }

    if hibernate {
        let secs = hibernate_delay_secs();
        let mut options = widgets::opts(&[
            ("1800", "30 minutes"),
            ("3600", "1 hour"),
            ("7200", "2 hours"),
            ("10800", "3 hours"),
            ("14400", "4 hours"),
        ]);
        let now = secs.to_string();
        if !options.iter().any(|(v, _)| *v == now) {
            options.push((now.clone(), format!("{} minutes", secs / 60)));
        }
        let (r, dd) = widgets::choice_row(
            "Hibernate after suspending for",
            "Used by “Suspend, then hibernate”: wake-ups are quick at first, then the battery lasts longer.",
            options,
            &now,
            {
                let now = now.clone();
                move |v| {
                    if v != now {
                        crate::platform::set_hibernate_delay(&v, "power");
                    }
                }
            },
        );
        dd.set_sensitive(can_edit);
        widgets::keywords("sleep delay hibernate timer battery");
        g.add(&r);
    }
}

/// How long the keyboard backlight stays on without input.
fn timeout_row() -> Option<gtk::Box> {
    if !kbdidle::available() {
        return None;
    }
    let cur = kbdidle::load().timeout_secs;
    let mut options: Vec<(String, String)> = kbdidle::CHOICES.iter().map(|(s, l)| (s.to_string(), l.to_string())).collect();
    if !kbdidle::CHOICES.iter().any(|(s, _)| *s == cur) {
        options.push((cur.to_string(), format!("{cur} seconds")));
    }
    let (r, _) = widgets::choice_row(
        "Turn the keyboard light off after",
        "",
        options,
        &cur.to_string(),
        |v| {
            let secs = v.parse::<u32>().unwrap_or(0);
            cmd::background(
                move || kbdidle::apply(&kbdidle::Config { timeout_secs: secs }).map_err(|e| format!("{e:#}")),
                move |r| match r {
                    Ok(()) if secs == 0 => window::toast("Keyboard backlight stays on"),
                    Ok(()) => window::toast("Keyboard backlight timeout set"),
                    Err(e) => window::toast(&format!("Couldn't set the timeout: {e}")),
                },
            );
        },
    );
    widgets::keywords("keyboard backlight timeout idle off sleep dim inactivity typing trackpad");
    Some(r)
}

#[cfg(test)]
mod tests {
    #[test]
    fn logind_later_files_win() {
        let m = super::parse_logind([
            "[Login]\nHandlePowerKey=poweroff\n#HandleLidSwitch=x\n",
            "[Login]\nHandlePowerKey=ignore\nHandleLidSwitch = suspend\n",
        ]);
        assert_eq!(m["HandlePowerKey"], "ignore");
        assert_eq!(m["HandleLidSwitch"], "suspend");
    }

    #[test]
    fn time_spans() {
        assert_eq!(super::time_span_secs("7200"), Some(7200));
        assert_eq!(super::time_span_secs("90min"), Some(5400));
        assert_eq!(super::time_span_secs("2h"), Some(7200));
        assert_eq!(super::time_span_secs("1h 30min"), Some(5400));
        assert_eq!(super::time_span_secs("soon"), None);
        assert_eq!(super::time_span_secs(""), None);
    }

    use super::*;

    #[test]
    fn battery_lines_skip_empty_parts() {
        let full = "percentage\t100%\nstate\tfully-charged\nrate\t0W\nsize\t90Wh\ntime\t\ncycles\t0\nthreshold\t75-80%\n";
        assert_eq!(battery_lines(full), ("100%  ·  Fully charged  ·  90Wh".into(), "Plugged in".into()));
        let on_battery = "percentage\t64%\nstate\tdischarging\nrate\t12.3W\nsize\t90Wh\ntime\t3h 5m\n";
        assert_eq!(battery_lines(on_battery), ("64%  ·  On battery  ·  12.3W / 90Wh".into(), "3h 5m left".into()));
        let charging = "percentage\t40%\nstate\tcharging\nrate\t45W\nsize\t90Wh\ntime\t\n";
        assert_eq!(battery_lines(charging).1, "Estimating…");
        let holding = "percentage\t80%\nstate\tholding\nrate\t0W\nsize\t90Wh\ntime\t\nthreshold\t75-80%\n";
        assert_eq!(battery_lines(holding).0, "80%  ·  Holding at 75-80%  ·  90Wh");
        assert_eq!(battery_lines(""), (String::new(), String::new()));
    }

    fn all() -> Vec<String> {
        ["power-saver", "balanced", "performance"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn uses_the_saved_profile() {
        assert_eq!(effective_profile(Some("power-saver"), "battery", &all()), "power-saver");
        assert_eq!(effective_profile(Some("balanced"), "ac", &all()), "balanced");
    }

    #[test]
    fn falls_back_like_omarchy() {
        assert_eq!(effective_profile(None, "ac", &all()), "performance");
        assert_eq!(effective_profile(None, "battery", &all()), "balanced");
        let no_perf: Vec<String> = vec!["power-saver".into(), "balanced".into()];
        assert_eq!(effective_profile(None, "ac", &no_perf), "balanced");
        // A saved profile this machine doesn't offer is ignored.
        assert_eq!(effective_profile(Some("performance"), "ac", &no_perf), "balanced");
    }

    #[test]
    fn records_the_other_source_without_running_omarchy() {
        let dir = std::env::temp_dir().join(format!("settings-power-test-{}", std::process::id()));
        // SAFETY: no other test reads or writes this variable.
        unsafe {
            std::env::set_var("OMARCHY_POWERPROFILES_STATE_DIR", &dir);
            std::env::set_var("NYX_POWER_PROFILES_STATE_DIR", &dir);
        }
        remember("battery", "power-saver", false).unwrap();
        assert_eq!(read_saved("battery").as_deref(), Some("power-saver"));
        assert!(read_saved("ac").is_none());
        let _ = std::fs::remove_dir_all(&dir);
        unsafe {
            std::env::remove_var("OMARCHY_POWERPROFILES_STATE_DIR");
            std::env::remove_var("NYX_POWER_PROFILES_STATE_DIR");
        }
    }
}
