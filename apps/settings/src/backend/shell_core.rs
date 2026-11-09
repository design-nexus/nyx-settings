//! Nyx Shell Core's settings (the `shell` domain), through its socket
//! (`nyx-shell/docs/PROTOCOL.md`): read them merged, change or reset one key, and hear
//! when they change. Shell Core owns them; Settings never writes its files.
//!
//! Each request is its own short connection, so a Shell Core restart costs nothing; the
//! change subscription reconnects by itself and reads the settings again when it does.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const PROTOCOL: u64 = 1;

pub fn socket() -> std::path::PathBuf {
    nyx_capability::providers::ShellCore::socket()
}

/// Whether Shell Core is answering.
pub fn available() -> bool {
    crate::capability::state("daemon:shell-core").is_available()
}

fn connect() -> Result<UnixStream, String> {
    let s = UnixStream::connect(socket()).map_err(|_| "Shell Core isn't running".to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(3)));
    Ok(s)
}

fn request_on(stream: &mut UnixStream, reader: &mut BufReader<UnixStream>, op: &str, body: Value) -> Result<Value, String> {
    let line = json!({ "v": PROTOCOL, "id": 1, "op": op, "body": body }).to_string();
    stream.write_all(format!("{line}\n").as_bytes()).map_err(|e| format!("Shell Core: {e}"))?;
    let mut buf = String::new();
    loop {
        buf.clear();
        if reader.read_line(&mut buf).map_err(|e| format!("Shell Core: {e}"))? == 0 {
            return Err("Shell Core closed the connection".into());
        }
        let Ok(v) = serde_json::from_str::<Value>(&buf) else { continue };
        // Events can arrive before the reply.
        if v.get("ev").is_some() {
            continue;
        }
        if let Some(ok) = v.get("ok") {
            return Ok(ok.clone());
        }
        if let Some(err) = v.get("err") {
            let msg = err.get("msg").and_then(Value::as_str).unwrap_or("refused");
            return Err(format!("Shell Core: {msg}"));
        }
    }
}

fn request(op: &str, body: Value) -> Result<Value, String> {
    let mut stream = connect()?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    request_on(&mut stream, &mut reader, op, body)
}

/// The merged shell settings, and the problems Shell Core found in them.
pub fn config() -> Result<(Value, Vec<String>), String> {
    let r = request("GetShellConfig", json!({}))?;
    let errors = r
        .get("errors")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|e| e.as_str().map(String::from)).collect())
        .unwrap_or_default();
    Ok((r.get("config").cloned().unwrap_or(Value::Null), errors))
}

/// Change one setting (a dotted key, as in `focus.restore`).
pub fn set(key: &str, value: Value) -> Result<(), String> {
    request("UpdateShellConfig", json!({ "key": key, "value": value })).map(|_| ())
}

/// Hand one setting back to NyxOS's default.
pub fn reset(key: &str) -> Result<(), String> {
    request("ResetShellConfig", json!({ "key": key })).map(|_| ())
}

/// One screen's settings (`monitors.screens."<stable id>"`), if it has any.
pub fn screen(stable_id: &str) -> Option<Value> {
    if !available() {
        return None;
    }
    let (c, _) = config().ok()?;
    c.pointer("/monitors/screens").and_then(|s| s.get(stable_id)).cloned()
}

/// Every screen's settings with one changed. Screen ids hold `|`, which a dotted key
/// can't, so the whole `monitors` section is written.
fn write_screens(f: impl FnOnce(&mut serde_json::Map<String, Value>)) -> Result<(), String> {
    let (c, _) = config()?;
    let mut monitors = c.get("monitors").cloned().unwrap_or_else(|| json!({}));
    let screens =
        monitors.as_object_mut().ok_or("Shell Core's monitors aren't a table")?.entry("screens").or_insert_with(|| json!({}));
    let Some(map) = screens.as_object_mut() else { return Err("Shell Core's screens aren't a table".into()) };
    f(map);
    set("monitors", monitors)
}

pub fn set_screen(stable_id: &str, enabled: bool) -> Result<(), String> {
    let id = stable_id.to_string();
    write_screens(move |m| {
        let s = m.entry(id).or_insert_with(|| json!({ "enabled": true, "components": {} }));
        s["enabled"] = json!(enabled);
    })
}

pub fn forget_screen(stable_id: &str) -> Result<(), String> {
    if !available() {
        return Err("Shell Core isn't running, so its setting for this display stays".into());
    }
    let id = stable_id.to_string();
    write_screens(move |m| {
        m.remove(&id);
    })
}

thread_local! {
    static FOLLOWING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// While Shell Core is up, redraw the pages showing its settings when they change.
/// Called at start and whenever the capability registry sees Shell Core come or go
/// (its socket appearing), so after a restart the settings are read again.
pub fn follow() {
    if !available() || FOLLOWING.with(|f| f.replace(true)) {
        return;
    }
    let (tx, rx) = async_channel::unbounded::<Value>();
    std::thread::Builder::new()
        .name("shell-core-config".into())
        .spawn(move || {
            let Ok(mut stream) = connect() else { return };
            let _ = stream.set_read_timeout(None);
            let Ok(read) = stream.try_clone() else { return };
            let mut reader = BufReader::new(read);
            let Ok(first) = request_on(&mut stream, &mut reader, "SubscribeConfigChanges", json!({})) else { return };
            let _ = tx.send_blocking(first.get("config").cloned().unwrap_or(Value::Null));
            let mut buf = String::new();
            while reader.read_line(&mut buf).is_ok_and(|n| n > 0) {
                if let Ok(v) = serde_json::from_str::<Value>(&buf)
                    && v.get("ev").and_then(Value::as_str) == Some("ConfigChanged")
                    && tx.send_blocking(v.pointer("/body/config").cloned().unwrap_or(Value::Null)).is_err()
                {
                    return;
                }
                buf.clear();
            }
        })
        .ok();
    gtk::glib::spawn_future_local(async move {
        let mut first = true;
        while rx.recv().await.is_ok() {
            // The first answer is what the pages were built from.
            if !std::mem::take(&mut first) {
                for page in SHOWS_SHELL {
                    crate::window::rebuild_if_built(page);
                }
            }
        }
        // The connection ended: Shell Core stopped or restarted.
        FOLLOWING.with(|f| f.set(false));
        for page in SHOWS_SHELL {
            crate::window::rebuild_if_built(page);
        }
    });
}

/// Pages with Shell Core's settings on them.
const SHOWS_SHELL: [&str; 2] = ["displays", "workspaces"];
