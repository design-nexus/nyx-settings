use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::gio;
use gtk::prelude::*;

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

pub fn build(page: &Page) {
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
        "Used by <tt>Super Enter</tt> and anything that opens a terminal.",
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
        "Opens config files, including this app's Open config buttons.",
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

    let g = page.group("Files and links");
    g.note("Which app opens each kind of file or link, from the apps you have installed.");
    for c in CATEGORIES {
        if let Some(r) = mime_row(c) {
            widgets::keywords(c.keywords);
            g.add(&r);
        }
    }
}

// ----- Files and links (MIME defaults) -----

struct Category {
    title: &'static str,
    desc: &'static str,
    /// Every type this row sets; the first is read for the current choice.
    /// An app that opens any of them is offered.
    types: &'static [&'static str],
    keywords: &'static str,
    /// Desktop-entry categories whose apps open these types but don't fit the
    /// row (browsers under Email, video editors under Video player).
    skip: &'static [&'static str],
}

const CATEGORIES: &[Category] = &[
    Category {
        title: "File manager",
        desc: "Opens folders from every app.",
        types: &["inode/directory"],
        keywords: "folders files browse",
        skip: &[],
    },
    Category {
        title: "Image viewer",
        desc: "Opens photos and pictures.",
        types: &[
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "image/bmp",
            "image/tiff",
            "image/svg+xml",
            "image/avif",
            "image/heic",
            "image/x-icon",
        ],
        keywords: "photos pictures images jpg png viewer",
        skip: &["WebBrowser", "AudioVideo", "TextEditor"],
    },
    Category {
        title: "Video player",
        desc: "Opens video files.",
        types: &["video/mp4", "video/x-matroska", "video/webm", "video/quicktime", "video/x-msvideo", "video/mpeg", "video/ogg"],
        keywords: "movies videos mp4 mkv player",
        skip: &["AudioVideoEditing", "TextEditor"],
    },
    Category {
        title: "Music player",
        desc: "Opens audio files.",
        types: &["audio/mpeg", "audio/flac", "audio/ogg", "audio/wav", "audio/x-wav", "audio/mp4", "audio/aac", "audio/opus"],
        keywords: "music songs audio mp3 flac player",
        skip: &["AudioVideoEditing", "TextEditor"],
    },
    Category {
        title: "PDFs",
        desc: "Opens PDF documents.",
        types: &["application/pdf"],
        keywords: "pdf reader document viewer",
        skip: &[],
    },
    Category {
        title: "Documents",
        desc: "Word processing files (.docx, .odt, .doc).",
        types: &[
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.oasis.opendocument.text",
            "application/msword",
            "application/rtf",
        ],
        keywords: "office word docx odt writer libreoffice document",
        skip: &["WebBrowser"],
    },
    Category {
        title: "Spreadsheets",
        desc: "Spreadsheet files (.xlsx, .ods, .xls).",
        types: &[
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/vnd.oasis.opendocument.spreadsheet",
            "application/vnd.ms-excel",
        ],
        keywords: "office excel xlsx ods calc sheets spreadsheet",
        skip: &["WebBrowser"],
    },
    Category {
        title: "Presentations",
        desc: "Slide decks (.pptx, .odp, .ppt).",
        types: &[
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "application/vnd.oasis.opendocument.presentation",
            "application/vnd.ms-powerpoint",
        ],
        keywords: "office powerpoint pptx odp impress slides presentation",
        skip: &["WebBrowser"],
    },
    Category {
        title: "Text files",
        desc: "Plain text, Markdown, logs and data files opened from a file manager.",
        types: &[
            "text/plain",
            "text/markdown",
            "text/csv",
            "text/x-log",
            "application/json",
            "application/xml",
            "text/xml",
            "application/x-yaml",
            "text/x-shellscript",
        ],
        keywords: "text txt markdown notes json",
        skip: &["WebBrowser"],
    },
    Category {
        title: "Archives",
        desc: "Opens zip, tar and other compressed files.",
        types: &[
            "application/zip",
            "application/x-tar",
            "application/x-7z-compressed",
            "application/vnd.rar",
            "application/gzip",
            "application/x-compressed-tar",
            "application/zstd",
        ],
        keywords: "zip archive compressed extract unzip rar 7z",
        skip: &["WebBrowser", "TextEditor"],
    },
    Category {
        title: "Email",
        desc: "Opens <tt>mailto:</tt> links.",
        types: &["x-scheme-handler/mailto"],
        keywords: "mail email mailto",
        skip: &["WebBrowser", "TextEditor"],
    },
    Category {
        title: "Calendar",
        desc: "Opens calendar invites and subscriptions.",
        types: &["text/calendar", "x-scheme-handler/webcal"],
        keywords: "calendar ics invite events",
        skip: &["WebBrowser", "TextEditor"],
    },
    Category {
        title: "Torrents",
        desc: "Opens magnet links and .torrent files.",
        types: &["x-scheme-handler/magnet", "application/x-bittorrent"],
        keywords: "torrent magnet bittorrent download",
        skip: &["WebBrowser", "TextEditor"],
    },
];

/// Two apps with the same name get their program added so they can be told
/// apart: "Image Viewer (imv)" and "Image Viewer (loupe)".
fn disambiguate(options: &mut [(String, String)], program: impl Fn(&str) -> String) {
    let names: Vec<String> = options.iter().map(|(_, n)| n.clone()).collect();
    for (id, name) in options.iter_mut() {
        if names.iter().filter(|n| *n == name).count() > 1 {
            let exe = program(id);
            if !exe.is_empty() {
                *name = format!("{name} ({exe})");
            }
        }
    }
}

/// True when the app's desktop entry lists any of the categories.
fn in_categories(app: &gio::AppInfo, skip: &[&str]) -> bool {
    let Some(id) = app.id() else { return false };
    let dirs = std::iter::once(gtk::glib::user_data_dir()).chain(gtk::glib::system_data_dirs());
    for dir in dirs {
        let file = gtk::glib::KeyFile::new();
        if file.load_from_file(dir.join("applications").join(id.as_str()), gtk::glib::KeyFileFlags::NONE).is_err() {
            continue;
        }
        let categories = file.string("Desktop Entry", "Categories").unwrap_or_default();
        return categories.split(';').any(|c| skip.contains(&c));
    }
    false
}

/// (id, name) choices: visible apps, no repeats, and the current default kept
/// even if it doesn't declare the type (so the row always shows the truth).
fn choices(apps: &[(String, String, bool)], current: Option<(String, String)>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (id, name, visible) in apps {
        if *visible && !out.iter().any(|(i, _)| i == id) {
            out.push((id.clone(), name.clone()));
        }
    }
    if let Some((id, name)) = current
        && !out.iter().any(|(i, _)| *i == id)
    {
        out.insert(0, (id, name));
    }
    out
}

fn mime_row(c: &'static Category) -> Option<gtk::Box> {
    let info = |a: &gio::AppInfo| (a.id().map(|s| s.to_string()).unwrap_or_default(), a.name().to_string());
    // Apps that open any of the row's types, those for the main type first.
    let apps: Vec<(String, String, bool)> = c
        .types
        .iter()
        .flat_map(|t| gio::AppInfo::all_for_type(t))
        .filter(|a| a.id().is_some())
        .map(|a| {
            let (id, name) = info(&a);
            (id, name, a.should_show() && !in_categories(&a, c.skip))
        })
        .collect();
    let current = gio::AppInfo::default_for_type(c.types[0], false).map(|a| info(&a));
    let current_id = current.as_ref().map(|(i, _)| i.clone()).unwrap_or_default();
    let mut options = choices(&apps, current);
    disambiguate(&mut options, |id| {
        gio::AppInfo::all()
            .into_iter()
            .find(|a| a.id().is_some_and(|i| i == id))
            .and_then(|a| a.executable().file_name().map(|e| e.to_string_lossy().to_string()))
            .unwrap_or_default()
    });
    if options.is_empty() {
        return None;
    }
    let (r, _) = widgets::choice_row(c.title, c.desc, options.clone(), &current_id, move |id| {
        let Some(app) = gio::AppInfo::all().into_iter().find(|a| a.id().is_some_and(|i| i == id.as_str())) else { return };
        let failed: Vec<String> =
            c.types.iter().filter_map(|t| app.set_as_default_for_type(t).err().map(|e| format!("{t}: {e}"))).collect();
        let name = options.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone()).unwrap_or(id);
        if failed.is_empty() {
            window::toast(&format!("{} set to {name}", c.title));
        } else {
            window::toast(&format!("Couldn't set {}: {}", c.title, failed.join(", ")));
        }
    });
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, visible: bool) -> (String, String, bool) {
        (id.to_string(), id.trim_end_matches(".desktop").to_string(), visible)
    }

    #[test]
    fn choices_hide_helpers_and_keep_the_default() {
        let apps =
            [app("imv.desktop", true), app("imv.desktop", true), app("userapp-x.desktop", false), app("pinta.desktop", true)];
        let c = choices(&apps, Some(("imv.desktop".into(), "imv".into())));
        assert_eq!(c.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>(), ["imv.desktop", "pinta.desktop"]);
        // A default that doesn't list the type is still shown, first.
        let c = choices(&apps, Some(("odd.desktop".into(), "Odd".into())));
        assert_eq!(c[0].0, "odd.desktop");
        assert!(choices(&[], None).is_empty());
    }

    #[test]
    fn same_names_get_their_program() {
        let mut o = vec![
            ("a.desktop".to_string(), "Image Viewer".to_string()),
            ("b.desktop".into(), "Image Viewer".into()),
            ("c.desktop".into(), "Zed".into()),
        ];
        disambiguate(&mut o, |id| id.trim_end_matches(".desktop").to_string());
        assert_eq!(o.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(), ["Image Viewer (a)", "Image Viewer (b)", "Zed"]);
    }
}
