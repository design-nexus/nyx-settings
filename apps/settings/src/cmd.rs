//! Running external commands without blocking the UI thread.

pub use nyx_core::cmd::*;

// nyx-core's run, output, present and spawn, with Omarchy's scripts this computer lacks
// (NyxOS) done by `nyx-platform` instead: `omarchy-network-band …` runs as
// `nyx-platform network-band …` there, with the same arguments and output.

/// Run a command to completion and return its trimmed stdout.
pub fn run(args: &[&str]) -> Result<String> {
    let args = nyx_platform::resolve(args);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    nyx_core::cmd::run(&refs)
}

/// Like [`run`] but returns `None` on any failure.
pub fn output(args: &[&str]) -> Option<String> {
    run(args).ok()
}

/// Is `program` on `PATH` (or a script `nyx-platform` stands in for)?
pub fn present(program: &str) -> bool {
    nyx_platform::present(program)
}

/// Fire and forget.
pub fn spawn(args: &[&str]) {
    let args = nyx_platform::resolve(args);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    nyx_core::cmd::spawn(&refs);
}

use anyhow::{Context, Result};

/// An absolute executable path for a systemd unit, keeping profile symlinks intact.
pub fn unit_program(name: &str) -> Result<String> {
    let path = nyx_capability::providers::Programs::find(name).with_context(|| format!("{name} is not installed"))?;
    let path = path.to_str().with_context(|| format!("{name} has a non-UTF-8 path"))?;
    let path = path.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%");
    Ok(format!("\"{path}\""))
}
use gtk::{gio, glib};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Run a command off the main thread and deliver the result back on it.
pub fn run_async<F>(args: &[&str], done: F)
where
    F: FnOnce(Result<String>) + 'static,
{
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    background(
        move || {
            let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
            run(&refs)
        },
        done,
    );
}

/// Run arbitrary blocking work off the main thread, then call `done` on it.
pub fn background<T, W, F>(work: W, done: F)
where
    T: Send + 'static,
    W: FnOnce() -> T + Send + 'static,
    F: FnOnce(T) + 'static,
{
    let handle = gio::spawn_blocking(work);
    glib::spawn_future_local(async move {
        if let Ok(value) = handle.await {
            done(value);
        }
    });
}

type Fetched = (Vec<String>, Instant, Result<String, String>);

/// Answers of read-only commands run ahead on worker threads, for pages built soon after.
static AHEAD: Mutex<Vec<Fetched>> = Mutex::new(Vec::new());
/// How long an answer fetched ahead stands in for running the command.
const AHEAD_FOR: Duration = Duration::from_secs(30);

thread_local! {
    static BUILDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run these read-only commands now, each on its own thread, so the pages that
/// read them with [`read`] don't wait on them while being built.
pub fn fetch_ahead<C: AsRef<[&'static str]>>(commands: &[C]) {
    for args in commands {
        let args: Vec<String> = args.as_ref().iter().map(|s| s.to_string()).collect();
        std::thread::spawn(move || {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let answer = run(&refs).map_err(|e| format!("{e:#}"));
            let mut ahead = AHEAD.lock().unwrap_or_else(|e| e.into_inner());
            ahead.retain(|(a, _, _)| *a != args);
            ahead.push((args, Instant::now(), answer));
        });
    }
}

/// Drop every answer fetched ahead: something changed, so they may be stale.
pub fn forget_ahead() {
    AHEAD.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Build a page: inside, [`read`] uses answers fetched ahead.
pub fn building<T>(build: impl FnOnce() -> T) -> T {
    let was = BUILDING.with(|b| b.replace(true));
    let out = build();
    BUILDING.with(|b| b.set(was));
    out
}

/// [`run`], or while a page is first built, the recent answer [`fetch_ahead`] got.
/// Live checks and later redraws always run the command.
pub fn read(args: &[&str]) -> Result<String> {
    if BUILDING.with(|b| b.get()) {
        let ahead = AHEAD.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, _, answer)) =
            ahead.iter().find(|(a, at, _)| at.elapsed() < AHEAD_FOR && a.iter().map(String::as_str).eq(args.iter().copied()))
        {
            return answer.clone().map_err(anyhow::Error::msg);
        }
    }
    run(args)
}
