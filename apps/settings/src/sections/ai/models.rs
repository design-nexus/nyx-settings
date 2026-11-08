//! Local AI models: downloading from Hugging Face, what's installed, updates, and
//! removal. Downloads belong to Nyx AI Core and go on if Settings closes.

use super::{call, capabilities, change, size};
use crate::widgets::{self, Page};
use crate::window;
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::BufRead;

const PAGE: &str = "local-ai-models";

thread_local! {
    /// Downloads this window started: model -> (done bytes, total bytes, status).
    static DOWNLOADS: RefCell<BTreeMap<String, (u64, u64, String)>> = RefCell::default();
    static BARS: RefCell<Vec<(String, glib::WeakRef<gtk::ProgressBar>)>> = RefCell::default();
}

enum Update {
    Progress(u64, u64, String),
    Done(Result<(), String>),
}

/// `nyx-ai pull MODEL --json`: progress lines, then the result.
fn download(model: String) {
    if DOWNLOADS.with(|d| d.borrow().contains_key(&model)) {
        return;
    }
    DOWNLOADS.with(|d| d.borrow_mut().insert(model.clone(), (0, 0, "Starting".into())));
    let (tx, rx) = std::sync::mpsc::channel::<Update>();
    let target = model.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let mut child = std::process::Command::new(super::PROGRAM)
                .args(["pull", &target, "--json"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| e.to_string())?;
            let mut outcome = Err("Nyx AI Core stopped reporting the download".to_string());
            for line in std::io::BufReader::new(child.stdout.take().expect("piped")).lines().map_while(Result::ok) {
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if v["ok"] == true {
                    outcome = Ok(());
                } else if let Some(e) = v["error"]["message"].as_str() {
                    outcome = Err(e.to_string());
                } else if v["total"].is_u64() {
                    let _ = tx.send(Update::Progress(
                        v["completed"].as_u64().unwrap_or(0),
                        v["total"].as_u64().unwrap_or(0),
                        v["status"].as_str().unwrap_or("").to_string(),
                    ));
                }
            }
            let _ = child.wait();
            outcome
        })();
        let _ = tx.send(Update::Done(result));
    });
    glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
        while let Ok(u) = rx.try_recv() {
            match u {
                Update::Progress(done, total, status) => {
                    DOWNLOADS.with(|d| d.borrow_mut().insert(model.clone(), (done, total, status)));
                    show(&model);
                }
                Update::Done(result) => {
                    DOWNLOADS.with(|d| d.borrow_mut().remove(&model));
                    match result {
                        Ok(()) => window::toast(&format!("Downloaded {model}")),
                        Err(e) => window::toast(&format!("Couldn't download {model}: {e}")),
                    }
                    window::rebuild(PAGE);
                    return glib::ControlFlow::Break;
                }
            }
        }
        glib::ControlFlow::Continue
    });
    window::rebuild(PAGE);
}

fn show(model: &str) {
    let Some((done, total, status)) = DOWNLOADS.with(|d| d.borrow().get(model).cloned()) else { return };
    BARS.with(|b| {
        b.borrow_mut().retain(|(_, w)| w.upgrade().is_some());
        for (m, w) in b.borrow().iter() {
            if m == model
                && let Some(bar) = w.upgrade()
            {
                if total > 0 {
                    bar.set_fraction(done as f64 / total as f64);
                    bar.set_text(Some(&format!("{} of {}", size(done), size(total))));
                } else {
                    bar.set_text(Some(&status));
                }
            }
        }
    });
}

fn progress_bar(model: &str) -> gtk::ProgressBar {
    let bar = gtk::ProgressBar::new();
    bar.set_show_text(true);
    bar.set_width_request(220);
    bar.set_valign(gtk::Align::Center);
    BARS.with(|b| b.borrow_mut().push((model.to_string(), bar.downgrade())));
    show(model);
    bar
}

fn button(label: &str, action: impl Fn(&gtk::Button) + 'static) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.set_valign(gtk::Align::Center);
    b.connect_clicked(action);
    b
}

pub fn build(page: &Page) {
    let models = match call(&["models"]) {
        Ok(Value::Array(list)) => list,
        Ok(_) => Vec::new(),
        Err(e) => {
            page.banner(&format!("Nyx AI Core isn't reachable: {e}"), true);
            return;
        }
    };

    // Downloads Nyx AI Core is running, including its own updates.
    let running: BTreeMap<String, (u64, u64)> = call(&["downloads"])
        .ok()
        .and_then(|v| v.as_array().cloned())
        .into_iter()
        .flatten()
        .filter(|d| matches!(d["state"].as_str(), Some("queued" | "running")))
        .filter_map(|d| Some((d["model"].as_str()?.to_string(), (d["completed"].as_u64().unwrap_or(0), d["total"].as_u64().unwrap_or(0)))))
        .collect();

    let g = page.group("Download a model");
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("org/repo-GGUF:Q4_K_M"));
    entry.set_width_chars(30);
    let get = {
        let entry = entry.clone();
        move || {
            let name = entry.text().trim().to_string();
            if name.contains('/') && name.contains(':') {
                entry.set_text("");
                download(name);
            } else {
                window::toast("Name a Hugging Face GGUF repository and quantization, like org/repo-GGUF:Q4_K_M");
            }
        }
    };
    let go = get.clone();
    entry.connect_activate(move |_| go());
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.append(&entry);
    controls.append(&button("Download", move |_| get()));
    g.add(&widgets::row(
        "From Hugging Face",
        "A GGUF repository and quantization. Its vision projector and draft head come along when it has them.",
        Some(controls.upcast_ref()),
    ));
    let pending: Vec<String> = DOWNLOADS.with(|d| d.borrow().keys().cloned().collect());
    for model in &pending {
        if !models.iter().any(|m| m["id"].as_str() == Some(model)) {
            g.add(&widgets::row(model, "Downloading", Some(progress_bar(model).upcast_ref())));
        }
    }

    let installed: Vec<&Value> = models.iter().filter(|m| m["state"].as_str() != Some("missing")).collect();
    let total: u64 = installed.iter().map(|m| m["size"].as_u64().unwrap_or(0)).sum();
    let g = page.group("Installed");
    let updates = installed.iter().filter(|m| m["update_available"] == true).count();
    let check = button("Check for updates", |b| {
        b.set_sensitive(false);
        b.set_label("Checking…");
        crate::cmd::background(
            || call(&["updates"]),
            |r| {
                match r {
                    Ok(Value::Array(found)) if found.is_empty() => window::toast("Every model is on its newest version"),
                    Ok(Value::Array(found)) => window::toast(&format!("{} model update{} available", found.len(), if found.len() == 1 { "" } else { "s" })),
                    Ok(_) => {}
                    Err(e) => window::toast(&format!("Couldn't check: {e}")),
                }
                window::rebuild(PAGE);
            },
        );
    });
    let summary = format!(
        "{} model{} · {}{}",
        installed.len(),
        if installed.len() == 1 { "" } else { "s" },
        size(total),
        if updates > 0 { format!(" · {updates} with updates") } else { String::new() }
    );
    g.add(&widgets::row("Models on this computer", &summary, Some(check.upcast_ref())));
    if installed.is_empty() {
        g.note("Nothing installed yet. Download a model above, or from Chat's Marketplace.");
    }
    for m in installed {
        let id = m["id"].as_str().unwrap_or("").to_string();
        let mut bits = vec![size(m["size"].as_u64().unwrap_or(0))];
        for key in ["parameters", "quantization", "architecture"] {
            if let Some(v) = m[key].as_str() {
                bits.push(v.to_string());
            }
        }
        let caps = capabilities(m);
        if !caps.is_empty() {
            bits.push(caps.join(", "));
        }
        let state = m["state"].as_str().unwrap_or("");
        match state {
            "ready" | "busy" => bits.push("loaded".into()),
            "loading" => bits.push("loading".into()),
            "failed" => bits.push(format!("can't load: {}", m["error"].as_str().unwrap_or("unknown"))),
            _ => {}
        }
        if m["source"].as_str() == Some("folder") {
            bits.push("in a model folder".into());
        }
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        if DOWNLOADS.with(|d| d.borrow().contains_key(&id)) {
            controls.append(&progress_bar(&id));
        } else if let Some((done, total)) = running.get(&id) {
            let text = if *total > 0 { format!("Updating · {} of {}", size(*done), size(*total)) } else { "Updating".to_string() };
            let l = gtk::Label::new(Some(&text));
            l.add_css_class("dim");
            controls.append(&l);
        } else if m["update_available"] == true {
            let target = id.clone();
            let label = match m["update_size"].as_u64() {
                Some(bytes) if bytes > 0 => format!("Update ({})", size(bytes)),
                _ => "Update".to_string(),
            };
            controls.append(&button(&label, move |_| download(target.clone())));
        }
        if m["removable"] == true && !matches!(state, "busy" | "loading") {
            let target = id.clone();
            let remove = widgets::two_click("Remove", "Click again to remove", move || {
                change(vec!["rm".into(), target.clone()], PAGE, "Model removed");
            });
            remove.set_valign(gtk::Align::Center);
            controls.append(&remove);
        }
        let row = widgets::row(&id, &glib::markup_escape_text(&bits.join(" · ")), Some(controls.upcast_ref()));
        g.add(&row);
    }
    g.note("Models in ~/.local/share/nyx-ai/models and Nyx Assistant's model folder are listed too; Settings never deletes those files.");
    widgets::keywords("ai model models llm gguf hugging face download remove update llama local");
}
