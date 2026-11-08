//! Small helpers: files, commands, the state folder, the Omarchy accent.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A file's text, trimmed; `None` if it can't be read.
pub fn read(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

pub fn read_num(path: impl AsRef<Path>) -> Option<i64> {
    read(path)?.parse().ok()
}

/// Whether a program is on PATH.
pub fn present(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

/// Run a program; its stdout, or its stderr as the error.
pub fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program).args(args).output().with_context(|| format!("couldn't start {program}"))?;
    if !out.status.success() {
        // Some programs (nvidia-smi) say what went wrong on stdout.
        let mut err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if err.is_empty() {
            err = String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
        bail!("{}", if err.is_empty() { format!("{program} failed") } else { err });
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Run a program as root through pkexec (it asks for a password).
pub fn run_root(program: &str, args: &[&str]) -> Result<String> {
    let mut all = vec![program];
    all.extend(args);
    run("pkexec", &all)
}

/// Where this extension may keep small things (Settings sets it).
pub fn state_dir() -> PathBuf {
    let dir = std::env::var_os("SETTINGS_EXTENSION_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("nyx-extension-state"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// The Omarchy theme's accent colour (`#rrggbb`), for "follow the theme". Omarchy
/// keeps the active theme under `$XDG_STATE_HOME`; `~/.config` is where it used to.
pub fn theme_accent() -> Option<String> {
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(".local/state"));
    let text = std::fs::read_to_string(state.join("omarchy/current/theme/colors.toml"))
        .or_else(|_| std::fs::read_to_string(home().join(".config/omarchy/current/theme/colors.toml")))
        .ok()?;
    for key in ["accent", "color4", "foreground"] {
        if let Some(v) = text.lines().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
        }) && v.starts_with('#')
            && v.len() == 7
        {
            return Some(v);
        }
    }
    None
}

/// `#rrggbb` → (r, g, b).
pub fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some((p(0)?, p(2)?, p(4)?))
}

pub fn hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours() {
        assert_eq!(parse_hex("#7aa2F7"), Some((0x7a, 0xa2, 0xf7)));
        assert_eq!(parse_hex("nope"), None);
        assert_eq!(hex(1, 2, 255), "#0102ff");
    }
}
