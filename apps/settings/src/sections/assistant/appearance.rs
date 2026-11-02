//! Aura appearance. Values live in Assistant's validated configuration.

use super::{choice, client, save, slider, switch};
use crate::widgets::{self, Page};

pub fn build(page: &Page) {
    let Some(cfg) = super::config_or_banner(page) else { return };
    let g = page.group("Aura");
    g.add(&switch(&cfg, "ui.aura_enabled", "Aura", "Show an ambient border while the assistant is active."));
    g.add(&choice(
        &cfg,
        "ui.aura_style",
        "Style",
        "The border shape and movement.",
        &[("full", "Full Aura"), ("edge-flow", "Edge Flow")],
    ));
    g.add(&choice(
        &cfg,
        "ui.aura_visibility",
        "Visibility",
        "Persistent stays faint during an active conversation.",
        &[("persistent", "Persistent"), ("activity", "Activity only")],
    ));
    g.add(&slider(
        &cfg,
        "ui.aura_thickness",
        "Thickness",
        "Controls Full Aura's glow reach and Edge Flow's line width.",
        (1.0, 10.0, 1.0),
        0,
        " px",
    ));
    g.add(&slider(
        &cfg,
        "ui.aura_intensity",
        "Glow intensity",
        "Keep the border subtle over other apps.",
        (0.0, 1.0, 0.05),
        2,
        "",
    ));
    g.add(&slider(
        &cfg,
        "ui.aura_speed",
        "Animation speed",
        "How fast Edge Flow travels and Full Aura breathes.",
        (0.25, 3.0, 0.05),
        2,
        "×",
    ));
    g.add(&slider(
        &cfg,
        "ui.aura_segment_length",
        "Flow length",
        "Portion of the side-and-bottom path lit in Edge Flow.",
        (0.04, 0.4, 0.01),
        2,
        "",
    ));
    g.add(&switch(&cfg, "ui.aura_audio_reactive", "React to voice", "Use Assistant's existing microphone and playback levels."));
    g.add(&switch(&cfg, "ui.aura_persona_colors", "Persona colors", "Blend the active persona's Aura tint with state colors."));
    g.add(&choice(
        &cfg,
        "ui.aura_reduce_motion",
        "Reduce motion",
        "Follow the desktop setting, or override it for Aura.",
        &[("system", "Follow system"), ("on", "On"), ("off", "Off")],
    ));
    g.add(&choice(
        &cfg,
        "ui.aura_monitor_mode",
        "Displays",
        "Choose where Aura appears; headless displays are skipped.",
        &[("active", "Active display"), ("primary", "Primary display"), ("all", "All displays")],
    ));
    let (row, _) = widgets::entry_row(
        "Primary output",
        "Used by Primary display mode. Leave empty to use the display at the layout origin.",
        &client::get_str(&cfg, "ui.aura_primary_monitor"),
        "e.g. eDP-2",
        |text| save("ui.aura_primary_monitor", client::quoted(text.trim())),
    );
    g.add(&row);
    widgets::keywords("assistant appearance aura border glow full edge flow conversation display monitor motion");
}
