//! What the assistant may do: allowed, asks first, or never.

use super::{client, save, words};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;

const HELPER: &str = "/usr/local/lib/nyx-assistant/nyx-assistant-helper";
const HELPER_INSTALL: &str = "sudo nyx-assistant install-helper";

const CAPABILITIES: &[(&str, &str, &str)] = &[
    ("permissions.launch_app", "Open apps", ""),
    ("permissions.volume", "Change the volume", ""),
    ("permissions.files_read", "Read files", ""),
    ("permissions.files_write", "Change files", "Create, edit, move and delete."),
    ("permissions.shutdown", "Shut down and restart", ""),
];

fn decisions(ask_only: bool) -> Vec<(String, String)> {
    let mut v = vec![];
    if !ask_only {
        v.push(("allow".to_string(), "Allowed".to_string()));
    }
    v.push(("confirm".into(), "Asks first".into()));
    v.push(("deny".into(), "Never".into()));
    v
}

/// The config spells "asks first" as confirm or ask.
fn decision(cfg: &serde_json::Value, key: &str) -> String {
    match client::get_str(cfg, key).as_str() {
        "ask" => "confirm".into(),
        d => d.into(),
    }
}

fn rule_row(tool: String, current: &str) -> gtk::Box {
    let dd = widgets::dropdown(&decisions(false), current);
    let key = format!("mcp.permissions.{tool}");
    let opts = decisions(false);
    dd.connect_selected_notify(move |d| {
        if let Some((id, _)) = opts.get(d.selected() as usize) {
            save(&key, client::quoted(id));
        }
    });
    let remove = gtk::Button::from_icon_name("user-trash-symbolic");
    remove.add_css_class("flat");
    remove.set_tooltip_text(Some("Remove this rule"));
    let t = tool.clone();
    remove.connect_clicked(move |_| {
        let key = format!("mcp.permissions.{t}");
        cmd::background(
            move || client::run(&["config", "unset", &key]),
            |r| {
                if let Err(e) = r {
                    window::toast(&format!("Couldn't remove the rule: {e}"));
                }
                window::rebuild("assistant-permissions");
            },
        );
    });
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    controls.append(&dd);
    controls.append(&remove);
    widgets::row(&tool, "", Some(controls.upcast_ref()))
}

/// Pick a tool and a decision, then Add.
fn add_rule_form(tools: Vec<String>) -> gtk::Box {
    let form = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let names: Vec<(String, String)> = tools.iter().map(|t| (t.clone(), t.clone())).collect();
    let tool = widgets::dropdown(&names, "");
    tool.set_enable_search(true);
    tool.set_hexpand(true);
    let choice = widgets::dropdown(&decisions(false), "confirm");
    let add = gtk::Button::with_label("Add");
    add.add_css_class("suggested-action");
    let opts = decisions(false);
    let (t, c) = (tool.clone(), choice.clone());
    add.connect_clicked(move |_| {
        let (Some(name), Some((d, _))) = (tools.get(t.selected() as usize).cloned(), opts.get(c.selected() as usize).cloned())
        else {
            window::toast("Pick a tool first.");
            return;
        };
        let key = format!("mcp.permissions.{name}");
        cmd::background(
            move || client::set(&key, &client::quoted(&d)),
            |r| {
                if let Err(e) = r {
                    window::toast(&format!("Couldn't add the rule: {e}"));
                }
                window::rebuild("assistant-permissions");
            },
        );
    });
    form.append(&tool);
    form.append(&choice);
    form.append(&add);
    form
}

pub fn build(page: &Page) {
    let Some(cfg) = super::config_or_banner(page) else { return };

    let g = page.group("What it may do");
    for &(key, title, desc) in CAPABILITIES {
        let (r, _) =
            widgets::choice_row(title, desc, decisions(false), &decision(&cfg, key), move |id| save(key, client::quoted(&id)));
        g.add(&r);
    }
    let (r, _) = widgets::choice_row(
        "Install, remove and update software",
        "Also system services. Always asks for your password.",
        decisions(true),
        &decision(&cfg, "permissions.admin"),
        |id| save("permissions.admin", client::quoted(&id)),
    );
    g.add(&r);
    g.note("Commands the assistant writes itself always ask first.");
    widgets::keywords("permission allow ask confirm deny files shutdown admin sudo install");

    let g = page.group("Rules for single tools");
    g.note("A rule for one tool wins over the general setting above.");
    let rules = client::get(&cfg, "mcp.permissions").as_object().cloned().unwrap_or_default();
    for (tool, d) in rules {
        let d = match d.as_str().unwrap_or("confirm") {
            "ask" => "confirm",
            d => d,
        };
        g.add(&rule_row(tool, d));
    }
    let tools: Vec<String> = client::status()
        .and_then(|s| s["mcp_tools"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();
    if tools.is_empty() {
        g.note("Start the assistant to add rules for its tools.");
    } else {
        g.add(&widgets::form_disclosure("Add a rule", "", &add_rule_form(tools)));
    }
    widgets::keywords("tool rule mcp");

    let g = page.group("Capability health");
    if let Some(status) = client::status() {
        let (search_row, search) = widgets::entry_row(
            "Find capability",
            "Search registered tools and current permission reasons.",
            "",
            "Search tools",
            |_| {},
        );
        g.add(&search_row);
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        let mut rows = Vec::new();
        for item in status["capability_health"].as_array().into_iter().flatten() {
            let name = item["name"].as_str().unwrap_or("");
            let permission = item["permission"].as_str().unwrap_or("unknown");
            let reason = item["reason"].as_str().unwrap_or("");
            let row = widgets::row(name, &format!("{permission} · {reason}"), None);
            list.append(&row);
            rows.push((name.to_lowercase(), row));
        }
        search.connect_changed(move |entry| {
            let query = entry.text().to_lowercase();
            for (name, row) in &rows {
                row.set_visible(query.is_empty() || name.contains(&query));
            }
        });
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_max_content_height(360);
        scroll.set_propagate_natural_height(true);
        scroll.set_child(Some(&list));
        g.add(&scroll);
        for error in status["mcp_errors"].as_array().into_iter().flatten() {
            g.note(&gtk::glib::markup_escape_text(&format!("Unavailable: {error}")));
        }
    } else {
        g.note("Start the assistant to inspect current capability health.");
    }

    let g = page.group("Using apps for you");
    g.add(&words(
        &cfg,
        "computer.deny_classes",
        "Never touch these apps",
        "Window classes, comma-separated. Password managers, terminals and lock screens are never touched anyway.",
        "e.g. keepassxc",
    ));
    g.add(&words(
        &cfg,
        "computer.confirm_words",
        "Always ask before clicking",
        "Buttons with these words ask first, even during an approved task.",
        "send, buy, delete",
    ));
    widgets::keywords("computer use click type window");

    let g = page.group("Administrator helper");
    if std::path::Path::new(HELPER).exists() {
        let (r, _) = widgets::info_row("Helper", "Installed");
        g.add(&r);
    } else {
        let (r, _) = widgets::button_row(
            "Not installed",
            &format!("Software and service changes need it. Run <tt>{HELPER_INSTALL}</tt> in a terminal."),
            "Copy",
            |b| {
                b.clipboard().set_text(HELPER_INSTALL);
                window::toast("Copied the command.");
            },
        );
        g.add(&r);
    }
}
