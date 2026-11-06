use crate::platform::{PLATFORM, Platform};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use nyx_ui::default_apps::{self as defaults, CATEGORIES, Category, WEB_BROWSER};

fn installed(command: &str) -> bool {
    cmd::present(command)
}

fn default_row(
    title: &str,
    desc: &str,
    setter: &'static str,
    choices: &[(&str, &str, &str)], // (id, label, command that must exist)
) -> gtk::Box {
    let current = cmd::output(&[setter]).unwrap_or_default().trim().to_string();
    let options: Vec<(String, String)> = choices
        .iter()
        .filter(|(id, _, bin)| installed(bin) || *id == current)
        .map(|(id, label, _)| (id.to_string(), label.to_string()))
        .collect();
    let (r, _) = widgets::choice_row(title, desc, options, &current, move |id| {
        let label = id.clone();
        cmd::run_async(&[setter, &id], move |r| match r {
            Ok(_) => window::toast(&format!("Default set to {label}")),
            Err(e) => window::toast(&format!("{e}")),
        });
    });
    r
}

const TERMINAL_DESC: &str = "Used by <tt>Super Enter</tt> and anything that opens a terminal.";
const EDITOR_DESC: &str = "Opens config files, including this app's Open config buttons.";

/// Omarchy's defaults, through its own commands.
fn omarchy_defaults(page: &Page) {
    let g = page.group("Defaults");
    g.add(&default_row(
        "Web browser",
        "Opens links from every app.",
        "omarchy-default-browser",
        &[
            ("chromium", "Chromium", "chromium"),
            ("chrome", "Google Chrome", "google-chrome-stable"),
            ("brave", "Brave", "brave"),
            ("brave-origin", "Brave Origin", "brave-origin"),
            ("edge", "Microsoft Edge", "microsoft-edge-stable"),
            ("firefox", "Firefox", "firefox"),
            ("zen", "Zen", "zen-browser"),
        ],
    ));
    g.add(&default_row(
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
    g.add(&default_row(
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
    if let Some(r) = omarchy_agent_row() {
        g.add(&r);
    }
}

const AGENT_DESC: &str = "Opens in a terminal with <tt>Super Shift Ctrl A</tt>, set to work without stopping to ask.";

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
fn agent_row(mut options: Vec<(String, String)>, current: &str, set: impl Fn(&str) -> Result<(), String> + 'static) -> gtk::Box {
    options.insert(0, ("none".into(), "None".into()));
    let current = if current.is_empty() { "none" } else { current };
    let names = options.clone();
    let (r, _) = widgets::choice_row("Coding agent", AGENT_DESC, options, current, move |id| {
        let name = names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone()).unwrap_or_default();
        match set(&id) {
            Ok(()) if id == "none" => window::toast("No default coding agent"),
            Ok(()) => window::toast(&format!("Coding agent set to {name}")),
            Err(e) => window::toast(&format!("Couldn't set the coding agent: {e}")),
        }
    });
    r
}

/// The default agent in `nyx agent`, which knows each one's command and flags.
fn nyxos_agent_row() -> Option<gtk::Box> {
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
    Some(agent_row(options, &current, |id| cmd::read(&["nyx", "agent", "default", id]).map(|_| ()).map_err(|e| e.to_string())))
}

/// Omarchy's default agent is the file its launcher reads. `omarchy-default-agent` would
/// also start the agent, so the row writes the file itself.
fn omarchy_agent_file() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    home.join(".config/omarchy/defaults/agent")
}

fn omarchy_agent_row() -> Option<gtk::Box> {
    let current = std::fs::read_to_string(omarchy_agent_file()).unwrap_or_default().trim().to_string();
    let options: Vec<(String, String)> = OMARCHY_AGENTS
        .iter()
        .filter(|(id, _, bin)| installed(bin) || *id == current)
        .map(|(id, label, _)| (id.to_string(), label.to_string()))
        .collect();
    if options.is_empty() {
        return None;
    }
    Some(agent_row(options, &current, |id| {
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

/// NyxOS's defaults: the browser as a MIME default, the terminal in
/// xdg-terminals.list and the editor in ~/.config/nyx/editor.
fn nyxos_defaults(page: &Page) {
    let g = page.group("Defaults");
    if let Some(r) = mime_row(&WEB_BROWSER) {
        g.add(&r);
    }
    let (options, current) = defaults::terminals();
    if !options.is_empty() {
        let names = options.clone();
        let (r, _) = widgets::choice_row("Terminal", TERMINAL_DESC, options, &current, move |id| {
            let name = names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone()).unwrap_or_default();
            match defaults::set_terminal(&id) {
                Ok(()) => window::toast(&format!("Terminal set to {name}")),
                Err(e) => window::toast(&format!("Couldn't set the terminal: {e}")),
            }
        });
        g.add(&r);
    }
    let (options, current) = defaults::editors();
    if !options.is_empty() {
        let names = options.clone();
        let (r, _) = widgets::choice_row("Text editor", EDITOR_DESC, options, &current, move |id| {
            let name = names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone()).unwrap_or_default();
            match defaults::set_editor(&id) {
                Ok(()) => window::toast(&format!("Text editor set to {name}; new terminals use it")),
                Err(e) => window::toast(&format!("Couldn't set the editor: {e}")),
            }
        });
        g.add(&r);
    }
    if let Some(r) = nyxos_agent_row() {
        g.add(&r);
    }
}

pub fn build(page: &Page) {
    match PLATFORM {
        Platform::NyxOS => nyxos_defaults(page),
        Platform::Omarchy => omarchy_defaults(page),
    }

    let g = page.group("Files and links");
    g.note("Which app opens each kind of file or link, from the apps you have installed.");
    for c in CATEGORIES {
        if let Some(r) = mime_row(c) {
            widgets::keywords(c.keywords);
            g.add(&r);
        }
    }

    if installed("nyx-browser") {
        browser_group(page);
    }
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
            cmd::run_async(&["xdg-settings", "set", "default-web-browser", BROWSER_DESKTOP], |r| match r {
                Ok(_) => window::toast("Default set to Browser"),
                Err(e) => window::toast(&format!("{e}")),
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

fn mime_row(c: &'static Category) -> Option<gtk::Box> {
    let (options, current) = defaults::options(c);
    if options.is_empty() {
        return None;
    }
    let names = options.clone();
    let (r, _) = widgets::choice_row(c.title, c.desc, options, &current, move |id| {
        let name = names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone()).unwrap_or(id.clone());
        match defaults::set(c, &id) {
            Ok(()) => window::toast(&format!("{} set to {name}", c.title)),
            Err(e) => window::toast(&format!("Couldn't set {}: {e}", c.title)),
        }
    });
    Some(r)
}
