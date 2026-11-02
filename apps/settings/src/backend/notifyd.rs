//! Nyx Notification Center's service (`nyx-notifyd`), over its session-bus interface
//! `io.github.design_nexus.nyx.Notifications1` (JSON in and out). Called through
//! `busctl`, so nothing here links against the Notification Center.

use anyhow::{Context, Result, bail};
use serde_json::Value;

const DEST: [&str; 3] = [
    "io.github.design_nexus.nyx.Notifications",
    "/io/github/design_nexus/nyx/Notifications",
    "io.github.design_nexus.nyx.Notifications1",
];

/// Call a method; returns busctl's JSON reply (`{"type": …, "data": […]}`).
fn call(method: &str, sig_args: &[&str]) -> Result<Value> {
    let mut args = vec!["busctl", "--user", "--json=short", "call", DEST[0], DEST[1], DEST[2], method];
    args.extend_from_slice(sig_args);
    let out = crate::cmd::run(&args)?;
    serde_json::from_str(&out).context("unexpected reply from nyx-notifyd")
}

/// A method that answers with a JSON string, parsed.
fn json(method: &str) -> Result<Value> {
    let reply = call(method, &[])?;
    let text = reply["data"][0].as_str().context("unexpected reply from nyx-notifyd")?;
    serde_json::from_str(text).context("unexpected reply from nyx-notifyd")
}

/// "nyx" when Nyx receives notifications and shows the pop-ups, else "omarchy".
pub fn mode() -> Option<String> {
    call("Mode", &[]).ok()?["data"][0].as_str().map(String::from)
}

/// `{retention_days, schedule: {enabled, from, to}, excluded, no_popups, hide_popup_text}`
pub fn settings() -> Result<Value> {
    json("Settings")
}

pub fn set_settings(s: &Value) -> Result<()> {
    let text = s.to_string();
    let reply = call("SetSettings", &["s", &text])?;
    if reply["data"][0].is_string() { Ok(()) } else { bail!("nyx-notifyd didn't save the settings") }
}

/// `[{app, app_icon, count, excluded}]`
pub fn apps() -> Vec<Value> {
    json("Apps").ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

pub fn delete_history() -> Result<()> {
    call("DeleteHistory", &[]).map(|_| ())
}

/// "Omarchy" for Omarchy's own apps' passing messages, as the panel shows them.
pub fn app_name(app: &str) -> String {
    match app {
        "omarchy-action" => "Omarchy".into(),
        "notify-send" | "" => "Notification".into(),
        a => a.to_string(),
    }
}

/// Minutes after midnight as "HH:MM", and back.
pub fn hhmm(minutes: u64) -> String {
    format!("{:02}:{:02}", (minutes / 60) % 24, minutes % 60)
}

pub fn minutes(t: &str) -> Option<u64> {
    let (h, m) = t.trim().split_once(':')?;
    let (h, m): (u64, u64) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60 && t.trim().len() == 5).then_some(h * 60 + m)
}

#[cfg(test)]
mod tests {
    #[test]
    fn times() {
        assert_eq!(super::hhmm(22 * 60 + 5), "22:05");
        assert_eq!(super::minutes("07:30"), Some(450));
        assert_eq!(super::minutes("7:30"), None);
        assert_eq!(super::minutes("24:00"), None);
    }
}
