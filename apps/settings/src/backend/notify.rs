//! Omarchy's notification centre: do-not-disturb and the recent history. The
//! shell keeps each past notification as one JSON file under
//! `~/.local/state/omarchy/notifications/history`; Settings only reads them and
//! asks the shell (over its IPC) to change anything.

use crate::{cmd, paths};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub app: String,
    pub summary: String,
    pub body: String,
    /// Milliseconds since the epoch.
    pub timestamp: i64,
}

fn history_dir() -> PathBuf {
    paths::home().join(".local/state/omarchy/notifications/history")
}

pub fn parse_entry(json: &str) -> Option<Entry> {
    let v: Value = serde_json::from_str(json).ok()?;
    let text = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    let summary = text("summary");
    let body = text("body");
    if summary.is_empty() && body.is_empty() {
        return None;
    }
    Some(Entry { app: text("app"), summary, body, timestamp: v.get("timestamp").and_then(|x| x.as_i64()).unwrap_or(0) })
}

/// Newest first.
pub fn history() -> Vec<Entry> {
    let mut out: Vec<Entry> = std::fs::read_dir(history_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| parse_entry(&t))
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
    out
}

pub fn clear_history() -> anyhow::Result<String> {
    cmd::run(&["omarchy-shell", "notifications", "clear"])
}

pub fn dnd() -> bool {
    cmd::output(&["omarchy-shell", "notifications", "isDnd"]).map(|s| s.trim() == "on").unwrap_or_else(|| {
        std::fs::read_to_string(paths::home().join(".local/state/omarchy/notifications.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.get("dnd").and_then(|d| d.as_bool()))
            .unwrap_or(false)
    })
}

pub fn set_dnd(on: bool) -> anyhow::Result<()> {
    cmd::run(&["omarchy-shell", "notifications", "setDnd", if on { "on" } else { "off" }])?;
    // The bar's indicator shows the state too.
    let _ = cmd::run(&["omarchy-shell", "-q", "omarchy.indicators", "refresh"]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_history_entries() {
        let e = parse_entry(
            r#"{"id":6,"app":"Firefox","summary":"Download finished","body":"report.pdf","glyph":"","timestamp":1790711524161}"#,
        )
        .unwrap();
        assert_eq!(e.app, "Firefox");
        assert_eq!(e.summary, "Download finished");
        assert_eq!(e.body, "report.pdf");
        assert_eq!(e.timestamp, 1790711524161);
        assert!(parse_entry(r#"{"summary":"","body":""}"#).is_none());
        assert!(parse_entry("not json").is_none());
    }
}
