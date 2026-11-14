//! RGB lighting for every device OpenRGB sees: motherboards, memory, graphics
//! cards, coolers, LED strips and peripherals, from any brand.
//!
//! Pages: `all` (one colour everywhere, following the theme, profiles), and
//! one per device.

mod mock;

use anyhow::{Context, Result, bail};
use nyx_ext::openrgb::{self, COLOR_MODE_SPECIFIC, Client, Device, Rgb};
use nyx_ext::page::{self, Page, desc};
use nyx_ext::{Extension, lighting, sys};
use serde_json::{Value, json};

const ALL: &str = "all";
const DEVICE: &str = "dev-";

struct Rgbx;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Whether this Settings draws a preview pane beside the groups.
fn preview_supported() -> bool {
    std::env::var("SETTINGS_KINDS").is_ok_and(|k| k.split(' ').any(|x| x == "preview"))
}

fn devices() -> Vec<Device> {
    if mock::on() { mock::devices() } else { lighting::devices() }
}

fn setup_rows() -> Vec<Value> {
    if mock::on() { Vec::new() } else { lighting::setup_rows() }
}

fn light_rows(d: &Device) -> Vec<Value> {
    if mock::on() { mock::rows(d) } else { lighting::rows(d) }
}

fn with_icon(mut row: Value) -> Value {
    let (icon, text) = match row["key"].as_str().unwrap_or("") {
        "light/effect" => ("starred-symbolic", Some("The device's own effects")),
        "light/brightness" => ("display-brightness-symbolic", None),
        "light/speed" => ("media-seek-forward-symbolic", None),
        "light/theme" => ("applications-graphics-symbolic", Some("Takes the theme's accent as it changes")),
        "light/keep" => ("document-save-symbolic", Some("Stores the effect on the device, so it stays without OpenRGB")),
        "openrgb/install" => ("system-software-install-symbolic", None),
        "openrgb/start" => ("media-playback-start-symbolic", None),
        "all/theme" => ("applications-graphics-symbolic", None),
        "all/off" => ("system-shutdown-symbolic", None),
        "profile/save" => ("document-save-symbolic", None),
        "profile/load" => ("view-list-symbolic", None),
        _ => ("weather-clear-symbolic", None),
    };
    row["icon"] = json!(icon);
    if let Some(t) = text {
        row["desc"] = json!(t);
    }
    row
}

/// The colour a device shows now: its effect's colour, else its LEDs'.
fn current_colour(d: &Device) -> Option<Rgb> {
    match d.mode() {
        Some(m) if m.color_mode == COLOR_MODE_SPECIFIC => m.colors.first().copied(),
        _ => d.colors.first().copied(),
    }
}

/// For the preview pane: the lighting rows with icons, and the colour row moved to
/// a `rows` block.
fn split_colour(rows: Vec<Value>, d: &Device) -> (Vec<Value>, Option<Value>) {
    let follow = rows.iter().any(|r| r["key"] == "light/theme" && r["value"] == true);
    let mut left = Vec::new();
    let mut colour = Vec::new();
    for mut r in rows {
        if r["key"] == "light/colour" {
            r["title"] = json!("");
            colour.push(r);
        } else {
            left.push(with_icon(r));
        }
    }
    let effect = d.mode().map(|m| m.name.clone()).unwrap_or_default();
    let title = if follow { "Colour · following the theme".to_string() } else { format!("Colour · {effect}") };
    (left, (!colour.is_empty()).then(|| json!({ "kind": "rows", "title": title, "rows": colour })))
}

fn device_sub(d: &Device) -> String {
    if d.zones.len() > 1 { format!("{} · {} zones", d.kind_name(), d.zones.len()) } else { d.kind_name().to_string() }
}

impl Rgbx {
    fn all_page(&self, pane: bool) -> Result<Value> {
        let setup = setup_rows();
        if !setup.is_empty() {
            let setup = if pane { setup.into_iter().map(with_icon).collect() } else { setup };
            return Ok(Page::new("RGB lighting for motherboards, memory, graphics cards, fans and peripherals of every brand, through OpenRGB.")
                .group("OpenRGB", "", setup)
                .build());
        }
        let devices = devices();
        let list: Vec<Value> = devices.iter().map(|d| page::info(&d.id(), &d.name, d.kind_name())).collect();
        let theme = sys::theme_accent();
        let colour = page::colour_with_theme("all/colour", "One colour everywhere", "#ffffff", theme.as_deref());
        let mut rows = if pane { Vec::new() } else { vec![colour.clone()] };
        if theme.is_some() {
            rows.push(desc(page::button("all/theme", "Use the theme colour everywhere", "Apply"), "Each device's own page can follow the theme as it changes."));
        }
        rows.push(desc(page::button("all/off", "Turn every light off", "Off"), ""));
        let profiles = if mock::on() {
            mock::profiles()
        } else {
            let mut c = Client::connect()?;
            c.profiles().unwrap_or_default()
        };
        let mut prof = vec![page::entry_row("profile/save", "Save the lighting as a profile", "", "Profile name")];
        if !profiles.is_empty() {
            let opts: Vec<(String, String)> = profiles.iter().map(|p| (p.clone(), p.clone())).collect();
            prof.push(page::choice_owned("profile/load", "Load a profile", "", &opts));
        }
        if !pane {
            return Ok(Page::new(&format!("<b>{}</b> devices", devices.len()))
                .group("Everything", "", rows)
                .group("Profiles", "Profiles are OpenRGB's: the effect and colours of every device.", prof)
                .group("Devices", "Each one has its own page for effects, brightness and speed.", list)
                .build());
        }
        let mut colour = colour;
        colour["title"] = json!("");
        let items: Vec<Value> = devices
            .iter()
            .map(|d| {
                let value = current_colour(d).map(Rgb::hex).or_else(|| d.mode().map(|m| m.name.clone())).unwrap_or_default();
                json!({ "icon": d.icon(), "label": d.name, "sub": device_sub(d), "value": value })
            })
            .collect();
        let mut v = Page::new("")
            .group("Everything", "", rows.into_iter().map(with_icon).collect())
            .group(
                "Profiles",
                "Profiles are OpenRGB's: the effect and colours of every device.",
                prof.into_iter().map(with_icon).collect(),
            )
            .build();
        v["preview"] = json!({
            "title": format!("{} devices", devices.len()),
            "line": "Through OpenRGB · each has its own page for effects, brightness and speed",
            "blocks": [
                { "kind": "rows", "title": "One colour everywhere", "rows": [colour] },
                { "kind": "list", "items": items },
            ],
        });
        Ok(v)
    }

    fn device_page(&self, dev: &str, pane: bool) -> Result<Value> {
        let Some(d) = devices().into_iter().find(|d| d.id() == dev) else { bail!("OpenRGB doesn't see that device any more") };
        let mut info = vec![page::info("kind", "Kind", d.kind_name())];
        for (k, t, v) in [("vendor", "Maker", &d.vendor), ("version", "Firmware", &d.version), ("serial", "Serial", &d.serial), ("location", "Connected at", &d.location)] {
            if !v.is_empty() {
                info.push(page::info(k, t, v));
            }
        }
        if !pane {
            if !d.zones.is_empty() {
                let zones: Vec<String> = d.zones.iter().map(|z| format!("{} ({} LEDs)", z.name, z.leds_count)).collect();
                info.push(page::info("zones", "Zones", &zones.join("\n")));
            }
            return Ok(Page::new(&format!("<b>{}</b> · {}", esc(&d.name), d.kind_name()))
                .group("Lighting", "", light_rows(&d))
                .group("Device", "", info)
                .build());
        }
        let (rows, colour) = split_colour(light_rows(&d), &d);
        let facts: Vec<Value> = info.iter().map(|r| json!({ "label": r["title"], "value": r["value"] })).collect();
        let mut blocks: Vec<Value> = colour.into_iter().collect();
        blocks.push(json!({ "kind": "facts", "items": facts }));
        if !d.zones.is_empty() {
            let zones: Vec<Value> =
                d.zones.iter().map(|z| json!({ "label": z.name, "sub": format!("{} LEDs", z.leds_count) })).collect();
            blocks.push(json!({ "kind": "list", "title": "Zones", "items": zones }));
        }
        let mut v = Page::new("").group("Lighting", "", rows).build();
        v["preview"] = json!({ "title": d.name, "line": format!("{} · through OpenRGB", d.kind_name()), "blocks": blocks });
        Ok(v)
    }

    fn describe_page(&self, id: &str, pane: bool) -> Result<Value> {
        if id == ALL {
            return self.all_page(pane);
        }
        let Some(dev) = id.strip_prefix(DEVICE) else { bail!("no such page") };
        self.device_page(dev, pane)
    }
}

impl Extension for Rgbx {
    fn pages(&self) -> Value {
        if !mock::on() {
            let (installed, _) = lighting::status();
            if !installed && !sys::present("openrgb") {
                // Nothing to show until OpenRGB is installed: one page to set it up.
                return json!([page::entry(ALL, "RGB lighting", "weather-clear-symbolic", "Lighting for RGB devices of every brand, through OpenRGB.", "rgb lighting led openrgb colour")]);
            }
        }
        let mut out = vec![page::entry(ALL, "RGB lighting", "weather-clear-symbolic", "Every RGB device at once: one colour, the theme colour, saved profiles.", "rgb lighting led openrgb colour profile all")];
        for d in devices() {
            out.push(page::entry(
                &format!("{DEVICE}{}", d.id()),
                &d.name,
                d.icon(),
                &format!("{} lighting.", d.kind_name()),
                &format!("rgb lighting led {} {} {}", d.kind_name().to_lowercase(), d.vendor.to_lowercase(), d.name.to_lowercase()),
            ));
        }
        Value::Array(out)
    }

    fn describe(&self, id: &str) -> Result<Value> {
        self.describe_page(id, preview_supported())
    }

    fn set(&self, page: &str, key: &str, value: &str) -> Result<Option<Value>> {
        let device = page.strip_prefix(DEVICE).unwrap_or("");
        if mock::on() {
            return mock::set(device, key, value);
        }
        if let Some(reply) = lighting::set(device, key, value)? {
            return Ok(reply);
        }
        let mut c = Client::connect()?;
        match key {
            "all/colour" | "all/theme" | "all/off" => {
                let colour = match key {
                    "all/colour" => Rgb::parse(value).context("not a colour")?,
                    "all/theme" => sys::theme_accent().and_then(|t| Rgb::parse(&t)).context("the theme has no accent colour")?,
                    _ => Rgb(0, 0, 0),
                };
                let devices = c.devices()?;
                for d in &devices {
                    openrgb::set_colour(&mut c, d, colour)?;
                }
                Ok(nyx_ext::toast(&format!("{} devices set", devices.len())))
            }
            "profile/save" => {
                let name = value.trim();
                if name.is_empty() || name.contains('/') {
                    bail!("Give the profile a name");
                }
                c.save_profile(name)?;
                Ok(nyx_ext::refresh())
            }
            "profile/load" => {
                c.load_profile(value)?;
                Ok(nyx_ext::toast(&format!("Loaded {value}")))
            }
            _ => bail!("unknown setting {key}"),
        }
    }

    fn theme_changed(&self) -> Result<()> {
        // Pretend devices following the theme take its colour when drawn.
        if mock::on() {
            return Ok(());
        }
        lighting::theme_changed()
    }
}

fn main() {
    nyx_ext::run(&Rgbx);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock() {
        // SAFETY: tests in this crate only ever set this one value.
        unsafe { std::env::set_var("RGB_MOCK", "1") };
    }

    fn left_rows(page: &Value) -> Vec<&Value> {
        page["groups"].as_array().unwrap().iter().flat_map(|g| g["rows"].as_array().unwrap()).collect()
    }

    #[test]
    fn every_device_page() {
        mock();
        let all = Rgbx.describe_page(ALL, true).unwrap();
        let pv = &all["preview"];
        assert_eq!(pv["title"], "5 devices");
        assert_eq!(pv["blocks"][0]["rows"][0]["key"], "all/colour");
        let list = pv["blocks"][1]["items"].as_array().unwrap();
        assert_eq!(list.len(), 5);
        assert_eq!((list[0]["label"].as_str(), list[0]["sub"].as_str()), (Some("ROG STRIX Z790-E"), Some("Motherboard · 4 zones")));
        assert!(list.iter().all(|i| i["value"].as_str().is_some_and(|v| v.starts_with('#') && v.len() == 7)));
        assert!(left_rows(&all).iter().all(|r| r["icon"].as_str().is_some_and(|i| !i.is_empty())));
        assert!(left_rows(&all).iter().all(|r| r["key"] != "all/colour"));

        let old = Rgbx.describe_page(ALL, false).unwrap();
        assert!(old.get("preview").is_none());
        assert_eq!(old["subtitle"], "<b>5</b> devices");
        assert!(left_rows(&old).iter().all(|r| r.get("icon").is_none()));
        assert_eq!(old["groups"][2]["rows"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn one_device_page() {
        mock();
        let id = format!("{DEVICE}{}", mock::devices()[0].id());
        let d = Rgbx.describe_page(&id, true).unwrap();
        let pv = &d["preview"];
        assert_eq!(pv["title"], "ROG STRIX Z790-E");
        let kinds: Vec<&str> = pv["blocks"].as_array().unwrap().iter().map(|b| b["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["rows", "facts", "list"]);
        assert_eq!(pv["blocks"][0]["rows"][0]["key"], "light/colour");
        assert_eq!(pv["blocks"][0]["rows"][0]["value"], "#ef4444");
        assert_eq!(pv["blocks"][1]["items"][2]["value"], "AULA3-AR32-0207");
        let zones = pv["blocks"][2]["items"].as_array().unwrap();
        assert_eq!((zones.len(), zones[1]["sub"].as_str()), (4, Some("36 LEDs")));
        let rows = left_rows(&d);
        assert!(rows.iter().all(|r| r["icon"].as_str().is_some_and(|i| !i.is_empty())));
        assert_eq!(rows[0]["key"], "light/effect");

        let old = Rgbx.describe_page(&id, false).unwrap();
        assert!(old.get("preview").is_none());
        assert_eq!(old["groups"][1]["title"], "Device");
        assert!(left_rows(&old).iter().all(|r| r.get("icon").is_none()));
        assert!(Rgbx.describe_page("dev-nope", true).is_err());
    }

    #[test]
    fn pages_list_the_pretend_devices() {
        mock();
        let p = Rgbx.pages();
        let names: Vec<&str> = p.as_array().unwrap().iter().map(|e| e["title"].as_str().unwrap()).collect();
        assert_eq!(names, ["RGB lighting", "ROG STRIX Z790-E", "Vengeance RGB × 2", "RTX 4080 SUPER", "LL120 fans × 3", "Basilisk V3"]);
    }
}
