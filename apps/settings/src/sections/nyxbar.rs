//! Nyx Bar: its size, the status icons as one pill or separate, the calendar's layout
//! and the workspaces' look. Read through every layer (NyxOS's defaults, then the
//! user's file); written into `~/.config/nyx/bar.toml` with `toml_edit`, keeping the
//! layout and comments the bar wrote there. The bar follows the file as it changes.

use crate::widgets::{self, Page};
use crate::window;
use std::path::PathBuf;
use toml_edit::{Array, DocumentMut, Item, Value};

fn path() -> PathBuf {
    gtk::glib::user_config_dir().join("nyx/bar.toml")
}

fn read_user() -> DocumentMut {
    std::fs::read_to_string(path()).ok().and_then(|t| t.parse().ok()).unwrap_or_default()
}

fn write_user(doc: &DocumentMut) {
    let p = path();
    let r = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(p.parent().unwrap_or(&p))?;
        let tmp = p.with_extension("toml.new");
        std::fs::write(&tmp, doc.to_string())?;
        std::fs::rename(&tmp, &p)
    })();
    if let Err(e) = r {
        window::toast(&format!("Couldn't save: {e}"));
    }
}

/// Size: one of the presets; an exact `height` would win over it, so it goes (its
/// comment stays, above the size).
pub fn set_size(doc: &mut DocumentMut, size: &str) {
    let comment = doc.as_table().key("height").and_then(|k| k.leaf_decor().prefix()).cloned();
    doc.remove("height");
    let had = doc.contains_key("size");
    doc["size"] = toml_edit::value(size);
    if !had
        && let (Some(c), Some(mut k)) = (comment, doc.as_table_mut().key_mut("size"))
    {
        k.leaf_decor_mut().set_prefix(c);
    }
}

/// One plugin's setting, in `[plugins."<id>"]`.
pub fn set_plugin(doc: &mut DocumentMut, plugin: &str, key: &str, value: &str) {
    if !doc.contains_table("plugins") {
        let mut t = toml_edit::Table::new();
        t.set_implicit(true);
        doc["plugins"] = Item::Table(t);
    }
    let plugins = doc["plugins"].as_table_mut().expect("plugins is a table");
    if !plugins.contains_table(plugin) {
        plugins.insert(plugin, Item::Table(toml_edit::Table::new()));
    }
    plugins[plugin][key] = toml_edit::value(value);
}

/// Where the status items sit in the right section: grouped (`["nyx.system"]`) or not,
/// or `None` when the layout doesn't place them there.
pub fn status_grouped(doc: &DocumentMut) -> Option<bool> {
    let right = doc.get("right")?.as_array()?;
    right.iter().find_map(|v| match v {
        Value::String(s) if s.value() == "nyx.system" => Some(false),
        Value::Array(a) if a.iter().any(|x| x.as_str() == Some("nyx.system")) => Some(true),
        _ => None,
    })
}

/// The status items as one pill or as separate items, in place in the right section.
pub fn set_status_grouped(doc: &mut DocumentMut, grouped: bool) -> bool {
    let Some(right) = doc.get_mut("right").and_then(|r| r.as_array_mut()) else { return false };
    let Some(i) = right.iter().position(|v| match v {
        Value::String(s) => s.value() == "nyx.system",
        Value::Array(a) => a.len() == 1 && a.iter().any(|x| x.as_str() == Some("nyx.system")),
        _ => false,
    }) else {
        return false;
    };
    let decor = right.get(i).map(|v| v.decor().clone()).unwrap_or_default();
    let mut new: Value = if grouped {
        let mut a = Array::new();
        a.push("nyx.system");
        Value::Array(a)
    } else {
        Value::from("nyx.system")
    };
    *new.decor_mut() = decor;
    right.replace(i, new);
    true
}

pub fn build(page: &Page) {
    let merged = nyx_config::Layers::standard().merged("bar").unwrap_or_default();
    let plugin = |id: &str, key: &str, d: &str| {
        merged.get("plugins").and_then(|p| p.get(id)).and_then(|p| p.get(key)).and_then(|v| v.as_str()).unwrap_or(d).to_string()
    };
    let opts = |o: &[(&str, &str)]| o.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();

    let g = page.group("Bar");
    let size = if merged.contains_key("height") { "custom".to_string() } else { merged.get("size").and_then(|v| v.as_str()).unwrap_or("default").to_string() };
    let mut sizes = opts(&[("compact", "Compact (28 px)"), ("default", "Default (32 px)"), ("cozy", "Cozy (36 px)"), ("spacious", "Spacious (40 px)")]);
    if size == "custom" {
        sizes.push(("custom".into(), "Custom height".into()));
    }
    let (r, _) = widgets::choice_row("Size", "The bar's height, and the text and icons in it.", sizes, &size, |v| {
        if v == "custom" {
            return;
        }
        let mut doc = read_user();
        set_size(&mut doc, &v);
        write_user(&doc);
    });
    g.add(&r);

    let doc = read_user();
    match status_grouped(&doc) {
        Some(grouped) => {
            let (r, _) = widgets::switch_row(
                "Status icons in one pill",
                "Network, Bluetooth, sound and battery together, opening Control Center. Off: each on its own.",
                grouped,
                |on| {
                    let mut doc = read_user();
                    if set_status_grouped(&mut doc, on) {
                        write_user(&doc);
                    }
                },
            );
            g.add(&r);
        }
        None => g.note("The status icons aren't in the bar's right section, so they can't be grouped here."),
    }

    let g = page.group("Calendar and workspaces");
    let (r, _) = widgets::choice_row(
        "Calendar",
        "What a click on the clock opens: the month with your events and reminders beside it, or the month alone.",
        opts(&[("wide", "Month and agenda"), ("compact", "Month only")]),
        &plugin("nyx.clock", "layout", "wide"),
        |v| {
            let mut doc = read_user();
            set_plugin(&mut doc, "nyx.clock", "layout", &v);
            write_user(&doc);
        },
    );
    g.add(&r);
    let (r, _) = widgets::choice_row(
        "Workspaces",
        "How the workspaces look: pills (the current one holds its windows' icons), numbers, app icons, dots or an underline.",
        opts(&[("pills", "Pills"), ("numbers", "Numbers"), ("icons", "App icons"), ("dots", "Dots"), ("underline", "Underline")]),
        &plugin("nyx.workspaces", "look", "pills"),
        |v| {
            let mut doc = read_user();
            set_plugin(&mut doc, "nyx.workspaces", "look", &v);
            write_user(&doc);
        },
    );
    g.add(&r);
    page.group("Items").note("Which items show and their order: <tt>nyx plugin list</tt>, <tt>nyx plugin configure</tt>.");
    widgets::keywords("nyx bar top panel height size compact cozy spacious status pill calendar clock workspaces look");
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUT: &str = r#"# Nyx Bar layout.
left = [["nyx.launcher", "nyx.workspaces"], "nyx.window"]
right = [
    "nyx.tray",
    ["nyx.system"],
    ["nyx.panels/notifications"],
]

# Size: compact (28 px), default (32), cozy (36) or spacious (40).
height = 30
"#;

    #[test]
    fn edits_keep_the_layout_and_comments() {
        let mut doc: DocumentMut = LAYOUT.parse().unwrap();
        assert_eq!(status_grouped(&doc), Some(true));
        assert!(set_status_grouped(&mut doc, false));
        assert_eq!(status_grouped(&doc), Some(false));
        set_size(&mut doc, "cozy");
        set_plugin(&mut doc, "nyx.clock", "layout", "compact");
        let text = doc.to_string();
        assert!(text.contains("# Nyx Bar layout."));
        assert!(text.contains("# Size: compact"));
        assert!(text.contains("\n    \"nyx.system\",\n"), "{text}");
        assert!(text.contains("size = \"cozy\""));
        assert!(!text.contains("height"));
        assert!(text.contains("[plugins.\"nyx.clock\"]\nlayout = \"compact\""), "{text}");
        let mut doc: DocumentMut = text.parse().unwrap();
        assert!(set_status_grouped(&mut doc, true));
        assert_eq!(status_grouped(&doc), Some(true));
    }
}
