//! Preamp + 9-band equalizer as a PipeWire filter-chain.
//!
//! Hosted by its own `pipewire -c settings-eq.conf` client under a user unit —
//! the same arrangement Omarchy uses for its speaker tuning — so it can be
//! switched and reconfigured without restarting the audio daemon.
//!
//! Chain (per channel): linear preamp → 9 × peaking biquad → LSP limiter.
//! The limiter (when `lsp-plugins-lv2` is installed) is what makes a large
//! preamp safe: loud peaks are caught instead of clipping.
//!
//! The virtual sink is `settings_eq`; its playback stream `settings_eq.output`
//! feeds the chosen hardware sink. Omarchy's volume keys resolve through a DSP
//! sink by that naming convention, so they keep controlling real loudness.

use crate::{cmd, paths};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const SINK: &str = "settings_eq";
pub const PLAYBACK: &str = "settings_eq.output";
pub const UNIT: &str = "settings-eq.service";
pub const FREQUENCIES: [u32; 9] = [31, 63, 125, 250, 500, 1000, 2000, 4000, 8000];
pub const Q: f64 = 1.41;
pub const GAIN_RANGE: (f64, f64) = (-12.0, 12.0);
pub const PREAMP_RANGE: (f64, f64) = (-12.0, 24.0);
/// Without a limiter there's nothing to stop clipping, so boost is capped lower.
pub const PREAMP_MAX_UNLIMITED: f64 = 9.0;
const LIMITER_URI: &str = "http://lsp-plug.in/plugins/lv2/limiter_stereo";
const LIMITER_TTL: &str = "/usr/lib/lv2/lsp-plugins.lv2/limiter_stereo.ttl";
/// -1 dBFS ceiling.
const LIMITER_THRESHOLD: f64 = 0.891;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Eq {
    pub enabled: bool,
    pub preamp_db: f64,
    pub gains: [f64; 9],
    /// Hardware sink the equalizer plays into. Empty: whatever was the default.
    pub target: String,
    pub preset: String,
}

impl Default for Eq {
    fn default() -> Self {
        // A modest boost out of the box: Omarchy's default level is quiet.
        Self { enabled: true, preamp_db: 6.0, gains: [0.0; 9], target: String::new(), preset: "flat".into() }
    }
}

pub fn settings_file() -> PathBuf {
    paths::app_dir().join("audio.toml")
}

pub fn conf_file() -> PathBuf {
    paths::config_home().join("pipewire/settings-eq.conf")
}

pub fn unit_file() -> PathBuf {
    paths::config_home().join("systemd/user").join(UNIT)
}

pub fn presets_file() -> PathBuf {
    paths::app_dir().join("eq-presets.toml")
}

pub fn config_files() -> Vec<PathBuf> {
    vec![settings_file(), conf_file()]
}

pub fn limiter_available() -> bool {
    Path::new(LIMITER_TTL).exists()
}

pub fn preamp_max() -> f64 {
    if limiter_available() { PREAMP_RANGE.1 } else { PREAMP_MAX_UNLIMITED }
}

pub fn load() -> Eq {
    std::fs::read_to_string(settings_file()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(eq: &Eq) -> Result<()> {
    cmd::atomic_write(&settings_file(), &toml::to_string_pretty(eq)?)
}

pub fn db_to_linear(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

fn clamp(eq: &Eq) -> Eq {
    let mut e = eq.clone();
    e.preamp_db = e.preamp_db.clamp(PREAMP_RANGE.0, preamp_max());
    for g in e.gains.iter_mut() {
        *g = g.clamp(GAIN_RANGE.0, GAIN_RANGE.1);
    }
    e
}

/// Control values as the graph should run them (bypass is all-flat, unity gain).
pub fn controls(eq: &Eq) -> Vec<(String, f64)> {
    let eq = clamp(eq);
    let mut out = Vec::new();
    let mult = if eq.enabled { db_to_linear(eq.preamp_db) } else { 1.0 };
    for side in ["l", "r"] {
        out.push((format!("pre_{side}:Mult"), (mult * 10000.0).round() / 10000.0));
        for (i, gain) in eq.gains.iter().enumerate() {
            let g = if eq.enabled { *gain } else { 0.0 };
            out.push((format!("eq_{side}_{i}:Gain"), g));
        }
    }
    out
}

pub fn generate_conf(eq: &Eq, target: &str, limiter: bool) -> String {
    let values: std::collections::HashMap<String, f64> = controls(eq).into_iter().collect();
    let mut nodes = String::new();
    let mut links = String::new();
    for side in ["l", "r"] {
        nodes.push_str(&format!(
            "          {{ type = builtin name = pre_{side} label = linear control = {{ \"Mult\" = {:.4} \"Add\" = 0.0 }} }}\n",
            values[&format!("pre_{side}:Mult")]
        ));
        for (i, f) in FREQUENCIES.iter().enumerate() {
            nodes.push_str(&format!(
                "          {{ type = builtin name = eq_{side}_{i} label = bq_peaking control = {{ \"Freq\" = {f}.0 \"Q\" = {Q} \"Gain\" = {:.2} }} }}\n",
                values[&format!("eq_{side}_{i}:Gain")]
            ));
        }
        links.push_str(&format!("          {{ output = \"pre_{side}:Out\" input = \"eq_{side}_0:In\" }}\n"));
        for i in 0..FREQUENCIES.len() - 1 {
            links.push_str(&format!("          {{ output = \"eq_{side}_{i}:Out\" input = \"eq_{side}_{}:In\" }}\n", i + 1));
        }
    }
    let last = FREQUENCIES.len() - 1;
    let outputs = if limiter {
        nodes.push_str(&format!(
            "          {{ type = lv2 name = limiter plugin = \"{LIMITER_URI}\"\n            \
             control = {{ \"alr\" = 0 \"boost\" = 0 \"th\" = {LIMITER_THRESHOLD} }} }}\n"
        ));
        links.push_str(&format!("          {{ output = \"eq_l_{last}:Out\" input = \"limiter:in_l\" }}\n"));
        links.push_str(&format!("          {{ output = \"eq_r_{last}:Out\" input = \"limiter:in_r\" }}\n"));
        "[ \"limiter:out_l\" \"limiter:out_r\" ]".to_string()
    } else {
        format!("[ \"eq_l_{last}:Out\" \"eq_r_{last}:Out\" ]")
    };
    let target_line = if target.is_empty() { String::new() } else { format!("        target.object = \"{target}\"\n") };

    format!(
        r#"# Generated by Settings (managed). Preamp + 9-band equalizer.
# Change it from Settings → Sound; edits here are overwritten.
# Runs as: pipewire -c settings-eq.conf ({UNIT})

context.properties = {{
    log.level = 0
}}

context.spa-libs = {{
    audio.convert.* = audioconvert/libspa-audioconvert
    support.*       = support/libspa-support
}}

context.modules = [
    {{ name = libpipewire-module-rt
        args = {{ }}
        flags = [ ifexists nofail ]
    }}
    {{ name = libpipewire-module-protocol-native }}
    {{ name = libpipewire-module-client-node }}
    {{ name = libpipewire-module-adapter }}
    {{ name = libpipewire-module-filter-chain
      args = {{
        node.description = "Equalizer"
        media.name       = "Equalizer"
        filter.graph = {{
        nodes = [
{nodes}        ]
        links = [
{links}        ]
        inputs  = [ "pre_l:In" "pre_r:In" ]
        outputs = {outputs}
        }}
        audio.channels = 2
        audio.position = [ FL FR ]
        capture.props = {{
          node.name   = "{SINK}"
          media.class = Audio/Sink
        }}
        playback.props = {{
          node.name          = "{PLAYBACK}"
          node.passive       = true
{target_line}          node.dont-move     = true
          node.dont-fallback = true
          stream.dont-remix  = true
        }}
      }}
    }}
]
"#
    )
}

pub fn generate_unit(program: &str) -> String {
    format!(
        "# Generated by Settings (managed).\n\
         [Unit]\n\
         Description=Settings preamp and equalizer\n\
         After=pipewire.service wireplumber.service\n\
         Requires=pipewire.service\n\
         Wants=wireplumber.service\n\
         PartOf=pipewire.service\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={program} -c settings-eq.conf\n\
         Restart=on-failure\n\
         RestartSec=2\n\
         \n\
         [Install]\n\
         WantedBy=graphical-session.target\n"
    )
}

// ----- Devices -----

#[derive(Debug, Clone)]
pub struct Device {
    pub name: String,
    pub description: String,
}

fn list(kind: &str) -> Vec<Device> {
    let Some(text) = cmd::output(&["pactl", "--format=json", "list", kind]) else { return vec![] };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&text) else { return vec![] };
    items
        .iter()
        .filter_map(|d| {
            Some(Device {
                name: d.get("name")?.as_str()?.to_string(),
                description: d.get("description").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// Real outputs: everything except our own sink.
pub fn hardware_sinks() -> Vec<Device> {
    list("sinks").into_iter().filter(|d| d.name != SINK).collect()
}

pub fn sources() -> Vec<Device> {
    list("sources").into_iter().filter(|d| !d.name.ends_with(".monitor")).collect()
}

pub fn default_sink() -> String {
    cmd::output(&["pactl", "get-default-sink"]).unwrap_or_default()
}

pub fn default_source() -> String {
    cmd::output(&["pactl", "get-default-source"]).unwrap_or_default()
}

/// The sink whose volume is real loudness (resolves through any DSP sink).
pub fn volume_sink() -> String {
    if cmd::present("omarchy-audio-output-sink")
        && let Some(s) = cmd::output(&["omarchy-audio-output-sink"]).filter(|s| !s.is_empty())
    {
        return s;
    }
    let d = default_sink();
    if d == SINK {
        let eq = load();
        if !eq.target.is_empty() {
            return eq.target;
        }
    }
    d
}

pub fn volume_percent(sink: &str) -> Option<u32> {
    let text = cmd::output(&["pactl", "get-sink-volume", sink])?;
    text.split_whitespace().find(|w| w.ends_with('%'))?.trim_end_matches('%').parse().ok()
}

pub fn muted(sink: &str) -> bool {
    cmd::output(&["pactl", "get-sink-mute", sink]).is_some_and(|s| s.ends_with("yes"))
}

// ----- Running graph -----

pub fn running() -> bool {
    cmd::output(&["systemctl", "--user", "is-active", UNIT]).is_some_and(|s| s.trim() == "active")
}

fn pw_node_id(name: &str) -> Option<u64> {
    let text = cmd::output(&["pw-dump", "Node"])?;
    let nodes: Value = serde_json::from_str(&text).ok()?;
    nodes.as_array()?.iter().find_map(|n| {
        let props = n.get("info")?.get("props")?;
        (props.get("node.name")?.as_str()? == name).then(|| n.get("id")?.as_u64()).flatten()
    })
}

/// Current control values read back from the running filter.
pub fn live_controls() -> Option<Vec<(String, f64)>> {
    let text = cmd::output(&["pw-dump", "Node"])?;
    let nodes: Value = serde_json::from_str(&text).ok()?;
    let node = nodes.as_array()?.iter().find(|n| {
        n.get("info").and_then(|i| i.get("props")).and_then(|p| p.get("node.name")).and_then(|v| v.as_str()) == Some(SINK)
    })?;
    let props = node.get("info")?.get("params")?.get("Props")?.as_array()?;
    for p in props {
        if let Some(params) = p.get("params").and_then(|x| x.as_array()) {
            let mut out = Vec::new();
            for pair in params.chunks(2) {
                if let [Value::String(k), v] = pair
                    && let Some(f) = v.as_f64()
                {
                    out.push((k.clone(), f));
                }
            }
            if !out.is_empty() {
                return Some(out);
            }
        }
    }
    None
}

/// Push control values into the running filter. No restart, no audio drop.
pub fn set_live(eq: &Eq) -> Result<()> {
    let id = pw_node_id(SINK).context("the equalizer isn't running")?;
    let params: Vec<String> = controls(eq).into_iter().map(|(k, v)| format!("\"{k}\" {v:.4}")).collect();
    let pod = format!("{{ params = [ {} ] }}", params.join(" "));
    cmd::run(&["pw-cli", "set-param", &id.to_string(), "Props", &pod])?;
    Ok(())
}

/// Write config + unit, (re)start the host, route the default output through it.
pub fn install_and_start(eq: &Eq) -> Result<()> {
    let target = if eq.target.is_empty() {
        let d = default_sink();
        if d.is_empty() || d == SINK { hardware_sinks().first().map(|s| s.name.clone()).unwrap_or_default() } else { d }
    } else {
        eq.target.clone()
    };
    if target.is_empty() {
        bail!("no audio output found");
    }
    let program = cmd::unit_program("pipewire")?;
    let mut eq = eq.clone();
    eq.target = target.clone();
    save(&eq)?;
    cmd::atomic_write(&conf_file(), generate_conf(&eq, &target, limiter_available()))?;
    cmd::atomic_write(&unit_file(), generate_unit(&program))?;
    cmd::run(&["systemctl", "--user", "daemon-reload"])?;
    cmd::run(&["systemctl", "--user", "enable", UNIT])?;
    cmd::run(&["systemctl", "--user", "restart", UNIT])?;
    // Wait for the sink to appear, then make it the default output.
    for _ in 0..30 {
        if pw_node_id(SINK).is_some() {
            let _ = cmd::run(&["pactl", "set-default-sink", SINK]);
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    bail!("the equalizer started but its output didn't appear")
}

/// Stop the host and put the hardware output back as default.
pub fn stop(eq: &Eq) -> Result<()> {
    let target = eq.target.clone();
    let _ = cmd::run(&["systemctl", "--user", "disable", "--now", UNIT]);
    if (default_sink() == SINK || default_sink().is_empty()) && !target.is_empty() {
        let _ = cmd::run(&["pactl", "set-default-sink", &target]);
    }
    Ok(())
}

// ----- Presets -----

pub fn builtin_presets() -> Vec<(&'static str, &'static str, [f64; 9])> {
    vec![
        ("flat", "Flat", [0.0; 9]),
        ("bass", "Bass boost", [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        ("treble", "Treble boost", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 5.0, 6.0]),
        ("vocal", "Vocal & podcasts", [-3.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.0, 1.0, 0.0]),
        ("loudness", "Loudness", [5.0, 4.0, 2.0, 0.0, -1.0, 0.0, 1.0, 3.0, 4.0]),
        ("laptop", "Laptop speakers", [-6.0, -3.0, 1.0, 2.0, 2.0, 1.0, 2.0, 3.0, 2.0]),
        ("headphones", "Headphones", [3.0, 2.0, 1.0, 0.0, -1.0, 0.0, 1.0, 2.0, 1.0]),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserPresets {
    #[serde(default)]
    pub preset: Vec<UserPreset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreset {
    pub name: String,
    pub gains: [f64; 9],
    #[serde(default)]
    pub preamp_db: Option<f64>,
}

pub fn user_presets() -> UserPresets {
    std::fs::read_to_string(presets_file()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_user_presets(p: &UserPresets) -> Result<()> {
    cmd::atomic_write(&presets_file(), &toml::to_string_pretty(p)?)
}

/// Linear-in-dB headroom estimate: how far the loudest band pushes past 0 dBFS.
pub fn peak_boost(eq: &Eq) -> f64 {
    let max_band = eq.gains.iter().cloned().fold(0.0_f64, f64::max);
    if eq.enabled { eq.preamp_db + max_band } else { 0.0 }
}

// ----- Volume keys beyond 100% -----

/// `settings --volume raise|lower|+N|-N`: like Omarchy's, but up to the max set here.
pub fn step_volume(action: &str, max: u32) -> Result<()> {
    let sink = volume_sink();
    if sink.is_empty() {
        bail!("no sink");
    }
    let delta: i64 = match action {
        "raise" => 5,
        "lower" => -5,
        s => s.parse().context("expected raise, lower, +N or -N")?,
    };
    let current = volume_percent(&sink).unwrap_or(0) as i64;
    let next = (current + delta).clamp(0, max as i64);
    cmd::run(&["pactl", "set-sink-mute", &sink, "0"])?;
    cmd::run(&["pactl", "set-sink-volume", &sink, &format!("{next}%")])?;
    if cmd::present("omarchy-osd") {
        let icon = if next == 0 { "volume-muted" } else { "volume-high" };
        let _ = cmd::run(&["omarchy-osd", "-i", icon, "-p", &next.min(100).to_string()]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conf_has_full_chain_with_limiter() {
        let eq = Eq { preamp_db: 6.0, gains: [1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, -2.0], ..Eq::default() };
        let c = generate_conf(&eq, "alsa_output.x", true);
        assert!(c.contains("node.name   = \"settings_eq\""));
        assert!(c.contains("target.object = \"alsa_output.x\""));
        assert!(c.contains("label = linear"));
        assert_eq!(c.matches("bq_peaking").count(), 18);
        assert!(c.contains("\"eq_l_8:Out\" input = \"limiter:in_l\""));
        assert!(c.contains("outputs = [ \"limiter:out_l\" \"limiter:out_r\" ]"));
        assert!(c.contains("\"Mult\" = 1.9953"));
        assert!(c.contains("\"Freq\" = 31.0"));
        assert!(c.contains("\"boost\" = 0"));
    }

    #[test]
    fn conf_without_limiter_ends_on_last_band() {
        let c = generate_conf(&Eq::default(), "", false);
        assert!(c.contains("outputs = [ \"eq_l_8:Out\" \"eq_r_8:Out\" ]"));
        assert!(!c.contains("target.object"));
        assert!(!c.contains("lv2"));
    }

    #[test]
    fn bypass_is_unity() {
        let eq = Eq { enabled: false, preamp_db: 12.0, gains: [6.0; 9], ..Eq::default() };
        for (_, v) in controls(&eq) {
            assert!(v == 1.0 || v == 0.0);
        }
    }

    #[test]
    fn values_are_clamped() {
        let eq = Eq { preamp_db: 99.0, gains: [40.0; 9], ..Eq::default() };
        let c: std::collections::HashMap<_, _> = controls(&eq).into_iter().collect();
        assert!(c["eq_l_0:Gain"] <= 12.0);
        assert!(c["pre_l:Mult"] <= db_to_linear(PREAMP_RANGE.1) + 1e-6);
    }

    #[test]
    fn links_are_a_chain() {
        let c = generate_conf(&Eq::default(), "x", true);
        for side in ["l", "r"] {
            assert!(c.contains(&format!("\"pre_{side}:Out\" input = \"eq_{side}_0:In\"")));
            for i in 0..8 {
                assert!(c.contains(&format!("\"eq_{side}_{i}:Out\" input = \"eq_{side}_{}:In\"", i + 1)));
            }
        }
    }
}
