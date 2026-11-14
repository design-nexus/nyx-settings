//! The language model: the recommended models for this computer and the others on it
//! in a list; the picked one fills the preview with its card (what it is, the memory it
//! needs on the graphics card, how it differs from the one in use) and Use or Download.

use super::{client, size};
use crate::sections::ai as kit;
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::BufRead;
use std::rc::Rc;

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
    /// The model picked in the list (a recommended model's file name, or a file's path).
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

fn downloading(urls: &[String]) -> bool {
    DOWNLOADS.with(|d| urls.iter().any(|u| d.borrow().contains_key(u)))
}

/// A Download button, or a progress bar while that file downloads.
fn download_control(urls: Vec<String>, name: &str) -> gtk::Widget {
    if let Some(p) = DOWNLOADS.with(|d| urls.iter().find_map(|u| d.borrow().get(u).cloned().map(|p| (u.clone(), p)))) {
        let bar = gtk::ProgressBar::new();
        bar.set_show_text(true);
        bar.set_width_request(200);
        bar.set_valign(gtk::Align::Center);
        // The projector follows the model in the same bar.
        for u in &urls {
            BARS.with(|b| b.borrow_mut().push((u.clone(), bar.downgrade())));
        }
        show(&p.0, &p.1);
        return bar.upcast();
    }
    let b = gtk::Button::new();
    b.set_child(Some(&{
        let c = widgets::hbox(6);
        c.append(&gtk::Image::from_icon_name("download-symbolic"));
        c.append(&gtk::Label::new(Some("Download")));
        c
    }));
    b.add_css_class("suggested-action");
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
    b.add_css_class("suggested-action");
    let path = path.to_string();
    b.connect_clicked(move |b| {
        b.set_sensitive(false);
        use_model(path.clone());
    });
    b
}

/// A model the page can show: a recommended one (downloaded or not) or a file on this computer.
#[derive(Clone)]
struct Model {
    /// What picks it: a recommended model's file name, or a file's path.
    key: String,
    name: String,
    /// The recommended model's line about itself.
    about: String,
    /// "5.3 GB" to download, or the file's size.
    size: String,
    /// About how much graphics memory it needs, when the assistant knows.
    vram: Option<f64>,
    vision: bool,
    /// Where it is on this computer.
    path: Option<String>,
    /// Why it's a poor fit, when the assistant knows.
    unsuitable: Option<String>,
    /// Where to download it (the model, then its projector).
    urls: Vec<String>,
    repo: Option<String>,
    curated: bool,
    /// One of the models recommended for this computer's graphics card.
    fits: bool,
}

impl Model {
    fn curated(m: &Value) -> Model {
        let mut urls = vec![m["download_url"].as_str().unwrap_or("").to_string()];
        if let Some(p) = m["mmproj_url"].as_str() {
            urls.push(p.to_string());
        }
        Model {
            key: m["filename"].as_str().unwrap_or("").to_string(),
            name: kit::plain_name(m["name"].as_str().unwrap_or("")),
            about: m["description"].as_str().unwrap_or("").to_string(),
            size: m["size_str"].as_str().unwrap_or("").to_string(),
            vram: m["vram_gb"].as_f64(),
            vision: m["mmproj"].is_string(),
            path: m["path"].as_str().map(str::to_string),
            unsuitable: m["unsuitable"].as_str().map(str::to_string),
            urls,
            repo: m["hf_repo"].as_str().map(str::to_string),
            curated: true,
            fits: false,
        }
    }

    fn file(m: &Value, curated: Option<&Value>) -> Model {
        let path = m["path"].as_str().unwrap_or("").to_string();
        let file = m["name"].as_str().unwrap_or("");
        match curated {
            Some(c) => Model { key: path.clone(), path: Some(path), size: size(m["bytes"].as_u64().unwrap_or(0)), ..Model::curated(c) },
            None => Model {
                key: path.clone(),
                name: kit::plain_name(file),
                about: String::new(),
                size: size(m["bytes"].as_u64().unwrap_or(0)),
                vram: None,
                vision: m["vision"].as_bool().unwrap_or(false),
                path: Some(path),
                unsuitable: None,
                urls: Vec::new(),
                repo: None,
                curated: false,
                fits: false,
            },
        }
    }

    fn in_use(&self, current: &str) -> bool {
        self.path.as_deref() == Some(current)
    }
}

/// Where a model file sits, in words.
fn folder_words(path: &str, models_dir: &str) -> &'static str {
    if !models_dir.is_empty() && path.starts_with(models_dir) {
        "In the assistant's folder"
    } else if path.contains("/.lmstudio/") {
        "In LM Studio's folder"
    } else if path.contains("/huggingface/") {
        "Downloaded from Hugging Face"
    } else {
        "In a model folder"
    }
}

/// What a row shows at its right: In use, Downloaded, the download's size.
fn row_end(m: &Model, current: &str) -> gtk::Widget {
    if m.in_use(current) {
        return kit::pill("In use", "use").upcast();
    }
    let text = if downloading(&m.urls) {
        "Downloading".to_string()
    } else if m.fits && m.path.is_some() {
        "Downloaded".to_string()
    } else {
        m.size.clone()
    };
    let l = widgets::label(&text, "row-value");
    l.set_valign(gtk::Align::Center);
    l.upcast()
}

/// The models' rows, so the picked one can be marked.
type Rows = Rc<RefCell<Vec<(String, gtk::Box)>>>;

fn model_row(m: &Model, desc: &str, current: &str, rows: &Rows, pick: &Rc<dyn Fn(&str)>) -> gtk::Box {
    let r = widgets::row(&m.name, &glib::markup_escape_text(desc), Some(&row_end(m, current)));
    r.prepend(&kit::art(&m.name, 30));
    r.add_css_class("with-tile");
    r.add_css_class("alm-row");
    if let Some(p) = &m.path {
        r.set_tooltip_text(Some(p));
    }
    let (key, pick) = (m.key.clone(), pick.clone());
    kit::on_click(&r, move || pick(&key));
    rows.borrow_mut().push((m.key.clone(), r.clone()));
    r
}

/// A figure in the card: a big number, its unit and what it is.
fn stat(big: &str, unit: &str, what: &str) -> gtk::Box {
    let b = widgets::vbox(2);
    b.add_css_class("alm-stat");
    b.set_hexpand(true);
    let top = widgets::hbox(4);
    top.append(&widgets::label(big, "alm-stat-big"));
    if !unit.is_empty() {
        let u = widgets::label(unit, "alm-stat-unit");
        u.set_valign(gtk::Align::End);
        top.append(&u);
    }
    b.append(&top);
    let w = widgets::label(what, "alm-stat-what");
    w.set_wrap(true);
    b.append(&w);
    b
}

/// The picked model's card, and how it compares with the one in use.
fn render(into: &gtk::Box, m: &Model, in_use: Option<&Model>, current: &str, card_gb: Option<f64>, narrow: bool) {
    kit::clear(into);
    let card = widgets::vbox(14);
    card.add_css_class("preview-card");
    card.add_css_class("alm-card");

    let head = widgets::hbox(14);
    head.append(&kit::art(&m.name, if narrow { 44 } else { 56 }));
    let text = widgets::vbox(3);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(&m.name, "alm-name");
    t.set_wrap(true);
    text.append(&t);
    let mut line = Vec::new();
    if let Some(repo) = &m.repo {
        line.push(format!("From {repo}"));
    }
    match (&m.path, m.curated) {
        (None, _) => line.push(format!("{} download", m.size)),
        (Some(_), true) => line.push(format!("{} · downloaded", m.size)),
        (Some(p), false) => line.push(format!("{} · {}", m.size, folder_words(p, "").to_lowercase())),
    }
    let l = widgets::label(&line.join(" · "), "alm-line");
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    text.append(&l);
    head.append(&text);
    let control: gtk::Widget = match &m.path {
        Some(p) if p == current => kit::pill("In use", "use").upcast(),
        Some(p) => use_button(p).upcast(),
        None => download_control(m.urls.clone(), &m.name),
    };
    control.set_valign(gtk::Align::Center);
    head.append(&control);
    card.append(&head);

    let mut chips = vec![if m.vision {
        kit::chip("object-select-symbolic", "Sees the screen", "good")
    } else {
        kit::chip("window-close-symbolic", "Can't see the screen", "no")
    }];
    if m.fits {
        chips.insert(0, kit::chip("object-select-symbolic", "Recommended for this computer", "good"));
    }
    card.append(&kit::chips(chips));
    if !m.about.is_empty() {
        let a = widgets::label(&m.about, "alm-about");
        a.set_wrap(true);
        card.append(&a);
    }
    if let Some(why) = &m.unsuitable {
        card.append(&kit::icon_line("dialog-warning-symbolic", why, "down"));
    }

    let stats = widgets::hbox(0);
    stats.add_css_class("alm-stats");
    stats.set_homogeneous(true);
    let (n, unit) = m.size.rsplit_once(' ').unwrap_or((&m.size, ""));
    stats.append(&stat(n, unit, if m.path.is_some() { "On disk" } else { "To download" }));
    if let Some(gb) = m.vram {
        stats.append(&stat(&format!("{gb:.1}"), "GB", "Graphics memory"));
    }
    if let Some(card_gb) = card_gb {
        stats.append(&stat(&format!("{card_gb:.0}"), "GB", "This computer's card"));
    }
    card.append(&stats);

    // The memory it needs, drawn on the graphics card.
    if let (Some(gb), Some(card_gb)) = (m.vram, card_gb) {
        let fit = widgets::vbox(8);
        fit.add_css_class("alm-fit");
        let spare = card_gb - gb;
        let figure = if spare >= 0.0 {
            format!("Fits, with {spare:.1} GB to spare")
        } else {
            format!("Needs {:.1} GB more than the card has", -spare)
        };
        let h = kit::bar_head("On this computer's graphics card", &figure);
        if spare < 0.0 {
            h.add_css_class("over");
        }
        fit.append(&h);
        fit.append(&kit::stacked_bar(vec![(gb, None)], card_gb, 10));
        let legend = widgets::hbox(14);
        legend.append(&kit::legend_item(kit::Swatch::Section, &m.name, &format!("{gb:.1} GB")));
        legend.append(&kit::legend_item(kit::Swatch::Free, "Free", &format!("{:.1} GB", spare.max(0.0))));
        fit.append(&legend);
        card.append(&fit);
    }
    into.append(&card);

    // How it differs from the one in use.
    let Some(now) = in_use.filter(|now| now.key != m.key && !narrow) else { return };
    let vs = widgets::vbox(10);
    vs.add_css_class("preview-card");
    vs.add_css_class("alm-vs");
    vs.append(&widgets::label(&format!("Compared with {}, the one in use", now.name), "alm-vs-title"));
    if let (Some(a), Some(b)) = (m.vram, now.vram) {
        let scale = card_gb.unwrap_or(a.max(b)).max(a).max(b);
        for (name, gb, colour) in [(&m.name, a, None), (&now.name, b, Some(kit::GRAPHITE))] {
            let r = widgets::hbox(12);
            let n = widgets::label(name, "alm-vs-name");
            n.set_width_chars(16);
            n.set_max_width_chars(16);
            n.set_ellipsize(gtk::pango::EllipsizeMode::End);
            r.append(&n);
            let bar = kit::stacked_bar(vec![(gb, colour)], scale, 8);
            bar.set_valign(gtk::Align::Center);
            r.append(&bar);
            r.append(&widgets::label(&format!("{gb:.1} GB"), "alm-vs-figure"));
            vs.append(&r);
        }
        let d = a - b;
        if d.abs() >= 0.1 {
            let (icon, class, text) = if d > 0.0 {
                ("go-up-symbolic", "down", format!("Needs {d:.1} GB more graphics memory"))
            } else {
                ("go-down-symbolic", "up", format!("Needs {:.1} GB less graphics memory", -d))
            };
            vs.append(&kit::icon_line(icon, &text, class));
        }
    }
    match (m.vision, now.vision) {
        (true, false) => vs.append(&kit::icon_line("go-up-symbolic", "Can look at the screen, so “what's this?” works", "up")),
        (false, true) => vs.append(&kit::icon_line("go-down-symbolic", "Can't look at the screen, so “what's this?” won't work", "down")),
        _ => {}
    }
    if m.path.is_none() {
        vs.append(&kit::icon_line("download-symbolic", &format!("Needs a {} download first", m.size), ""));
    }
    if vs.observe_children().n_items() > 1 {
        into.append(&vs);
    }
}

pub fn build(page: &Page) {
    let Some(list) = client::json(&["models", "list", "--json"]) else {
        page.banner("Couldn't list the models. Update Nyx Assistant (./install.sh assistant) and try again.", true);
        return;
    };
    let current = list["current"].as_str().unwrap_or("").to_string();
    let models_dir = list["models_dir"].as_str().unwrap_or("").to_string();
    let card_gb = list["vram_gb"].as_f64().filter(|g| *g > 0.0);
    let host = glib::host_name().to_string();

    let recommended: Vec<Model> = list["recommended"].as_array().into_iter().flatten().map(|m| Model { fits: true, ..Model::curated(m) }).collect();
    let curated: Vec<Value> = list["curated"].as_array().cloned().unwrap_or_default();
    // Every language model file on the computer that isn't in the recommended list.
    let mut others: Vec<Model> = list["installed"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| {
            let name = m["name"].as_str().unwrap_or("").to_ascii_lowercase();
            m["path"].is_string()
                && name.ends_with(".gguf")
                && !["mmproj", "drafter", "dflash", "codec", ".encoder.", ".decoder."].iter().any(|part| name.contains(part))
        })
        .filter(|m| !recommended.iter().any(|r| r.path.as_deref() == m["path"].as_str()))
        .map(|m| Model::file(m, curated.iter().find(|c| c["filename"] == m["name"])))
        .collect();
    others.sort_by_key(|m| (!m.in_use(&current), m.name.to_lowercase()));
    let all: Vec<Model> = recommended.iter().chain(others.iter()).cloned().collect();
    let in_use = all.iter().find(|m| m.in_use(&current)).cloned().or_else(|| {
        list["current_model"].as_object().map(|_| Model { path: Some(current.clone()), ..Model::curated(&list["current_model"]) })
    });

    // The choice survives a redraw (a download finishing); otherwise the model in use.
    let chosen = SELECTED
        .with(|s| s.borrow().clone())
        .filter(|k| all.iter().any(|m| m.key == *k))
        .or_else(|| in_use.as_ref().map(|m| m.key.clone()).filter(|k| all.iter().any(|m| m.key == *k)))
        .or_else(|| all.first().map(|m| m.key.clone()));

    let pv = page.split();
    let holder = widgets::vbox(14);
    pv.add(&holder);
    let foot = pv.foot(
        "info-symbolic",
        "Switching restarts the assistant. The graphics card also holds the speech and voice models, and the conversation.",
    );
    let rows: Rows = Rc::default();
    let narrow = Rc::new(std::cell::Cell::new(false));
    let pick: Rc<dyn Fn(&str)> = {
        let (holder, rows, all, in_use, current, narrow) = (holder.clone(), rows.clone(), all.clone(), in_use.clone(), current.clone(), narrow.clone());
        Rc::new(move |key: &str| {
            SELECTED.with(|s| *s.borrow_mut() = Some(key.to_string()));
            for (k, r) in rows.borrow().iter() {
                if k == key {
                    r.add_css_class("ai-picked");
                } else {
                    r.remove_css_class("ai-picked");
                }
            }
            match all.iter().find(|m| m.key == key) {
                Some(m) => render(&holder, m, in_use.as_ref(), &current, card_gb, narrow.get()),
                None => {
                    kit::clear(&holder);
                    holder.append(&kit::empty_card("package-x-generic-symbolic", "No language models here yet", "Download one of the recommended models to start."));
                }
            }
        })
    };

    // ----- The controls: the models that fit, then the others here -----
    let g = page.group(&format!("Recommended for {host}"));
    if let Some(gb) = card_gb {
        g.note(&format!("These fit its {gb:.0} GB graphics card."));
    }
    if recommended.is_empty() {
        g.note("None of the recommended models fits this computer's graphics memory.");
    }
    for m in &recommended {
        g.add(&model_row(m, &m.about, &current, &rows, &pick));
    }

    const SHOWN: usize = 4;
    if !others.is_empty() {
        let g = page.group("Also on this computer");
        for m in others.iter().take(SHOWN) {
            let mut desc = vec![m.path.as_deref().map(|p| folder_words(p, &models_dir)).unwrap_or("").to_string()];
            desc.push(if m.vision { "can see the screen".into() } else { "can't see the screen".into() });
            g.add(&model_row(m, &desc.join(" · "), &current, &rows, &pick));
        }
        if others.len() > SHOWN {
            let rest: Vec<Model> = others.iter().skip(SHOWN).cloned().collect();
            let names: Vec<&str> = rest.iter().take(2).map(|m| m.name.as_str()).collect();
            let more = if rest.len() > 2 { format!("{} and others", names.join(", ")) } else { names.join(" and ") };
            let (pick, current, models_dir) = (pick.clone(), current.clone(), models_dir.clone());
            let (r, _) = widgets::nav_row(None, &format!("{} more", rest.len()), &glib::markup_escape_text(&more), "", move || {
                let window: Rc<RefCell<Option<gtk::Window>>> = Rc::default();
                let (pick, rest, current, models_dir, w) = (pick.clone(), rest.clone(), current.clone(), models_dir.clone(), window.clone());
                let sub = widgets::subpage("Also on this computer", 560, move |page| {
                    let g = page.group("");
                    g.note("Pick one to see it beside the recommended models.");
                    for m in &rest {
                        let desc = format!(
                            "{} · {} · {}",
                            m.size,
                            m.path.as_deref().map(|p| folder_words(p, &models_dir)).unwrap_or(""),
                            if m.vision { "can see the screen" } else { "can't see the screen" }
                        );
                        let control: gtk::Widget = match &m.path {
                            Some(p) if *p == current => kit::pill("In use", "use").upcast(),
                            Some(p) => use_button(p).upcast(),
                            None => gtk::Box::new(gtk::Orientation::Horizontal, 0).upcast(),
                        };
                        let r = widgets::row(&m.name, &glib::markup_escape_text(&desc), Some(&control));
                        r.prepend(&kit::art(&m.name, 30));
                        r.add_css_class("with-tile");
                        r.set_tooltip_text(m.path.as_deref());
                        let (key, pick, w) = (m.key.clone(), pick.clone(), w.clone());
                        kit::on_click(&r, move || {
                            pick(&key);
                            if let Some(w) = w.borrow().as_ref() {
                                w.close();
                            }
                        });
                        g.add(&r);
                    }
                });
                *window.borrow_mut() = Some(sub);
            });
            g.add(&r);
        }
    }

    if crate::capability::program(kit::PROGRAM) {
        let g = page.group("");
        let (r, _) = widgets::nav_row(
            Some("package-x-generic-symbolic"),
            "Every model",
            "Update or remove the models on this computer",
            &all.len().to_string(),
            || window::navigate("local-ai-models"),
        );
        g.add(&r);
        let (r, _) = widgets::nav_row(Some("download-symbolic"), "Get one from Hugging Face", "By its name, like org/model-GGUF", "", || {
            window::navigate("local-ai-models")
        });
        g.add(&r);
    }
    widgets::keywords("model llm gguf download installed available gemma nemotron bolt qwen vision");

    if let Some(key) = &chosen {
        pick(key);
    } else {
        pick("");
    }
    // The comparison and the foot make way for the list in a narrow window.
    {
        let (foot, pick, narrow_now) = (foot.clone(), pick.clone(), narrow.clone());
        kit::on_narrow(&page.body, move |narrow| {
            narrow_now.set(narrow);
            if let Some(f) = foot.parent() {
                f.set_visible(!narrow);
            }
            if let Some(key) = SELECTED.with(|s| s.borrow().clone()) {
                pick(&key);
            }
        });
    }
}
