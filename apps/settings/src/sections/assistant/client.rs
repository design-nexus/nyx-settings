//! Talking to Nyx Assistant: everything goes through its `nyx-assistant` command (JSON
//! out), so the assistant keeps checking its own settings and Settings never edits its files.

use crate::cmd;
use anyhow::{Context, Result};
use serde_json::Value;

pub const PROGRAM: &str = "nyx-assistant";

pub fn installed() -> bool {
    cmd::present(PROGRAM)
}

/// Run `nyx-assistant ARGS…` and return its output.
pub fn run(args: &[&str]) -> Result<String> {
    let mut all = vec![PROGRAM];
    all.extend_from_slice(args);
    cmd::run(&all)
}

/// The last JSON object a command printed (some print a status line first).
pub fn parse(text: &str) -> Option<Value> {
    text.lines().rev().find_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
}

pub fn json(args: &[&str]) -> Option<Value> {
    run(args).ok().and_then(|t| parse(&t))
}

/// The assistant's settings as they are on disk.
pub fn config() -> Value {
    json(&["config", "get", "--json"]).unwrap_or(Value::Null)
}

/// A setting by dotted key, from a config read with [`config`].
pub fn get<'a>(cfg: &'a Value, key: &str) -> &'a Value {
    if let Some(tool) = key.strip_prefix("mcp.permissions.") {
        return &cfg["mcp"]["permissions"][tool];
    }
    key.split('.').fold(cfg, |v, k| &v[k])
}

pub fn get_str(cfg: &Value, key: &str) -> String {
    match get(cfg, key) {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

/// Change one setting. The assistant checks it, keeps the file's comments, and reloads
/// (or restarts, for the model and devices). The error says why a value was refused.
pub fn set(key: &str, value: &str) -> Result<()> {
    run(&["config", "set", key, value]).map(|_| ()).map_err(|e| {
        // `cmd::run` puts the whole command line first; only the reason matters.
        let text = e.to_string();
        anyhow::anyhow!("{}", text.rsplit("Error: ").next().unwrap_or(&text).trim())
    })
}

/// A TOML string for `config set` (bare words would do, but quotes keep spaces and symbols).
pub fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A TOML array of strings.
pub fn list(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|s| quoted(s)).collect::<Vec<_>>().join(", "))
}

/// The daemon's health (`status --json`): running, model loaded, pending approvals…
pub fn status() -> Option<Value> {
    run(&["status", "--json"])
        .ok()
        .and_then(|t| t.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find(|v| v.get("ok").is_some()))
}

pub fn running() -> bool {
    cmd::run(&["systemctl", "--user", "is-active", "--quiet", "nyx-assistantd.service"]).is_ok()
}

pub fn service(action: &str) -> Result<()> {
    cmd::run(&["systemctl", "--user", action, "nyx-assistantd.service"]).map(|_| ()).context("systemctl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_settings_by_key() {
        let cfg: Value = serde_json::from_str(
            r#"{"duplex":{"on_speech":"pause","duck_gain":0.25},"mcp":{"permissions":{"files.write":"deny"}}}"#,
        )
        .unwrap();
        assert_eq!(get_str(&cfg, "duplex.on_speech"), "pause");
        assert_eq!(get(&cfg, "duplex.duck_gain").as_f64(), Some(0.25));
        assert_eq!(get_str(&cfg, "mcp.permissions.files.write"), "deny");
        assert_eq!(get_str(&cfg, "nope.missing"), "");
        assert_eq!(parse("saved; restarted\n{\"a\":1}").unwrap()["a"], 1);
        assert_eq!(quoted(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(list(&["a".into(), "b c".into()]), r#"["a", "b c"]"#);
    }
}
