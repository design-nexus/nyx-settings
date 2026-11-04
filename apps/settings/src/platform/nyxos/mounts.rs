//! Drives and shares mounted at startup on NyxOS: part of the system configuration,
//! through the System Manager. A share's login is kept apart (root only), never in it.

use crate::backend::mounts::{self, Mount};
use anyhow::{Result, bail};
use nyx_system_proto::{Change, Mount as SystemMount};
use std::io::Write;
use std::process::{Command, Stdio};

/// Run `nyx-system ARGS` (with `input` on stdin), off the window's thread.
fn nyx_system(args: &[&str], input: Option<&str>) -> Result<String> {
    let mut child = Command::new("nyx-system")
        .args(args)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(text.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it didn't work");
        bail!("{}", last.trim().trim_start_matches("nyx-system: "));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn options(text: &str) -> Vec<String> {
    if text == "-" || text.is_empty() { Vec::new() } else { text.split(',').map(str::to_string).collect() }
}

fn add(id: &str, mount: &SystemMount, login: &str) -> Result<String> {
    let json = serde_json::to_string(mount)?;
    if login.is_empty() {
        nyx_system(&["mount", "add", id, &json], None)
    } else {
        nyx_system(&["mount", "add", id, &json, "--login-stdin"], Some(login))
    }
}

pub fn add_drive(id: &str, uuid: &str, dir: &str, fs: &str, opts: &str) -> Result<String> {
    let mount = SystemMount { what: uuid.into(), dir: dir.into(), fs_type: fs.into(), options: options(opts), login: false };
    add(id, &mount, "")
}

/// `login` is the user, password and domain on three lines (empty for a guest).
pub fn add_network(id: &str, fstype: &str, source: &str, dir: &str, opts: &str, login: &str) -> Result<String> {
    let login = if login.trim().is_empty() {
        String::new()
    } else {
        let mut lines = login.lines();
        let (user, pass, domain) = (lines.next().unwrap_or(""), lines.next().unwrap_or(""), lines.next().unwrap_or(""));
        mounts::credentials_text(user, pass, domain).map_err(anyhow::Error::msg)?
    };
    let mount =
        SystemMount { what: source.into(), dir: dir.into(), fs_type: fstype.into(), options: options(opts), login: false };
    add(id, &mount, &login)
}

pub fn remove(id: &str) -> Result<String> {
    let json = serde_json::to_string(&[Change::RemoveMount { id: id.into() }])?;
    nyx_system(&["apply", &json], None)
}

/// fstab with the mounts the System Manager made named by their ids.
pub fn fstab() -> Vec<Mount> {
    let ours = super::settings().mounts;
    mounts::fstab()
        .into_iter()
        .map(|mut m| {
            if let Some((id, _)) = ours.iter().find(|(_, o)| o.dir == m.dir) {
                m.id = id.clone();
            }
            m
        })
        .collect()
}
