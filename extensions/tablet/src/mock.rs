//! A pretend tablet for testing pages without one: `TABLET_MOCK=wacom-intuos-s`.
//! Its settings are kept in the state folder; nothing writes Hyprland's config or
//! talks to Hyprland while it's on.

use crate::Hardware;

pub fn on() -> bool {
    std::env::var("TABLET_MOCK").is_ok_and(|v| !v.is_empty())
}

pub fn hardware() -> Hardware {
    Hardware {
        tablets: vec!["wacom-intuos-s-pen".into()],
        monitors: vec![("eDP-1".into(), 2560.0, 1600.0)],
        size: Some((152.0, 95.0)),
        info: Some(("Wacom Intuos S (CTL-4100)".into(), 4)),
        otd: true,
    }
}
