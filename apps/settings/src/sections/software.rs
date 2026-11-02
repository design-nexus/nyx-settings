//! Apps and packages: web apps, terminal apps, a package search, the update
//! channel and Omarchy's installers. The Omarchy scripts ask for sudo
//! themselves, so the ones that need it run in a terminal.

use crate::dialog::{Field, ask};
use crate::sections::accounts::terminal;
use crate::widgets::{self, Page};
use crate::{cmd, paths, window};
use gtk::prelude::*;
use std::collections::HashSet;

pub fn build(page: &Page) {
    channel(page);
    let launchers = launchers();
    web_apps(page, &launchers);
    terminal_apps(page, &launchers);
    packages(page);
    installers(page);
    maintenance(page);
}

// ----- Launchers made by Omarchy -----

#[derive(Debug, PartialEq)]
enum Kind {
    Web(String),
    Tui(String, bool),
}

#[derive(Debug, PartialEq)]
struct Launcher {
    /// The file name without `.desktop`, which the remove scripts take.
    id: String,
    name: String,
    /// Icon name or file, from the desktop file.
    icon: String,
    kind: Kind,
}

/// Read one desktop file: a web app (`omarchy-launch-webapp URL`) or a terminal
/// app (`xdg-terminal-exec --app-id=TUI.float|tile -e COMMAND`).
fn parse_launcher(id: &str, text: &str) -> Option<Launcher> {
    let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)).map(|v| v.trim().to_string());
    let name = field("Name=").filter(|n| !n.is_empty()).unwrap_or_else(|| id.to_string());
    let exec = field("Exec=")?;
    let icon = field("Icon=").unwrap_or_default();
    if let Some(rest) = exec.strip_prefix("omarchy-launch-webapp ") {
        return Some(Launcher { id: id.into(), name, icon, kind: Kind::Web(rest.trim().trim_matches('"').to_string()) });
    }
    if exec.starts_with("xdg-terminal-exec") && exec.contains("app-id=TUI.") {
        let float = exec.contains("TUI.float");
        let command = exec.split_once(" -e ").map(|(_, c)| c.trim().to_string())?;
        return Some(Launcher { id: id.into(), name, icon, kind: Kind::Tui(command, float) });
    }
    None
}

fn launchers() -> Vec<Launcher> {
    let dir = paths::data_home().join("applications");
    let mut found: Vec<Launcher> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let id = path.file_name()?.to_str()?.strip_suffix(".desktop")?.to_string();
            parse_launcher(&id, &std::fs::read_to_string(&path).ok()?)
        })
        .collect();
    found.sort_by_key(|l| l.name.to_lowercase());
    found
}

fn after(done: &'static str) -> impl FnOnce(anyhow::Result<String>) + 'static {
    move |r| {
        match r {
            Ok(_) => window::toast(done),
            Err(e) => window::toast(&format!("{e}")),
        }
        window::rebuild("software");
    }
}

/// The launcher's icon, small, at the start of its row.
fn launcher_icon(icon: &str, fallback: &str) -> gtk::Image {
    let img = if icon.starts_with('/') && std::path::Path::new(icon).exists() {
        gtk::Image::from_file(icon)
    } else if !icon.is_empty() && gtk::gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(icon)) {
        gtk::Image::from_icon_name(icon)
    } else {
        gtk::Image::from_icon_name(fallback)
    };
    img.set_pixel_size(28);
    img.set_valign(gtk::Align::Center);
    img.add_css_class("launcher-icon");
    img
}

/// A launcher's row: its icon, name and detail, with Edit and Remove that show
/// on hover or keyboard focus.
fn launcher_row(
    l: &Launcher,
    desc: &str,
    fallback_icon: &str,
    edit: impl Fn() + 'static,
    remove: impl Fn() + 'static,
) -> gtk::Box {
    let actions = widgets::hbox(2);
    actions.add_css_class("row-actions");
    let e = gtk::Button::from_icon_name("document-edit-symbolic");
    e.add_css_class("flat");
    e.set_tooltip_text(Some("Edit"));
    e.connect_clicked(move |_| edit());
    let r = gtk::Button::from_icon_name("user-trash-symbolic");
    r.add_css_class("flat");
    r.set_tooltip_text(Some("Remove"));
    r.connect_clicked(move |b| {
        b.set_sensitive(false);
        remove();
    });
    actions.append(&e);
    actions.append(&r);
    let row = widgets::row(&l.name, desc, Some(actions.upcast_ref()));
    row.add_css_class("launcher-row");
    row.prepend(&launcher_icon(&l.icon, fallback_icon));
    row
}

/// A filter box above a long list of rows (more than eight).
fn list_filter(g: &widgets::Group, rows: Vec<(gtk::Box, String)>, what: &str) {
    if rows.len() <= 8 {
        return;
    }
    let f = gtk::SearchEntry::new();
    f.set_placeholder_text(Some(&format!("Filter {} {what}", rows.len())));
    f.set_hexpand(true);
    // The first row of the card, so an open group stays one card.
    let holder = widgets::hbox(0);
    holder.add_css_class("settings-option");
    holder.add_css_class("card-row");
    holder.add_css_class("filter-row");
    holder.append(&f);
    g.list.prepend(&holder);
    let list = g.list.clone();
    f.connect_search_changed(move |f| {
        let terms = crate::search::terms(&f.text());
        for (row, text) in &rows {
            row.set_visible(terms.is_empty() || crate::search::matches(text, &terms));
        }
        widgets::mark_first_rows(list.upcast_ref());
    });
}

fn web_apps(page: &Page, all: &[Launcher]) {
    if !cmd::present("omarchy-webapp-install") {
        return;
    }
    let g = page.collapsible("Web apps", false);
    g.note("A website that opens in its own window and shows up in the app launcher.");
    let mut rows = Vec::new();
    for l in all {
        if let Kind::Web(url) = &l.kind {
            let (id, name, url2) = (l.id.clone(), l.name.clone(), url.clone());
            let edit = {
                let (id, name, url) = (id.clone(), name.clone(), url2.clone());
                move || edit_web_app(&id, &name, &url)
            };
            let remove = move || {
                let (name, url) = (name.clone(), url2.clone());
                cmd::run_async(&["omarchy-webapp-remove", &id], move |r| {
                    window::rebuild("software");
                    match r {
                        // Removing deletes the launcher; putting it back makes it again.
                        Ok(_) => window::toast_action(&format!("Removed {name}"), "Undo", move || {
                            cmd::run_async(&["omarchy-webapp-install", &name, &url, ""], after("Web app restored"));
                        }),
                        Err(e) => window::toast(&format!("{e}")),
                    }
                });
            };
            let row = launcher_row(l, &gtk::glib::markup_escape_text(url), "applications-internet-symbolic", edit, remove);
            g.add(&row);
            widgets::keywords("webapp web app website launcher pwa");
            rows.push((row, crate::search::normalise(&format!("{} {url}", l.name))));
        }
    }
    list_filter(&g, rows, "web apps");
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Name"));
    let url = gtk::Entry::new();
    url.set_placeholder_text(Some("https://example.com"));
    let add = gtk::Button::with_label("Add");
    add.add_css_class("suggested-action");
    let form = widgets::vbox(8);
    form.append(&name);
    form.append(&url);
    form.append(&add);
    add.connect_clicked(move |b| {
        let (n, u) = (name.text().trim().to_string(), url.text().trim().to_string());
        if n.is_empty() || u.is_empty() || n.contains('/') {
            window::toast("Enter a name (without /) and a web address");
            return;
        }
        b.set_sensitive(false);
        // An empty icon makes Omarchy fetch the site's own.
        cmd::run_async(&["omarchy-webapp-install", &n, &u, ""], after("Web app added"));
    });
    g.add(&widgets::form_disclosure("Add a web app…", "The icon is fetched from the site.", &form));
    widgets::keywords("new create webapp website");
}

/// Replace a launcher: make the new one first, so a failed install keeps the old
/// one, then take the old one away if it had another name (the file is named after it).
fn replace_launcher(remove: &'static str, id: String, install: Vec<String>, done: &'static str) {
    cmd::background(
        move || {
            let refs: Vec<&str> = install.iter().map(String::as_str).collect();
            let out = cmd::run(&refs)?;
            if install.get(1).is_some_and(|name| *name != id) {
                cmd::run(&[remove, &id])?;
            }
            Ok(out)
        },
        after(done),
    );
}

fn edit_web_app(id: &str, name: &str, url: &str) {
    let id = id.to_string();
    ask(
        "Edit web app",
        "The icon is fetched from the site again.",
        vec![Field::text_with("Name", name), Field::text_with("https://example.com", url)],
        "Save",
        None,
        move |v, _| {
            let (n, u) = (v[0].trim().to_string(), v[1].trim().to_string());
            if n.is_empty() || u.is_empty() || n.contains('/') {
                return Some("Enter a name (without /) and a web address.".into());
            }
            replace_launcher(
                "omarchy-webapp-remove",
                id.clone(),
                vec!["omarchy-webapp-install".into(), n, u, String::new()],
                "Web app updated",
            );
            None
        },
    );
}

fn edit_terminal_app(id: &str, name: &str, command: &str, float: bool) {
    let id = id.to_string();
    let styles = widgets::opts(&[("float", "Floating window"), ("tile", "Tiled window")]);
    ask(
        "Edit terminal app",
        "",
        vec![
            Field::text_with("Name", name),
            Field::text_with("Command", command),
            Field::choice(styles, if float { "float" } else { "tile" }),
        ],
        "Save",
        None,
        move |v, _| {
            let (n, c) = (v[0].trim().to_string(), v[1].trim().to_string());
            if n.is_empty() || c.is_empty() || n.contains('/') {
                return Some("Enter a name (without /) and a command.".into());
            }
            replace_launcher(
                "omarchy-tui-remove",
                id.clone(),
                vec!["omarchy-tui-install".into(), n, c, v[2].clone(), "utilities-terminal".into()],
                "Terminal app updated",
            );
            None
        },
    );
}

fn terminal_apps(page: &Page, all: &[Launcher]) {
    if !cmd::present("omarchy-tui-install") {
        return;
    }
    let g = page.collapsible("Terminal apps", false);
    g.note("A command-line program with an entry in the app launcher.");
    let mut rows = Vec::new();
    for l in all {
        if let Kind::Tui(command, float) = &l.kind {
            let desc = format!("{} · {}", gtk::glib::markup_escape_text(command), if *float { "floating" } else { "tiled" });
            let (id, name, command2, float) = (l.id.clone(), l.name.clone(), command.clone(), *float);
            let edit = {
                let (id, name, command) = (id.clone(), name.clone(), command2.clone());
                move || edit_terminal_app(&id, &name, &command, float)
            };
            let remove = move || {
                let (name, command) = (name.clone(), command2.clone());
                cmd::run_async(&["omarchy-tui-remove", &id], move |r| {
                    window::rebuild("software");
                    match r {
                        Ok(_) => window::toast_action(&format!("Removed {name}"), "Undo", move || {
                            let style = if float { "float" } else { "tile" };
                            cmd::run_async(
                                &["omarchy-tui-install", &name, &command, style, "utilities-terminal"],
                                after("Terminal app restored"),
                            );
                        }),
                        Err(e) => window::toast(&format!("{e}")),
                    }
                });
            };
            let row = launcher_row(l, &desc, "utilities-terminal-symbolic", edit, remove);
            g.add(&row);
            widgets::keywords("tui terminal app launcher command");
            rows.push((row, crate::search::normalise(&format!("{} {command}", l.name))));
        }
    }
    list_filter(&g, rows, "terminal apps");
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Name"));
    let command = gtk::Entry::new();
    command.set_placeholder_text(Some("Command, e.g. btop"));
    let style = widgets::dropdown(&widgets::opts(&[("float", "Floating window"), ("tile", "Tiled window")]), "float");
    let add = gtk::Button::with_label("Add");
    add.add_css_class("suggested-action");
    let form = widgets::vbox(8);
    form.append(&name);
    form.append(&command);
    form.append(&style);
    form.append(&add);
    add.connect_clicked(move |b| {
        let (n, c) = (name.text().trim().to_string(), command.text().trim().to_string());
        if n.is_empty() || c.is_empty() || n.contains('/') {
            window::toast("Enter a name (without /) and a command");
            return;
        }
        let s = if style.selected() == 1 { "tile" } else { "float" };
        b.set_sensitive(false);
        // Omarchy needs an icon: use the theme's generic terminal one.
        cmd::run_async(&["omarchy-tui-install", &n, &c, s, "utilities-terminal"], after("Terminal app added"));
    });
    g.add(&widgets::form_disclosure("Add a terminal app…", "", &form));
    widgets::keywords("new create tui command");
}

// ----- Packages -----

fn valid_package(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 100
        && !s.starts_with(['-', '.'])
        && s.chars().all(|c| c.is_ascii_alphanumeric() || "@._+-".contains(c))
}

fn packages(page: &Page) {
    if !cmd::present("pacman") {
        return;
    }
    let g = page.collapsible("Packages", false);
    let explicit = cmd::output(&["pacman", "-Qqe"]).map(|t| t.lines().count()).unwrap_or(0);
    let (r, _) = widgets::info_row("Installed on purpose", &format!("{explicit} packages"));
    g.add(&r);
    let results = widgets::vbox(6);
    let (r, entry) = widgets::entry_row(
        "Find a package",
        "Search the Arch repositories and the AUR, by name. Press Enter.",
        "",
        "package name",
        {
            let results = results.clone();
            move |q| search(&results, q.trim())
        },
    );
    entry.set_width_chars(22);
    widgets::keywords("pacman yay aur install uninstall search app software");
    g.add(&r);
    g.add(&results);
}

fn search(results: &gtk::Box, query: &str) {
    widgets::forget_rows(results);
    while let Some(c) = results.first_child() {
        results.remove(&c);
    }
    if query.is_empty() || query.starts_with('-') {
        return;
    }
    results.append(&widgets::label("Searching…", "dim"));
    let helper = if cmd::present("yay") { "yay" } else { "pacman" };
    let q = query.to_string();
    let results = results.clone();
    cmd::background(
        move || {
            let found = cmd::output(&[helper, "-Ssq", "--", &q]).unwrap_or_default();
            let installed: HashSet<String> =
                cmd::output(&["pacman", "-Qq"]).unwrap_or_default().lines().map(str::to_string).collect();
            (found, installed)
        },
        move |(found, installed)| {
            while let Some(c) = results.first_child() {
                results.remove(&c);
            }
            widgets::begin_section("software");
            let names: Vec<&str> = found.lines().filter(|n| valid_package(n)).take(15).collect();
            if names.is_empty() {
                results.append(&widgets::label("Nothing found.", "dim"));
            }
            for n in names {
                let on = installed.contains(n);
                let b = gtk::Button::with_label(if on { "Remove…" } else { "Install…" });
                let name = n.to_string();
                b.connect_clicked(move |_| {
                    let q = cmd::shell_quote(&name);
                    terminal(&if on { format!("sudo pacman -Rns -- {q}") } else { format!("{} -S --needed -- {q}", helper) });
                });
                results.append(&widgets::row(n, if on { "Installed" } else { "" }, Some(b.upcast_ref())));
            }
        },
    );
}

// ----- Omarchy installers -----

/// How to tell an app is installed.
#[derive(Clone, Copy)]
enum Check {
    /// Any of these packages.
    Pkgs(&'static [&'static str]),
    /// A launcher in `~/.local/share/applications` (by file name, without `.desktop`).
    Desktop(&'static str),
    /// A Flatpak app id.
    Flatpak(&'static str),
}

/// (name, how to tell, install script, remove script)
type Item = (&'static str, Check, &'static str, Option<&'static str>);

const GROUPS: &[(&str, &str, &[Item])] = &[
    (
        "Editors",
        "code editor vscode zed helix emacs",
        &[
            ("VS Code", Check::Pkgs(&["visual-studio-code-bin"]), "omarchy-install-editor-vscode", None),
            ("Zed", Check::Pkgs(&["zed"]), "omarchy-install-editor-zed", None),
            ("Helix", Check::Pkgs(&["helix"]), "omarchy-install-editor-helix", None),
            ("Emacs", Check::Pkgs(&["omarchy-emacs"]), "omarchy-install-editor-emacs", None),
        ],
    ),
    (
        "Games",
        "gaming steam lutris heroic retroarch battle.net geforce xbox controller",
        &[
            ("Steam", Check::Pkgs(&["steam"]), "omarchy-install-gaming-steam", Some("omarchy-remove-gaming-steam")),
            ("Lutris", Check::Pkgs(&["lutris"]), "omarchy-install-gaming-lutris", Some("omarchy-remove-gaming-lutris")),
            (
                "Heroic",
                Check::Pkgs(&["heroic-games-launcher-bin"]),
                "omarchy-install-gaming-heroic",
                Some("omarchy-remove-gaming-heroic"),
            ),
            (
                "RetroArch",
                Check::Pkgs(&["retroarch"]),
                "omarchy-install-gaming-retroarch",
                Some("omarchy-remove-gaming-retroarch"),
            ),
            (
                "Battle.net",
                Check::Desktop("battlenet"),
                "omarchy-install-gaming-battlenet",
                Some("omarchy-remove-gaming-battlenet"),
            ),
            (
                "GeForce Now",
                Check::Flatpak("com.nvidia.geforcenow"),
                "omarchy-install-gaming-geforce-now",
                Some("omarchy-remove-gaming-geforce-now"),
            ),
            (
                "Xbox Cloud Gaming",
                Check::Desktop("Xbox Cloud Gaming"),
                "omarchy-install-gaming-xbox-cloud",
                Some("omarchy-remove-gaming-xbox-cloud"),
            ),
            (
                "Xbox controllers",
                Check::Pkgs(&["xpadneo-dkms"]),
                "omarchy-install-gaming-xbox-controllers",
                Some("omarchy-remove-gaming-xbox-controllers"),
            ),
        ],
    ),
    (
        "AI",
        "ai assistant claude chatgpt ollama lm studio openclaw hermes",
        &[
            ("Claude", Check::Pkgs(&["claude-desktop"]), "omarchy-install-ai-claude", Some("omarchy-remove-ai-claude")),
            ("ChatGPT", Check::Pkgs(&["openai-codex-desktop"]), "omarchy-install-ai-chatgpt", Some("omarchy-remove-ai-chatgpt")),
            ("Hermes", Check::Desktop("hermes"), "omarchy-install-ai-hermes", Some("omarchy-remove-ai-hermes")),
            ("OpenClaw", Check::Pkgs(&["openclaw"]), "omarchy-install-ai-openclaw", Some("omarchy-remove-ai-openclaw")),
            ("T3 Code", Check::Pkgs(&["t3code-bin"]), "omarchy-install-ai-t3-code", Some("omarchy-remove-ai-t3-code")),
            (
                "Ollama",
                Check::Pkgs(&["ollama", "ollama-cuda", "ollama-rocm", "ollama-vulkan"]),
                "",
                Some("omarchy-remove-ai-ollama"),
            ),
            ("LM Studio", Check::Pkgs(&["lmstudio-bin"]), "", Some("omarchy-remove-ai-lm-studio")),
        ],
    ),
    (
        "Services",
        "service 1password dropbox signal spotify sunshine tailscale nordvpn",
        &[
            (
                "1Password",
                Check::Pkgs(&["1password"]),
                "omarchy-install-service-1password",
                Some("omarchy-remove-service-1password"),
            ),
            ("Dropbox", Check::Pkgs(&["dropbox"]), "omarchy-install-service-dropbox", Some("omarchy-remove-service-dropbox")),
            ("Signal", Check::Pkgs(&["signal-desktop"]), "omarchy-install-service-signal", None),
            ("Spotify", Check::Pkgs(&["spotify"]), "omarchy-install-service-spotify", None),
            ("Sunshine", Check::Pkgs(&["sunshine"]), "omarchy-install-service-sunshine", Some("omarchy-remove-service-sunshine")),
            (
                "Tailscale",
                Check::Pkgs(&["tailscale"]),
                "omarchy-install-service-tailscale",
                Some("omarchy-remove-service-tailscale"),
            ),
            ("NordVPN", Check::Pkgs(&["nordvpn-bin"]), "omarchy-install-service-nordvpn", None),
        ],
    ),
];

/// What is installed, found once for the whole page.
struct Installed {
    packages: HashSet<String>,
    flatpaks: HashSet<String>,
}

impl Installed {
    fn read() -> Installed {
        let list = |args: &[&str]| {
            cmd::output(args).unwrap_or_default().lines().map(|l| l.split_whitespace().next().unwrap_or("").to_string()).collect()
        };
        let flatpaks =
            if cmd::present("flatpak") { list(&["flatpak", "list", "--app", "--columns=application"]) } else { HashSet::new() };
        Installed { packages: list(&["pacman", "-Qq"]), flatpaks }
    }

    fn has(&self, check: Check) -> bool {
        match check {
            Check::Pkgs(names) => names.iter().any(|n| self.packages.contains(*n)),
            Check::Desktop(stem) => paths::data_home().join("applications").join(format!("{stem}.desktop")).exists(),
            Check::Flatpak(id) => self.flatpaks.contains(id),
        }
    }
}

fn icon_button(icon: &str, tip: &str, action: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("flat");
    b.set_tooltip_text(Some(tip));
    b.connect_clicked(move |_| action());
    b
}

fn installers(page: &Page) {
    let g = page.collapsible("Install apps", false);
    g.note("Omarchy's own installers. A terminal opens to show progress and ask for your password.");
    let installed = Installed::read();
    for (title, words, items) in GROUPS {
        let (wrapper, content) = widgets::disclosure(title, "");
        widgets::keywords(words);
        let mut any = false;
        for (name, check, install, remove) in *items {
            let on = installed.has(*check);
            let button = if on {
                // Omarchy's remover when it has one; otherwise take the package off.
                match (*remove).filter(|r| cmd::present(r)) {
                    Some(script) => icon_button("user-trash-symbolic", "Remove", move || terminal(script)),
                    None => match check {
                        Check::Pkgs(names) => {
                            let found: Vec<String> =
                                names.iter().filter(|n| installed.packages.contains(**n)).map(|n| cmd::shell_quote(n)).collect();
                            icon_button("user-trash-symbolic", "Remove", move || {
                                terminal(&format!("sudo pacman -Rns -- {}", found.join(" ")))
                            })
                        }
                        _ => continue,
                    },
                }
            } else if !install.is_empty() && cmd::present(install) {
                let script = *install;
                icon_button("list-add-symbolic", "Install", move || terminal(script))
            } else {
                continue;
            };
            any = true;
            content.append(&widgets::row(name, if on { "Installed" } else { "" }, Some(button.upcast_ref())));
        }
        if any {
            g.add(&wrapper);
        }
    }
    let controls = widgets::hbox(8);
    for (script, label) in [("omarchy-install-preinstalls", "Install…"), ("omarchy-remove-preinstalls", "Remove…")] {
        if cmd::present(script) {
            let b = gtk::Button::with_label(label);
            b.connect_clicked(move |_| terminal(script));
            controls.append(&b);
        }
    }
    if controls.first_child().is_some() {
        g.add(&widgets::row(
            "Starter apps",
            "The apps Omarchy comes with, installed or removed together.",
            Some(controls.upcast_ref()),
        ));
        widgets::keywords("preinstalls default bundled bloat");
    }
}

// ----- Channel and update tools -----

fn channel(page: &Page) {
    if !cmd::present("omarchy-channel-set") {
        return;
    }
    let g = page.group("Updates");
    let current = cmd::output(&["omarchy-channel-current"]).unwrap_or_else(|| "unknown".into());
    let mut options = widgets::opts(&[("stable", "Stable"), ("rc", "Release candidate"), ("edge", "Edge")]);
    if !["stable", "rc", "edge"].contains(&current.as_str()) {
        options.insert(0, (current.clone(), current.clone()));
    }
    let (r, _) = widgets::choice_row(
        "Update channel",
        "Stable is tested the most. Edge gets changes first and may break. Switching opens a terminal.",
        options,
        &current,
        {
            let current = current.clone();
            move |c| {
                if c != current && ["stable", "rc", "edge"].contains(&c.as_str()) {
                    terminal(&format!("omarchy-channel-set {c}"));
                }
            }
        },
    );
    widgets::keywords("release edge rc beta omarchy version");
    g.add(&r);
}

fn maintenance(page: &Page) {
    let tools: &[(&str, &str, &str, &str)] = &[
        ("omarchy-update-firmware", "Firmware updates", "Update the firmware of your hardware.", "fwupd bios device"),
        ("omarchy-update-aur-pkgs", "AUR package updates", "Update the packages built from the AUR.", "yay aur"),
        (
            "omarchy-update-keyring",
            "Refresh package keys",
            "Fix 'invalid signature' errors when installing packages.",
            "pacman keyring gpg signature",
        ),
        ("omarchy-update-orphan-pkgs", "Remove unused packages", "Packages nothing needs any more.", "orphans cleanup clean"),
        ("omarchy-update-analyze-logs", "Check the last update", "Look through the update log for problems.", "log errors"),
    ];
    let tools: Vec<_> = tools.iter().filter(|t| cmd::present(t.0)).collect();
    if tools.is_empty() {
        return;
    }
    let g = page.collapsible("Maintenance", false);
    for (script, title, desc, words) in tools {
        let script = *script;
        let (r, _) = widgets::button_row(title, desc, "Run…", move |_| terminal(script));
        widgets::keywords(words);
        g.add(&r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_launcher() {
        let l = parse_launcher(
            "Google Docs",
            "[Desktop Entry]\nName=Google Docs\nExec=omarchy-launch-webapp \"https://docs.google.com\"\n",
        )
        .unwrap();
        assert_eq!(l.kind, Kind::Web("https://docs.google.com".into()));
        assert_eq!(l.name, "Google Docs");
    }

    #[test]
    fn terminal_launcher() {
        let l =
            parse_launcher("Disk Usage", "Name=Disk Usage\nExec=xdg-terminal-exec --app-id=TUI.float -e bash -c \"dua i /\"\n")
                .unwrap();
        assert_eq!(l.kind, Kind::Tui("bash -c \"dua i /\"".into(), true));
        let l = parse_launcher("Docker", "Exec=xdg-terminal-exec --app-id=TUI.tile -e omarchy-launch-docker-tui\n").unwrap();
        assert_eq!(l.kind, Kind::Tui("omarchy-launch-docker-tui".into(), false));
        assert_eq!(l.name, "Docker");
    }

    #[test]
    fn other_launchers_are_ignored() {
        assert!(parse_launcher("chromium", "Name=Chromium\nExec=chromium %U\n").is_none());
        assert!(parse_launcher("x", "Name=x\n").is_none());
    }

    #[test]
    fn package_names() {
        assert!(valid_package("lib32-gcc-libs") && valid_package("qt6-base") && valid_package("a+b"));
        assert!(!valid_package("-Rns") && !valid_package("a b") && !valid_package("a;b") && !valid_package(""));
    }
}
