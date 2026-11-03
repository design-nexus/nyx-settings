//! Apps that draw their own titlebar (with window controls). On a floating workspace,
//! Omarchy gives every window a titlebar of its own (its hyprbars plugin), so these
//! apps show two; a `hyprbars:no_bar` window rule leaves Omarchy's off for them.
//!
//! The plugin defines that rule, and Omarchy loads the plugin only once something
//! floats, so the rule is added when the plugin is there (now, or when a window opens
//! or its rules are looked at again later), and a failure never breaks the config.

use super::lua::quote;
use super::profiles::class_pattern;
use super::state::State;
use crate::{cmd, paths};
use anyhow::{Context, Result};
use gtk::gio::prelude::*;
use nyx_desktop::apps::Index;

/// Apps known to draw their own titlebar, used until the list is changed: ChatGPT and
/// the Chromium-based browsers. Only installed ones count.
pub const DEFAULTS: &[&str] = &["chatgpt", "chromium", "google-chrome", "brave-browser", "microsoft-edge", "vivaldi-stable"];

fn installed(id: &str) -> bool {
    let file = format!("{id}.desktop");
    gtk::gio::AppInfo::all().iter().any(|a| a.id().is_some_and(|i| i == file.as_str()))
}

/// The apps (desktop ids, or `class:<class>`) that get no second titlebar.
pub fn apps(state: &State) -> Vec<String> {
    match &state.own_titlebar_apps {
        Some(list) => list.clone(),
        None => DEFAULTS.iter().filter(|id| installed(id)).map(|s| s.to_string()).collect(),
    }
}

/// The Lua for `apps`, or nothing when the list is empty.
pub fn lua(apps: &[String], index: &Index) -> String {
    let patterns: Vec<String> = apps.iter().filter_map(|a| class_pattern(a, index)).collect();
    if patterns.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\n-- Apps with their own titlebar: no second one on floating workspaces\n\
         local nyx_own_titlebar = {\n",
    );
    for p in &patterns {
        out.push_str(&format!("  {},\n", quote(p)));
    }
    out.push_str(
        "}\n\
         local nyx_own_titlebar_set = false\n\
         local function nyx_own_titlebar_rules()\n\
         \x20 if nyx_own_titlebar_set or not (hl.plugin and hl.plugin.hyprbars) then return end\n\
         \x20 nyx_own_titlebar_set = true\n\
         \x20 for _, class in ipairs(nyx_own_titlebar) do\n\
         \x20   pcall(hl.window_rule, { match = { class = class }, [\"hyprbars:no_bar\"] = true })\n\
         \x20 end\n\
         end\n\
         nyx_own_titlebar_rules()\n\
         hl.on(\"window.open\", nyx_own_titlebar_rules)\n\
         hl.on(\"window.update_rules\", nyx_own_titlebar_rules)\n",
    );
    out
}

/// Add `app` to the list (or take it off), write Hyprland's settings and reload.
/// For `nyx-settings --own-titlebar APP [off]`, run from the Switcher's overview.
pub fn set(app: &str, on: bool) -> Result<String> {
    let path = paths::state_file();
    let mut state = State::load(&path);
    let mut list = apps(&state);
    let app = app.trim().trim_end_matches(".desktop").to_string();
    if app.is_empty() {
        anyhow::bail!("no app given");
    }
    list.retain(|a| !a.eq_ignore_ascii_case(&app));
    if on {
        list.push(app.clone());
    }
    state.own_titlebar_apps = Some(list);
    super::hypr::write(&state)?;
    let text = serde_json::to_string_pretty(&state)?;
    cmd::atomic_write(&path, &text).context("save Settings' state")?;
    let _ = std::process::Command::new("hyprctl").arg("reload").output();
    Ok(if on { format!("{app} keeps only its own titlebar") } else { format!("{app} gets Omarchy's titlebar again") })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nyx_desktop::apps::Entry;

    #[test]
    fn rules_only_once_the_plugin_is_there() {
        let index = Index::new(&[Entry { id: "chatgpt".into(), exec: "chatgpt".into(), ..Default::default() }]);
        let lua = lua(&["chatgpt".into(), "class:Kitty".into()], &index);
        assert!(lua.contains("\"(?i)^(chatgpt)$\","));
        assert!(lua.contains("\"(?i)^(kitty)$\","));
        assert!(lua.contains("pcall(hl.window_rule, { match = { class = class }, [\"hyprbars:no_bar\"] = true })"));
        assert!(lua.contains("not (hl.plugin and hl.plugin.hyprbars)"));
        assert!(super::lua(&[], &index).is_empty());
    }
}
