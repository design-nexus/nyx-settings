use crate::backend::store;
use crate::widgets::{self, Page};
use crate::{cmd, paths, window};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
struct Schedule {
    on: bool,
    evening: String,
    morning: String,
    temperature: i64,
}

fn conf_path() -> std::path::PathBuf {
    paths::hypr_dir().join("hyprsunset.conf")
}

const MARK: &str = "# Managed by Settings";

fn read_schedule() -> Schedule {
    let text = std::fs::read_to_string(conf_path()).unwrap_or_default();
    let managed = text.starts_with(MARK);
    let mut times = Vec::new();
    let mut temperature = 4000;
    for line in text.lines().map(str::trim).filter(|l| !l.starts_with('#')) {
        if let Some(t) = line.strip_prefix("time =") {
            times.push(t.trim().to_string());
        }
        if let Some(t) = line.strip_prefix("temperature =") {
            temperature = t.trim().parse().unwrap_or(4000);
        }
    }
    Schedule {
        on: managed && store::read(|s| s.autostart.contains_key("hyprsunset")),
        morning: times.first().cloned().filter(|_| managed).unwrap_or("07:00".into()),
        evening: times.get(1).cloned().filter(|_| managed).unwrap_or("20:00".into()),
        temperature,
    }
}

fn write_schedule(s: &Schedule) -> anyhow::Result<()> {
    let path = conf_path();
    if let Ok(old) = std::fs::read_to_string(&path)
        && !old.starts_with(MARK)
    {
        std::fs::write(path.with_extension("conf.before-settings"), old)?;
    }
    let text = if s.on {
        format!(
            "{MARK}. Change it in Settings → Night Light.\n\nprofile {{\n    time = {}\n    identity = true\n}}\n\nprofile {{\n    time = {}\n    temperature = {}\n}}\n",
            s.morning, s.evening, s.temperature
        )
    } else {
        format!("{MARK}. Change it in Settings → Night Light.\n\nprofile {{\n    time = 07:00\n    identity = true\n}}\n")
    };
    cmd::atomic_write(&path, &text)?;
    store::update(true, |st| {
        if s.on {
            st.autostart.insert("hyprsunset".into(), "uwsm-app -- hyprsunset".into());
        } else {
            st.autostart.remove("hyprsunset");
        }
    });
    // Restart so the new schedule is read.
    if cmd::present("omarchy-restart-hyprsunset") {
        cmd::spawn(&["omarchy-restart-hyprsunset"]);
    }
    Ok(())
}

fn valid_time(t: &str) -> bool {
    let parts: Vec<&str> = t.split(':').collect();
    parts.len() == 2
        && parts[0].parse::<u32>().is_ok_and(|h| h < 24)
        && parts[1].len() == 2
        && parts[1].parse::<u32>().is_ok_and(|m| m < 60)
}

/// Omarchy's toggle where it's there; elsewhere hyprsunset itself.
fn omarchy_toggle() -> bool {
    crate::capability::program("omarchy-toggle-nightlight")
}

fn on_now() -> bool {
    if omarchy_toggle() {
        let status =
            cmd::output(&["omarchy-toggle-nightlight", "--status"]).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
        return status.as_ref().and_then(|v| v.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(false);
    }
    // hyprsunset answers with the temperature it's showing; 6000 K and over is neutral.
    cmd::output(&["hyprctl", "hyprsunset", "temperature"]).and_then(|t| t.trim().parse::<i64>().ok()).is_some_and(|k| k < 6000)
}

fn set_now(on: bool, temperature: i64) {
    if omarchy_toggle() {
        cmd::spawn(&["omarchy-toggle-nightlight"]);
    } else if !on {
        cmd::spawn(&["hyprctl", "hyprsunset", "identity"]);
    } else if cmd::run(&["pgrep", "-x", "hyprsunset"]).is_ok() {
        cmd::spawn(&["hyprctl", "hyprsunset", "temperature", &temperature.to_string()]);
    } else {
        cmd::spawn(&["uwsm", "app", "--", "hyprsunset", "-t", &temperature.to_string()]);
    }
}

pub fn build(page: &Page) {
    let g = page.group("Now");
    let what = if omarchy_toggle() {
        "Warm the screen colors right away. Also in the Omarchy menu."
    } else {
        "Warm the screen colors right away."
    };
    let warmth = read_schedule().temperature;
    let (r, _) = widgets::switch_row("Night light", what, on_now(), move |on| set_now(on, warmth));
    g.add(&r);

    let sched = Rc::new(RefCell::new(read_schedule()));
    let temp = sched.borrow().temperature as f64;
    let s2 = sched.clone();
    let pending: Rc<std::cell::Cell<Option<gtk::glib::SourceId>>> = Rc::default();
    let (r, _) =
        widgets::slider_row("Warmth", "Lower is warmer. 6500 K is neutral.", (2500.0, 6000.0, 100.0), temp, 0, " K", move |v| {
            let k = v.round() as i64;
            s2.borrow_mut().temperature = k;
            // Preview immediately when hyprsunset is running.
            cmd::spawn(&["hyprctl", "hyprsunset", "temperature", &k.to_string()]);
            // Save the schedule once the slider settles (it restarts hyprsunset).
            if let Some(id) = pending.take() {
                id.remove();
            }
            let s3 = s2.clone();
            let p2 = pending.clone();
            pending.set(Some(gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
                p2.set(None);
                if s3.borrow().on {
                    let _ = write_schedule(&s3.borrow());
                }
            })));
        });
    widgets::keywords("temperature kelvin warm");
    g.add(&r);

    let g = page.group("Schedule");
    let times = widgets::vbox(8);
    times.set_sensitive(sched.borrow().on);
    let t2 = times.clone();
    let s3 = sched.clone();
    let (r, _) = widgets::switch_row(
        "Turn on automatically",
        "Warm the screen every evening and return to normal in the morning.",
        sched.borrow().on,
        move |on| {
            s3.borrow_mut().on = on;
            t2.set_sensitive(on);
            if let Err(e) = write_schedule(&s3.borrow()) {
                window::toast(&format!("Couldn't save the schedule: {e}"));
            }
        },
    );
    g.add(&r);

    for (label, evening) in [("Turns on at", true), ("Turns off at", false)] {
        let current = if evening { sched.borrow().evening.clone() } else { sched.borrow().morning.clone() };
        let s = sched.clone();
        let (r, e) = widgets::entry_row(label, "24-hour time, like 20:30.", &current, "HH:MM", move |t| {
            let t = t.trim().to_string();
            if !valid_time(&t) {
                window::toast("Use a 24-hour time like 20:30");
                return;
            }
            if evening {
                s.borrow_mut().evening = t;
            } else {
                s.borrow_mut().morning = t;
            }
            if let Err(e) = write_schedule(&s.borrow()) {
                window::toast(&format!("Couldn't save the schedule: {e}"));
            }
        });
        e.set_width_chars(8);
        e.add_css_class("mono");
        times.append(&r);
    }
    g.add(&times);
}
