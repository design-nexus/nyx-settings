//! Network beyond joining Wi-Fi: DNS, band, firewall, VPNs and a speed test.
//! Wi-Fi networks and Bluetooth stay on their own page.

use crate::sections::accounts::{helper_notice, run_admin};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

pub fn build(page: &Page) {
    connections(page);
    dns(page);
    wifi_tools(page);
    crate::platform::firewall_group(page);
    if cmd::present("tailscale") {
        tailscale(page);
    }
    if cmd::present("nordvpn") {
        nordvpn(page);
    }
}

// ----- Connections -----

fn connections(page: &Page) {
    if !cmd::present("nmcli") {
        return;
    }
    let g = page.group("Connections");
    let text = cmd::output(&["nmcli", "-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"]).unwrap_or_default();
    let mut any = false;
    for line in text.lines() {
        let f = crate::backend::net::split_terse(line);
        if f.len() < 4 || !f[2].starts_with("connected") || !["wifi", "ethernet"].contains(&f[1].as_str()) {
            continue;
        }
        any = true;
        let ip = cmd::output(&["nmcli", "-g", "IP4.ADDRESS", "device", "show", &f[0]]).unwrap_or_default();
        let kind = if f[1] == "wifi" { "Wi-Fi" } else { "Ethernet" };
        let (r, _) = widgets::info_row(
            &format!("{kind} ({})", f[0]),
            &format!("{} · {}", f[3], ip.lines().next().unwrap_or("no address")),
        );
        widgets::keywords("ip address ethernet wired lan");
        g.add(&r);
    }
    if !any {
        g.note("Not connected to a network.");
    }
}

// ----- DNS -----

fn dns(page: &Page) {
    if !cmd::present("omarchy-dns") {
        return;
    }
    let g = page.group("DNS");
    let current = cmd::output(&["omarchy-dns"]).unwrap_or_default();
    let mut options =
        widgets::opts(&[("Cloudflare", "Cloudflare (1.1.1.1)"), ("Google", "Google (8.8.8.8)"), ("DHCP", "From the network")]);
    if current == "Custom" {
        options.push(("Custom".into(), "Custom servers".into()));
    }
    let (r, _) = widgets::choice_row(
        "DNS provider",
        "Which servers look up website names. Changing it asks for your password.",
        options,
        &current,
        {
            let current = current.clone();
            move |p| {
                if p == current {
                    return;
                }
                let name = p.clone();
                cmd::run_async(&["omarchy-dns", &p], move |r| {
                    match r {
                        Ok(_) => window::toast(&format!("DNS set to {name}")),
                        Err(e) => window::toast(&format!("{e}")),
                    }
                    window::rebuild("network");
                });
            }
        },
    );
    widgets::keywords("resolver nameserver cloudflare google dhcp privacy");
    g.add(&r);
    // Show the servers already in use, so they can be changed.
    let in_use = if current == "Custom" {
        custom_servers(&std::fs::read_to_string("/etc/systemd/resolved.conf").unwrap_or_default())
    } else {
        String::new()
    };
    let (r, _) = widgets::entry_row(
        "Custom DNS servers",
        "Addresses separated by spaces, e.g. 192.168.1.1 1.1.1.1. Press Enter to use them.",
        &in_use,
        "192.168.1.1 1.1.1.1",
        |servers| {
            let servers = servers.trim().to_string();
            if servers.is_empty() {
                return;
            }
            if !servers.chars().all(|c| c.is_ascii_hexdigit() || " .:,".contains(c)) {
                window::toast("Those don't look like IP addresses");
                return;
            }
            cmd::background(
                move || with_stdin(&["omarchy-dns", "Custom"], &format!("{servers}\n")),
                |r| {
                    match r {
                        Ok(()) => window::toast("Custom DNS servers set"),
                        Err(e) => window::toast(&e),
                    }
                    window::rebuild("network");
                },
            );
        },
    );
    widgets::keywords("resolver nameserver dns custom");
    g.add(&r);
}

/// The `DNS=` line of `resolved.conf`.
fn custom_servers(text: &str) -> String {
    text.lines().find_map(|l| l.trim().strip_prefix("DNS=")).map(|v| v.trim().to_string()).unwrap_or_default()
}

fn with_stdin(args: &[&str], text: &str) -> Result<(), String> {
    let (program, rest) = args.split_first().ok_or("empty command")?;
    let mut child = Command::new(program)
        .args(rest)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut pipe) = child.stdin.take() {
        let _ = pipe.write_all(text.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

// ----- Wi-Fi tools -----

/// `omarchy-network-band` prints `band<TAB>5` and `available<TAB>2.4 5 6`.
fn parse_band(text: &str) -> Option<(String, Vec<String>)> {
    let mut band = None;
    let mut available = Vec::new();
    for line in text.lines() {
        match line.split_once('\t') {
            Some(("band", v)) => band = Some(v.trim().to_string()),
            Some(("available", v)) => available = v.split_whitespace().map(str::to_string).collect(),
            _ => {}
        }
    }
    band.map(|b| (b, available))
}

/// Download or upload speed from the live samples (Mbps, one per line): the mean of the later half.
fn speed(text: &str) -> Option<f64> {
    let v: Vec<f64> = text.lines().filter_map(|l| l.trim().parse().ok()).collect();
    let tail = &v[v.len() / 2..];
    (!tail.is_empty()).then(|| tail.iter().sum::<f64>() / tail.len() as f64)
}

fn measure(direction: &str) -> Option<f64> {
    let out = Command::new("timeout").args(["8", "omarchy-network-speedtest", direction]).stdin(Stdio::null()).output().ok()?;
    speed(&String::from_utf8_lossy(&out.stdout))
}

fn wifi_tools(page: &Page) {
    let g = page.collapsible("Tools", false);
    g.note("Choose a Wi-Fi band, test your speed, or restart Wi-Fi and Bluetooth.");
    if cmd::present("omarchy-network-band")
        && let Some((band, available)) = cmd::output(&["omarchy-network-band"]).as_deref().and_then(parse_band)
        && !available.is_empty()
    {
        let mut options = vec![("auto".to_string(), "Automatic".to_string())];
        options.extend(available.iter().map(|b| (b.clone(), format!("{b} GHz"))));
        let (r, _) = widgets::choice_row(
            "Wi-Fi band",
            "Stick to one band on this network. 2.4 GHz reaches further; 5 and 6 GHz are faster.",
            options,
            &band,
            move |b| {
                cmd::run_async(&["omarchy-network-band", &b], |r| match r {
                    Ok(_) => window::toast("Band changed"),
                    Err(e) => window::toast(&format!("{e}")),
                });
            },
        );
        widgets::keywords("2.4 5ghz 6ghz frequency channel");
        g.add(&r);
    }
    if cmd::present("omarchy-network-speedtest") {
        let (r, _) = widgets::button_row("Speed test", "Measures download and upload for a few seconds each.", "Run test", |b| {
            b.set_sensitive(false);
            b.set_label("Testing…");
            let b = b.clone();
            cmd::background(
                || (measure("down"), measure("up")),
                move |(down, up)| {
                    b.set_sensitive(true);
                    b.set_label("Run test");
                    let fmt = |v: Option<f64>| v.map(|v| format!("{v:.0} Mbps")).unwrap_or_else(|| "no result".into());
                    window::toast(&format!("Download {} · Upload {}", fmt(down), fmt(up)));
                },
            );
        });
        widgets::keywords("bandwidth internet mbps download upload");
        g.add(&r);
    }
    if cmd::present("omarchy-restart-wifi") || cmd::present("omarchy-restart-bluetooth") {
        let controls = widgets::hbox(8);
        for (cmdname, label) in [("omarchy-restart-wifi", "Restart Wi-Fi"), ("omarchy-restart-bluetooth", "Restart Bluetooth")] {
            if cmd::present(cmdname) {
                let b = gtk::Button::with_label(label);
                b.connect_clicked(move |b| {
                    b.set_sensitive(false);
                    let b = b.clone();
                    cmd::run_async(&[cmdname], move |r| {
                        b.set_sensitive(true);
                        match r {
                            Ok(_) => window::toast("Restarted"),
                            Err(e) => window::toast(&format!("{e}")),
                        }
                    });
                });
                controls.append(&b);
            }
        }
        g.add(&widgets::row("Restart", "Try this when a connection or device stops responding.", Some(controls.upcast_ref())));
        widgets::keywords("fix reset reload radio adapter");
    }
}

// ----- Firewall -----

/// "22", "22/tcp" or "8000:8010/udp" as (port, protocol).
pub(crate) fn parse_rule(text: &str) -> Option<(String, String)> {
    let (port, proto) = match text.trim().split_once('/') {
        Some((p, pr)) => (p.trim(), pr.trim().to_lowercase()),
        None => (text.trim(), "any".to_string()),
    };
    (!port.is_empty() && ["tcp", "udp", "any"].contains(&proto.as_str())).then(|| (port.to_string(), proto))
}

/// One numbered rule from `ufw status numbered`: (number, port, action, from).
fn parse_ufw(text: &str) -> Vec<(String, String, String, String)> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix('[')?;
            let (n, rule) = rest.split_once(']')?;
            let n = n.trim();
            if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            // "22/tcp   ALLOW IN   Anywhere (v6)": the action is the first upper-case word after the port.
            let words: Vec<&str> = rule.split_whitespace().collect();
            let at = words.iter().position(|w| ["ALLOW", "DENY", "REJECT", "LIMIT"].contains(w))?;
            let port = words[..at].join(" ");
            let mut action = words[at].to_string();
            let mut from = at + 1;
            if let Some(dir) = words.get(at + 1).filter(|w| ["IN", "OUT", "FWD"].contains(w)) {
                action = format!("{action} {dir}");
                from += 1;
            }
            Some((n.to_string(), port, action, words[from..].join(" ")))
        })
        .collect()
}

/// Omarchy's firewall, ufw.
pub(crate) fn ufw_group(page: &Page) {
    if !cmd::present("ufw") {
        return;
    }
    let g = page.collapsible("Firewall", false);
    g.note("Other computers can only reach the ports you allow here.");
    let (can_edit, notice) = helper_notice("network", "changing the firewall");
    if let Some(n) = notice {
        g.top(&n);
    }
    let on = cmd::output(&["systemctl", "is-active", "ufw.service"]).is_some_and(|s| s == "active");
    let (r, sw) = widgets::switch_row("Block incoming connections", "Turns the firewall (ufw) on.", on, move |now| {
        if now != on {
            run_admin(
                &["firewall", if now { "enable" } else { "disable" }],
                None,
                if now { "Firewall is on" } else { "Firewall is off" },
                "network",
            );
        }
    });
    sw.set_sensitive(can_edit);
    widgets::keywords("firewall ufw block ports security");
    g.add(&r);
    if !can_edit {
        return;
    }
    let (wrapper, content) = widgets::disclosure("Rules", "What other computers may reach. Showing them asks for your password.");
    let list = widgets::vbox(0);
    list.add_css_class("rule-list");
    let load = gtk::Button::with_label("Show rules");
    load.set_halign(gtk::Align::Start);
    {
        let list = list.clone();
        load.connect_clicked(move |b| show_rules(Some(b), &list));
    }
    content.append(&load);
    content.append(&list);
    g.add(&wrapper);
    let (r, _) = widgets::entry_row(
        "Allow a port",
        "A port number, optionally with /tcp or /udp, e.g. 8080/tcp. Press Enter.",
        "",
        "8080/tcp",
        |t| {
            let t = t.trim().to_string();
            if t.is_empty() {
                return;
            }
            match parse_rule(&t) {
                Some((port, proto)) => run_admin(&["firewall", "allow", &port, &proto], None, "Port allowed", "network"),
                None => window::toast("Use a port like 8080 or 8080/tcp"),
            }
        },
    );
    widgets::keywords("open port allow rule");
    g.add(&r);
}

/// Read the rules (asks for the password) and list them, each with its own Remove.
fn show_rules(button: Option<&gtk::Button>, list: &gtk::Box) {
    if let Some(b) = button {
        b.set_sensitive(false);
    }
    let (button, list) = (button.cloned(), list.clone());
    cmd::background(
        || crate::backend::accounts::helper(&["firewall", "rules"], None).map_err(|e| e.to_string()),
        move |r| {
            if let Some(b) = &button {
                b.set_sensitive(true);
            }
            let out = match r {
                Ok(out) => out,
                Err(e) => return window::toast(&e),
            };
            if let Some(b) = &button {
                b.set_visible(false);
            }
            widgets::forget_rows(&list);
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            widgets::begin_section("network");
            let rules = parse_ufw(&out);
            if rules.is_empty() {
                let l = widgets::label(
                    if out.contains("inactive") { "The firewall is off." } else { "No rules: nothing is allowed in." },
                    "dim",
                );
                l.set_xalign(0.0);
                list.append(&l);
                return;
            }
            for (n, port, action, from) in rules {
                let desc = if from.is_empty() { action.clone() } else { format!("{action} · from {from}") };
                let desc = gtk::glib::markup_escape_text(&desc).to_string();
                let remove = widgets::confirm_button("Remove", "Remove rule?", {
                    let list = list.clone();
                    move |b| {
                        b.set_sensitive(false);
                        let (n, list) = (n.clone(), list.clone());
                        cmd::background(
                            move || {
                                crate::backend::accounts::helper(&["firewall", "delete", &n], None).map_err(|e| e.to_string())
                            },
                            move |r| match r {
                                // Numbers shift after a removal: read the list again.
                                Ok(_) => {
                                    window::toast("Rule removed");
                                    show_rules(None, &list);
                                }
                                Err(e) => window::toast(&e),
                            },
                        );
                    }
                });
                remove.set_valign(gtk::Align::Center);
                list.append(&widgets::row(&port, &desc, Some(remove.upcast_ref())));
            }
        },
    );
}

// ----- Tailscale -----

struct Tailnet {
    state: String,
    host: String,
    ip: String,
    peers: Vec<(String, String, bool)>,
}

fn parse_tailscale(json: &str) -> Option<Tailnet> {
    let v: Value = serde_json::from_str(json).ok()?;
    let me = v.get("Self");
    let text = |x: Option<&Value>, k: &str| x.and_then(|x| x.get(k)).and_then(Value::as_str).unwrap_or("").to_string();
    let mut peers: Vec<(String, String, bool)> = v
        .get("Peer")
        .and_then(Value::as_object)
        .map(|m| {
            m.values()
                .map(|p| {
                    let ip = p
                        .get("TailscaleIPs")
                        .and_then(Value::as_array)
                        .and_then(|a| a.first())
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    (text(Some(p), "HostName"), ip, p.get("Online").and_then(Value::as_bool).unwrap_or(false))
                })
                .collect()
        })
        .unwrap_or_default();
    peers.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    Some(Tailnet {
        state: text(Some(&v), "BackendState"),
        host: text(me, "HostName"),
        ip: me
            .and_then(|m| m.get("TailscaleIPs"))
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        peers,
    })
}

fn tailscale(page: &Page) {
    let g = page.collapsible("Tailscale", false);
    g.note("A private network between your own devices.");
    let Some(t) = cmd::output(&["tailscale", "status", "--json"]).as_deref().and_then(parse_tailscale) else {
        g.note("Tailscale isn't running. Start it with: sudo systemctl enable --now tailscaled");
        return;
    };
    let connected = t.state == "Running";
    let (r, _) = widgets::switch_row(
        "Tailscale",
        &gtk::glib::markup_escape_text(&if connected {
            format!("Connected as {} ({}).", t.host, t.ip)
        } else {
            format!("Not connected ({}).", t.state)
        }),
        connected,
        move |now| {
            if now == connected {
                return;
            }
            cmd::run_async(&["tailscale", if now { "up" } else { "down" }], move |r| {
                match r {
                    Ok(_) => window::toast(if now { "Tailscale connected" } else { "Tailscale disconnected" }),
                    Err(e) => {
                        window::toast(&format!("{e}. If it asks for permission, run once: sudo tailscale set --operator=$USER"))
                    }
                }
                window::rebuild("network");
            });
        },
    );
    widgets::keywords("vpn tailnet mesh wireguard");
    g.add(&r);
    if connected {
        for (host, ip, online) in t.peers.iter().take(12) {
            let (r, _) = widgets::info_row(host, &format!("{ip} · {}", if *online { "online" } else { "offline" }));
            widgets::keywords("tailscale device peer machine");
            g.add(&r);
        }
    }
}

// ----- NordVPN -----

fn nordvpn(page: &Page) {
    let g = page.collapsible("NordVPN", false);
    g.note("Connect through NordVPN's servers.");
    let status = cmd::output(&["nordvpn", "status"]).unwrap_or_default();
    let connected = status.lines().any(|l| l.trim().eq_ignore_ascii_case("status: connected"));
    let detail: Vec<String> = status
        .lines()
        .filter(|l| ["Server:", "Country:", "City:"].iter().any(|k| l.trim_start().starts_with(k)))
        .map(|l| l.trim().to_string())
        .collect();
    let desc =
        if connected { gtk::glib::markup_escape_text(&detail.join(" · ")).to_string() } else { "Not connected.".to_string() };
    let (r, _) = widgets::button_row("NordVPN", &desc, if connected { "Disconnect" } else { "Connect" }, move |b| {
        b.set_sensitive(false);
        b.set_label("Working…");
        cmd::run_async(&["nordvpn", if connected { "disconnect" } else { "connect" }], |r| {
            match r {
                Ok(_) => window::toast("Done"),
                Err(e) => window::toast(&format!("{e}")),
            }
            window::rebuild("network");
        });
    });
    widgets::keywords("vpn nord privacy");
    g.add(&r);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_output() {
        let (b, a) = parse_band("band\t6\navailable\t2.4 5 6\n").unwrap();
        assert_eq!(b, "6");
        assert_eq!(a, ["2.4", "5", "6"]);
        assert!(parse_band("nothing").is_none());
    }

    #[test]
    fn speed_is_the_later_mean() {
        assert_eq!(speed("100\n200\n300\n400\n"), Some(350.0));
        assert_eq!(speed(""), None);
    }

    #[test]
    fn dns_line() {
        assert_eq!(custom_servers("[Resolve]\nDNS=192.168.1.1 1.1.1.1\nFallbackDNS=9.9.9.9\n"), "192.168.1.1 1.1.1.1");
        assert_eq!(custom_servers("[Resolve]\n#DNS=\n"), "");
    }

    #[test]
    fn rules() {
        assert_eq!(parse_rule("22"), Some(("22".into(), "any".into())));
        assert_eq!(parse_rule("8080/TCP"), Some(("8080".into(), "tcp".into())));
        assert_eq!(parse_rule("80/icmp"), None);
        assert_eq!(parse_rule(""), None);
    }

    #[test]
    fn ufw_rules() {
        let out = "Status: active\n\n     To                         Action      From\n     --                         ------      ----\n[ 1] 22/tcp                     ALLOW IN    Anywhere\n[ 2] 8000:8010/udp              DENY IN     192.168.1.0/24\n[10] 22/tcp (v6)                ALLOW IN    Anywhere (v6)\n";
        let r = parse_ufw(out);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0], ("1".into(), "22/tcp".into(), "ALLOW IN".into(), "Anywhere".into()));
        assert_eq!(r[1].3, "192.168.1.0/24");
        assert_eq!(r[2].0, "10");
        assert_eq!(r[2].1, "22/tcp (v6)");
    }

    #[test]
    fn tailnet() {
        let t = parse_tailscale(
            r#"{"BackendState":"Running","Self":{"HostName":"a","TailscaleIPs":["100.1.1.1"]},"Peer":{"x":{"HostName":"z","Online":false,"TailscaleIPs":["100.1.1.3"]},"y":{"HostName":"b","Online":true,"TailscaleIPs":["100.1.1.2"]}}}"#,
        )
        .unwrap();
        assert_eq!(t.host, "a");
        assert_eq!(t.ip, "100.1.1.1");
        assert_eq!(t.peers[0].0, "b");
        assert_eq!(t.peers[1].0, "z");
    }
}
