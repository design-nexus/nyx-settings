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
    ])
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
        _ => Err(format!("unknown tool {name}")),
    }
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
