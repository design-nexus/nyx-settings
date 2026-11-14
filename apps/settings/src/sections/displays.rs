use super::arrange;
use crate::backend::devices::{self, LiveMonitor};
use crate::backend::state::{Monitor, State};
use crate::backend::{hypr, shell_core, store};
use crate::sketch;
use crate::widgets::{self, Page, opts};
use crate::{cmd, window};
use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};

const SCALES: &[f64] = &[1.0, 1.25, 1.333333, 1.5, 1.6, 1.666667, 1.75, 2.0, 2.5, 3.0];

fn scale_label(s: f64) -> String {
    format!("{}%", (s * 100.0).round() as i64)
}

/// After a display change, ask to keep it; put the old layout back if nobody answers.
fn confirm_or_revert(previous: BTreeMap<String, Monitor>) {
    keep_or_revert(
        15,
        || {},
        move || {
            let p = previous.clone();
            store::update(true, move |s| s.monitors = p);
            store::flush();
            glib::timeout_add_local_once(std::time::Duration::from_millis(900), || window::rebuild("displays"));
        },
    );
}

/// Ask whether to keep a change that was just made. `revert` runs on Revert, Escape, or
/// when nobody answers within `seconds`; `keep` on Keep.
pub(super) fn keep_or_revert(seconds: u32, keep: impl Fn() + 'static, revert: impl Fn() + 'static) {
    let Some(win) = window::window() else { return };
    let dialog = gtk::Window::builder().transient_for(&win).modal(true).title("Keep these display settings?").build();
    dialog.add_css_class("settings-window");
    dialog.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    let card = widgets::vbox(14);
    card.add_css_class("dialog-card");
    card.append(&widgets::label("Keep these display settings?", "section-title"));
    let msg = widgets::label(&format!("Going back in {seconds} seconds."), "dim");
    card.append(&msg);
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    let revert_button = gtk::Button::with_label("Go back");
    let keep_button = gtk::Button::with_label("Keep");
    keep_button.add_css_class("suggested-action");
    buttons.append(&revert_button);
    buttons.append(&keep_button);
    card.append(&buttons);
    dialog.set_child(Some(&card));

    let done = Rc::new(Cell::new(false));
    let revert = Rc::new(revert);
    let do_revert = {
        let dialog = dialog.clone();
        let done = done.clone();
        move || {
            if done.replace(true) {
                return;
            }
            revert();
            dialog.close();
        }
    };
    let r1 = do_revert.clone();
    revert_button.connect_clicked(move |_| r1());
    let d2 = dialog.clone();
    let done2 = done.clone();
    keep_button.connect_clicked(move |_| {
        if done2.replace(true) {
            return;
        }
        keep();
        d2.close();
    });
    let keys = gtk::EventControllerKey::new();
    let revert_on_escape = do_revert.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            revert_on_escape();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    dialog.add_controller(keys);
    let left = Rc::new(Cell::new(seconds as i32));
    let r2 = do_revert.clone();
    let done3 = done.clone();
    glib::timeout_add_seconds_local(1, move || {
        if done3.get() {
            return glib::ControlFlow::Break;
        }
        let n = left.get() - 1;
        left.set(n);
        msg.set_text(&format!("Going back in {n} seconds."));
        if n <= 0 {
            r2();
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    dialog.present();
}

/// The entry for the display on `connector`: kept by the display's identity (so it
/// follows it to another port), starting from how it runs now. One kept by port from
/// before moves over.
fn entry<'a>(s: &'a mut State, connector: &str, live: &[LiveMonitor], current: Option<Monitor>) -> &'a mut Monitor {
    let this = live.iter().find(|m| m.connector == connector);
    let key = this.map(|m| devices::monitor_key(m, live)).unwrap_or_else(|| connector.to_string());
    let old = (key != connector).then(|| s.monitors.remove(connector)).flatten();
    let m = s.monitors.entry(key.clone()).or_insert_with(|| old.unwrap_or_else(|| current.unwrap_or_default()));
    if let Some(t) = this {
        devices::describe(m, t, &key);
        devices::remember_monitor(t, &key);
    }
    m
}

fn change(output: &str, f: impl FnOnce(&mut Monitor)) {
    let previous = store::read(|s| s.monitors.clone());
    let current = live_monitor(output);
    let live = devices::live_monitors();
    store::update(true, |s| f(entry(s, output, &live, current)));
    store::flush();
    confirm_or_revert(previous);
}

/// The monitor as it runs now, as a starting point for an override.
fn live_monitor(name: &str) -> Option<Monitor> {
    let mons = hypr::json(&["monitors", "all"])?;
    let m = mons.as_array()?.iter().find(|m| m.get("name").and_then(|n| n.as_str()) == Some(name))?;
    Some(Monitor {
        mode: Some(format!(
            "{}x{}@{:.2}",
            m.get("width")?.as_i64()?,
            m.get("height")?.as_i64()?,
            m.get("refreshRate")?.as_f64()?
        )),
        position: Some(format!("{}x{}", m.get("x")?.as_i64()?, m.get("y")?.as_i64()?)),
        scale: m.get("scale").and_then(|v| v.as_f64()),
        transform: m.get("transform").and_then(|v| v.as_i64()),
        disabled: m.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false),
        ..Default::default()
    })
}

/// Give `w` its page's section colour as its CSS `color` (`.section-<key>`), for
/// outlines and fills drawn with `currentColor`.
pub(super) fn tint(w: &impl IsA<gtk::Widget>) {
    w.connect_realize(|w| {
        let key = std::iter::successors(w.parent(), |w| w.parent())
            .find_map(|w| w.css_classes().iter().find_map(|c| c.strip_prefix("colour-").map(str::to_string)));
        if let Some(k) = key {
            w.add_css_class(&format!("section-{k}"));
        }
    });
}

// ---------- What a display is called ----------

fn text<'a>(m: &'a Value, key: &str) -> &'a str {
    m.get(key).and_then(Value::as_str).unwrap_or("")
}

fn int(m: &Value, key: &str) -> i64 {
    m.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn disabled(m: &Value) -> bool {
    m.get("disabled").and_then(Value::as_bool).unwrap_or(false)
}

fn built_in(connector: &str) -> bool {
    ["eDP", "LVDS", "DSI"].iter().any(|p| connector.starts_with(p))
}

/// How it's connected, from the port's name.
fn port(connector: &str) -> String {
    let kind = connector.split('-').next().unwrap_or(connector);
    match kind {
        "HDMI" => "HDMI".into(),
        "DP" => "DisplayPort".into(),
        "DVI" => "DVI".into(),
        "VGA" => "VGA".into(),
        "HEADLESS" => format!("Virtual output · {connector}"),
        "WL" | "X11" => format!("Nested window · {connector}"),
        _ => connector.to_string(),
    }
}

/// "Laptop screen" for the built-in one; the maker and model for the rest.
fn title(m: &Value, live: &[LiveMonitor]) -> String {
    let name = text(m, "name");
    if built_in(name) && live.iter().filter(|l| built_in(&l.connector)).count() == 1 {
        return "Laptop screen".into();
    }
    live.iter()
        .find(|l| l.connector == name)
        .filter(|l| l.identified())
        .map(|l| l.label())
        .unwrap_or_else(|| name.to_string())
}

fn subtitle(m: &Value) -> String {
    let name = text(m, "name");
    let what = if built_in(name) { "Built in".to_string() } else { port(name) };
    if disabled(m) { format!("{what} · off") } else { what }
}

/// "2880 × 1800 · 175%".
fn mode_line(m: &Value) -> String {
    if disabled(m) {
        return "Off".into();
    }
    format!("{} × {} · {}", int(m, "width"), int(m, "height"), scale_label(m.get("scale").and_then(Value::as_f64).unwrap_or(1.0)))
}

// ---------- The page ----------

/// The pages one level down, opened from the More card.
#[derive(Clone, Copy, PartialEq)]
enum Sub {
    Rendering,
    Split,
    Remembered,
    Laptop,
}

thread_local! {
    /// The display whose settings are shown, kept when the page is rebuilt.
    static CHOSEN: RefCell<String> = const { RefCell::new(String::new()) };
    /// The page one level down that's open, so a rebuild opens it again.
    static OPEN: RefCell<Option<(Sub, gtk::Window)>> = const { RefCell::new(None) };
}

/// What follows the display that's chosen: its cards, its button in the picker, the
/// desk's outline and the scale sample.
#[derive(Default)]
struct Chooser {
    current: RefCell<String>,
    cards: RefCell<Vec<(String, Vec<gtk::Widget>)>>,
    buttons: RefCell<Vec<(String, gtk::ToggleButton)>>,
    /// With many displays, a list instead of the buttons.
    list: RefCell<Option<(gtk::DropDown, Vec<String>)>>,
    desk: RefCell<Option<arrange::Desk>>,
    sample: RefCell<Option<Rc<dyn Fn(&str)>>>,
}

impl Chooser {
    fn choose(&self, name: &str) {
        if *self.current.borrow() == name {
            return;
        }
        *self.current.borrow_mut() = name.to_string();
        CHOSEN.with(|c| *c.borrow_mut() = name.to_string());
        for (n, widgets) in self.cards.borrow().iter() {
            for w in widgets {
                w.set_visible(n == name);
            }
        }
        for (n, b) in self.buttons.borrow().iter() {
            if n == name && !b.is_active() {
                b.set_active(true);
            }
        }
        if let Some((dd, names)) = self.list.borrow().as_ref()
            && let Some(i) = names.iter().position(|n| n == name)
            && dd.selected() != i as u32
        {
            dd.set_selected(i as u32);
        }
        if let Some(d) = self.desk.borrow().as_ref() {
            d.select(name);
        }
        let sample = self.sample.borrow().clone();
        if let Some(s) = sample {
            s(name);
        }
    }
}

pub fn build(page: &Page) {
    // A split screen's regions are its virtual monitors, set up under Split a screen;
    // the screen itself (the host) is listed like any other.
    let monitors: Vec<Value> = hypr::json(&["monitors", "all"])
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|m| super::virtual_monitors::role(m) != "region")
        .collect();
    if monitors.is_empty() {
        page.banner("Couldn't read the displays from Hyprland.", true);
        return;
    }
    let live: Vec<LiveMonitor> = monitors.iter().filter_map(LiveMonitor::from_json).collect();
    // Settings kept by port from before follow the display now.
    let mut kept = store::read(|s| s.monitors.clone());
    if devices::adopt_monitors(&mut kept, &live) {
        store::update(true, move |s| s.monitors = kept);
    }
    // A page one level down that was open before the rebuild opens again over the new one.
    let reopen = OPEN.with(|o| o.borrow_mut().take()).map(|(sub, win)| {
        win.close();
        sub
    });
    let monitors = Rc::new(monitors);
    let live = Rc::new(live);

    let names: Vec<String> = monitors.iter().map(|m| text(m, "name").to_string()).collect();
    let chosen = CHOSEN.with(|c| c.borrow().clone());
    let chosen = if names.contains(&chosen) {
        chosen
    } else {
        monitors
            .iter()
            .find(|m| m.get("focused").and_then(Value::as_bool).unwrap_or(false))
            .or_else(|| monitors.iter().find(|m| !disabled(m)))
            .map(|m| text(m, "name").to_string())
            .unwrap_or_else(|| names[0].clone())
    };
    let chooser = Rc::new(Chooser::default());

    let pv = page.split();
    let sample = desk(&pv, &monitors, &live, &chosen, &chooser);
    // In a narrow window the desk is enough above the settings.
    let narrow = move |b: &gtk::Box| sample.set_visible(!b.has_css_class("narrow"));
    narrow(&page.body);
    page.body.connect_notify_local(Some("css-classes"), move |b, _| narrow(b));
    pv.foot("dialog-information-symbolic", "Changes ask “Keep these settings?” and go back on their own after 15 seconds.");

    picker(page, &monitors, &live, &chooser);
    let brightness = crate::platform::brightness();
    for m in monitors.iter() {
        let cards = display_cards(page, m, &monitors, &live, brightness);
        chooser.cards.borrow_mut().push((text(m, "name").to_string(), cards));
    }
    more(page, &monitors, &live);
    chooser.choose(&chosen);

    if let Some(sub) = reopen {
        let (m, l) = (monitors.clone(), live.clone());
        glib::idle_add_local_once(move || open(sub, m, l));
    }
}

/// The preview: Extend, Mirror or One screen and Identify, the desk, and how big
/// things are at the chosen display's scale.
fn desk(pv: &widgets::Preview, monitors: &Rc<Vec<Value>>, live: &[LiveMonitor], chosen: &str, chooser: &Rc<Chooser>) -> gtk::Box {
    let identify = gtk::Button::new();
    let inner = widgets::hbox(6);
    inner.append(&gtk::Image::from_icon_name("dialog-information-symbolic"));
    inner.append(&gtk::Label::new(Some("Identify")));
    identify.set_child(Some(&inner));
    identify.set_tooltip_text(Some("Show each display's name on the display itself"));
    identify.connect_clicked(|_| arrange::identify());
    pv.head("Your desk", "Click a screen to change it; drag it to move it.", &[]);
    // Under the heading, so they wrap rather than hold the preview wide.
    let tools = super::audio::wrap_box();
    tools.add_css_class("dsp-tools");
    if let Some(seg) = screens_mode(monitors) {
        tools.append(&seg);
    }
    tools.append(&identify);
    pv.add(&tools);
    widgets::keywords("identify which monitor name");

    let shown: Vec<&Value> = monitors.iter().filter(|m| !disabled(m)).collect();
    let rects: Vec<arrange::Rect> = shown
        .iter()
        .map(|m| {
            let num = |k: &str| m.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            let (w, h) = arrange::logical_size(num("width"), num("height"), num("scale"), int(m, "transform"));
            arrange::Rect { name: text(m, "name").to_string(), x: num("x"), y: num("y"), w, h }
        })
        .collect();
    let captions: Vec<arrange::Caption> =
        shown.iter().map(|m| arrange::Caption { title: title(m, live), detail: mode_line(m) }).collect();
    let weak = Rc::downgrade(chooser);
    let desk = arrange::canvas(
        rects,
        captions,
        chosen,
        move |name| {
            if let Some(c) = weak.upgrade() {
                c.choose(&name);
            }
        },
        |positions| {
            let previous = store::read(|s| s.monitors.clone());
            let current: Vec<(String, Option<Monitor>)> = positions.iter().map(|(n, _, _)| (n.clone(), live_monitor(n))).collect();
            let live = devices::live_monitors();
            store::update(true, move |s| {
                for ((name, x, y), (_, current)) in positions.iter().zip(current) {
                    entry(s, name, &live, current).position = Some(format!("{x}x{y}"));
                }
            });
            store::flush();
            confirm_or_revert(previous);
        },
    );
    pv.add(&desk.area);
    *chooser.desk.borrow_mut() = Some(desk);

    // How big text and controls are at the chosen screen's scale.
    let card = pv.card();
    card.add_css_class("dsp-sample");
    // The picture and its words side by side, or one over the other when narrow.
    let row = super::audio::wrap_box();
    card.append(&row);
    let left = widgets::vbox(6);
    let caption = widgets::label("", "dsp-sample-caption");
    caption.set_xalign(0.0);
    left.append(&caption);
    let scale_now = Rc::new(Cell::new(1.0));
    let drawing = {
        let s = scale_now.clone();
        widgets::drawing(132, move |cr, p, accent, w, h| draw_sample(cr, p, accent, w, h, s.get()))
    };
    drawing.set_content_width(210);
    drawing.set_hexpand(false);
    left.append(&drawing);
    row.append(&left);
    let right = widgets::vbox(6);
    right.set_valign(gtk::Align::Center);
    right.add_css_class("dsp-sample-words");
    right.append(&widgets::label("Text and controls", "dsp-sample-title"));
    let line = widgets::label("", "dsp-sample-line");
    line.set_wrap(true);
    line.set_xalign(0.0);
    right.append(&line);
    row.append(&right);

    let monitors = monitors.clone();
    let live: Vec<LiveMonitor> = live.to_vec();
    *chooser.sample.borrow_mut() = Some(Rc::new(move |name: &str| {
        let Some(m) = monitors.iter().find(|m| text(m, "name") == name) else { return };
        let scale = m.get("scale").and_then(Value::as_f64).unwrap_or(1.0);
        scale_now.set(scale);
        drawing.queue_draw();
        let who = title(m, &live);
        let who = if who == "Laptop screen" { "the laptop screen".to_string() } else { who };
        caption.set_text(&format!("At {} on {who}", scale_label(scale)));
        if disabled(m) {
            line.set_text("This display is off.");
            return;
        }
        let (w, h) = arrange::logical_size(int(m, "width") as f64, int(m, "height") as f64, scale, int(m, "transform"));
        let times = format!("{scale:.2}").trim_end_matches('0').trim_end_matches('.').to_string();
        line.set_text(&format!(
            "{} which leaves windows {} × {} of room. Higher makes things bigger and the room smaller.",
            if (scale - 1.0).abs() < 0.01 { "Drawn at their own size,".to_string() } else { format!("Drawn {times} times as big as at 100%,") },
            w.round() as i64,
            h.round() as i64
        ));
    }));
    card
}

/// A window drawn with text the size `scale` makes it.
fn draw_sample(cr: &gtk::cairo::Context, p: &sketch::Pal, accent: sketch::Rgb, w: f64, h: f64, scale: f64) {
    let k = scale.clamp(0.75, 3.0);
    sketch::rounded(cr, 0.5, 0.5, w - 1.0, h - 1.0, 8.0);
    sketch::set(cr, p.bg, 1.0);
    let _ = cr.fill_preserve();
    sketch::set(cr, p.dim, 0.3);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    cr.save().ok();
    sketch::rounded(cr, 0.5, 0.5, w - 1.0, h - 1.0, 8.0);
    cr.clip();
    let u = 7.0 * k;
    // The title bar: three dots and the window's name.
    let bar = u * 2.6;
    for i in 0..3 {
        sketch::set(cr, p.dim, 0.6);
        cr.arc(u * 1.4 + i as f64 * u * 1.1, bar / 2.0, u * 0.32, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
    cr.set_font_size(u * 1.15);
    sketch::set(cr, p.text, 1.0);
    cr.move_to(u * 5.0, bar / 2.0 + u * 0.4);
    let _ = cr.show_text("Files");
    sketch::set(cr, p.dim, 0.25);
    cr.rectangle(0.0, bar, w, 1.0);
    let _ = cr.fill();
    // Two folders.
    for (i, (name, colour)) in [("Documents", accent), ("Pictures", sketch::mix(accent, p.text, 0.35))].iter().enumerate() {
        let y = bar + u * 0.9 + i as f64 * u * 3.4;
        sketch::rounded(cr, u * 1.2, y, u * 2.4, u * 2.4, u * 0.5);
        sketch::set(cr, *colour, 0.9);
        let _ = cr.fill();
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
        cr.set_font_size(u * 1.1);
        sketch::set(cr, p.text, 1.0);
        cr.move_to(u * 4.4, y + u * 1.05);
        let _ = cr.show_text(name);
        cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
        cr.set_font_size(u * 0.85);
        sketch::set(cr, p.dim, 1.0);
        cr.move_to(u * 4.4, y + u * 2.25);
        let _ = cr.show_text("Folder");
    }
    cr.restore().ok();
}

/// Extend, Mirror or One screen: Omarchy's switches for a laptop's own screen, shown
/// while another display is plugged in.
fn screens_mode(monitors: &[Value]) -> Option<gtk::Box> {
    if !laptop_tools() || !cmd::present("omarchy-hyprland-monitor-internal-mirror") || monitors.len() < 2 {
        return None;
    }
    let current = if flag("internal-monitor-mirror") {
        "mirror"
    } else if flag("internal-monitor-disable") {
        "one"
    } else {
        "extend"
    };
    let seg = widgets::segmented(&opts(&[("extend", "Extend"), ("mirror", "Mirror"), ("one", "One screen")]), current, |v| {
        let mut steps: Vec<&'static [&'static str]> = Vec::new();
        let (mirrored, off) = (flag("internal-monitor-mirror"), flag("internal-monitor-disable"));
        match v.as_str() {
            "mirror" => {
                if off {
                    steps.push(&["omarchy-hyprland-monitor-internal", "on"]);
                }
                steps.push(&["omarchy-hyprland-monitor-internal-mirror", "on"]);
            }
            "one" => {
                if mirrored {
                    steps.push(&["omarchy-hyprland-monitor-internal-mirror", "off"]);
                }
                steps.push(&["omarchy-hyprland-monitor-internal", "off"]);
            }
            _ => {
                if mirrored {
                    steps.push(&["omarchy-hyprland-monitor-internal-mirror", "off"]);
                }
                if off {
                    steps.push(&["omarchy-hyprland-monitor-internal", "on"]);
                }
            }
        }
        run_steps(steps);
    });
    seg.set_tooltip_text(Some("Extend: the laptop screen and the others side by side. Mirror: the same picture on both. One screen: the laptop's own screen off."));
    Some(seg)
}

/// Run Omarchy's display commands one after another, then show the result.
fn run_steps(mut steps: Vec<&'static [&'static str]>) {
    if steps.is_empty() {
        return;
    }
    let first = steps.remove(0);
    cmd::run_async(first, move |r| {
        if let Err(e) = r {
            window::toast(&format!("{e}"));
        }
        if steps.is_empty() {
            glib::timeout_add_local_once(std::time::Duration::from_millis(900), || window::rebuild("displays"));
        } else {
            run_steps(steps);
        }
    });
}

/// The displays as buttons: the chosen one's settings are shown under them.
fn picker(page: &Page, monitors: &[Value], live: &[LiveMonitor], chooser: &Rc<Chooser>) {
    if monitors.len() > 4 {
        let g = page.group("Display");
        let options: Vec<(String, String)> =
            monitors.iter().map(|m| (text(m, "name").to_string(), format!("{} · {}", title(m, live), subtitle(m)))).collect();
        let dd = widgets::dropdown(&options, "");
        let names: Vec<String> = options.iter().map(|(n, _)| n.clone()).collect();
        let (weak, n2) = (Rc::downgrade(chooser), names.clone());
        dd.connect_selected_notify(move |d| {
            if let (Some(c), Some(name)) = (weak.upgrade(), n2.get(d.selected() as usize)) {
                c.choose(name);
            }
        });
        g.add(&widgets::icon_row("video-display-symbolic", "Settings for", "", Some(dd.upcast_ref())));
        *chooser.list.borrow_mut() = Some((dd, names));
        return;
    }
    let g = page.plain_group("Display");
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_homogeneous(true);
    flow.set_min_children_per_line(2);
    flow.set_max_children_per_line(2);
    flow.set_column_spacing(8);
    flow.set_row_spacing(8);
    flow.add_css_class("dsp-picker");
    let mut first: Option<gtk::ToggleButton> = None;
    for m in monitors {
        let name = text(m, "name").to_string();
        let b = gtk::ToggleButton::new();
        b.add_css_class("dsp-pick");
        tint(&b);
        if let Some(f) = &first {
            b.set_group(Some(f));
        } else {
            first = Some(b.clone());
        }
        let inner = widgets::hbox(10);
        let icon = gtk::Image::from_icon_name(if built_in(&name) { "computer-laptop-symbolic" } else { "video-display-symbolic" });
        icon.set_valign(gtk::Align::Center);
        inner.append(&icon);
        let words = widgets::vbox(1);
        words.set_valign(gtk::Align::Center);
        let t = widgets::label(&title(m, live), "dsp-pick-name");
        let d = widgets::label(&subtitle(m), "dsp-pick-detail");
        for l in [&t, &d] {
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_max_width_chars(18);
            words.append(l);
        }
        inner.append(&words);
        b.set_child(Some(&inner));
        b.set_tooltip_text(Some(&format!("{} · {}", text(m, "description"), name)));
        let weak: Weak<Chooser> = Rc::downgrade(chooser);
        let n = name.clone();
        b.connect_toggled(move |b| {
            if b.is_active()
                && let Some(c) = weak.upgrade()
            {
                c.choose(&n);
            }
        });
        flow.append(&b);
        chooser.buttons.borrow_mut().push((name, b));
    }
    g.add(&flow);
}

/// A display's settings, in the cards shown while it's chosen.
fn display_cards(page: &Page, m: &Value, monitors: &[Value], live: &[LiveMonitor], brightness: Option<f64>) -> Vec<gtk::Widget> {
    let name = text(m, "name").to_string();
    let this = live.iter().find(|l| l.connector == name).cloned();
    let key = this.as_ref().map(|t| devices::monitor_key(t, live)).unwrap_or_else(|| name.clone());
    let off = disabled(m);
    let mut out: Vec<gtk::Widget> = Vec::new();

    // The picture: resolution, refresh rate and rotation.
    let g = page.group("");
    out.push(g.wrapper.clone().upcast());
    if this.as_ref().is_some_and(|t| devices::ambiguous(t, live)) {
        g.note("Another connected display looks just the same (neither reports a serial number), so this one's settings are kept for this port.");
    }
    if let Some(problem) = store::read(|s| s.monitors.get(&key).cloned()).and_then(|saved| not_applied(&saved, m)) {
        g.note(&problem);
    }
    if monitors.len() > 1 || off {
        let n = name.clone();
        let sw = gtk::Switch::new();
        sw.set_active(!off);
        sw.connect_active_notify(move |s| {
            let on = s.is_active();
            change(&n, |mon| mon.disabled = !on)
        });
        g.add(&widgets::icon_row(
            "video-display-symbolic",
            "Use this display",
            if off { "Off now. Its settings come back when it's on." } else { "" },
            Some(sw.upcast_ref()),
        ));
        widgets::keywords("enable disable turn off monitor");
    }
    if off {
        return out;
    }

    let modes: Vec<(i64, i64, f64)> = m
        .get("availableModes")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().and_then(parse_mode)).collect())
        .unwrap_or_default();
    let (w, h, hz) = (int(m, "width"), int(m, "height"), m.get("refreshRate").and_then(Value::as_f64).unwrap_or(0.0));
    let mut sizes: Vec<(i64, i64)> = Vec::new();
    for (mw, mh, _) in &modes {
        if !sizes.contains(&(*mw, *mh)) {
            sizes.push((*mw, *mh));
        }
    }
    if !sizes.contains(&(w, h)) {
        sizes.push((w, h));
    }
    sizes.sort_by_key(|(a, b)| std::cmp::Reverse(a * b));
    let size_opts: Vec<(String, String)> = sizes.iter().map(|(a, b)| (format!("{a}x{b}"), format!("{a} × {b}"))).collect();
    {
        let (n, modes) = (name.clone(), modes.clone());
        let (r, _) = widgets::choice_row("Resolution", "", size_opts, &format!("{w}x{h}"), move |size| {
            let (sw, sh) = size.split_once('x').map(|(a, b)| (a.parse().unwrap_or(0), b.parse().unwrap_or(0))).unwrap_or((0, 0));
            // Keep the refresh rate if this size has it, else the fastest it has.
            let rate = modes
                .iter()
                .filter(|(a, b, _)| (*a, *b) == (sw, sh))
                .map(|(_, _, r)| *r)
                .min_by(|a, b| (a - hz).abs().total_cmp(&(b - hz).abs()).then(b.total_cmp(a)))
                .unwrap_or(hz);
            change(&n, |mon| mon.mode = Some(format!("{sw}x{sh}@{rate:.2}")));
        });
        r.prepend(&widgets::tile("video-display-symbolic"));
        r.add_css_class("with-tile");
        widgets::keywords("resolution mode size pixels");
        g.add(&r);
    }

    // The rates this resolution offers, rounded as people know them.
    let mut rates: Vec<f64> = modes.iter().filter(|(a, b, _)| (*a, *b) == (w, h)).map(|(_, _, r)| *r).collect();
    rates.sort_by(|a, b| a.total_cmp(b));
    rates.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    if !rates.iter().any(|r| (r - hz).abs() < 0.5) {
        rates.push(hz);
        rates.sort_by(|a, b| a.total_cmp(b));
    }
    let current_rate = rates.iter().copied().min_by(|a, b| (a - hz).abs().total_cmp(&(b - hz).abs())).unwrap_or(hz);
    let rate_opts: Vec<(String, String)> = rates
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let label = if i + 1 == rates.len() && rates.len() <= 4 { format!("{} Hz", r.round()) } else { format!("{}", r.round()) };
            (format!("{r:.2}"), label)
        })
        .collect();
    let set_rate = {
        let n = name.clone();
        move |r: String| change(&n, |mon| mon.mode = Some(format!("{w}x{h}@{r}")))
    };
    let rate_control: gtk::Widget = if rates.len() <= 4 {
        widgets::segmented(&rate_opts, &format!("{current_rate:.2}"), set_rate).upcast()
    } else {
        let labels: Vec<(String, String)> = rate_opts.iter().map(|(id, l)| (id.clone(), format!("{l} Hz"))).collect();
        let dd = widgets::dropdown(&labels, &format!("{current_rate:.2}"));
        dd.connect_selected_notify(move |d| {
            if let Some((id, _)) = labels.get(d.selected() as usize) {
                set_rate(id.clone());
            }
        });
        dd.upcast()
    };
    g.add(&widgets::icon_row("view-refresh-symbolic", "Refresh rate", "", Some(&rate_control)));
    widgets::keywords("refresh rate hz hertz fps smooth");

    let transform = int(m, "transform");
    let n = name.clone();
    let (r, _) = widgets::choice_row(
        "Rotation",
        "",
        opts(&[
            ("0", "Normal"),
            ("1", "90°"),
            ("2", "180°"),
            ("3", "270°"),
            ("4", "Flipped"),
            ("5", "Flipped 90°"),
            ("6", "Flipped 180°"),
            ("7", "Flipped 270°"),
        ]),
        &transform.to_string(),
        move |t| {
            let v: i64 = t.parse().unwrap_or(0);
            change(&n, |mon| mon.transform = Some(v));
        },
    );
    r.prepend(&widgets::tile("object-rotate-right-symbolic"));
    r.add_css_class("with-tile");
    widgets::keywords("rotate portrait transform");
    g.add(&r);

    // Size and light.
    let g = page.group("Size and light");
    out.push(g.wrapper.clone().upcast());
    let scale = m.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let mut scale_opts: Vec<(String, String)> = SCALES.iter().map(|s| (format!("{s}"), scale_label(*s))).collect();
    let current_scale = SCALES.iter().find(|s| (*s - scale).abs() < 0.01).map(|s| format!("{s}")).unwrap_or_else(|| {
        scale_opts.push((format!("{scale}"), scale_label(scale)));
        format!("{scale}")
    });
    let n = name.clone();
    let (r, _) = widgets::choice_row("Scale", "", scale_opts, &current_scale, move |s| {
        let v: f64 = s.parse().unwrap_or(1.0);
        change(&n, |mon| mon.scale = Some(v));
    });
    r.prepend(&widgets::tile("view-fullscreen-symbolic"));
    r.add_css_class("with-tile");
    widgets::keywords("hidpi zoom size bigger text sharp");
    g.add(&r);

    // The platform's brightness is the focused screen's.
    if let Some(b) = brightness.filter(|_| m.get("focused").and_then(|f| f.as_bool()).unwrap_or(false)) {
        let n = name.clone();
        let (r, slider) = widgets::slider_row("Brightness", "", (1.0, 100.0, 1.0), b, 0, "%", move |v| {
            // Omarchy's script reaches external screens too (DDC); elsewhere the backlight.
            if crate::capability::program("omarchy-brightness-display") {
                cmd::spawn(&["omarchy-brightness-display", "--no-osd", "--monitor", &n, &format!("{}%", v.round() as i64)]);
            } else {
                crate::platform::set_brightness(v.round() as i64);
            }
        });
        slider.scale.set_width_request(130);
        r.prepend(&widgets::tile("display-brightness-symbolic"));
        r.add_css_class("with-tile");
        widgets::keywords("brightness backlight dim bright");
        g.add(&r);
    }

    // Shell Core's own switch for this screen: the bar, dock and widgets on it.
    if let Some(t) = this.as_ref().filter(|t| shell_core::available() && t.identified()) {
        let id = t.id().key().to_string();
        let on = shell_core::screen(&id).and_then(|v| v.get("enabled").and_then(serde_json::Value::as_bool)).unwrap_or(true);
        let sw = gtk::Switch::new();
        sw.set_active(on);
        sw.connect_active_notify(move |s| {
            if let Err(e) = shell_core::set_screen(&id, s.is_active()) {
                window::toast(&e);
            }
        });
        g.add(&widgets::icon_row("user-desktop-symbolic", "Bar, dock and widgets", "Turn off for a TV or a projector.", Some(sw.upcast_ref())));
        widgets::keywords("desktop on this display bar dock widgets shell screen projector tv");
    }
    if g.list.first_child().is_none() {
        g.wrapper.set_visible(false);
    }

    // Where exactly it sits, and handing it back: rarely needed.
    let managed = store::read(|s| s.monitors.contains_key(&key) || s.monitors.contains_key(&name));
    if monitors.len() > 1 || managed {
        let g = page.collapsible("Position and reset", false);
        out.push(g.wrapper.clone().upcast());
        if monitors.len() > 1 {
            let pos = format!("{}x{}", int(m, "x"), int(m, "y"));
            let n = name.clone();
            let (r, e) = widgets::entry_row(
                "Position",
                "Like <tt>2560x0</tt> or <tt>auto-right</tt>.",
                &pos,
                "auto",
                move |p| change(&n, |mon| mon.position = Some(p.trim().to_string())),
            );
            e.set_width_chars(11);
            r.prepend(&widgets::tile("view-grid-symbolic"));
            r.add_css_class("with-tile");
            widgets::keywords("arrange arrangement position layout left right above below multiple monitors");
            g.add(&r);
        }
        if managed {
            let (n, k) = (name.clone(), key.clone());
            let (r, _) = widgets::button_row(
                "Settings manages this display",
                "Hand it back to the system's settings.",
                "Reset",
                move |_| {
                    let previous = store::read(|s| s.monitors.clone());
                    let (n, k) = (n.clone(), k.clone());
                    store::update(true, move |s| {
                        s.monitors.remove(&n);
                        s.monitors.remove(&k);
                    });
                    store::flush();
                    confirm_or_revert(previous);
                },
            );
            r.prepend(&widgets::tile("edit-undo-symbolic"));
            r.add_css_class("with-tile");
            g.add(&r);
        }
    }
    out
}

/// "2880x1800@120.00Hz" → (2880, 1800, 120.0).
fn parse_mode(s: &str) -> Option<(i64, i64, f64)> {
    let (size, hz) = s.trim_end_matches("Hz").split_once('@')?;
    let (w, h) = size.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?, hz.parse().ok()?))
}

// ---------- More: the pages one level down ----------

fn more(page: &Page, monitors: &Rc<Vec<Value>>, live: &Rc<Vec<LiveMonitor>>) {
    let g = page.group("More");
    let row = |sub: Sub, icon: &str, title: &str, value: &str| {
        let (m, l) = (monitors.clone(), live.clone());
        let (r, _) = widgets::nav_row(Some(icon), title, "", value, move || open(sub, m.clone(), l.clone()));
        r
    };
    g.add(&row(Sub::Rendering, "applications-engineering-symbolic", "Rendering", &vrr_summary()));
    widgets::keywords(
        "variable refresh rate vrr adaptive sync freesync gsync hardware software cursor pointer direct scanout fullscreen video game compositor igpu gpu performance",
    );
    if let Some(on) = super::virtual_monitors::summary() {
        let r = row(Sub::Split, "view-dual-symbolic", "Split a screen", if on { "On" } else { "Off" });
        if let Some(text) = r.first_child().and_then(|t| t.next_sibling()).and_downcast::<gtk::Box>() {
            let title = widgets::hbox(8);
            if let Some(l) = text.first_child() {
                text.remove(&l);
                title.append(&l);
            }
            let tag = widgets::tag("Experimental");
            tag.add_css_class("dsp-experimental");
            title.append(&tag);
            text.prepend(&title);
        }
        g.add(&r);
        widgets::keywords("virtual monitors split ultrawide wide screen regions divide three two experimental");
    }
    let remembered = devices::remembered_monitors(live);
    if !remembered.is_empty() {
        g.add(&row(Sub::Remembered, "document-open-recent-symbolic", "Remembered displays", &remembered.len().to_string()));
        widgets::keywords("remembered disconnected unplugged previous external monitor forget");
    }
    if laptop_tools() {
        let state = if flag("internal-monitor-mirror") {
            "Mirrored"
        } else if flag("internal-monitor-disable") {
            "Off"
        } else {
            "On"
        };
        g.add(&row(Sub::Laptop, "computer-laptop-symbolic", "Laptop screen", state));
        widgets::keywords("built-in screen internal laptop display eDP clamshell off disable mirror duplicate projector presentation recover black dark blank fix");
    }
}

/// What variable refresh is set to, for the Rendering row.
fn vrr_summary() -> String {
    let id = store::option("misc.vrr").or_else(|| hypr::get_option("misc.vrr")).and_then(|v| v.as_i64()).unwrap_or(0);
    match id {
        1 => "Variable refresh on",
        2 => "Variable refresh in full screen",
        3 => "Variable refresh for games",
        _ => "Variable refresh off",
    }
    .into()
}

fn open(sub: Sub, monitors: Rc<Vec<Value>>, live: Rc<Vec<LiveMonitor>>) {
    let title = match sub {
        Sub::Rendering => "Rendering",
        Sub::Split => "Split a screen",
        Sub::Remembered => "Remembered displays",
        Sub::Laptop => "Laptop screen",
    };
    let win = widgets::subpage(title, 600, |p| match sub {
        Sub::Rendering => rendering(p, &monitors, &live),
        Sub::Split => super::virtual_monitors::build(p, &monitors),
        Sub::Remembered => remembered(p, &live),
        Sub::Laptop => laptop_screen(p),
    });
    OPEN.with(|o| *o.borrow_mut() = Some((sub, win.clone())));
    win.connect_close_request(|w| {
        OPEN.with(|o| {
            let mine = o.borrow().as_ref().is_some_and(|(_, open)| open == w);
            if mine {
                *o.borrow_mut() = None;
            }
        });
        glib::Propagation::Proceed
    });
}

fn rendering(page: &Page, monitors: &[Value], live: &[LiveMonitor]) {
    let g = page.group("For every display");
    g.add(&widgets::hypr_choice_typed(
        "misc.vrr",
        "Variable refresh rate",
        "Smoother games and video when the frame rate dips, on screens that support it.",
        opts(&[("0", "Off"), ("1", "On"), ("2", "In full screen"), ("3", "For games and video")]),
        true,
    ));
    widgets::keywords("adaptive sync freesync gsync vrr power gpu");
    g.add(&widgets::hypr_choice_typed(
        "cursor.no_hardware_cursors",
        "Hardware cursor",
        "Less work for the pointer. Turn off if it leaves trails or disappears.",
        opts(&[("0", "When possible"), ("1", "Off"), ("2", "Automatic")]),
        true,
    ));
    widgets::keywords("hardware software cursor pointer igpu gpu performance");
    g.add(&widgets::hypr_choice_typed(
        "render.direct_scanout",
        "Direct to screen",
        "Full-screen games skip a step. Turn off if they flicker.",
        opts(&[("0", "Off"), ("1", "On"), ("2", "Automatic for games")]),
        true,
    ));
    widgets::keywords("direct scanout fullscreen video game compositor igpu gpu performance");

    // What they're doing on each screen right now.
    let g = page.group("Right now");
    for m in monitors.iter().filter(|m| !disabled(m)) {
        let line = |field: &str| match m.get(field).and_then(Value::as_bool) {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unknown",
        };
        let (r, _) = widgets::info_row(
            &title(m, live),
            &format!("Variable refresh active: {} · Hardware cursor in use: {}", line("vrr"), line("hardwareCursorsInUse")),
        );
        g.add(&r);
    }
}

/// Omarchy's commands for a laptop's own screen, on a laptop.
fn laptop_tools() -> bool {
    cmd::present("omarchy-hyprland-monitor-internal") && cmd::run(&["omarchy-hw-laptop"]).is_ok()
}

/// Omarchy keeps each of its display toggles as a flag file while it's in effect.
fn flag(name: &str) -> bool {
    crate::paths::home().join(format!(".local/state/omarchy/toggles/hypr/{name}.lua")).exists()
}

/// The built-in screen: off while an external monitor is in use, or mirrored onto it.
fn laptop_screen(page: &Page) {
    let g = page.group("Laptop screen");
    let run = |args: &'static [&'static str]| run_steps(vec![args]);
    let (r, _) = widgets::switch_row(
        "Built-in screen",
        "Turn the laptop's own screen off, for example with the lid closed on a desk.",
        !flag("internal-monitor-disable"),
        move |on| run(if on { &["omarchy-hyprland-monitor-internal", "on"] } else { &["omarchy-hyprland-monitor-internal", "off"] }),
    );
    g.add(&r);
    widgets::keywords("internal laptop display eDP clamshell off disable");
    if cmd::present("omarchy-hyprland-monitor-internal-mirror") {
        let (r, _) = widgets::switch_row(
            "Mirror to an external screen",
            "Show the laptop's screen on the connected monitor too, for presenting.",
            flag("internal-monitor-mirror"),
            move |on| {
                run(if on {
                    &["omarchy-hyprland-monitor-internal-mirror", "on"]
                } else {
                    &["omarchy-hyprland-monitor-internal-mirror", "off"]
                })
            },
        );
        g.add(&r);
        widgets::keywords("duplicate projector presentation external monitor");
    }
    let (r, _) = widgets::button_row(
        "Recover the built-in screen",
        "Use this if the laptop screen stays dark after unplugging a monitor.",
        "Recover",
        move |_| run(&["omarchy-hyprland-monitor-internal", "recover"]),
    );
    widgets::keywords("black dark blank fix");
    g.add(&r);
}

/// Displays set up here that aren't connected: their settings, changeable until they
/// come back, and Forget.
fn remembered(page: &Page, live: &[LiveMonitor]) {
    let list = devices::remembered_monitors(live);
    let g = page.group("Not connected");
    g.note("Displays you set up that aren't connected now. Their settings come back when they're plugged in again, on any port; you can change them here in the meantime.");
    widgets::keywords("remembered disconnected unplugged previous external monitor forget");
    if list.is_empty() {
        g.add(&widgets::row("None", "Every display you've set up is connected.", None));
    }
    for (key, m, profile) in list {
        let body = widgets::vbox(8);
        let scale = m.scale.unwrap_or(1.0);
        let mut scale_opts: Vec<(String, String)> = SCALES.iter().map(|s| (format!("{s}"), scale_label(*s))).collect();
        let current = SCALES.iter().find(|s| (*s - scale).abs() < 0.01).map(|s| format!("{s}")).unwrap_or_else(|| {
            scale_opts.push((format!("{scale}"), scale_label(scale)));
            format!("{scale}")
        });
        let k = key.clone();
        let (r, _) = widgets::choice_row("Scale", "", scale_opts, &current, move |v| {
            let v: f64 = v.parse().unwrap_or(1.0);
            offline(&k, move |m| m.scale = Some(v));
        });
        body.append(&r);
        let k = key.clone();
        let (r, _) = widgets::entry_row(
            "Position",
            "Where it sits beside the others, like <tt>2560x0</tt> or <tt>auto-right</tt>.",
            m.position.as_deref().unwrap_or("auto"),
            "auto",
            move |p| offline(&k, move |m| m.position = Some(p.trim().to_string())),
        );
        body.append(&r);
        let k = key.clone();
        let (r, _) = widgets::switch_row("Use this display", "", !m.disabled, move |on| offline(&k, move |m| m.disabled = !on));
        body.append(&r);
        let forget = gtk::Button::with_label("Forget this display");
        forget.set_halign(gtk::Align::Start);
        let label = m.label.clone().unwrap_or_else(|| key.clone());
        let k = key.clone();
        forget.connect_clicked(move |_| {
            let k = k.clone();
            crate::dialog::ask(
                &format!("Forget {label}?"),
                "Its scale, position and other settings go, here, in the dock and in the shell. It'll be like a new display next time it's plugged in.",
                Vec::new(),
                "Forget",
                None,
                move |_, _| {
                    match devices::forget(&nyx_devices::DeviceId::new(nyx_devices::Category::Monitor, &k)) {
                        Ok(()) => window::toast("Display forgotten"),
                        Err(e) => window::toast(&format!("Couldn't forget all of it: {e}")),
                    }
                    window::rebuild("displays");
                    None
                },
            );
        });
        body.append(&forget);
        let title = m.label.clone().unwrap_or_else(|| key.clone());
        g.add(&widgets::stacked_row(&title, &devices::last_seen(profile.as_ref()), body.upcast_ref()));
    }
}

/// What of a display's saved settings Hyprland isn't using (a mode it doesn't offer, a
/// scale it adjusted), so the page can say so instead of showing it as applied.
fn not_applied(saved: &Monitor, live: &Value) -> Option<String> {
    if saved.disabled || live.get("disabled").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let mut out = Vec::new();
    if let Some(want) = saved.scale {
        let have = live.get("scale").and_then(Value::as_f64).unwrap_or(want);
        if (have - want).abs() > 0.01 {
            out.push(format!("the scale is {} rather than the saved {}", scale_label(have), scale_label(want)));
        }
    }
    if let Some(mode) = saved.mode.as_deref().filter(|m| *m != "preferred") {
        let size = mode.split('@').next().unwrap_or(mode);
        let have = format!(
            "{}x{}",
            live.get("width").and_then(Value::as_i64).unwrap_or(0),
            live.get("height").and_then(Value::as_i64).unwrap_or(0)
        );
        if size != have {
            out.push(format!("it runs at {} rather than the saved {}", have.replace('x', " × "), size.replace('x', " × ")));
        }
    }
    (!out.is_empty()).then(|| {
        format!(
            "Not all of this display's saved settings took effect: {}. Hyprland may not support them on this screen.",
            out.join(", and ")
        )
    })
}

/// Change a display that isn't connected: saved for when it is (nothing to try now).
fn offline(key: &str, f: impl FnOnce(&mut Monitor) + 'static) {
    let key = key.to_string();
    store::update(true, move |s| {
        if let Some(m) = s.monitors.get_mut(&key) {
            f(m);
        }
    });
    store::flush();
}
