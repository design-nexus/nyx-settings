//! Date and time, language and the computer's name. Changes use the root helper.

use crate::cmd;
use crate::sections::accounts::{require_helper, run_admin};
use crate::widgets::{self, Page};
use gtk::prelude::*;
use std::collections::HashMap;

/// `timedatectl show` prints `Key=Value` lines.
fn parse_show(text: &str) -> HashMap<String, String> {
    text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// `LANG` from `/etc/locale.conf`.
fn parse_lang(text: &str) -> Option<String> {
    text.lines().find_map(|l| l.strip_prefix("LANG=")).map(|v| v.trim().trim_matches('"').to_string()).filter(|v| !v.is_empty())
}

/// `de_DE.utf8` and `de_DE.UTF-8` name the same locale.
fn same_locale(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.to_lowercase().replace("utf-8", "utf8");
    norm(a) == norm(b)
}

/// `locale -a` says `de_DE.utf8`; locale.conf and localectl use `de_DE.UTF-8`.
fn canonical_locale(l: &str) -> String {
    match l.split_once('.') {
        Some((base, rest)) if rest.to_lowercase().starts_with("utf8") => format!("{base}.UTF-8{}", &rest[4..]),
        _ => l.to_string(),
    }
}

pub fn build(page: &Page) {
    let can_edit = require_helper(page, "region", "changing the date, language and name");
    time_group(page, can_edit);
    language_group(page, can_edit);
    name_group(page, can_edit);
}

fn time_group(page: &Page, can_edit: bool) {
    let g = page.group("Date & time");
    let show = parse_show(&cmd::output(&["timedatectl", "show"]).unwrap_or_default());
    let zone = show.get("Timezone").cloned().unwrap_or_default();
    let ntp = show.get("NTP").is_some_and(|v| v == "yes");

    let (r, _) = widgets::info_row("Now", &cmd::output(&["date", "+%A %-d %B %Y, %H:%M"]).unwrap_or_default());
    g.add(&r);

    let zones: Vec<String> =
        cmd::output(&["timedatectl", "list-timezones"]).unwrap_or_default().lines().map(str::to_string).collect();
    if !zones.is_empty() {
        let mut options: Vec<(String, String)> = zones.iter().map(|z| (z.clone(), z.replace('_', " "))).collect();
        // Keep a zone like US/Central that the list doesn't offer, so it shows as current.
        if !zone.is_empty() && !zones.contains(&zone) {
            options.insert(0, (zone.clone(), zone.replace('_', " ")));
        }
        let (r, dd) =
            widgets::choice_row("Time zone", "Where this computer is. Changes the clock shown everywhere.", options, &zone, {
                let zone = zone.clone();
                move |z| {
                    if z != zone {
                        run_admin(&["set-timezone", &z], None, "Time zone changed", "region");
                    }
                }
            });
        dd.set_enable_search(true);
        dd.set_sensitive(can_edit);
        widgets::keywords("timezone clock location utc city");
        g.add(&r);
    }

    let (r, sw) =
        widgets::switch_row("Set the time automatically", "Keep the clock right using the internet.", ntp, move |now| {
            if now != ntp {
                run_admin(
                    &["set-ntp", if now { "1" } else { "0" }],
                    None,
                    if now { "Automatic time is on" } else { "Automatic time is off" },
                    "region",
                );
            }
        });
    sw.set_sensitive(can_edit);
    widgets::keywords("ntp network time sync clock");
    g.add(&r);

    if !ntp && can_edit {
        let (r, _) = widgets::entry_row(
            "Set the time",
            "Date and time as 2026-01-31 14:05:00. Press Enter.",
            "",
            "2026-01-31 14:05:00",
            |t| {
                let t = t.trim().to_string();
                if !t.is_empty() {
                    run_admin(&["set-time", &t], None, "Time set", "region");
                }
            },
        );
        widgets::keywords("manual date clock");
        g.add(&r);
    }
}

fn language_group(page: &Page, can_edit: bool) {
    let g = page.group("Language");
    let current = parse_lang(&std::fs::read_to_string("/etc/locale.conf").unwrap_or_default()).unwrap_or_else(|| "C".into());
    let built: Vec<String> =
        cmd::output(&["locale", "-a"]).unwrap_or_default().lines().filter(|l| l.contains("utf8")).map(str::to_string).collect();
    if !built.is_empty() {
        let mut options: Vec<(String, String)> = built.iter().map(|l| (l.clone(), l.clone())).collect();
        let selected = built.iter().find(|b| same_locale(b, &current)).cloned().unwrap_or_else(|| {
            options.insert(0, (current.clone(), current.clone()));
            current.clone()
        });
        let (r, dd) = widgets::choice_row(
            "System language",
            "Language and formats for the whole system. Applies after you sign in again.",
            options,
            &selected,
            {
                let selected = selected.clone();
                move |l| {
                    if l != selected {
                        run_admin(
                            &["set-locale", &canonical_locale(&l)],
                            None,
                            "Language changed. Sign out and back in to use it.",
                            "region",
                        );
                    }
                }
            },
        );
        dd.set_sensitive(can_edit);
        widgets::keywords("locale lang english german french formats region");
        g.add(&r);
    }
    if can_edit {
        let (r, _) = widgets::entry_row(
            "Add a language",
            "Build another locale, as it appears in /usr/share/i18n/SUPPORTED, e.g. de_DE.UTF-8 UTF-8. Then pick it above. Press Enter.",
            "",
            "de_DE.UTF-8 UTF-8",
            |t| {
                let t = t.trim().to_string();
                if !t.is_empty() {
                    run_admin(&["enable-locale", &t], None, "Language added", "region");
                }
            },
        );
        widgets::keywords("locale-gen generate install");
        g.add(&r);
    }
}

fn name_group(page: &Page, can_edit: bool) {
    let g = page.group("This computer");
    let host = cmd::output(&["hostnamectl", "hostname"]).unwrap_or_default();
    if can_edit {
        let (r, _) = widgets::entry_row(
            "Name",
            "How this computer appears on the network. Letters, digits and -. Press Enter.",
            &host,
            "name",
            {
                let host = host.clone();
                move |n| {
                    let n = n.trim().to_string();
                    if !n.is_empty() && n != host {
                        run_admin(&["set-hostname", &n], None, "Name changed", "region");
                    }
                }
            },
        );
        widgets::keywords("hostname computer name network");
        g.add(&r);
    } else {
        let (r, _) = widgets::info_row("Name", &host);
        g.add(&r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_and_lang() {
        let m = parse_show("Timezone=US/Central\nNTP=yes\nNTPSynchronized=yes\n");
        assert_eq!(m["Timezone"], "US/Central");
        assert_eq!(parse_lang("# x\nLANG=en_US.UTF-8\n"), Some("en_US.UTF-8".into()));
        assert_eq!(parse_lang("LANG=\n"), None);
        assert_eq!(parse_lang(""), None);
    }

    #[test]
    fn locale_names_match() {
        assert!(same_locale("en_US.utf8", "en_US.UTF-8"));
        assert!(!same_locale("en_US.utf8", "de_DE.UTF-8"));
        assert_eq!(canonical_locale("de_DE.utf8"), "de_DE.UTF-8");
        assert_eq!(canonical_locale("sr_RS.utf8@latin"), "sr_RS.UTF-8@latin");
        assert_eq!(canonical_locale("C"), "C");
    }
}
