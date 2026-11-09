//! Trackpad swipe/pinch gestures, emitted as native `hl.gesture` calls.
//!
//! Hyprland only sees libinput swipe gestures with three or more fingers (two
//! fingers are scrolling), and it cannot unbind a gesture once defined, so the
//! managed file is the single source of every gesture.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::lua::quote;

/// (id, label, native Hyprland action or None when it runs a command)
pub const ACTIONS: &[(&str, &str)] = &[
    ("none", "Nothing"),
    ("workspace-follow", "Switch workspace (follows fingers)"),
    ("next-workspace", "Next workspace"),
    ("previous-workspace", "Previous workspace"),
    ("last-workspace", "Last used workspace"),
    ("special", "Toggle scratchpad"),
    ("fullscreen", "Fullscreen window"),
    ("maximize", "Maximize window"),
    ("float", "Float / tile window"),
    ("close", "Close window"),
    ("move", "Move window (follows fingers)"),
    ("resize", "Resize window (follows fingers)"),
    ("menu", "Omarchy menu"),
    ("apps", "App launcher"),
    ("glimpse", "Glimpse"),
    ("notifications", "Notification history"),
    ("screenshot", "Screenshot"),
    ("volume-up", "Volume up"),
    ("volume-down", "Volume down"),
    ("brightness-up", "Brightness up"),
    ("brightness-down", "Brightness down"),
    ("lock", "Lock screen"),
    ("app", "Open an app…"),
    ("command", "Run a command…"),
];

/// Actions that only make sense on the horizontal axis.
pub fn horizontal_only(id: &str) -> bool {
    id == "workspace-follow"
}

pub const DIRECTIONS: &[&str] = &["left", "right", "up", "down", "pinchin", "pinchout"];

pub fn direction_label(dir: &str) -> &'static str {
    match dir {
        "left" => "Swipe left",
        "right" => "Swipe right",
        "up" => "Swipe up",
        "down" => "Swipe down",
        "pinchin" => "Pinch in",
        "pinchout" => "Pinch out",
        _ => "",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[derive(Default)]
pub struct FingerSet {
    /// direction -> action id
    pub actions: BTreeMap<String, String>,
    /// direction -> shell command, used when the action is `command`
    pub commands: BTreeMap<String, String>,
    /// direction -> desktop entry id (`firefox.desktop`), used when the action is `app`
    pub apps: BTreeMap<String, String>,
    pub reverse_horizontal: bool,
    pub reverse_vertical: bool,
}

impl FingerSet {
    pub fn action(&self, dir: &str) -> &str {
        self.actions.get(dir).map(String::as_str).unwrap_or("none")
    }

    fn with(pairs: &[(&str, &str)]) -> Self {
        Self { actions: pairs.iter().map(|(d, a)| (d.to_string(), a.to_string())).collect(), ..Self::default() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Gestures {
    /// Settings writes no gestures at all until this is on, leaving Omarchy's
    /// (and the user's own) gestures alone.
    pub enabled: bool,
    pub three: FingerSet,
    pub four: FingerSet,
    pub swipe_distance: Option<i64>,
    pub create_new_workspace: Option<bool>,
    pub swipe_forever: Option<bool>,
    /// Workspace swipes wrap around: past the last workspace comes the first.
    pub loop_workspaces: bool,
    /// Workspaces 1 to this always exist (like the bar shows them), so swipes
    /// reach empty ones too. With Hyprland's native wrap the loop also takes in
    /// any higher workspace that's open; the fallback wraps around 1..N only.
    pub loop_count: u32,
}

impl Default for Gestures {
    fn default() -> Self {
        Self {
            enabled: false,
            three: FingerSet::with(&[
                ("left", "workspace-follow"),
                ("right", "workspace-follow"),
                ("up", "apps"),
                ("down", "special"),
            ]),
            four: FingerSet::with(&[
                // Same sense as the three-finger swipe: fingers left, next workspace.
                ("left", "next-workspace"),
                ("right", "previous-workspace"),
                ("up", "fullscreen"),
                ("down", "close"),
                ("pinchin", "float"),
            ]),
            swipe_distance: None,
            create_new_workspace: None,
            swipe_forever: None,
            loop_workspaces: false,
            loop_count: 5,
        }
    }
}

fn exec(cmd: &str) -> String {
    format!("function() hl.exec_cmd({}) end", quote(cmd))
}

fn dispatch(dsp: &str) -> String {
    format!("function() hl.dispatch({dsp}) end")
}

/// Name of the Lua helper that steps through workspaces 1..N with wrap-around.
const STEP_FN: &str = "settings_workspace_step";

/// Whether the running Hyprland wraps workspace swipes itself
/// (`gestures:workspace_swipe_wrap`, from the Nyx patch). `hl.get_config` returns
/// nil for an option it doesn't know, where `hl.config` would show a config error.
const NATIVE_WRAP: &str = "settings_native_wrap";

fn native_wrap_check() -> String {
    format!("local {NATIVE_WRAP} = hl.get_config(\"gestures.workspace_swipe_wrap\") ~= nil")
}

/// Defined once, before any gesture that uses it. With native wrap it steps
/// through whatever workspaces the monitor has (`m+1` wraps); otherwise, off the
/// ends of 1..N (on a higher-numbered workspace) the next step lands on 1, the
/// previous on N.
fn step_helper() -> String {
    format!(
        "local function {STEP_FN}(delta, count)\n\
         \x20 if {NATIVE_WRAP} then\n\
         \x20   hl.dispatch(hl.dsp.focus({{ workspace = delta > 0 and \"m+1\" or \"m-1\" }}))\n\
         \x20   return\n\
         \x20 end\n\
         \x20 local ws = hl.get_active_workspace()\n\
         \x20 local id = ws and ws.id or 1\n\
         \x20 local target\n\
         \x20 if id < 1 or id > count then\n\
         \x20   target = delta > 0 and 1 or count\n\
         \x20 else\n\
         \x20   target = (id - 1 + delta) % count + 1\n\
         \x20 end\n\
         \x20 hl.dispatch(hl.dsp.focus({{ workspace = tostring(target) }}))\n\
         end"
    )
}

/// Desktop entry ids are limited to these characters, so one can go into a shell
/// command unquoted.
fn valid_app_id(id: &str) -> bool {
    id.ends_with(".desktop") && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// The `action = …` part (plus any extra fields) for one action id, or None.
/// `arg` is the command for `command` and the desktop entry id for `app`.
/// `loop_count` makes next/previous workspace wrap around 1..N.
fn action_lua(action: &str, arg: Option<&str>, loop_count: Option<u32>) -> Option<String> {
    if let Some(n) = loop_count {
        match action {
            "next-workspace" => return Some(format!("action = function() {STEP_FN}(1, {n}) end")),
            "previous-workspace" => return Some(format!("action = function() {STEP_FN}(-1, {n}) end")),
            _ => {}
        }
    }
    let body = match action {
        "none" => return None,
        "workspace-follow" => "action = \"workspace\"".to_string(),
        "special" => "action = \"special\", workspace_name = \"scratchpad\"".to_string(),
        "fullscreen" => "action = \"fullscreen\"".to_string(),
        "float" => "action = \"float\"".to_string(),
        "close" => "action = \"close\"".to_string(),
        "move" => "action = \"move\"".to_string(),
        "resize" => "action = \"resize\"".to_string(),
        "maximize" => format!("action = {}", dispatch("hl.dsp.window.fullscreen({ mode = \"maximized\" })")),
        "next-workspace" => format!("action = {}", dispatch("hl.dsp.focus({ workspace = \"e+1\" })")),
        "previous-workspace" => format!("action = {}", dispatch("hl.dsp.focus({ workspace = \"e-1\" })")),
        "last-workspace" => format!("action = {}", dispatch("hl.dsp.focus({ workspace = \"previous\" })")),
        "menu" => format!("action = {}", exec("omarchy-menu toggle root")),
        "apps" => format!("action = {}", exec("omarchy-menu toggle apps")),
        "notifications" => format!("action = {}", exec("omarchy-shell notifications showHistory")),
        "screenshot" => format!("action = {}", exec("omarchy-capture-screenshot")),
        "volume-up" => format!("action = {}", exec("omarchy-audio-output-volume raise")),
        "volume-down" => format!("action = {}", exec("omarchy-audio-output-volume lower")),
        "brightness-up" => format!("action = {}", exec("omarchy-brightness-display +5%")),
        "brightness-down" => format!("action = {}", exec("omarchy-brightness-display 5%-")),
        "lock" => format!("action = {}", exec("omarchy-system-lock")),
        "glimpse" => format!("action = {}", exec("nyx-glimpse --toggle")),
        "app" => {
            let id = arg.map(str::trim).filter(|id| valid_app_id(id))?;
            format!("action = {}", exec(&format!("uwsm-app -- {id}")))
        }
        "command" => {
            let cmd = arg.map(str::trim).filter(|c| !c.is_empty())?;
            format!("action = {}", exec(cmd))
        }
        _ => return None,
    };
    Some(body)
}

/// The physical direction a configured direction fires on, after reversal.
pub fn physical(set: &FingerSet, dir: &str) -> &'static str {
    match (dir, set.reverse_horizontal, set.reverse_vertical) {
        ("left", true, _) => "right",
        ("right", true, _) => "left",
        ("up", _, true) => "down",
        ("down", _, true) => "up",
        ("left", ..) => "left",
        ("right", ..) => "right",
        ("up", ..) => "up",
        ("down", ..) => "down",
        ("pinchin", ..) => "pinchin",
        ("pinchout", ..) => "pinchout",
        _ => "left",
    }
}

/// Fallback for a Hyprland without native wrap: placeholder workspaces either
/// side of 1..N. Stock Hyprland's smooth swipe can't wrap, but it slides into
/// any workspace that exists, so these give it somewhere to go past each end;
/// arriving on one jumps to the other end of the loop. The bar only shows
/// workspaces 1-10, so neither appears there. A named workspace gets a negative
/// id, which puts it before 1.
pub const LOOP_START: &str = "settings-loop-start";
pub const LOOP_END_ID: u32 = 11;

fn loop_redirect(count: u32) -> String {
    format!(
        "if {NATIVE_WRAP} then\n\
\x20 hl.config({{ gestures = {{ workspace_swipe_wrap = true }} }})\n\
else\n\
\x20 hl.workspace_rule({{ workspace = \"name:{LOOP_START}\", monitor = settings_gesture_monitor, persistent = true }})\n\
\x20 hl.workspace_rule({{ workspace = \"{LOOP_END_ID}\", monitor = settings_gesture_monitor, persistent = true }})\n\
\x20 local settings_loop_last = (hl.get_active_workspace() or {{ id = 1 }}).id\n\
\x20 hl.on(\"workspace.active\", function()\n\
\x20   local ws = hl.get_active_workspace()\n\
\x20   if not ws then return end\n\
\x20   local target\n\
\x20   if ws.name == \"{LOOP_END_ID}\" then\n\
\x20     -- Past the last: normally wrap to 1 (or to the last when backing out of 1).\n\
\x20     target = settings_loop_last == 1 and {count} or 1\n\
\x20   elseif ws.name == \"{LOOP_START}\" then\n\
\x20     target = settings_loop_last == {count} and 1 or {count}\n\
\x20   else\n\
\x20     settings_loop_last = ws.id\n\
\x20     return\n\
\x20   end\n\
\x20   -- Just after the swipe finishes, never while it's still running.\n\
\x20   hl.timer(function() hl.dispatch(hl.dsp.focus({{ workspace = tostring(target) }})) end, {{ timeout = 30, type = \"oneshot\" }})\n\
\x20 end)\n\
end"
    )
}

fn set_lines(fingers: u8, set: &FingerSet, loop_count: Option<u32>, out: &mut Vec<String>) -> Option<bool> {
    let mut follow_invert = None;
    let follow = set.action("left") == "workspace-follow" || set.action("right") == "workspace-follow";
    if follow {
        out.push(format!("hl.gesture({{ fingers = {fingers}, direction = \"horizontal\", action = \"workspace\" }})"));
        follow_invert = Some(set.reverse_horizontal);
    }
    for dir in DIRECTIONS {
        if follow && (*dir == "left" || *dir == "right") {
            continue;
        }
        let action = set.action(dir);
        if horizontal_only(action) {
            continue;
        }
        let arg = if action == "app" { set.apps.get(*dir) } else { set.commands.get(*dir) };
        if let Some(body) = action_lua(action, arg.map(String::as_str), loop_count) {
            let direction = physical(set, dir);
            out.push(format!("hl.gesture({{ fingers = {fingers}, direction = \"{direction}\", {body} }})"));
        }
    }
    follow_invert
}

/// Gestures keep workspaces 1..=N alive; this is N (0: none).
pub fn persistent_upto(g: &Gestures) -> u32 {
    if g.enabled { g.loop_count.clamp(2, 10) } else { 0 }
}

/// The gestures NyxOS ships (`[[gestures]]` in `/etc/nyxos/defaults/hyprland.toml`), as
/// fingers and direction; one tied to a Nyx app counts only when the app is installed.
/// Empty on Omarchy.
pub fn shipped() -> Vec<(u8, String)> {
    let dir = std::env::var_os("NYX_CONFIG_DEFAULTS").map(std::path::PathBuf::from).unwrap_or_else(|| "/etc/nyxos/defaults".into());
    let Ok(text) = std::fs::read_to_string(dir.join("hyprland.toml")) else { return Vec::new() };
    let Ok(table) = text.parse::<toml::Table>() else { return Vec::new() };
    let Some(list) = table.get("gestures").and_then(toml::Value::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|g| {
            let fingers = u8::try_from(g.get("fingers")?.as_integer()?).ok()?;
            let direction = g.get("direction")?.as_str()?.to_string();
            let installed = g.get("app").and_then(toml::Value::as_str).is_none_or(|app| crate::cmd::present(&format!("nyx-{app}")));
            installed.then_some((fingers, direction))
        })
        .collect()
}

/// Whether Hyprland would refuse `ours` because `theirs` (on the same fingers) is set.
fn shadows(theirs: &str, ours: &str) -> bool {
    let axis = |d: &str| match d {
        "left" | "right" | "horizontal" => Some("horizontal"),
        "up" | "down" | "vertical" => Some("vertical"),
        _ => None,
    };
    theirs == ours
        || (theirs == "horizontal" || theirs == "vertical" || ours == "horizontal" || ours == "vertical")
            && axis(theirs).is_some()
            && axis(theirs) == axis(ours)
}

/// Unset each shipped gesture one of `lines` would be shadowed by, before the first of them.
fn unset_shipped(lines: &mut Vec<String>, shipped: &[(u8, String)]) {
    let ours: Vec<(u8, String)> = lines
        .iter()
        .filter_map(|l| {
            let rest = l.strip_prefix("hl.gesture({ fingers = ")?;
            let (fingers, rest) = rest.split_once(", direction = \"")?;
            let (direction, _) = rest.split_once('"')?;
            Some((fingers.parse().ok()?, direction.to_string()))
        })
        .collect();
    let mut unset: Vec<String> = Vec::new();
    for (f, d) in shipped {
        let line = format!("hl.gesture({{ fingers = {f}, direction = \"{d}\", action = \"unset\" }})");
        if ours.iter().any(|(of, od)| of == f && shadows(d, od)) && !unset.contains(&line) {
            unset.push(line);
        }
    }
    if let Some(at) = lines.iter().position(|l| l.starts_with("hl.gesture(")) {
        lines.splice(at..at, unset);
    }
}

/// Lua lines for every configured gesture, plus the gesture options.
pub fn lua(g: &Gestures) -> Vec<String> {
    let mut out = lua_over(g);
    unset_shipped(&mut out, &shipped());
    out
}

fn lua_over(g: &Gestures) -> Vec<String> {
    let mut out = Vec::new();
    if !g.enabled {
        return out;
    }
    let count = g.loop_count.clamp(2, 10);
    let loop_count = g.loop_workspaces.then_some(count);
    // Omarchy can add a virtual display. Keep the swipe path on the laptop
    // panel instead of letting empty persistent workspaces land on HEADLESS-1.
    out.push("local settings_gesture_monitor = (function() local active = hl.get_active_monitor(); for _, monitor in ipairs(hl.get_monitors()) do if monitor.name:match(\"^eDP\") or monitor.name:match(\"^LVDS\") or monitor.name:match(\"^DSI\") then return monitor.name end end; return active and active.name or nil end)()".into());
    if loop_count.is_some() {
        out.push(native_wrap_check());
    }
    // Keep 1..N alive even when empty. Hyprland removes empty workspaces, and a
    // swipe only moves between workspaces that exist, so without this it would
    // skip straight over the empty ones the bar still shows.
    for id in 1..=count {
        out.push(format!("hl.workspace_rule({{ workspace = \"{id}\", monitor = settings_gesture_monitor, persistent = true }})"));
    }
    let helper_at = out.len();
    let invert3 = set_lines(3, &g.three, loop_count, &mut out);
    let invert4 = set_lines(4, &g.four, loop_count, &mut out);
    let smooth_loop = loop_count.is_some() && (invert3.is_some() || invert4.is_some());
    if smooth_loop {
        out.push(loop_redirect(count));
    }
    if out.iter().any(|l| l.contains(STEP_FN)) {
        out.insert(helper_at, step_helper());
    }
    if smooth_loop || out.iter().any(|l| l.contains(STEP_FN)) {
        // Slide in from the correct side when wrapping from the last to the first.
        out.push("hl.config({ animations = { workspace_wraparound = true } })".into());
    }
    let mut opts = Vec::new();
    // Hyprland's default is inverted (content follows the fingers, like a
    // phone); "reverse" flips that. The option is global, so three fingers win.
    if let Some(reverse) = invert3.or(invert4) {
        opts.push(format!("workspace_swipe_invert = {}", !reverse));
    }
    if let Some(d) = g.swipe_distance {
        opts.push(format!("workspace_swipe_distance = {d}"));
    }
    // Off unless asked for: swiping past the last workspace would otherwise
    // open ones the bar doesn't show.
    opts.push(format!("workspace_swipe_create_new = {}", g.create_new_workspace.unwrap_or(false)));
    if let Some(v) = g.swipe_forever {
        opts.push(format!("workspace_swipe_forever = {v}"));
    }
    if !opts.is_empty() {
        out.push(format!("hl.config({{ gestures = {{ {} }} }})", opts.join(", ")));
    }
    out
}

const OFF_PREFIX: &str = "-- (off: gestures are managed by Settings) ";

fn is_gesture_line(line: &str) -> bool {
    let t = line.trim_start();
    !t.starts_with("--") && t.starts_with("hl.gesture(")
}

/// Other Hyprland files that define gestures. They load before Settings' file,
/// so theirs win ("Gesture will be overshadowed").
pub fn foreign_sources() -> Vec<(std::path::PathBuf, usize)> {
    let Ok(entries) = std::fs::read_dir(crate::paths::hypr_dir()) else { return vec![] };
    let mut out: Vec<(std::path::PathBuf, usize)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "lua") && *p != crate::paths::managed_lua())
        .filter_map(|p| {
            let n = std::fs::read_to_string(&p).ok()?.lines().filter(|l| is_gesture_line(l)).count();
            (n > 0).then_some((p, n))
        })
        .collect();
    out.sort();
    out
}

pub fn disable_in(text: &str) -> String {
    let mut out: Vec<String> =
        text.lines().map(|l| if is_gesture_line(l) { format!("{OFF_PREFIX}{l}") } else { l.to_string() }).collect();
    if text.ends_with('\n') {
        out.push(String::new());
    }
    out.join("\n")
}

/// Comment out gestures in the other files (each backed up first).
pub fn disable_foreign() -> anyhow::Result<usize> {
    let mut n = 0;
    for (path, count) in foreign_sources() {
        let text = std::fs::read_to_string(&path)?;
        std::fs::write(path.with_extension(format!("lua.settings-bak.{}", super::hypr::timestamp())), &text)?;
        crate::cmd::atomic_write(&path, disable_in(&text))?;
        n += count;
    }
    Ok(n)
}

#[cfg(test)]
mod shipped_tests {
    use super::*;

    #[test]
    fn shipped_gestures_on_the_same_fingers_and_axis_are_unset_first() {
        let mut lines = vec![
            "local x = 1".to_string(),
            "hl.gesture({ fingers = 3, direction = \"horizontal\", action = \"workspace\" })".to_string(),
            "hl.gesture({ fingers = 4, direction = \"left\", action = \"close\" })".to_string(),
            "hl.gesture({ fingers = 4, direction = \"pinchout\", action = \"float\" })".to_string(),
        ];
        let shipped = vec![
            (3, "horizontal".to_string()),
            (3, "up".to_string()),
            (4, "left".to_string()),
            (4, "pinchin".to_string()),
        ];
        unset_shipped(&mut lines, &shipped);
        assert_eq!(lines[0], "local x = 1");
        assert_eq!(lines[1], "hl.gesture({ fingers = 3, direction = \"horizontal\", action = \"unset\" })");
        assert_eq!(lines[2], "hl.gesture({ fingers = 4, direction = \"left\", action = \"unset\" })");
        assert!(lines[3].contains("\"workspace\""));
        assert_eq!(lines.len(), 6, "nothing unset that Settings doesn't replace");
    }

    #[test]
    fn an_axis_shadows_its_directions() {
        assert!(shadows("horizontal", "left"));
        assert!(shadows("right", "horizontal"));
        assert!(!shadows("left", "right"));
        assert!(!shadows("horizontal", "up"));
        assert!(!shadows("pinchin", "pinchout"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_out_only_live_gestures() {
        let text = "a\nhl.gesture({ fingers = 3 })\n-- hl.gesture({ fingers = 4 })\n  hl.gesture({ fingers = 4 })\n";
        let out = disable_in(text);
        assert_eq!(out.matches(OFF_PREFIX).count(), 2);
        assert!(out.ends_with('\n'));
        assert!(out.starts_with("a\n"));
    }

    fn looping() -> Gestures {
        Gestures { enabled: true, loop_workspaces: true, ..Gestures::default() }
    }

    fn line<'a>(lines: &'a [String], fingers: u8, dir: &str) -> &'a str {
        lines
            .iter()
            .find(|l| {
                l.starts_with("hl.gesture(")
                    && l.contains(&format!("fingers = {fingers},"))
                    && l.contains(&format!("direction = \"{dir}\""))
            })
            .map(String::as_str)
            .unwrap_or("")
    }

    #[test]
    fn loop_keeps_one_smooth_swipe_and_adds_placeholders() {
        let lines = lua(&looping());
        // The plain smooth swipe, always; nothing swapped at runtime.
        assert!(line(&lines, 3, "horizontal").contains("action = \"workspace\""));
        assert!(!lines.iter().any(|l| l.contains("action = \"unset\"")));
        let r = lines.iter().find(|l| l.contains("settings-loop-start")).unwrap();
        assert!(r.contains("workspace = \"name:settings-loop-start\", monitor = settings_gesture_monitor, persistent = true"));
        assert!(r.contains("workspace = \"11\", monitor = settings_gesture_monitor, persistent = true"));
        assert!(r.contains("settings_loop_last == 1 and 5 or 1"));
        assert!(r.contains("settings_loop_last == 5 and 1 or 5"));
        assert!(lines.iter().any(|l| l.contains("workspace_wraparound = true")));
        assert!(lines.iter().any(|l| l.contains("workspace_swipe_invert = true")));
    }

    #[test]
    fn loop_prefers_native_wrap() {
        let lines = lua(&looping());
        let check = lines.iter().position(|l| l.contains("local settings_native_wrap = hl.get_config(\"gestures.workspace_swipe_wrap\") ~= nil"));
        let rules = lines.iter().position(|l| l.contains("workspace = \"1\""));
        assert!(check.is_some() && check < rules, "checked before anything uses it");
        let r = lines.iter().find(|l| l.contains("settings-loop-start")).unwrap();
        let native = r.find("workspace_swipe_wrap = true").unwrap();
        let fallback = r.find("else").unwrap();
        assert!(r.starts_with("if settings_native_wrap then") && native < fallback);
        assert!(r[fallback..].contains("name:settings-loop-start"), "placeholders only as the fallback");
        let step = lines.iter().find(|l| l.starts_with("local function settings_workspace_step")).unwrap();
        assert!(step.contains("if settings_native_wrap then") && step.contains("\"m+1\" or \"m-1\""));
    }

    #[test]
    fn no_native_check_without_loop() {
        let lines = lua(&Gestures { enabled: true, ..Gestures::default() });
        assert!(!lines.iter().any(|l| l.contains("settings_native_wrap")));
    }

    #[test]
    fn loop_respects_reverse() {
        let mut g = looping();
        g.three.reverse_horizontal = true;
        let lines = lua(&g);
        assert!(lines.iter().any(|l| l.contains("workspace_swipe_invert = false")));
    }

    #[test]
    fn no_placeholders_without_loop() {
        let lines = lua(&Gestures { enabled: true, ..Gestures::default() });
        assert!(!lines.iter().any(|l| l.contains("settings-loop-start") || l.contains("\"11\"")));
    }

    #[test]
    fn loop_applies_to_next_previous_actions_and_count() {
        let mut g = looping();
        g.loop_count = 8;
        let lines = lua(&g);
        assert!(line(&lines, 4, "left").contains("settings_workspace_step(1, 8)"));
        assert!(line(&lines, 4, "right").contains("settings_workspace_step(-1, 8)"));
    }

    #[test]
    fn no_loop_keeps_plain_steps() {
        let g = Gestures { enabled: true, ..Gestures::default() };
        let lines = lua(&g);
        assert!(!lines.iter().any(|l| l.contains("settings_workspace_step")));
        assert!(line(&lines, 4, "left").contains("e+1"));
    }

    #[test]
    fn keeps_bar_workspaces_and_stops_creating_new_ones() {
        let lines = lua(&Gestures { enabled: true, ..Gestures::default() });
        for id in 1..=5 {
            assert!(lines.iter().any(|l| l
                == &format!(
                    "hl.workspace_rule({{ workspace = \"{id}\", monitor = settings_gesture_monitor, persistent = true }})"
                )));
        }
        assert!(!lines.iter().any(|l| l.contains("workspace = \"6\"")));
        assert!(lines.iter().any(|l| l.contains("workspace_swipe_create_new = false")));
    }

    #[test]
    fn disabled_writes_nothing() {
        assert!(lua(&Gestures::default()).is_empty());
    }

    #[test]
    fn follow_uses_one_horizontal_gesture() {
        let g = Gestures { enabled: true, ..Gestures::default() };
        let lines = lua(&g);
        let horizontal: Vec<_> = lines.iter().filter(|l| l.contains("fingers = 3") && l.contains("horizontal")).collect();
        assert_eq!(horizontal.len(), 1);
        assert!(!lines.iter().any(|l| l.contains("fingers = 3") && l.contains("\"left\"")));
        assert!(lines.iter().any(|l| l.contains("workspace_swipe_invert = true")));
    }

    #[test]
    fn reverse_swaps_directions() {
        let mut g = Gestures { enabled: true, ..Gestures::default() };
        g.four.reverse_horizontal = true;
        g.four.reverse_vertical = true;
        let lines = lua(&g);
        let next = lines.iter().find(|l| l.contains("fingers = 4") && l.contains("e+1")).unwrap();
        assert!(next.contains("direction = \"right\""), "{next}");
        let full = lines.iter().find(|l| l.contains("fingers = 4") && l.contains("fullscreen")).unwrap();
        assert!(full.contains("direction = \"down\""), "{full}");
    }

    #[test]
    fn reverse_follow_flips_invert() {
        let mut g = Gestures { enabled: true, ..Gestures::default() };
        g.three.reverse_horizontal = true;
        assert!(lua(&g).iter().any(|l| l.contains("workspace_swipe_invert = false")));
    }

    #[test]
    fn empty_command_is_skipped() {
        let mut g = Gestures { enabled: true, ..Gestures::default() };
        g.four.actions.insert("pinchout".into(), "command".into());
        assert!(!lua(&g).iter().any(|l| l.contains("pinchout")));
        g.four.commands.insert("pinchout".into(), "kitty".into());
        assert!(lua(&g).iter().any(|l| l.contains("pinchout") && l.contains("'kitty'") || l.contains("\"kitty\"")));
    }

    #[test]
    fn glimpse_toggles_the_panel() {
        let mut g = Gestures { enabled: true, ..Gestures::default() };
        g.three.actions.insert("up".into(), "glimpse".into());
        assert!(line(&lua(&g), 3, "up").contains("nyx-glimpse --toggle"));
    }

    #[test]
    fn app_launches_the_chosen_desktop_entry() {
        let mut g = Gestures { enabled: true, ..Gestures::default() };
        g.four.actions.insert("pinchout".into(), "app".into());
        assert!(line(&lua(&g), 4, "pinchout").is_empty(), "no app chosen yet");
        g.four.apps.insert("pinchout".into(), "firefox.desktop; rm -rf ~".into());
        assert!(line(&lua(&g), 4, "pinchout").is_empty(), "not a desktop entry id");
        g.four.apps.insert("pinchout".into(), "org.mozilla.firefox.desktop".into());
        assert!(line(&lua(&g), 4, "pinchout").contains("uwsm-app -- org.mozilla.firefox.desktop"));
    }

    #[test]
    fn no_duplicate_directions() {
        let g = Gestures { enabled: true, ..Gestures::default() };
        let lines = lua(&g);
        for fingers in [3, 4] {
            let mut seen = std::collections::HashSet::new();
            for l in lines.iter().filter(|l| l.contains(&format!("fingers = {fingers},"))) {
                let dir = l.split("direction = \"").nth(1).unwrap().split('"').next().unwrap();
                assert!(seen.insert(dir.to_string()), "duplicate {dir} for {fingers}");
            }
        }
    }
}
