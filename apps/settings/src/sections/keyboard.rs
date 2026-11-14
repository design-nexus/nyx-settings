//! Input › Keyboard: layouts, what Caps Lock does, key repeat and the keyboard light,
//! beside a picture of the keyboard as it types in the picked layout.
//!
//! The keyboard drawing ([`Board`]) is shared with Keyboard Shortcuts.

use crate::backend::{hypr, kbdidle, store};
use crate::sketch::{self, Pal, Rgb};
use crate::widgets::{self, Page, hypr_slider, hypr_switch};
use crate::{cmd, window};
use gtk::cairo::Context;
use gtk::prelude::*;
use gtk::{gdk, glib};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

// ---------- The keyboard drawing ----------

/// A key on the drawn keyboard: its X keycode (Hyprland's `code:N`), its xkb name,
/// what it says on a US keyboard, and how wide it is in key widths.
pub struct Key {
    pub code: u32,
    pub name: &'static str,
    pub us: &'static str,
    pub w: f64,
}

const fn k(code: u32, name: &'static str, us: &'static str, w: f64) -> Key {
    Key { code, name, us, w }
}

/// A laptop keyboard, row by row.
pub const ROWS: [&[Key]; 6] = [
    &[
        k(9, "ESC", "Esc", 1.2),
        k(67, "FK01", "F1", 1.0),
        k(68, "FK02", "F2", 1.0),
        k(69, "FK03", "F3", 1.0),
        k(70, "FK04", "F4", 1.0),
        k(71, "FK05", "F5", 1.0),
        k(72, "FK06", "F6", 1.0),
        k(73, "FK07", "F7", 1.0),
        k(74, "FK08", "F8", 1.0),
        k(75, "FK09", "F9", 1.0),
        k(76, "FK10", "F10", 1.0),
        k(95, "FK11", "F11", 1.0),
        k(96, "FK12", "F12", 1.0),
        k(107, "PRSC", "Print", 1.3),
    ],
    &[
        k(49, "TLDE", "`", 1.0),
        k(10, "AE01", "1", 1.0),
        k(11, "AE02", "2", 1.0),
        k(12, "AE03", "3", 1.0),
        k(13, "AE04", "4", 1.0),
        k(14, "AE05", "5", 1.0),
        k(15, "AE06", "6", 1.0),
        k(16, "AE07", "7", 1.0),
        k(17, "AE08", "8", 1.0),
        k(18, "AE09", "9", 1.0),
        k(19, "AE10", "0", 1.0),
        k(20, "AE11", "-", 1.0),
        k(21, "AE12", "=", 1.0),
        k(22, "BKSP", "Back", 1.8),
    ],
    &[
        k(23, "TAB", "Tab", 1.5),
        k(24, "AD01", "Q", 1.0),
        k(25, "AD02", "W", 1.0),
        k(26, "AD03", "E", 1.0),
        k(27, "AD04", "R", 1.0),
        k(28, "AD05", "T", 1.0),
        k(29, "AD06", "Y", 1.0),
        k(30, "AD07", "U", 1.0),
        k(31, "AD08", "I", 1.0),
        k(32, "AD09", "O", 1.0),
        k(33, "AD10", "P", 1.0),
        k(34, "AD11", "[", 1.0),
        k(35, "AD12", "]", 1.0),
        k(51, "BKSL", "\\", 1.3),
    ],
    &[
        k(66, "CAPS", "Caps", 1.8),
        k(38, "AC01", "A", 1.0),
        k(39, "AC02", "S", 1.0),
        k(40, "AC03", "D", 1.0),
        k(41, "AC04", "F", 1.0),
        k(42, "AC05", "G", 1.0),
        k(43, "AC06", "H", 1.0),
        k(44, "AC07", "J", 1.0),
        k(45, "AC08", "K", 1.0),
        k(46, "AC09", "L", 1.0),
        k(47, "AC10", ";", 1.0),
        k(48, "AC11", "'", 1.0),
        k(36, "RTRN", "Return", 2.0),
    ],
    &[
        k(50, "LFSH", "Shift", 2.3),
        k(52, "AB01", "Z", 1.0),
        k(53, "AB02", "X", 1.0),
        k(54, "AB03", "C", 1.0),
        k(55, "AB04", "V", 1.0),
        k(56, "AB05", "B", 1.0),
        k(57, "AB06", "N", 1.0),
        k(58, "AB07", "M", 1.0),
        k(59, "AB08", ",", 1.0),
        k(60, "AB09", ".", 1.0),
        k(61, "AB10", "/", 1.0),
        k(62, "RTSH", "Shift", 2.5),
    ],
    &[
        k(37, "LCTL", "Ctrl", 1.4),
        k(0, "FN", "Fn", 1.0),
        k(133, "LWIN", "Super", 1.4),
        k(64, "LALT", "Alt", 1.2),
        k(65, "SPCE", "Space", 6.2),
        k(108, "RALT", "Alt", 1.2),
        k(105, "RCTL", "Ctrl", 1.4),
        k(113, "LEFT", "←", 1.0),
        k(114, "RGHT", "→", 1.0),
    ],
];

/// How one key is drawn.
#[derive(Clone, Default)]
pub struct Look {
    /// The key's label; empty keeps the US one.
    pub label: String,
    /// A second, bolder label under a small one (Caps Lock: "Caps" over "Compose").
    pub action: String,
    /// Filled with this colour (a coloured group, a marked key).
    pub fill: Option<Rgb>,
    /// Outlined in this colour.
    pub ring: Option<Rgb>,
    /// Lit plainly (a held modifier).
    pub lit: bool,
    /// Picked: a strong outline in the accent.
    pub picked: bool,
}

/// Where each key is in a `w`×`h` drawing.
pub fn key_rects(w: f64, h: f64) -> Vec<(&'static Key, sketch::R)> {
    let gap = (w * 0.008).clamp(3.0, 6.0);
    let rh = (h - gap * 5.0) / 6.0;
    let mut out = Vec::new();
    for (ri, row) in ROWS.iter().enumerate() {
        let units: f64 = row.iter().map(|k| k.w).sum();
        let unit = (w - gap * (row.len() as f64 - 1.0)) / units;
        let mut x = 0.0;
        let y = ri as f64 * (rh + gap);
        for key in row.iter() {
            let kw = unit * key.w;
            out.push((key, sketch::R::new(x, y, kw, rh)));
            x += kw + gap;
        }
    }
    out
}

/// The key under (x, y) in a `w`×`h` drawing.
pub fn key_at(w: f64, h: f64, x: f64, y: f64) -> Option<&'static Key> {
    key_rects(w, h).into_iter().find(|(_, r)| x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h).map(|(k, _)| k)
}

/// Draw the keyboard, each key as `look` says.
pub fn draw_board(cr: &Context, p: &Pal, w: f64, h: f64, look: impl Fn(&Key) -> Look) {
    let rects = key_rects(w, h);
    let rh = rects.first().map(|(_, r)| r.h).unwrap_or(20.0);
    for (key, r) in rects {
        let l = look(key);
        let r = sketch::R::new(r.x + 0.5, r.y + 0.5, r.w - 1.0, r.h - 1.0);
        let radius = (rh * 0.16).clamp(3.0, 6.0);
        sketch::rounded(cr, r.x, r.y, r.w, r.h, radius);
        let base = sketch::mix(p.bg, p.text, 0.07);
        let fill = match (l.fill, l.lit) {
            (Some(c), _) => sketch::mix(p.bg, c, 0.32),
            (None, true) => sketch::mix(p.bg, p.text, 0.85),
            _ => base,
        };
        sketch::set(cr, fill, 1.0);
        let _ = cr.fill_preserve();
        let (edge, width) = match (l.picked, l.ring, l.fill) {
            (true, _, _) => (p.accent, 2.0),
            (_, Some(c), _) => (c, 1.6),
            (_, _, Some(c)) => (c, 1.0),
            _ => (sketch::mix(p.bg, p.text, 0.16), 1.0),
        };
        sketch::set(cr, edge, if l.fill.is_some() && !l.picked && l.ring.is_none() { 0.7 } else { 1.0 });
        cr.set_line_width(width);
        let _ = cr.stroke();
        let ink = if l.lit { p.bg } else if l.fill.is_some() || l.picked { p.text } else { p.dim };
        let label = if l.label.is_empty() { key.us.to_string() } else { l.label.clone() };
        let size = (rh * 0.3).clamp(8.0, 12.0);
        if l.action.is_empty() {
            key_text(cr, &label, r, size, ink, key.w > 1.05);
        } else {
            cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
            fit_font(cr, &label, size * 0.8, r.w - 8.0);
            sketch::set(cr, ink, 0.75);
            cr.move_to(r.x + 4.0, r.y + size * 0.95 + 2.0);
            let _ = cr.show_text(&label);
            cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
            fit_font(cr, &l.action, size, r.w - 8.0);
            sketch::set(cr, p.text, 1.0);
            cr.move_to(r.x + 4.0, r.y + r.h - 5.0);
            let _ = cr.show_text(&l.action);
        }
    }
}

/// Set the font size to `size`, or smaller so `text` fits in `room`.
fn fit_font(cr: &Context, text: &str, size: f64, room: f64) {
    cr.set_font_size(size);
    if let Ok(ext) = cr.text_extents(text)
        && ext.x_advance() > room
        && room > 0.0
    {
        cr.set_font_size((size * room / ext.x_advance()).max(6.0));
    }
}

/// A key's label: centred on a letter key, at the bottom left of a wide one.
fn key_text(cr: &Context, text: &str, r: sketch::R, size: f64, ink: Rgb, wide: bool) {
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
    fit_font(cr, text, if wide { size * 0.85 } else { size }, r.w - 6.0);
    let Ok(ext) = cr.text_extents(text) else { return };
    sketch::set(cr, ink, 1.0);
    if wide {
        cr.move_to(r.x + 6.0, r.y + r.h - 6.0);
    } else {
        cr.move_to(r.cx() - ext.width() / 2.0 - ext.x_bearing(), r.cy() - ext.height() / 2.0 - ext.y_bearing());
    }
    let _ = cr.show_text(text);
}

// ---------- Layouts, from the system's xkb data ----------

/// Where xkb keeps its layouts.
fn xkb_root() -> Option<PathBuf> {
    let mut places: Vec<PathBuf> = Vec::new();
    if let Some(r) = std::env::var_os("XKB_CONFIG_ROOT") {
        places.push(r.into());
    }
    places.extend(["/usr/share/X11/xkb", "/run/current-system/sw/share/X11/xkb", "/etc/X11/xkb"].map(PathBuf::from));
    places.into_iter().find(|p| p.join("symbols").is_dir())
}

/// Every layout and variant xkb knows, with its name: (layout, variant, name).
fn catalogue() -> Vec<(String, String, String)> {
    let Some(root) = xkb_root() else { return Vec::new() };
    let text = ["evdev.lst", "base.lst"].iter().find_map(|f| std::fs::read_to_string(root.join("rules").join(f)).ok()).unwrap_or_default();
    let mut out = Vec::new();
    let mut part = "";
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("! ") {
            part = p.trim();
            continue;
        }
        let line = line.trim();
        let Some((code, rest)) = line.split_once(char::is_whitespace) else { continue };
        let rest = rest.trim();
        match part {
            "layout" => out.push((code.to_string(), String::new(), rest.to_string())),
            "variant" => {
                if let Some((layout, name)) = rest.split_once(": ") {
                    out.push((layout.to_string(), code.to_string(), name.to_string()));
                }
            }
            _ => {}
        }
    }
    out
}

thread_local! {
    static CATALOGUE: RefCell<Option<Rc<Vec<(String, String, String)>>>> = const { RefCell::new(None) };
}

fn known() -> Rc<Vec<(String, String, String)>> {
    CATALOGUE.with(|c| c.borrow_mut().get_or_insert_with(|| Rc::new(catalogue())).clone())
}

/// A layout's name, like "German (no dead keys)", or its code when xkb doesn't know it.
fn layout_name(layout: &str, variant: &str) -> String {
    known()
        .iter()
        .find(|(l, v, _)| l == layout && v == variant)
        .map(|(_, _, n)| n.clone())
        .unwrap_or_else(|| if variant.is_empty() { layout.to_string() } else { format!("{layout} ({variant})") })
}

/// Level 1 and 2 of every key a layout sets, by xkb key name, following its includes.
fn symbols(layout: &str, variant: &str) -> HashMap<String, (String, String)> {
    let mut out = HashMap::new();
    if let Some(root) = xkb_root() {
        let spec = if variant.is_empty() { layout.to_string() } else { format!("{layout}({variant})") };
        include(&root, &spec, 0, &mut out);
    }
    out
}

fn include(root: &std::path::Path, spec: &str, depth: u32, out: &mut HashMap<String, (String, String)>) {
    if depth > 8 {
        return;
    }
    for part in spec.split(['+', '|']).map(str::trim).filter(|s| !s.is_empty()) {
        // Parts for another group (`us:2`) don't change what the first one types.
        let part = match part.split_once(':') {
            Some((p, g)) if g.trim() == "1" => p,
            Some(_) => continue,
            None => part,
        };
        let (file, section) = match part.split_once('(') {
            Some((f, s)) => (f, Some(s.trim_end_matches(')'))),
            None => (part, None),
        };
        if file.contains('/') || file.contains("..") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join("symbols").join(file)) else { continue };
        if let Some(body) = section_body(&text, section) {
            read_section(root, &body, depth, out);
        }
    }
}

/// The body of `xkb_symbols "section"` in a symbols file (the default one when `None`).
fn section_body(text: &str, section: Option<&str>) -> Option<String> {
    let clean: String = text.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    let mut found: Option<(usize, bool, String)> = None;
    let mut first: Option<usize> = None;
    let mut at = 0;
    while let Some(i) = clean[at..].find("xkb_symbols") {
        let start = at + i;
        let before = &clean[at..start];
        let is_default = before.split_whitespace().rev().take_while(|w| !w.ends_with(';') && !w.ends_with('}')).any(|w| w == "default");
        let rest = &clean[start..];
        let name = rest.split('"').nth(1).unwrap_or("").to_string();
        first.get_or_insert(start);
        let wanted = match section {
            Some(s) => name == s,
            None => is_default,
        };
        if wanted {
            found = Some((start, is_default, name));
            break;
        }
        at = start + 11;
    }
    let start = found.map(|(s, _, _)| s).or(if section.is_none() { first } else { None })?;
    let open = start + clean[start..].find('{')?;
    let mut depth = 0;
    for (i, c) in clean[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(clean[open + 1..open + i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn read_section(root: &std::path::Path, body: &str, depth: u32, out: &mut HashMap<String, (String, String)>) {
    let mut statements = Vec::new();
    let (mut level, mut cur) = (0, String::new());
    for c in body.chars() {
        match c {
            '{' | '[' => level += 1,
            '}' | ']' => level -= 1,
            _ => {}
        }
        if c == ';' && level == 0 {
            statements.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    statements.push(cur);
    for s in statements {
        let s = s.trim();
        let word = s.split_whitespace().next().unwrap_or("");
        if matches!(word, "include" | "augment" | "replace" | "override") && !s.contains("key <") {
            if let Some(spec) = s.split('"').nth(1) {
                include(root, spec, depth + 1, out);
            }
            continue;
        }
        let Some(i) = s.find("key <") else { continue };
        let rest = &s[i + 5..];
        let Some((name, rest)) = rest.split_once('>') else { continue };
        let Some(open) = rest.find('[') else { continue };
        let Some(close) = rest[open..].find(']') else { continue };
        let syms: Vec<String> = rest[open + 1..open + close].split(',').map(|x| x.trim().to_string()).collect();
        let l1 = syms.first().cloned().unwrap_or_default();
        let l2 = syms.get(1).cloned().unwrap_or_default();
        if !l1.is_empty() {
            out.insert(name.trim().to_string(), (l1, l2));
        }
    }
}

/// The character a keysym types, like `adiaeresis` → ä.
fn sym_char(s: &str) -> Option<String> {
    if s.chars().count() == 1 {
        return Some(s.to_string());
    }
    if let Some(hex) = s.strip_prefix('U').filter(|h| h.len() >= 4 && h.chars().all(|c| c.is_ascii_hexdigit())) {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).map(String::from);
    }
    if let Some(dead) = s.strip_prefix("dead_") {
        let c = match dead {
            "acute" => "´",
            "grave" => "`",
            "circumflex" => "^",
            "tilde" => "~",
            "diaeresis" => "¨",
            "cedilla" => "¸",
            "abovering" => "°",
            "caron" => "ˇ",
            _ => return None,
        };
        return Some(c.to_string());
    }
    gdk::Key::from_name(s).and_then(|k| k.to_unicode()).filter(|c| !c.is_control()).map(String::from)
}

/// What each letter and symbol key says in a layout: capitals for letters.
fn key_labels(layout: &str, variant: &str) -> HashMap<String, String> {
    symbols(layout, variant)
        .into_iter()
        .filter_map(|(name, (l1, l2))| {
            let c1 = sym_char(&l1)?;
            let upper = sym_char(&l2).filter(|u| *u == c1.to_uppercase() && *u != c1);
            Some((name, upper.unwrap_or(c1)))
        })
        .collect()
}

// ---------- The settings ----------

fn option_str(key: &str) -> String {
    store::option(key).and_then(|v| v.as_str().map(String::from)).or_else(|| hypr::get_str(key)).unwrap_or_default()
}

fn option_f64(key: &str, default: f64) -> f64 {
    store::option(key).and_then(|v| v.as_f64()).or_else(|| hypr::get_f64(key)).unwrap_or(default)
}

/// The layouts in order, each with its variant.
fn layouts() -> Vec<(String, String)> {
    let codes = option_str("input.kb_layout");
    let variants = option_str("input.kb_variant");
    let variants: Vec<&str> = variants.split(',').map(str::trim).collect();
    let mut out: Vec<(String, String)> = codes
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .enumerate()
        .map(|(i, c)| (c.to_string(), variants.get(i).copied().unwrap_or("").to_string()))
        .collect();
    if out.is_empty() {
        out.push(("us".into(), String::new()));
    }
    out
}

fn save_layouts(list: &[(String, String)]) {
    let codes: Vec<&str> = list.iter().map(|(c, _)| c.as_str()).collect();
    let variants: Vec<&str> = list.iter().map(|(_, v)| v.as_str()).collect();
    store::set_option("input.kb_layout", json!(codes.join(",")));
    let v = if variants.iter().all(|v| v.is_empty()) { String::new() } else { variants.join(",") };
    if v.is_empty() && !store::is_managed("input.kb_variant") && option_str("input.kb_variant").is_empty() {
        return;
    }
    store::set_option("input.kb_variant", json!(v));
}

/// What Caps Lock does: the xkb options for it, and what it reads as.
const CAPS: &[(&str, &str)] = &[
    ("compose:caps,shift:both_capslock_cancel", "Compose"),
    ("", "Caps Lock"),
    ("caps:escape", "Escape"),
    ("caps:escape_shifted_capslock", "Escape (Shift: Caps)"),
    ("ctrl:nocaps", "Control"),
    ("caps:backspace", "Backspace"),
    ("caps:super", "Super"),
    ("caps:none", "Nothing"),
    ("compose:ralt", "Caps (Right Alt: Compose)"),
];

/// How to type é, for what Caps Lock does now.
fn caps_desc(rest: &str) -> &'static str {
    let has = |o: &str| rest.split(',').any(|p| p == o);
    if has("compose:caps") {
        "Caps, ' then e types é"
    } else if has(OMARCHY_CAPS.0) {
        "Shift + Caps, ' then e types é"
    } else if has("compose:ralt") {
        "Right Alt, ' then e types é"
    } else {
        ""
    }
}

/// Omarchy's own option (defined in its `/etc/xkb`): Caps Lock stays Caps Lock, and
/// Shift + Caps Lock is Compose.
const OMARCHY_CAPS: (&str, &str) = ("omarchy:shift_caps_compose", "Caps (Shift: Compose)");

/// Ways to switch between layouts (xkb's `grp:` options).
const SWITCH: &[(&str, &str)] = &[
    ("", "No key"),
    ("grp:alts_toggle", "Both Alt keys"),
    ("grp:alt_shift_toggle", "Alt + Shift"),
    ("grp:win_space_toggle", "Super + Space"),
    ("grp:ctrl_shift_toggle", "Ctrl + Shift"),
    ("grp:caps_toggle", "Caps Lock"),
];

/// `input.kb_options` split into the layout switch and everything else.
fn split_options(all: &str) -> (String, String) {
    let parts: Vec<&str> = all.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
    let switch = parts.iter().find(|p| p.starts_with("grp:")).map(|s| s.to_string()).unwrap_or_default();
    let rest: Vec<&str> = parts.into_iter().filter(|p| !p.starts_with("grp:")).collect();
    (switch, rest.join(","))
}

fn join_options(switch: &str, rest: &str) -> String {
    [rest, switch].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(",")
}

/// The keys marked on the picture, and what the legend says about them.
struct Marks {
    keys: HashMap<&'static str, (String, bool)>,
    legend: Vec<(String, bool)>,
}

/// `hot` marks are what Caps Lock (or Compose) became; the others switch layouts.
fn marks(options: &str, several: bool) -> Marks {
    let (switch, rest) = split_options(options);
    let mut keys = HashMap::new();
    let mut legend = Vec::new();
    let has = |o: &str| rest.split(',').any(|p| p == o);
    let caps = if has("compose:caps") {
        Some("Compose")
    } else if has("caps:escape") || has("caps:escape_shifted_capslock") {
        Some("Esc")
    } else if has("ctrl:nocaps") || has("caps:ctrl_modifier") {
        Some("Ctrl")
    } else if has("caps:backspace") {
        Some("⌫")
    } else if has("caps:super") {
        Some("Super")
    } else if has("caps:none") {
        Some("Off")
    } else {
        None
    };
    if let Some(c) = caps {
        keys.insert("CAPS", (c.to_string(), true));
        legend.push((if c == "Off" { "Caps Lock does nothing".to_string() } else { format!("Caps Lock is {c}") }, true));
    } else if has(OMARCHY_CAPS.0) {
        keys.insert("CAPS", ("⇧ Compose".to_string(), true));
        legend.push(("Shift + Caps Lock is Compose".to_string(), true));
    }
    if has("compose:ralt") {
        keys.insert("RALT", ("Compose".to_string(), true));
        legend.push(("Right Alt is Compose".to_string(), true));
    }
    if several {
        let (names, text): (&[&'static str], &str) = match switch.as_str() {
            "grp:alts_toggle" => (&["LALT", "RALT"], "Both Alt keys switch layouts"),
            "grp:alt_shift_toggle" => (&["LALT", "LFSH"], "Alt + Shift switches layouts"),
            "grp:win_space_toggle" => (&["LWIN", "SPCE"], "Super + Space switches layouts"),
            "grp:ctrl_shift_toggle" => (&["LCTL", "LFSH"], "Ctrl + Shift switches layouts"),
            "grp:caps_toggle" => (&["CAPS"], "Caps Lock switches layouts"),
            _ => (&[], ""),
        };
        for n in names {
            keys.entry(*n).or_insert((String::new(), false));
        }
        if !text.is_empty() {
            legend.push((text.to_string(), false));
        }
    }
    Marks { keys, legend }
}

/// The line under the picture's title.
fn switch_line(list: &[(String, String)], shown: usize, options: &str) -> String {
    if list.len() < 2 {
        return "Add another layout to type in more than one language.".into();
    }
    let (switch, _) = split_options(options);
    let next = layout_name(&list[(shown + 1) % list.len()].0, &list[(shown + 1) % list.len()].1);
    match SWITCH.iter().find(|(id, _)| *id == switch && !id.is_empty()) {
        Some((_, how)) => format!("Press {} to switch to {next}.", how.replace("Both", "both")),
        None => format!("Pick a key on the left to switch to {next}."),
    }
}

/// The strip under Try it: the wait after pressing a key, then a tick for each repeat.
fn draw_repeat(cr: &Context, p: &Pal, w: f64, h: f64, delay: f64, rate: f64) {
    let total = 1000.0;
    let x = |ms: f64| 10.0 + ms / total * (w - 20.0);
    let base = h - 26.0;
    let col = p.accent;
    sketch::set(cr, col, 0.16);
    sketch::rounded(cr, x(0.0), base - 28.0, x(delay) - x(0.0), 28.0, 6.0);
    let _ = cr.fill();
    sketch::set(cr, p.dim, 0.5);
    cr.set_line_width(1.0);
    cr.move_to(x(0.0), base + 0.5);
    cr.line_to(x(total), base + 0.5);
    let _ = cr.stroke();
    sketch::set(cr, col, 1.0);
    cr.set_line_width(3.0);
    cr.move_to(x(0.0), base - 36.0);
    cr.line_to(x(0.0), base);
    let _ = cr.stroke();
    cr.set_line_width(2.0);
    let step = 1000.0 / rate.max(1.0);
    let mut t = delay;
    while t <= total {
        cr.move_to(x(t), base - 16.0);
        cr.line_to(x(t), base);
        t += step;
    }
    let _ = cr.stroke();
    cr.select_font_face("sans-serif", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
    cr.set_font_size(11.0);
    let wait = format!("wait {} ms", delay.round() as i64);
    if let Ok(ext) = cr.text_extents(&wait)
        && ext.width() + 8.0 < x(delay) - x(0.0)
    {
        sketch::set(cr, p.text, 0.85);
        cr.move_to((x(0.0) + x(delay)) / 2.0 - ext.width() / 2.0, base - 10.0);
        let _ = cr.show_text(&wait);
    }
    sketch::set(cr, p.dim, 1.0);
    for ms in [0.0, 250.0, 500.0, 750.0, 1000.0] {
        let text = match ms as i64 {
            0 => "press".to_string(),
            1000 => "1 s".to_string(),
            m => format!("{m} ms"),
        };
        let Ok(ext) = cr.text_extents(&text) else { continue };
        let tx = match ms as i64 {
            0 => x(ms),
            1000 => x(ms) - ext.width(),
            _ => x(ms) - ext.width() / 2.0,
        };
        cr.move_to(tx, h - 6.0);
        let _ = cr.show_text(&text);
    }
}

// ---------- Small parts the Input and Desktop pages built here share ----------

/// A row (from the `hypr_*` family, say) with an icon tile before it.
pub fn tiled(row: gtk::Box, icon: &str) -> gtk::Box {
    row.prepend(&widgets::tile(icon));
    row.add_css_class("with-tile");
    row
}

/// `w` in a holder that can shrink below it, so a chip or a line in the preview never
/// holds the window wider than the screen it's on (it's clipped instead, and only in a
/// window too narrow to show it).
pub fn fit(w: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let s = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(true)
        .propagate_natural_height(true)
        .child(w)
        .build();
    s.add_css_class("in-fit");
    s.set_valign(gtk::Align::Center);
    s
}

/// Let a preview heading's line wrap early, so controls at its right keep their width.
pub fn tame_head(head: &gtk::Box, chars: i32) {
    if let Some(text) = head.first_child() {
        let mut c = text.first_child();
        while let Some(w) = c {
            if let Some(l) = w.downcast_ref::<gtk::Label>() {
                l.set_wrap(true);
                l.set_max_width_chars(chars);
            }
            c = w.next_sibling();
        }
    }
}

/// The first widget of type `T` inside `w`.
pub fn find<T: IsA<gtk::Widget>>(w: &gtk::Widget) -> Option<T> {
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(t) = child.downcast_ref::<T>() {
            return Some(t.clone());
        }
        if let Some(t) = find::<T>(&child) {
            return Some(t);
        }
        c = child.next_sibling();
    }
    None
}

/// Make the slider in a row `width` wide, for short cards.
pub fn slim_slider(row: gtk::Box, width: i32) -> gtk::Box {
    if let Some(s) = find::<gtk::Scale>(row.upcast_ref()) {
        s.set_width_request(width);
    }
    row
}

/// Lines of small items that wrap to the width they get (legends, chips).
pub fn flow() -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_homogeneous(false);
    f.set_min_children_per_line(1);
    f.set_max_children_per_line(12);
    f.set_column_spacing(14);
    f.set_row_spacing(6);
    f.add_css_class("in-flow");
    f
}

/// Add to a [`flow`] without its children taking keyboard focus.
pub fn flow_add(f: &gtk::FlowBox, w: &impl IsA<gtk::Widget>) {
    w.set_halign(gtk::Align::Start);
    f.append(w);
    if let Some(c) = f.last_child() {
        c.set_focusable(false);
    }
}

/// Keep a split page's controls near their 430 px: wrapping labels there ask for no
/// more than `chars` characters a line, so the preview gets the rest of the width.
pub fn hold_controls(page: &Page, chars: i32) {
    fn walk(w: &gtk::Widget, chars: i32) {
        if w.has_css_class("preview-pane") {
            return;
        }
        // Rows expand, which would make the column share the spare width with the preview.
        if w.has_css_class("split-controls") {
            w.set_hexpand(false);
        }
        if let Some(l) = w.downcast_ref::<gtk::Label>()
            && l.wraps()
            && (l.max_width_chars() < 0 || l.max_width_chars() > chars)
        {
            l.set_max_width_chars(chars);
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            walk(&child, chars);
            c = child.next_sibling();
        }
    }
    walk(page.body.upcast_ref(), chars);
}

/// Rows that live a level down: made with the page (so search and the list of changed
/// settings know them), shown in a sub-page when it opens and taken back when it closes.
/// Returns the box to fill with groups' rows and the function that opens the sub-page.
pub fn more_page(title: &str, width: i32) -> (gtk::Box, Rc<dyn Fn()>) {
    let content = widgets::vbox(0);
    content.add_css_class("in-more");
    let (c, title) = (content.clone(), title.to_string());
    let open: Rc<dyn Fn()> = Rc::new(move || {
        if c.parent().is_some() {
            return;
        }
        let c2 = c.clone();
        let win = widgets::subpage(&title, width, move |p| {
            let g = p.plain_group("");
            g.add(&c2);
        });
        let c3 = c.clone();
        win.connect_close_request(move |_| {
            if let Some(parent) = c3.parent().and_downcast::<gtk::Box>() {
                parent.remove(&c3);
            }
            glib::Propagation::Proceed
        });
    });
    (content, open)
}

/// A card of rows inside a [`more_page`], with an optional title over it.
pub fn more_card(content: &gtk::Box, title: &str) -> gtk::Box {
    if !title.is_empty() {
        let t = widgets::label(&title.to_uppercase(), "group-title");
        content.append(&t);
    }
    let card = widgets::vbox(0);
    card.add_css_class("settings-card");
    card.set_overflow(gtk::Overflow::Hidden);
    content.append(&card);
    card
}

/// Empty a box.
pub fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// A small coloured square for a legend, as a CSS class says.
pub fn legend_item(text: &str, dot_class: &str) -> gtk::Box {
    let item = widgets::hbox(6);
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("in-dot");
    dot.add_css_class(dot_class);
    dot.set_valign(gtk::Align::Center);
    item.append(&dot);
    let l = widgets::label(text, "in-legend-text");
    l.set_wrap(true);
    l.set_max_width_chars(40);
    item.append(&l);
    item
}

pub fn build(page: &Page) {
    let list = Rc::new(RefCell::new(layouts()));
    let shown = Rc::new(Cell::new(0usize));
    let options = Rc::new(RefCell::new(option_str("input.kb_options")));

    // ----- The picture -----
    let pv = page.split();
    let chooser_slot = widgets::hbox(0);
    let head = pv.head("", "", &[fit(&chooser_slot).upcast_ref()]);
    let (title, line) = {
        let text = head.first_child().and_downcast::<gtk::Box>().expect("preview head text");
        let title = text.first_child().and_downcast::<gtk::Label>().expect("preview title");
        let line = widgets::label("", "preview-line");
        line.set_wrap(true);
        text.append(&line);
        (title, line)
    };
    if list.borrow().len() > 1 {
        tame_head(&head, 26);
    }
    let labels: Rc<RefCell<HashMap<String, String>>> = Rc::default();
    let board_card = pv.card();
    board_card.add_css_class("kb-card");
    let board = {
        let (labels, options, list) = (labels.clone(), options.clone(), list.clone());
        widgets::drawing(230, move |cr, p, _, w, h| {
            let m = marks(&options.borrow(), list.borrow().len() > 1);
            let labels = labels.borrow();
            draw_board(cr, p, w, h, |key| {
                let mut look = Look { label: labels.get(key.name).cloned().unwrap_or_default(), ..Default::default() };
                if let Some((action, hot)) = m.keys.get(key.name) {
                    look.action = action.clone();
                    if *hot {
                        look.fill = Some(p.accent);
                        look.ring = Some(p.accent);
                    } else {
                        look.ring = Some(p.accent);
                    }
                }
                look
            })
        })
    };
    board.set_content_height(230);
    board_card.append(&board);
    let legend = flow();
    legend.add_css_class("kb-legend");
    board_card.append(&legend);

    let refresh: Rc<dyn Fn()> = {
        let (list, shown, options, labels, title, line, board, legend) =
            (list.clone(), shown.clone(), options.clone(), labels.clone(), title.clone(), line.clone(), board.clone(), legend.clone());
        Rc::new(move || {
            let l = list.borrow();
            let i = shown.get().min(l.len().saturating_sub(1));
            let (code, variant) = l.get(i).cloned().unwrap_or_default();
            *labels.borrow_mut() = key_labels(&code, &variant);
            title.set_text(&layout_name(&code, &variant));
            line.set_text(&switch_line(&l, i, &options.borrow()));
            legend.remove_all();
            for (text, hot) in marks(&options.borrow(), l.len() > 1).legend {
                flow_add(&legend, &legend_item(&text, if hot { "kb-dot-hot" } else { "kb-dot-switch" }));
            }
            legend.set_visible(legend.first_child().is_some());
            board.queue_draw();
        })
    };

    // The chip above the picture picks which layout it draws.
    let rebuild_chooser: Rc<dyn Fn()> = {
        let (list, shown, refresh, slot) = (list.clone(), shown.clone(), refresh.clone(), chooser_slot.clone());
        Rc::new(move || {
            while let Some(c) = slot.first_child() {
                slot.remove(&c);
            }
            let l = list.borrow();
            if shown.get() >= l.len() {
                shown.set(0);
            }
            if l.len() > 1 {
                let opts: Vec<(String, String)> =
                    l.iter().enumerate().map(|(i, (c, v))| (i.to_string(), short_name(&layout_name(c, v)))).collect();
                let (shown, refresh) = (shown.clone(), refresh.clone());
                let seg = widgets::segmented(&opts, &shown.get().to_string(), move |id| {
                    shown.set(id.parse().unwrap_or(0));
                    refresh();
                });
                slot.append(&seg);
            }
        })
    };

    let try_card = pv.card();
    try_card.add_css_class("kb-try");
    let th = widgets::hbox(8);
    th.append(&widgets::label("Try it", "kb-try-title"));
    let hint = widgets::label("Hold a key here to feel the repeat", "kb-try-hint");
    hint.set_wrap(true);
    hint.set_hexpand(true);
    th.append(&hint);
    try_card.append(&th);
    let field = gtk::Entry::new();
    field.set_placeholder_text(Some("Type here"));
    field.set_width_chars(4);
    field.set_hexpand(true);
    field.add_css_class("kb-try-field");
    try_card.append(&field);
    let strip = widgets::drawing(74, |cr, p, _, w, h| {
        draw_repeat(cr, p, w, h, option_f64("input.repeat_delay", 600.0), option_f64("input.repeat_rate", 25.0))
    });
    try_card.append(&strip);
    pv.foot("input-keyboard-symbolic", "Changes apply as soon as you make them.");

    {
        let (options, refresh, strip) = (options.clone(), refresh.clone(), strip.clone());
        store::subscribe(&strip.clone(), "input.", move |key, value| {
            if key == "input.kb_options" {
                *options.borrow_mut() = value.and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| option_str(key));
                refresh();
            }
            if key.starts_with("input.repeat_") {
                strip.queue_draw();
            }
        });
    }

    // ----- Layouts -----
    let g = page.group("Layouts");
    if list.borrow().len() > 1 {
        g.note("The first is the one you start with. Drag to reorder.");
    }
    let rows = widgets::vbox(0);
    g.add(&rows);
    let fill_rows: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::default();
    let fill: Rc<dyn Fn()> = {
        let (list, rows, fill_rows, rebuild_chooser, refresh) = (list.clone(), rows.clone(), fill_rows.clone(), rebuild_chooser.clone(), refresh.clone());
        Rc::new(move || {
            widgets::forget_rows(&rows);
            while let Some(c) = rows.first_child() {
                rows.remove(&c);
            }
            let l = list.borrow().clone();
            for (i, (code, variant)) in l.iter().enumerate() {
                let r = layout_row(i, code, variant, l.len());
                let remove = widgets::icon_button("window-close-symbolic", "Remove");
                remove.set_visible(l.len() > 1);
                {
                    let (list, fill_rows) = (list.clone(), fill_rows.clone());
                    remove.connect_clicked(move |_| {
                        list.borrow_mut().remove(i);
                        save_layouts(&list.borrow());
                        if let Some(f) = fill_rows.borrow().clone() {
                            glib::idle_add_local_once(move || f());
                        }
                    });
                }
                r.append(&remove);
                // Drag a row onto another to move it there.
                let src = gtk::DragSource::new();
                src.set_actions(gdk::DragAction::MOVE);
                src.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&(i as u32).to_value())));
                r.add_controller(src);
                let target = gtk::DropTarget::new(u32::static_type(), gdk::DragAction::MOVE);
                {
                    let (list, fill_rows) = (list.clone(), fill_rows.clone());
                    target.connect_drop(move |_, value, _, _| {
                        let Ok(from) = value.get::<u32>() else { return false };
                        let from = from as usize;
                        let mut l = list.borrow_mut();
                        if from == i || from >= l.len() {
                            return false;
                        }
                        let item = l.remove(from);
                        l.insert(i, item);
                        save_layouts(&l);
                        if let Some(f) = fill_rows.borrow().clone() {
                            glib::idle_add_local_once(move || f());
                        }
                        true
                    });
                }
                r.add_controller(target);
                rows.append(&r);
            }
            let add = add_row();
            {
                let (list, fill_rows) = (list.clone(), fill_rows.clone());
                let click = gtk::GestureClick::new();
                click.connect_released(move |_, _, _, _| {
                    let (list, fill_rows) = (list.clone(), fill_rows.clone());
                    add_layout_dialog(move |code, variant| {
                        let mut l = list.borrow_mut();
                        if !l.iter().any(|(c, v)| *c == code && *v == variant) {
                            l.push((code, variant));
                            save_layouts(&l);
                        }
                        drop(l);
                        if let Some(f) = fill_rows.borrow().clone() {
                            f();
                        }
                    });
                });
                add.add_controller(click);
            }
            rows.append(&add);
            widgets::mark_first_rows(rows.upcast_ref());
            rebuild_chooser();
            refresh();
        })
    };
    *fill_rows.borrow_mut() = Some(fill.clone());
    fill();
    widgets::keywords("layout language us de variant xkb add remove order");

    // ----- Keys -----
    let g = page.group("");
    let (switch, rest) = split_options(&options.borrow());
    let mut switch_opts = widgets::opts(SWITCH);
    if !SWITCH.iter().any(|(id, _)| *id == switch) {
        switch_opts.push((switch.clone(), format!("Custom: {switch}")));
    }
    let mut caps_opts = widgets::opts(CAPS);
    if crate::platform::PLATFORM == crate::platform::Platform::Omarchy || rest == OMARCHY_CAPS.0 {
        caps_opts.insert(0, (OMARCHY_CAPS.0.into(), OMARCHY_CAPS.1.into()));
    }
    if !caps_opts.iter().any(|(id, _)| *id == rest) {
        caps_opts.push((rest.clone(), format!("Custom: {rest}")));
    }
    let set_part = {
        let options = options.clone();
        move |switch_part: Option<String>, rest_part: Option<String>| {
            let (s, r) = split_options(&options.borrow());
            let all = join_options(&switch_part.unwrap_or(s), &rest_part.unwrap_or(r));
            *options.borrow_mut() = all.clone();
            store::set_option("input.kb_options", json!(all));
        }
    };
    {
        let set_part = set_part.clone();
        let (r, _) = widgets::choice_row("Switch layouts with", "", switch_opts, &switch, move |id| set_part(Some(id), None));
        g.add(&tiled(r, "object-flip-horizontal-symbolic"));
    }
    widgets::keywords("switch layout toggle alt shift group");
    {
        let desc: Rc<RefCell<Option<gtk::Label>>> = Rc::default();
        let d = desc.clone();
        let (r, _) = widgets::choice_row("Caps Lock key", "-", caps_opts, &rest, move |id| {
            if let Some(l) = d.borrow().as_ref() {
                l.set_text(caps_desc(&id));
                l.set_visible(!caps_desc(&id).is_empty());
            }
            set_part(None, Some(id))
        });
        let l = find::<gtk::Box>(r.upcast_ref()).and_then(|text| text.last_child()).and_downcast::<gtk::Label>();
        if let Some(l) = &l {
            l.set_text(caps_desc(&rest));
            l.set_visible(!caps_desc(&rest).is_empty());
        }
        *desc.borrow_mut() = l;
        g.add(&tiled(r, "input-keyboard-symbolic"));
    }
    widgets::keywords("caps lock compose escape control xkb options");
    g.add(&tiled(hypr_switch("input.numlock_by_default", "Num Lock on at sign-in", ""), "system-lock-screen-symbolic"));
    widgets::keywords("numlock num lock login");

    // ----- Typing -----
    let g = page.group("Typing");
    g.add(&tiled(
        slim_slider(hypr_slider("input.repeat_delay", "Repeat after", "", (150.0, 800.0, 10.0), 0, " ms", true), 120),
        "preferences-system-time-symbolic",
    ));
    g.add(&tiled(
        slim_slider(hypr_slider("input.repeat_rate", "Repeat speed", "", (10.0, 80.0, 1.0), 0, " a second", true), 110),
        "media-playlist-repeat-symbolic",
    ));
    widgets::keywords("key repeat rate delay hold");
    if kbdidle::available() {
        let cur = kbdidle::load().timeout_secs;
        let mut options: Vec<(String, String)> = kbdidle::CHOICES.iter().map(|(s, l)| (s.to_string(), l.to_string())).collect();
        if !kbdidle::CHOICES.iter().any(|(s, _)| *s == cur) {
            options.push((cur.to_string(), format!("{cur} seconds")));
        }
        let (r, _) = widgets::choice_row("Keyboard light off after", "Without typing or touching", options, &cur.to_string(), |v| {
            let secs = v.parse::<u32>().unwrap_or(0);
            cmd::background(
                move || kbdidle::apply(&kbdidle::Config { timeout_secs: secs }).map_err(|e| format!("{e:#}")),
                move |r| match r {
                    Ok(()) if secs == 0 => window::toast("Keyboard light stays on"),
                    Ok(()) => window::toast("Keyboard light timeout set"),
                    Err(e) => window::toast(&format!("Couldn't set the timeout: {e}")),
                },
            );
        });
        g.add(&tiled(r, "display-brightness-symbolic"));
        widgets::keywords("keyboard backlight light timeout idle off");
    }
    hold_controls(page, 24);
}

/// "English (US)" stays; "German (no dead keys)" becomes "German" for the chip.
fn short_name(name: &str) -> String {
    if name.ends_with("(US)") || name.ends_with("(UK)") {
        return name.to_string();
    }
    name.split(" (").next().unwrap_or(name).to_string()
}

/// A layout in the list: a grip, its code, its name.
fn layout_row(i: usize, code: &str, variant: &str, count: usize) -> gtk::Box {
    let name = layout_name(code, variant);
    let desc = match (i, variant.is_empty()) {
        (0, true) if count > 1 => format!("First · {code}"),
        (0, false) if count > 1 => format!("First · {code}, {variant}"),
        (_, true) => code.to_string(),
        (_, false) => format!("{code}, {variant}"),
    };
    let r = widgets::row(&name, &desc, None);
    r.add_css_class("kb-layout");
    let flag = widgets::label(&code.chars().take(3).collect::<String>().to_uppercase(), "kb-flag");
    flag.set_valign(gtk::Align::Center);
    r.prepend(&flag);
    if count > 1 {
        let grip = gtk::Image::from_icon_name("list-drag-handle-symbolic");
        grip.add_css_class("kb-grip");
        grip.set_valign(gtk::Align::Center);
        r.prepend(&grip);
        r.set_cursor_from_name(Some("grab"));
    }
    r
}

fn add_row() -> gtk::Box {
    let r = widgets::hbox(12);
    r.add_css_class("settings-option");
    r.add_css_class("card-row");
    r.add_css_class("kb-add");
    let plus = gtk::Image::from_icon_name("list-add-symbolic");
    r.append(&plus);
    r.append(&widgets::label("Add a layout…", "kb-add-text"));
    r.set_cursor_from_name(Some("pointer"));
    r
}

/// Search every layout xkb knows by name, and add one.
fn add_layout_dialog(on_pick: impl Fn(String, String) + 'static) {
    let (dialog, card) = widgets::dialog("Add a layout", 460);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search by language or country"));
    card.append(&search);
    let listbox = gtk::ListBox::new();
    listbox.add_css_class("kb-pick-list");
    listbox.set_selection_mode(gtk::SelectionMode::Single);
    let all = known();
    if all.is_empty() {
        card.append(&widgets::label("This system has no list of keyboard layouts.", "dim"));
    }
    let mut haystack = Vec::new();
    for (code, variant, name) in all.iter() {
        let row = gtk::ListBoxRow::new();
        let line = widgets::hbox(10);
        let n = widgets::label(name, "");
        n.set_hexpand(true);
        n.set_ellipsize(gtk::pango::EllipsizeMode::End);
        line.append(&n);
        let c = if variant.is_empty() { code.clone() } else { format!("{code}, {variant}") };
        line.append(&widgets::label(&c, "dim"));
        row.set_child(Some(&line));
        haystack.push(format!("{name} {c}").to_lowercase());
        listbox.append(&row);
    }
    {
        let search = search.clone();
        listbox.set_filter_func(move |row| {
            let q = search.text().to_lowercase();
            q.is_empty() || haystack.get(row.index().max(0) as usize).is_some_and(|t| t.contains(&q))
        });
    }
    {
        let lb = listbox.clone();
        search.connect_search_changed(move |_| lb.invalidate_filter());
    }
    let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).min_content_height(320).child(&listbox).build();
    card.append(&scroll);
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let add = gtk::Button::with_label("Add");
    add.add_css_class("suggested-action");
    add.set_sensitive(false);
    buttons.append(&cancel);
    buttons.append(&add);
    card.append(&buttons);
    {
        let d = dialog.clone();
        cancel.connect_clicked(move |_| d.close());
    }
    {
        let add = add.clone();
        listbox.connect_row_selected(move |_, row| add.set_sensitive(row.is_some()));
    }
    let on_pick = Rc::new(on_pick);
    let pick = {
        let (d, lb) = (dialog.clone(), listbox.clone());
        move || {
            if let Some((code, variant, _)) = lb.selected_row().and_then(|row| all.get(row.index().max(0) as usize)) {
                on_pick(code.clone(), variant.clone());
                d.close();
            }
        }
    };
    {
        let pick = pick.clone();
        add.connect_clicked(move |_| pick());
    }
    listbox.connect_row_activated(move |_, _| pick());
    dialog.present();
    search.grab_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_options() {
        assert_eq!(
            split_options("compose:caps,shift:both_capslock_cancel,grp:alts_toggle"),
            ("grp:alts_toggle".to_string(), "compose:caps,shift:both_capslock_cancel".to_string())
        );
        assert_eq!(join_options("grp:alts_toggle", "caps:escape"), "caps:escape,grp:alts_toggle");
        assert_eq!(join_options("", ""), "");
    }

    #[test]
    fn keysyms_to_characters() {
        assert_eq!(sym_char("q").as_deref(), Some("q"));
        assert_eq!(sym_char("U00E9").as_deref(), Some("é"));
        assert_eq!(sym_char("dead_acute").as_deref(), Some("´"));
    }

    #[test]
    fn reads_layouts_from_xkb() {
        if xkb_root().is_none() {
            return;
        }
        let de = key_labels("de", "");
        assert_eq!(de.get("AD06").map(String::as_str), Some("Z"));
        assert_eq!(de.get("AC10").map(String::as_str), Some("Ö"));
        assert_eq!(de.get("AE11").map(String::as_str), Some("ß"));
        let us = key_labels("us", "");
        assert_eq!(us.get("AD01").map(String::as_str), Some("Q"));
        assert_eq!(us.get("AE01").map(String::as_str), Some("1"));
        assert!(layout_name("de", "nodeadkeys").starts_with("German"));
    }

    #[test]
    fn rows_fill_the_width() {
        for (_, r) in key_rects(600.0, 200.0) {
            assert!(r.x + r.w <= 600.5);
        }
    }
}
