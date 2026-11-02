//! The language model: the one in use, and the recommended ones for this computer in a list,
//! with what the picked one is and Use or Download beneath.

use super::{client, size};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::BufRead;

/// A download in progress, by URL. Kept outside the page so a redraw doesn't lose it.
#[derive(Clone, Default)]
struct Progress {
    bytes: u64,
    total: u64,
    speed: f64,
}

thread_local! {
    static DOWNLOADS: RefCell<HashMap<String, Progress>> = RefCell::default();
    /// The progress bars showing each download (the page may be redrawn meanwhile).
    static BARS: RefCell<Vec<(String, glib::WeakRef<gtk::ProgressBar>)>> = RefCell::default();
    /// The model picked in the list (its file name).
    static SELECTED: RefCell<Option<String>> = RefCell::default();
}

enum Update {
    Progress(String, Progress),
    Done(String, Result<(), String>),
}

/// Download each URL in turn (a model, then its projector) with `nyx-assistant models download`.
fn download(urls: Vec<String>, name: String) {
    let urls: Vec<String> = urls.into_iter().filter(|u| !DOWNLOADS.with(|d| d.borrow().contains_key(u))).collect();
    if urls.is_empty() {
        return;
    }
    for u in &urls {
        DOWNLOADS.with(|d| d.borrow_mut().insert(u.clone(), Progress::default()));
    }
    let (tx, rx) = std::sync::mpsc::channel::<Update>();
    std::thread::spawn(move || {
        for url in urls {
            let result = (|| -> Result<(), String> {
                let mut child = std::process::Command::new(client::PROGRAM)
                    .args(["models", "download", &url])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .map_err(|e| e.to_string())?;
                let mut failed = None;
                for line in std::io::BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
                    let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                    if let Some(e) = v["error"].as_str() {
                        failed = Some(e.to_string());
                    }
                    let p = Progress {
                        bytes: v["bytes"].as_u64().unwrap_or(0),
                        total: v["total"].as_u64().unwrap_or(0),
                        speed: v["speed"].as_f64().unwrap_or(0.0),
                    };
                    let _ = tx.send(Update::Progress(url.clone(), p));
                }
                let out = child.wait_with_output().map_err(|e| e.to_string())?;
                match failed {
                    Some(e) => Err(e),
                    None if !out.status.success() => {
                        Err(String::from_utf8_lossy(&out.stderr).rsplit("Error: ").next().unwrap_or("").trim().to_string())
                    }
                    None => Ok(()),
                }
            })();
            let stop = result.is_err();
            let _ = tx.send(Update::Done(url, result));
            if stop {
                break;
            }
        }
    });
    glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
        while let Ok(u) = rx.try_recv() {
            match u {
                Update::Progress(url, p) => {
                    show(&url, &p);
                    DOWNLOADS.with(|d| d.borrow_mut().insert(url, p));
                }
                Update::Done(url, result) => {
                    DOWNLOADS.with(|d| d.borrow_mut().remove(&url));
                    if let Err(e) = result {
                        window::toast(&format!("Couldn't download {name}: {e}"));
                        // The rest of this batch was skipped.
                        DOWNLOADS.with(|d| d.borrow_mut().clear());
                        window::rebuild("assistant-models");
                        return glib::ControlFlow::Break;
                    }
                    if DOWNLOADS.with(|d| d.borrow().is_empty()) {
                        window::toast(&format!("Downloaded {name}."));
                        window::rebuild("assistant-models");
                        return glib::ControlFlow::Break;
                    }
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

fn show(url: &str, p: &Progress) {
    BARS.with(|b| {
        b.borrow_mut().retain(|(_, w)| w.upgrade().is_some());
        for (u, w) in b.borrow().iter() {
            if u == url
                && let Some(bar) = w.upgrade()
            {
                if p.total > 0 {
                    bar.set_fraction(p.bytes as f64 / p.total as f64);
                }
                bar.set_text(Some(&format!("{} of {} · {}/s", size(p.bytes), size(p.total), size(p.speed as u64))));
            }
        }
    });
}

/// A Download button, or a progress bar while that file downloads.
fn download_control(urls: Vec<String>, name: &str) -> gtk::Widget {
    if let Some(p) = DOWNLOADS.with(|d| urls.iter().find_map(|u| d.borrow().get(u).cloned().map(|p| (u.clone(), p)))) {
        let bar = gtk::ProgressBar::new();
        bar.set_show_text(true);
        bar.set_width_request(220);
        bar.set_valign(gtk::Align::Center);
        // The projector follows the model in the same bar.
        for u in &urls {
            BARS.with(|b| b.borrow_mut().push((u.clone(), bar.downgrade())));
        }
        show(&p.0, &p.1);
        return bar.upcast();
    }
    let b = gtk::Button::with_label("Download");
    let name = name.to_string();
    b.connect_clicked(move |_| {
        download(urls.clone(), name.clone());
        window::rebuild("assistant-models");
    });
    b.upcast()
}

fn use_model(path: String) {
    cmd::background(
        move || client::run(&["models", "use", &path]),
        |r| {
            match r {
                Ok(_) => window::toast("Switched models. The assistant restarted to load it."),
                Err(e) => window::toast(&format!("Couldn't switch: {}", e.to_string().rsplit("Error: ").next().unwrap_or(""))),
            }
            window::rebuild("assistant-models");
        },
    );
}

fn use_button(path: &str) -> gtk::Button {
    let b = gtk::Button::with_label("Use");
    let path = path.to_string();
    b.connect_clicked(move |b| {
        b.set_sensitive(false);
        use_model(path.clone());
    });
    b
}

/// What a recommended model is, beneath the list, with Use or Download.
fn details(into: &gtk::Box, m: &Value, current: &str) {
    while let Some(c) = into.first_child() {
        into.remove(&c);
    }
    let name = m["name"].as_str().unwrap_or("");
    let path = m["path"].as_str();
    let control: gtk::Widget = match path {
        Some(p) if p == current => widgets::tag("In use").upcast(),
        Some(p) => use_button(p).upcast(),
        None => {
            let mut urls = vec![m["download_url"].as_str().unwrap_or("").to_string()];
            if let Some(p) = m["mmproj_url"].as_str() {
                urls.push(p.to_string());
            }
            download_control(urls, name)
        }
    };
    let mut facts = format!(
        "{} download · about {:.1} GB of graphics memory",
        m["size_str"].as_str().unwrap_or(""),
        m["vram_gb"].as_f64().unwrap_or(0.0)
    );
    if m["mmproj"].is_string() {
        facts.push_str(" · can look at the screen");
    }
    if path.is_some_and(|p| p != current) {
        facts.push_str(" · downloaded");
    }
    into.append(&widgets::row(&gtk::glib::markup_escape_text(m["description"].as_str().unwrap_or("")), &facts, Some(&control)));
}

pub fn build(page: &Page) {
    let Some(list) = client::json(&["models", "list", "--json"]) else {
        page.banner("Couldn't list the models. Update Nyx Assistant (./install.sh assistant) and try again.", true);
        return;
    };
    let current = list["current"].as_str().unwrap_or("");

    let g = page.group("In use");
    let vision = if list["projector"].is_null() { "Can't look at the screen." } else { "Can look at the screen." };
    match list["current_model"].as_object() {
        Some(m) => g.add(&widgets::row(
            m["name"].as_str().unwrap_or(""),
            &format!("{} {vision}", gtk::glib::markup_escape_text(m["description"].as_str().unwrap_or(""))),
            None,
        )),
        None => g.add(&widgets::row(
            current.rsplit('/').next().unwrap_or("none"),
            &format!("Not one of the recommended models. {vision}"),
            None,
        )),
    }

    let g = page.group("Installed models");
    let installed: Vec<Value> = list["installed"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| {
            let name = m["name"].as_str().unwrap_or("").to_ascii_lowercase();
            m["path"].is_string()
                && name.ends_with(".gguf")
                && !["mmproj", "drafter", "dflash", "codec", ".encoder.", ".decoder."].iter().any(|part| name.contains(part))
        })
        .cloned()
        .collect();
    if installed.is_empty() {
        g.note("No local language models found.");
    } else {
        let options: Vec<(String, String)> = installed
            .iter()
            .map(|m| {
                let path = m["path"].as_str().unwrap_or("");
                let name = m["name"].as_str().unwrap_or("Model").trim_end_matches(".gguf");
                (path.to_string(), if path == current { format!("{name} (in use)") } else { name.to_string() })
            })
            .collect();
        let selected = options.iter().find(|(path, _)| path == current).map(|(path, _)| path.as_str()).unwrap_or(&options[0].0);
        let picker = widgets::dropdown(&options, selected);
        picker.set_enable_search(true);
        picker.set_hexpand(true);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let update = {
            let actions = actions.clone();
            let installed = installed.clone();
            let current = current.to_string();
            move |index: usize| {
                while let Some(child) = actions.first_child() {
                    actions.remove(&child);
                }
                let Some(model) = installed.get(index) else { return };
                let path = model["path"].as_str().unwrap_or("");
                if path == current {
                    actions.append(&widgets::tag("In use"));
                } else {
                    actions.append(&use_button(path));
                }
                actions.set_tooltip_text(Some(path));
            }
        };
        update(picker.selected() as usize);
        picker.connect_selected_notify(move |picker| update(picker.selected() as usize));
        g.add(&widgets::row("Model", "Includes downloaded models outside the recommended list.", Some(picker.upcast_ref())));
        g.add(&actions);
        g.note("Selecting a model does not change the default. Use switches the assistant and restarts it.");
    }

    let g = page.group("Recommended models");
    let models = list["recommended"].as_array().cloned().unwrap_or_default();
    if models.is_empty() {
        g.note("None of the recommended models fits this computer's graphics memory.");
        return;
    }
    let options: Vec<(String, String)> = models
        .iter()
        .map(|m| {
            let name = m["name"].as_str().unwrap_or("").to_string();
            let label = match m["path"].as_str() {
                Some(p) if p == current => format!("{name} (in use)"),
                Some(_) => format!("{name} (downloaded)"),
                None => name,
            };
            (m["filename"].as_str().unwrap_or("").to_string(), label)
        })
        .collect();
    // The choice survives a redraw (a download finishing); otherwise the model in use.
    let chosen = SELECTED
        .with(|s| s.borrow().clone())
        .filter(|f| options.iter().any(|(id, _)| id == f))
        .or_else(|| {
            models.iter().find(|m| m["path"].as_str() == Some(current)).and_then(|m| m["filename"].as_str().map(String::from))
        })
        .unwrap_or_else(|| options[0].0.clone());
    let dd = widgets::dropdown(&options, &chosen);
    g.add(&widgets::row("Model", "Pick one to see what it is.", Some(dd.upcast_ref())));
    let info = gtk::Box::new(gtk::Orientation::Vertical, 0);
    if let Some(m) = models.iter().find(|m| m["filename"].as_str() == Some(chosen.as_str())) {
        details(&info, m, current);
    }
    g.add(&info);
    let current = current.to_string();
    dd.connect_selected_notify(move |dd| {
        let Some(m) = models.get(dd.selected() as usize) else { return };
        SELECTED.with(|s| *s.borrow_mut() = m["filename"].as_str().map(String::from));
        details(&info, m, &current);
    });
    g.note("Switching restarts the assistant. The graphics card also holds the speech and voice models, and the conversation's context.");
    widgets::keywords("model llm gguf download installed available gemma nemotron bolt qwen vision");
}
