//! Users and groups. Reading needs no privileges; changes go through
//! `nyx-settings-helper`, which runs as root via pkexec and only does a fixed set of
//! account operations (see `src/bin/nyx-settings-helper.rs`).

use super::helper_version::HELPER_VERSION;
use super::passwd::{self, Group, User};
use crate::cmd;
use anyhow::{Context, Result, bail};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const HELPER: &str = "/usr/local/lib/nyx-settings/nyx-settings-helper";
const POLICY: &str = "/usr/share/polkit-1/actions/io.github.design_nexus.nyx.settings.policy";
const POLICY_XML: &str = include_str!("../../data/io.github.design_nexus.nyx.settings.policy");

/// The system profile, where NixOS installs the helper and its policy.
const PROFILE: &str = "/run/current-system/sw";

/// `usual` (under /usr/local or /usr), or the same file in the system profile on NixOS.
fn system_file(usual: &str) -> PathBuf {
    let rest = usual.strip_prefix("/usr/local/").or_else(|| usual.strip_prefix("/usr/")).unwrap_or(usual);
    let packaged = Path::new(PROFILE).join(rest);
    if !Path::new(usual).exists() && packaged.exists() { packaged } else { PathBuf::from(usual) }
}

/// Where the helper is installed.
pub fn helper_path() -> PathBuf {
    system_file(HELPER)
}

/// Installed with the system rather than by `--install-helper`: the system keeps it and
/// its policy current.
fn system_installed() -> bool {
    helper_path().starts_with(PROFILE)
}

pub fn helper_installed() -> bool {
    helper_path().is_file() && system_file(POLICY).is_file() && cmd::present("pkexec")
}

#[derive(PartialEq)]
pub enum HelperState {
    Missing,
    /// Installed, but older than the one this version of Settings carries.
    Outdated,
    Ready,
}

pub fn helper_state() -> HelperState {
    if !helper_installed() {
        return HelperState::Missing;
    }
    // Its permission policy changes too (how long a password is remembered).
    if !system_installed() && std::fs::read_to_string(POLICY).ok().as_deref() != Some(POLICY_XML) {
        return HelperState::Outdated;
    }
    if installed_version().is_none_or(|v| v < HELPER_VERSION) {
        return HelperState::Outdated;
    }
    HelperState::Ready
}

/// The installed helper's version; None for one from before versions (it can't say).
fn installed_version() -> Option<u32> {
    let out = Command::new(helper_path()).arg("--version").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Run the helper through pkexec. Blocking: call it from `cmd::background`.
pub fn helper(args: &[&str], stdin: Option<&str>) -> Result<String> {
    let mut child = Command::new("pkexec")
        .arg(helper_path())
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not start pkexec")?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let _ = pipe.write_all(text.as_bytes());
    }
    // Reads stdout and stderr together, so a chatty child can't fill one pipe and stall.
    let output = child.wait_with_output()?;
    let out = String::from_utf8_lossy(&output.stdout).to_string();
    let err = String::from_utf8_lossy(&output.stderr).to_string();
    match output.status.code() {
        Some(0) => Ok(out),
        // pkexec: 126 is a dismissed prompt, 127 a failed authentication.
        Some(126) | Some(127) => bail!("Canceled"),
        _ => bail!("{}", if err.trim().is_empty() { "The change failed" } else { err.trim() }),
    }
}

/// Copy the helper next to this binary into a root-owned place and register the
/// polkit action. Run as root (`sudo nyx-settings --install-helper`); a user-writable
/// helper would let any program of yours become root.
pub fn install_helper() -> Result<()> {
    if system_installed() {
        bail!("the helper comes with the system");
    }
    if !nix_is_root() {
        bail!("run this as root: sudo nyx-settings --install-helper");
    }
    let exe = std::env::current_exe()?.canonicalize()?;
    let source = exe.with_file_name("nyx-settings-helper");
    if !source.is_file() {
        bail!("{} not found next to the nyx-settings binary", source.display());
    }
    let dir = Path::new(HELPER).parent().context("no helper directory")?;
    std::fs::create_dir_all(dir)?;
    // The directory must be root's too, or its owner could swap the helper out later.
    let status = Command::new("chown").args(["root:root"]).arg(dir).status()?;
    if !status.success() {
        bail!("could not take ownership of {}", dir.display());
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))?;
    let status = Command::new("install").args(["-m", "755", "-o", "root", "-g", "root"]).arg(&source).arg(HELPER).status()?;
    if !status.success() {
        bail!("could not install the helper");
    }
    std::fs::write(POLICY, POLICY_XML)?;
    Ok(())
}

fn nix_is_root() -> bool {
    cmd::output(&["id", "-u"]).is_some_and(|u| u == "0")
}

pub fn me() -> String {
    std::env::var("USER").ok().or_else(|| cmd::output(&["id", "-un"])).unwrap_or_default()
}

pub fn people() -> Vec<User> {
    passwd::users().into_iter().filter(User::is_human).collect()
}

/// Groups a user is in, including their main group, sorted.
pub fn groups_of(user: &User, all: &[Group]) -> Vec<String> {
    let mut names: Vec<String> =
        all.iter().filter(|g| g.gid == user.gid || g.members.contains(&user.name)).map(|g| g.name.clone()).collect();
    names.sort();
    names
}

pub fn is_admin(user: &User, all: &[Group]) -> bool {
    all.iter().any(|g| g.name == "wheel" && g.members.contains(&user.name))
}

/// Locked accounts have a `!` in front of the password hash (`passwd -S` says `L`).
/// `None` when it can't be read: only root may ask about other users.
pub fn is_locked(name: &str) -> Option<bool> {
    cmd::output(&["passwd", "-S", name]).and_then(|s| s.split_whitespace().nth(1).map(|f| f == "L"))
}

/// Public keys in `~/.ssh/authorized_keys`: (type, comment, full line).
pub fn parse_keys(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            // Options may come first; the type is the field starting with "ssh-", "ecdsa-" or "sk-".
            let kind = f.by_ref().find(|w| w.starts_with("ssh-") || w.starts_with("ecdsa-") || w.starts_with("sk-"))?;
            let _blob = f.next()?;
            let comment: Vec<&str> = f.collect();
            Some((kind.to_string(), comment.join(" "), l.to_string()))
        })
        .collect()
}

pub fn sshd_active() -> bool {
    cmd::output(&["systemctl", "is-active", "sshd.service"]).is_some_and(|s| s == "active")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        let text = "# keys\nssh-ed25519 AAAAC3 ken@laptop\n\ncommand=\"x\",no-pty ssh-rsa BBBB work key\nbogus\n";
        let k = parse_keys(text);
        assert_eq!(k.len(), 2);
        assert_eq!(k[0].0, "ssh-ed25519");
        assert_eq!(k[0].1, "ken@laptop");
        assert_eq!(k[1].0, "ssh-rsa");
        assert_eq!(k[1].1, "work key");
    }

    #[test]
    fn membership() {
        let users = passwd::parse_passwd("ken:x:1000:1000::/home/ken:/bin/bash\n");
        let groups = passwd::parse_group("wheel:x:998:ken\nken:x:1000:\ndocker:x:970:sam\n");
        assert_eq!(groups_of(&users[0], &groups), ["ken", "wheel"]);
        assert!(is_admin(&users[0], &groups));
    }
}
