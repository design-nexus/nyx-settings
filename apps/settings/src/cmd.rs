//! Running external commands without blocking the UI thread.

pub use nyx_core::cmd::*;

use anyhow::Result;
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
