use crate::backend::kbdidle;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;

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

/// What the page asks while being built, for [`cmd::fetch_ahead`].
pub const READS: &[&[&str]] = &[&["powerprofilesctl", "list"], &["powerprofilesctl", "get"]];

pub fn build(page: &Page) {
    // The rows for a battery, screen brightness and a keyboard light come and go with them.
    crate::capability::rebuild_on(&["hw:battery", "hw:backlight", "hw:keyboard-backlight"], "power");
    // ----- Profile -----
    let g = page.group("Power profile");
    let profiles = crate::platform::power_profiles();
    let current = cmd::read(&["powerprofilesctl", "get"]).unwrap_or_default();
    let label = |p: &str| match p {
        "power-saver" => "Power saver".to_string(),
        "balanced" => "Balanced".to_string(),
        "performance" => "Performance".to_string(),
        other => other.to_string(),
    };
    let options: Vec<(String, String)> = profiles.iter().map(|p| (p.clone(), label(p))).collect();
    let (r, _) = widgets::choice_row("Right now", "", options.clone(), &current, |p| {
        cmd::run_async(&["powerprofilesctl", "set", &p], |r| {
            if let Err(e) = r {
                window::toast(&format!("{e}"));
            }
        });
    });
    g.add(&r);
    let on_battery = on_battery();
    for (when, title) in [("ac", "When plugged in"), ("battery", "On battery")] {
        let in_use = (when == "battery") == on_battery;
        let desc = if in_use {
            "Switched to automatically when power changes. This is the one in use now."
        } else {
            "Switched to automatically when power changes."
        };
        let saved = effective_profile(read_saved(when).as_deref(), when, &profiles);
        let (r, _) = widgets::choice_row(title, desc, options.clone(), &saved, move |p| {
            cmd::background(
                move || remember(when, &p, in_use).map_err(|e| format!("{e:#}")),
                move |r| match r {
                    // The profile in use changed, so "Right now" is stale.
                    Ok(()) if in_use => window::rebuild("power"),
                    Ok(()) => {}
                    Err(e) => window::toast(&e),
                },
            );
        });
        g.add(&r);
    }

    // ----- Battery -----
    if crate::platform::battery_present() {
        let g = page.group("Battery");
        let read = || {
            let (status, time) = battery_lines(&crate::platform::battery_shell());
            let or_unknown = |s: String| if s.is_empty() { "Unknown".to_string() } else { s };
            (or_unknown(status), or_unknown(time))
        };
        let (status, time) = read();
        let (r, label) = widgets::info_row("Status", &status);
        label.set_wrap(false);
        g.add(&r);
        let (r, time_label) = widgets::info_row("Time remaining", &time);
        time_label.set_wrap(false);
        g.add(&r);
        // Keep it current while the page exists.
        let (weak, weak_time) = (label.downgrade(), time_label.downgrade());
        gtk::glib::timeout_add_seconds_local(10, move || match (weak.upgrade(), weak_time.upgrade()) {
            (Some(l), Some(t)) => {
                if l.is_mapped() {
                    let (status, time) = read();
                    l.set_text(&status);
                    t.set_text(&time);
                }
                gtk::glib::ControlFlow::Continue
            }
            _ => gtk::glib::ControlFlow::Break,
        });
    }

    // ----- Brightness -----
    let g = page.group("Brightness");
    if let Some(b) = crate::platform::brightness() {
        let (r, _) = widgets::slider_row("Display", "The focused screen.", (1.0, 100.0, 1.0), b, 0, "%", |v| {
            crate::platform::set_brightness(v.round() as i64);
        });
        g.add(&r);
    }
    if cmd::present("brightnessctl") {
        let kbd = cmd::output(&["brightnessctl", "-l", "-c", "leds", "-m"]).unwrap_or_default();
        if let Some(line) = kbd.lines().find(|l| l.contains("kbd_backlight")) {
            let parts: Vec<&str> = line.split(',').collect();
            let dev = parts.first().unwrap_or(&"").to_string();
            let max: f64 = parts.get(4).and_then(|m| m.parse().ok()).unwrap_or(3.0);
            let cur: f64 = parts.get(2).and_then(|m| m.parse().ok()).unwrap_or(0.0);
            let (r, _) = widgets::slider_row("Keyboard backlight", "", (0.0, max, 1.0), cur, 0, "", move |v| {
                cmd::spawn(&["brightnessctl", "-d", &dev, "set", &(v.round() as i64).to_string()]);
            });
            widgets::keywords("keyboard light backlit");
            g.add(&r);
            if let Some(t) = timeout_row() {
                g.add(&t);
            }
        }
    }

    // ----- Lid & buttons -----
    let g = page.collapsible("Session", false);
    let now = widgets::hbox(8);
    now.append(&widgets::command_button("Lock", crate::platform::lock_command()));
    // Omarchy hides these when suspend is switched off or hibernation isn't set up.
    if cmd::run(&["omarchy-toggle-enabled", "suspend-off"]).is_err() {
        now.append(&widgets::command_button("Suspend", &["systemctl", "suspend"]));
    }
    if cmd::run(&["omarchy-hibernation-available"]).is_ok() {
        now.append(&widgets::command_button("Hibernate", &["systemctl", "hibernate"]));
    }
    g.add(&widgets::row("Lock or sleep", "", Some(now.upcast_ref())));
    let end = widgets::hbox(8);
    for (label, confirm, command) in [
        ("Log out", "Click again to log out", "omarchy-system-logout"),
        ("Restart", "Click again to restart", "omarchy-system-reboot"),
        ("Shut down", "Click again to shut down", "omarchy-system-shutdown"),
    ] {
        let b = widgets::confirm_button(label, confirm, move |_| cmd::spawn(&[command]));
        b.add_css_class("destructive-action");
        end.append(&b);
    }
    g.add(&widgets::row("End the session", "Open apps are closed. Each needs a second click.", Some(end.upcast_ref())));
    widgets::keywords("shut down shutdown power off restart reboot log out logout suspend sleep hibernate lock");

    buttons_group(page);
    graphics_group(page);
}

/// Integrated or dedicated graphics on laptops with both.
fn graphics_group(page: &Page) {
    if !cmd::present("supergfxctl") || cmd::run(&["omarchy-hw-hybrid-gpu"]).is_err() {
        return;
    }
    let g = page.collapsible("Graphics", false);
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
    let g = page.collapsible("Lid & buttons", false);
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
        "Turn off after",
        "Switch the backlight off after this long without typing or touching the trackpad. It comes back on the next key \
         press or touch.",
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
    widgets::keywords("keyboard backlight timeout idle off sleep dim inactivity");
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
