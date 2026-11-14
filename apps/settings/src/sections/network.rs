//! Network beyond joining Wi-Fi: the way out to the internet (connections, VPN, DNS,
//! Tailscale and the firewall), drawn step by step, with VPN services, firewall rules
//! and tools a level down. Wi-Fi networks and Bluetooth stay on their own pages.

use crate::backend::{net, sysinfo};
use crate::sections::accounts::{helper_notice, run_admin};
use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::io::Write;
use std::process::{Command, Stdio};

pub fn build(page: &Page) {
    let pv = page.split();
    // The picture arrives a moment later; the page keeps its width meanwhile.
    pv.pane.set_size_request(520, -1);
    let test = gtk::Button::new();
    let speed_present = cmd::present("omarchy-network-speedtest");
    if speed_present {
        let inner = widgets::hbox(6);
        inner.append(&gtk::Image::from_icon_name("view-refresh-symbolic"));
        inner.append(&gtk::Label::new(Some("Test speed")));
        test.set_child(Some(&inner));
    }
    let head: Vec<&gtk::Widget> = if speed_present { vec![test.upcast_ref()] } else { vec![] };
    pv.head("Your way out", "How this computer reaches the internet, step by step.", &head);
    let body = widgets::vbox(14);
    pv.add(&body);
    let foot = pv.foot("emblem-ok-symbolic", "Reading the network…");

    connections(page);
    way_out(page);
    crate::platform::firewall_group(page);
    more(page);

    let speed_card = speed_card();
    if speed_present {
        let card = speed_card.clone();
        test.connect_clicked(move |b| run_speed_test(b, &card));
    }
    let (body2, foot2) = (body.clone(), foot.clone());
    cmd::background(Route::read, move |route| draw_route(&body2, &foot2, &route, &speed_card));
}

// ----- What the way out is -----

/// Everything between this computer and the internet, read once.
#[derive(Default)]
struct Route {
    host: String,
    /// The firewall's state, when there's one to ask.
    firewall: Option<bool>,
    /// (kind, name, address) of the connection in use: ("Wi-Fi", "Home-5G", "192.168.1.24").
    link: Option<(String, String, String)>,
    router: String,
    /// The VPN tunnel in use.
    vpn: Option<String>,
    dns: Vec<String>,
    dns_provider: String,
    /// NetworkManager's check: full, limited, portal or none.
    internet: String,
    tailnet: Option<Tailnet>,
}

impl Route {
    fn read() -> Route {
        let status = cmd::output(&["nmcli", "-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"]).unwrap_or_default();
        let link = status.lines().map(net::split_terse).find(|f| {
            f.len() >= 4 && f[2] == "connected" && matches!(f[1].as_str(), "wifi" | "ethernet")
        });
        let (link, router) = match link {
            Some(f) => {
                let details = net::details(&f[0]);
                let get = |k: &str| details.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).unwrap_or_default();
                let kind = if f[1] == "wifi" { "Wi-Fi" } else { "Ethernet" };
                (Some((kind.to_string(), f[3].clone(), get("IP address"))), get("Gateway"))
            }
            None => (None, String::new()),
        };
        let firewall = ["ufw.service", "firewall.service"].iter().find_map(|u| {
            let s = cmd::output(&["systemctl", "is-active", u]).unwrap_or_default();
            match s.trim() {
                "active" => Some(true),
                "inactive" | "failed" => Some(false),
                _ => None,
            }
        });
        let vpn = active_vpn();
        Route {
            host: sysinfo::hostname(),
            firewall,
            link,
            router,
            vpn,
            dns: dns_servers(&cmd::output(&["resolvectl", "dns"]).unwrap_or_default()),
            dns_provider: cmd::output(&["omarchy-dns"]).unwrap_or_default().trim().to_string(),
            internet: cmd::output(&["nmcli", "networking", "connectivity"]).unwrap_or_default().trim().to_string(),
            tailnet: cmd::present("tailscale")
                .then(|| cmd::output(&["tailscale", "status", "--json"]))
                .flatten()
                .as_deref()
                .and_then(parse_tailscale),
        }
    }
}

/// The DNS servers `resolvectl dns` lists: the global ones, else each link's.
fn dns_servers(text: &str) -> Vec<String> {
    let mut global = Vec::new();
    let mut links = Vec::new();
    for line in text.lines() {
        let Some((who, servers)) = line.split_once(':').filter(|(w, _)| w.starts_with("Global") || w.starts_with("Link")) else {
            continue;
        };
        // "Link 2 (wlo1): 192.168.1.1": the address after the first ": ".
        let servers: Vec<String> = servers.split_whitespace().map(str::to_string).collect();
        if who.starts_with("Global") {
            global.extend(servers);
        } else {
            for s in servers {
                if !links.contains(&s) {
                    links.push(s);
                }
            }
        }
    }
    if global.is_empty() { links } else { global }
}

/// The VPN connected now: NordVPN's, or a NetworkManager VPN or WireGuard tunnel.
fn active_vpn() -> Option<String> {
    if cmd::present("nordvpn") {
        let status = cmd::output(&["nordvpn", "status"]).unwrap_or_default();
        if status.lines().any(|l| l.trim().eq_ignore_ascii_case("status: connected")) {
            let city = status.lines().find_map(|l| l.trim().strip_prefix("City:").map(|c| c.trim().to_string()));
            return Some(match city {
                Some(c) => format!("NordVPN · {c}"),
                None => "NordVPN".into(),
            });
        }
    }
    let active = cmd::output(&["nmcli", "-t", "-f", "NAME,TYPE", "connection", "show", "--active"]).unwrap_or_default();
    active
        .lines()
        .map(net::split_terse)
        .find(|f| {
            f.len() >= 2
                && matches!(f[1].as_str(), "vpn" | "wireguard")
                && !f[0].to_lowercase().contains("tailscale")
                && f[0] != "nordlynx"
        })
        .map(|f| f[0].clone())
}

// ----- The picture -----

/// One step on the way out: a round icon, a name and a line, joined to the next.
fn hop(icon: &str, title: &str, line: &str, class: &str) -> gtk::Box {
    let row = widgets::hbox(12);
    row.add_css_class("cn-nw-hop");
    if !class.is_empty() {
        row.add_css_class(class);
    }
    let node = widgets::hbox(0);
    node.add_css_class("cn-nw-node");
    node.set_valign(gtk::Align::Start);
    node.set_halign(gtk::Align::Start);
    let i = gtk::Image::from_icon_name(icon);
    i.set_halign(gtk::Align::Center);
    i.set_valign(gtk::Align::Center);
    i.set_margin_start(9);
    i.set_margin_end(9);
    node.append(&i);
    row.append(&node);
    let text = widgets::vbox(1);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(title, "cn-strong");
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&t);
    if !line.is_empty() {
        let l = widgets::label(line, "dim");
        l.set_wrap(true);
        l.set_xalign(0.0);
        text.append(&l);
    }
    row.append(&text);
    row
}

fn wire(class: &str) -> gtk::Box {
    let w = widgets::hbox(0);
    w.set_halign(gtk::Align::Start);
    w.add_css_class("cn-nw-wire");
    if !class.is_empty() {
        w.add_css_class(class);
    }
    w
}

fn draw_route(body: &gtk::Box, foot: &gtk::Label, r: &Route, speed: &gtk::Box) {
    let row = widgets::hbox(14);
    let path = widgets::vbox(0);
    path.add_css_class("cn-card");
    path.add_css_class("cn-nw-path");
    path.set_hexpand(true);
    let fw = match r.firewall {
        Some(true) => "Firewall on: only the ports you allow get in",
        Some(false) => "Firewall off",
        None => "This computer",
    };
    path.append(&hop("computer-symbolic", &r.host, fw, "me"));
    let tunnel = if r.vpn.is_some() { "tunnel" } else { "" };
    match &r.link {
        Some((kind, name, ip)) => {
            path.append(&wire(""));
            let icon = if kind == "Wi-Fi" { "network-wireless-symbolic" } else { "network-wired-symbolic" };
            let line = if ip.is_empty() { kind.clone() } else { format!("{kind} · {ip}") };
            path.append(&hop(icon, name, &line, ""));
            if !r.router.is_empty() {
                path.append(&wire(""));
                path.append(&hop("network-server-symbolic", "Router", &r.router, ""));
            }
            if let Some(vpn) = &r.vpn {
                path.append(&wire(""));
                path.append(&hop("changes-prevent-symbolic", vpn, "VPN: a private tunnel from here on", "vpn"));
            }
            let dns = if r.dns.is_empty() {
                String::new()
            } else {
                let who = match r.dns_provider.as_str() {
                    "" | "DHCP" => "From the network".to_string(),
                    "Custom" => "Your own servers".to_string(),
                    p => p.to_string(),
                };
                format!("{who} · {}", r.dns.iter().take(2).cloned().collect::<Vec<_>>().join(", "))
            };
            if !dns.is_empty() {
                let side = widgets::hbox(10);
                side.add_css_class("cn-nw-dns");
                let i = gtk::Image::from_icon_name("system-search-symbolic");
                i.add_css_class("dim");
                side.append(&i);
                let t = widgets::vbox(1);
                t.append(&widgets::label("DNS looks up names", "cn-strong"));
                let l = widgets::label(&dns, "dim");
                l.set_wrap(true);
                l.set_xalign(0.0);
                t.append(&l);
                side.append(&t);
                path.append(&side);
            }
            path.append(&wire(tunnel));
            let (line, ok) = match r.internet.as_str() {
                "full" => ("Reachable".to_string(), true),
                "portal" => ("Behind a sign-in page: open a browser to sign in".to_string(), false),
                "limited" => ("Limited: the network is there, the internet isn't".to_string(), false),
                "none" => ("Not reachable".to_string(), false),
                _ => (String::new(), true),
            };
            path.append(&hop("network-workgroup-symbolic", "Internet", &line, if ok { "end" } else { "bad" }));
        }
        None => {
            path.append(&wire("off"));
            path.append(&hop("network-offline-symbolic", "Not connected", "Join a Wi-Fi network or plug in a cable.", "bad"));
        }
    }
    row.append(&path);

    let side = widgets::vbox(14);
    side.set_size_request(190, -1);
    side.append(speed);
    if let Some(t) = &r.tailnet {
        side.append(&tailnet_card(t));
    }
    row.append(&side);
    body.append(&row);

    foot.set_text(&match (&r.link, &r.vpn) {
        (None, _) => "Not connected to a network.".to_string(),
        (Some(_), Some(v)) => format!("Everything leaving {} goes through {v}. Devices nearby, like a printer, still work.", r.host),
        (Some(_), None) => "Straight out through your router: no VPN is on.".to_string(),
    });
}

fn tailnet_card(t: &Tailnet) -> gtk::Box {
    let c = widgets::vbox(8);
    c.add_css_class("cn-card");
    c.add_css_class("cn-nw-tail");
    let head = widgets::hbox(8);
    let title = widgets::label("Tailscale", "cn-strong");
    title.set_hexpand(true);
    head.append(&title);
    let on = t.state == "Running";
    let chip = widgets::label(if on { "On" } else { "Off" }, "cn-chip");
    if on {
        chip.add_css_class("ok");
    }
    head.append(&chip);
    c.append(&head);
    if on {
        let mut list = vec![(t.host.clone(), format!("{} · this one", t.ip), true)];
        list.extend(t.peers.iter().take(5).cloned());
        for (host, ip, online) in list {
            let r = widgets::hbox(8);
            let dot = widgets::hbox(0);
            dot.add_css_class("cn-nw-dot");
            if online {
                dot.add_css_class("on");
            }
            dot.set_valign(gtk::Align::Center);
            r.append(&dot);
            let tx = widgets::vbox(0);
            let h = widgets::label(&host, "cn-tail-host");
            h.set_ellipsize(gtk::pango::EllipsizeMode::End);
            tx.append(&h);
            let l = widgets::label(if online { ip.as_str() } else { "Offline" }, "cn-note");
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            tx.append(&l);
            r.append(&tx);
            c.append(&r);
        }
        if t.peers.len() > 5 {
            c.append(&widgets::label(&format!("{} more", t.peers.len() - 5), "cn-note"));
        }
    } else {
        let l = widgets::label(&format!("Not connected ({}).", t.state), "dim");
        l.set_wrap(true);
        l.set_xalign(0.0);
        c.append(&l);
    }
    c
}

thread_local! {
    /// The last speed test this session: (down, up) in Mb/s, and when.
    static SPEED: RefCell<Option<(Option<f64>, Option<f64>, i64)>> = const { RefCell::new(None) };
}

fn speed_card() -> gtk::Box {
    let c = widgets::vbox(6);
    c.add_css_class("cn-card");
    c.add_css_class("cn-nw-speed");
    fill_speed(&c);
    c
}

fn fill_speed(c: &gtk::Box) {
    while let Some(w) = c.first_child() {
        c.remove(&w);
    }
    c.append(&widgets::label("Last speed test", "cn-note"));
    let last = SPEED.with(|s| *s.borrow());
    let nums = widgets::hbox(18);
    let fmt = |v: Option<f64>| v.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into());
    let (down, up, when) = last.unwrap_or((None, None, 0));
    for (v, what) in [(fmt(down), "Mb/s down"), (fmt(up), "Mb/s up")] {
        let b = widgets::vbox(0);
        b.append(&widgets::label(&v, "cn-nw-speed-num"));
        b.append(&widgets::label(what, "cn-note"));
        nums.append(&b);
    }
    c.append(&nums);
    let line = if when == 0 {
        if cmd::present("omarchy-network-speedtest") { "Not tested yet" } else { "Needs omarchy-network-speedtest" }.to_string()
    } else {
        crate::units::ago(gtk::glib::real_time() / 1_000_000 - when)
    };
    c.append(&widgets::label(&line, "dim"));
}

fn run_speed_test(b: &gtk::Button, card: &gtk::Box) {
    b.set_sensitive(false);
    let (b, card) = (b.clone(), card.clone());
    if let Some(l) = card.last_child().and_downcast::<gtk::Label>() {
        l.set_text("Testing… about 16 seconds");
    }
    cmd::background(
        || (measure("down"), measure("up")),
        move |(down, up)| {
            b.set_sensitive(true);
            SPEED.with(|s| *s.borrow_mut() = Some((down, up, gtk::glib::real_time() / 1_000_000)));
            fill_speed(&card);
            if down.is_none() && up.is_none() {
                window::toast("The speed test didn't get a result");
            }
        },
    );
}

// ----- Connections -----

fn connections(page: &Page) {
    if !cmd::present("nmcli") {
        return;
    }
    let g = page.group("Connections");
    let text = cmd::output(&["nmcli", "-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"]).unwrap_or_default();
    let devices: Vec<Vec<String>> = text.lines().map(net::split_terse).filter(|f| f.len() >= 4).collect();
    let ip = |dev: &str| {
        cmd::output(&["nmcli", "-g", "IP4.ADDRESS", "device", "show", dev])
            .unwrap_or_default()
            .lines()
            .next()
            .map(|a| a.split('/').next().unwrap_or(a).to_string())
            .unwrap_or_default()
    };
    // In a box: a label straight in a card row gets the card's label padding.
    let state_chip = |text: &str, ok: bool| {
        let l = widgets::label(text, if ok { "cn-chip" } else { "row-value" });
        if ok {
            l.add_css_class("ok");
        }
        let b = widgets::hbox(0);
        b.append(&l);
        b
    };
    // Wi-Fi: the network in use, or why there isn't one.
    let wifi = devices.iter().find(|f| f[1] == "wifi");
    if let Some(f) = wifi {
        let connected = f[2] == "connected";
        let desc = if connected { format!("{} · {}", f[3], ip(&f[0])) } else { "Not connected".into() };
        let end = state_chip(if connected { "Connected" } else { "Off" }, connected);
        let r = widgets::icon_row("network-wireless-symbolic", "Wi-Fi", &gtk::glib::markup_escape_text(&desc), Some(end.upcast_ref()));
        widgets::keywords("wifi wireless ip address");
        clickable(&r, || window::navigate("wifi"));
        g.add(&r);
    }
    // Wired: each port, or that a cable or dock adds one.
    let wired: Vec<&Vec<String>> = devices.iter().filter(|f| f[1] == "ethernet").collect();
    for f in &wired {
        let connected = f[2].starts_with("connected");
        let desc = if connected { format!("{} · {}", f[3], ip(&f[0])) } else { "Plug in a cable".to_string() };
        let end = state_chip(if connected { "Connected" } else { "Not plugged in" }, connected);
        let r = widgets::icon_row("network-wired-symbolic", "Ethernet", &gtk::glib::markup_escape_text(&desc), Some(end.upcast_ref()));
        widgets::keywords("ip address ethernet wired lan cable");
        g.add(&r);
    }
    if wired.is_empty() {
        let end = state_chip("Not plugged in", false);
        let r = widgets::icon_row("network-wired-symbolic", "Ethernet", "Plug in a cable or a dock.", Some(end.upcast_ref()));
        widgets::keywords("ip address ethernet wired lan cable");
        g.add(&r);
    }
}

/// A whole row that opens something.
fn clickable(row: &gtk::Box, f: impl Fn() + 'static) {
    row.add_css_class("cn-click");
    row.set_cursor_from_name(Some("pointer"));
    let click = gtk::GestureClick::new();
    click.connect_released(move |_, _, _, _| f());
    row.add_controller(click);
}

// ----- On the way out: VPN, DNS, Tailscale -----

fn way_out(page: &Page) {
    let g = page.group("On the way out");
    vpn_row(&g);
    dns(&g);
    if cmd::present("tailscale") {
        tailscale(&g);
    }
}

fn tiled(r: gtk::Box, icon: &str) -> gtk::Box {
    r.prepend(&widgets::tile(icon));
    r.add_css_class("with-tile");
    r
}

/// A switch for the VPN the Control Center uses: NordVPN's app, or a NetworkManager tunnel.
fn vpn_row(g: &widgets::Group) {
    let active = active_vpn();
    let (nm, chosen) = if cmd::present("nmcli") { crate::sections::vpn_accounts::nm_connections() } else { (Vec::new(), None) };
    let nord = cmd::present("nordvpn");
    // Which one the switch turns on: the Control Center's choice, else NordVPN, else the first tunnel.
    let target: Option<(String, Option<String>)> = match chosen.as_deref() {
        Some(c) if c.starts_with("nm:") => {
            let uuid = &c[3..];
            nm.iter().find(|(_, u)| u == uuid).map(|(n, u)| (n.clone(), Some(u.clone())))
        }
        Some("nord") if nord => Some(("NordVPN".into(), None)),
        _ => None,
    }
    .or_else(|| nord.then(|| ("NordVPN".to_string(), None)))
    .or_else(|| nm.first().map(|(n, u)| (n.clone(), Some(u.clone()))));
    let Some((name, uuid)) = target else {
        if active.is_none() {
            let end = widgets::hbox(0);
            end.append(&widgets::label("None set up", "row-value"));
            let r = tiled(widgets::row("VPN", "Add one under VPN services below.", Some(end.upcast_ref())), "changes-prevent-symbolic");
            widgets::keywords("vpn nordvpn wireguard openvpn tunnel");
            g.add(&r);
        }
        return;
    };
    let on = active.is_some();
    let desc = active.clone().unwrap_or_else(|| format!("Off · {name}"));
    let (r, sw) = widgets::switch_row("VPN", &gtk::glib::markup_escape_text(&desc), on, move |now| {
        if now == on {
            return;
        }
        let args: Vec<String> = match (&uuid, now) {
            (Some(u), true) => vec!["nmcli".into(), "connection".into(), "up".into(), "uuid".into(), u.clone()],
            (Some(u), false) => vec!["nmcli".into(), "connection".into(), "down".into(), "uuid".into(), u.clone()],
            (None, true) => vec!["nordvpn".into(), "connect".into()],
            (None, false) => vec!["nordvpn".into(), "disconnect".into()],
        };
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        cmd::run_async(&args, move |r| {
            match r {
                Ok(_) => window::toast(if now { "VPN connected" } else { "VPN disconnected" }),
                Err(e) => window::toast(&format!("{e}")),
            }
            window::rebuild("network");
        });
    });
    sw.set_tooltip_text(Some("The Control Center's VPN tile does the same"));
    widgets::keywords("vpn nordvpn nord privacy wireguard openvpn tunnel connect");
    g.add(&tiled(r, "changes-prevent-symbolic"));
}

// ----- DNS -----

fn dns(g: &widgets::Group) {
    if !cmd::present("omarchy-dns") {
        return;
    }
    let current = cmd::output(&["omarchy-dns"]).unwrap_or_default();
    let options = widgets::opts(&[
        ("Cloudflare", "Cloudflare"),
        ("Google", "Google"),
        ("DHCP", "From the network"),
        ("Custom", "Your own servers"),
    ]);
    // Your own servers, shown once that's the choice.
    let in_use = if current == "Custom" {
        custom_servers(&std::fs::read_to_string("/etc/systemd/resolved.conf").unwrap_or_default())
    } else {
        String::new()
    };
    let (custom, _) = widgets::entry_row(
        "Custom DNS servers",
        "Addresses with spaces between. Press Enter to use them.",
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
    custom.set_visible(current == "Custom");
    let (r, _) = widgets::choice_row("DNS", "Which servers look up website names. Asks for your password.", options, &current, {
        let (current, custom) = (current.clone(), custom.clone());
        move |p| {
            if p == current {
                return;
            }
            if p == "Custom" {
                // Applied once the servers are typed in.
                custom.set_visible(true);
                return;
            }
            custom.set_visible(false);
            let name = p.clone();
            cmd::run_async(&["omarchy-dns", &p], move |r| {
                match r {
                    Ok(_) => window::toast(&format!("DNS set to {name}")),
                    Err(e) => window::toast(&format!("{e}")),
                }
                window::rebuild("network");
            });
        }
    });
    widgets::keywords("dns provider resolver nameserver cloudflare google dhcp privacy");
    g.add(&tiled(r, "system-search-symbolic"));
    g.add(&custom);
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

// ----- Tailscale -----

fn tailscale(g: &widgets::Group) {
    let Some(t) = cmd::output(&["tailscale", "status", "--json"]).as_deref().and_then(parse_tailscale) else {
        let r = widgets::row("Tailscale", "Isn't running. Start it with: sudo systemctl enable --now tailscaled", None);
        widgets::keywords("vpn tailnet mesh wireguard");
        g.add(&tiled(r, "emblem-shared-symbolic"));
        return;
    };
    let connected = t.state == "Running";
    let (r, _) = widgets::switch_row(
        "Tailscale",
        &gtk::glib::markup_escape_text(&if connected {
            format!("Your own devices, from anywhere · {} ({})", t.host, t.ip)
        } else {
            format!("Your own devices, from anywhere · off ({})", t.state)
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
    widgets::keywords("vpn tailnet mesh wireguard tailscale device peer machine");
    g.add(&tiled(r, "emblem-shared-symbolic"));
}

// ----- A level down: VPN services and tools -----

fn more(page: &Page) {
    let g = page.group("");
    let count = crate::sections::vpn_accounts::count();
    let (r, _) = widgets::nav_row(
        Some("changes-prevent-symbolic"),
        "VPN services",
        "Add one, sign in, and pick the Control Center's.",
        &if count == 0 { String::new() } else { count.to_string() },
        crate::sections::vpn_accounts::subpage,
    );
    widgets::keywords("vpn nordvpn expressvpn proton pia private internet access mullvad surfshark windscribe wireguard openvpn add");
    g.add(&r);
    let tools: Vec<&str> = [
        ("omarchy-network-band", "Wi-Fi band"),
        ("omarchy-network-speedtest", "speed test"),
        ("omarchy-restart-wifi", "restart Wi-Fi"),
    ]
    .iter()
    .filter(|(c, _)| cmd::present(c))
    .map(|(_, l)| *l)
    .collect();
    if !tools.is_empty() || cmd::present("omarchy-restart-bluetooth") {
        let mut desc = tools.join(", ");
        if let Some(f) = desc.get(..1) {
            desc = f.to_uppercase() + &desc[1..];
        }
        let (r, _) = widgets::nav_row(Some("preferences-other-symbolic"), "Tools", &desc, "", || {
            widgets::subpage("Tools", 560, wifi_tools);
        });
        widgets::keywords("tools band speed test restart wifi bluetooth fix reset 2.4 5ghz 6ghz bandwidth mbps");
        g.add(&r);
    }
}

// ----- Wi-Fi tools -----

/// `omarchy-network-band`: the band chosen (`selected`, else the one in use) and those available.
fn parse_band(text: &str) -> Option<(String, Vec<String>)> {
    let (mut band, mut selected, mut available) = (None, None, Vec::new());
    for line in text.lines() {
        match line.split_once('\t') {
            Some(("band", v)) => band = Some(v.trim().to_string()),
            Some(("selected", v)) => selected = Some(v.trim().to_string()),
            Some(("available", v)) => available = v.split_whitespace().map(str::to_string).collect(),
            _ => {}
        }
    }
    selected.or(band).map(|b| (b, available))
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
    let g = page.group("");
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
                    SPEED.with(|s| *s.borrow_mut() = Some((down, up, gtk::glib::real_time() / 1_000_000)));
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
        g.add(&widgets::stacked_row("Restart", "Try this when a connection or device stops responding.", controls.upcast_ref()));
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

/// Omarchy's firewall, ufw: the switch, and its rules a level down.
pub(crate) fn ufw_group(page: &Page) {
    if !cmd::present("ufw") {
        return;
    }
    let g = page.group("Firewall");
    let (can_edit, notice) = helper_notice("network", "changing the firewall");
    if let Some(n) = notice {
        g.top(&n);
    }
    let on = cmd::output(&["systemctl", "is-active", "ufw.service"]).is_some_and(|s| s == "active");
    let (r, sw) = widgets::switch_row("Block incoming connections", "Only the ports you allow get in.", on, move |now| {
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
    g.add(&tiled(r, "security-high-symbolic"));
    if !can_edit {
        return;
    }
    let (r, _) = widgets::nav_row(Some("view-list-symbolic"), "Firewall rules", "What other computers may reach.", "", || {
        widgets::subpage("Firewall rules", 560, ufw_rules);
    });
    widgets::keywords("firewall rules open port allow rule remove");
    g.add(&r);
}

/// The rules (showing them asks for the password) and a line to allow a port.
fn ufw_rules(page: &Page) {
    let g = page.group("");
    g.note("Other computers can only reach the ports you allow here. Showing the rules asks for your password.");
    let list = widgets::vbox(0);
    list.add_css_class("rule-list");
    let load = gtk::Button::with_label("Show rules");
    load.set_halign(gtk::Align::Start);
    load.set_margin_start(14);
    load.set_margin_top(8);
    load.set_margin_bottom(8);
    {
        let list = list.clone();
        load.connect_clicked(move |b| show_rules(Some(b), &list));
    }
    g.add(&load);
    g.add(&list);
    let (r, _) = widgets::entry_row(
        "Allow a port",
        "Like 8080, or 8080/tcp. Press Enter.",
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


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_output() {
        let (b, a) = parse_band("band\t6\navailable\t2.4 5 6\n").unwrap();
        assert_eq!(b, "6");
        assert_eq!(parse_band("band\t6\navailable\t2.4 5 6\nselected\tauto\n").unwrap().0, "auto");
        assert_eq!(a, ["2.4", "5", "6"]);
        assert!(parse_band("nothing").is_none());
    }

    #[test]
    fn resolver_servers() {
        let text = "Global:\nLink 4 (incusbr0):\nLink 2 (wlo1): 192.168.1.1\nLink 3 (eth0): 192.168.1.1 9.9.9.9\n";
        assert_eq!(dns_servers(text), ["192.168.1.1", "9.9.9.9"]);
        assert_eq!(dns_servers("Global: 1.1.1.1 1.0.0.1\nLink 2 (wlo1): 192.168.1.1\n"), ["1.1.1.1", "1.0.0.1"]);
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
