//! `NVIDIA_MOCK=rtx-5080-laptop`: a pretend GeForce RTX 5080 Laptop GPU, for trying
//! the page without touching a real one. It answers the nvidia-smi commands the page
//! runs; the power limit and persistence mode it's given are kept in the extension's
//! state folder.

use anyhow::{Context, Result, bail};
use nyx_ext::sys;
use std::path::PathBuf;

pub fn on() -> bool {
    std::env::var("NVIDIA_MOCK").is_ok_and(|v| !v.trim().is_empty())
}

const BUS: &str = "00000000:01:00.0";
const LIMITS: (f64, f64, f64) = (80.0, 150.0, 175.0);
const DRAW: f64 = 84.0;
const APPS: &str = "1820, /usr/bin/Hyprland, 212\n4410, /usr/lib/nyx/nyx-ai-core, 8840\n5127, /usr/lib/firefox/firefox, 164\n";

fn dir() -> PathBuf {
    let d = sys::state_dir().join("nvidia-mock");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn limit() -> f64 {
    sys::read(dir().join("limit")).and_then(|s| s.parse().ok()).unwrap_or(LIMITS.1)
}

fn persistence() -> bool {
    sys::read(dir().join("persistence")).as_deref() == Some("1")
}

/// One line of `--query-gpu`, in the order the page asks for.
fn gpu_line() -> String {
    let (lo, _, hi) = LIMITS;
    let lim = limit();
    let pm = if persistence() { "Enabled" } else { "Disabled" };
    format!(
        "0, NVIDIA GeForce RTX 5080 Laptop GPU, 580.95, 61, 38, 9421, 16384, {:.2}, {lim:.2}, {lo:.2}, {hi:.2}, 1980, 7001, 34, {pm}, P2, {BUS}\n",
        DRAW.min(lim)
    )
}

/// What nvidia-smi would print for these arguments.
pub fn smi(args: &[&str]) -> Result<String> {
    match args {
        [q, "--format=csv,noheader,nounits"] if q.starts_with("--query-gpu=") => Ok(gpu_line()),
        [q, "--format=csv,noheader,nounits"] if q.starts_with("--query-compute-apps=") => Ok(APPS.to_string()),
        ["-i", "0", "-pl", w] => {
            let w: f64 = w.parse().context("the power limit is a number")?;
            if !(LIMITS.0..=LIMITS.2).contains(&w) {
                bail!("Provided power limit {w:.2} W is not a valid power limit which should be between {:.2} W and {:.2} W for GPU {BUS}", LIMITS.0, LIMITS.2);
            }
            let old = limit();
            std::fs::write(dir().join("limit"), w.to_string())?;
            Ok(format!("Power limit for GPU {BUS} was set to {w:.2} W from {old:.2} W.\n"))
        }
        ["-i", "0", "-pm", m @ ("0" | "1")] => {
            std::fs::write(dir().join("persistence"), m)?;
            Ok(format!("{} persistence mode for GPU {BUS}.\n", if *m == "1" { "Enabled" } else { "Disabled" }))
        }
        ["-i", i, ..] => bail!("No devices were found (the pretend machine has GPU 0, not {i})"),
        _ => bail!("the pretend GPU can't answer nvidia-smi {}", args.join(" ")),
    }
}
