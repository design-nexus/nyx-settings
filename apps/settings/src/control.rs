//! Settings' tools on the control interface (`nyx_core::control`), for Nyx Assistant:
//! which pages there are, and showing one. Read-only by design: nothing here changes a
//! setting (computer use can't reach Settings either).

use crate::{sections, window};
use nyx_core::control::text;
use serde_json::{Value, json};

pub fn tools() -> Value {
    json!([
        {"name": "settings.pages", "description": "The pages of Nyx Settings (id, title, what each covers).", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}}},
        {"name": "settings.open", "description": "Show a page of Nyx Settings (by id or title from settings.pages) for the person to change something themselves.", "inputSchema": {"type": "object", "properties": {"page": {"type": "string"}}, "required": ["page"]}},
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
        _ => Err(format!("unknown tool {name}")),
    }
}
