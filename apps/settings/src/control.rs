//! Settings' tools on the control interface (`nyx_core::control`), for Nyx Assistant:
//! which pages there are and showing one, what this computer can do, where a setting's
//! value comes from, and the remembered devices. Read-only by design: nothing here
//! changes a setting (computer use can't reach Settings either); the person changes it
//! on the page.

use crate::{sections, window};
use nyx_core::control::text;
use serde_json::{Value, json};

pub fn tools() -> Value {
    json!([
        {"name": "settings.pages", "description": "The pages of Nyx Settings (id, title, what each covers).", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}}},
        {"name": "settings.open", "description": "Show a page of Nyx Settings (by id or title from settings.pages) for the person to change something themselves.", "inputSchema": {"type": "object", "properties": {"page": {"type": "string"}}, "required": ["page"]}},
        {"name": "settings.capabilities", "description": "What this computer can do now: installed programs, services, hardware, each available or why not.", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}}},
        {"name": "settings.value", "description": "A setting of a NyxOS configuration domain (dock, control-center, switcher, bar, shell, idle, appearance, notifications): the value in effect, NyxOS's default, and the person's own value if they set one.", "inputSchema": {"type": "object", "properties": {"domain": {"type": "string"}, "key": {"type": "string"}}, "required": ["domain", "key"]}},
        {"name": "settings.devices", "description": "Devices with their own settings that Settings remembers (displays, mice, sound devices), and when each was last seen.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "settings.virtual_monitors", "description": "Virtual monitors (one wide screen split into two or three): whether they're on, each split screen's layout and regions as Hyprland has them now, and the saved layouts. Changing them is for the person, on the Displays page or with `nyx display virtual`.", "inputSchema": {"type": "object", "properties": {}}},
    ])
}

/// API 2 for `nyx settings …`, `nyx devices …`, Glimpse and Nyx Assistant: Settings' own
/// tools (all read-only; `settings.open` only shows a page), and the phones paired
/// through Flux (Nyx Connect): listing them, ringing one, sending the clipboard to one
/// (which asks first: it leaves this computer), and their page. Pairing and access stay
/// on the page. D-Bus starts Settings without its window for a call.
pub fn description(app_id: &str) -> Value {
    let x = [
        ("settings.pages", json!({"risk": "read_only", "aliases": ["find"], "positional": ["query"], "result": "object"})),
        ("settings.open", json!({"icon": "emblem-system-symbolic", "risk": "writes", "confirmation": "never", "aliases": ["show"], "positional": ["page"], "suggest": {"page": "settings.page_names"}, "result": "object"})),
        ("settings.capabilities", json!({"risk": "read_only", "positional": ["query"], "result": "object"})),
        ("settings.value", json!({"risk": "read_only", "positional": ["domain", "key"], "result": "object"})),
        ("settings.devices", json!({"risk": "read_only", "result": "object"})),
        ("settings.virtual_monitors", json!({"risk": "read_only", "result": "object"})),
    ];
    let mut list = tools();
    for t in list.as_array_mut().into_iter().flatten() {
        if let Some((_, m)) = x.iter().find(|(n, _)| t["name"] == *n) {
            t["x-nyx"] = m.clone();
        }
        let title = match t["name"].as_str().unwrap_or("") {
            "settings.pages" => "Settings pages",
            "settings.open" => "Show a page",
            "settings.capabilities" => "What this computer can do",
            "settings.value" => "Where a setting comes from",
            "settings.devices" => "Remembered devices",
            "settings.virtual_monitors" => "Virtual monitors",
            _ => "",
        };
        if !title.is_empty() {
            t["annotations"] = json!({ "title": title });
        }
    }
    let device = json!({"type": "string", "minLength": 1, "description": "a paired device, by name or ID"});
    let one = json!({"type": "object", "properties": {"device": device}, "required": ["device"], "additionalProperties": false});
    let none = json!({"type": "object", "properties": {}, "additionalProperties": false});
    if let Some(a) = list.as_array_mut() {
        a.extend([
            json!({"name": "settings.page_names", "description": "Settings pages whose title starts with a prefix, for completing a page.",
                   "inputSchema": {"type": "object", "properties": {"prefix": {"type": "string"}, "limit": {"type": "integer"}}, "additionalProperties": false},
                   "annotations": {"title": "Page names", "readOnlyHint": true}, "x-nyx": {"risk": "read_only", "result": "object"}}),
            json!({"name": "devices.list", "description": "Phones and other devices paired through Flux: connected or not, battery, what each can do now. A prefix keeps those whose name starts with it.",
                   "inputSchema": {"type": "object", "properties": {"prefix": {"type": "string"}, "limit": {"type": "integer"}}, "additionalProperties": false},
                   "annotations": {"title": "Devices", "readOnlyHint": true}, "x-nyx": {"icon": "phone-symbolic", "risk": "read_only", "aliases": ["phones"], "positional": ["prefix"], "result": "list"}}),
            json!({"name": "devices.ring", "description": "Ring a connected phone, to find it.", "inputSchema": one, "annotations": {"title": "Ring"},
                   "x-nyx": {"risk": "writes", "aliases": ["find"], "positional": ["device"], "suggest": {"device": "devices.list"}, "result": "message"}}),
            json!({"name": "devices.send", "description": "Send what's on this computer's clipboard to a connected device.", "inputSchema": one, "annotations": {"title": "Send the clipboard"},
                   "x-nyx": {"risk": "external", "aliases": ["clipboard"], "positional": ["device"], "suggest": {"device": "devices.list"}, "result": "message"}}),
            json!({"name": "devices.open", "description": "Open Settings › Devices (pairing, access).", "inputSchema": none, "annotations": {"title": "Open Devices"},
                   "x-nyx": {"risk": "writes", "confirmation": "never", "result": "none"}}),
        ]);
    }
    json!({
        "tools": list,
        "nyx": {"api": 2, "app": app_id, "name": "Settings", "icon": crate::APP_ID, "namespaces": ["settings", "devices"],
                "bus_name": app_id, "activation": "dbus", "default": "settings.pages", "open": {"action": "settings.open"},
                "topics": [{"name": "devices"}],
                "panel": {"id": "devices", "topics": ["devices"], "size": {"min": [420, 300], "preferred": [500, 380]}}}
    })
}

thread_local! {
    static SERVICE: std::cell::RefCell<Option<nyx_core::control::Service>> = const { std::cell::RefCell::new(None) };
    static TIMER: std::cell::RefCell<Option<gtk::glib::SourceId>> = const { std::cell::RefCell::new(None) };
}

/// Register the interface (from `connect_startup`; primary instance only). The
/// `devices` topic is read from Flux every 5 s only while something watches it.
pub fn serve(app: &gtk::Application) {
    use gtk::prelude::*;
    let id = app.application_id().map(|s| s.to_string()).unwrap_or_else(|| crate::APP_ID.to_string());
    let window_app = app.clone();
    let Some(service) = nyx_core::control::serve_with(app, description(&id), move |request, answer| {
        let failed = |e: String| nyx_actions::Error::new(nyx_actions::ErrorKind::Failed, e);
        match request.action.as_str() {
            "settings.open" | "devices.open" => {
                let want = if request.action == "devices.open" { "devices".to_string() } else { request.arguments["page"].as_str().unwrap_or("").trim().to_lowercase() };
                let page = sections::all().into_iter().filter(|s| s.shown()).find(|s| want.is_empty() || s.id == want || s.title.to_lowercase() == want);
                match page {
                    Some(p) => {
                        window::present(&window_app, Some(p.id));
                        answer(Ok(json!({"showing": p.title})))
                    }
                    None => answer(Err(failed("no Settings page of that name; settings pages lists them".into()))),
                }
            }
            "settings.page_names" => {
                let prefix = request.arguments["prefix"].as_str().unwrap_or("").to_lowercase();
                let items: Vec<Value> = sections::all()
                    .into_iter()
                    .filter(|s| s.shown() && s.title.to_lowercase().starts_with(&prefix))
                    .map(|s| json!({"value": s.title, "label": s.title, "detail": s.group}))
                    .collect();
                answer(Ok(json!({"items": items})))
            }
            "devices.list" => {
                let prefix = request.arguments["prefix"].as_str().unwrap_or("").to_lowercase();
                nyx_core::cmd::background(devices, move |found| answer(found.map(|v| keep(v, &prefix)).map_err(failed)));
            }
            "devices.ring" | "devices.send" => {
                let ring = request.action == "devices.ring";
                let wanted = request.arguments["device"].as_str().unwrap_or("").trim().to_string();
                nyx_core::cmd::background(
                    move || -> Result<String, String> {
                        let client = nyx_connect::Client::new();
                        let snapshot = client.state().map_err(|e| e.message)?;
                        let all: Vec<_> = snapshot.paired().collect();
                        let lower = wanted.to_lowercase();
                        let d = all
                            .iter()
                            .find(|d| d.id == wanted || d.display_name(&snapshot.devices).to_lowercase() == lower)
                            .ok_or_else(|| format!("No paired device called “{wanted}”; devices list shows them"))?;
                        let name = d.display_name(&snapshot.devices);
                        if !d.online {
                            return Err(format!("{name} isn't connected"));
                        }
                        if ring {
                            client.ring(&d.id).map_err(|e| e.message)?;
                            Ok(format!("Ringing {name}"))
                        } else {
                            client.send_clipboard(&d.id, "").map_err(|e| e.message)?;
                            Ok(format!("Sent the clipboard to {name}"))
                        }
                    },
                    move |done| answer(done.map(|m| json!({ "message": m })).map_err(failed)),
                );
            }
            _ => answer(handle(&request.action, request.arguments).map_err(failed)),
        }
    }) else {
        return;
    };
    service.on_watch(|_, on| {
        TIMER.with(|t| {
            if let Some(id) = t.borrow_mut().take() {
                id.remove();
            }
            if on {
                publish();
                *t.borrow_mut() = Some(gtk::glib::timeout_add_seconds_local(5, || {
                    publish();
                    gtk::glib::ControlFlow::Continue
                }));
            }
        });
    });
    SERVICE.with(|s| *s.borrow_mut() = Some(service));
}

/// The paired devices, connected first, as `devices.list` and the topic show them.
fn devices() -> Result<Value, String> {
    use nyx_connect::{Access, Feature};
    let s = nyx_connect::Client::new().state().map_err(|e| e.message)?;
    let mut items: Vec<Value> = s
        .paired()
        .map(|d| {
            let name = d.display_name(&s.devices);
            let battery = d.battery.filter(|_| d.online).map(|b| b.charge);
            let subtitle = match (d.online, battery) {
                (true, Some(b)) => format!("Connected · {b}%{}", if d.battery.is_some_and(|b| b.charging) { ", charging" } else { "" }),
                (true, None) => "Connected".to_string(),
                _ => "Not connected".to_string(),
            };
            json!({"title": name, "value": name, "id": d.id, "subtitle": subtitle, "icon": d.icon(), "connected": d.online, "battery": battery,
                   "can_ring": d.online && d.has(Feature::Ring),
                   "can_send": d.online && d.has(Feature::Clipboard) && s.allowed(&d.id, Access::Clipboard)})
        })
        .collect();
    items.sort_by_key(|v| (v["connected"] != true, v["title"].as_str().unwrap_or("").to_lowercase()));
    Ok(json!({ "items": items }))
}

fn keep(mut v: Value, prefix: &str) -> Value {
    if let Some(items) = v["items"].as_array_mut() {
        items.retain(|i| i["title"].as_str().unwrap_or("").to_lowercase().starts_with(prefix));
    }
    v
}

fn publish() {
    let Some(service) = SERVICE.with(|s| s.borrow().clone()) else { return };
    if !service.watched("devices") {
        return;
    }
    nyx_core::cmd::background(devices, move |found| service.publish("devices", found.unwrap_or_else(|e| json!({"error": e}))));
}

fn matches(s: &sections::Section, words: &[String]) -> bool {
    let hay = format!("{} {} {} {} {}", s.id, s.title, s.group, s.description, s.keywords).to_lowercase();
    words.iter().all(|w| hay.contains(w))
}

pub fn handle(name: &str, args: Value) -> Result<Value, String> {
    let shown = || sections::all().into_iter().filter(|s| s.shown());
    match name {
        "settings.pages" => {
            let words: Vec<String> = args["query"].as_str().unwrap_or("").split_whitespace().map(str::to_lowercase).collect();
            let pages: Vec<Value> = shown()
                .filter(|s| matches(s, &words))
                .map(|s| json!({"id": s.id, "title": s.title, "group": s.group, "about": s.description}))
                .collect();
            Ok(json!({"pages": pages}))
        }
        "settings.open" => {
            let want = text(&args, "page")?.trim().to_lowercase();
            let page = shown()
                .find(|s| s.id == want || s.title.to_lowercase() == want)
                .ok_or("no Settings page of that name; settings.pages lists them")?;
            if window::window().is_none() {
                return Err("Settings isn't ready".into());
            }
            window::navigate(page.id);
            Ok(json!({"showing": page.title}))
        }
        "settings.capabilities" => {
            let words: Vec<String> = args["query"].as_str().unwrap_or("").split_whitespace().map(str::to_lowercase).collect();
            let caps: Vec<Value> = crate::capability::registry()
                .snapshot()
                .into_values()
                .filter(|c| words.iter().all(|w| c.id.to_lowercase().contains(w)))
                .map(|c| json!({"id": c.id, "state": c.state, "reason": c.reason}))
                .collect();
            Ok(json!({"capabilities": caps}))
        }
        "settings.value" => {
            let domain = text(&args, "domain")?;
            let key = text(&args, "key")?;
            explain(&domain, &key)
        }
        "settings.devices" => {
            let devices: Vec<Value> = nyx_devices::Index::load_standard()
                .devices()
                .map(|(id, p)| json!({"id": id.0, "kind": p.category.label(), "name": p.name, "last_seen": p.last_seen, "port": p.connector}))
                .collect();
            Ok(json!({"devices": devices}))
        }
        "settings.virtual_monitors" => virtual_monitors(),
        _ => Err(format!("unknown tool {name}")),
    }
}

fn virtual_monitors() -> Result<Value, String> {
    use nyx_desktop::virtual_monitors as vm;
    let saved = vm::Paths::default_paths().and_then(|p| vm::Settings::load(&p)).map_err(|e| format!("{e:#}"))?;
    let live = nyx_desktop::hypr::Instance::find().ok().and_then(|h| vm::status(&h).ok());
    let screens: Vec<Value> = saved
        .displays
        .iter()
        .map(|d| json!({"id": d.id, "port": d.connector, "split": d.enabled, "layout": d.layout.preset.as_str(), "proportions": d.layout.proportions().unwrap_or_default(), "profile": d.profile, "workspaces": d.workspaces}))
        .collect();
    let now: Vec<Value> = live
        .iter()
        .flat_map(|s| &s.groups)
        .map(|g| json!({"screen": g.host, "state": g.state, "regions": g.regions.iter().map(|r| json!({"name": r.name, "width": r.width, "active_workspace": r.active_workspace})).collect::<Vec<_>>()}))
        .collect();
    Ok(json!({
        "supported": live.is_some(),
        "on": saved.enabled && live.as_ref().is_some_and(|s| s.available),
        "screens": screens,
        "now": now,
        "saved_layouts": saved.profiles.keys().collect::<Vec<_>>(),
    }))
}

/// Where a setting comes from, layer by layer (the files are readable by everyone).
fn explain(domain: &str, key: &str) -> Result<Value, String> {
    use nyx_config::Layer;
    if !nyx_config::valid_domain(domain) {
        return Err("not a configuration domain".into());
    }
    let layers = nyx_config::Layers::standard();
    let at = |layer: Layer| -> Result<Option<toml::Value>, String> {
        let t = layers.read(layer, domain).map_err(|e| format!("{e:#}"))?;
        Ok(key.split('.').try_fold(&toml::Value::Table(t.clone()), |v, k| v.get(k)).cloned())
    };
    let user = at(Layer::User)?;
    let default = at(Layer::System)?.or(at(Layer::Defaults)?);
    let source = match (&user, at(Layer::System)?, at(Layer::Defaults)?) {
        (Some(_), ..) => "yours",
        (None, Some(_), _) => "administrator's",
        (None, None, Some(_)) => "NyxOS default",
        _ => "the program's own",
    };
    let j = |v: &Option<toml::Value>| v.as_ref().and_then(|v| serde_json::to_value(v).ok()).unwrap_or(Value::Null);
    Ok(json!({"value": j(&user.clone().or(default.clone())), "source": source, "default": j(&default), "yours": j(&user)}))
}

#[cfg(test)]
mod description_tests {
    #[test]
    fn the_description_is_api_2() {
        let d = nyx_actions::Description::from_value(&super::description(crate::APP_ID), None).unwrap();
        assert!(d.action("devices.send").unwrap().asks(), "sending leaves this computer");
        assert!(!d.action("devices.ring").unwrap().asks());
        assert!(d.actions.iter().filter(|a| a.id.starts_with("settings.") && a.id != "settings.open").all(|a| a.risk == nyx_actions::Risk::ReadOnly));
    }
}
