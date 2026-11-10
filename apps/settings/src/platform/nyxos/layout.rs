//! NyxOS's sidebar: its twelve categories, the pages it names its own way, and the
//! pages that are its own (the system pages, and Accounts).

use crate::sections::{Build, Section};

/// The categories, in order (Home sits above them).
pub const GROUPS: &[&str] = &[
    "Accounts",
    "Appearance",
    "Displays",
    "Desktop",
    "Input",
    "Network",
    "Sound",
    "Power",
    "Security",
    "Apps & Defaults",
    "Storage & Backup",
    "System",
];

/// A page's category, from its id, or else from the group it was written for.
pub fn group(id: &str, group: &'static str) -> &'static str {
    match id {
        "online-accounts" | "accounts" => "Accounts",
        "theme" | "look" => "Appearance",
        "displays" | "nightlight" => "Displays",
        "bar" | "plugins" | "dock" | "switcher" | "control-center" | "notifications" | "workspaces" | "workspace-profiles"
        | "rules" | "capture" => "Desktop",
        "keyboard" | "mouse" | "trackpad" | "keybindings" => "Input",
        "wifi" | "bluetooth" | "devices" | "network" | "sharing" => "Network",
        "audio" => "Sound",
        "power" => "Power",
        "sign-in" | "idle" => "Security",
        "apps" | "software" | "extensions" => "Apps & Defaults",
        "disks" | "mounts" => "Storage & Backup",
        "region" | "boot" | "services" | "printers" | "updates" | "about" => "System",
        _ => match group {
            "" => "",
            "Connections" => "Network",
            "Personalization" => "Appearance",
            "Assistant" | "Local AI" => "Apps & Defaults",
            "Hardware" => "System",
            other => other,
        },
    }
}

fn never() -> bool {
    false
}

/// Nyx apps come from the System Manager here (Store and Welcome), so that page goes.
pub fn apply(list: &mut [Section]) {
    for s in list.iter_mut() {
        s.group = group(s.id, s.group);
        s.title = match s.id {
            "online-accounts" => "Accounts",
            "accounts" => "Users",
            "boot" => "Boot & Sign-in",
            "idle" => "Lock Screen",
            _ => s.title,
        };
        let own: Option<fn(&crate::widgets::Page)> = match s.id {
            "online-accounts" => Some(super::accounts::build),
            "accounts" => Some(super::users::build),
            "region" => Some(super::pages::region),
            "services" => Some(super::pages::services),
            "software" => Some(super::pages::software),
            "boot" => Some(super::pages::boot),
            "idle" => Some(super::lock::build),
            _ => None,
        };
        if let Some(build) = own {
            s.build = Build::Native(build);
        }
        if s.id == "nyx" {
            s.visible = never;
        }
    }
    list.sort_by_key(|s| GROUPS.iter().position(|g| *g == s.group).map_or(0, |i| i + 1));
}

/// Users opens from Accounts.
pub fn off_sidebar(id: &str) -> bool {
    id == "accounts"
}

/// The categories take the colours of the groups they come from.
pub fn colour_group(group: &str) -> &str {
    match group {
        "Appearance" => "Personalization",
        "Displays" | "Sound" | "Power" | "Storage & Backup" => "Hardware",
        "Network" => "Connections",
        "Security" => "Accounts",
        "Apps & Defaults" => "System",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_lands_in_one_of_the_categories() {
        for s in crate::sections::all_with(false) {
            let g = group(s.id, s.group);
            assert!((s.id == "home" && g.is_empty()) || GROUPS.contains(&g), "{} lands in {g:?}", s.id);
        }
    }

    #[test]
    fn the_layout_keeps_home_first_and_the_categories_in_order() {
        let mut list = crate::sections::all_with(false);
        apply(&mut list);
        assert_eq!(list[0].id, "home");
        let order: Vec<usize> = list.iter().skip(1).map(|s| GROUPS.iter().position(|g| *g == s.group).unwrap()).collect();
        assert!(order.windows(2).all(|w| w[0] <= w[1]));
    }
}
