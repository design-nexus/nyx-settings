//! Printers through CUPS. Adding and removing needs the root helper; picking
//! your default and printing a test page don't.

use crate::dialog::{Field, ask};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use std::process::{Command, Stdio};

#[derive(Debug, PartialEq)]
struct Printer {
    name: String,
    state: String,
    uri: String,
}

/// `lpstat -p` ("printer NAME is idle.") joined with `lpstat -v` ("device for NAME: URI").
fn parse_printers(status: &str, devices: &str) -> Vec<Printer> {
    status
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("printer ")?;
            let (name, state) = rest.split_once(' ')?;
            let uri = devices.lines().find_map(|d| d.strip_prefix(&format!("device for {name}: "))).unwrap_or("").to_string();
            let state = state.trim_start_matches("is ").split('.').next().unwrap_or("").to_string();
            Some(Printer { name: name.to_string(), state, uri })
        })
        .collect()
}

/// `system default destination: NAME`
fn parse_default(text: &str) -> Option<String> {
    text.strip_prefix("system default destination: ").map(|n| n.trim().to_string())
}

/// A found printer: (display name, ipp address).
type Found = (String, String);

/// `avahi-browse -rtp _ipp._tcp` resolved lines: `=;iface;proto;name;type;domain;host;addr;port;"txt"`.
fn parse_found(text: &str) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    for line in text.lines().filter(|l| l.starts_with('=')) {
        let f: Vec<&str> = line.split(';').collect();
        if f.len() < 10 || f[2] != "IPv4" {
            continue;
        }
        let rp = f[9].split('"').find_map(|t| t.strip_prefix("rp=")).unwrap_or("ipp/print");
        let uri = format!("ipp://{}:{}/{}", f[7], f[8], rp.trim_start_matches('/'));
        if !found.iter().any(|(_, u)| *u == uri) {
            found.push((f[3].replace("\\032", " "), uri));
        }
    }
    found
}

/// A queue name CUPS accepts, made from a display name.
fn queue_name(display: &str) -> String {
    let mut s: String = display.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    s = s.trim_matches('_').to_string();
    if s.is_empty() { "Printer".into() } else { s.chars().take(60).collect() }
}

/// A job waiting to print: (job id like `Office-12`, who sent it).
type Job = (String, String);

/// `lpstat -o QUEUE`: `QUEUE-12  ken  1024  Fri 13 Nov 2026 ...`.
fn parse_jobs(text: &str, queue: &str) -> Vec<Job> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let id = f.next()?;
            let user = f.next().unwrap_or("").to_string();
            id.strip_prefix(queue).and_then(|r| r.strip_prefix('-')).map(|_| (id.to_string(), user))
        })
        .collect()
}

/// What CUPS's state means to a person, and whether it's a good one.
fn status(state: &str) -> (&'static str, &'static str, bool) {
    if state.starts_with("idle") {
        ("Ready", "Ready to print", true)
    } else if state.starts_with("now printing") || state.starts_with("printing") {
        ("Printing", "Printing now", true)
    } else if state.contains("disabled") {
        ("Paused", "Paused: it won't print until it's turned back on", false)
    } else {
        ("Not ready", "Not ready to print", false)
    }
}

/// How it's connected, from its address.
fn connection(uri: &str) -> &'static str {
    match uri.split(':').next().unwrap_or("") {
        "usb" => "By USB",
        "ipp" | "ipps" | "http" | "https" | "dnssd" | "socket" | "lpd" | "smb" => "On the network",
        "file" => "To a file",
        _ => "",
    }
}

fn shown_name(queue: &str) -> String {
    queue.replace('_', " ")
}

thread_local! {
    /// The printer shown on the right, kept when the page is rebuilt.
    static CHOSEN: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// A printer drawn: the paper tray at the top, the body with its light, a sheet out
/// of the front. `ok` lights it green, else amber; `on` false draws it faded.
fn draw_printer(cr: &gtk::cairo::Context, p: &crate::sketch::Pal, w: f64, h: f64, ok: bool, on: bool) {
    use crate::sketch::{mix, rounded, set};
    let a = if on { 1.0 } else { 0.45 };
    let s = (w / 1.4).min(h);
    let (x, y) = ((w - s * 1.3) / 2.0, (h - s) / 2.0);
    let bw = s * 1.3;
    // Paper waiting at the top.
    rounded(cr, x + bw * 0.2, y, bw * 0.6, s * 0.28, s * 0.03);
    set(cr, mix(p.surface, p.text, 0.25), a);
    let _ = cr.fill();
    // The body.
    let g = gtk::cairo::LinearGradient::new(0.0, y + s * 0.18, 0.0, y + s * 0.78);
    let top = mix(p.surface, p.text, 0.32);
    let bot = mix(p.surface, p.text, 0.16);
    g.add_color_stop_rgba(0.0, top.0, top.1, top.2, a);
    g.add_color_stop_rgba(1.0, bot.0, bot.1, bot.2, a);
    rounded(cr, x, y + s * 0.18, bw, s * 0.6, s * 0.1);
    let _ = cr.set_source(&g);
    let _ = cr.fill();
    // The slot and the light.
    set(cr, mix(p.bg, (0.0, 0.0, 0.0), 0.4), a);
    rounded(cr, x + bw * 0.15, y + s * 0.62, bw * 0.7, s * 0.05, s * 0.02);
    let _ = cr.fill();
    let led = if ok { (0.33, 0.82, 0.42) } else { (0.96, 0.66, 0.2) };
    cr.new_path();
    cr.arc(x + bw * 0.86, y + s * 0.33, (s * 0.045).max(2.0), 0.0, std::f64::consts::TAU);
    set(cr, led, if on { 1.0 } else { 0.3 });
    let _ = cr.fill();
    // A sheet coming out.
    rounded(cr, x + bw * 0.25, y + s * 0.64, bw * 0.5, s * 0.36, s * 0.02);
    set(cr, mix(p.text, (1.0, 1.0, 1.0), 0.6), 0.92 * a);
    let _ = cr.fill();
    set(cr, p.dim, 0.6 * a);
    for i in 0..2 {
        let ly = y + s * (0.78 + i as f64 * 0.08);
        cr.rectangle(x + bw * 0.32, ly, bw * if i == 0 { 0.32 } else { 0.22 }, (s * 0.02).max(1.0));
    }
    let _ = cr.fill();
}

/// A small printer, for the rows.
fn printer_icon(ok: bool) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(34);
    a.set_content_height(28);
    a.set_valign(gtk::Align::Center);
    a.set_draw_func(move |_, cr, w, h| draw_printer(cr, &crate::sketch::pal(), w as f64, h as f64, ok, true));
    a
}

/// A heading for the preview: the title, a line under it and something at its right.
fn preview_head(title: &str, line: &str, end: Option<&gtk::Widget>) -> gtk::Box {
    let head = widgets::hbox(10);
    head.add_css_class("preview-head");
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(title, "preview-title");
    t.set_xalign(0.0);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&t);
    if !line.is_empty() {
        let l = widgets::label(line, "preview-line");
        l.set_xalign(0.0);
        l.set_wrap(true);
        text.append(&l);
    }
    head.append(&text);
    if let Some(e) = end {
        e.set_valign(gtk::Align::Center);
        head.append(e);
    }
    head
}

pub fn build(page: &Page) {
    // On NyxOS the switch below turns printing on through the System Manager, which
    // brings CUPS and lpstat with it.
    if !cmd::present("lpstat") && crate::platform::PLATFORM == crate::platform::Platform::Omarchy {
        page.banner("Printing isn't installed. Install cups to use printers: sudo pacman -S cups", true);
        return;
    }
    let (can_edit, notice) = crate::platform::system_notice("printers", "adding and removing printers");
    if let Some(n) = notice {
        page.body.append(&n);
    }
    let pv = page.split();
    // What's shown on the right is rebuilt as printers are found and chosen.
    let shown = widgets::vbox(14);
    shown.set_vexpand(true);
    pv.add(&shown);

    let g = page.group("");
    let on = crate::platform::printing_on();
    let sw = gtk::Switch::new();
    sw.set_active(on);
    sw.connect_active_notify(move |s| {
        if s.is_active() != on {
            crate::platform::set_printing(s.is_active(), "printers");
        }
    });
    sw.set_sensitive(can_edit);
    g.add(&widgets::icon_row("printer-symbolic", "Printing", "Sends documents to printers. Off if you never print.", Some(sw.upcast_ref())));
    widgets::keywords("printing service cups print spooler");
    if !on {
        shown.append(&preview_head("Printing is off", "Turn it on to add a printer or print to one.", None));
        let card = widgets::vbox(0);
        card.add_css_class("preview-card");
        card.add_css_class("prn-hero");
        let pic = widgets::drawing(150, |cr, p, _, w, h| draw_printer(cr, p, w, h, false, false));
        card.append(&pic);
        shown.append(&card);
        return;
    }

    // Asking CUPS can take a moment the first time, so the page opens at once and
    // the printers appear when it answers.
    let g = page.group("Your printers");
    let holder = widgets::vbox(0);
    holder.append(&widgets::row("Looking for printers…", "", None));
    g.add(&holder);
    shown.append(&preview_head("Printers", "Looking for your printers…", None));
    cmd::background(
        || {
            // One call: each lpstat run costs about a second.
            // lpstat exits non-zero when there are no printers, so read stdout whatever the status.
            let all = Command::new("lpstat")
                .args(["-d", "-p", "-v"])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            (parse_printers(&all, &all), all.lines().find_map(parse_default))
        },
        move |(printers, default)| {
            // These rows are added after the page was built, so tell search where they belong.
            widgets::begin_section("printers");
            fill(&holder, &shown, can_edit, printers, default);
        },
    );

    if !can_edit {
        return;
    }
    add_group(page);
}

/// Finding printers on the network, and adding one by its address.
fn add_group(page: &Page) {
    let g = page.group("Add a printer");
    let results = widgets::vbox(0);
    let search = gtk::Button::new();
    let inner = widgets::hbox(6);
    inner.append(&gtk::Image::from_icon_name("system-search-symbolic"));
    let search_label = gtk::Label::new(Some("Search"));
    inner.append(&search_label);
    search.set_child(Some(&inner));
    {
        let results = results.clone();
        search.connect_clicked(move |b| {
            b.set_sensitive(false);
            search_label.set_text("Searching…");
            let (b, results, label) = (b.clone(), results.clone(), search_label.clone());
            cmd::background(
                || {
                    Command::new("timeout")
                        .args(["6", "avahi-browse", "-rtp", "_ipp._tcp"])
                        .stdin(Stdio::null())
                        .output()
                        .map(|o| parse_found(&String::from_utf8_lossy(&o.stdout)))
                        .unwrap_or_default()
                },
                move |found| {
                    b.set_sensitive(true);
                    label.set_text("Search again");
                    widgets::forget_rows(&results);
                    widgets::begin_section("printers");
                    while let Some(c) = results.first_child() {
                        results.remove(&c);
                    }
                    if found.is_empty() {
                        results.append(&widgets::row("No printers found", "Check that the printer is on and on the same network.", None));
                    }
                    for (name, uri) in found {
                        let add = gtk::Button::with_label("Add");
                        add.add_css_class("suggested-action");
                        let (queue, u) = (queue_name(&name), uri.clone());
                        add.connect_clicked(move |b| {
                            b.set_sensitive(false);
                            b.set_label("Adding…");
                            crate::platform::printer_add(&queue, &u, "printers");
                        });
                        let r = widgets::row(&name, &gtk::glib::markup_escape_text(&uri), Some(add.upcast_ref()));
                        r.prepend(&printer_icon(true));
                        results.append(&r);
                    }
                },
            );
        });
    }
    let r = widgets::icon_row("network-wireless-symbolic", "Find printers", "On your network, without a driver.", Some(search.upcast_ref()));
    widgets::keywords("add discover scan network ipp airprint avahi search");
    if !cmd::present("avahi-browse") {
        r.set_visible(false);
    }
    g.add(&r);
    g.add(&results);

    let (r, e) = widgets::entry_row("Add by address", "", "", "ipp://…", |uri| {
        let uri = uri.trim().to_string();
        if uri.is_empty() {
            return;
        }
        let host = uri.split("//").nth(1).and_then(|r| r.split(['/', ':']).next()).unwrap_or("Printer").to_string();
        crate::platform::printer_add(&queue_name(&host), &uri, "printers");
    });
    e.set_width_chars(16);
    e.set_tooltip_text(Some("Like ipp://192.168.1.50/ipp/print, then Enter"));
    r.prepend(&widgets::tile("applications-internet-symbolic"));
    r.add_css_class("with-tile");
    widgets::keywords("add printer address uri url ip");
    g.add(&r);
}

fn fill(holder: &gtk::Box, shown: &gtk::Box, can_edit: bool, printers: Vec<Printer>, default: Option<String>) {
    widgets::forget_rows(holder);
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    if printers.is_empty() {
        holder.append(&widgets::row("No printers yet", "Find one on your network below, or enter its address.", None));
        show_none(shown, can_edit);
        return;
    }
    let chosen = CHOSEN.with(|c| c.borrow().clone());
    let chosen = if printers.iter().any(|p| p.name == chosen) {
        chosen
    } else {
        default.clone().filter(|d| printers.iter().any(|p| p.name == *d)).unwrap_or_else(|| printers[0].name.clone())
    };
    let printers = std::rc::Rc::new(printers);
    let rows: std::rc::Rc<std::cell::RefCell<Vec<(String, gtk::Box)>>> = std::rc::Rc::default();
    let choose = {
        let (printers, rows, shown, default) = (printers.clone(), rows.clone(), shown.clone(), default.clone());
        std::rc::Rc::new(move |name: &str| {
            CHOSEN.with(|c| *c.borrow_mut() = name.to_string());
            for (n, r) in rows.borrow().iter() {
                if n == name {
                    r.add_css_class("prn-chosen");
                } else {
                    r.remove_css_class("prn-chosen");
                }
            }
            if let Some(p) = printers.iter().find(|p| p.name == name) {
                show_printer(&shown, p, default.as_deref() == Some(&p.name), can_edit);
            }
        })
    };
    for p in printers.iter() {
        let (word, _, ok) = status(&p.state);
        let is_default = default.as_deref() == Some(&p.name);
        let chevron = gtk::Image::from_icon_name("go-next-symbolic");
        chevron.add_css_class("chevron");
        let r = widgets::row(&shown_name(&p.name), word, Some(chevron.upcast_ref()));
        r.prepend(&printer_icon(ok));
        r.add_css_class("nav-row");
        r.add_css_class(if ok { "prn-ok" } else { "prn-warn" });
        if is_default {
            widgets::tag_row(&r, "Default");
        }
        widgets::keywords("printer print queue default test page");
        r.set_cursor_from_name(Some("pointer"));
        r.set_focusable(true);
        let click = gtk::GestureClick::new();
        let (c, n) = (choose.clone(), p.name.clone());
        click.connect_released(move |_, _, _, _| c(&n));
        r.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        let (c, n) = (choose.clone(), p.name.clone());
        keys.connect_key_pressed(move |_, key, _, _| {
            if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::space) {
                c(&n);
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
        r.add_controller(keys);
        holder.append(&r);
        rows.borrow_mut().push((p.name.clone(), r));
    }
    choose(&chosen);
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// No printers yet: a printer drawn, and how to add one.
fn show_none(shown: &gtk::Box, can_edit: bool) {
    clear(shown);
    shown.append(&preview_head("No printers yet", "Add one to print from any app.", None));
    let card = widgets::vbox(10);
    card.add_css_class("preview-card");
    card.add_css_class("prn-hero");
    card.append(&widgets::drawing(150, |cr, p, _, w, h| draw_printer(cr, p, w, h, true, false)));
    let l = widgets::label(
        if can_edit {
            "Search for printers on your network that don't need a driver, or add one by its address."
        } else {
            "Adding printers needs the system helper; see the note above."
        },
        "prn-hint",
    );
    l.set_wrap(true);
    l.set_justify(gtk::Justification::Center);
    card.append(&l);
    shown.append(&card);
}

/// The chosen printer: how it is, a test page, its address, what's waiting to print.
fn show_printer(shown: &gtk::Box, p: &Printer, is_default: bool, can_edit: bool) {
    clear(shown);
    let (word, sentence, ok) = status(&p.state);
    let pill = widgets::label("Default", "prn-default");
    super::displays::tint(&pill);
    let line = connection(&p.uri);
    shown.append(&preview_head(&shown_name(&p.name), line, is_default.then_some(pill.upcast_ref())));

    // How it is, and what to do with it.
    let hero = widgets::hbox(22);
    hero.add_css_class("preview-card");
    hero.add_css_class("prn-hero");
    let pic = {
        let ok = ok;
        widgets::drawing(130, move |cr, p, _, w, h| draw_printer(cr, p, w, h, ok, true))
    };
    pic.set_content_width(120);
    pic.set_hexpand(false);
    hero.append(&pic);
    let text = widgets::vbox(4);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let state = widgets::label(word, if ok { "prn-state-ok" } else { "prn-state-warn" });
    let big = widgets::label(sentence, "prn-sentence");
    for l in [&state, &big] {
        l.set_xalign(0.0);
        l.set_wrap(true);
        text.append(l);
    }
    let buttons = gtk::FlowBox::new();
    buttons.set_selection_mode(gtk::SelectionMode::None);
    buttons.set_column_spacing(8);
    buttons.set_row_spacing(8);
    buttons.set_max_children_per_line(3);
    buttons.add_css_class("prn-buttons");
    let test = gtk::Button::with_label("Print a test page");
    test.add_css_class("suggested-action");
    let name = p.name.clone();
    test.connect_clicked(move |_| {
        cmd::run_async(&["lp", "-d", &name, "/usr/share/cups/data/default-testpage.pdf"], |r| match r {
            Ok(_) => window::toast("Test page sent"),
            Err(e) => window::toast(&format!("{e}")),
        });
    });
    widgets::keywords("test page");
    buttons.append(&test);
    if can_edit {
        let edit = gtk::Button::with_label("Edit address");
        let (name, uri) = (p.name.clone(), p.uri.clone());
        edit.connect_clicked(move |_| {
            let name = name.clone();
            ask(
                "Edit printer address",
                "Where the printer is on the network.",
                vec![Field::text_with("ipp://…", &uri)],
                "Save",
                None,
                move |v, _| {
                    let uri = v[0].trim().to_string();
                    if uri.is_empty() {
                        return Some("Enter the printer's address.".into());
                    }
                    crate::platform::printer_set_uri(&name, &uri, "printers");
                    None
                },
            );
        });
        buttons.append(&edit);
    }
    if !is_default {
        let name = p.name.clone();
        let b = gtk::Button::with_label("Make default");
        b.set_tooltip_text(Some("Apps print to the default printer first"));
        b.connect_clicked(move |_| {
            cmd::run_async(&["lpoptions", "-d", &name], |r| {
                match r {
                    Ok(_) => window::toast("Default printer changed"),
                    Err(e) => window::toast(&format!("{e}")),
                }
                window::rebuild("printers");
            });
        });
        buttons.append(&b);
    }
    text.append(&buttons);
    hero.append(&text);
    shown.append(&hero);

    // Where it is.
    let facts = gtk::Grid::new();
    facts.add_css_class("preview-card");
    facts.add_css_class("prn-facts");
    facts.set_column_spacing(18);
    facts.set_row_spacing(10);
    facts.set_column_homogeneous(true);
    let mut at = 0;
    for (k, v) in [("Address", p.uri.clone()), ("Connected", connection(&p.uri).to_string()), ("Queue", p.name.clone())] {
        if v.is_empty() {
            continue;
        }
        let col = widgets::vbox(2);
        let a = widgets::label(k, "prn-fact-key");
        let b = widgets::label(&v, "prn-fact-value");
        b.set_selectable(true);
        b.set_can_focus(false);
        b.set_wrap(true);
        b.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        for l in [&a, &b] {
            l.set_xalign(0.0);
            col.append(l);
        }
        facts.attach(&col, at % 2, at / 2, 1, 1);
        at += 1;
    }
    shown.append(&facts);

    // What's waiting to print, asked of CUPS now.
    let queue = widgets::vbox(0);
    queue.add_css_class("preview-card");
    queue.add_css_class("prn-queue");
    let qhead = widgets::hbox(8);
    let qt = widgets::label("Waiting to print", "prn-queue-title");
    qt.set_hexpand(true);
    qt.set_xalign(0.0);
    qhead.append(&qt);
    let count = widgets::label("", "prn-fact-key");
    qhead.append(&count);
    queue.append(&qhead);
    let jobs_box = widgets::vbox(0);
    queue.append(&jobs_box);
    shown.append(&queue);
    let name = p.name.clone();
    cmd::background(
        {
            let name = name.clone();
            move || {
                Command::new("lpstat")
                    .args(["-o", &name])
                    .stdin(Stdio::null())
                    .stderr(Stdio::null())
                    .output()
                    .map(|o| parse_jobs(&String::from_utf8_lossy(&o.stdout), &name))
                    .unwrap_or_default()
            }
        },
        move |jobs| {
            count.set_text(&if jobs.is_empty() { String::new() } else { jobs.len().to_string() });
            if jobs.is_empty() {
                let l = widgets::label("Nothing. Documents you print wait here until they're done.", "prn-fact-key");
                l.set_xalign(0.0);
                l.set_wrap(true);
                jobs_box.append(&l);
            }
            for (id, user) in jobs {
                let cancel = gtk::Button::from_icon_name("window-close-symbolic");
                cancel.add_css_class("flat");
                cancel.set_tooltip_text(Some("Cancel"));
                let id2 = id.clone();
                cancel.connect_clicked(move |b| {
                    b.set_sensitive(false);
                    cmd::run_async(&["cancel", &id2], |r| match r {
                        Ok(_) => window::toast("Cancelled"),
                        Err(e) => window::toast(&format!("{e}")),
                    });
                });
                let number = id.rsplit('-').next().unwrap_or(&id).to_string();
                let r = widgets::row(&format!("Job {number}"), &format!("Sent by {user}"), Some(cancel.upcast_ref()));
                jobs_box.append(&r);
            }
        },
    );

    if can_edit {
        let spacer = widgets::vbox(0);
        spacer.set_vexpand(true);
        shown.append(&spacer);
        let name = p.name.clone();
        let remove = widgets::confirm_button("Remove this printer", "Click again to remove it", move |_| {
            crate::platform::printer_remove(&name, "printers")
        });
        remove.add_css_class("flat");
        remove.add_css_class("prn-remove");
        remove.set_halign(gtk::Align::End);
        widgets::keywords("remove delete printer");
        shown.append(&remove);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printers_and_default() {
        let p = parse_printers(
            "printer HP_Laser is idle.  enabled since Mon\nprinter Brother disabled since x\n",
            "device for HP_Laser: ipp://h/ipp/print\ndevice for Brother: usb://B\n",
        );
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], Printer { name: "HP_Laser".into(), state: "idle".into(), uri: "ipp://h/ipp/print".into() });
        assert_eq!(p[1].uri, "usb://B");
        assert_eq!(parse_default("system default destination: HP_Laser"), Some("HP_Laser".into()));
        assert_eq!(parse_default("no system default destination"), None);
    }

    #[test]
    fn found_printers() {
        let t = "+;eth0;IPv4;HP\\032LaserJet;_ipp._tcp;local\n=;eth0;IPv4;HP\\032LaserJet;_ipp._tcp;local;hp.local;192.168.1.50;631;\"rp=ipp/print\" \"ty=HP\"\n=;eth0;IPv6;HP;_ipp._tcp;local;hp.local;fe80::1;631;\"rp=ipp/print\"\n";
        let f = parse_found(t);
        assert_eq!(f, [("HP LaserJet".to_string(), "ipp://192.168.1.50:631/ipp/print".to_string())]);
    }

    #[test]
    fn jobs_for_one_printer() {
        let t = "Office-12  ken  1024  Fri 13 Nov 2026\nOffice_2-3  sam  10  Fri\n";
        assert_eq!(parse_jobs(t, "Office"), [("Office-12".to_string(), "ken".to_string())]);
    }

    #[test]
    fn queue_names() {
        assert_eq!(queue_name("HP LaserJet 400"), "HP_LaserJet_400");
        assert_eq!(queue_name("!!"), "Printer");
        assert_eq!(queue_name("192.168.1.5"), "192_168_1_5");
    }
}
