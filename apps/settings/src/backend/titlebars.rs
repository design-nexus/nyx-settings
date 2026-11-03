//! Apps that draw their own titlebar (with window controls). On a floating workspace,
//! Omarchy gives every window a titlebar of its own (its hyprbars plugin), so these
//! apps show two; a `hyprbars:no_bar` window rule leaves Omarchy's off for them.
//! Nyx apps tag their windows `nyx-own-controls` once they draw controls, and get the
//! same rule; the listed apps get the tag from a window rule, so the dock's hover
//! buttons leave them alone too. With hover buttons chosen, no window on a floating
//! workspace gets
//! Omarchy's titlebar: the Nyx Dock shows small buttons over the focused one instead.
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

/// Apps known to draw their own titlebar, used until the list is changed: ChatGPT,
/// and the Chromium- and Firefox-based browsers. Only installed ones count.
pub const DEFAULTS: &[&str] = &[
    "chatgpt",
    "chromium",
    "google-chrome",
    "brave-browser",
    "microsoft-edge",
    "vivaldi-stable",
    "firefox",
    "zen",
];

/// The tag Nyx apps put on windows that draw their own window controls.
pub const OWN_CONTROLS_TAG: &str = "nyx-own-controls";

/// The tag Omarchy puts on every window on a floating workspace.
pub const FLOATING_WORKSPACE_TAG: &str = "omarchy-floating-workspace";

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

/// The Lua for `apps` and Nyx's own windows; with `hover`, for every window on a
/// floating workspace.
pub fn lua(apps: &[String], index: &Index, hover: bool) -> String {
    let patterns: Vec<String> = apps.iter().filter_map(|a| class_pattern(a, index)).collect();
    let mut out = String::from(
        "\n-- Apps with their own titlebar: no second one on floating workspaces\n\
         local nyx_own_titlebar = {\n",
    );
    for p in &patterns {
        out.push_str(&format!("  {},\n", quote(p)));
    }
    out.push_str(&format!(
        "}}\n\
         for _, class in ipairs(nyx_own_titlebar) do\n\
         \x20 pcall(hl.window_rule, {{ match = {{ class = class }}, tag = \"+{OWN_CONTROLS_TAG}\" }})\n\
         end\n\
         local nyx_own_titlebar_tags = {{ {} }}\n\
         local nyx_own_titlebar_set = false\n\
         local function nyx_own_titlebar_rules()\n\
         \x20 if nyx_own_titlebar_set or not (hl.plugin and hl.plugin.hyprbars) then return end\n\
         \x20 nyx_own_titlebar_set = true\n\
         \x20 for _, tag in ipairs(nyx_own_titlebar_tags) do\n\
         \x20   pcall(hl.window_rule, {{ match = {{ tag = tag }}, [\"hyprbars:no_bar\"] = true }})\n\
         \x20 end\n\
         \x20 for _, class in ipairs(nyx_own_titlebar) do\n\
         \x20   pcall(hl.window_rule, {{ match = {{ class = class }}, [\"hyprbars:no_bar\"] = true }})\n\
         \x20 end\n\
         end\n\
         nyx_own_titlebar_rules()\n\
         hl.on(\"window.open\", nyx_own_titlebar_rules)\n\
         hl.on(\"window.update_rules\", nyx_own_titlebar_rules)\n",
        tags(hover).iter().map(|t| quote(t)).collect::<Vec<_>>().join(", ")
    ));
    out
}

fn tags(hover: bool) -> Vec<&'static str> {
    if hover { vec![OWN_CONTROLS_TAG, FLOATING_WORKSPACE_TAG] } else { vec![OWN_CONTROLS_TAG] }
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
        let lua = lua(&["chatgpt".into(), "class:Kitty".into()], &index, false);
        assert!(lua.contains("\"(?i)^(chatgpt)$\","));
        assert!(lua.contains("\"(?i)^(kitty)$\","));
        assert!(lua.contains("pcall(hl.window_rule, { match = { class = class }, [\"hyprbars:no_bar\"] = true })"));
        assert!(lua.contains("not (hl.plugin and hl.plugin.hyprbars)"));
        assert!(lua.contains("pcall(hl.window_rule, { match = { class = class }, tag = \"+nyx-own-controls\" })"));
    }

    #[test]
    fn nyx_windows_always_and_every_window_with_hover() {
        let index = Index::new(&[]);
        let plain = lua(&[], &index, false);
        assert!(plain.contains("local nyx_own_titlebar_tags = { \"nyx-own-controls\" }"));
        assert!(plain.contains("pcall(hl.window_rule, { match = { tag = tag }, [\"hyprbars:no_bar\"] = true })"));
        let hover = lua(&[], &index, true);
        assert!(hover.contains("{ \"nyx-own-controls\", \"omarchy-floating-workspace\" }"));
    }
}
