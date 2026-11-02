//! Window and layer rules added in Settings, and what each can do.

use super::lua::quote;
use super::state::Rule;

#[derive(Clone, Copy, PartialEq)]
pub enum Value {
    /// On or off: written as `true`.
    Flag,
    /// A number.
    Number,
    /// Text such as `0.9 0.8` or `3`.
    Text,
}

pub struct Effect {
    pub key: &'static str,
    pub label: &'static str,
    pub value: Value,
    /// What to type for a value effect.
    pub hint: &'static str,
}

const fn e(key: &'static str, label: &'static str, value: Value, hint: &'static str) -> Effect {
    Effect { key, label, value, hint }
}

/// Effects Hyprland accepts on a window rule (each checked against the running compositor).
pub const WINDOW: &[Effect] = &[
    e("float", "Open floating", Value::Flag, ""),
    e("tile", "Always tile", Value::Flag, ""),
    e("center", "Center on the screen", Value::Flag, ""),
    e("pin", "Show on every workspace", Value::Flag, ""),
    e("fullscreen", "Open fullscreen", Value::Flag, ""),
    e("no_focus", "Don't take focus", Value::Flag, ""),
    e("no_blur", "No blur behind", Value::Flag, ""),
    e("no_shadow", "No shadow", Value::Flag, ""),
    e("no_anim", "No animations", Value::Flag, ""),
    e("opacity", "Opacity", Value::Text, "0.9 0.8 (active, inactive)"),
    e("workspace", "Open on workspace", Value::Text, "3, or special silent"),
    e("size", "Size", Value::Text, "800 600"),
    e("rounding", "Corner rounding", Value::Number, "0"),
];

pub const LAYER: &[Effect] = &[
    e("blur", "Blur behind", Value::Flag, ""),
    e("xray", "See through to the wallpaper", Value::Flag, ""),
    e("dim_around", "Dim everything else", Value::Flag, ""),
    e("no_anim", "No animations", Value::Flag, ""),
    e("ignore_alpha", "Ignore transparency below", Value::Number, "0.5"),
];

pub fn effects(layer: bool) -> &'static [Effect] {
    if layer { LAYER } else { WINDOW }
}

/// Words a rule effect may contain: no quotes or control characters.
fn plain(s: &str) -> bool {
    s.len() < 200 && s.chars().all(|c| !c.is_control() && c != '"' && c != '\\')
}

/// The Lua call for a rule, or `None` when it isn't complete or valid.
pub fn lua_line(layer: bool, r: &Rule) -> Option<String> {
    let effect = effects(layer).iter().find(|f| f.key == r.effect)?;
    let value = match effect.value {
        Value::Flag => "true".to_string(),
        Value::Number => r.value.trim().parse::<f64>().ok().filter(|n| n.is_finite())?.to_string(),
        Value::Text => {
            let v = r.value.trim();
            if v.is_empty() || !plain(v) {
                return None;
            }
            quote(v)
        }
    };
    let mut matches = Vec::new();
    if layer {
        if r.class.trim().is_empty() || !plain(&r.class) {
            return None;
        }
        matches.push(format!("namespace = {}", quote(r.class.trim())));
    } else {
        if !r.class.trim().is_empty() {
            if !plain(&r.class) {
                return None;
            }
            matches.push(format!("class = {}", quote(r.class.trim())));
        }
        if !r.title.trim().is_empty() {
            if !plain(&r.title) {
                return None;
            }
            matches.push(format!("title = {}", quote(r.title.trim())));
        }
        if matches.is_empty() {
            return None;
        }
    }
    let call = if layer { "layer_rule" } else { "window_rule" };
    Some(format!("hl.{call}({{ match = {{ {} }}, {} = {} }})", matches.join(", "), effect.key, value))
}

/// A process name `pgrep -x` can look for (it matches the first 15 characters).
pub fn valid_process(s: &str) -> bool {
    !s.is_empty() && s.len() <= 15 && s.chars().all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
}

/// The process to look for when `command` is already running: its program name.
pub fn process_of(command: &str) -> Option<String> {
    let mut words = command.split_whitespace().skip_while(|w| *w == "uwsm-app" || *w == "--" || *w == "env" || w.contains('='));
    let program = words.next()?.rsplit('/').next()?;
    let name: String = program.chars().take(15).collect();
    valid_process(&name).then_some(name)
}

/// A short description of what a rule matches, for lists.
pub fn describe(layer: bool, r: &Rule) -> (String, String) {
    let who = if layer {
        format!("Layer {}", r.class)
    } else {
        match (r.class.is_empty(), r.title.is_empty()) {
            (false, false) => format!("{} · {}", r.class, r.title),
            (false, true) => r.class.clone(),
            _ => format!("Title {}", r.title),
        }
    };
    let what = effects(layer).iter().find(|f| f.key == r.effect).map(|f| f.label).unwrap_or(&r.effect);
    let what = if r.value.is_empty() { what.to_string() } else { format!("{what}: {}", r.value) };
    (who, what)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(class: &str, title: &str, effect: &str, value: &str) -> Rule {
        Rule { class: class.into(), title: title.into(), effect: effect.into(), value: value.into() }
    }

    #[test]
    fn window_rules() {
        assert_eq!(
            lua_line(false, &rule("^mpv$", "", "float", "")).unwrap(),
            "hl.window_rule({ match = { class = \"^mpv$\" }, float = true })"
        );
        assert_eq!(
            lua_line(false, &rule("foo", "bar", "opacity", "0.9 0.8")).unwrap(),
            "hl.window_rule({ match = { class = \"foo\", title = \"bar\" }, opacity = \"0.9 0.8\" })"
        );
        assert!(lua_line(false, &rule("a", "", "rounding", "4")).unwrap().ends_with("rounding = 4 })"));
    }

    #[test]
    fn layer_rules() {
        assert_eq!(
            lua_line(true, &rule("waybar", "", "blur", "")).unwrap(),
            "hl.layer_rule({ match = { namespace = \"waybar\" }, blur = true })"
        );
    }

    #[test]
    fn processes() {
        assert_eq!(process_of("uwsm-app -- hyprsunset"), Some("hyprsunset".into()));
        assert_eq!(process_of("/usr/bin/nm-applet --indicator"), Some("nm-applet".into()));
        assert_eq!(process_of("env FOO=1 mylongprogramname-x"), Some("mylongprogramna".into()));
        assert_eq!(process_of(""), None);
        assert!(!valid_process("a; b") && !valid_process(""));
    }

    #[test]
    fn incomplete_or_unsafe_rules_are_dropped() {
        assert!(lua_line(false, &rule("", "", "float", "")).is_none());
        assert!(lua_line(false, &rule("a", "", "bogus", "")).is_none());
        assert!(lua_line(false, &rule("a", "", "opacity", "")).is_none());
        assert!(lua_line(false, &rule("a", "", "rounding", "x")).is_none());
        assert!(lua_line(false, &rule("a\"); os.execute(\"x", "", "float", "")).is_none());
        assert!(lua_line(false, &rule("a", "", "workspace", "1\") --")).is_none());
        assert!(lua_line(true, &rule("", "", "blur", "")).is_none());
    }
}
