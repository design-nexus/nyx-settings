//! Nyx Bar's arrangement (NyxOS): the left, center and right sections of
//! `~/.config/nyx/bar.toml`, their order, pills (entries sharing one background),
//! dividers and where other plugins go (`"*"`). Rows move by dragging or with their
//! buttons. The layout comes from `nyx-bar --layout-json` (through every layer); the first
//! change writes all three sections into the user's file, which the bar follows.

use crate::widgets::{self, opts};
use gtk::prelude::*;
use gtk::{gdk, glib};
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use toml_edit::{Array, DocumentMut, Value};

pub const SECTIONS: [&str; 3] = ["left", "center", "right"];
pub const OTHERS: &str = "*";
pub const DIVIDER: &str = "|";

fn section_title(i: usize) -> &'static str {
    ["Left", "Center", "Right"][i.min(2)]
}

/// An entry of a section: a plugin or one of its items, a pill of them, `"|"` or `"*"`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Entry {
    One(String),
    Group(Vec<String>),
}

impl Entry {
    fn items(&self) -> Vec<String> {
        match self {
            Entry::One(s) => vec![s.clone()],
            Entry::Group(g) => g.clone(),
        }
    }

    /// A plugin or a pill, not a divider or the "other plugins" mark.
    pub(crate) fn is_item(&self) -> bool {
        !matches!(self, Entry::One(s) if s == DIVIDER || s == OTHERS)
    }
}

pub type Layout = [Vec<Entry>; 3];

#[derive(Debug, Clone, Default, Deserialize)]
struct Sections {
    #[serde(default)]
    left: Vec<Entry>,
    #[serde(default)]
    center: Vec<Entry>,
    #[serde(default)]
    right: Vec<Entry>,
}

/// What `nyx-bar --layout-json` says.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Info {
    #[serde(default)]
    sections: Sections,
    #[serde(default)]
    pub user_sets_layout: bool,
    #[serde(default)]
    names: HashMap<String, String>,
    /// What each entry shows, from its plugin's manifest ("Dots while an app uses…").
    #[serde(default)]
    descriptions: HashMap<String, String>,
    /// Entries whose plugin runs and part is on; `None` from an older nyx-bar.
    #[serde(default)]
    on: Option<Vec<String>>,
    #[serde(default)]
    pub unplaced: Vec<String>,
}

impl Info {
    pub fn layout(&self) -> Layout {
        [self.sections.left.clone(), self.sections.center.clone(), self.sections.right.clone()]
    }

    pub(crate) fn name(&self, entry: &str) -> String {
        self.names.get(entry).cloned().unwrap_or_else(|| entry.to_string())
    }

    fn entry_name(&self, e: &Entry) -> String {
        e.items().iter().map(|i| self.name(i)).collect::<Vec<_>>().join(" · ")
    }

    fn description(&self, e: &Entry) -> String {
        let all: Vec<&str> = e.items().iter().filter_map(|i| self.descriptions.get(i)).map(String::as_str).filter(|d| !d.is_empty()).collect();
        if all.len() == 1 { all[0].to_string() } else { String::new() }
    }

    /// Whether an entry shows anything on the bar: a plugin that's off (Keep awake until
    /// it's turned on) keeps its place in the layout but has nothing to arrange.
    pub(crate) fn is_on(&self, e: &Entry) -> bool {
        match &self.on {
            None => true,
            Some(on) => !e.is_item() || e.items().iter().any(|i| on.contains(i)),
        }
    }
}

pub fn read_info() -> Option<Info> {
    crate::cmd::output(&["nyx-bar", "--layout-json"]).and_then(|t| serde_json::from_str(&t).ok())
}

// ---------- Edits ----------

/// Move entry `i` of section `s` to section `to`, before entry `before` there (the end
/// when `None`); `before` counts the entries as they are before the move.
pub fn move_entry(l: &mut Layout, s: usize, i: usize, to: usize, before: Option<usize>) {
    if s > 2 || to > 2 || i >= l[s].len() {
        return;
    }
    let e = l[s].remove(i);
    let at = match before {
        None => l[to].len(),
        Some(b) if s == to && b > i => b - 1,
        Some(b) => b,
    }
    .min(l[to].len());
    l[to].insert(at, e);
}

/// Put entry `i` and the next one in one pill.
pub fn join_next(l: &mut Layout, s: usize, i: usize) -> bool {
    let sec = &mut l[s];
    if i + 1 >= sec.len() || !sec[i].is_item() || !sec[i + 1].is_item() {
        return false;
    }
    let mut items = sec.remove(i).items();
    items.extend(sec.remove(i).items());
    sec.insert(i, Entry::Group(items));
    true
}

/// A pill's entries each on their own, in its place.
pub fn split(l: &mut Layout, s: usize, i: usize) -> bool {
    let Some(Entry::Group(g)) = l[s].get(i).cloned() else { return false };
    l[s].splice(i..=i, g.into_iter().map(Entry::One));
    true
}

/// Take one entry out of a pill, just after it.
pub fn take_out(l: &mut Layout, s: usize, i: usize, item: &str) -> bool {
    let Some(Entry::Group(g)) = l[s].get(i).cloned() else { return false };
    let rest: Vec<String> = g.iter().filter(|x| *x != item).cloned().collect();
    if rest.len() == g.len() {
        return false;
    }
    l[s][i] = Entry::Group(rest);
    l[s].insert(i + 1, Entry::One(item.to_string()));
    true
}

pub fn remove(l: &mut Layout, s: usize, i: usize) {
    if i < l[s].len() {
        l[s].remove(i);
    }
}

pub fn add(l: &mut Layout, s: usize, entry: &str) {
    l[s.min(2)].push(Entry::One(entry.to_string()));
}

fn value_of(e: &Entry) -> Value {
    match e {
        Entry::One(s) => Value::from(s.as_str()),
        Entry::Group(g) => {
            let mut a = Array::new();
            for x in g {
                a.push(x.as_str());
            }
            Value::Array(a)
        }
    }
}

/// The three sections into the user's file, keeping everything else in it.
pub fn write_sections(doc: &mut DocumentMut, l: &Layout) {
    for (i, s) in SECTIONS.iter().enumerate() {
        let mut arr = Array::new();
        for e in &l[i] {
            arr.push(value_of(e));
        }
        if arr.len() > 3 {
            for v in arr.iter_mut() {
                v.decor_mut().set_prefix("\n    ");
            }
            arr.set_trailing("\n");
            arr.set_trailing_comma(true);
        }
        doc[s] = toml_edit::value(arr);
    }
}

/// Back to NyxOS's layout: the user's file no longer sets the sections.
pub fn reset(doc: &mut DocumentMut) {
    for s in SECTIONS {
        doc.remove(s);
    }
}

// ---------- The editor ----------

struct Editor {
    root: gtk::Box,
    info: RefCell<Info>,
}

type Ed = Rc<Editor>;

/// Change the layout and save it; the bar reloads on its own.
fn edit(ed: &Ed, change: impl FnOnce(&mut Layout)) {
    let mut l = ed.info.borrow().layout();
    change(&mut l);
    let mut doc = super::nyxbar::read_user();
    write_sections(&mut doc, &l);
    super::nyxbar::write_user(&doc);
    refresh(ed);
}

fn encode(s: usize, i: usize) -> String {
    format!("{s}\t{i}")
}

fn decode(v: &str) -> Option<(usize, usize)> {
    let (a, b) = v.split_once('\t')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

fn clear_drop_marks(w: &gtk::Widget) {
    w.remove_css_class("drop-before");
    w.remove_css_class("drop-after");
}

fn flat_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("flat");
    b.set_tooltip_text(Some(tip));
    b.set_valign(gtk::Align::Center);
    b
}

fn row(ed: &Ed, s: usize, i: usize, e: &Entry, len: usize, next_is_item: bool) -> gtk::Box {
    let info = ed.info.borrow().clone();
    let row = widgets::hbox(10);
    row.add_css_class("bar-item");
    let handle = gtk::Image::from_icon_name("list-drag-handle-symbolic");
    handle.add_css_class("dim");
    row.append(&handle);
    let text = widgets::vbox(0);
    text.set_hexpand(true);
    let title = widgets::hbox(8);
    match e {
        Entry::One(x) if x == DIVIDER => {
            row.add_css_class("divider-row");
            title.append(&widgets::label("Divider", "bar-item-name"));
        }
        Entry::One(x) if x == OTHERS => {
            title.append(&widgets::label("Other plugins", "bar-item-name"));
            let t = widgets::tag("Their place");
            t.set_tooltip_text(Some("Plugins this layout doesn't place, AI Hub's for one, go here."));
            title.append(&t);
        }
        Entry::One(x) => title.append(&widgets::label(&info.name(x), "bar-item-name")),
        Entry::Group(g) => {
            row.add_css_class("pill");
            title.append(&widgets::label(&info.entry_name(e), "bar-item-name"));
            let t = widgets::tag("Pill");
            t.set_tooltip_text(Some(if g.len() > 1 { "These share one background in the bar." } else { "On a background of its own in the bar." }));
            title.append(&t);
        }
    }
    text.append(&title);
    // What it shows (some only now and then: while recording, with updates waiting).
    let about = info.description(e);
    if e.is_item() && !about.is_empty() {
        let d = widgets::label(&about, "dim");
        d.add_css_class("bar-item-id");
        d.set_wrap(true);
        d.set_xalign(0.0);
        text.append(&d);
    }
    if e.is_item() {
        text.set_tooltip_text(Some(&e.items().join("  ")));
    }
    row.append(&text);

    for (icon, tip, delta) in [("go-up-symbolic", "Move up", -1i64), ("go-down-symbolic", "Move down", 1)] {
        let b = flat_button(icon, tip);
        let target = i as i64 + delta;
        b.set_sensitive(target >= 0 && target < len as i64);
        let ed = ed.clone();
        b.connect_clicked(move |_| {
            let before = if delta < 0 { i - 1 } else { i + 2 };
            edit(&ed, |l| move_entry(l, s, i, s, Some(before)));
        });
        row.append(&b);
    }

    // More: move to another section, pills, remove.
    let menu = gtk::MenuButton::new();
    menu.set_icon_name("view-more-symbolic");
    menu.set_has_frame(false);
    menu.set_tooltip_text(Some("More"));
    menu.set_valign(gtk::Align::Center);
    let pop = gtk::Popover::new();
    let items = widgets::vbox(2);
    let action = |label: &str, f: Box<dyn Fn(&mut Layout)>| {
        let b = gtk::Button::with_label(label);
        b.add_css_class("flat");
        let (ed, pop) = (ed.clone(), pop.clone());
        b.connect_clicked(move |_| {
            pop.popdown();
            edit(&ed, |l| f(l));
        });
        items.append(&b);
    };
    for to in (0..3).filter(|t| *t != s) {
        action(&format!("Move to {}", section_title(to)), Box::new(move |l| move_entry(l, s, i, to, None)));
    }
    if e.is_item() && next_is_item {
        action("Join with the next in one pill", Box::new(move |l| {
            join_next(l, s, i);
        }));
    }
    if let Entry::Group(g) = e {
        if g.len() > 1 {
            action("Split the pill", Box::new(move |l| {
                split(l, s, i);
            }));
            for item in g.clone() {
                let name = info.name(&item);
                action(&format!("Take {name} out of the pill"), Box::new(move |l| {
                    take_out(l, s, i, &item);
                }));
            }
        } else {
            action("Without a pill", Box::new(move |l| {
                split(l, s, i);
            }));
        }
    } else if e.is_item() {
        action("On a pill of its own", Box::new(move |l| {
            if let Some(Entry::One(x)) = l[s].get(i).cloned() {
                l[s][i] = Entry::Group(vec![x]);
            }
        }));
    }
    action(
        if e.is_item() { "Remove from the bar" } else { "Remove" },
        Box::new(move |l| remove(l, s, i)),
    );
    pop.set_child(Some(&items));
    menu.set_popover(Some(&pop));
    row.append(&menu);

    let drag = gtk::DragSource::new();
    drag.set_actions(gdk::DragAction::MOVE);
    {
        let payload = encode(s, i);
        drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&payload.to_value())));
        let r = row.clone();
        drag.connect_drag_begin(move |src, _| {
            src.set_icon(Some(&gtk::WidgetPaintable::new(Some(&r))), 20, 20);
            r.add_css_class("dragging");
        });
        let r = row.clone();
        drag.connect_drag_end(move |_, _, _| r.remove_css_class("dragging"));
    }
    row.add_controller(drag);

    let drop = gtk::DropTarget::new(glib::types::Type::STRING, gdk::DragAction::MOVE);
    {
        let r = row.clone();
        drop.connect_motion(move |_, _, y| {
            clear_drop_marks(r.upcast_ref());
            r.add_css_class(if y < r.height() as f64 / 2.0 { "drop-before" } else { "drop-after" });
            gdk::DragAction::MOVE
        });
        let r = row.clone();
        drop.connect_leave(move |_| clear_drop_marks(r.upcast_ref()));
        let (r, ed) = (row.clone(), ed.clone());
        drop.connect_drop(move |_, value, _, y| {
            clear_drop_marks(r.upcast_ref());
            let Some((fs, fi)) = value.get::<String>().ok().as_deref().and_then(decode) else { return false };
            let before = if y < r.height() as f64 / 2.0 { i } else { i + 1 };
            if fs == s && (before == fi || before == fi + 1) {
                return true;
            }
            edit(&ed, |l| move_entry(l, fs, fi, s, Some(before)));
            true
        });
    }
    row.add_controller(drop);
    row
}

fn section_card(ed: &Ed, s: usize, entries: &[Entry]) -> gtk::Box {
    let card = widgets::vbox(4);
    card.add_css_class("bar-section");
    let top = widgets::hbox(8);
    let info = ed.info.borrow().clone();
    let head = widgets::label(&format!("{} · {}", section_title(s).to_uppercase(), entries.iter().filter(|e| e.is_item() && info.is_on(e)).count()), "bar-section-title");
    head.set_hexpand(true);
    top.append(&head);
    let divider = gtk::Button::with_label("+ Divider");
    divider.add_css_class("chip");
    divider.set_tooltip_text(Some("A thin line at the end of this section; drag it where you want it"));
    {
        let ed = ed.clone();
        divider.connect_clicked(move |_| edit(&ed, |l| add(l, s, DIVIDER)));
    }
    top.append(&divider);
    card.append(&top);
    let list = widgets::vbox(4);
    for (i, e) in entries.iter().enumerate() {
        if !info.is_on(e) {
            continue;
        }
        let next_is_item = entries.get(i + 1).is_some_and(Entry::is_item);
        list.append(&row(ed, s, i, e, entries.len(), next_is_item));
    }
    card.append(&list);

    let end = gtk::Label::new(Some(if entries.is_empty() { "Drop items here" } else { "" }));
    end.add_css_class("bar-drop-end");
    end.add_css_class("dim");
    let drop = gtk::DropTarget::new(glib::types::Type::STRING, gdk::DragAction::MOVE);
    {
        let e = end.clone();
        drop.connect_enter(move |_, _, _| {
            e.add_css_class("drop-before");
            gdk::DragAction::MOVE
        });
        let e = end.clone();
        drop.connect_leave(move |_| e.remove_css_class("drop-before"));
        let (e, ed) = (end.clone(), ed.clone());
        drop.connect_drop(move |_, value, _, _| {
            e.remove_css_class("drop-before");
            let Some((fs, fi)) = value.get::<String>().ok().as_deref().and_then(decode) else { return false };
            edit(&ed, |l| move_entry(l, fs, fi, s, None));
            true
        });
    }
    end.add_controller(drop);
    card.append(&end);
    card
}

fn refresh(ed: &Ed) {
    while let Some(c) = ed.root.first_child() {
        ed.root.remove(&c);
    }
    let Some(info) = read_info() else {
        ed.root.append(&widgets::label("The bar didn't say its layout (nyx-bar --layout-json).", "dim"));
        return;
    };
    *ed.info.borrow_mut() = info.clone();
    let layout = info.layout();
    for (s, entries) in layout.iter().enumerate() {
        ed.root.append(&section_card(ed, s, entries));
    }

    // Add what isn't on the bar: enabled items it doesn't place, the "other plugins" mark.
    let mut addable: Vec<(String, String)> = info.unplaced.iter().map(|e| (e.clone(), info.name(e))).collect();
    if !layout.iter().flatten().any(|e| *e == Entry::One(OTHERS.into())) {
        addable.push((OTHERS.into(), "Other plugins (where unplaced ones go)".into()));
    }
    if !addable.is_empty() {
        let line = widgets::hbox(10);
        line.add_css_class("bar-add");
        let pick = widgets::dropdown(&addable, &addable[0].0);
        pick.set_hexpand(true);
        line.append(&pick);
        let section = Rc::new(RefCell::new(2usize));
        let s2 = section.clone();
        let seg = widgets::segmented(&opts(&[("left", "Left"), ("center", "Center"), ("right", "Right")]), "right", move |v| {
            *s2.borrow_mut() = SECTIONS.iter().position(|x| *x == v).unwrap_or(2);
        });
        line.append(&seg);
        let b = gtk::Button::with_label("Add");
        b.add_css_class("suggested-action");
        {
            let ed = ed.clone();
            b.connect_clicked(move |_| {
                let Some((id, _)) = addable.get(pick.selected() as usize).cloned() else { return };
                let s = *section.borrow();
                edit(&ed, |l| add(l, s, &id));
            });
        }
        line.append(&b);
        let card = widgets::vbox(6);
        card.add_css_class("bar-section");
        card.append(&widgets::label("ADD TO THE BAR", "bar-section-title"));
        card.append(&line);
        ed.root.append(&card);
    }
}

/// The editor: the three sections and an "add to the bar" line.
pub fn editor() -> gtk::Box {
    let root = widgets::vbox(10);
    let ed: Ed = Rc::new(Editor { root: root.clone(), info: RefCell::default() });
    refresh(&ed);
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(s: &str) -> Entry {
        Entry::One(s.into())
    }

    fn layout() -> Layout {
        [
            vec![Entry::Group(vec!["nyx.launcher".into(), "nyx.workspaces".into()]), one("nyx.window")],
            vec![Entry::Group(vec!["nyx.clock".into()])],
            vec![one("nyx.privacy"), one(OTHERS), Entry::Group(vec!["nyx.weather".into(), "nyx.stats".into()]), one("nyx.tray")],
        ]
    }

    #[test]
    fn moves_within_and_across_sections() {
        let mut l = layout();
        move_entry(&mut l, 2, 0, 2, Some(3));
        assert_eq!(l[2][2], one("nyx.privacy"), "down past two: before the old index 3");
        move_entry(&mut l, 2, 2, 2, Some(0));
        assert_eq!(l[2][0], one("nyx.privacy"));
        move_entry(&mut l, 0, 1, 1, None);
        assert_eq!(l[1], vec![Entry::Group(vec!["nyx.clock".into()]), one("nyx.window")]);
        assert_eq!(l[0].len(), 1);
    }

    #[test]
    fn pills_join_split_and_take_out() {
        let mut l = layout();
        assert!(!join_next(&mut l, 2, 0), "the other-plugins mark doesn't join");
        assert!(join_next(&mut l, 2, 2));
        assert_eq!(l[2][2], Entry::Group(vec!["nyx.weather".into(), "nyx.stats".into(), "nyx.tray".into()]));
        assert!(take_out(&mut l, 2, 2, "nyx.stats"));
        assert_eq!(l[2][3], one("nyx.stats"));
        assert!(split(&mut l, 0, 0));
        assert_eq!(&l[0][..2], &[one("nyx.launcher"), one("nyx.workspaces")]);
    }

    #[test]
    fn writes_the_sections_and_keeps_the_rest() {
        let mut doc: DocumentMut = "# mine\nsize = \"cozy\"\n[plugins.\"nyx.clock\"]\nlayout = \"wide\"\n".parse().unwrap();
        let mut l = layout();
        add(&mut l, 2, DIVIDER);
        write_sections(&mut doc, &l);
        let text = doc.to_string();
        assert!(text.contains("# mine") && text.contains("size = \"cozy\"") && text.contains("layout = \"wide\""), "{text}");
        let back: toml::Table = text.parse().unwrap();
        let right: Vec<Entry> = back["right"].clone().try_into().unwrap();
        assert_eq!(right.last(), Some(&one(DIVIDER)));
        assert_eq!(right[1], one(OTHERS));
        reset(&mut doc);
        assert!(!doc.to_string().contains("right ="), "{doc}");
        assert!(doc.to_string().contains("size = \"cozy\""));
    }
}
