//! A pretend controller for testing pages without one: `CONTROLLERS_MOCK=dualsense`
//! (or `xbox`). Its lightbar and player lights are kept in the state folder; nothing
//! touches real devices while it's on.

use crate::Snapshot;
use crate::input::State;
use anyhow::{Result, bail};
use nyx_ext::sys;
use serde_json::{Value, json};

pub fn on() -> bool {
    std::env::var("CONTROLLERS_MOCK").is_ok_and(|v| !v.is_empty())
}

fn kind() -> String {
    std::env::var("CONTROLLERS_MOCK").unwrap_or_default()
}

fn state_file() -> std::path::PathBuf {
    sys::state_dir().join("controllers-mock.json")
}

fn saved() -> Value {
    std::fs::read_to_string(state_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

pub fn snapshots() -> Vec<Snapshot> {
    let s = saved();
    let held = vec!["A / ✕".to_string(), "RB / R1".to_string()];
    let state = State { buttons: held, left: Some((-0.40, 0.12)), right: Some((0.0, 0.0)), lt: Some(0.0), rt: Some(0.72), dpad: Some((0.0, 0.0)) };
    if kind() == "xbox" {
        return vec![Snapshot {
            id: "045e0b13-mock".into(),
            title: "Xbox Wireless Controller".into(),
            battery: Some((80, "Discharging".into())),
            bluetooth: true,
            ids: (0x045e, 0x0b13),
            rumble: true,
            state: Ok(state),
            lightbar: None,
            lightbar_writable: false,
            players: 0,
            players_lit: 0,
            players_writable: false,
        }];
    }
    vec![Snapshot {
        id: "054c0ce6-mock".into(),
        title: "DualSense".into(),
        battery: Some((60, "Discharging".into())),
        bluetooth: true,
        ids: (0x054c, 0x0ce6),
        rumble: true,
        state: Ok(state),
        lightbar: Some(s["lightbar"].as_str().map(String::from).or_else(sys::theme_accent).unwrap_or_else(|| "#3d6bff".into())),
        lightbar_writable: true,
        players: 4,
        players_lit: s["players"].as_u64().unwrap_or(1) as usize,
        players_writable: true,
    }]
}

/// A change to the pretend controller: only its saved state changes.
pub fn set(key: &str, value: &str) -> Result<Option<Value>> {
    let mut s = saved();
    if !s.is_object() {
        s = json!({});
    }
    match key {
        "access" => return Ok(nyx_ext::refresh()),
        "rumble" => return Ok(None),
        "lightbar" => {
            sys::parse_hex(value).ok_or_else(|| anyhow::anyhow!("not a colour"))?;
            s["lightbar"] = json!(value);
        }
        "players" => s["players"] = json!(value.parse::<usize>()?),
        _ => bail!("unknown setting {key}"),
    }
    std::fs::write(state_file(), s.to_string())?;
    Ok(None)
}
