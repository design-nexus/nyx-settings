//! Workspace profiles (the format is nyx-desktop's `profiles`): named sets of workspaces
//! with their apps' workspaces. Settings is what applies one: it copies the profile's
//! workspaces into its state, turns its apps into window rules, rewrites
//! `hypr/settings.lua`, reloads Hyprland and moves open windows of those apps.
//!
//! While a profile is active, the Tiling & Workspaces page edits it: each save of the
//! state writes the workspaces back into the active profile.

use super::state::{Rule, State, WorkspaceRule};
use crate::{cmd, paths};
use anyhow::{Context, Result, anyhow};
use nyx_desktop::apps::Index;
use nyx_desktop::profiles::{self, Profile, Store, Workspace};

pub fn store() -> Store {
    Store::user()
}

/// Settings' workspaces into `profile` (its other keys, such as a later wallpaper,
/// stay as they are).
pub fn from_state(state: &State, profile: &mut Profile) {
    profile.workspaces = state.persistent_workspaces;
    let old = std::mem::take(&mut profile.workspace_list);
    let mut list: Vec<Workspace> = state
        .workspaces
        .iter()
        .map(|r| Workspace {
            id: r.workspace,
            name: r.name.clone(),
            monitor: r.monitor.clone(),
            mode: r.mode.clone(),
            gaps: r.gaps.map(|(a, b)| [a, b]),
            no_border: r.no_border,
            no_rounding: r.no_rounding,
            no_shadow: r.no_shadow,
            extra: old.iter().find(|w| w.id == r.workspace).map(|w| w.extra.clone()).unwrap_or_default(),
        })
        .collect();
    for w in old {
        if !w.extra.is_empty() && !list.iter().any(|l| l.id == w.id) {
            list.push(Workspace { id: w.id, extra: w.extra, ..Default::default() });
        }
    }
    list.sort_by_key(|w| w.id);
    profile.workspace_list = list;
}

/// `profile` into Settings' state: its workspaces and its apps' window rules.
pub fn to_state(profile: &Profile, state: &mut State, index: &Index) {
    state.persistent_workspaces = profile.workspaces;
    state.workspaces = profile
        .workspace_list
        .iter()
        .map(|w| WorkspaceRule {
            workspace: w.id,
            monitor: w.monitor.clone(),
            name: w.name.clone(),
            mode: w.mode.clone(),
            gaps: w.gaps.map(|[a, b]| (a, b)),
            no_border: w.no_border,
            no_rounding: w.no_rounding,
            no_shadow: w.no_shadow,
        })
        .filter(|r| !r.is_empty())
        .collect();
    state.profile_rules = placement_rules(profile, index);
}

/// A window rule per app: its windows open on its workspace (without taking you there).
/// Classes are matched ignoring case, since apps and desktop files disagree on it.
pub fn placement_rules(profile: &Profile, index: &Index) -> Vec<Rule> {
    profile
        .apps
        .iter()
        .filter_map(|a| {
            let mut classes = match a.app.strip_prefix("class:") {
                Some(class) => vec![class.to_lowercase()],
                None => index.classes_of(&a.app),
            };
            if classes.is_empty() {
                classes.push(a.app.to_lowercase());
            }
            // Rule text can't hold quotes or backslashes; a '.' left unescaped matches
            // itself (and anything else, which is harmless here).
            classes.retain(|c| !c.is_empty() && c.chars().all(|ch| !ch.is_control() && !"\"\\()|".contains(ch)));
            (!classes.is_empty()).then(|| Rule {
                class: format!("(?i)^({})$", classes.join("|")),
                title: String::new(),
                effect: "workspace".into(),
                value: format!("{} silent", a.workspace),
            })
        })
        .collect()
}

/// The first time profiles are used, today's setup becomes "Default" (and active), so
/// switching away and back loses nothing. Returns the active profile's id.
pub fn ensure_default(state: &State) -> Result<String> {
    let store = store();
    if let Some(id) = store.active() {
        return Ok(id);
    }
    // No profile is active (none yet, or the active one was removed): what's set now is
    // kept as Default before anything replaces it.
    let mut p = Profile::new(&store.new_id("Default"), "Default");
    from_state(state, &mut p);
    store.save(&p).map_err(|e| anyhow!(e))?;
    store.set_active(&p.id).map_err(|e| anyhow!(e))?;
    Ok(p.id)
}

/// Write Settings' workspaces back into the active profile (after each save).
pub fn sync_active(state: &State) {
    let store = store();
    let Some(id) = store.active() else { return };
    let Ok(mut p) = store.load(&id) else { return };
    let before = p.clone();
    from_state(state, &mut p);
    if p != before
        && let Err(e) = store.save(&p)
    {
        eprintln!("nyx-settings: workspace profile {id}: {e}");
    }
}

/// Switch to the profile `id_or_name` (an id, a name, or `next` / `previous`): apply it, make it active, and
/// move open windows of its apps when it says so. Works without the Settings window
/// (`nyx-settings --workspace-profile NAME`).
pub fn switch(id_or_name: &str) -> Result<String> {
    let store = store();
    let target = match id_or_name {
        // Cycle through the profiles in name order.
        "next" | "previous" => {
            let list = store.list();
            if list.len() < 2 {
                return Err(anyhow!("there's no other workspace profile to switch to"));
            }
            let here = store.active().and_then(|a| list.iter().position(|p| p.id == a)).unwrap_or(0);
            let n = list.len();
            let i = if id_or_name == "next" { (here + 1) % n } else { (here + n - 1) % n };
            list[i].clone()
        }
        _ => store.find(id_or_name).ok_or_else(|| anyhow!("there's no workspace profile called “{id_or_name}”"))?,
    };
    let path = paths::state_file();
    let mut state = State::load(&path);
    // Keep the current setup in the profile that's active now.
    let active = ensure_default(&state)?;
    if active != target.id {
        sync_active(&state);
    }
    let index = nyx_desktop::apps::from_gio();
    to_state(&target, &mut state, &index);
    // Hyprland's file first: if it can't be written, nothing has changed.
    super::hypr::write(&state)?;
    let text = serde_json::to_string_pretty(&state)?;
    cmd::atomic_write(&path, &text).context("save Settings' state")?;
    store.set_active(&target.id).map_err(|e| anyhow!(e))?;
    // Reload first (and wait), so the rules are in before windows move.
    let _ = std::process::Command::new("hyprctl").arg("reload").output();
    let mut moved = 0;
    if target.move_open_windows
        && let Ok(hypr) = nyx_desktop::hypr::Instance::find()
        && let Ok(desktop) = nyx_desktop::Desktop::snapshot(&hypr)
    {
        for (address, ws) in profiles::moves(&target, &desktop, &index) {
            if nyx_desktop::actions::move_to_workspace(&address, &ws.to_string(), false).run_on(&hypr) {
                moved += 1;
            }
        }
    }
    Ok(match moved {
        0 => format!("{} is the workspace profile", target.name),
        1 => format!("{} is the workspace profile; 1 window moved", target.name),
        n => format!("{} is the workspace profile; {n} windows moved", target.name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nyx_desktop::apps::Entry;
    use nyx_desktop::profiles::App;

    #[test]
    fn state_and_profile_round_trip() {
        let mut state = State {
            persistent_workspaces: 4,
            workspaces: vec![WorkspaceRule { workspace: 2, name: Some("Mail".into()), gaps: Some((3, 6)), ..Default::default() }],
            ..Default::default()
        };
        let mut p = Profile::new("work", "Work");
        let mut wall = toml::Table::new();
        wall.insert("wallpaper".into(), toml::Value::String("w.jpg".into()));
        p.workspace_list = vec![Workspace { id: 2, extra: wall.clone(), ..Default::default() }, Workspace { id: 7, extra: wall, ..Default::default() }];
        from_state(&state, &mut p);
        assert_eq!(p.workspaces, 4);
        assert_eq!(p.workspace(2).unwrap().name.as_deref(), Some("Mail"));
        assert!(p.workspace(2).unwrap().extra.contains_key("wallpaper"), "kept");
        assert!(p.workspace(7).is_some(), "a workspace with only a wallpaper stays");
        let mut back = State::default();
        to_state(&p, &mut back, &Index::default());
        assert_eq!(back.persistent_workspaces, 4);
        assert_eq!(back.workspaces, state.workspaces, "workspace 7 has nothing for Hyprland");
        state.workspaces.clear();
    }

    #[test]
    fn apps_become_window_rules() {
        let index = Index::new(&[Entry { id: "org.mozilla.firefox".into(), exec: "firefox %u".into(), ..Default::default() }]);
        let mut p = Profile::new("work", "Work");
        p.apps = vec![
            App { app: "org.mozilla.firefox".into(), workspace: 3 },
            App { app: "class:Kitty".into(), workspace: 1 },
            App { app: "bad\"one".into(), workspace: 1 },
        ];
        let rules = placement_rules(&p, &index);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].class, "(?i)^(firefox|org.mozilla.firefox)$");
        assert_eq!((rules[0].effect.as_str(), rules[0].value.as_str()), ("workspace", "3 silent"));
        assert_eq!(rules[1].class, "(?i)^(kitty)$");
        let line = super::super::rules::lua_line(false, &rules[0]).unwrap();
        assert_eq!(line, "hl.window_rule({ match = { class = \"(?i)^(firefox|org.mozilla.firefox)$\" }, workspace = \"3 silent\" })");
    }
}
