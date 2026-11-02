//! Live views for extension pages: a camera preview (`camera` rows) and a
//! microphone level graph (`meter` rows). Both run a child process only while
//! the row is on screen, and stop it when the page is hidden or rebuilt.

use crate::theme;
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ----- Shared bits -----

/// A child process and what its reader thread has seen.
struct Running<T> {
    child: Child,
    shared: Arc<Mutex<Feed<T>>>,
}

#[derive(Default)]
struct Feed<T> {
    latest: Option<T>,
    ended: bool,
    error: String,
}

impl<T> Drop for Running<T> {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawn `argv`; `pump` reads its stdout on a thread and stores results in the feed.
fn start<T: Send + 'static>(
    argv: &[String],
    pump: impl FnOnce(std::process::ChildStdout, &Arc<Mutex<Feed<T>>>) + Send + 'static,
) -> Result<Running<T>, String> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't start {}: {e}", argv[0]))?;
    let shared: Arc<Mutex<Feed<T>>> = Arc::new(Mutex::new(Feed { latest: None, ended: false, error: String::new() }));
    let out = child.stdout.take().ok_or("no output")?;
    let mut err = child.stderr.take().ok_or("no error output")?;
    let s = shared.clone();
    std::thread::spawn(move || {
        pump(out, &s);
        // The process ended: keep the last line it printed, it says why.
        let mut text = String::new();
        let _ = err.read_to_string(&mut text);
        let mut f = s.lock().unwrap();
        f.ended = true;
        f.error = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
    });
    Ok(Running { child, shared })
}

fn rgb(hex: &str) -> (f64, f64, f64) {
    let c = gdk::RGBA::parse(hex).unwrap_or(gdk::RGBA::WHITE);
    (c.red() as f64, c.green() as f64, c.blue() as f64)
}

// ----- Camera -----

/// Split complete JPEG frames (FFD8 … FFD9) off the front of `buf`.
pub fn split_frames(buf: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    loop {
        let Some(start) = buf.windows(2).position(|w| w == [0xFF, 0xD8]) else {
            buf.clear();
            break;
        };
        let Some(end) = buf[start + 2..].windows(2).position(|w| w == [0xFF, 0xD9]).map(|e| start + 2 + e + 2) else {
            buf.drain(..start);
            break;
        };
        frames.push(buf[start..end].to_vec());
        buf.drain(..end);
    }
    frames
}

fn camera_command(device: &str) -> Vec<String> {
    [
        "ffmpeg",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "v4l2",
        "-i",
        device,
        "-vf",
        "fps=15,scale=640:-2",
        "-c:v",
        "mjpeg",
        "-q:v",
        "5",
        "-f",
        "image2pipe",
        "-",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// A preview of a video device, started and stopped with a button.
pub fn camera(title: &str, desc: &str, device: &str) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 10);
    outer.add_css_class("settings-option");
    outer.add_css_class("tall");

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let text = crate::widgets::vbox(2);
    text.set_hexpand(true);
    text.append(&crate::widgets::label(if title.is_empty() { "Preview" } else { title }, "settings-option-title"));
    if !desc.is_empty() {
        let d = crate::widgets::label(desc, "settings-option-description");
        d.set_wrap(true);
        text.append(&d);
    }
    head.append(&text);
    let button = gtk::Button::with_label("Start preview");
    button.set_valign(gtk::Align::Center);
    head.append(&button);
    outer.append(&head);

    let frame = gtk::Overlay::new();
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_size_request(-1, 360);
    picture.add_css_class("camera-preview");
    crate::widgets::paint(&picture, "rgba(0,0,0,0.35)");
    frame.set_child(Some(&picture));
    let status = crate::widgets::label("Press Start preview to see the camera.", "dim");
    status.set_halign(gtk::Align::Center);
    status.set_valign(gtk::Align::Center);
    status.set_wrap(true);
    status.set_justify(gtk::Justification::Center);
    frame.add_overlay(&status);
    outer.append(&frame);

    let running: Rc<RefCell<Option<Running<Vec<u8>>>>> = Rc::default();
    let stop = {
        let (running, button) = (running.clone(), button.clone());
        Rc::new(move || {
            running.borrow_mut().take();
            button.set_label("Start preview");
        })
    };
    {
        let (running, stop, picture, status) = (running.clone(), stop.clone(), picture.clone(), status.clone());
        let device = device.to_string();
        button.connect_clicked(move |b| {
            if running.borrow().is_some() {
                stop();
                status.set_text("Preview stopped.");
                status.set_visible(true);
                picture.set_paintable(None::<&gdk::Paintable>);
                return;
            }
            let pump = |mut out: std::process::ChildStdout, feed: &Arc<Mutex<Feed<Vec<u8>>>>| {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 65536];
                while let Ok(n) = out.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(last) = split_frames(&mut buf).pop() {
                        feed.lock().unwrap().latest = Some(last);
                    }
                }
            };
            match start(&camera_command(&device), pump) {
                Ok(r) => {
                    *running.borrow_mut() = Some(r);
                    b.set_label("Stop");
                    status.set_text("Starting the camera…");
                    status.set_visible(true);
                }
                Err(e) => {
                    status.set_text(&e);
                    status.set_visible(true);
                }
            }
        });
    }
    // Snapshots (a developer aid) start the preview so it shows up in the picture.
    if std::env::var_os("SETTINGS_SNAPSHOT").is_some() {
        let b = button.clone();
        glib::idle_add_local_once(move || b.emit_clicked());
    }
    // Show new frames; stop when the page goes away.
    let weak = outer.downgrade();
    glib::timeout_add_local(Duration::from_millis(66), move || {
        let Some(outer) = weak.upgrade() else {
            running.borrow_mut().take();
            return glib::ControlFlow::Break;
        };
        if running.borrow().is_none() {
            return glib::ControlFlow::Continue;
        }
        if !outer.is_mapped() {
            stop();
            status.set_text("Preview stopped.");
            return glib::ControlFlow::Continue;
        }
        let (frame, ended, error) = {
            let r = running.borrow();
            let mut f = r.as_ref().unwrap().shared.lock().unwrap();
            (f.latest.take(), f.ended, f.error.clone())
        };
        if let Some(bytes) = frame
            && let Ok(t) = gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes))
        {
            picture.set_paintable(Some(&t));
            status.set_visible(false);
        }
        if ended {
            stop();
            let why = if error.contains("busy") {
                "The camera is in use by another app.".to_string()
            } else if error.is_empty() {
                "The preview stopped.".to_string()
            } else {
                format!("The preview stopped: {error}")
            };
            status.set_text(&why);
            status.set_visible(true);
        }
        glib::ControlFlow::Continue
    });
    outer
}

// ----- Microphone meter -----

/// Peak of signed 16-bit little-endian samples, 0.0–1.0.
pub fn peak_s16(bytes: &[u8]) -> f32 {
    bytes.as_chunks::<2>().0.iter().map(|b| (i16::from_le_bytes(*b) as f32 / 32768.0).abs()).fold(0.0, f32::max)
}

/// Level for drawing: -60 dB → 0, 0 dB → 1.
pub fn level(peak: f32) -> f64 {
    if peak <= 0.0 {
        return 0.0;
    }
    ((20.0 * (peak as f64).log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

fn meter_command(source: &str) -> Vec<String> {
    let mut a: Vec<String> = ["pw-record", "--raw", "--rate", "16000", "--channels", "1", "--format", "s16", "--latency", "20ms"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if !source.is_empty() {
        a.extend(["--target".into(), source.into()]);
    }
    a.push("-".into());
    a
}

const HISTORY: usize = 150; // 5 s at 30 ms

/// A live level graph for a PipeWire source, running while it's on screen.
pub fn meter(title: &str, desc: &str, source: &str) -> gtk::Box {
    let area = gtk::DrawingArea::new();
    area.set_content_height(80);
    area.set_hexpand(true);
    let history: Rc<RefCell<VecDeque<f64>>> = Rc::new(RefCell::new(VecDeque::from(vec![0.0; HISTORY])));
    let note: Rc<RefCell<String>> = Rc::new(RefCell::new("Listening…".into()));
    {
        let (history, note) = (history.clone(), note.clone());
        area.set_draw_func(move |_, cr, w, h| {
            let pal = theme::current_palette();
            let (ar, ag, ab) = rgb(&pal.accent);
            let (dr, dg, db) = rgb(&pal.dim_text);
            let (br, bg, bb) = rgb(&pal.border);
            let (w, h) = (w as f64, h as f64);
            let bar_w = 14.0;
            let graph_w = w - bar_w - 12.0;
            // Guides at -40, -20 and 0 dB.
            cr.set_line_width(1.0);
            for db_mark in [-40.0, -20.0, 0.0] {
                let y = (h - (db_mark + 60.0) / 60.0 * h).round() + 0.5;
                cr.set_source_rgba(br, bg, bb, 0.45);
                cr.move_to(0.0, y);
                cr.line_to(graph_w, y);
                let _ = cr.stroke();
            }
            let hist = history.borrow();
            let step = graph_w / (HISTORY - 1) as f64;
            cr.move_to(0.0, h);
            for (i, v) in hist.iter().enumerate() {
                cr.line_to(i as f64 * step, h - v * h);
            }
            cr.line_to(graph_w, h);
            cr.close_path();
            cr.set_source_rgba(ar, ag, ab, 0.35);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(ar, ag, ab, 1.0);
            cr.set_line_width(1.5);
            let _ = cr.stroke();
            // The current level as a bar on the right.
            let now = hist.back().copied().unwrap_or(0.0);
            cr.set_source_rgba(br, bg, bb, 0.5);
            cr.rectangle(w - bar_w, 0.0, bar_w, h);
            let _ = cr.fill();
            cr.set_source_rgba(ar, ag, ab, 1.0);
            cr.rectangle(w - bar_w, h - now * h, bar_w, now * h);
            let _ = cr.fill();
            let n = note.borrow();
            if !n.is_empty() {
                cr.set_source_rgba(dr, dg, db, 1.0);
                cr.set_font_size(12.0);
                cr.move_to(8.0, 16.0);
                let _ = cr.show_text(&n);
            }
        });
    }

    let running: Rc<RefCell<Option<Running<Vec<f32>>>>> = Rc::default();
    let quiet_ticks = Rc::new(Cell::new(0u32));
    let source = source.to_string();
    let weak = area.downgrade();
    glib::timeout_add_local(Duration::from_millis(30), move || {
        let Some(area) = weak.upgrade() else {
            running.borrow_mut().take();
            return glib::ControlFlow::Break;
        };
        if !area.is_mapped() {
            running.borrow_mut().take();
            return glib::ControlFlow::Continue;
        }
        if running.borrow().is_none() {
            let pump = |mut out: std::process::ChildStdout, feed: &Arc<Mutex<Feed<Vec<f32>>>>| {
                let mut chunk = [0u8; 960]; // 30 ms of 16 kHz mono s16
                while out.read_exact(&mut chunk).is_ok() {
                    feed.lock().unwrap().latest.get_or_insert_with(Vec::new).push(peak_s16(&chunk));
                }
            };
            match start(&meter_command(&source), pump) {
                Ok(r) => *running.borrow_mut() = Some(r),
                Err(e) => {
                    *note.borrow_mut() = e;
                    area.queue_draw();
                    return glib::ControlFlow::Break;
                }
            }
        }
        let (peaks, ended, error) = {
            let r = running.borrow();
            let mut f = r.as_ref().unwrap().shared.lock().unwrap();
            (f.latest.take().unwrap_or_default(), f.ended, f.error.clone())
        };
        if ended {
            running.borrow_mut().take();
            *note.borrow_mut() =
                if error.is_empty() { "The microphone stopped.".into() } else { format!("Couldn't listen: {error}") };
            area.queue_draw();
            return glib::ControlFlow::Continue;
        }
        if !peaks.is_empty() {
            let mut h = history.borrow_mut();
            for p in peaks {
                let l = level(p);
                quiet_ticks.set(if l < 0.1 { quiet_ticks.get() + 1 } else { 0 });
                h.push_back(l);
                while h.len() > HISTORY {
                    h.pop_front();
                }
            }
            *note.borrow_mut() = if quiet_ticks.get() > 100 { "No sound: say something".into() } else { String::new() };
            area.queue_draw();
        }
        glib::ControlFlow::Continue
    });

    crate::widgets::stacked_row(if title.is_empty() { "Level" } else { title }, desc, area.upcast_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_jpeg_frames() {
        let mut buf = vec![0x00, 0xFF, 0xD8, 1, 2, 0xFF, 0xD9, 0xFF, 0xD8, 3, 0xFF, 0xD9, 0xFF, 0xD8, 4];
        let f = split_frames(&mut buf);
        assert_eq!(f, vec![vec![0xFF, 0xD8, 1, 2, 0xFF, 0xD9], vec![0xFF, 0xD8, 3, 0xFF, 0xD9]]);
        assert_eq!(buf, vec![0xFF, 0xD8, 4], "the partial frame waits for more data");
        buf.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(split_frames(&mut buf).len(), 1);
        assert!(buf.is_empty());
        let mut junk = vec![1, 2, 3];
        assert!(split_frames(&mut junk).is_empty() && junk.is_empty());
    }

    #[test]
    fn levels() {
        let loud = [0x00u8, 0x80, 0xFF, 0x7F]; // -32768, 32767
        assert!((peak_s16(&loud) - 1.0).abs() < 1e-3);
        assert_eq!(peak_s16(&[0, 0, 0, 0]), 0.0);
        assert_eq!(level(0.0), 0.0);
        assert!((level(1.0) - 1.0).abs() < 1e-9);
        assert!((level(0.1) - (40.0 / 60.0)).abs() < 1e-6, "-20 dB sits two thirds up");
    }

    #[test]
    fn commands() {
        assert!(camera_command("/dev/video0").contains(&"/dev/video0".to_string()));
        let m = meter_command("alsa_input.usb-OBSBOT");
        assert!(m.contains(&"--raw".to_string()) && m.ends_with(&["alsa_input.usb-OBSBOT".to_string(), "-".to_string()]));
        assert!(!meter_command("").contains(&"--target".to_string()), "no source: the default mic");
    }
}
