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
