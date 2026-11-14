//! Local AI models: the library (one search box that also takes a Hugging Face name to
//! download, filters for what models do, a row per model) and the picked model's card in
//! the preview, with its update and Remove. Downloads belong to Nyx AI Core and go on if
//! Settings closes.

use super::{art, call, capabilities, change, chip, chips, has, model_name, size};
use crate::widgets::{self, Page};
use crate::window;
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::BufRead;
use std::rc::Rc;

const PAGE: &str = "local-ai-models";

thread_local! {
    /// Downloads this window started: model -> (done bytes, total bytes, status).
    static DOWNLOADS: RefCell<BTreeMap<String, (u64, u64, String)>> = RefCell::default();
    static BARS: RefCell<Vec<(String, glib::WeakRef<gtk::ProgressBar>)>> = RefCell::default();
    /// The model picked in the library, and the filter it's showing.
    static SELECTED: RefCell<Option<String>> = RefCell::default();
    static FILTER: RefCell<String> = RefCell::new("all".into());
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

fn progress_bar(model: &str, width: i32) -> gtk::ProgressBar {
    let bar = gtk::ProgressBar::new();
    bar.set_show_text(true);
    bar.set_width_request(width);
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

fn icon_button(icon: &str, label: &str, action: impl Fn(&gtk::Button) + 'static) -> gtk::Button {
    let b = gtk::Button::new();
    let c = widgets::hbox(6);
    c.append(&gtk::Image::from_icon_name(icon));
    c.append(&gtk::Label::new(Some(label)));
    b.set_child(Some(&c));
    b.set_valign(gtk::Align::Center);
    b.connect_clicked(action);
    b
}

/// The apps that use a model by default, by role.
const ROLES: [(&str, &str, &str, &str); 4] = [
    ("assistant", "Assistant", "settings-sparkle-symbolic", ""),
    ("chat", "Chat", "chat-symbolic", ""),
    ("embedding", "Search and knowledge", "system-search-symbolic", ""),
    ("coding", "Coding tools", "code-symbolic", "when a tool names no model"),
];

/// What a model is good at, in words, with an icon each.
fn good_at(m: &Value) -> Vec<(&'static str, &'static str)> {
    m["capabilities"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| match c.as_str()? {
            "chat" => Some(("chat-symbolic", "Chat")),
            "tools" => Some(("applications-engineering-symbolic", "Using tools")),
            "thinking" => Some(("starred-symbolic", "Thinking it through")),
            "vision" => Some(("view-reveal-symbolic", "Sees images")),
            "audio" => Some(("audio-input-microphone-symbolic", "Hears audio")),
            "embedding" => Some(("system-search-symbolic", "Finds text by meaning")),
            "draft" => Some(("power-profile-performance-symbolic", "Speeds up another model")),
            _ => None,
        })
        .collect()
}

/// When the model's file arrived, from the file itself.
fn downloaded_on(m: &Value) -> Option<String> {
    let modified = std::fs::metadata(m["path"].as_str()?).ok()?.modified().ok()?;
    let secs = modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    let t = glib::DateTime::from_unix_local(secs).ok()?;
    t.format("%e %B %Y").ok().map(|s| s.trim().to_string())
}

/// Where a model came from: its Hugging Face repository, or the folder it's in.
fn origin(m: &Value) -> String {
    let id = m["id"].as_str().unwrap_or("");
    if m["source"].as_str() == Some("folder") {
        let home = glib::home_dir().to_string_lossy().to_string();
        let folder = m["path"].as_str().and_then(|p| p.rsplit_once('/')).map(|(d, _)| d.replacen(&home, "~", 1)).unwrap_or_default();
        return format!("In a model folder · {folder}");
    }
    format!("From Hugging Face · {}", id.split(':').next().unwrap_or(id))
}

/// What the filter chips sort by.
fn matches(filter: &str, m: &Value, used: bool) -> bool {
    match filter {
        "chat" => has(m, "chat"),
        "vision" => has(m, "vision"),
        "embedding" => has(m, "embedding"),
        "used" => used,
        _ => true,
    }
}

/// A short fact in the card's grid.
fn fact(key: &str, value: &str) -> gtk::Box {
    let b = widgets::vbox(2);
    b.add_css_class("lmx-fact");
    b.set_hexpand(true);
    b.append(&widgets::label(key, "lmx-fact-key"));
    let v = widgets::label(value, "lmx-fact-value");
    v.set_wrap(true);
    v.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    b.append(&v);
    b
}

/// The picked model in the preview: where it's from, what it is, what uses it, its update.
#[allow(clippy::too_many_arguments)]
fn render(
    pv_head: &gtk::Box,
    into: &gtk::Box,
    m: &Value,
    used_by: &[(&'static str, &'static str, &'static str)],
    kept_by: &[String],
    running: &BTreeMap<String, (u64, u64)>,
    narrow: bool,
) {
    let id = m["id"].as_str().unwrap_or("").to_string();
    let name = model_name(m);
    let state = m["state"].as_str().unwrap_or("");

    // The heading: its name, when it came and what it's doing, and Remove.
    super::clear(pv_head);
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(&name, "preview-title");
    t.set_wrap(true);
    t.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    text.append(&t);
    let mut line = Vec::new();
    if let Some(on) = downloaded_on(m) {
        line.push(format!("Downloaded {on}"));
    }
    match state {
        "ready" | "busy" => line.push("loaded now".into()),
        "loading" => line.push("loading".into()),
        "failed" => line.push(format!("can't load: {}", m["error"].as_str().unwrap_or("unknown"))),
        _ => {}
    }
    if !line.is_empty() {
        let mut s = line.join(" · ");
        if let Some(f) = s.get_mut(0..1) {
            f.make_ascii_uppercase();
        }
        let l = widgets::label(&s, "preview-line");
        l.set_wrap(true);
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        text.append(&l);
    }
    pv_head.append(&text);
    if m["removable"] == true && !matches!(state, "busy" | "loading") {
        let target = id.clone();
        let remove = widgets::two_click("Remove", "Click again to remove", move || {
            change(vec!["rm".into(), target.clone()], PAGE, "Model removed");
        });
        remove.set_valign(gtk::Align::Center);
        pv_head.append(&remove);
    } else if m["source"].as_str() == Some("folder") {
        let lock = gtk::Image::from_icon_name("changes-prevent-symbolic");
        lock.set_tooltip_text(Some("It's in another app's folder, so Settings doesn't delete it."));
        lock.add_css_class("lmx-lock");
        lock.set_valign(gtk::Align::Center);
        pv_head.append(&lock);
    }

    super::clear(into);
    let card = widgets::vbox(14);
    card.add_css_class("preview-card");
    card.add_css_class("lmx-card");
    let head = widgets::hbox(14);
    head.append(&art(&name, if narrow { 44 } else { 52 }));
    let text = widgets::vbox(3);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let n = widgets::label(&name, "lmx-name");
    n.set_wrap(true);
    text.append(&n);
    let o = widgets::label(&origin(m), "lmx-origin");
    o.set_wrap(true);
    o.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    o.set_selectable(true);
    o.set_can_focus(false);
    text.append(&o);
    head.append(&text);
    card.append(&head);

    let grid = gtk::Grid::new();
    grid.add_css_class("lmx-facts");
    grid.set_column_homogeneous(true);
    let mut facts = vec![fact("On disk", &size(m["size"].as_u64().unwrap_or(0)))];
    if let Some(p) = m["parameters"].as_str() {
        facts.push(fact("Size", &super::parameters(p)));
    }
    if let Some(q) = m["quantization"].as_str() {
        facts.push(fact("Quality", &super::quality(q)));
    }
    if let Some(ctx) = m["context_length"].as_u64() {
        facts.push(fact("Longest conversation", &format!("{} tokens", super::thousands(ctx))));
    }
    for (i, f) in facts.iter().enumerate() {
        if i % 2 == 1 {
            f.add_css_class("right");
        }
        if i >= 2 {
            f.add_css_class("lower");
        }
        grid.attach(f, (i % 2) as i32, (i / 2) as i32, 1, 1);
    }
    card.append(&grid);

    let good = good_at(m);
    if !good.is_empty() {
        let row = widgets::hbox(12);
        let l = widgets::label("Good at", "lmx-key");
        l.set_valign(gtk::Align::Start);
        l.set_width_chars(8);
        row.append(&l);
        let c = chips(good.into_iter().map(|(icon, text)| chip(icon, text, "")).collect());
        c.set_hexpand(true);
        row.append(&c);
        card.append(&row);
    }
    let row = widgets::hbox(12);
    let l = widgets::label("Used by", "lmx-key");
    l.set_valign(gtk::Align::Start);
    l.set_width_chars(8);
    row.append(&l);
    let mut users: Vec<gtk::Box> = used_by
        .iter()
        .map(|(icon, who, when)| {
            let c = chip(icon, who, "on");
            if !when.is_empty() {
                c.append(&widgets::label(when, "lmx-when"));
            }
            c
        })
        .collect();
    for who in kept_by {
        users.push(chip("view-pin-symbolic", &format!("Kept loaded by {who}"), "on"));
    }
    if users.is_empty() {
        let none = widgets::label("No app uses it by default.", "lmx-none");
        none.set_hexpand(true);
        row.append(&none);
    } else {
        let c = chips(users);
        c.set_hexpand(true);
        row.append(&c);
    }
    card.append(&row);
    into.append(&card);

    // An update: offered, or on its way.
    let downloading = DOWNLOADS.with(|d| d.borrow().contains_key(&id));
    let updating = running.get(&id);
    if m["update_available"] == true || downloading || updating.is_some() {
        let upd = widgets::hbox(12);
        upd.add_css_class("preview-card");
        upd.add_css_class("lmx-update");
        let i = gtk::Image::from_icon_name("download-symbolic");
        i.set_valign(gtk::Align::Center);
        upd.append(&i);
        let text = widgets::vbox(2);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.append(&widgets::label("A newer version is out", "lmx-update-title"));
        let line = match m["update_size"].as_u64() {
            Some(bytes) if bytes > 0 => format!("{} · replaces this one when it's done", size(bytes)),
            _ => "It replaces this one when it's done".to_string(),
        };
        let l = widgets::label(&line, "lmx-update-line");
        l.set_wrap(true);
        text.append(&l);
        upd.append(&text);
        if downloading {
            upd.append(&progress_bar(&id, 160));
        } else if let Some((done, total)) = updating {
            let text = if *total > 0 { format!("Updating · {} of {}", size(*done), size(*total)) } else { "Updating".to_string() };
            upd.append(&widgets::label(&text, "dim"));
        } else {
            let target = id.clone();
            let b = button("Update", move |_| download(target.clone()));
            b.add_css_class("suggested-action");
            upd.append(&b);
        }
        into.append(&upd);
    }
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
    let settings = call(&["service"]).unwrap_or(Value::Null);
    let status = call(&["status"]).unwrap_or(Value::Null);
    super::watch(page, PAGE, super::signature(&status, &settings));
    let loaded: Vec<Value> = status["resources"]["loaded"].as_array().cloned().unwrap_or_default();
    let installed: Vec<Value> = models.iter().filter(|m| m["state"].as_str() != Some("missing")).cloned().collect();
    let used_by = |id: &str| -> Vec<(&'static str, &'static str, &'static str)> {
        ROLES.iter().filter(|(role, ..)| settings["defaults"][*role].as_str() == Some(id)).map(|(_, who, icon, when)| (*icon, *who, *when)).collect()
    };
    let kept_by = |id: &str| -> Vec<String> {
        loaded
            .iter()
            .filter(|l| l["id"].as_str() == Some(id))
            .flat_map(|l| l["pinned_by"].as_array().cloned().unwrap_or_default())
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    };
    let is_loaded = |m: &Value| matches!(m["state"].as_str(), Some("ready" | "busy" | "loading"));

    // The picked model: kept across redraws; otherwise one in use, else the first.
    let chosen = SELECTED
        .with(|s| s.borrow().clone())
        .filter(|k| installed.iter().any(|m| m["id"].as_str() == Some(k)))
        .or_else(|| installed.iter().find(|m| is_loaded(m)).and_then(|m| m["id"].as_str().map(str::to_string)))
        .or_else(|| {
            ROLES.iter().find_map(|(role, ..)| {
                settings["defaults"][*role].as_str().filter(|id| installed.iter().any(|m| m["id"].as_str() == Some(id))).map(str::to_string)
            })
        })
        .or_else(|| installed.first().and_then(|m| m["id"].as_str().map(str::to_string)));

    // ----- The preview: the picked model -----
    let pv = page.split();
    let pv_head = pv.head("", "", &[]);
    let holder = widgets::vbox(14);
    pv.add(&holder);
    let rows: Rc<RefCell<Vec<(String, gtk::Box)>>> = Rc::default();
    let narrow = Rc::new(std::cell::Cell::new(false));
    let pick: Rc<dyn Fn(&str)> = {
        let (pv_head, holder, rows, installed, running, narrow) = (pv_head.clone(), holder.clone(), rows.clone(), installed.clone(), running.clone(), narrow.clone());
        let used_by: Vec<(String, Vec<(&'static str, &'static str, &'static str)>)> =
            installed.iter().filter_map(|m| m["id"].as_str().map(|id| (id.to_string(), used_by(id)))).collect();
        let kept_by: Vec<(String, Vec<String>)> = installed.iter().filter_map(|m| m["id"].as_str().map(|id| (id.to_string(), kept_by(id)))).collect();
        Rc::new(move |key: &str| {
            SELECTED.with(|s| *s.borrow_mut() = Some(key.to_string()));
            for (k, r) in rows.borrow().iter() {
                if k == key {
                    r.add_css_class("ai-picked");
                } else {
                    r.remove_css_class("ai-picked");
                }
            }
            match installed.iter().find(|m| m["id"].as_str() == Some(key)) {
                Some(m) => {
                    let users = used_by.iter().find(|(id, _)| id == key).map(|(_, u)| u.clone()).unwrap_or_default();
                    let kept = kept_by.iter().find(|(id, _)| id == key).map(|(_, k)| k.clone()).unwrap_or_default();
                    render(&pv_head, &holder, m, &users, &kept, &running, narrow.get());
                }
                None => {
                    super::clear(&pv_head);
                    pv_head.append(&widgets::label("Models", "preview-title"));
                    super::clear(&holder);
                    holder.append(&super::empty_card(
                        "package-x-generic-symbolic",
                        "No models yet",
                        "Paste a Hugging Face name in the box, like org/model-GGUF:Q4_K_M, or get one from Chat's Marketplace.",
                    ));
                }
            }
        })
    };

    // ----- The controls: the library -----
    let g = page.plain_group("");
    let top = widgets::hbox(6);
    top.add_css_class("lmx-top");
    let entry = gtk::SearchEntry::new();
    entry.set_tooltip_text(Some("Search the models here, or paste a Hugging Face name like org/model-GGUF:Q4_K_M to download it"));
    entry.set_hexpand(true);
    top.append(&entry);
    let check = icon_button("view-refresh-symbolic", "Check for updates", |b| {
        b.set_sensitive(false);
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
    top.append(&check);
    g.add(&top);

    // Filters, with how many models each shows.
    let used_any = |m: &Value| {
        let id = m["id"].as_str().unwrap_or("");
        is_loaded(m) || !used_by(id).is_empty()
    };
    let filters = [("all", "All"), ("chat", "Chat"), ("vision", "Seeing"), ("embedding", "Search"), ("used", "In use")];
    let chip_row = widgets::hbox(6);
    chip_row.add_css_class("lmx-chips");
    let current_filter = FILTER.with(|f| f.borrow().clone());
    let mut group: Option<gtk::ToggleButton> = None;
    let filter_buttons: Rc<RefCell<Vec<gtk::ToggleButton>>> = Rc::default();
    for (id, label) in filters {
        let count = installed.iter().filter(|m| matches(id, m, used_any(m))).count();
        if count == 0 && id != "all" {
            continue;
        }
        let b = gtk::ToggleButton::new();
        b.add_css_class("lmx-chip");
        let c = widgets::hbox(5);
        c.append(&gtk::Label::new(Some(label)));
        c.append(&widgets::label(&count.to_string(), "lmx-count"));
        b.set_child(Some(&c));
        if let Some(f) = &group {
            b.set_group(Some(f));
        } else {
            group = Some(b.clone());
        }
        b.set_active(id == current_filter);
        b.set_widget_name(id);
        chip_row.append(&b);
        filter_buttons.borrow_mut().push(b);
    }
    if !filter_buttons.borrow().iter().any(|b| b.is_active())
        && let Some(first) = filter_buttons.borrow().first()
    {
        first.set_active(true);
        FILTER.with(|f| *f.borrow_mut() = "all".into());
    }
    g.add(&chip_row);

    let g = page.group("");
    g.wrapper.add_css_class("lmx-list");
    // A name pasted in the box offers to download it, at the top of the list.
    let offer_title = widgets::label("", "settings-option-title");
    let offer_desc = widgets::label("", "settings-option-description");
    offer_desc.set_wrap(true);
    let offer_button = button("Download", |_| {});
    let offer = widgets::hbox(12);
    offer.add_css_class("settings-option");
    offer.add_css_class("card-row");
    offer.add_css_class("with-tile");
    offer.add_css_class("lmx-offer");
    offer.append(&widgets::tile("download-symbolic"));
    let offer_text = widgets::vbox(2);
    offer_text.set_hexpand(true);
    offer_text.set_valign(gtk::Align::Center);
    offer_text.append(&offer_title);
    offer_text.append(&offer_desc);
    offer.append(&offer_text);
    offer.append(&offer_button);
    offer.set_visible(false);
    g.add(&offer);
    for model in DOWNLOADS.with(|d| d.borrow().keys().cloned().collect::<Vec<_>>()) {
        if !models.iter().any(|m| m["id"].as_str() == Some(&model)) {
            let r = widgets::row(&model, "Downloading", Some(progress_bar(&model, 140).upcast_ref()));
            r.prepend(&widgets::tile("download-symbolic"));
            r.add_css_class("with-tile");
            g.add(&r);
        }
    }
    let mut library: Vec<(gtk::Box, String, String)> = Vec::new();
    for m in &installed {
        let id = m["id"].as_str().unwrap_or("").to_string();
        let name = super::distinct_name(m, &installed);
        let mut bits = vec![size(m["size"].as_u64().unwrap_or(0))];
        let caps = capabilities(m);
        if !caps.is_empty() {
            bits.push(caps.iter().take(3).copied().collect::<Vec<_>>().join(", "));
        }
        let state = m["state"].as_str().unwrap_or("");
        if state == "failed" {
            bits.push(format!("can't load: {}", m["error"].as_str().unwrap_or("unknown")));
        }
        if DOWNLOADS.with(|d| d.borrow().contains_key(&id)) || running.contains_key(&id) {
            bits.push("updating".into());
        }
        let users: Vec<&str> = used_by(&id).iter().map(|(_, who, _)| if *who == "Search and knowledge" { "Search" } else { *who }).collect();
        let end: gtk::Widget = if DOWNLOADS.with(|d| d.borrow().contains_key(&id)) {
            progress_bar(&id, 120).upcast()
        } else if let Some((done, total)) = running.get(&id) {
            let text = if *total > 0 { format!("{} of {}", size(*done), size(*total)) } else { "Updating".to_string() };
            widgets::label(&text, "row-value").upcast()
        } else if m["update_available"] == true {
            let p = super::pill("Update", "use");
            p.add_css_class("lmx-update-pill");
            p.upcast()
        } else if !users.is_empty() {
            let l = widgets::label(&users.join(", "), "row-value");
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_max_width_chars(18);
            l.set_tooltip_text(Some(&users.join(", ")));
            l.upcast()
        } else if m["source"].as_str() == Some("folder") {
            let i = gtk::Image::from_icon_name("folder-symbolic");
            i.add_css_class("lmx-folder");
            i.set_tooltip_text(Some("In another app's folder; Settings doesn't delete it."));
            i.upcast()
        } else {
            gtk::Box::new(gtk::Orientation::Horizontal, 0).upcast()
        };
        let r = widgets::row(&name, &glib::markup_escape_text(&bits.join(" · ")), Some(&end));
        if is_loaded(m) {
            // A green dot after the name: it's in memory.
            widgets::tag_row(&r, "");
            if let Some(tag) = find_tag(&r) {
                tag.remove_css_class("tag");
                tag.add_css_class("lmx-loaded");
                tag.set_tooltip_text(Some("Loaded now"));
            }
        }
        r.prepend(&art(&model_name(m), 30));
        r.add_css_class("with-tile");
        r.add_css_class("lmx-row");
        r.set_tooltip_text(Some(&id));
        let (key, pick) = (id.clone(), pick.clone());
        super::on_click(&r, move || pick(&key));
        rows.borrow_mut().push((id.clone(), r.clone()));
        g.add(&r);
        let search = format!("{name} {id} {}", caps.join(" ")).to_lowercase();
        let tags: Vec<&str> = filters.iter().map(|(f, _)| *f).filter(|f| matches(f, m, used_any(m))).collect();
        library.push((r, search, tags.join(" ")));
    }
    let empty = widgets::label("No model matches.", "lmx-none");
    empty.add_css_class("lmx-empty");
    empty.set_visible(false);
    g.add(&empty);
    if installed.is_empty() {
        g.note("Nothing installed yet. Paste a Hugging Face name above, or get one from Chat's Marketplace.");
    }

    // Show the rows the search and the filter leave.
    let library = Rc::new(library);
    let refilter = {
        let (library, entry, empty, offer, offer_title, offer_desc, offer_button) =
            (library.clone(), entry.clone(), empty.clone(), offer.clone(), offer_title.clone(), offer_desc.clone(), offer_button.clone());
        let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
        move || {
            let text = entry.text().trim().to_string();
            let query = text.to_lowercase();
            let filter = FILTER.with(|f| f.borrow().clone());
            // A Hugging Face name: "org/repo" is a name, not a search.
            let name = text.contains('/');
            let mut shown = 0;
            for (row, search, tags) in library.iter() {
                let visible = (filter == "all" || tags.split(' ').any(|t| t == filter)) && (name || query.is_empty() || search.contains(&query));
                row.set_visible(visible);
                shown += visible as usize;
            }
            empty.set_visible(shown == 0 && !library.is_empty() && !name);
            offer.set_visible(name);
            if name {
                let ready = text.contains(':');
                offer_title.set_text(&format!("Download {text}"));
                offer_desc.set_text(if ready {
                    "From Hugging Face. Its vision projector and draft head come along when it has them."
                } else {
                    "Add the quantization after a colon, like :Q4_K_M."
                });
                offer_button.set_sensitive(ready);
                if let Some(h) = handler.borrow_mut().take() {
                    offer_button.disconnect(h);
                }
                let (entry, target) = (entry.clone(), text.clone());
                *handler.borrow_mut() = Some(offer_button.connect_clicked(move |_| {
                    entry.set_text("");
                    download(target.clone());
                }));
            }
        }
    };
    let rf = refilter.clone();
    entry.connect_search_changed(move |_| rf());
    entry.connect_activate(move |e| {
        let name = e.text().trim().to_string();
        if !name.contains('/') {
            return;
        }
        if name.contains(':') {
            e.set_text("");
            download(name);
        } else {
            window::toast("Name a Hugging Face GGUF repository and quantization, like org/repo-GGUF:Q4_K_M");
        }
    });
    for b in filter_buttons.borrow().iter() {
        let rf = refilter.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                FILTER.with(|f| *f.borrow_mut() = b.widget_name().to_string());
                rf();
            }
        });
    }
    refilter();

    // What it all takes on the disk.
    let total: u64 = installed.iter().map(|m| m["size"].as_u64().unwrap_or(0)).sum();
    let updates = installed.iter().filter(|m| m["update_available"] == true).count();
    let mut summary = vec![format!("{} model{} · {}", installed.len(), if installed.len() == 1 { "" } else { "s" }, size(total))];
    if updates > 0 {
        summary.push(format!("{updates} with {}", if updates == 1 { "an update" } else { "updates" }));
    }
    if let Some(free) = free_space() {
        summary.push(format!("{} free on the disk", size(free)));
    }
    let g = page.plain_group("");
    let sum = super::icon_line("drive-harddisk-symbolic", &summary.join(" · "), "lmx-sum");
    g.add(&sum);
    widgets::keywords("models on this computer check for updates");
    g.add(&super::icon_line("info-symbolic", "Models in another app's folder are listed too, but Settings never deletes them.", "lmx-sum"));
    widgets::keywords("ai model models llm gguf hugging face download remove update llama local from hugging face");

    if let Some(key) = &chosen {
        pick(key);
    } else {
        pick("");
    }
    {
        let (pick, narrow_now, entry) = (pick.clone(), narrow.clone(), entry.clone());
        super::on_narrow(&page.body, move |narrow| {
            narrow_now.set(narrow);
            entry.set_placeholder_text(Some(if narrow { "Search, or paste a Hugging Face name" } else { "Search or paste a name" }));
            if let Some(key) = SELECTED.with(|s| s.borrow().clone()) {
                pick(&key);
            }
        });
    }
}

/// The tag [`widgets::tag_row`] put after a row's title.
fn find_tag(row: &gtk::Box) -> Option<gtk::Label> {
    fn walk(w: &gtk::Widget) -> Option<gtk::Label> {
        if let Some(l) = w.downcast_ref::<gtk::Label>()
            && l.has_css_class("tag")
        {
            return Some(l.clone());
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            if let Some(found) = walk(&child) {
                return Some(found);
            }
            c = child.next_sibling();
        }
        None
    }
    walk(row.upcast_ref())
}

/// Free space where Hugging Face downloads go.
fn free_space() -> Option<u64> {
    let home = glib::home_dir();
    let cache = std::env::var_os("HF_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".cache"));
    let dir = if cache.exists() { cache } else { home };
    let out = crate::cmd::output(&["df", "-B1", "--output=avail", &dir.to_string_lossy()])?;
    out.lines().nth(1)?.trim().parse().ok()
}
