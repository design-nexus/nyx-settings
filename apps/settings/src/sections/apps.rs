//! Default apps: the browser, email and files apps and the tools on the left, and on the
//! right a map from every kind of file to the app it opens in. Clicking a kind lists the
//! apps that can open it.

use crate::platform::{PLATFORM, Platform};
use crate::sections::accounts::{Link, Links, head, linked, roomy};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::gio;
use gtk::prelude::*;
use nyx_ui::default_apps::{self as defaults, CATEGORIES, Category, WEB_BROWSER};
use std::rc::Rc;

fn installed(command: &str) -> bool {
    cmd::present(command)
}

/// Every app's desktop ID, program and icon, to put faces to app names.
struct Icons(Vec<(String, String, Option<gio::Icon>)>);

impl Icons {
    fn load() -> Icons {
        Icons(
            gio::AppInfo::all()
                .into_iter()
                .filter_map(|a| {
                    let exe = a
                        .commandline()
                        .and_then(|c| c.to_string_lossy().split_whitespace().next().map(str::to_string))
                        .map(|c| c.rsplit('/').next().unwrap_or(&c).to_string())
                        .unwrap_or_default();
                    Some((a.id()?.to_string(), exe, a.icon()))
                })
                .collect(),
        )
    }

    /// The icon of the app with this desktop ID, or whose program this is.
    fn find(&self, key: &str) -> Option<gio::Icon> {
        self.0
            .iter()
            .find(|(id, _, _)| id == key)
            .or_else(|| self.0.iter().find(|(_, exe, _)| !exe.is_empty() && exe == key))
            .or_else(|| self.0.iter().find(|(id, _, _)| id.trim_end_matches(".desktop").rsplit('.').next() == Some(key)))
            .and_then(|(_, _, icon)| icon.clone())
    }
}

/// A drop-down whose choices show their app's icon.
fn app_dropdown(options: &[(String, String)], current: &str, icons: &[Option<gio::Icon>]) -> gtk::DropDown {
    let labels: Vec<&str> = options.iter().map(|(_, l)| l.as_str()).collect();
    let dd = gtk::DropDown::from_strings(&labels);
    let icons: Rc<Vec<Option<gio::Icon>>> = Rc::new(icons.to_vec());
    for short in [true, false] {
        let f = gtk::SignalListItemFactory::new();
        f.connect_setup(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
            let row = widgets::hbox(8);
            let image = gtk::Image::new();
            image.set_pixel_size(16);
            row.append(&image);
            let l = gtk::Label::new(None);
            l.set_xalign(0.0);
            if short {
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                l.set_max_width_chars(11);
            } else {
                l.set_hexpand(true);
            }
            row.append(&l);
            item.set_child(Some(&row));
        });
        let icons = icons.clone();
        f.connect_bind(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
            let Some(s) = item.item().and_downcast::<gtk::StringObject>() else { return };
            let Some(row) = item.child() else { return };
            if let Some(image) = row.first_child().and_downcast::<gtk::Image>() {
                match icons.get(item.position() as usize).cloned().flatten() {
                    Some(icon) => image.set_from_gicon(&icon),
                    None => image.set_icon_name(Some("application-x-executable-symbolic")),
                }
            }
            if let Some(l) = row.last_child().and_downcast::<gtk::Label>() {
                l.set_text(&s.string());
            }
        });
        if short {
            dd.set_factory(Some(&f));
        } else {
            dd.set_list_factory(Some(&f));
        }
    }
    match options.iter().position(|(id, _)| id == current) {
        Some(i) => dd.set_selected(i as u32),
        None => dd.set_selected(gtk::INVALID_LIST_POSITION),
    }
    dd.add_css_class("dfa-dropdown");
    dd
}

/// A row with a tile and an app drop-down; `pick` gets the chosen ID.
fn app_row(
    icon: &str,
    title: &str,
    desc: &str,
    options: Vec<(String, String)>,
    current: &str,
    icons: &[Option<gio::Icon>],
    pick: impl Fn(String) + 'static,
) -> gtk::Box {
    let dd = app_dropdown(&options, current, icons);
    let current = current.to_string();
    dd.connect_selected_notify(move |d| {
        if let Some((id, _)) = options.get(d.selected() as usize)
            && *id != current
        {
            pick(id.clone());
        }
    });
    widgets::icon_row(icon, title, desc, Some(dd.upcast_ref()))
}

/// A choice set by one of Omarchy's `omarchy-default-*` commands.
fn default_row(
    icons: &Icons,
    icon: &str,
    title: &str,
    desc: &str,
    setter: &'static str,
    choices: &[(&str, &str, &str)], // (id, label, command that must exist)
) -> gtk::Box {
    let current = cmd::output(&[setter]).unwrap_or_default().trim().to_string();
    let shown: Vec<&(&str, &str, &str)> = choices.iter().filter(|(id, _, bin)| installed(bin) || *id == current).collect();
    let options: Vec<(String, String)> = shown.iter().map(|(id, label, _)| (id.to_string(), label.to_string())).collect();
    let faces: Vec<Option<gio::Icon>> = shown.iter().map(|(id, _, bin)| icons.find(bin).or_else(|| icons.find(id))).collect();
    app_row(icon, title, desc, options, &current, &faces, move |id| {
        let label = id.clone();
        // An app Omarchy's setter doesn't know is offered by its desktop entry (Browser) and
        // set the XDG way.
        let argv: Vec<&str> = if id.ends_with(".desktop") {
            vec!["xdg-settings", "set", "default-web-browser", &id]
        } else {
            vec![setter, &id]
        };
        cmd::run_async(&argv, move |r| match r {
            Ok(_) => window::toast(&format!("Default set to {label}")),
            Err(e) => window::toast(&format!("{e}")),
        });
    })
}

const TERMINAL_DESC: &str = "Super + Enter opens it";
const EDITOR_DESC: &str = "Opens config files";

fn category(title: &str) -> Option<&'static Category> {
    CATEGORIES.iter().find(|c| c.title == title)
}

/// The browser, email and files apps.
fn everyday(page: &Page, icons: &Icons) {
    let g = page.group("Everyday");
    match PLATFORM {
        Platform::Omarchy => g.add(&default_row(
            icons,
            "web-browser-symbolic",
            "Web browser",
            "Links from every app",
            "omarchy-default-browser",
            &[
                ("chromium", "Chromium", "chromium"),
                ("chrome", "Google Chrome", "google-chrome-stable"),
                ("brave", "Brave", "brave"),
                ("brave-origin", "Brave Origin", "brave-origin"),
                ("edge", "Microsoft Edge", "microsoft-edge-stable"),
                ("firefox", "Firefox", "firefox"),
                ("zen", "Zen", "zen-browser"),
                ("io.github.design_nexus.nyx.Browser.desktop", "Browser", "nyx-browser"),
            ],
        )),
        Platform::NyxOS => match mime_row(&WEB_BROWSER, icons, "web-browser-symbolic", "Links from every app") {
            Some(r) => g.add(&r),
            None => g.add(&widgets::icon_row(
                "web-browser-symbolic",
                WEB_BROWSER.title,
                "No browser is installed. Get one in the Store.",
                None,
            )),
        },
    }
    widgets::keywords(WEB_BROWSER.keywords);
    if let Some(r) = category("Email").and_then(|c| mime_row(c, icons, "mail-unread-symbolic", "mailto: links")) {
        widgets::keywords("email mail mailto");
        g.add(&r);
    }
    if let Some(r) = category("File manager").and_then(|c| mime_row(c, icons, "system-file-manager-symbolic", "Folders from every app")) {
        widgets::keywords("folders files browse file manager");
        g.add(&r);
    }
}

/// The terminal, the editor and the coding agent.
fn tools(page: &Page, icons: &Icons) {
    let g = page.group("Tools");
    match PLATFORM {
        Platform::Omarchy => {
            g.add(&default_row(
                icons,
                "utilities-terminal-symbolic",
                "Terminal",
                TERMINAL_DESC,
                "omarchy-default-terminal",
                &[
                    ("alacritty", "Alacritty", "alacritty"),
                    ("foot", "foot", "foot"),
                    ("ghostty", "Ghostty", "ghostty"),
                    ("kitty", "kitty", "kitty"),
                ],
            ));
            widgets::keywords("terminal console super enter");
            g.add(&default_row(
                icons,
                "accessories-text-editor-symbolic",
                "Text editor",
                EDITOR_DESC,
                "omarchy-default-editor",
                &[
                    ("nvim", "Neovim", "nvim"),
                    ("code", "VS Code", "code"),
                    ("cursor", "Cursor", "cursor"),
                    ("zed", "Zed", "zeditor"),
                    ("sublime_text", "Sublime Text", "subl"),
                    ("helix", "Helix", "helix"),
                    ("vim", "Vim", "vim"),
                    ("emacs", "Emacs", "emacs"),
                ],
            ));
            widgets::keywords("editor text config open");
            if let Some(r) = omarchy_agent_row(icons) {
                g.add(&r);
            }
        }
        Platform::NyxOS => {
            let (options, current) = defaults::terminals();
            if !options.is_empty() {
                let names = options.clone();
                let faces: Vec<_> = options.iter().map(|(id, _)| icons.find(id)).collect();
                g.add(&app_row("utilities-terminal-symbolic", "Terminal", TERMINAL_DESC, options, &current, &faces, move |id| {
                    let name = names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone()).unwrap_or_default();
                    match defaults::set_terminal(&id) {
                        Ok(()) => window::toast(&format!("Terminal set to {name}")),
                        Err(e) => window::toast(&format!("Couldn't set the terminal: {e}")),
                    }
                }));
                widgets::keywords("terminal console super enter");
            }
            let (options, current) = defaults::editors();
            if !options.is_empty() {
                let names = options.clone();
                let faces: Vec<_> = options.iter().map(|(id, _)| icons.find(id)).collect();
                g.add(&app_row("accessories-text-editor-symbolic", "Text editor", EDITOR_DESC, options, &current, &faces, move |id| {
                    let name = names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone()).unwrap_or_default();
                    match defaults::set_editor(&id) {
                        Ok(()) => window::toast(&format!("Text editor set to {name}; new terminals use it")),
                        Err(e) => window::toast(&format!("Couldn't set the editor: {e}")),
                    }
                }));
                widgets::keywords("editor text config open");
            }
            if let Some(r) = nyxos_agent_row(icons) {
                g.add(&r);
            }
        }
    }
}

const AGENT_DESC: &str = "Super + Shift + Ctrl + A";

/// The coding agents Omarchy's launcher knows: (id, label, command).
const OMARCHY_AGENTS: &[(&str, &str, &str)] = &[
    ("claude", "Claude Code", "claude"),
    ("codex", "Codex", "codex"),
    ("agy", "Antigravity", "agy"),
    ("cursor-agent", "Cursor CLI", "cursor-agent"),
    ("grok", "Grok", "grok"),
    ("crush", "Crush", "crush"),
    ("opencode", "OpenCode", "opencode"),
    ("omp", "Oh My Pi", "omp"),
    ("pi", "Pi", "pi"),
    ("copilot", "GitHub Copilot", "copilot"),
];

/// The agents a `nyx agent list` prints, one `command<TAB>name` per line.
fn agent_list(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(|l| l.split_once('\t')).map(|(c, n)| (c.to_string(), n.to_string())).collect()
}

/// A "Coding agent" row: None first, then the installed agents and the current one.
fn agent_row(
    icons: &Icons,
    mut options: Vec<(String, String)>,
    current: &str,
    set: impl Fn(&str) -> Result<(), String> + 'static,
) -> gtk::Box {
    options.insert(0, ("none".into(), "None".into()));
    let current = if current.is_empty() { "none" } else { current };
    let names = options.clone();
    let faces: Vec<_> = options.iter().map(|(id, _)| icons.find(id)).collect();
    let r = app_row("applications-engineering-symbolic", "Coding agent", AGENT_DESC, options, current, &faces, move |id| {
        let name = names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone()).unwrap_or_default();
        match set(&id) {
            Ok(()) if id == "none" => window::toast("No default coding agent"),
            Ok(()) => window::toast(&format!("Coding agent set to {name}")),
            Err(e) => window::toast(&format!("Couldn't set the coding agent: {e}")),
        }
    });
    r.set_tooltip_text(Some("Opens in a terminal with Super Shift Ctrl A, set to work without stopping to ask."));
    widgets::keywords("coding agent ai claude codex cursor");
    r
}

/// The default agent in `nyx agent`, which knows each one's command and flags.
fn nyxos_agent_row(icons: &Icons) -> Option<gtk::Box> {
    let installed = agent_list(&cmd::output(&["nyx", "agent", "list"])?);
    let current = cmd::output(&["nyx", "agent", "default"]).unwrap_or_default().trim().to_string();
    let mut options = installed;
    if !current.is_empty() && !options.iter().any(|(c, _)| *c == current) {
        let all = agent_list(&cmd::output(&["nyx", "agent", "list", "--all"]).unwrap_or_default());
        let name = all.into_iter().find(|(c, _)| *c == current).map(|(_, n)| n).unwrap_or(current.clone());
        options.push((current.clone(), format!("{name} (not installed)")));
    }
    if options.is_empty() {
        return None;
    }
    Some(agent_row(icons, options, &current, |id| {
        cmd::read(&["nyx", "agent", "default", id]).map(|_| ()).map_err(|e| e.to_string())
    }))
}

/// Omarchy's default agent is the file its launcher reads. `omarchy-default-agent` would
/// also start the agent, so the row writes the file itself.
fn omarchy_agent_file() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    home.join(".config/omarchy/defaults/agent")
}

fn omarchy_agent_row(icons: &Icons) -> Option<gtk::Box> {
    let current = std::fs::read_to_string(omarchy_agent_file()).unwrap_or_default().trim().to_string();
    let options: Vec<(String, String)> = OMARCHY_AGENTS
        .iter()
        .filter(|(id, _, bin)| installed(bin) || *id == current)
        .map(|(id, label, _)| (id.to_string(), label.to_string()))
        .collect();
    if options.is_empty() {
        return None;
    }
    Some(agent_row(icons, options, &current, |id| {
        let file = omarchy_agent_file();
        if id == "none" {
            return std::fs::remove_file(&file)
                .or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
                .map_err(|e| e.to_string());
        }
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&file, format!("{id}\n")).map_err(|e| e.to_string())
    }))
}

/// The kinds of file the map shows: (category, short name, icon, an ending it covers).
const KINDS: &[(&str, &str, &str, &str)] = &[
    ("Image viewer", "Images", "image-x-generic-symbolic", ".jpg"),
    ("Video player", "Videos", "video-x-generic-symbolic", ".mp4"),
    ("Music player", "Music", "audio-x-generic-symbolic", ".mp3"),
    ("PDFs", "PDFs", "application-pdf-symbolic", ".pdf"),
    ("Documents", "Documents", "x-office-document-symbolic", ".docx"),
    ("Spreadsheets", "Spreadsheets", "x-office-spreadsheet-symbolic", ".xlsx"),
    ("Presentations", "Presentations", "x-office-presentation-symbolic", ".pptx"),
    ("Text files", "Text files", "text-x-generic-symbolic", ".txt"),
    ("Archives", "Archives", "package-x-generic-symbolic", ".zip"),
    ("Calendar", "Calendar invites", "x-office-calendar-symbolic", ".ics"),
    ("Torrents", "Torrents", "folder-download-symbolic", "magnet:"),
];

pub fn build(page: &Page) {
    let icons = Icons::load();
    preview(page, &icons);
    everyday(page, &icons);
    tools(page, &icons);

    let g = page.group("");
    let all: Vec<&'static Category> =
        CATEGORIES.iter().filter(|c| !matches!(c.title, "Email" | "File manager")).filter(|c| !defaults::options(c).0.is_empty()).collect();
    let words: Vec<&str> = all.iter().map(|c| c.keywords).collect();
    let (r, _) = widgets::nav_row(
        Some("text-x-generic-symbolic"),
        "Each kind of file",
        "Images, music, PDFs, documents and more",
        &all.len().to_string(),
        || {
            widgets::subpage("Each kind of file", 600, |p| {
                let icons = Icons::load();
                let g = p.group("Files and links");
                g.note("Which app opens each kind of file or link, from the apps you have installed.");
                for c in CATEGORIES {
                    let icon = KINDS.iter().find(|k| k.0 == c.title).map_or("text-x-generic-symbolic", |k| k.2);
                    if let Some(r) = mime_row(c, &icons, icon, c.desc) {
                        widgets::keywords(c.keywords);
                        g.add(&r);
                    }
                }
            });
        },
    );
    widgets::keywords(&format!("files and links xdg mime open with {}", words.join(" ")));
    g.add(&r);
    if installed("nyx-browser") {
        let (r, _) = widgets::nav_row(
            Some("web-browser-symbolic"),
            "Browser's connections",
            "Share, Capture and the Assistant",
            "",
            || {
                widgets::subpage("Browser", 560, browser_group);
            },
        );
        widgets::keywords("browser share capture assistant integrations default browser");
        g.add(&r);
    }
}

/// The picture: every kind of file, a line to the app it opens in.
fn preview(page: &Page, icons: &Icons) {
    let pv = page.split();
    let content = roomy(&pv);
    content.append(&head("What opens what", "Every kind of file and the app it opens in. Click one to change it.", &[]));
    let map = widgets::hbox(0);
    map.add_css_class("dfa-map");
    let kinds = widgets::vbox(4);
    let gap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    gap.set_hexpand(true);
    gap.set_size_request(60, -1);
    let apps = widgets::vbox(4);
    apps.set_homogeneous(true);
    apps.set_valign(gtk::Align::Fill);
    map.append(&kinds);
    map.append(&gap);
    map.append(&apps);

    let links: Links = Rc::default();
    let mut app_nodes: Vec<(String, gtk::Widget)> = Vec::new();
    let mut unset: Option<gtk::Widget> = None;
    let (overlay, area) = linked(&map, links.clone());
    for (title, short, icon, ending) in KINDS {
        let Some(c) = category(title) else { continue };
        let (options, current) = defaults::options(c);
        if options.is_empty() {
            continue;
        }
        let node = gtk::MenuButton::new();
        node.add_css_class("dfa-kind");
        let row = widgets::hbox(8);
        row.append(&gtk::Image::from_icon_name(icon));
        let name = widgets::label(short, "dfa-kind-name");
        name.set_hexpand(true);
        row.append(&name);
        row.append(&widgets::label(ending, "dfa-ending"));
        node.set_child(Some(&row));
        node.set_tooltip_text(Some(&c.desc.replace("<tt>", "").replace("</tt>", "")));
        kinds.append(&node);

        let target = if current.is_empty() {
            unset
                .get_or_insert_with(|| {
                    let n = app_node(None, "Nothing yet");
                    n.add_css_class("none");
                    n.upcast()
                })
                .clone()
        } else if let Some((_, n)) = app_nodes.iter().find(|(id, _)| *id == current) {
            n.clone()
        } else {
            let label = options.iter().find(|(id, _)| *id == current).map_or(current.clone(), |(_, l)| l.clone());
            let n: gtk::Widget = app_node(icons.find(&current), &label).upcast();
            app_nodes.push((current.clone(), n.clone()));
            n
        };
        let index = links.borrow().len();
        let resting = if current.is_empty() { Link::Off } else { Link::On };
        links.borrow_mut().push((node.clone().upcast(), target.clone(), resting));
        node.set_popover(Some(&chooser(c, short, &options, &current, icons)));
        let (links, area) = (links.clone(), area.clone());
        node.connect_active_notify(move |n| {
            let on = n.is_active();
            for w in [n.upcast_ref::<gtk::Widget>(), &target] {
                if on {
                    w.add_css_class("hot");
                } else {
                    w.remove_css_class("hot");
                }
            }
            if let Some(l) = links.borrow_mut().get_mut(index) {
                l.2 = if on { Link::Hot } else { resting };
            }
            area.queue_draw();
        });
    }
    for (_, n) in &app_nodes {
        apps.append(n);
    }
    if let Some(n) = unset {
        apps.append(&n);
    }
    content.append(&overlay);
    pv.foot("dialog-information-symbolic", "Only apps that say they can open a kind of file are offered for it.");
}

/// An app on the right of the map: its icon and name.
fn app_node(icon: Option<gio::Icon>, name: &str) -> gtk::Box {
    let b = widgets::hbox(8);
    b.add_css_class("dfa-app");
    b.set_valign(gtk::Align::Center);
    let image = gtk::Image::new();
    image.set_pixel_size(18);
    match icon {
        Some(i) => image.set_from_gicon(&i),
        None => image.set_icon_name(Some("application-x-executable-symbolic")),
    }
    b.append(&image);
    let l = widgets::label(name, "dfa-app-name");
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(16);
    b.append(&l);
    b
}

/// The apps that can open a kind of file, the current one ticked.
fn chooser(c: &'static Category, short: &str, options: &[(String, String)], current: &str, icons: &Icons) -> gtk::Popover {
    let pop = gtk::Popover::new();
    pop.add_css_class("dfa-chooser");
    pop.set_position(gtk::PositionType::Right);
    let col = widgets::vbox(2);
    col.append(&widgets::label(&format!("Open {} with", short.to_lowercase()), "dfa-chooser-head"));
    for (id, name) in options {
        let b = gtk::Button::new();
        b.add_css_class("flat");
        b.add_css_class("dfa-choice");
        let row = widgets::hbox(10);
        let image = gtk::Image::new();
        image.set_pixel_size(20);
        match icons.find(id) {
            Some(i) => image.set_from_gicon(&i),
            None => image.set_icon_name(Some("application-x-executable-symbolic")),
        }
        row.append(&image);
        let l = widgets::label(name, "");
        l.set_hexpand(true);
        row.append(&l);
        if id == current {
            row.append(&gtk::Image::from_icon_name("object-select-symbolic"));
            b.add_css_class("current");
        }
        b.set_child(Some(&row));
        let (id, name, p) = (id.clone(), name.clone(), pop.clone());
        b.connect_clicked(move |_| {
            p.popdown();
            match defaults::set(c, &id) {
                Ok(()) => window::toast(&format!("{} set to {name}", c.title)),
                Err(e) => window::toast(&format!("Couldn't set {}: {e}", c.title)),
            }
            window::rebuild("apps");
        });
        col.append(&b);
    }
    pop.set_child(Some(&col));
    pop
}

const BROWSER_DESKTOP: &str = "io.github.design_nexus.nyx.Browser.desktop";

/// What Browser may reach. The browser's native host reads this file on every request, so a
/// change applies at once. A group that isn't listed is on.
fn browser_integrations_file() -> std::path::PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".config")))
        .unwrap_or_default();
    config.join("nyx-browser/integrations.json")
}

fn browser_integrations() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(browser_integrations_file())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn set_browser_integration(key: &str, on: bool) -> std::io::Result<()> {
    let mut all = browser_integrations();
    all.insert(key.to_string(), on.into());
    let file = browser_integrations_file();
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = file.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::Value::Object(all).to_string())?;
    std::fs::rename(&temporary, &file)
}

fn browser_group(page: &Page) {
    let g = page.group("Browser");
    g.note("Theme, tabs and sidebar are set in Browser itself.");

    let is_default = cmd::output(&["xdg-settings", "get", "default-web-browser"]).is_some_and(|d| d.trim() == BROWSER_DESKTOP);
    let (r, _) = widgets::switch_row("Default browser", "Opens links from every app in Browser.", is_default, |on| {
        if on {
            cmd::run_async(&["xdg-settings", "set", "default-web-browser", BROWSER_DESKTOP], |r| {
                match r {
                    Ok(_) => window::toast("Default set to Browser"),
                    Err(e) => window::toast(&format!("{e}")),
                }
                window::rebuild_if_built("apps");
            });
        } else {
            window::toast("Choose another browser under Web browser");
        }
    });
    g.add(&r);

    let saved = browser_integrations();
    for (key, title, desc) in [
        ("share", "Share", "Lets Browser send pages, selections, images and downloads to the Share Sheet."),
        ("capture", "Capture", "Lets Browser open screenshots of pages in Capture."),
        ("assistant", "Assistant", "Lets Browser ask the local Assistant about a page. It asks you before reading each page."),
    ] {
        let on = saved.get(key).and_then(serde_json::Value::as_bool).unwrap_or(true);
        let (r, _) = widgets::switch_row(title, desc, on, move |on| {
            if let Err(e) = set_browser_integration(key, on) {
                window::toast(&format!("Couldn't save that: {e}"));
            }
        });
        g.add(&r);
    }
}

/// A row choosing the app for a kind of file, with app icons; `desc` replaces the kind's own.
fn mime_row(c: &'static Category, icons: &Icons, icon: &str, desc: &str) -> Option<gtk::Box> {
    let (options, current) = defaults::options(c);
    if options.is_empty() {
        return None;
    }
    let names = options.clone();
    let faces: Vec<_> = options.iter().map(|(id, _)| icons.find(id)).collect();
    Some(app_row(icon, c.title, desc, options, &current, &faces, move |id| {
        let name = names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone()).unwrap_or(id.clone());
        match defaults::set(c, &id) {
            Ok(()) => window::toast(&format!("{} set to {name}", c.title)),
            Err(e) => window::toast(&format!("Couldn't set {}: {e}", c.title)),
        }
    }))
}
