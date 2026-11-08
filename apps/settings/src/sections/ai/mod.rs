//! Nyx AI Core, the shared local model runtime: its status, how it runs, default
//! models, and the models themselves. Everything goes through the `nyx-ai` command
//! (JSON out), so AI Core checks every change and Settings never edits its files.

pub mod models;
pub mod runtime;

use crate::{cmd, window};
use serde_json::Value;

pub const PROGRAM: &str = "nyx-ai";

pub fn installed() -> bool {
    cmd::present(PROGRAM)
}

/// Run `nyx-ai ARGS… --json`. The error is AI Core's own message.
pub fn call(args: &[&str]) -> Result<Value, String> {
    let out = std::process::Command::new(PROGRAM)
        .args(args)
        .arg("--json")
        .output()
        .map_err(|e| format!("couldn't run {PROGRAM}: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    match serde_json::from_str::<Value>(text.trim()) {
        Ok(v) if v["error"].is_object() => Err(v["error"]["message"].as_str().unwrap_or("Nyx AI Core refused").to_string()),
        Ok(v) if out.status.success() => Ok(v),
        _ if out.status.success() => Ok(Value::Null),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("nyx-ai: ").to_string()),
    }
}

/// Run a change off the main thread, then redraw `page` (or say why it failed).
pub fn change(args: Vec<String>, page: &'static str, done: &'static str) {
    cmd::background(
        move || {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            call(&refs)
        },
        move |r| {
            match r {
                Ok(_) if !done.is_empty() => window::toast(done),
                Ok(_) => {}
                Err(e) => window::toast(&format!("Couldn't change Nyx AI Core: {e}")),
            }
            window::rebuild(page);
        },
    );
}

/// "5.3 GB", "640 MB".
pub fn size(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", bytes as f64 / 1e6) }
}

/// The capabilities AI Core lists, in words.
pub fn capabilities(m: &Value) -> Vec<&'static str> {
    m["capabilities"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| match c.as_str()? {
            "chat" => Some("Chat"),
            "tools" => Some("Tools"),
            "thinking" => Some("Thinking"),
            "vision" => Some("Vision"),
            "audio" => Some("Audio"),
            "embedding" => Some("Embedding"),
            "draft" => Some("Drafter"),
            _ => None,
        })
        .collect()
}

pub fn has(m: &Value, capability: &str) -> bool {
    m["capabilities"].as_array().is_some_and(|c| c.iter().any(|v| v.as_str() == Some(capability)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        assert_eq!(size(5_300_000_000), "5.3 GB");
        assert_eq!(size(640_000_000), "640 MB");
        let m: Value = serde_json::from_str(r#"{"capabilities":["chat","vision","nope"]}"#).unwrap();
        assert_eq!(capabilities(&m), ["Chat", "Vision"]);
        assert!(has(&m, "vision") && !has(&m, "embedding"));
    }
}
