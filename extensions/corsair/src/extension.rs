//! The Settings extension protocol: `pages`, `describe PAGE`, `set PAGE KEY VALUE`.

use crate::usb::{self, Access, Device};
use crate::{access, mock, models};
use anyhow::{Result, bail};
use nyx_ext::lighting;
use nyx_ext::openrgb::{self, COLOR_MODE_SPECIFIC, Rgb};
use serde_json::{Value, json};
use std::process::Command;

/// Pages for Corsair devices OpenRGB finds that aren't one of the keyboards above.
const ORGB: &str = "orgb-";

fn corsair_lights() -> Vec<openrgb::Device> {
    let all = if mock::on() { mock::devices() } else { lighting::devices() };
    all.into_iter().filter(|d| d.vendor.to_lowercase().contains("corsair") || d.name.to_lowercase().contains("corsair")).collect()
}

/// Whether this Settings draws a preview pane beside the groups.
fn preview_supported() -> bool {
    std::env::var("SETTINGS_KINDS").is_ok_and(|k| k.split(' ').any(|x| x == "preview"))
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
        "web-hub" => (
            "web-browser-symbolic",
            Some("Key remaps, the dials and firmware updates, saved on the keyboard. Opens in Chrome or Chromium"),
        ),
        "allow" => ("dialog-password-symbolic", None),
        _ => ("dialog-information-symbolic", None),
    };
    row["icon"] = json!(icon);
    if let Some(t) = text {
        row["desc"] = json!(t);
    }
    row
}

/// The colour a device shows now: its effect's colour, else its LEDs'.
fn current_colour(d: &openrgb::Device) -> Option<Rgb> {
    match d.mode() {
        Some(m) if m.color_mode == COLOR_MODE_SPECIFIC => m.colors.first().copied(),
        _ => d.colors.first().copied(),
    }
}

/// The preview's picture for an OpenRGB device kind.
fn picture(kind: i32) -> Option<&'static str> {
    match kind {
        1 => Some("memory"),
        2 => Some("gpu"),
        5 => Some("keyboard"),
        6 => Some("mouse"),
        8 => Some("headset"),
        10 => Some("gamepad"),
        _ => None,
    }
}

fn picture_block(kind: &str, light: Option<&openrgb::Device>) -> Value {
    let mut b = json!({ "kind": "picture", "picture": kind });
    if let Some(c) = light.and_then(current_colour) {
        b["accent"] = json!(c.hex());
    }
    b
}

fn facts(items: &[(&str, String)]) -> Value {
    let items: Vec<Value> = items.iter().filter(|(_, v)| !v.is_empty()).map(|(l, v)| json!({ "label": l, "value": v })).collect();
    json!({ "kind": "facts", "items": items })
}

/// `2 × 10 LEDs`, `99 LEDs`, or `3 zones · 48 LEDs`.
fn zones_text(l: &openrgb::Device) -> String {
    let counts: Vec<u32> = l.zones.iter().map(|z| z.leds_count).collect();
    match counts.as_slice() {
        [] => String::new(),
        [n] => format!("{n} LEDs"),
        [first, rest @ ..] if rest.iter().all(|n| n == first) => format!("{} × {first} LEDs", counts.len()),
        _ => format!("{} zones · {} LEDs", counts.len(), counts.iter().sum::<u32>()),
    }
}

/// The OpenRGB device for a USB keyboard: same model name.
fn lights_for<'a>(d: &Device, lights: &'a [openrgb::Device]) -> Option<&'a openrgb::Device> {
    let want = title(d).to_lowercase();
    let first = want.split_whitespace().next().unwrap_or("").to_string();
    lights.iter().find(|l| l.kind == 5 && l.name.to_lowercase().contains(&want)).or_else(|| {
        lights.iter().find(|l| l.kind == 5 && !first.is_empty() && l.name.to_lowercase().contains(&first))
    })
}

/// The Lighting group, and (with a preview pane) its colour picker to draw there.
fn lighting_group(light: Option<&openrgb::Device>, pane: bool) -> (Value, Option<Value>) {
    let mut rows = if mock::on() { Vec::new() } else { lighting::setup_rows() };
    if rows.is_empty() {
        match light {
            Some(l) if mock::on() => rows = mock::rows(l),
            Some(l) => rows = lighting::rows(l),
            None => rows.push(json!({ "kind": "info", "title": "Lighting", "value": "OpenRGB doesn't list this device" ,
                "tooltip": "OpenRGB supports most Corsair keyboards, mice, headsets, memory and fan hubs. A model it doesn't know yet still works with Corsair Web Hub." })),
        }
    }
    if !pane {
        return (json!({ "title": "Lighting", "rows": rows }), None);
    }
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
    let title = match light.and_then(|l| l.mode()) {
        _ if follow => "Colour · following the theme".to_string(),
        Some(m) => format!("Colour · {}", m.name),
        None => "Colour".to_string(),
    };
    let block = (!colour.is_empty()).then(|| json!({ "kind": "rows", "title": title, "rows": colour }));
    (json!({ "title": "Lighting", "rows": left }), block)
}

pub const WEB_HUB: &str = "https://www.corsair.com/web-hub";
const SETUP: &str = "setup";

/// Corsair devices that get a page: known keyboards, and anything else with a keyboard interface.
fn keyboards(all: &[Device]) -> Vec<&Device> {
    all.iter().filter(|d| !models::is_stream_deck(d.product) && (models::by_keyboard(d.product).is_some() || d.has_keyboard())).collect()
}

fn title(d: &Device) -> String {
    match models::by_keyboard(d.product) {
        Some(m) => m.name.trim_start_matches("Corsair ").to_string(),
        None => d.name.clone(),
    }
}

pub fn pages() -> Value {
    let all = usb::scan();
    let boards = keyboards(&all);
    if !boards.is_empty() && boards.iter().all(|d| usb::access(d) == Access::Denied) {
        return json!([{
            "id": SETUP, "title": "Corsair", "icon": "input-keyboard-symbolic",
            "description": "A Corsair keyboard is plugged in, but this account can't talk to it yet.",
            "keywords": "corsair keyboard permission access udev web hub",
        }]);
    }
    let lights = corsair_lights();
    let mut out: Vec<Value> = boards
        .iter()
        .map(|d| {
            json!({
                "id": d.page_id(), "title": title(d), "icon": "input-keyboard-symbolic",
                "description": "Corsair keyboard: lighting, device details and access for Corsair Web Hub.",
                "keywords": format!("corsair keyboard galleon lighting rgb web hub {}", d.name.to_lowercase()),
            })
        })
        .collect();
    // Mice, headsets, memory and fan hubs, and keyboards with no USB page.
    for l in &lights {
        if boards.iter().any(|d| lights_for(d, &lights).is_some_and(|x| x.id() == l.id())) {
            continue;
        }
        out.push(json!({
            "id": format!("{ORGB}{}", l.id()), "title": l.name.trim_start_matches("Corsair ").to_string(), "icon": l.icon(),
            "description": format!("Corsair {}: lighting.", l.kind_name().to_lowercase()),
            "keywords": format!("corsair lighting rgb {} {}", l.kind_name().to_lowercase(), l.name.to_lowercase()),
        }));
    }
    Value::Array(out)
}

fn light_page(id: &str, pane: bool) -> Result<Value> {
    let Some(l) = corsair_lights().into_iter().find(|l| l.id() == id) else { bail!("OpenRGB doesn't see that device any more") };
    let (group, colour) = lighting_group(Some(&l), pane);
    if pane {
        let mut blocks: Vec<Value> = picture(l.kind).map(|p| picture_block(p, Some(&l))).into_iter().collect();
        blocks.extend(colour);
        blocks.push(facts(&[
            ("Kind", l.kind_name().to_string()),
            ("Firmware", l.version.clone()),
            ("Serial", l.serial.clone()),
            ("Zones", zones_text(&l)),
        ]));
        return Ok(json!({
            "groups": [group],
            "preview": {
                "title": l.name.trim_start_matches("Corsair "), "line": format!("{} · through OpenRGB", l.kind_name()), "blocks": blocks,
            },
        }));
    }
    let mut rows = vec![json!({ "kind": "info", "title": "Kind", "value": l.kind_name() })];
    if !l.version.is_empty() {
        rows.push(json!({ "kind": "info", "title": "Firmware", "value": l.version }));
    }
    if !l.serial.is_empty() {
        rows.push(json!({ "kind": "info", "title": "Serial", "value": l.serial }));
    }
    Ok(json!({
        "subtitle": format!("<b>{}</b> · {}", gtk_escape(&l.name), l.kind_name()),
        "groups": [group, { "title": "Device", "rows": rows }],
    }))
}

fn gtk_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn access_group() -> Value {
    let installed = access::rule_installed();
    json!({ "title": "Device access", "rows": [{
        "kind": "button", "key": "allow", "title": "Allow access to Corsair keyboards",
        "desc": format!(
            "{}Installs <tt>{}</tt>, which lets your account (and Corsair Web Hub in the browser) talk to them. Asks for your password.",
            if installed { "Already set up. " } else { "" },
            access::rule_path().display()
        ),
        "label": if installed { "Set up again" } else { "Allow access" },
        "keywords": "permission udev hidraw access",
    }]})
}

fn setup_page() -> Value {
    json!({
        "banners": [{ "text": "Linux only lets <b>root</b> talk to these devices until a small rule allows your account to.", "warning": true }],
        "groups": [access_group()],
    })
}

fn stream_deck_installed() -> bool {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
    data.join("settings/extensions/streamdeck").is_dir()
}

fn stream_deck_running() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "galleon-deck.service"])
        .status()
        .is_ok_and(|s| s.success())
}

pub fn describe(id: &str) -> Result<Value> {
    describe_page(id, preview_supported())
}

fn describe_page(id: &str, pane: bool) -> Result<Value> {
    if id == SETUP {
        let mut page = setup_page();
        if pane {
            page["groups"][0]["rows"][0] = with_icon(page["groups"][0]["rows"][0].take());
        }
        return Ok(page);
    }
    if let Some(l) = id.strip_prefix(ORGB) {
        return light_page(l, pane);
    }
    let all = usb::scan();
    let Some(d) = keyboards(&all).into_iter().find(|d| d.page_id() == id).cloned() else {
        bail!("that keyboard isn't plugged in any more");
    };
    let model = models::by_keyboard(d.product);
    let mut groups = Vec::new();
    let mut banners = Vec::new();
    if model.is_none() {
        banners.push(json!({ "text": "Lighting for this keyboard comes from OpenRGB; key remaps and firmware are in Corsair Web Hub.", "warning": false }));
    }
    if d.simulated {
        banners.push(json!({ "text": "Simulated keyboard (<tt>CORSAIR_MOCK</tt>).", "warning": false }));
    }

    // Lighting through OpenRGB; key remaps, dials and firmware in Web Hub.
    let lights = corsair_lights();
    let light = lights_for(&d, &lights);
    let (group, colour) = lighting_group(light, pane);
    groups.push(group);
    let web_hub = json!({
        "kind": "button", "key": "web-hub", "title": "Corsair Web Hub",
        "desc": "Key remaps, the dials and firmware updates, saved on the keyboard itself. Opens in your browser; needs Chrome or Chromium and device access.",
        "label": "Open", "keywords": "remap macro firmware update web hub",
    });
    groups.push(json!({ "title": "Keys and firmware", "rows": [if pane { with_icon(web_hub) } else { web_hub }]}));

    if let Some(deck) = model.and_then(|m| m.stream_deck) {
        let present = all.iter().any(|x| x.product == deck);
        let value = match (present, stream_deck_installed(), stream_deck_running()) {
            (false, _, _) => "Not detected",
            (true, false, _) => "Install the Stream Deck extension (Settings → Extensions)",
            (true, true, true) => "Running: set it up on the Stream Deck page",
            (true, true, false) => "Stopped: start it on the Stream Deck page",
        };
        let tooltip = "The 12 screen keys, top screen and dials are a separate device, run by the Stream Deck extension.";
        if pane {
            groups.push(json!({ "title": "Built-in Stream Deck", "rows": [{
                "kind": "info", "title": "Stream Deck", "desc": value, "icon": "view-grid-symbolic",
                "tooltip": tooltip, "keywords": "stream deck lcd keys dials screen",
            }]}));
        } else {
            groups.push(json!({ "title": "Stream Deck", "rows": [{
                "kind": "info", "title": "Built-in Stream Deck", "value": value,
                "tooltip": tooltip,
                "keywords": "stream deck lcd keys dials screen",
            }]}));
        }
    }

    if pane {
        if usb::access(&d) != Access::Allowed || !access::rule_installed() {
            let mut g = access_group();
            g["rows"][0] = with_icon(g["rows"][0].take());
            groups.push(g);
        }
        return Ok(json!({ "banners": banners, "groups": groups, "preview": keyboard_preview(&d, model, light, colour) }));
    }

    let mut rows = vec![
        json!({ "kind": "info", "title": "Model", "value": model.map(|m| m.name).unwrap_or(&d.name) }),
        json!({ "kind": "info", "title": "USB id", "value": format!("{:04x}:{:04x}", models::CORSAIR, d.product) }),
        json!({ "kind": "info", "title": "USB release", "value": d.release_text(), "tooltip": "The version number the keyboard reports over USB." }),
    ];
    if !d.serial.is_empty() {
        rows.push(json!({ "kind": "info", "title": "Serial", "value": d.serial }));
    }
    let ifaces: Vec<String> = d.interfaces.iter().map(|i| format!("{}: {}", i.number, i.summary().page_name())).collect();
    rows.push(json!({ "kind": "info", "title": "Interfaces", "value": ifaces.join("\n") }));
    groups.push(json!({ "title": "Device", "rows": rows }));

    if usb::access(&d) != Access::Allowed || !access::rule_installed() {
        groups.push(access_group());
    }
    Ok(json!({ "banners": banners, "groups": groups }))
}

/// The keyboard lit in its current colour, its colour picker, and what it reports over USB.
fn keyboard_preview(d: &Device, model: Option<&models::Model>, light: Option<&openrgb::Device>, colour: Option<Value>) -> Value {
    let usb_id = format!("USB {:04x}:{:04x}", models::CORSAIR, d.product);
    let line = if model.and_then(|m| m.stream_deck).is_some() {
        format!("A keyboard with a Stream Deck built in · {usb_id}")
    } else {
        format!("Keyboard · {usb_id}")
    };
    let mut ifaces: Vec<String> = Vec::new();
    for i in d.interfaces.iter().filter(|i| !i.descriptor.is_empty()) {
        let name = if i.class == 3 && i.protocol == 1 { "Keyboard".to_string() } else { i.summary().page_name() };
        let name = name.split(" (").next().unwrap_or_default().to_string();
        if !ifaces.contains(&name) {
            ifaces.push(name);
        }
    }
    let mut blocks = vec![picture_block("keyboard", light)];
    blocks.extend(colour);
    blocks.push(facts(&[
        ("Model", model.map(|m| m.name).unwrap_or(&d.name).to_string()),
        ("USB release", d.release_text()),
        ("Serial", d.serial.clone()),
        ("Interfaces", ifaces.join(" · ")),
    ]));
    json!({ "title": title(d), "line": line, "blocks": blocks })
}

fn probe_path() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("corsair-probe.txt")
}

pub fn save_probe(path: &std::path::Path) -> Result<()> {
    std::fs::write(path, crate::probe::report(true))?;
    Ok(())
}

pub fn set(page: &str, key: &str, value: &str) -> Result<Option<Value>> {
    if key.starts_with("light/") || key.starts_with("openrgb/") {
        let device = match page.strip_prefix(ORGB) {
            Some(id) => id.to_string(),
            None => {
                let all = usb::scan();
                let lights = corsair_lights();
                keyboards(&all).into_iter().find(|d| d.page_id() == page).and_then(|d| lights_for(d, &lights)).map(|l| l.id()).unwrap_or_default()
            }
        };
        if mock::on() {
            return mock::set(&device, key, value);
        }
        if let Some(reply) = lighting::set(&device, key, value)? {
            return Ok(reply);
        }
    }
    match key {
        "allow" if mock::on() => Ok(Some(json!({ "toast": "Access allowed (simulated)", "reload": true }))),
        "allow" => {
            access::install_rule()?;
            Ok(Some(json!({ "toast": "Access allowed", "reload": true })))
        }
        "web-hub" => {
            Command::new("xdg-open").arg(WEB_HUB).spawn()?;
            Ok(None)
        }
        "probe" => {
            let path = probe_path();
            save_probe(&path)?;
            Ok(Some(json!({ "toast": format!("Saved {}", path.display()) })))
        }
        _ => bail!("unknown setting {key}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock() {
        // SAFETY: tests in this crate only ever set this one value.
        unsafe { std::env::set_var("CORSAIR_MOCK", "galleon-100-sd") };
    }

    fn left_rows(page: &Value) -> Vec<&Value> {
        page["groups"].as_array().unwrap().iter().flat_map(|g| g["rows"].as_array().unwrap()).collect()
    }

    #[test]
    fn one_page_for_the_galleon_not_its_deck() {
        mock();
        let p = pages();
        let p = p.as_array().unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0]["title"], "Galleon 100 SD");
        assert_eq!(p[0]["id"], "2b0c-sim0000galleon");
        assert_eq!(p[1]["title"], "Vengeance RGB");
    }

    #[test]
    fn describes_the_galleon() {
        mock();
        let d = describe_page("2b0c-sim0000galleon", false).unwrap();
        let titles: Vec<&str> = d["groups"].as_array().unwrap().iter().map(|g| g["title"].as_str().unwrap()).collect();
        assert_eq!(&titles[..4], ["Lighting", "Keys and firmware", "Stream Deck", "Device"]);
        let device = &d["groups"][3]["rows"];
        assert_eq!(device[1]["value"], "1b1c:2b0c");
        assert!(device[4]["value"].as_str().unwrap().contains("1: Vendor-defined (0xFF42)"));
        assert!(d.get("preview").is_none());
        assert!(left_rows(&d).iter().all(|r| r.get("icon").is_none()));
        assert_eq!(d["groups"][0]["rows"][0]["key"], "light/effect");
        assert!(describe("nope").is_err());
        assert!(set("x", "teleport", "").is_err());
    }

    #[test]
    fn galleon_preview() {
        mock();
        let d = describe_page("2b0c-sim0000galleon", true).unwrap();
        let titles: Vec<&str> = d["groups"].as_array().unwrap().iter().map(|g| g["title"].as_str().unwrap()).collect();
        assert_eq!(&titles[..3], ["Lighting", "Keys and firmware", "Built-in Stream Deck"]);
        assert!(!titles.contains(&"Device"));
        assert!(left_rows(&d).iter().all(|r| r["icon"].as_str().is_some_and(|i| !i.is_empty())));
        assert!(left_rows(&d).iter().all(|r| r["key"] != "light/colour"));
        let pv = &d["preview"];
        assert_eq!(pv["title"], "Galleon 100 SD");
        assert_eq!(pv["line"], "A keyboard with a Stream Deck built in · USB 1b1c:2b0c");
        let kinds: Vec<&str> = pv["blocks"].as_array().unwrap().iter().map(|b| b["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["picture", "rows", "facts"]);
        assert_eq!(pv["blocks"][0]["picture"], "keyboard");
        assert_eq!(pv["blocks"][1]["rows"][0]["key"], "light/colour");
        let facts = &pv["blocks"][2]["items"];
        assert_eq!((facts[1]["value"].as_str(), facts[2]["value"].as_str()), (Some("1.04"), Some("SIM0000GALLEON")));
        assert_eq!(facts[3]["value"], "Keyboard · Vendor-defined · Consumer");
    }

    #[test]
    fn memory_preview() {
        mock();
        let id = format!("{ORGB}{}", mock::devices()[1].id());
        let d = describe_page(&id, true).unwrap();
        assert!(left_rows(&d).iter().all(|r| r["icon"].as_str().is_some_and(|i| !i.is_empty())));
        let pv = &d["preview"];
        assert_eq!((pv["title"].as_str(), pv["line"].as_str()), (Some("Vengeance RGB"), Some("Memory · through OpenRGB")));
        assert_eq!(pv["blocks"][0]["picture"], "memory");
        assert_eq!(pv["blocks"][0]["accent"], "#8b5cf6");
        let facts = pv["blocks"].as_array().unwrap().last().unwrap()["items"].as_array().unwrap();
        assert_eq!(facts.iter().map(|f| f["value"].as_str().unwrap()).collect::<Vec<_>>(), ["Memory", "2 × 10 LEDs"]);

        let old = describe_page(&id, false).unwrap();
        assert!(old.get("preview").is_none());
        assert_eq!(old["groups"][1]["title"], "Device");
        assert!(left_rows(&old).iter().all(|r| r.get("icon").is_none()));
    }
}
