//! Nyx-style building blocks: pages, groups and option rows, plus rows bound
//! directly to Hyprland options.

pub use nyx_ui::widgets::*;

use crate::backend::{hypr, store};
use crate::cmd;
use crate::sketch::{self, Sketch};
use gtk::prelude::*;
use gtk::{glib, pango};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

// ---------- Search registry ----------

pub struct SearchItem {
    pub section: String,
    pub text: String,
    pub row: gtk::Widget,
    pub group: Option<gtk::Widget>,
}

thread_local! {
    static CURRENT_SECTION: RefCell<String> = const { RefCell::new(String::new()) };
    static CURRENT_GROUP: RefCell<Option<gtk::Widget>> = const { RefCell::new(None) };
    pub static SEARCH: RefCell<Vec<SearchItem>> = const { RefCell::new(Vec::new()) };
}

fn register(row: &impl IsA<gtk::Widget>, title: &str, desc: &str, keywords: &str) {
    let section = CURRENT_SECTION.with(|s| s.borrow().clone());
    let group = CURRENT_GROUP.with(|g| g.borrow().clone());
    SEARCH.with(|s| {
        s.borrow_mut().push(SearchItem {
            section,
            text: crate::search::normalise(&format!("{title} {desc} {keywords}")),
            row: row.clone().upcast(),
            group,
        })
    });
}

/// Extra search words for the most recently added row.
pub fn keywords(words: &str) {
    SEARCH.with(|s| {
        if let Some(last) = s.borrow_mut().last_mut() {
            last.text.push(' ');
            last.text.push_str(&crate::search::normalise(words));
        }
    });
}

/// Rows made from now on belong to this section (for pages that fill in later).
pub fn begin_section(section: &str) {
    CURRENT_SECTION.with(|s| *s.borrow_mut() = section.to_string());
    CURRENT_GROUP.with(|g| *g.borrow_mut() = None);
}

/// Drop the search entries for rows inside `within` (before they're replaced).
pub fn forget_rows(within: &impl IsA<gtk::Widget>) {
    let within = within.as_ref();
    SEARCH.with(|s| s.borrow_mut().retain(|item| !item.row.is_ancestor(within)));
}

// ---------- Page / group ----------

pub struct Page {
    pub root: gtk::ScrolledWindow,
    pub body: gtk::Box,
    /// Where groups go: the body, or the controls column once the page is split.
    target: RefCell<gtk::Box>,
}

/// A page. Its name shows in the top bar and its description in the status
/// bar, so the page itself starts straight with its content.
pub fn page(section: &str) -> Page {
    CURRENT_SECTION.with(|s| *s.borrow_mut() = section.to_string());
    CURRENT_GROUP.with(|g| *g.borrow_mut() = None);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.add_css_class("settings-page");
    body.add_css_class(&format!("page-{section}"));
    let colour = CURRENT_COLOUR.with(|c| c.borrow().clone());
    if !colour.is_empty() {
        body.add_css_class(&format!("colour-{colour}"));
    }

    body.set_hexpand(true);

    // Content stays a comfortable reading width on wide windows.
    let clamp = crate::clamp::Clamp::new(&body, MAX_CONTENT_WIDTH);
    let root = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        // Scrolls with wheel, trackpad and keyboard; no visible scrollbar.
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&clamp)
        .vexpand(true)
        .build();
    Page::new(root, body)
}

/// The widest a page's content gets, padding included.
const MAX_CONTENT_WIDTH: i32 = 920;
/// The widest a split page gets: controls and preview side by side.
const MAX_SPLIT_WIDTH: i32 = 1400;
/// The controls column of a split page.
const SPLIT_CONTROLS_WIDTH: i32 = 430;

thread_local! {
    /// The theme's name for the colour of the section being built (`section-<key>`),
    /// for the tiles in its rows.
    static CURRENT_COLOUR: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The section colour the rows built from now on take (`connections`, `hardware`…).
pub fn set_section_colour(key: &str) {
    CURRENT_COLOUR.with(|c| *c.borrow_mut() = key.to_string());
}

impl Page {
    pub fn new(root: gtk::ScrolledWindow, body: gtk::Box) -> Page {
        Page { root, body: body.clone(), target: RefCell::new(body) }
    }

    /// The box new groups go into.
    fn holder(&self) -> gtk::Box {
        self.target.borrow().clone()
    }

    /// Controls beside a picture of what they change: the groups made from now on go
    /// in a column on the left, and the returned preview fills the right. In a narrow
    /// window the preview sits above the controls.
    pub fn split(&self) -> Preview {
        let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        split.add_css_class("split-page");
        split.set_vexpand(true);
        let controls = vbox(0);
        controls.add_css_class("split-controls");
        controls.set_hexpand(false);
        // Wrapping row text asks to be one line wide; the column stays its own width.
        let column = nyx_ui::clamp::Clamp::new(&controls, SPLIT_CONTROLS_WIDTH);
        column.set_start(true);
        column.add_css_class("split-column");
        column.set_width_request(SPLIT_CONTROLS_WIDTH);
        let pane = vbox(14);
        pane.add_css_class("preview-pane");
        // A drawing or a row of chips mustn't hold the window too wide to go narrow.
        let holder = nyx_ui::clamp::Clamp::new(&pane, i32::MAX);
        holder.set_shrink(true);
        holder.add_css_class("split-preview");
        // Side by side, each column scrolls on its own and is only as tall as the window,
        // so the preview's foot stays on screen however long the controls are.
        let left = column_scroller(&column);
        left.add_css_class("split-column-scroll");
        let right = column_scroller(&holder);
        right.add_css_class("split-preview-scroll");
        right.set_hexpand(true);
        split.append(&left);
        split.append(&right);
        self.body.append(&split);
        // The page this split is on: the body itself, or (an extension's page, drawn into
        // a box inside the body) the settings page around it. It takes the split's width.
        let host = page_of(self.body.upcast_ref());
        self.body.add_css_class("split-body");
        host.add_css_class("split-body");
        if let Some(clamp) = host.parent().and_downcast::<nyx_ui::clamp::Clamp>() {
            clamp.set_max(MAX_SPLIT_WIDTH);
        }
        *self.target.borrow_mut() = controls;
        narrow_split(self.body.upcast_ref(), host.has_css_class("narrow"));
        Preview { pane, foot: RefCell::default(), filled: Cell::default() }
    }
    /// A titled group whose rows share one card, split by hairlines.
    pub fn group(&self, title: &str) -> Group {
        self.make_group(title, true)
    }

    /// A titled group without the card, for content that brings its own layout
    /// (a grid of theme previews, the bar editor).
    pub fn plain_group(&self, title: &str) -> Group {
        self.make_group(title, false)
    }

    fn make_group(&self, title: &str, card: bool) -> Group {
        let wrapper = gtk::Box::new(gtk::Orientation::Vertical, 0);
        wrapper.add_css_class("settings-group");
        let head = gtk::Box::new(gtk::Orientation::Vertical, 0);
        if !title.is_empty() {
            let l = gtk::Label::new(Some(&title.to_uppercase()));
            l.add_css_class("group-title");
            l.set_xalign(0.0);
            head.append(&l);
        } else {
            wrapper.add_css_class("untitled");
        }
        wrapper.append(&head);
        let list = group_list(card);
        wrapper.append(&list);
        self.holder().append(&wrapper);
        CURRENT_GROUP.with(|g| *g.borrow_mut() = Some(wrapper.clone().upcast()));
        Group { wrapper, head, list }
    }

    /// A group that folds away under its title, for settings people rarely need.
    /// Whether it's open is remembered; search opens it when a row inside matches.
    pub fn collapsible(&self, title: &str, open_by_default: bool) -> Group {
        let section = CURRENT_SECTION.with(|s| s.borrow().clone());
        let key = format!("{section}/{title}");
        let open = crate::prefs::get().open_groups.get(&key).copied().unwrap_or(open_by_default);

        let wrapper = gtk::Box::new(gtk::Orientation::Vertical, 0);
        wrapper.add_css_class("settings-group");
        wrapper.add_css_class("collapsible");

        let toggle = gtk::Button::new();
        toggle.add_css_class("group-toggle");
        let line = hbox(12);
        let text = vbox(2);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.append(&label(title, "settings-option-title"));
        // What's inside, so a folded group can still be found by eye.
        let summary = label("", "settings-option-description");
        summary.set_ellipsize(pango::EllipsizeMode::End);
        summary.set_visible(false);
        text.append(&summary);
        line.append(&text);
        let chevron = gtk::Image::from_icon_name("pan-end-symbolic");
        chevron.add_css_class("chevron");
        chevron.set_valign(gtk::Align::Center);
        line.append(&chevron);
        toggle.set_child(Some(&line));
        wrapper.append(&toggle);

        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(if crate::prefs::get().reduce_motion { 0 } else { 200 })
            .reveal_child(open)
            .build();
        let inner = vbox(0);
        let head = vbox(0);
        head.add_css_class("collapsible-head");
        inner.append(&head);
        let list = group_list(true);
        inner.append(&list);
        revealer.set_child(Some(&inner));
        wrapper.append(&revealer);
        set_toggle_open(&toggle, open);

        {
            let (revealer, key) = (revealer.clone(), key.clone());
            toggle.connect_clicked(move |t| {
                let open = !revealer.reveals_child();
                revealer.set_reveal_child(open);
                set_toggle_open(t, open);
                let key = key.clone();
                crate::prefs::update(|p| {
                    p.open_groups.insert(key, open);
                });
            });
        }
        COLLAPSIBLES.with(|c| {
            let mut c = c.borrow_mut();
            c.retain(|g| g.wrapper.upgrade().is_some());
            c.push(Collapsible {
                wrapper: wrapper.upcast_ref::<gtk::Widget>().downgrade(),
                revealer: revealer.downgrade(),
                toggle: toggle.downgrade(),
                key,
                open_by_default,
            });
        });
        {
            // Rows are added after this returns: read their titles once the page is built.
            let (list, head, toggle) = (list.clone(), head.clone(), toggle.clone());
            glib::idle_add_local_once(move || {
                // A note says best what the group is for; otherwise list its rows.
                // The note moves up into the header, so it isn't shown twice.
                if let Some(note) = head.first_child().and_downcast::<gtk::Label>().filter(|l| l.has_css_class("group-note")) {
                    summary.set_text(&note.text());
                    summary.set_visible(true);
                    note.set_visible(false);
                    return;
                }
                // Row titles only while folded: once open, the rows are right there.
                let titles = row_titles(&list);
                if !titles.is_empty() {
                    summary.set_text(&titles.join(", "));
                    summary.add_css_class("titles-summary");
                    summary.set_visible(!toggle.has_css_class("open"));
                }
            });
        }
        self.holder().append(&wrapper);
        CURRENT_GROUP.with(|g| *g.borrow_mut() = Some(wrapper.clone().upcast()));
        Group { wrapper, head, list }
    }

    pub fn banner(&self, text: &str, warning: bool) -> gtk::Box {
        let b = banner(text, warning);
        self.holder().append(&b);
        b
    }
}

#[derive(Clone)]
pub struct Group {
    pub wrapper: gtk::Box,
    /// The title and notes, above the rows.
    head: gtk::Box,
    pub list: gtk::Box,
}

impl Group {
    pub fn add(&self, w: &impl IsA<gtk::Widget>) {
        self.list.append(w);
        // An empty card would draw as a stray outline.
        self.list.set_visible(true);
    }

    /// Put a widget (a banner, say) between the title and the rows.
    pub fn top(&self, w: &impl IsA<gtk::Widget>) {
        self.wrapper.remove_css_class("untitled");
        self.head.append(w);
    }

    pub fn note(&self, text: &str) {
        let l = gtk::Label::new(None);
        set_markup_or_text(&l, text);
        l.add_css_class("group-note");
        l.set_xalign(0.0);
        l.set_wrap(true);
        // Notes sit just under the group title.
        self.head.append(&l);
    }
}

// ---------- Split pages: controls beside a preview ----------

/// The right side of a split page ([`Page::split`]): a picture of what the
/// controls change, with a heading, cards and a line at its foot.
pub struct Preview {
    pub pane: gtk::Box,
    foot: RefCell<Option<gtk::Box>>,
    filled: Cell<bool>,
}

impl Preview {
    /// The preview's heading: a title, a line under it, and controls at the right.
    pub fn head(&self, title: &str, line: &str, end: &[&gtk::Widget]) -> gtk::Box {
        let head = hbox(10);
        head.add_css_class("preview-head");
        let text = vbox(2);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.append(&label(title, "preview-title"));
        if !line.is_empty() {
            let l = label(line, "preview-line");
            l.set_wrap(true);
            text.append(&l);
        }
        head.append(&text);
        for w in end {
            w.set_valign(gtk::Align::Center);
            head.append(*w);
        }
        self.add(&head);
        head
    }

    /// Add to the preview, above its foot line.
    pub fn add(&self, w: &impl IsA<gtk::Widget>) {
        match self.foot.borrow().as_ref() {
            Some(foot) => w.insert_before(&self.pane, Some(foot)),
            None => self.pane.append(w),
        }
    }

    /// Add `w` to take the height the preview has spare, rather than leaving it above the foot.
    pub fn fill(&self, w: &impl IsA<gtk::Widget>) {
        w.set_vexpand(true);
        self.add(w);
        self.filled.set(true);
        if let Some(spacer) = self.foot.borrow().as_ref() {
            spacer.set_vexpand(false);
        }
    }

    /// A card in the preview, for a group of facts or a smaller picture.
    pub fn card(&self) -> gtk::Box {
        let c = vbox(0);
        c.add_css_class("preview-card");
        self.add(&c);
        c
    }

    /// A drawing in the theme's colours, `height` tall and as wide as the preview,
    /// redrawn when the theme changes. `draw` gets the context, the palette, the
    /// section's colour and the size.
    pub fn drawing(&self, height: i32, draw: impl Fn(&gtk::cairo::Context, &sketch::Pal, sketch::Rgb, f64, f64) + 'static) -> gtk::DrawingArea {
        let area = drawing(height, draw);
        self.add(&area);
        area
    }

    /// A line at the foot of the preview: an icon and a short note. Call it last.
    pub fn foot(&self, icon: &str, text: &str) -> gtk::Label {
        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_vexpand(!self.filled.get());
        self.pane.append(&spacer);
        let foot = hbox(8);
        foot.add_css_class("preview-foot");
        let i = gtk::Image::from_icon_name(icon);
        i.set_valign(gtk::Align::Start);
        foot.append(&i);
        let l = label(text, "preview-foot-text");
        l.set_wrap(true);
        l.set_hexpand(true);
        foot.append(&l);
        self.pane.append(&foot);
        *self.foot.borrow_mut() = Some(spacer);
        l
    }
}

/// A drawing in the theme's colours (see [`Preview::drawing`]), for previews and cards.
pub fn drawing(height: i32, draw: impl Fn(&gtk::cairo::Context, &sketch::Pal, sketch::Rgb, f64, f64) + 'static) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_height(height);
    area.set_hexpand(true);
    area.add_css_class("preview-drawing");
    let made_for = CURRENT_COLOUR.with(|c| c.borrow().clone());
    area.set_draw_func(move |a, cr, w, h| {
        let key = page_colour(a.upcast_ref()).unwrap_or_else(|| made_for.clone());
        draw(cr, &sketch::pal(), colour_rgb(&key), w as f64, h as f64)
    });
    let a = area.clone();
    crate::theme::subscribe(&area, move || a.queue_draw());
    area
}

/// The colour of the section being built, as Cairo wants it.
pub fn section_rgb() -> sketch::Rgb {
    colour_rgb(&CURRENT_COLOUR.with(|c| c.borrow().clone()))
}

/// The section colour of the page `w` is on (its body's `colour-<key>` class).
fn page_colour(w: &gtk::Widget) -> Option<String> {
    std::iter::successors(w.parent(), |w| w.parent())
        .find_map(|w| w.css_classes().iter().find_map(|c| c.strip_prefix("colour-").map(str::to_string)))
}

fn colour_rgb(key: &str) -> sketch::Rgb {
    crate::fangraph::rgb(&crate::theme::section_accent("settings", key))
}

/// One side of a split page: scrolls on its own while the page is side by side.
fn column_scroller(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(child)
        .vexpand(true)
        .build()
}

/// The widget a scrolled window shows (inside the viewport GTK adds).
fn scrolled_child(s: &gtk::ScrolledWindow) -> Option<gtk::Widget> {
    let c = s.child()?;
    if c.is::<gtk::Viewport>() { c.first_child() } else { Some(c) }
}

/// Put a split page's preview above its controls in a narrow window, beside them
/// otherwise. Side by side, the controls and the preview each scroll on their own at
/// the window's height; stacked, the page scrolls as one. `body` is a page body.
/// The settings page `w` is on (`w` itself when it's the page).
pub fn page_of(w: &gtk::Widget) -> gtk::Widget {
    std::iter::successors(Some(w.clone()), |w| w.parent()).find(|w| w.has_css_class("settings-page")).unwrap_or_else(|| w.clone())
}

/// Stack a page's split (controls above the preview) when the window is narrow. The split
/// is in the page body, or a level or two down (an extension's page).
pub fn narrow_split(body: &gtk::Widget, narrow: bool) {
    narrow_split_in(body, narrow, 2);
}

fn narrow_split_in(body: &gtk::Widget, narrow: bool, depth: u32) {
    let page = body.ancestor(gtk::ScrolledWindow::static_type()).and_downcast::<gtk::ScrolledWindow>();
    let mut c = body.first_child();
    while let Some(child) = c {
        if depth > 0 && !child.has_css_class("split-page") && child.is::<gtk::Box>() {
            narrow_split_in(&child, narrow, depth - 1);
        }
        if child.has_css_class("split-page")
            && let Some(split) = child.downcast_ref::<gtk::Box>()
            && let (Some(first), Some(last)) = (split.first_child(), split.last_child())
        {
            let (left, right) = if first.has_css_class("split-column-scroll") { (first, last) } else { (last, first) };
            let (Some(left), Some(right)) = (left.downcast_ref::<gtk::ScrolledWindow>(), right.downcast_ref::<gtk::ScrolledWindow>())
            else {
                return;
            };
            let Some(column) = scrolled_child(left).and_downcast::<nyx_ui::clamp::Clamp>() else { return };
            let own = if narrow { gtk::PolicyType::Never } else { gtk::PolicyType::External };
            left.set_vscrollbar_policy(own);
            right.set_vscrollbar_policy(own);
            if let Some(page) = &page {
                page.set_vscrollbar_policy(if narrow { gtk::PolicyType::External } else { gtk::PolicyType::Never });
            }
            if narrow {
                split.set_orientation(gtk::Orientation::Vertical);
                split.reorder_child_after(right, None::<&gtk::Widget>);
                column.set_width_request(-1);
                column.set_max(i32::MAX);
                for s in [left, right] {
                    s.set_vexpand(false);
                    s.set_propagate_natural_height(true);
                }
            } else {
                split.set_orientation(gtk::Orientation::Horizontal);
                split.reorder_child_after(right, Some(left));
                column.set_width_request(SPLIT_CONTROLS_WIDTH);
                column.set_max(SPLIT_CONTROLS_WIDTH);
                for s in [left, right] {
                    s.set_vexpand(true);
                    s.set_propagate_natural_height(false);
                }
            }
        }
        c = child.next_sibling();
    }
}

/// The scrolled window that scrolls `w` into view: the nearest one around it that
/// scrolls (a split page's column, or the page itself).
pub fn scroller_for(w: &gtk::Widget) -> Option<gtk::ScrolledWindow> {
    std::iter::successors(w.parent(), |w| w.parent())
        .filter_map(|w| w.downcast::<gtk::ScrolledWindow>().ok())
        .find(|s| s.vscrollbar_policy() != gtk::PolicyType::Never)
}

/// The scroller holding a page's controls: the page root, or a split page's controls
/// column while it scrolls on its own.
pub fn main_scroller(root: &gtk::ScrolledWindow) -> gtk::ScrolledWindow {
    if root.vscrollbar_policy() == gtk::PolicyType::Never {
        fn find(w: &gtk::Widget) -> Option<gtk::ScrolledWindow> {
            if w.has_css_class("split-column-scroll") {
                return w.clone().downcast::<gtk::ScrolledWindow>().ok();
            }
            let mut c = w.first_child();
            while let Some(child) = c {
                if let Some(s) = find(&child) {
                    return Some(s);
                }
                c = child.next_sibling();
            }
            None
        }
        if let Some(s) = find(root.upcast_ref()) {
            return s;
        }
    }
    root.clone()
}

// ---------- Tiles and rows that lead somewhere ----------

/// An icon on a tile in the section's colour, drawn as Umbra draws its icons (a
/// satin tile, a glow and a rim of the colour, the glyph in it).
pub fn tile(icon: &str) -> gtk::Box {
    let t = hbox(0);
    t.add_css_class("row-tile");
    // The colour of the page it's on (a row may be made after another page was built).
    t.connect_realize(|t| {
        let key = page_colour(t.upcast_ref());
        if let Some(key) = key.or_else(|| Some(CURRENT_COLOUR.with(|c| c.borrow().clone())).filter(|k| !k.is_empty())) {
            t.add_css_class(&format!("section-{key}"));
        }
    });
    t.set_valign(gtk::Align::Center);
    t.set_halign(gtk::Align::Center);
    t.set_hexpand(false);
    let i = gtk::Image::from_icon_name(icon);
    i.set_halign(gtk::Align::Center);
    i.set_valign(gtk::Align::Center);
    t.append(&i);
    t
}

/// [`row`] with an icon on a tile before its text.
pub fn icon_row(icon: &str, title: &str, desc: &str, control: Option<&gtk::Widget>) -> gtk::Box {
    let r = row(title, desc, control);
    r.prepend(&tile(icon));
    r.add_css_class("with-tile");
    r
}

/// A row that opens something one level down (a sub-page, a sheet): its current
/// value and a chevron at the right; the whole row is the button.
pub fn nav_row(icon: Option<&str>, title: &str, desc: &str, value: &str, on_open: impl Fn() + 'static) -> (gtk::Box, gtk::Label) {
    let end = hbox(6);
    let v = label(value, "row-value");
    v.set_ellipsize(pango::EllipsizeMode::End);
    v.set_max_width_chars(28);
    v.set_visible(!value.is_empty());
    end.append(&v);
    let chevron = gtk::Image::from_icon_name("go-next-symbolic");
    chevron.add_css_class("chevron");
    end.append(&chevron);
    let r = match icon {
        Some(i) => icon_row(i, title, desc, Some(end.upcast_ref())),
        None => row(title, desc, Some(end.upcast_ref())),
    };
    r.add_css_class("nav-row");
    r.set_cursor_from_name(Some("pointer"));
    r.set_focusable(true);
    let click = gtk::GestureClick::new();
    let open = Rc::new(on_open);
    let o = open.clone();
    click.connect_released(move |_, _, _, _| o());
    r.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::space) {
            open();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    r.add_controller(keys);
    (r, v)
}

/// A page one level down, over the window: `title` at its top with a way back, and
/// `build` filling it like a page (groups, splits). For the settings a page keeps a
/// click away (troubleshooting, remembered devices, details).
pub fn subpage(title: &str, width: i32, build: impl FnOnce(&Page)) -> gtk::Window {
    let (dialog, card) = dialog(title, width);
    card.add_css_class("subpage");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.add_css_class("settings-page");
    body.add_css_class("subpage-body");
    // As tall as its content, but never taller than the window it opens over.
    let tallest = main_window().map(|w| w.height()).filter(|h| *h > 0).map_or(620, |h| (h - 200).clamp(260, 860));
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .propagate_natural_height(true)
        .max_content_height(tallest)
        .child(&body)
        .build();
    card.append(&scroll);
    let page = Page::new(scroll.clone(), body.clone());
    build(&page);
    // A split sub-page's columns scroll on their own, so give them the room.
    if body.has_css_class("split-body") {
        scroll.set_size_request(-1, tallest);
    }
    let done = gtk::Button::with_label("Done");
    done.set_halign(gtk::Align::End);
    let d = dialog.clone();
    done.connect_clicked(move |_| d.close());
    card.append(&done);
    dialog.present();
    dialog
}

/// The bar at the foot of a page whose changes are applied together (NyxOS's system
/// pages): how many are waiting, Discard and Apply.
pub struct ChangesBar {
    pub root: gtk::Box,
    pub label: gtk::Label,
    pub detail: gtk::Label,
    pub discard: gtk::Button,
    pub apply: gtk::Button,
}

impl ChangesBar {
    /// Show `n` changes waiting (hidden at none) and what they are.
    pub fn set(&self, n: usize, detail: &str) {
        self.root.set_visible(n > 0);
        self.label.set_text(&match n {
            1 => "1 change waiting".to_string(),
            n => format!("{n} changes waiting"),
        });
        self.detail.set_text(detail);
        self.detail.set_visible(!detail.is_empty());
    }
}

pub fn changes_bar() -> ChangesBar {
    let root = hbox(12);
    root.add_css_class("changes-bar");
    let icon = gtk::Image::from_icon_name("document-save-symbolic");
    icon.set_valign(gtk::Align::Center);
    root.append(&icon);
    let text = vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let label = label("", "changes-bar-title");
    let detail = label_wrapped("", "changes-bar-detail");
    text.append(&label);
    text.append(&detail);
    root.append(&text);
    let discard = gtk::Button::with_label("Discard");
    discard.add_css_class("flat");
    let apply = gtk::Button::with_label("Apply");
    apply.add_css_class("suggested-action");
    for b in [&discard, &apply] {
        b.set_valign(gtk::Align::Center);
        root.append(b);
    }
    root.set_visible(false);
    ChangesBar { root, label, detail, discard, apply }
}

fn label_wrapped(text: &str, class: &str) -> gtk::Label {
    let l = label(text, class);
    l.set_wrap(true);
    l.set_xalign(0.0);
    l
}

/// The titles of the rows directly in a card (not the ones inside disclosures).
fn row_titles(list: &gtk::Box) -> Vec<String> {
    fn walk(w: &gtk::Widget, out: &mut Vec<String>) {
        let mut c = w.first_child();
        while let Some(child) = c {
            if let Some(l) = child.downcast_ref::<gtk::Label>()
                && l.has_css_class("settings-option-title")
            {
                let t = l.text().to_string();
                if !t.is_empty() && !out.contains(&t) {
                    out.push(t);
                }
                return;
            }
            // A row's own title is the first label found in it; don't look into its controls.
            if !child.has_css_class("disclosure-content") {
                walk(&child, out);
            }
            c = child.next_sibling();
        }
    }
    let mut out = Vec::new();
    walk(list.upcast_ref(), &mut out);
    out
}

fn group_list(card: bool) -> gtk::Box {
    let list = gtk::Box::new(gtk::Orientation::Vertical, if card { 0 } else { 12 });
    if card {
        list.add_css_class("settings-card");
        // Keeps a highlighted first or last row inside the rounded corners.
        list.set_overflow(gtk::Overflow::Hidden);
    } else {
        list.add_css_class("plain-list");
    }
    list.set_visible(false);
    list
}

/// Mark the first visible row of each card inside `within` (class `first-shown`),
/// so search hiding the real first row doesn't leave a hairline at the top.
pub fn mark_first_rows(within: &gtk::Widget) {
    fn rows(w: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
        let mut c = w.first_child();
        while let Some(child) = c {
            if child.has_css_class("card-row") {
                out.push(child.clone());
            } else {
                rows(&child, out);
            }
            c = child.next_sibling();
        }
    }
    fn cards(w: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
        if w.has_css_class("settings-card") {
            out.push(w.clone());
            return;
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            cards(&child, out);
            c = child.next_sibling();
        }
    }
    let mut found = Vec::new();
    cards(within, &mut found);
    for card in found {
        let mut list = Vec::new();
        rows(&card, &mut list);
        let mut first = true;
        for r in list {
            // Visible here means the row and everything between it and the card.
            let shown = std::iter::successors(Some(r.clone()), |w| w.parent()).take_while(|w| *w != card).all(|w| w.is_visible());
            if shown && first {
                r.add_css_class("first-shown");
                first = false;
            } else {
                r.remove_css_class("first-shown");
            }
        }
    }
}

fn set_toggle_open(toggle: &gtk::Button, open: bool) {
    if open {
        toggle.add_css_class("open");
    } else {
        toggle.remove_css_class("open");
    }
    fn walk(w: &gtk::Widget, open: bool) {
        if w.has_css_class("titles-summary") {
            w.set_visible(!open);
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            walk(&child, open);
            c = child.next_sibling();
        }
    }
    walk(toggle.upcast_ref(), open);
}

thread_local! {
    /// Collapsible groups by their wrapper: the revealer, its toggle, and the prefs
    /// key with whether the group starts open.
    /// Weak, so a rebuilt page's groups go away with it.
    static COLLAPSIBLES: RefCell<Vec<Collapsible>> = const { RefCell::new(Vec::new()) };
}

struct Collapsible {
    wrapper: glib::WeakRef<gtk::Widget>,
    revealer: glib::WeakRef<gtk::Revealer>,
    toggle: glib::WeakRef<gtk::Button>,
    key: String,
    open_by_default: bool,
}

/// Search: open a collapsible group (`wrapper`) for now without remembering it,
/// or with `None`, put every group back the way the user left it.
pub fn reveal_for_search(wrapper: Option<&gtk::Widget>) {
    let saved = if wrapper.is_none() { crate::prefs::get().open_groups } else { Default::default() };
    COLLAPSIBLES.with(|c| {
        let mut c = c.borrow_mut();
        c.retain(|g| g.wrapper.upgrade().is_some());
        for g in c.iter() {
            let (Some(w), Some(revealer), Some(toggle)) = (g.wrapper.upgrade(), g.revealer.upgrade(), g.toggle.upgrade()) else {
                continue;
            };
            let open = match wrapper {
                Some(target) if *target == w => true,
                Some(_) => continue,
                None => saved.get(&g.key).copied().unwrap_or(g.open_by_default),
            };
            if revealer.reveals_child() != open {
                revealer.set_reveal_child(open);
                set_toggle_open(&toggle, open);
            }
        }
    });
}

// ---------- Config files and bar buttons ----------

// ---------- Rows ----------

/// An option card: title and description on the left, control on the right.
pub fn row(title: &str, desc: &str, control: Option<&gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.add_css_class("settings-option");
    row.add_css_class("card-row");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.add_css_class("row-text");
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("settings-option-title");
    t.set_xalign(0.0);
    t.set_wrap(true);
    // Long names with no spaces (service units, mount points) may break mid-word.
    t.set_wrap_mode(pango::WrapMode::WordChar);
    text.append(&t);
    if !desc.is_empty() {
        let d = gtk::Label::new(None);
        set_markup_or_text(&d, desc);
        d.add_css_class("settings-option-description");
        d.set_xalign(0.0);
        d.set_wrap(true);
        d.set_wrap_mode(pango::WrapMode::WordChar);
        text.append(&d);
    }
    row.append(&text);
    if let Some(c) = control {
        c.set_valign(gtk::Align::Center);
        row.append(c);
    }
    register(&row, title, desc, "");
    row
}

/// A row whose control sits underneath the text (for wide controls).
pub fn stacked_row(title: &str, desc: &str, control: &gtk::Widget) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 10);
    outer.add_css_class("settings-option");
    outer.add_css_class("card-row");
    outer.add_css_class("tall");
    if !title.is_empty() {
        let t = gtk::Label::new(Some(title));
        t.add_css_class("settings-option-title");
        t.set_xalign(0.0);
        outer.append(&t);
    }
    if !desc.is_empty() {
        let d = gtk::Label::new(None);
        set_markup_or_text(&d, desc);
        d.add_css_class("settings-option-description");
        d.set_xalign(0.0);
        d.set_wrap(true);
        outer.append(&d);
    }
    outer.append(control);
    register(&outer, title, desc, "");
    outer
}

pub fn switch_row(title: &str, desc: &str, active: bool, on_change: impl Fn(bool) + 'static) -> (gtk::Box, gtk::Switch) {
    let sw = gtk::Switch::new();
    sw.set_active(active);
    sw.connect_active_notify(move |s| on_change(s.is_active()));
    let r = row(title, desc, Some(sw.upcast_ref()));
    (r, sw)
}

pub struct Slider {
    pub scale: gtk::Scale,
    pub readout: gtk::Label,
}

pub fn format_value(v: f64, digits: u32, unit: &str) -> String {
    let n = if digits == 0 { format!("{}", v.round() as i64) } else { format!("{:.*}", digits as usize, v) };
    format!("{n}{unit}")
}

pub fn slider(min: f64, max: f64, step: f64, value: f64, digits: u32, unit: &str) -> (gtk::Box, Slider) {
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, step);
    scale.set_value(value);
    scale.set_draw_value(false);
    scale.set_digits(digits as i32);
    // Wide enough to drag precisely, narrow enough for a half-screen window.
    scale.set_width_request(180);
    let readout = gtk::Label::new(Some(&format_value(value, digits, unit)));
    readout.add_css_class("value-readout");
    readout.set_xalign(1.0);
    let unit = unit.to_string();
    let r = readout.clone();
    scale.connect_value_changed(move |s| r.set_text(&format_value(s.value(), digits, &unit)));
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    b.append(&scale);
    b.append(&readout);
    (b, Slider { scale, readout })
}

pub fn slider_row(
    title: &str,
    desc: &str,
    (min, max, step): (f64, f64, f64),
    value: f64,
    digits: u32,
    unit: &str,
    on_change: impl Fn(f64) + 'static,
) -> (gtk::Box, Slider) {
    let (b, s) = slider(min, max, step, value, digits, unit);
    s.scale.connect_value_changed(move |sc| on_change(sc.value()));
    let r = row(title, desc, Some(b.upcast_ref()));
    (r, s)
}

pub fn dropdown(options: &[(String, String)], current: &str) -> gtk::DropDown {
    let labels: Vec<&str> = options.iter().map(|(_, l)| l.as_str()).collect();
    let dd = gtk::DropDown::from_strings(&labels);
    // The closed button shortens a long choice with "…" instead of being as wide as
    // the longest option; the open list shows every option in full.
    dd.set_factory(Some(&string_factory(true)));
    dd.set_list_factory(Some(&string_factory(false)));
    if let Some(i) = options.iter().position(|(id, _)| id == current) {
        dd.set_selected(i as u32);
    } else {
        dd.set_selected(gtk::INVALID_LIST_POSITION);
    }
    dd
}

/// Labels for a drop-down of strings; `short` ones ellipsize. The open list's rows
/// also mark the current choice with a check, as GTK's own list does.
fn string_factory(short: bool) -> gtk::SignalListItemFactory {
    let f = gtk::SignalListItemFactory::new();
    f.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        let l = gtk::Label::new(None);
        l.set_xalign(0.0);
        if short {
            l.set_ellipsize(pango::EllipsizeMode::End);
            l.set_width_chars(8);
            l.set_max_width_chars(22);
            item.set_child(Some(&l));
            return;
        }
        l.set_hexpand(true);
        let check = gtk::Image::from_icon_name("object-select-symbolic");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&l);
        row.append(&check);
        item.set_child(Some(&row));
        item.connect_selected_notify(move |it| check.set_opacity(if it.is_selected() { 1.0 } else { 0.0 }));
    });
    f.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        let Some(s) = item.item().and_downcast::<gtk::StringObject>() else { return };
        let child = item.child();
        let label = child.as_ref().and_then(|c| c.downcast_ref::<gtk::Label>().cloned().or_else(|| c.first_child().and_downcast::<gtk::Label>()));
        if let Some(l) = label {
            l.set_text(&s.string());
        }
        if let Some(check) = child.and_then(|c| c.last_child()).and_downcast::<gtk::Image>() {
            check.set_opacity(if item.is_selected() { 1.0 } else { 0.0 });
        }
    });
    f
}

pub fn choice_row(
    title: &str,
    desc: &str,
    options: Vec<(String, String)>,
    current: &str,
    on_change: impl Fn(String) + 'static,
) -> (gtk::Box, gtk::DropDown) {
    let dd = dropdown(&options, current);
    dd.connect_selected_notify(move |d| {
        if let Some((id, _)) = options.get(d.selected() as usize) {
            on_change(id.clone());
        }
    });
    let r = row(title, desc, Some(dd.upcast_ref()));
    (r, dd)
}

pub fn entry_row(
    title: &str,
    desc: &str,
    value: &str,
    placeholder: &str,
    on_apply: impl Fn(String) + 'static,
) -> (gtk::Box, gtk::Entry) {
    let e = gtk::Entry::new();
    e.set_text(value);
    e.set_placeholder_text(Some(placeholder));
    e.set_width_chars(22);
    let apply = Rc::new(on_apply);
    let last = Rc::new(RefCell::new(value.to_string()));
    let fire = {
        let apply = apply.clone();
        let last = last.clone();
        move |e: &gtk::Entry| {
            let t = e.text().to_string();
            if *last.borrow() != t {
                *last.borrow_mut() = t.clone();
                apply(t);
            }
        }
    };
    let f1 = fire.clone();
    e.connect_activate(move |e| f1(e));
    let focus = gtk::EventControllerFocus::new();
    let e2 = e.clone();
    focus.connect_leave(move |_| fire(&e2));
    e.add_controller(focus);
    let r = row(title, desc, Some(e.upcast_ref()));
    (r, e)
}

pub fn button_row(title: &str, desc: &str, label: &str, on_click: impl Fn(&gtk::Button) + 'static) -> (gtk::Box, gtk::Button) {
    let b = gtk::Button::with_label(label);
    b.connect_clicked(on_click);
    let r = row(title, desc, Some(b.upcast_ref()));
    (r, b)
}

pub fn info_row(title: &str, value: &str) -> (gtk::Box, gtk::Label) {
    let l = gtk::Label::new(Some(value));
    l.add_css_class("dim");
    l.set_selectable(true);
    // Selectable with the mouse, but not a stop for Tab (it would open with its text selected).
    l.set_can_focus(false);
    l.set_wrap(true);
    l.set_xalign(1.0);
    l.set_max_width_chars(48);
    let r = row(title, "", Some(l.upcast_ref()));
    (r, l)
}

pub fn command_button(label: &str, args: &'static [&'static str]) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.connect_clicked(move |_| cmd::spawn(args));
    b
}

/// A button that needs a second click within a few seconds, for things that
/// can't be undone (restart, shut down, removing something).
pub fn confirm_button(label: &str, confirm: &str, on_confirm: impl Fn(&gtk::Button) + 'static) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    let (label, confirm) = (label.to_string(), confirm.to_string());
    b.connect_clicked(move |b| {
        if !b.has_css_class("armed") {
            b.add_css_class("armed");
            b.set_label(&confirm);
            let (weak, label) = (b.downgrade(), label.clone());
            glib::timeout_add_seconds_local_once(4, move || {
                if let Some(b) = weak.upgrade() {
                    b.remove_css_class("armed");
                    b.set_label(&label);
                }
            });
            return;
        }
        b.remove_css_class("armed");
        on_confirm(b);
    });
    b
}

// ---------- Rows bound to Hyprland options ----------

thread_local! {
    /// What each Hyprland option row is called and which page it's on, for the
    /// list of changed settings.
    static OPTION_ROWS: RefCell<std::collections::HashMap<String, (String, String)>> = RefCell::default();
}

fn remember_option(key: &str, title: &str) {
    let section = CURRENT_SECTION.with(|s| s.borrow().clone());
    OPTION_ROWS.with(|m| {
        m.borrow_mut().entry(key.to_string()).or_insert((title.to_string(), section));
    });
}

/// The title and page of the row for a Hyprland option, once that page has been built.
pub fn option_row(key: &str) -> Option<(String, String)> {
    OPTION_ROWS.with(|m| m.borrow().get(key).cloned())
}

/// Small circular "reset" button that hands the option back to the user's config.
fn reset_button(key: &str, on_reset: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::from_icon_name("edit-undo-symbolic");
    b.add_css_class("reset-button");
    b.add_css_class("flat");
    b.set_tooltip_text(Some("Reset — use the value from your own config or Omarchy's default"));
    show_reset(&b, store::is_managed(key));
    b.set_valign(gtk::Align::Center);
    let k = key.to_string();
    b.connect_clicked(move |btn| {
        store::reset_option(&k);
        show_reset(btn, false);
        on_reset();
    });
    b
}

/// Hidden reset buttons keep their space, so the control beside them doesn't jump.
fn show_reset(b: &gtk::Button, shown: bool) {
    b.set_opacity(if shown { 1.0 } else { 0.0 });
    b.set_sensitive(shown);
    b.set_can_target(shown);
    b.set_can_focus(shown);
}

/// Wait for Hyprland to reload, then re-read a value.
fn after_reload(f: impl Fn() + 'static) {
    glib::timeout_add_local_once(std::time::Duration::from_millis(700), f);
}

pub fn hypr_switch(key: &'static str, title: &str, desc: &str) -> gtk::Box {
    remember_option(key, title);
    let current = store::option(key).and_then(|v| v.as_bool()).or_else(|| hypr::get_bool(key)).unwrap_or(false);
    let guard = Rc::new(Cell::new(false));
    let reset_slot: Rc<RefCell<Option<gtk::Button>>> = Rc::new(RefCell::new(None));
    let (r, sw) = {
        let guard = guard.clone();
        let reset_slot = reset_slot.clone();
        switch_row(title, desc, current, move |on| {
            if guard.get() {
                return;
            }
            store::set_option(key, json!(on));
            if let Some(b) = reset_slot.borrow().as_ref() {
                show_reset(b, true);
            }
        })
    };
    let sw2 = sw.clone();
    let reset = reset_button(key, move || {
        let sw = sw2.clone();
        let guard = guard.clone();
        after_reload(move || {
            guard.set(true);
            sw.set_active(hypr::get_bool(key).unwrap_or(false));
            guard.set(false);
        });
    });
    // Reset sits between the text and the control.
    r.insert_child_after(&reset, r.first_child().as_ref());
    *reset_slot.borrow_mut() = Some(reset);
    r
}

#[allow(clippy::too_many_arguments)]
pub fn hypr_slider(
    key: &'static str,
    title: &str,
    desc: &str,
    range: (f64, f64, f64),
    digits: u32,
    unit: &str,
    integer: bool,
) -> gtk::Box {
    remember_option(key, title);
    let current = store::option(key).and_then(|v| v.as_f64()).or_else(|| hypr::get_f64(key)).unwrap_or(range.0);
    let guard = Rc::new(Cell::new(false));
    let reset_slot: Rc<RefCell<Option<gtk::Button>>> = Rc::new(RefCell::new(None));
    let (r, s) = {
        let guard = guard.clone();
        let reset_slot = reset_slot.clone();
        slider_row(title, desc, range, current, digits, unit, move |v| {
            if guard.get() {
                return;
            }
            let value = if integer { json!(v.round() as i64) } else { json!((v * 1000.0).round() / 1000.0) };
            store::set_option(key, value);
            if let Some(b) = reset_slot.borrow().as_ref() {
                show_reset(b, true);
            }
        })
    };
    let scale = s.scale.clone();
    let reset = reset_button(key, move || {
        let scale = scale.clone();
        let guard = guard.clone();
        after_reload(move || {
            guard.set(true);
            if let Some(v) = hypr::get_f64(key) {
                scale.set_value(v);
            }
            guard.set(false);
        });
    });
    r.insert_child_after(&reset, r.first_child().as_ref());
    *reset_slot.borrow_mut() = Some(reset);
    r
}

pub fn hypr_choice(key: &'static str, title: &str, desc: &str, options: Vec<(String, String)>) -> gtk::Box {
    hypr_choice_typed(key, title, desc, options, false)
}

/// `numeric`: the option is an integer in Hyprland, the ids are its values.
pub fn hypr_choice_typed(key: &'static str, title: &str, desc: &str, options: Vec<(String, String)>, numeric: bool) -> gtk::Box {
    remember_option(key, title);
    let as_id = |v: &Value| match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => (if *b { "1" } else { "0" }).to_string(),
        other => other.to_string(),
    };
    let current = store::option(key).map(|v| as_id(&v)).or_else(|| hypr::get_option(key).map(|v| as_id(&v))).unwrap_or_default();
    let guard = Rc::new(Cell::new(false));
    let reset_slot: Rc<RefCell<Option<gtk::Button>>> = Rc::new(RefCell::new(None));
    let opts2 = options.clone();
    let (r, dd) = {
        let guard = guard.clone();
        let reset_slot = reset_slot.clone();
        choice_row(title, desc, options, &current, move |id| {
            if guard.get() {
                return;
            }
            let value = if numeric { id.parse::<i64>().map(|n| json!(n)).unwrap_or(json!(id)) } else { json!(id) };
            store::set_option(key, value);
            if let Some(b) = reset_slot.borrow().as_ref() {
                show_reset(b, true);
            }
        })
    };
    let dd2 = dd.clone();
    let reset = reset_button(key, move || {
        let dd = dd2.clone();
        let guard = guard.clone();
        let opts = opts2.clone();
        after_reload(move || {
            guard.set(true);
            let now = hypr::get_option(key).map(|v| as_id(&v)).unwrap_or_default();
            if let Some(i) = opts.iter().position(|(id, _)| *id == now) {
                dd.set_selected(i as u32);
            }
            guard.set(false);
        });
    });
    r.insert_child_after(&reset, r.first_child().as_ref());
    *reset_slot.borrow_mut() = Some(reset);
    r
}

// ---------- Shared helpers for hardware pages ----------

/// Runs the last call after things have been quiet for a while.
#[derive(Clone, Default)]
pub struct Debounce(Rc<Cell<Option<glib::SourceId>>>);

impl Debounce {
    pub fn call(&self, ms: u64, f: impl FnOnce() + 'static) {
        if let Some(id) = self.0.take() {
            id.remove();
        }
        let slot = self.0.clone();
        self.0.set(Some(glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
            slot.set(None);
            f();
        })));
    }
}

/// A colour picker button starting at `hex` (`#rrggbb`).
pub fn colour_button(hex: &str, on_change: impl Fn(String) + 'static) -> gtk::ColorDialogButton {
    let b = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
    b.set_rgba(&gtk::gdk::RGBA::parse(hex).unwrap_or(gtk::gdk::RGBA::WHITE));
    b.connect_rgba_notify(move |b| {
        let c = b.rgba();
        on_change(format!(
            "#{:02x}{:02x}{:02x}",
            (c.red() * 255.0).round() as u8,
            (c.green() * 255.0).round() as u8,
            (c.blue() * 255.0).round() as u8
        ));
    });
    b
}

/// A row of small on/off chips, e.g. "Boot / Awake / Sleep / Shutdown".
pub fn chip_toggles(labels: &[&str], states: &[bool], on_change: impl Fn(Vec<bool>) + 'static) -> gtk::Box {
    let bx = hbox(6);
    let buttons: Rc<Vec<gtk::ToggleButton>> = Rc::new(
        labels
            .iter()
            .zip(states)
            .map(|(l, on)| {
                let b = gtk::ToggleButton::with_label(l);
                b.add_css_class("chip");
                b.set_active(*on);
                if *on {
                    b.add_css_class("selected");
                }
                b
            })
            .collect(),
    );
    let on_change = Rc::new(on_change);
    for b in buttons.iter() {
        bx.append(b);
        let all = buttons.clone();
        let cb = on_change.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                b.add_css_class("selected");
            } else {
                b.remove_css_class("selected");
            }
            cb(all.iter().map(|x| x.is_active()).collect());
        });
    }
    bx
}

// ---------- Segmented control, swatches, disclosure, tags ----------

pub const PRESET_COLOURS: &[(&str, &str)] = &[
    ("#ffffff", "White"),
    ("#ff3b30", "Red"),
    ("#ff8c1a", "Orange"),
    ("#ffd60a", "Yellow"),
    ("#34c759", "Green"),
    ("#32d7e6", "Cyan"),
    ("#0a84ff", "Blue"),
    ("#8e5cf7", "Purple"),
    ("#ff4fa3", "Pink"),
];

/// Inline colour picker: optional "Theme" swatch, presets, a custom picker and a hex field.
/// `theme` is (label, colour) for the theme swatch.
pub struct ColourPicker {
    pub widget: gtk::Box,
}

pub fn colour_picker(current: &str, theme: Option<(&str, &str)>, on_change: impl Fn(String) + 'static) -> ColourPicker {
    let outer = hbox(6);
    outer.add_css_class("swatches");
    // The swatches wrap onto a second line where the space is narrow (a preview pane).
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_max_children_per_line(16);
    flow.set_column_spacing(6);
    flow.set_row_spacing(6);
    flow.set_hexpand(true);
    flow.set_valign(gtk::Align::Center);
    outer.append(&flow);
    let swatches: Rc<RefCell<Vec<(String, gtk::Button)>>> = Rc::default();
    let entry = gtk::Entry::new();
    entry.add_css_class("mono");
    entry.set_width_chars(8);
    entry.set_max_length(7);
    let guard = Rc::new(Cell::new(false));
    let on_change: Rc<dyn Fn(String)> = Rc::new(on_change);

    let mark: Rc<dyn Fn(&str)> = {
        let swatches = swatches.clone();
        let entry = entry.clone();
        let guard = guard.clone();
        Rc::new(move |hex: &str| {
            let hex = hex.to_lowercase();
            for (c, b) in swatches.borrow().iter() {
                if *c == hex {
                    b.add_css_class("selected");
                } else {
                    b.remove_css_class("selected");
                }
            }
            guard.set(true);
            entry.set_text(&hex);
            guard.set(false);
        })
    };

    let pick: Rc<dyn Fn(String)> = {
        let mark = mark.clone();
        let on_change = on_change.clone();
        Rc::new(move |hex: String| {
            mark(&hex);
            on_change(hex);
        })
    };

    let add_swatch = |hex: &str, tip: &str, extra_class: Option<&str>| {
        let b = gtk::Button::new();
        b.add_css_class("swatch-button");
        if let Some(c) = extra_class {
            b.add_css_class(c);
        }
        b.set_tooltip_text(Some(tip));
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("swatch-dot");
        paint(&dot, hex);
        b.set_child(Some(&dot));
        let h = hex.to_lowercase();
        let pick = pick.clone();
        let h2 = h.clone();
        b.connect_clicked(move |_| pick(h2.clone()));
        flow.insert(&b, -1);
        swatches.borrow_mut().push((h, b));
    };
    if let Some((label, hex)) = theme {
        add_swatch(hex, &format!("{label} ({hex})"), Some("theme-swatch"));
    }
    for (hex, name) in PRESET_COLOURS {
        add_swatch(hex, name, None);
    }

    // Custom: the full picker.
    let custom = gtk::Button::from_icon_name("color-select-symbolic");
    custom.add_css_class("swatch-button");
    custom.set_tooltip_text(Some("Custom color…"));
    {
        let pick = pick.clone();
        let entry = entry.clone();
        custom.connect_clicked(move |b| {
            let dialog = gtk::ColorDialog::new();
            dialog.set_with_alpha(false);
            let start = gtk::gdk::RGBA::parse(entry.text().as_str()).unwrap_or(gtk::gdk::RGBA::WHITE);
            let window = b.root().and_downcast::<gtk::Window>();
            let pick = pick.clone();
            dialog.choose_rgba(window.as_ref(), Some(&start), gtk::gio::Cancellable::NONE, move |r| {
                if let Ok(c) = r {
                    pick(format!(
                        "#{:02x}{:02x}{:02x}",
                        (c.red() * 255.0).round() as u8,
                        (c.green() * 255.0).round() as u8,
                        (c.blue() * 255.0).round() as u8
                    ));
                }
            });
        });
    }
    flow.insert(&custom, -1);
    let mut child = flow.first_child();
    while let Some(c) = child {
        c.set_focusable(false);
        child = c.next_sibling();
    }

    {
        let pick = pick.clone();
        let guard = guard.clone();
        let apply = move |e: &gtk::Entry| {
            if guard.get() {
                return;
            }
            let t = e.text().trim().to_lowercase();
            let t = if t.starts_with('#') { t } else { format!("#{t}") };
            if t.len() == 7 && t[1..].chars().all(|c| c.is_ascii_hexdigit()) {
                pick(t);
            } else {
                e.add_css_class("error");
            }
        };
        let a2 = apply.clone();
        entry.connect_activate(move |e| a2(e));
        let focus = gtk::EventControllerFocus::new();
        let e2 = entry.clone();
        focus.connect_leave(move |_| apply(&e2));
        entry.add_controller(focus);
        entry.connect_changed(|e| e.remove_css_class("error"));
    }
    outer.append(&entry);
    mark(current);
    ColourPicker { widget: outer }
}

/// A row that reveals more rows when clicked (collapsed by default).
pub fn disclosure(title: &str, desc: &str) -> (gtk::Box, gtk::Box) {
    let content = vbox(6);
    content.set_visible(false);
    let arrow = gtk::Image::from_icon_name("pan-end-symbolic");
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("disclosure");
    let inner = hbox(10);
    let text = vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = label(title, "settings-option-title");
    t.set_wrap(true);
    text.append(&t);
    if !desc.is_empty() {
        let d = label(desc, "settings-option-description");
        d.set_wrap(true);
        d.set_wrap_mode(pango::WrapMode::WordChar);
        text.append(&d);
    }
    inner.append(&text);
    arrow.set_valign(gtk::Align::Center);
    inner.append(&arrow);
    button.set_child(Some(&inner));
    arrow.add_css_class("chevron");
    let c = content.clone();
    button.connect_clicked(move |b| {
        let open = !c.is_visible();
        c.set_visible(open);
        if open {
            b.add_css_class("open");
        } else {
            b.remove_css_class("open");
        }
    });
    content.add_css_class("disclosure-content");
    let wrapper = vbox(0);
    wrapper.add_css_class("card-row");
    wrapper.add_css_class("disclosure-row");
    wrapper.append(&button);
    wrapper.append(&content);
    register(&button, title, desc, "");
    (wrapper, content)
}

/// Search exposes controls inside disclosure rows as well as their enclosing section.
pub fn reveal_disclosure_ancestors(row: &gtk::Widget) {
    let mut ancestor = row.parent();
    while let Some(widget) = ancestor {
        if widget.has_css_class("disclosure-row")
            && let Some(wrapper) = widget.downcast_ref::<gtk::Box>()
        {
            wrapper.set_visible(true);
            open_disclosure(wrapper);
        }
        ancestor = widget.parent();
    }
}

/// Open a disclosure made by [`disclosure`] (its button, then its content).
pub fn open_disclosure(wrapper: &gtk::Box) {
    if let (Some(button), Some(content)) = (wrapper.first_child(), wrapper.last_child()) {
        button.set_visible(true);
        button.add_css_class("open");
        content.set_visible(true);
    }
}

/// A form behind an "Add…" row that opens it: the usual way to add things to a list.
pub fn form_disclosure(title: &str, desc: &str, form: &impl IsA<gtk::Widget>) -> gtk::Box {
    let (wrapper, content) = disclosure(title, desc);
    content.append(form);
    wrapper
}

/// "Open config" over a page: opens its config file, or offers a menu of them when
/// there are several.
pub fn open_config_button(files: &[std::path::PathBuf]) -> gtk::Widget {
    let content = hbox(6);
    content.append(&gtk::Image::from_icon_name("text-editor-symbolic"));
    content.append(&gtk::Label::new(Some("Open config")));
    let tooltip = match files {
        [path] => format!("Open {}", nyx_core::paths::pretty(path)),
        _ => "Open a config file".to_string(),
    };
    let widget: gtk::Widget = if let [path] = files {
        let b = gtk::Button::new();
        b.set_child(Some(&content));
        let path = path.clone();
        b.connect_clicked(move |_| nyx_core::cmd::open_in_editor(&path));
        b.upcast()
    } else {
        let menu = gtk::MenuButton::new();
        menu.set_child(Some(&content));
        menu.set_always_show_arrow(false);
        let list = vbox(2);
        let popover = gtk::Popover::new();
        for path in files {
            let b = gtk::Button::with_label(&nyx_core::paths::pretty(path));
            b.add_css_class("flat");
            if let Some(label) = b.child().and_downcast::<gtk::Label>() {
                label.set_xalign(0.0);
            }
            let p = path.clone();
            let pop = popover.clone();
            b.connect_clicked(move |_| {
                pop.popdown();
                nyx_core::cmd::open_in_editor(&p);
            });
            list.append(&b);
        }
        popover.set_child(Some(&list));
        menu.set_popover(Some(&popover));
        menu.upcast()
    };
    widget.add_css_class("open-config");
    widget.add_css_class("flat");
    widget.set_valign(gtk::Align::Center);
    widget.set_tooltip_text(Some(&tooltip));
    widget
}

/// A small pill label, e.g. "Restart needed".
pub fn tag(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("tag");
    l.set_valign(gtk::Align::Center);
    l
}

/// Put a tag right after a row's title.
pub fn tag_row(row: &gtk::Box, text: &str) {
    let mut texts = row.first_child();
    while let Some(t) = texts.as_ref().filter(|t| t.has_css_class("row-tile")) {
        texts = t.next_sibling();
    }
    if let Some(title) = texts.and_then(|t| t.first_child())
        && let (Some(parent), Some(t)) = (title.parent().and_downcast::<gtk::Box>(), title.downcast_ref::<gtk::Label>())
    {
        let line = hbox(8);
        parent.insert_child_after(&line, Some(t));
        parent.remove(t);
        // Beside a tag and a wide control, a title mustn't squeeze to a letter a line.
        t.set_width_chars(t.text().chars().count().min(10) as i32);
        line.append(t);
        line.append(&tag(text));
    }
}

// ---------- Pictures ----------

/// Picks a card by its id without telling the page.
pub type Pick = Rc<dyn Fn(&str)>;

/// Choices shown as picture cards: a drawing of each with its name under it.
/// Returns the row and a way to pick a card without calling `on_change`.
pub fn visual_choice(
    title: &str,
    desc: &str,
    options: Vec<(String, String, Sketch)>,
    current: &str,
    on_change: impl Fn(String) + 'static,
) -> (gtk::Box, gtk::Box, Pick) {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 12);
    outer.add_css_class("settings-option");
    outer.add_css_class("card-row");
    outer.add_css_class("tall");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("settings-option-title");
    t.set_xalign(0.0);
    text.append(&t);
    if !desc.is_empty() {
        let d = gtk::Label::new(None);
        set_markup_or_text(&d, desc);
        d.add_css_class("settings-option-description");
        d.set_xalign(0.0);
        d.set_wrap(true);
        text.append(&d);
    }
    head.append(&text);
    outer.append(&head);

    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_homogeneous(true);
    flow.set_min_children_per_line(2);
    flow.set_max_children_per_line(4);
    flow.set_column_spacing(10);
    flow.set_row_spacing(10);
    flow.add_css_class("picture-cards");
    let guard = Rc::new(Cell::new(false));
    let on_change = Rc::new(on_change);
    let mut buttons: Vec<(String, gtk::ToggleButton)> = Vec::new();
    let mut first: Option<gtk::ToggleButton> = None;
    for (id, name, sk) in options {
        let b = gtk::ToggleButton::new();
        b.add_css_class("picture-card");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let a = sketch::area(sk, 132, 80, Rc::new(Cell::new(true)));
        a.set_halign(gtk::Align::Center);
        content.append(&a);
        let l = gtk::Label::new(Some(&name));
        l.add_css_class("picture-card-label");
        l.set_wrap(true);
        l.set_justify(gtk::Justification::Center);
        l.set_max_width_chars(18);
        content.append(&l);
        b.set_child(Some(&content));
        b.set_tooltip_text(Some(&name));
        match &first {
            Some(g) => b.set_group(Some(g)),
            None => first = Some(b.clone()),
        }
        b.set_active(id == current);
        let (guard, f, id2) = (guard.clone(), on_change.clone(), id.clone());
        b.connect_toggled(move |b| {
            if b.is_active() && !guard.get() {
                f(id2.clone());
            }
        });
        flow.append(&b);
        buttons.push((id, b));
    }
    outer.append(&flow);
    register(&outer, title, desc, "");
    let set: Pick = Rc::new(move |id: &str| {
        guard.set(true);
        for (i, b) in &buttons {
            if i == id {
                b.set_active(true);
            }
        }
        guard.set(false);
    });
    (outer, head, set)
}

/// A Hyprland option as picture cards, with the usual reset button.
pub fn hypr_visual_choice(
    key: &'static str,
    title: &str,
    desc: &str,
    options: Vec<(String, String, Sketch)>,
    numeric: bool,
) -> gtk::Box {
    remember_option(key, title);
    let as_id = |v: &Value| match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => (if *b { "1" } else { "0" }).to_string(),
        other => other.to_string(),
    };
    let current = store::option(key).map(|v| as_id(&v)).or_else(|| hypr::get_option(key).map(|v| as_id(&v))).unwrap_or_default();
    let reset_slot: Rc<RefCell<Option<gtk::Button>>> = Rc::new(RefCell::new(None));
    let rs = reset_slot.clone();
    let (outer, head, set) = visual_choice(title, desc, options, &current, move |id| {
        let value = if numeric { id.parse::<i64>().map(|n| json!(n)).unwrap_or(json!(id)) } else { json!(id) };
        store::set_option(key, value);
        if let Some(b) = rs.borrow().as_ref() {
            show_reset(b, true);
        }
    });
    let reset = reset_button(key, move || {
        let set = set.clone();
        after_reload(move || set(&hypr::get_option(key).map(|v| as_id(&v)).unwrap_or_default()));
    });
    head.append(&reset);
    *reset_slot.borrow_mut() = Some(reset);
    outer
}

/// A small drawing at the start of a row. If the row has a switch, the
/// drawing shows its state.
pub fn sketched(row: &gtk::Box, s: Sketch) {
    fn find_switch(w: &gtk::Widget) -> Option<gtk::Switch> {
        if let Some(s) = w.downcast_ref::<gtk::Switch>() {
            return Some(s.clone());
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            if let Some(s) = find_switch(&c) {
                return Some(s);
            }
            child = c.next_sibling();
        }
        None
    }
    let on = Rc::new(Cell::new(true));
    let a = sketch::area(s, 64, 40, on.clone());
    a.set_valign(gtk::Align::Center);
    a.add_css_class("row-sketch");
    if let Some(sw) = find_switch(row.upcast_ref()) {
        on.set(sw.is_active());
        let a2 = a.clone();
        sw.connect_active_notify(move |s| {
            on.set(s.is_active());
            a2.queue_draw();
        });
    }
    row.prepend(&a);
}

/// `text` as Pango markup, or as it is when it isn't valid markup (a bare `&`, say),
/// so a description never goes blank.
pub fn set_markup_or_text(l: &gtk::Label, text: &str) {
    if gtk::pango::parse_markup(text, '\0').is_ok() {
        l.set_markup(text);
    } else {
        l.set_text(text);
    }
}
