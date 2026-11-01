//! Online account logos drawn in the theme's colours: a tile tinted with the
//! provider's swatch and a one-colour mark on it. Repainted on theme changes.

use super::online_presets::Preset;
use crate::theme::{self, Rgb};
use gtk::prelude::*;
use std::hash::{Hash, Hasher};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Logo {
    Google,
    Microsoft,
    Fastmail,
    Nextcloud,
    Zoho,
    Mail,
    WebDav,
    Generic,
}

impl From<Preset> for Logo {
    fn from(p: Preset) -> Self {
        match p {
            Preset::Fastmail => Self::Fastmail,
            Preset::Zoho => Self::Zoho,
        }
    }
}

impl Logo {
    fn id(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Microsoft => "microsoft",
            Self::Fastmail => "fastmail",
            Self::Nextcloud => "nextcloud",
            Self::Zoho => "zoho",
            Self::Mail => "mail",
            Self::WebDav => "webdav",
            Self::Generic => "generic",
        }
    }

    /// The theme swatch that stands in for the provider's brand colour.
    fn swatch(self) -> &'static str {
        match self {
            Self::Google => "red",
            Self::Microsoft => "blue",
            Self::Fastmail => "coral",
            Self::Nextcloud => "teal",
            Self::Zoho => "amber",
            Self::Mail => "green",
            Self::WebDav => "violet",
            Self::Generic => "graphite",
        }
    }

    /// The mark, in a 128px box; `{c}` is the swatch and `{t}` the tile.
    fn mark(self) -> &'static str {
        match self {
            Self::Google => {
                r#"<path d="M87 45A30 30 0 1 0 94 64H68" fill="none" stroke="{c}" stroke-width="13" stroke-linecap="round" stroke-linejoin="round"/>"#
            }
            Self::Microsoft => {
                r#"<g fill="{c}"><rect x="34" y="34" width="28" height="28" rx="3"/><rect x="66" y="34" width="28" height="28" rx="3"/><rect x="34" y="66" width="28" height="28" rx="3"/><rect x="66" y="66" width="28" height="28" rx="3"/></g>"#
            }
            Self::Fastmail => {
                r#"<rect x="28" y="38" width="72" height="52" rx="9" fill="{c}"/><path d="M34 46l30 22 30-22" fill="none" stroke="{t}" stroke-width="7" stroke-linecap="round" stroke-linejoin="round"/>"#
            }
            Self::Nextcloud => {
                r#"<g fill="none" stroke="{c}" stroke-width="9"><circle cx="64" cy="64" r="17"/><circle cx="33" cy="64" r="10"/><circle cx="95" cy="64" r="10"/></g>"#
            }
            Self::Zoho => {
                r#"<g fill="none" stroke="{c}" stroke-width="7"><rect x="35" y="35" width="25" height="25" rx="7"/><rect x="68" y="35" width="25" height="25" rx="7" transform="rotate(8 80 47)"/><rect x="35" y="68" width="25" height="25" rx="7" transform="rotate(-8 47 80)"/><rect x="68" y="68" width="25" height="25" rx="7"/></g>"#
            }
            Self::Mail => {
                r#"<g fill="none" stroke="{c}" stroke-width="7" stroke-linecap="round" stroke-linejoin="round"><rect x="26" y="36" width="64" height="46" rx="8"/><path d="M30 42l28 20 28-20"/></g><circle cx="90" cy="86" r="18" fill="{c}"/><g fill="none" stroke="{t}" stroke-width="5" stroke-linecap="round" stroke-linejoin="round"><path d="M84 94V78l-5 5M84 78l5 5"/><path d="M96 78v16l-5-5M96 94l5-5"/></g>"#
            }
            Self::WebDav => {
                r#"<g fill="{c}"><rect x="32" y="28" width="64" height="23" rx="6"/><rect x="32" y="56" width="64" height="23" rx="6"/></g><g fill="{t}"><circle cx="44" cy="39.5" r="4"/><circle cx="44" cy="67.5" r="4"/></g><path d="M64 80v14M44 98h40" fill="none" stroke="{c}" stroke-width="7" stroke-linecap="round"/>"#
            }
            Self::Generic => r#"<g fill="{c}"><circle cx="64" cy="50" r="15"/><path d="M36 98a28 24 0 0 1 56 0z"/></g>"#,
        }
    }
}

/// The logo for a native provider type.
pub fn logo_for_provider(provider_type: &str) -> Logo {
    match provider_type {
        "google" => Logo::Google,
        "fastmail" => Logo::Fastmail,
        "zoho" => Logo::Zoho,
        "microsoft_personal" => Logo::Microsoft,
        "nextcloud" => Logo::Nextcloud,
        "imap_smtp" => Logo::Mail,
        "webdav" => Logo::WebDav,
        _ => Logo::Generic,
    }
}

/// nyx-theme's swatches for themes that define none of their own (dark, then light).
const DEFAULT_SWATCHES: &[&str] = &[
    "#5b9cff", "#9d7bff", "#ff6fae", "#ff6b6b", "#ff7a59", "#f2b545", "#46d08a", "#4cc9d6", "#a3a3ab", "#2f6fea", "#6f4de0",
    "#c93a7b", "#d93a3a", "#d4532f", "#b7791f", "#1f9e5c", "#1a8fa0", "#5a5a64",
];

fn to_hsl(c: Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (c.0, c.1, c.2);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-9 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Rgb(r + m, g + m, b + m)
}

/// The logo's colour. A theme with swatches of its own (Nyx, Omarchy) gives it
/// directly; otherwise the default swatch keeps its hue and takes the theme
/// accent's saturation and lightness, so pastel themes get pastel logos.
fn colour(logo: Logo) -> Rgb {
    let palette = theme::palette();
    let swatch = theme::swatches().into_iter().find(|(name, _)| name == logo.swatch()).map(|(_, c)| c);
    let Some(swatch) = swatch else { return Rgb::hex(&palette.accent) };
    if !DEFAULT_SWATCHES.contains(&swatch.to_ascii_lowercase().as_str()) {
        return Rgb::hex(&swatch);
    }
    if logo == Logo::Generic {
        return Rgb::hex(&palette.dim_text);
    }
    let (h, _, _) = to_hsl(Rgb::hex(&swatch));
    let (_, s, l) = to_hsl(Rgb::hex(&palette.accent));
    if palette.light {
        from_hsl(h, s.clamp(0.4, 0.72), l.clamp(0.36, 0.48))
    } else {
        from_hsl(h, s.clamp(0.45, 0.9), l.clamp(0.4, 0.78))
    }
}

fn svg(logo: Logo) -> String {
    let card = Rgb::hex(&theme::look().card);
    let c = colour(logo);
    let tile = card.mix(c, 0.18).to_hex();
    let edge = card.mix(c, 0.35).to_hex();
    let mark = logo.mark().replace("{c}", &c.to_hex()).replace("{t}", &tile);
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"128\" height=\"128\" viewBox=\"0 0 128 128\">\
         <rect x=\"4\" y=\"4\" width=\"120\" height=\"120\" rx=\"30\" fill=\"{tile}\" stroke=\"{edge}\" stroke-width=\"3\"/>{mark}</svg>"
    )
}

/// The current SVG for `logo` on disk, named by its colours so GTK never shows
/// a stale copy. Older versions of the same logo are removed.
fn file_for(logo: Logo) -> Option<std::path::PathBuf> {
    let svg = svg(logo);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    svg.hash(&mut h);
    let dir = crate::paths::cache_dir().join("account-icons");
    std::fs::create_dir_all(&dir).ok()?;
    let name = format!("{}-{:016x}.svg", logo.id(), h.finish());
    let path = dir.join(&name);
    if !path.exists() {
        std::fs::write(&path, &svg).ok()?;
        let prefix = format!("{}-", logo.id());
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let file = entry.file_name().to_string_lossy().to_string();
            if file.starts_with(&prefix) && file != name {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Some(path)
}

fn paint(image: &gtk::Image, logo: Logo) {
    match file_for(logo) {
        Some(path) => {
            let paintable = gtk::IconPaintable::for_file(&gtk::gio::File::for_path(path), 128, 1);
            image.set_paintable(Some(&paintable));
        }
        None => image.set_icon_name(Some("avatar-default-symbolic")),
    }
}

/// `logo` at `size` px, repainted whenever the theme changes.
pub fn image(logo: Logo, size: i32) -> gtk::Image {
    let image = gtk::Image::new();
    image.set_pixel_size(size);
    image.set_valign(gtk::Align::Center);
    image.add_css_class("account-icon");
    paint(&image, logo);
    let weak = image.downgrade();
    theme::subscribe(&image, move || {
        if let Some(image) = weak.upgrade() {
            paint(&image, logo);
        }
    });
    image
}
