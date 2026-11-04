//! NyxOS's power pieces for Settings' Power page: the battery from UPower, the screen's
//! brightness with brightnessctl, and the profile for each power source (nyx-power-profiles,
//! a user service, switches to it when the computer is plugged in or unplugged).

use crate::cmd;

/// Where the profile for `ac` and for `battery` is kept.
pub fn profile_state_dir() -> std::path::PathBuf {
    std::env::var_os("NYX_POWER_PROFILES_STATE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| crate::paths::state_home().join("nyx/power-profiles"))
}

/// A system battery (not a mouse's or a headset's).
pub fn battery_present() -> bool {
    std::fs::read_dir("/sys/class/power_supply").is_ok_and(|d| {
        d.flatten().any(|s| {
            let read = |f: &str| std::fs::read_to_string(s.path().join(f)).unwrap_or_default().trim().to_string();
            read("type") == "Battery" && read("scope") != "Device"
        })
    })
}

/// `upower -i` for the battery, as the tab-separated `key\tvalue` lines the Power page
/// reads (state, time, percentage, rate, size).
pub fn battery_shell() -> String {
    let devices = cmd::output(&["upower", "--enumerate"]).unwrap_or_default();
    let Some(bat) = devices.lines().find(|l| l.contains("battery_BAT")) else { return String::new() };
    shell_from_upower(&cmd::output(&["upower", "--show-info", bat.trim()]).unwrap_or_default())
}

pub fn shell_from_upower(info: &str) -> String {
    let get = |k: &str| {
        info.lines()
            .find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim_start_matches(':').trim().to_string()))
            .unwrap_or_default()
    };
    let tight = |v: String| v.replace(' ', "");
    let time = [get("time to empty"), get("time to full")].into_iter().find(|t| !t.is_empty()).unwrap_or_default();
    [
        ("state", get("state")),
        ("time", time),
        ("percentage", get("percentage")),
        ("rate", tight(get("energy-rate"))),
        ("size", tight(get("energy-full"))),
    ]
    .iter()
    .map(|(k, v)| format!("{k}\t{v}"))
    .collect::<Vec<_>>()
    .join("\n")
}

/// The screen's brightness in percent, when it has a backlight.
pub fn brightness() -> Option<f64> {
    let out = cmd::output(&["brightnessctl", "--class=backlight", "--machine-readable"])?;
    out.lines().next()?.split(',').nth(3)?.trim_end_matches('%').parse().ok()
}

pub fn set_brightness(percent: i64) {
    cmd::spawn(&["brightnessctl", "--class=backlight", "set", &format!("{percent}%")]);
}

#[cfg(test)]
mod tests {
    #[test]
    fn upower_reads_as_the_power_pages_lines() {
        let info = "  native-path:          BAT0\n  battery\n    present:             yes\n    state:               discharging\n    energy-full:         52.3 Wh\n    energy-rate:         9.8 W\n    time to empty:       4.2 hours\n    percentage:          71%\n";
        let s = super::shell_from_upower(info);
        assert!(s.contains("state\tdischarging"));
        assert!(s.contains("time\t4.2 hours"));
        assert!(s.contains("percentage\t71%"));
        assert!(s.contains("rate\t9.8W"));
        assert!(s.contains("size\t52.3Wh"));
    }
}
