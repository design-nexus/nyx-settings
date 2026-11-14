//! Draws an extension's page from what `describe` prints, with the same rows as
//! built-in pages. Changes go back through `set`; the page is asked for again
//! when a row or the reply says so, and every `poll` seconds while it's showing.

use super::PageRef;
use super::protocol::{self, BlockKind, Kind, PageDesc, Row};
use crate::widgets::{self, Debounce, Page};
use crate::{cmd, fangraph, units, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(15);
const SET_TIMEOUT: Duration = Duration::from_secs(30);

struct State {
    page: Rc<PageRef>,
    section: &'static str,
    root: gtk::ScrolledWindow,
    header_desc: Option<gtk::Label>,
    content: gtk::Box,
    /// The last `describe` output, so polling only redraws when something changed.
    last: RefCell<String>,
    /// Polling holds off while the user is in the middle of changing something.
    busy_until: Cell<Instant>,
    polling: Cell<bool>,
    loading: Cell<bool>,
    next_row: Cell<usize>,
    row_widgets: RefCell<HashMap<usize, gtk::Widget>>,
    info_labels: RefCell<HashMap<usize, gtk::Label>>,
    /// The preview's readings (value, sub), in order, updated in place by polls.
    item_labels: RefCell<Vec<(gtk::Label, gtk::Label)>>,
}

type Shared = Rc<State>;

pub fn build(page: &Page, section: &'static str, r: Rc<PageRef>) {
    let content = widgets::vbox(0);
    let loading = widgets::label("Loading…", "dim");
    loading.set_margin_top(12);
    content.append(&loading);
    page.body.append(&content);
    // The description label in the page header, for the subtitle.
    let header_desc =
        page.body.first_child().and_then(|h| h.first_child()).and_then(|t| t.last_child()).and_downcast::<gtk::Label>();
    let st = Rc::new(State {
        page: r,
        section,
        root: page.root.clone(),
        header_desc,
        content,
        last: RefCell::default(),
        busy_until: Cell::new(Instant::now()),
        polling: Cell::new(false),
        loading: Cell::new(false),
        next_row: Cell::new(0),
        row_widgets: RefCell::default(),
        info_labels: RefCell::default(),
        item_labels: RefCell::default(),
    });
    load(&st, false);
}

fn load(st: &Shared, quiet: bool) {
    if st.loading.replace(true) {
        return;
    }
    let ext = st.page.ext.clone();
    let id = st.page.page.id.clone();
    let st = st.clone();
    cmd::background(
        move || ext.call(&["describe", &id], DESCRIBE_TIMEOUT).map_err(|e| format!("{e:#}")),
        move |r| {
            st.loading.set(false);
            match r {
                Ok(text) => {
                    if *st.last.borrow() == text {
                        return;
                    }
                    match protocol::parse_page(&text) {
                        Ok(desc) => {
                            if update_status(st.as_ref(), &text, &desc) {
                                *st.last.borrow_mut() = text;
                                return;
                            }
                            // Structural polls wait while an entry is being edited.
                            if quiet
                                && st
                                    .content
                                    .root()
                                    .and_then(|r| r.focus())
                                    .is_some_and(|w| w.is::<gtk::Text>() || w.is::<gtk::Entry>())
                            {
                                return;
                            }
                            *st.last.borrow_mut() = text;
                            render(&st, &desc);
                        }
                        Err(e) => show_error(&st, &format!("{} sent a page Settings can't read: {e}", st.page.ext.manifest.name)),
                    }
                }
                // A failed poll keeps what's shown; a failed first load says why.
                Err(e) if !quiet || st.last.borrow().is_empty() => show_error(&st, &e),
                Err(e) => eprintln!("nyx-settings: {e}"),
            }
        },
    );
}

/// Compare layout independently of changing read-only status values.
fn status_shape(text: &str) -> Option<serde_json::Value> {
    fn strip(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(object) => {
                if object.get("kind").and_then(|v| v.as_str()) == Some("info") {
                    object.remove("value");
                }
                object.remove("disabled");
                for child in object.values_mut() {
                    strip(child);
                }
            }
            serde_json::Value::Array(array) => {
                for child in array {
                    strip(child);
                }
            }
            _ => {}
        }
    }
    let mut value: serde_json::Value = serde_json::from_str(text).ok()?;
    if let Some(blocks) = value.pointer_mut("/preview/blocks").and_then(|b| b.as_array_mut()) {
        for block in blocks {
            for item in block.get_mut("items").and_then(|i| i.as_array_mut()).into_iter().flatten() {
                if let Some(o) = item.as_object_mut() {
                    o.remove("value");
                    o.remove("sub");
                    o.remove("tone");
                }
            }
        }
    }
    strip(&mut value);
    Some(value)
}
fn update_status(st: &State, text: &str, desc: &PageDesc) -> bool {
    let Some(old) = status_shape(&st.last.borrow()) else {
        return false;
    };
    if Some(old) != status_shape(text) {
        return false;
    }
    fn visit(st: &State, rows: &[Row], index: &mut usize) {
        for row in rows {
            let id = *index;
            *index += 1;
            if let Some(widget) = st.row_widgets.borrow().get(&id)
                && widget.is_sensitive() == row.disabled
            {
                widget.set_sensitive(!row.disabled);
            }
            if row.kind == Kind::Info
                && let Some(label) = st.info_labels.borrow().get(&id)
            {
                let value = row.str_value();
                if label.text().as_str() != value {
                    label.set_text(&value);
                }
            }
            if matches!(row.kind, Kind::Switch | Kind::Disclosure) {
                visit(st, &row.rows, index);
            }
        }
    }
    let mut index = 0;
    let blocks = desc.preview.as_ref().map(|p| p.blocks.as_slice()).unwrap_or_default();
    for b in blocks.iter().filter(|b| b.kind == BlockKind::Rows) {
        visit(st, &b.rows, &mut index);
    }
    for group in &desc.groups {
        visit(st, &group.rows, &mut index);
    }
    let labels = st.item_labels.borrow();
    let items = blocks.iter().filter(|b| has_readings(b.kind)).flat_map(|b| b.items.iter());
    for ((value, sub), item) in labels.iter().zip(items) {
        if value.text().as_str() != item.value {
            value.set_text(&item.value);
        }
        if sub.text().as_str() != item.sub {
            sub.set_text(&item.sub);
            sub.set_visible(!item.sub.is_empty());
        }
        for t in ["good", "warn", "bad", "accent"] {
            if t == item.tone { value.add_css_class(&format!("ext-{t}")) } else { value.remove_css_class(&format!("ext-{t}")) }
        }
    }
    true
}

fn clear(st: &Shared) {
    st.next_row.set(0);
    st.row_widgets.borrow_mut().clear();
    st.info_labels.borrow_mut().clear();
    st.item_labels.borrow_mut().clear();
    widgets::forget_rows(&st.content);
    while let Some(c) = st.content.first_child() {
        st.content.remove(&c);
    }
}

fn show_error(st: &Shared, msg: &str) {
    clear(st);
    st.last.borrow_mut().clear();
    st.content.append(&widgets::banner(&glib::markup_escape_text(msg), true));
    let again = gtk::Button::with_label("Try again");
    again.set_halign(gtk::Align::Start);
    again.set_margin_top(10);
    let s = st.clone();
    again.connect_clicked(move |_| load(&s, false));
    st.content.append(&again);
}

fn render(st: &Shared, desc: &PageDesc) {
    let scroll = st.root.vadjustment().value();
    clear(st);
    widgets::begin_section(st.section);
    if let Some(label) = &st.header_desc {
        let base = glib::markup_escape_text(&st.page.page.description).to_string();
        if desc.subtitle.is_empty() {
            label.set_text(&st.page.page.description);
        } else {
            label.set_markup(&format!("{base}\n{}", desc.subtitle));
        }
    }
    let page = Page::new(st.root.clone(), st.content.clone());
    match &desc.preview {
        Some(pv) => preview(st, &page.split(), pv),
        None => st.content.remove_css_class("split-body"),
    }
    for b in &desc.banners {
        page.banner(&b.text, b.warning);
    }
    for gd in &desc.groups {
        let g = if gd.collapsible { page.collapsible(&gd.title, gd.open_by_default) } else { page.group(&gd.title) };
        if !gd.note.is_empty() {
            g.note(&gd.note);
        }
        for row in &gd.rows {
            g.add(&row_widget(st, row));
        }
    }
    // Keep the reader's place after a redraw.
    let adj = st.root.vadjustment();
    glib::idle_add_local_once(move || adj.set_value(scroll));
    if desc.poll > 0 && !st.polling.replace(true) {
        start_polling(st, desc.poll);
    }
}

fn start_polling(st: &Shared, secs: u32) {
    let weak = Rc::downgrade(st);
    glib::timeout_add_seconds_local(secs.max(1), move || {
        let Some(st) = weak.upgrade() else { return glib::ControlFlow::Break };
        if st.content.root().is_none() {
            // The page was rebuilt or the window closed.
            return glib::ControlFlow::Break;
        }
        if st.content.is_mapped() && Instant::now() >= st.busy_until.get() {
            load(&st, true);
        }
        glib::ControlFlow::Continue
    });
}

fn touch(st: &Shared) {
    st.busy_until.set(Instant::now() + Duration::from_secs(4));
}

/// Send a change. `refresh` asks for the page again afterwards.
fn send(st: &Shared, key: &str, value: String, refresh: bool) {
    touch(st);
    let ext = st.page.ext.clone();
    let args = [st.page.page.id.clone(), key.to_string(), value];
    let st = st.clone();
    cmd::background(
        move || ext.call(&["set", &args[0], &args[1], &args[2]], SET_TIMEOUT).map_err(|e| format!("{e:#}")),
        move |r| match r {
            Ok(text) => {
                let reply = protocol::parse_reply(&text);
                if !reply.toast.is_empty() {
                    window::toast(&reply.toast);
                }
                if reply.reload {
                    // The sidebar is rebuilt, and this page with it.
                    cmd::background(super::refresh_pages, |_| window::reload_sections(false));
                } else if refresh || reply.refresh {
                    load(&st, false);
                }
            }
            Err(e) => {
                window::toast(&e);
                // Show what the device really has now.
                load(&st, true);
            }
        },
    );
}

fn sender(st: &Shared, row: &Row) -> Rc<dyn Fn(String)> {
    let (st, key, refresh) = (st.clone(), row.key.clone(), row.refresh);
    Rc::new(move |v: String| send(&st, &key, v, refresh))
}

fn decorate(r: &gtk::Box, row: &Row) {
    if !row.icon.is_empty() && r.orientation() == gtk::Orientation::Horizontal && !r.has_css_class("with-tile") {
        r.prepend(&widgets::tile(&row.icon));
        r.add_css_class("with-tile");
    }
    // Beside a wide control (segments, chips) a title mustn't break inside a short word.
    if r.orientation() == gtk::Orientation::Horizontal {
        let mut texts = r.first_child();
        while let Some(t) = texts.as_ref().filter(|t| t.has_css_class("row-tile")) {
            texts = t.next_sibling();
        }
        if let Some(title) = texts.and_then(|t| t.first_child()).and_downcast::<gtk::Label>() {
            title.set_width_chars(title.text().chars().count().min(10) as i32);
        }
    }
    if !row.keywords.is_empty() {
        widgets::keywords(&row.keywords);
    }
    if !row.tag.is_empty() {
        widgets::tag_row(r, &row.tag);
    }
    if !row.tooltip.is_empty() {
        r.set_tooltip_text(Some(&row.tooltip));
    }
}

fn children(st: &Shared, rows: &[Row]) -> gtk::Box {
    let b = widgets::vbox(6);
    for r in rows {
        b.append(&row_widget(st, r));
    }
    b
}

fn row_widget(st: &Shared, row: &Row) -> gtk::Widget {
    let id = st.next_row.get();
    st.next_row.set(id + 1);
    let widget = row_widget_inner(st, row, id);
    widget.set_sensitive(!row.disabled);
    st.row_widgets.borrow_mut().insert(id, widget.clone());
    widget
}
fn row_widget_inner(st: &Shared, row: &Row, id: usize) -> gtk::Widget {
    let send = sender(st, row);
    match row.kind {
        Kind::Info => {
            let (r, label) = widgets::info_row(&row.title, &row.str_value());
            st.info_labels.borrow_mut().insert(id, label);
            if !row.desc.is_empty() && row.tooltip.is_empty() {
                r.set_tooltip_text(Some(&row.desc));
            }
            decorate(&r, row);
            r.upcast()
        }
        Kind::Switch => {
            let shown = (!row.rows.is_empty()).then(|| widgets::vbox(6));
            let s2 = shown.clone();
            let (r, _) = widgets::switch_row(&row.title, &row.desc, row.bool_value(), move |on| {
                if let Some(b) = &s2 {
                    b.set_visible(on);
                }
                send(on.to_string());
            });
            decorate(&r, row);
            let Some(more) = shown else { return r.upcast() };
            for c in &row.rows {
                more.append(&row_widget(st, c));
            }
            more.set_visible(row.bool_value());
            let wrapper = widgets::vbox(6);
            wrapper.append(&r);
            wrapper.append(&more);
            wrapper.upcast()
        }
        Kind::Slider => slider(st, row, send).upcast(),
        Kind::Choice => {
            let (r, _) = widgets::choice_row(&row.title, &row.desc, row.options.clone(), &row.str_value(), move |v| send(v));
            decorate(&r, row);
            r.upcast()
        }
        Kind::Segmented => {
            let seg = widgets::segmented(&row.options, &row.str_value(), move |v| send(v));
            let r = if row.title.is_empty() && row.desc.is_empty() {
                widgets::stacked_row("", "", seg.upcast_ref())
            } else if row.options.len() > 4 || row.options.iter().map(|(_, l)| l.chars().count()).sum::<usize>() > 28 {
                widgets::stacked_row(&row.title, &row.desc, seg.upcast_ref())
            } else {
                widgets::row(&row.title, &row.desc, Some(seg.upcast_ref()))
            };
            decorate(&r, row);
            r.upcast()
        }
        Kind::Buttons => {
            let bx = widgets::hbox(6);
            for (id, label) in &row.options {
                let b = gtk::Button::with_label(label);
                let (send, id) = (send.clone(), id.clone());
                b.connect_clicked(move |_| send(id.clone()));
                bx.append(&b);
            }
            let r = widgets::row(&row.title, &row.desc, Some(bx.upcast_ref()));
            decorate(&r, row);
            r.upcast()
        }
        Kind::Entry => {
            let (r, _) = widgets::entry_row(&row.title, &row.desc, &row.str_value(), &row.placeholder, move |v| send(v));
            decorate(&r, row);
            r.upcast()
        }
        Kind::Button => {
            let label = if row.label.is_empty() { row.title.as_str() } else { row.label.as_str() };
            let b = if row.confirm.is_empty() {
                let b = gtk::Button::with_label(label);
                b.connect_clicked(move |_| send(String::new()));
                b
            } else {
                widgets::confirm_button(label, &row.confirm, move |_| send(String::new()))
            };
            if row.destructive {
                b.add_css_class("destructive-action");
            }
            let r = widgets::row(&row.title, &row.desc, Some(b.upcast_ref()));
            decorate(&r, row);
            r.upcast()
        }
        Kind::Colour => {
            let current = row.str_value();
            let r = if row.compact {
                let b = widgets::colour_button(&current, move |c| send(c));
                widgets::row(&row.title, &row.desc, Some(b.upcast_ref()))
            } else {
                let theme = row.theme.as_ref().map(|t| (t.label.as_str(), t.colour.as_str()));
                let p = widgets::colour_picker(&current, theme, move |c| send(c));
                widgets::stacked_row(&row.title, &row.desc, p.widget.upcast_ref())
            };
            decorate(&r, row);
            r.upcast()
        }
        Kind::Chips => {
            let labels: Vec<&str> = row.labels.iter().map(String::as_str).collect();
            let chips = widgets::chip_toggles(&labels, &row.bools(), move |v| send(protocol::encode_bools(&v)));
            let r = if labels.iter().map(|l| l.chars().count()).sum::<usize>() > 24 {
                widgets::stacked_row(&row.title, &row.desc, chips.upcast_ref())
            } else {
                widgets::row(&row.title, &row.desc, Some(chips.upcast_ref()))
            };
            decorate(&r, row);
            r.upcast()
        }
        Kind::Curve => curve(st, row).upcast(),
        Kind::Camera => {
            let r = super::media::camera(&row.title, &row.desc, &row.device);
            decorate(&r, row);
            r.upcast()
        }
        Kind::Meter => {
            let r = super::media::meter(&row.title, &row.desc, &row.source);
            decorate(&r, row);
            r.upcast()
        }
        Kind::Unknown => {
            let (r, _) =
                widgets::info_row(if row.title.is_empty() { "Unsupported row" } else { &row.title }, "Needs a newer Settings");
            r.upcast()
        }
        Kind::Disclosure => {
            let (wrapper, content) = widgets::disclosure(&row.title, &row.desc);
            if !row.icon.is_empty()
                && let Some(inner) = wrapper.first_child().and_then(|b| b.first_child()).and_downcast::<gtk::Box>()
            {
                inner.prepend(&widgets::tile(&row.icon));
                wrapper.add_css_class("with-tile");
            }
            if !row.keywords.is_empty() {
                widgets::keywords(&row.keywords);
            }
            content.append(&children(st, &row.rows));
            let key = format!("{}/disclosure/{}", st.section, if row.key.is_empty() { &row.title } else { &row.key });
            if crate::prefs::get().open_groups.get(&key).copied().unwrap_or(false) {
                widgets::open_disclosure(&wrapper);
            }
            if let Some(button) = wrapper.first_child().and_downcast::<gtk::Button>() {
                let content = content.clone();
                button.connect_clicked(move |_| {
                    let open = content.is_visible();
                    let key = key.clone();
                    crate::prefs::update(|p| {
                        p.open_groups.insert(key, open);
                    });
                });
            }
            wrapper.upcast()
        }
    }
}

fn slider(st: &Shared, row: &Row, send: Rc<dyn Fn(String)>) -> gtk::Box {
    // Temperatures arrive and leave in °C; the user sees their own unit.
    let temp = row.temperature;
    let show = move |c: f64| if temp { units::from_celsius(c) } else { c };
    let back = move |v: f64| -> String {
        if temp {
            units::to_celsius(v).to_string()
        } else {
            let digits = 10f64.powi(6);
            ((v * digits).round() / digits).to_string()
        }
    };
    let unit = if temp { format!("\u{a0}{}", units::symbol()) } else { row.unit.clone() };
    let step = if temp {
        units::step()
    } else if row.step > 0.0 {
        row.step
    } else {
        1.0
    };
    let (lo, hi) = (show(row.min), show(row.max.max(row.min + step)));
    let (slider_box, s) = widgets::slider(lo, hi, step, show(row.f64_value()).clamp(lo, hi), row.digits, &unit);
    // Beside an icon, a title and the value, a slider fits the controls column narrower.
    s.scale.set_width_request(110);

    if row.on_release {
        let click = gtk::GestureClick::new();
        let (scale, send2) = (s.scale.clone(), send.clone());
        click.connect_released(move |_, _, _, _| send2(back(scale.value())));
        s.scale.add_controller(click);
        let key = gtk::EventControllerKey::new();
        let (scale, send2) = (s.scale.clone(), send.clone());
        key.connect_key_released(move |_, _, _, _| send2(back(scale.value())));
        s.scale.add_controller(key);
        let st2 = st.clone();
        s.scale.connect_value_changed(move |_| touch(&st2));
    } else {
        let debounce = Debounce::default();
        let (send2, st2) = (send.clone(), st.clone());
        s.scale.connect_value_changed(move |sc| {
            touch(&st2);
            let (send, v) = (send2.clone(), back(sc.value()));
            debounce.call(400, move || send(v));
        });
    }

    let r = if row.marks.is_empty() {
        widgets::row(&row.title, &row.desc, Some(slider_box.upcast_ref()))
    } else {
        // A full-width slider, its quick picks under it (a narrow column can't fit both).
        slider_box.set_hexpand(true);
        s.scale.set_hexpand(true);
        let line = widgets::vbox(8);
        line.append(&slider_box);
        let picks = widgets::hbox(8);
        picks.set_halign(gtk::Align::End);
        line.append(&picks);
        for m in &row.marks {
            let b = gtk::Button::with_label(&widgets::format_value(show(*m), row.digits, &unit));
            b.add_css_class("chip");
            b.set_valign(gtk::Align::Center);
            let (scale, send, m, on_release) = (s.scale.clone(), send.clone(), *m, row.on_release);
            b.connect_clicked(move |_| {
                scale.set_value(show(m));
                // No release event follows a chip, so send it here (a live slider sends on change).
                if on_release {
                    send(back(show(m)));
                }
            });
            picks.append(&b);
        }
        widgets::stacked_row(&row.title, &row.desc, line.upcast_ref())
    };
    if let Some(d) = row.reset {
        let reset = gtk::Button::from_icon_name("edit-undo-symbolic");
        reset.add_css_class("reset-button");
        reset.add_css_class("flat");
        reset.set_valign(gtk::Align::Center);
        reset.set_tooltip_text(Some(&format!("Reset to the default ({})", widgets::format_value(show(d), row.digits, &unit))));
        // Only offered while the value differs from the default.
        let at_default = move |v: f64| (v - show(d)).abs() < step / 2.0;
        reset.set_visible(!at_default(s.scale.value()));
        let r2 = reset.clone();
        s.scale.connect_value_changed(move |sc| r2.set_visible(!at_default(sc.value())));
        let (scale, send) = (s.scale.clone(), send.clone());
        let on_release = row.on_release;
        reset.connect_clicked(move |_| {
            scale.set_value(show(d));
            if on_release {
                send(back(show(d)));
            }
        });
        r.insert_child_after(&reset, r.first_child().as_ref());
    }
    decorate(&r, row);
    r
}

fn curve(st: &Shared, row: &Row) -> gtk::Box {
    let series = Rc::new(RefCell::new(row.series.clone()));
    if series.borrow().is_empty() {
        return widgets::row(&row.title, "No curves to show.", None);
    }
    let selected = Rc::new(Cell::new(0usize));
    let debounces: Rc<Vec<Debounce>> = Rc::new((0..series.borrow().len()).map(|_| Debounce::default()).collect());
    let send_later = {
        let (st, series, debounces, key) = (st.clone(), series.clone(), debounces.clone(), row.key.clone());
        Rc::new(move |i: usize| {
            touch(&st);
            let s = series.borrow()[i].clone();
            let (st, key) = (st.clone(), format!("{key}/{}", s.id));
            debounces[i].call(500, move || send(&st, &key, protocol::encode_points(&s.points), false));
        })
    };
    let graph = {
        let first = series.borrow()[0].points.clone();
        let (series, selected, send_later) = (series.clone(), selected.clone(), send_later.clone());
        Rc::new(fangraph::fan_graph(first, move |p| {
            let i = selected.get();
            series.borrow_mut()[i].points = p.clone();
            send_later(i);
        }))
    };

    let head = widgets::hbox(10);
    let n = series.borrow().len();
    let copy = gtk::Button::with_label("");
    let copy_label = {
        let series = series.clone();
        move |i: usize| {
            let all = series.borrow();
            let others: Vec<&str> = all.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, s)| s.label.as_str()).collect();
            if others.len() == 1 { format!("Copy to {}", others[0]) } else { "Copy to all".to_string() }
        }
    };
    if n > 1 {
        let opts: Vec<(String, String)> =
            series.borrow().iter().enumerate().map(|(i, s)| (i.to_string(), s.label.clone())).collect();
        let (graph, series, selected, copy, copy_label) =
            (graph.clone(), series.clone(), selected.clone(), copy.clone(), copy_label.clone());
        head.append(&widgets::segmented(&opts, "0", move |id| {
            let i: usize = id.parse().unwrap_or(0);
            selected.set(i);
            graph.set(series.borrow()[i].points.clone());
            copy.set_label(&copy_label(i));
        }));
    }
    let tools = widgets::hbox(10);
    if row.presets {
        for (id, label) in [("quiet", "Quiet"), ("balanced", "Balanced"), ("max", "Maximum")] {
            let b = gtk::Button::with_label(label);
            b.add_css_class("chip");
            b.set_tooltip_text(Some("Set the speeds to a preset shape (temperatures stay as they are)"));
            let (graph, series, selected, send_later) = (graph.clone(), series.clone(), selected.clone(), send_later.clone());
            b.connect_clicked(move |_| {
                let i = selected.get();
                {
                    let mut all = series.borrow_mut();
                    let speeds = fangraph::preset(id, all[i].points.len());
                    for (p, s) in all[i].points.iter_mut().zip(speeds) {
                        p.1 = s;
                    }
                }
                graph.set(series.borrow()[i].points.clone());
                send_later(i);
            });
            tools.append(&b);
        }
    }
    if n > 1 {
        let spacer = widgets::hbox(0);
        spacer.set_hexpand(true);
        head.append(&spacer);
        copy.set_label(&copy_label(0));
        copy.add_css_class("chip");
        let (series, selected, send_later) = (series.clone(), selected.clone(), send_later.clone());
        copy.connect_clicked(move |_| {
            let i = selected.get();
            let src = series.borrow()[i].points.clone();
            for j in (0..series.borrow().len()).filter(|j| *j != i) {
                series.borrow_mut()[j].points = src.clone();
                send_later(j);
            }
            window::toast("Copied");
        });
        head.append(&copy);
    }

    let body = widgets::vbox(10);
    if head.first_child().is_some() {
        body.append(&head);
    }
    body.append(&graph.area);
    if tools.first_child().is_some() {
        body.append(&tools);
    }
    let hint = if row.hint.is_empty() {
        "Drag a point up or down for speed, left or right for temperature. Speeds can't drop as it gets hotter."
    } else {
        row.hint.as_str()
    };
    let hint = widgets::label(hint, "dim");
    hint.set_wrap(true);
    body.append(&hint);
    let r = widgets::stacked_row(&row.title, &row.desc, body.upcast_ref());
    decorate(&r, row);
    r
}

/// Blocks whose items are readings a poll updates in place.
fn has_readings(kind: BlockKind) -> bool {
    matches!(kind, BlockKind::Device | BlockKind::Stats | BlockKind::Facts | BlockKind::List | BlockKind::Picture)
}

fn accent_of(b: &protocol::Block) -> Option<crate::sketch::Rgb> {
    let hex = b.accent.trim_start_matches('#');
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| fangraph::rgb(&b.accent))
}

/// A device drawn in the theme's colours, its lit parts in the block's accent (or the
/// theme's).
fn picture(b: &protocol::Block, height: i32) -> gtk::DrawingArea {
    let (kind, accent) = (b.picture.clone(), accent_of(b));
    widgets::drawing(height, move |cr, p, _, w, h| super::pictures::draw(cr, p, accent.unwrap_or(p.accent), &kind, w, h))
}

/// A reading: its label small, the value large in its tone, a line under it.
fn reading(st: &Shared, item: &protocol::Item) -> gtk::Box {
    let b = widgets::vbox(1);
    b.add_css_class("ext-reading");
    let l = widgets::label(&item.label, "ext-reading-label");
    l.set_xalign(0.0);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    b.append(&l);
    let v = widgets::label(&item.value, "ext-reading-value");
    v.set_xalign(0.0);
    v.set_ellipsize(gtk::pango::EllipsizeMode::End);
    if !item.tone.is_empty() {
        v.add_css_class(&format!("ext-{}", item.tone));
    }
    b.append(&v);
    let sub = widgets::label(&item.sub, "ext-reading-sub");
    sub.set_xalign(0.0);
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    sub.set_visible(!item.sub.is_empty());
    b.append(&sub);
    st.item_labels.borrow_mut().push((v, sub));
    b
}

fn readings(st: &Shared, items: &[protocol::Item], most: usize) -> gtk::Grid {
    let g = gtk::Grid::new();
    g.add_css_class("ext-readings");
    g.set_column_homogeneous(true);
    g.set_column_spacing(14);
    g.set_row_spacing(10);
    // Four readings sit two by two; a narrow pane can't fit four across.
    let cols = if items.len() == 4 { 2 } else { items.len().clamp(1, most.min(3)) };
    for (i, item) in items.iter().enumerate() {
        g.attach(&reading(st, item), (i % cols) as i32, (i / cols) as i32, 1, 1);
    }
    g
}

/// The preview pane: the device or app, its readings, and what the page's pictures
/// show (a list, a colour, a curve, a deck's keys).
fn preview(st: &Shared, pv: &widgets::Preview, d: &protocol::PreviewDesc) {
    if !d.title.is_empty() {
        pv.head(&d.title, &d.line, &[]);
    }
    for b in &d.blocks {
        match b.kind {
            BlockKind::Device => {
                let card = widgets::hbox(14);
                card.add_css_class("preview-card");
                card.add_css_class("ext-device");
                let pic = picture(b, 96);
                pic.set_content_width(150);
                pic.set_hexpand(false);
                pic.set_valign(gtk::Align::Center);
                card.append(&pic);
                let text = widgets::vbox(2);
                text.set_hexpand(true);
                text.set_valign(gtk::Align::Center);
                if !b.title.is_empty() {
                    let t = widgets::label(&b.title, "ext-device-name");
                    t.set_xalign(0.0);
                    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    text.append(&t);
                }
                if !b.line.is_empty() {
                    let l = widgets::label(&b.line, "ext-device-line");
                    l.set_xalign(0.0);
                    l.set_wrap(true);
                    text.append(&l);
                }
                if !b.items.is_empty() {
                    let r = readings(st, &b.items, 2);
                    r.set_margin_top(10);
                    text.append(&r);
                }
                card.append(&text);
                pv.add(&card);
            }
            BlockKind::Picture => {
                let card = widgets::hbox(16);
                card.add_css_class("preview-card");
                card.add_css_class("ext-picture");
                if !b.items.is_empty() {
                    let labels = widgets::vbox(10);
                    labels.set_valign(gtk::Align::Center);
                    for item in &b.items {
                        labels.append(&reading(st, item));
                    }
                    card.append(&labels);
                }
                let pic = picture(b, 170);
                card.append(&pic);
                pv.add(&card);
            }
            BlockKind::Stats => {
                let card = widgets::vbox(0);
                card.add_css_class("preview-card");
                card.add_css_class("ext-stats");
                card.append(&readings(st, &b.items, 4));
                pv.add(&card);
            }
            BlockKind::Facts => {
                let card = widgets::vbox(0);
                card.add_css_class("preview-card");
                card.add_css_class("ext-facts");
                let g = gtk::Grid::new();
                g.set_column_homogeneous(true);
                g.set_column_spacing(14);
                g.set_row_spacing(10);
                for (i, item) in b.items.iter().enumerate() {
                    g.attach(&reading(st, item), (i % 2) as i32, (i / 2) as i32, 1, 1);
                }
                card.append(&g);
                pv.add(&card);
            }
            BlockKind::List => {
                let list = widgets::vbox(8);
                if !b.title.is_empty() {
                    let t = widgets::label(&b.title, "ext-caption");
                    t.set_xalign(0.0);
                    list.append(&t);
                }
                for item in &b.items {
                    let row = widgets::hbox(12);
                    row.add_css_class("preview-card");
                    row.add_css_class("ext-list-row");
                    if !item.icon.is_empty() {
                        row.append(&widgets::tile(&item.icon));
                    }
                    let text = widgets::vbox(1);
                    text.set_hexpand(true);
                    text.set_valign(gtk::Align::Center);
                    let t = widgets::label(&item.label, "ext-list-title");
                    t.set_xalign(0.0);
                    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    text.append(&t);
                    let sub = widgets::label(&item.sub, "ext-reading-sub");
                    sub.set_xalign(0.0);
                    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    sub.set_visible(!item.sub.is_empty());
                    text.append(&sub);
                    row.append(&text);
                    let v = widgets::label(&item.value, "ext-list-value");
                    v.set_valign(gtk::Align::Center);
                    if !item.tone.is_empty() {
                        v.add_css_class(&format!("ext-{}", item.tone));
                    }
                    row.append(&v);
                    st.item_labels.borrow_mut().push((v, sub));
                    list.append(&row);
                }
                pv.add(&list);
            }
            BlockKind::Banner => pv.add(&widgets::banner(&b.text, b.warning)),
            BlockKind::Caption => {
                let l = widgets::label(&b.text, "ext-caption");
                l.set_xalign(0.0);
                l.set_wrap(true);
                pv.add(&l);
            }
            BlockKind::Rows => {
                let holder = widgets::vbox(0);
                holder.add_css_class("ext-pane-rows");
                let page = Page::new(st.root.clone(), holder.clone());
                let g = page.group(&b.title);
                for row in &b.rows {
                    g.add(&row_widget(st, row));
                }
                pv.add(&holder);
            }
            BlockKind::Deck => pv.add(&deck(st, b)),
            BlockKind::Unknown => {}
        }
    }
    if !d.foot.is_empty() {
        pv.foot(if d.foot_icon.is_empty() { "dialog-information-symbolic" } else { &d.foot_icon }, &d.foot);
    }
}

/// A deck as it looks: its screen, its keys (click one to pick it) and its dials.
fn deck(st: &Shared, b: &protocol::Block) -> gtk::Box {
    let d = widgets::hbox(14);
    d.add_css_class("ext-deck");
    d.set_halign(gtk::Align::Center);
    if !b.screen.is_empty() {
        let screen = widgets::vbox(2);
        screen.add_css_class("ext-deck-screen");
        screen.set_valign(gtk::Align::Center);
        for (i, line) in b.screen.iter().enumerate() {
            let l = widgets::label(line, if i == 1 { "ext-deck-big" } else { "ext-deck-line" });
            screen.append(&l);
        }
        d.append(&screen);
    }
    let grid = gtk::Grid::new();
    grid.set_column_spacing(8);
    grid.set_row_spacing(8);
    grid.set_valign(gtk::Align::Center);
    let cols = b.columns.max(1) as usize;
    for (i, k) in b.keys.iter().enumerate() {
        let key = gtk::Button::new();
        key.add_css_class("ext-deck-key");
        if b.selected == Some(i as u32) {
            key.add_css_class("selected");
        }
        let inner = widgets::vbox(2);
        inner.set_valign(gtk::Align::Center);
        if !k.icon.is_empty() {
            inner.append(&gtk::Image::from_icon_name(&k.icon));
        }
        if !k.label.is_empty() {
            let l = widgets::label(&k.label, "ext-deck-label");
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_max_width_chars(7);
            inner.append(&l);
        }
        key.set_child(Some(&inner));
        key.set_tooltip_text(Some(&if k.label.is_empty() { format!("Key {}", i + 1) } else { format!("Key {}: {}", i + 1, k.label) }));
        if !b.key.is_empty() {
            let (st, name) = (st.clone(), b.key.clone());
            key.connect_clicked(move |_| send(&st, &name, i.to_string(), true));
        }
        grid.attach(&key, (i % cols) as i32, (i / cols) as i32, 1, 1);
    }
    d.append(&grid);
    if b.dials > 0 {
        let dials = widgets::vbox(14);
        dials.set_valign(gtk::Align::Center);
        for _ in 0..b.dials {
            let dial = widgets::hbox(0);
            dial.add_css_class("ext-deck-dial");
            dials.append(&dial);
        }
        d.append(&dials);
    }
    d
}

#[cfg(test)]
mod status_tests {
    use super::status_shape;
    #[test]
    fn changing_status_does_not_change_layout() {
        let a = r#"{"groups":[{"rows":[{"kind":"info","key":"ai-status","value":"Downloading 1%"},{"kind":"choice","key":"theme","disabled":true}]}]}"#;
        let b = r#"{"groups":[{"rows":[{"kind":"info","key":"ai-status","value":"Downloading 50%"},{"kind":"choice","key":"theme","disabled":false}]}]}"#;
        assert_eq!(status_shape(a), status_shape(b));
        assert_ne!(status_shape(a), status_shape(r#"{"groups":[]}"#));
    }

    #[test]
    fn changing_readings_does_not_change_layout() {
        let a = r#"{"groups":[],"preview":{"blocks":[{"kind":"stats","items":[{"label":"Fan","value":"2,400 rpm"}]}]}}"#;
        let b = r#"{"groups":[],"preview":{"blocks":[{"kind":"stats","items":[{"label":"Fan","value":"2,600 rpm","tone":"warn"}]}]}}"#;
        assert_eq!(status_shape(a), status_shape(b));
    }
}
