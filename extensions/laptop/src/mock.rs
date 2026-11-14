//! `LAPTOP_MOCK=framework-13`: a pretend Framework Laptop 13, for trying the page
//! without one. Its sysfs (DMI, battery and charge limits, keyboard light, camera
//! module) is a scratch tree in the extension's state folder, made the first time,
//! and the commands the page runs (powerprofilesctl, wpctl, busctl, modprobe) change
//! files there instead. Nothing real is read or changed while it's on.

use anyhow::{Context, Result, bail};
use nyx_ext::sys;
use std::path::{Path, PathBuf};

pub fn on() -> bool {
    std::env::var("LAPTOP_MOCK").is_ok_and(|v| !v.trim().is_empty())
}

const LED: &str = "sys/class/leds/framework_laptop::kbd_backlight";
const MODULES: &str = "proc/modules";
const UVC: &str = "uvcvideo 151552 0 - Live 0x0000000000000000\n";

const FILES: &[(&str, &str)] = &[
    ("sys/class/dmi/id/sys_vendor", "Framework"),
    ("sys/class/dmi/id/product_name", "Framework Laptop 13"),
    ("sys/class/dmi/id/chassis_type", "10"),
    ("sys/class/power_supply/BAT1/type", "Battery"),
    ("sys/class/power_supply/BAT1/capacity", "78"),
    ("sys/class/power_supply/BAT1/status", "Charging"),
    ("sys/class/power_supply/BAT1/energy_full", "56200000"),
    ("sys/class/power_supply/BAT1/energy_full_design", "61000000"),
    ("sys/class/power_supply/BAT1/cycle_count", "214"),
    ("sys/class/power_supply/BAT1/charge_control_start_threshold", "75"),
    ("sys/class/power_supply/BAT1/charge_control_end_threshold", "80"),
    ("sys/class/power_supply/ACAD/type", "Mains"),
    ("sys/class/power_supply/ACAD/online", "1"),
    ("sys/class/leds/framework_laptop::kbd_backlight/brightness", "2"),
    ("sys/class/leds/framework_laptop::kbd_backlight/max_brightness", "3"),
    ("sys/bus/usb/devices/3-1:1.0/bInterfaceClass", "0e"),
    ("sys/module/uvcvideo/refcnt", "0"),
    (MODULES, UVC),
    ("profile", "balanced"),
    ("mic-muted", "false"),
];

const PROFILES: &[&str] = &["performance", "balanced", "power-saver"];

/// The pretend machine's root.
pub fn root() -> PathBuf {
    let root = sys::state_dir().join("laptop-mock");
    if !root.exists() {
        for (file, text) in FILES {
            let path = root.join(file);
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, text);
        }
    }
    root
}

/// A path on the pretend machine, for an absolute one on a real one.
pub fn path(real: &str) -> PathBuf {
    root().join(real.trim_start_matches('/'))
}

fn write(rel: impl AsRef<Path>, text: &str) -> Result<()> {
    std::fs::write(root().join(rel), text).context("couldn't write the pretend laptop's state")
}

/// `powerprofilesctl list`, as the real one prints it.
fn profile_list(current: &str) -> String {
    PROFILES
        .iter()
        .map(|p| format!("{} {p}:\n    CpuDriver:\tamd_pstate\n    PlatformDriver:\tplatform_profile\n", if *p == current { "*" } else { " " }))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the page's commands do to the pretend machine.
pub fn run(program: &str, args: &[&str]) -> Result<String> {
    let read = |rel: &str| sys::read(root().join(rel)).unwrap_or_default();
    match (program, args) {
        ("powerprofilesctl", ["list"]) => Ok(profile_list(&read("profile"))),
        ("powerprofilesctl", ["set", p]) if PROFILES.contains(p) => write("profile", p).map(|_| String::new()),
        ("wpctl", ["get-volume", _]) => {
            Ok(format!("Volume: 0.60{}\n", if read("mic-muted") == "true" { " [MUTED]" } else { "" }))
        }
        ("wpctl", ["set-mute", _, m]) => write("mic-muted", if *m == "1" { "true" } else { "false" }).map(|_| String::new()),
        ("busctl", [.., "SetBrightness", "ssu", "leds", _, v]) => {
            let max = read(&format!("{LED}/max_brightness")).parse::<u32>().unwrap_or(0);
            let v = v.parse::<u32>()?.min(max);
            write(format!("{LED}/brightness"), &v.to_string()).map(|_| String::new())
        }
        ("modprobe", ["uvcvideo"]) => {
            std::fs::create_dir_all(root().join("sys/module/uvcvideo"))?;
            write("sys/module/uvcvideo/refcnt", "0")?;
            write(MODULES, UVC).map(|_| String::new())
        }
        ("modprobe", ["-r", "uvcvideo"]) => {
            let _ = std::fs::remove_dir_all(root().join("sys/module/uvcvideo"));
            write(MODULES, "").map(|_| String::new())
        }
        _ => bail!("the pretend laptop can't run {program} {}", args.join(" ")),
    }
}

/// Set the charge limits on the pretend battery, as the kernel would.
pub fn set_limits(bat: &Path, start: Option<i64>, end: Option<i64>) -> Result<()> {
    for (file, v) in [("charge_control_start_threshold", start), ("charge_control_end_threshold", end)] {
        if let Some(v) = v {
            std::fs::write(bat.join(file), v.to_string())?;
        }
    }
    Ok(())
}
