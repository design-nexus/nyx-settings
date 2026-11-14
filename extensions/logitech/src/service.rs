//! `settings-logitech-watch.service`: re-applies saved settings when devices
//! connect, for people using Logitech devices through Settings rather than Logi.

use anyhow::{Result, bail};
use std::process::Command;

pub const UNIT: &str = "settings-logitech-watch.service";

fn unit_file() -> std::path::PathBuf {
    crate::paths::config_home().join("systemd/user").join(UNIT)
}

fn systemctl(args: &[&str]) -> Result<String> {
    let out = Command::new("systemctl").arg("--user").args(args).output()?;
    if !out.status.success() {
        bail!("systemctl {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn unit_text(exe: &str) -> String {
    format!(
        "[Unit]\nDescription=Re-apply Logitech device settings when devices connect\nAfter=graphical-session.target\n\n\
         [Service]\nExecStart={exe} watch\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=graphical-session.target\n"
    )
}

/// Pretend mode's stand-in for the unit being enabled.
fn mock_flag() -> Option<std::path::PathBuf> {
    crate::devices::mock_state().map(|d| d.join("keep-applied"))
}

pub fn enabled() -> bool {
    if let Some(flag) = mock_flag() {
        return flag.exists();
    }
    systemctl(&["is-enabled", UNIT]).is_ok_and(|s| s == "enabled")
}

/// Logi's own service does the same job.
pub fn logi_watch_enabled() -> bool {
    mock_flag().is_none() && systemctl(&["is-enabled", "logi-watch.service"]).is_ok_and(|s| s == "enabled")
}

pub fn set(on: bool) -> Result<()> {
    if let Some(flag) = mock_flag() {
        if on {
            crate::files::atomic_write(&flag, "")?;
        } else if flag.exists() {
            std::fs::remove_file(&flag)?;
        }
        return Ok(());
    }
    if on {
        let exe = std::env::current_exe()?.display().to_string();
        crate::files::atomic_write(&unit_file(), &unit_text(&exe))?;
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", "--now", UNIT])?;
    } else {
        systemctl(&["disable", "--now", UNIT])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn unit_runs_watch() {
        let t = super::unit_text("/x/settings-logitech");
        assert!(t.contains("ExecStart=/x/settings-logitech watch\n"));
    }
}
