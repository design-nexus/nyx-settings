//! Nyx themes on the Appearance page: the system theme (where Omarchy isn't the one
//! choosing it), the theme library (installing from GitHub, importing Omarchy
//! themes, updating, removing), each app's theme, and resetting appearance. The
//! theme engine (`nyx_theme::engine`) does the work; this is its face.

use crate::widgets::{self, Group, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use nyx_theme::engine::{self, Origin, Registry, Severity, installer};
use std::cell::RefCell;
use std::rc::Rc;

/// Colours that show a theme at a glance: background, panel, text, accent, then swatches.
fn sample(registry: &Registry, id: &str) -> Vec<String> {
    let Some(r) = engine::resolver::resolve_id(registry, id, None, &engine::A11y::default()) else { return vec![] };
    let mut out: Vec<String> =
        ["surface.primary", "surface.secondary", "text.primary", "accent.primary"].iter().map(|t| r.tokens.color(t).to_string()).collect();
    out.extend(r.theme.look.swatches.iter().take(5).map(|(_, c)| c.clone()));
    out
}

fn strip(colours: &[String]) -> gtk::Box {
    let strip = widgets::hbox(0);
    strip.add_css_class("theme-strip");
    strip.set_homogeneous(true);
    for c in colours {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
        b.set_vexpand(true);
        nyx_ui::widgets::paint(&b, c);
        strip.append(&b);
    }
    strip
}

fn esc(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}

/// Whether an Omarchy theme is choosing the desktop's colours (Omarchy is here).
pub fn omarchy_chooses() -> bool {
    nyx_theme::omarchy_available()
}

/// The system theme as a grid of cards, where Omarchy isn't the one choosing it.
pub fn system_group(page: &Page) {
    let g = page.plain_group("Theme");
    g.note("Applies to the whole desktop and every Nyx app that follows the system theme.");
    let registry = Registry::load();
    let active = engine::get_active_theme();
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_max_children_per_line(4);
    flow.set_min_children_per_line(3);
    flow.set_row_spacing(10);
    flow.set_column_spacing(10);
    flow.set_homogeneous(true);
    let cards: Rc<RefCell<Vec<(String, gtk::Button)>>> = Default::default();
    for e in registry.entries() {
        let id = e.meta.id.clone();
        let card = gtk::Button::new();
        card.add_css_class("theme-card");
        card.set_tooltip_text(Some(&e.meta.name));
        let v = widgets::vbox(0);
        let art = gtk::Overlay::new();
        let pic = gtk::Picture::new();
        pic.set_content_fit(gtk::ContentFit::Cover);
        pic.set_size_request(120, 68);
        pic.set_can_shrink(true);
        pic.set_hexpand(true);
        art.set_child(Some(&pic));
        match &e.meta.preview {
            Some(p) => pic.set_filename(Some(p)),
            None => art.add_overlay(&strip(&sample(&registry, &id))),
        }
        let tick = gtk::Image::from_icon_name("object-select-symbolic");
        tick.add_css_class("theme-tick");
        tick.set_halign(gtk::Align::End);
        tick.set_valign(gtk::Align::Start);
        art.add_overlay(&tick);
        v.append(&art);
        let l = widgets::label(&e.meta.name, "theme-card-name");
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        v.append(&l);
        card.set_child(Some(&v));
        if id == active {
            card.add_css_class("selected");
        }
        let cards2 = cards.clone();
        let name = e.meta.name.clone();
        card.connect_clicked(move |_| match nyx_theme::choose_system_theme(&id) {
            Ok(()) => {
                for (other, b) in cards2.borrow().iter() {
                    if *other == id {
                        b.add_css_class("selected");
                    } else {
                        b.remove_css_class("selected");
                    }
                }
                window::toast(&format!("Theme set to {name}"));
            }
            Err(e) => window::toast(&e),
        });
        cards.borrow_mut().push((e.meta.id.clone(), card.clone()));
        flow.insert(&card, -1);
    }
    // A theme chosen elsewhere (another app, the command line) moves the tick too.
    nyx_theme::subscribe(&flow, move || {
        let active = engine::get_active_theme();
        for (id, b) in cards.borrow().iter() {
            if *id == active {
                b.add_css_class("selected");
            } else {
                b.remove_css_class("selected");
            }
        }
    });
    let row = widgets::stacked_row("", "", flow.upcast_ref());
    row.add_css_class("bare");
    widgets::keywords("theme system theme catppuccin mocha default nyx dark light");
    g.add(&row);
}

/// Run `work` off the main thread with the button showing `label`, then `done`.
fn busy<T: Send + 'static>(b: &gtk::Button, label: &str, work: impl FnOnce() -> Result<T, String> + Send + 'static, done: impl FnOnce(T) + 'static) {
    let old = b.label().map(|l| l.to_string()).unwrap_or_default();
    b.set_sensitive(false);
    b.set_label(label);
    let b = b.clone();
    cmd::background(work, move |r| {
        b.set_sensitive(true);
        b.set_label(&old);
        match r {
            Ok(v) => done(v),
            Err(e) => window::toast(&e),
        }
    });
}

fn issues_text(issues: &[engine::Issue]) -> String {
    issues
        .iter()
        .map(|i| {
            let mark = if i.severity == Severity::Error { "✕" } else { "!" };
            let key = if i.key.is_empty() { String::new() } else { format!("<tt>{}</tt>: ", esc(&i.key)) };
            format!("{mark} {key}{}", esc(&i.message))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What a theme says about itself and where it came from, in a popover.
fn details_button(meta: &engine::Meta, issues: &[engine::Issue]) -> gtk::MenuButton {
    let mut lines = Vec::new();
    lines.push(format!("<b>{}</b> <tt>{}</tt>", esc(&meta.name), esc(&meta.id)));
    if let Some(d) = &meta.description {
        lines.push(esc(d));
    }
    let mut facts = Vec::new();
    if let Some(a) = &meta.author {
        facts.push(format!("By {}", esc(a)));
    }
    if let Some(v) = &meta.version {
        facts.push(format!("Version {}", esc(v)));
    }
    if let Some(l) = &meta.license {
        facts.push(format!("Licence {}", esc(l)));
    }
    facts.push(format!("{} theme", if meta.appearance == engine::Appearance::Light { "Light" } else { "Dark" }));
    lines.push(facts.join(" · "));
    if let Some(s) = &meta.source {
        lines.push(format!("From {} at <tt>{}</tt>", esc(&s.url), esc(s.commit.get(..10).unwrap_or(&s.commit))));
        lines.push(format!("Installed {}{}", esc(&s.installed), s.updated.as_deref().map(|u| format!(", updated {}", esc(u))).unwrap_or_default()));
    }
    if let Some(c) = &meta.conversion {
        lines.push(format!("Converted from an {} theme.", esc(&c.format)));
        for n in c.notes.iter().take(8) {
            lines.push(format!("· {}", esc(n)));
        }
    }
    if !meta.wallpapers.is_empty() {
        lines.push(format!("{} wallpaper(s); choose one under Wallpaper.", meta.wallpapers.len()));
    }
    if !issues.is_empty() {
        lines.push(issues_text(issues));
    }
    let label = gtk::Label::new(None);
    label.set_markup(&lines.join("\n"));
    label.set_wrap(true);
    label.set_max_width_chars(60);
    label.set_xalign(0.0);
    label.set_margin_top(10);
    label.set_margin_bottom(10);
    label.set_margin_start(12);
    label.set_margin_end(12);
    let pop = gtk::Popover::new();
    pop.set_child(Some(&label));
    let b = gtk::MenuButton::new();
    b.set_label("Details");
    b.set_popover(Some(&pop));
    b.set_valign(gtk::Align::Center);
    b
}

/// Themes beyond the built-in ones: installed, with the system, or the user's own files.
pub fn library_group(page: &Page) {
    let g = page.group("Theme library");
    g.note("Themes you installed or wrote yourself. Built-in themes, Catppuccin Mocha among them, are always there.");
    widgets::keywords("theme library installed themes update remove details author licence");
    let registry = Registry::load();
    let mut any = false;
    for e in registry.entries().iter().filter(|e| e.meta.origin != Origin::BuiltIn || e.meta.overridden) {
        any = true;
        let r = engine::resolver::resolve_id(&registry, &e.meta.id, None, &engine::A11y::default());
        let issues = r.map(|r| r.issues).unwrap_or_default();
        let warnings = issues.iter().filter(|i| i.severity == Severity::Warning).count();
        let errors = issues.iter().filter(|i| i.severity == Severity::Error).count();
        let from = match e.meta.origin {
            Origin::Installed => e
                .meta
                .source
                .as_ref()
                .map(|s| match s.url.strip_prefix("file://") {
                    Some(path) => format!("Imported from {path}"),
                    None => format!("From {}", s.url.trim_end_matches(".git")),
                })
                .unwrap_or_else(|| "Installed".into()),
            Origin::System => "Installed with the system".into(),
            Origin::User => "Your theme file".into(),
            Origin::BuiltIn => "Built in, with your changes".into(),
            Origin::Omarchy => "Omarchy".into(),
        };
        let mut desc = from;
        if let Some(a) = &e.meta.author {
            desc = format!("{desc} · by {a}");
        }
        if errors > 0 {
            desc = format!("{desc} · {errors} problem(s)");
        } else if warnings > 0 {
            desc = format!("{desc} · {warnings} readability warning(s)");
        }
        let control = widgets::hbox(6);
        control.append(&details_button(&e.meta, &issues));
        let id = e.meta.id.clone();
        match e.meta.origin {
            Origin::Installed if e.meta.source.as_ref().is_some_and(|s| !s.url.starts_with("file://")) => {
                let update = gtk::Button::with_label("Update");
                update.set_valign(gtk::Align::Center);
                let id2 = id.clone();
                update.connect_clicked(move |b| {
                    let id = id2.clone();
                    busy(b, "Updating…", move || installer::update(&id), move |r| match r {
                        installer::Updated::UpToDate => window::toast("Already up to date"),
                        installer::Updated::Updated { .. } => {
                            nyx_theme::themes_changed("theme-updated");
                            window::toast("Theme updated");
                            window::rebuild("theme");
                        }
                    });
                });
                control.append(&update);
            }
            Origin::User => {
                if let Some(path) = e.meta.path.clone() {
                    let edit = gtk::Button::with_label("Edit");
                    edit.set_valign(gtk::Align::Center);
                    edit.connect_clicked(move |_| nyx_core::cmd::open_in_editor(&path));
                    control.append(&edit);
                }
            }
            _ => {}
        }
        if e.meta.removable() {
            let remove = widgets::confirm_button("Remove", "Click again to remove", move |b| {
                let id = id.clone();
                busy(b, "Removing…", move || installer::remove(&id), |()| {
                    nyx_theme::themes_changed("theme-removed");
                    window::toast("Theme removed");
                    window::rebuild("theme");
                });
            });
            remove.add_css_class("destructive-action");
            control.append(&remove);
        }
        g.add(&widgets::row(&e.meta.name, &desc, Some(control.upcast_ref())));
    }
    for issue in registry.skipped.iter().take(5) {
        g.add(&widgets::row("A theme couldn't be read", &esc(&issue.to_string()), None));
    }
    if !any {
        g.add(&widgets::row("No themes added yet", "Install one from GitHub below.", None));
    }
    let (r, _) = widgets::button_row("Check for updates", "Ask each theme's repository whether it has a newer version.", "Check", |b| {
        busy(
            b,
            "Checking…",
            || {
                let ids: Vec<String> = engine::list_themes().into_iter().filter(|m| m.origin == Origin::Installed).map(|m| m.id).collect();
                Ok(ids.iter().filter_map(|id| installer::check_update(id).ok()).filter(|c| c.available()).map(|c| c.id).collect::<Vec<_>>())
            },
            |ids: Vec<String>| {
                window::toast(&if ids.is_empty() { "Every theme is up to date".to_string() } else { format!("Updates for {}", ids.join(", ")) })
            },
        )
    });
    g.add(&r);
}

/// Install from a GitHub repository: check first, then confirm.
pub fn install_group(page: &Page) {
    let g = page.group("Install a theme");
    g.note(
        "Paste a GitHub repository with Nyx themes (<tt>nyx-theme.toml</tt>) or Omarchy themes (<tt>colors.toml</tt>). \
         Themes are only data: nothing in the repository is run.",
    );
    widgets::keywords("install theme github url repository download add omarchy import");
    let url = gtk::Entry::new();
    url.set_placeholder_text(Some("https://github.com/owner/repository"));
    url.set_hexpand(true);
    let check = gtk::Button::with_label("Check");
    check.add_css_class("suggested-action");
    let line = widgets::hbox(8);
    line.append(&url);
    line.append(&check);
    g.add(&widgets::stacked_row("Repository", "", line.upcast_ref()));
    let results = widgets::vbox(6);
    results.set_visible(false);
    g.add(&results);
    let fetched: Rc<RefCell<Option<installer::Inspection>>> = Default::default();
    {
        let (entry, results, fetched) = (url.clone(), results.clone(), fetched.clone());
        let go = move |b: &gtk::Button| {
            let text = entry.text().trim().to_string();
            if text.is_empty() {
                entry.grab_focus();
                return;
            }
            let (results, fetched) = (results.clone(), fetched.clone());
            busy(b, "Checking…", move || installer::inspect(&text), move |ins| show_candidates(&results, ins, &fetched));
        };
        let check2 = check.clone();
        let go2 = go.clone();
        url.connect_activate(move |_| go2(&check2));
        check.connect_clicked(go);
    }
    let omarchy = engine::omarchy::installed_themes();
    if !omarchy.is_empty() {
        let options: Vec<(String, String)> = omarchy.iter().map(|(n, p)| (p.display().to_string(), engine::omarchy::pretty_name(n))).collect();
        let first = options[0].0.clone();
        let chosen = Rc::new(RefCell::new(first.clone()));
        let dd = nyx_ui::widgets::dropdown(&options, &first);
        {
            let chosen = chosen.clone();
            dd.connect_selected_notify(move |d| {
                if let Some((p, _)) = options.get(d.selected() as usize) {
                    *chosen.borrow_mut() = p.clone();
                }
            });
        }
        let import = gtk::Button::with_label("Import");
        import.connect_clicked(move |b| {
            let path = std::path::PathBuf::from(chosen.borrow().clone());
            busy(b, "Importing…", move || installer::import_omarchy(&path), |id| {
                nyx_theme::themes_changed("theme-imported");
                window::toast(&format!("Imported as {id}"));
                window::rebuild("theme");
            });
        });
        let line = widgets::hbox(8);
        line.append(&dd);
        line.append(&import);
        g.add(&widgets::row(
            "Import an Omarchy theme",
            "Its colours become a Nyx theme you can choose anywhere, with its wallpapers. Its scripts and app settings aren't used.",
            Some(line.upcast_ref()),
        ));
    }
}

fn show_candidates(results: &gtk::Box, ins: installer::Inspection, fetched: &Rc<RefCell<Option<installer::Inspection>>>) {
    while let Some(c) = results.first_child() {
        results.remove(&c);
    }
    results.set_visible(true);
    let short = ins.commit.get(..10).unwrap_or(&ins.commit).to_string();
    results.append(&widgets::row(&format!("{} theme(s) found", ins.candidates.len()), &format!("{} at <tt>{short}</tt>", esc(&ins.repo.url)), None));
    for (i, c) in ins.candidates.iter().enumerate() {
        let mut desc = format!("{} theme", if c.format == installer::Format::Omarchy { "Omarchy (converted)" } else { "Nyx" });
        if let Some(a) = &c.meta.author {
            desc.push_str(&format!(" by {}", esc(a)));
        }
        if !c.subpath.is_empty() {
            desc.push_str(&format!(" · <tt>{}</tt>", esc(&c.subpath)));
        }
        if !c.issues.is_empty() {
            desc.push('\n');
            desc.push_str(&issues_text(&c.issues));
        }
        let control = widgets::hbox(8);
        if !c.colors.is_empty() {
            let s = strip(&c.colors);
            s.set_size_request(120, 24);
            s.set_valign(gtk::Align::Center);
            control.append(&s);
        }
        if c.installable() {
            let b = gtk::Button::with_label(if c.updates { "Update" } else { "Install" });
            b.add_css_class("suggested-action");
            b.set_valign(gtk::Align::Center);
            let fetched = fetched.clone();
            b.connect_clicked(move |b| {
                let Some(ins) = fetched.borrow_mut().take() else {
                    window::toast("Check the repository again");
                    return;
                };
                busy(b, "Installing…", move || installer::install(&ins, &[i]), |ids| {
                    nyx_theme::themes_changed("theme-installed");
                    window::toast(&format!("Installed {}", ids.join(", ")));
                    window::rebuild("theme");
                });
            });
            control.append(&b);
        } else {
            control.append(&widgets::tag("Can't be installed"));
        }
        results.append(&widgets::row(&c.meta.name, &desc, Some(control.upcast_ref())));
    }
    *fetched.borrow_mut() = Some(ins);
}

/// The value of an app's theme row that follows the system theme.
const FOLLOW: &str = "";

/// Each Nyx app's theme: Follow system (the default) or one of its own.
pub fn apps_group(page: &Page) {
    let g = page.collapsible("App themes", false);
    g.note("A Nyx app follows the system theme unless you give it its own here. Panels and the bar always follow the system.");
    widgets::keywords("app theme per app override follow system each app different theme");
    let suite = engine::suite();
    let system = engine::resolve_effective_theme(None, &suite, &engine::A11y::default()).meta.name;
    let mut options = vec![(FOLLOW.to_string(), format!("Follow system ({system})"))];
    options.extend(engine::list_themes().into_iter().map(|m| (m.id, m.name)));
    let mut apps: Vec<(String, String)> = super::nyx::apps()
        .filter(|(_, _, program, _)| super::nyx::installed(program) || cmd::present(program))
        .map(|(_, name, program, _)| (program.to_string(), name.to_string()))
        .collect();
    for app in suite.applications.keys() {
        if !apps.iter().any(|(p, _)| p == app) {
            apps.push((app.clone(), app.clone()));
        }
    }
    for (app, name) in apps {
        let current = suite.app(&app).theme.filter(|_| suite.app(&app).mode == engine::AppMode::Override).unwrap_or_default();
        let app2 = app.clone();
        let (r, _) = widgets::choice_row(&name, "", options.clone(), &current, move |id| {
            let choice = (id != FOLLOW).then_some(id.as_str());
            if let Err(e) = nyx_theme::choose_app_theme_for(&app2, choice) {
                window::toast(&e);
            }
        });
        g.add(&r);
    }
}

/// Back to the defaults.
pub fn reset_group(g: &Group) {
    let b = widgets::confirm_button("Reset", "Click again to reset", |b| {
        busy(b, "Resetting…", engine::reset_appearance, |()| {
            nyx_theme::themes_changed("reset");
            window::toast("Appearance reset");
            window::rebuild("theme");
        });
    });
    b.add_css_class("destructive-action");
    g.add(&widgets::row(
        "Reset appearance",
        "The default theme (Omarchy's on Omarchy, else Catppuccin Mocha), every app following it, and no colour changes of your own. Installed themes stay.",
        Some(b.upcast_ref()),
    ));
    widgets::keywords("reset default appearance theme catppuccin restore");
}
