//! VPN services, under Online Accounts: the ones set up, and adding one. The Control
//! Center's VPN tile then works with whatever is set up here.
//!
//! - Services with their own Linux tool (NordVPN, ExpressVPN, Private Internet Access,
//!   Mullvad, Proton VPN): install it, then sign in the way that tool does.
//! - Anything else (Surfshark, Windscribe, Proton's WireGuard files, your own server): a
//!   WireGuard (`.conf`) or OpenVPN (`.ovpn`) file imported into NetworkManager.
//!
//! Secrets (an activation code, an account number) go to the tool on stdin or in a
//! private temporary file removed straight after, never on a command line.

use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use std::io::Write;

thread_local! { static ADDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

pub fn adding() -> bool {
    ADDING.with(|a| a.get())
}

fn show_add(on: bool) {
    ADDING.with(|a| a.set(on));
    window::rebuild("online-accounts");
}

/// How a service is installed.
#[derive(Clone, Copy)]
enum Install {
    /// From the AUR, then these commands (as root, in the same terminal).
    Aur(&'static str, &'static str),
    /// From the provider's page.
    Page(&'static str),
    /// Nothing to install: a file into NetworkManager.
    None,
}

/// How you sign in once it's installed.
#[derive(Clone, Copy)]
enum SignIn {
    /// A command that asks in a terminal (or opens the browser).
    Terminal(&'static str),
    /// A code typed here, handed to the tool.
    Code { label: &'static str, run: fn(&str) -> anyhow::Result<()> },
    /// The provider's own app.
    App(&'static str),
    /// Import a WireGuard or OpenVPN file (from `page`).
    File(Option<&'static str>),
}

#[derive(Clone, Copy)]
struct Provider {
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    /// The program that shows it's installed.
    tool: Option<&'static str>,
    install: Install,
    sign_in: SignIn,
    /// Signing out (when the tool has a command for it).
    sign_out: Option<&'static [&'static str]>,
}

const PROVIDERS: &[Provider] = &[
    Provider {
        id: "nord",
        name: "NordVPN",
        desc: "Its own app; sign in in the browser",
        tool: Some("nordvpn"),
        install: Install::Aur("nordvpn-bin", "sudo systemctl enable --now nordvpnd && sudo usermod -aG nordvpn \"$USER\""),
        sign_in: SignIn::Terminal("nordvpn login"),
        sign_out: Some(&["nordvpn", "logout", "--persist-token"]),
    },
    Provider {
        id: "express",
        name: "ExpressVPN",
        desc: "Its own app; sign in with your activation code",
        tool: Some("expressvpnctl"),
        install: Install::Aur("expressvpn", "true"),
        sign_in: SignIn::Code { label: "Activation code", run: express_login },
        sign_out: Some(&["expressvpnctl", "logout"]),
    },
    Provider {
        id: "proton",
        name: "Proton VPN",
        desc: "Its own app (beta), or a WireGuard file from your Proton account",
        tool: Some("protonvpn"),
        install: Install::Page("https://protonvpn.com/support/linux-cli"),
        sign_in: SignIn::Terminal("protonvpn signin"),
        sign_out: Some(&["protonvpn", "signout"]),
    },
    Provider {
        id: "pia",
        name: "Private Internet Access",
        desc: "Its own app; sign in there once",
        tool: Some("piactl"),
        install: Install::Aur("piavpn-bin", "true"),
        sign_in: SignIn::App("/opt/piavpn/bin/pia-client"),
        sign_out: Some(&["piactl", "logout"]),
    },
    Provider {
        id: "mullvad",
        name: "Mullvad",
        desc: "Its own app; sign in with your account number",
        tool: Some("mullvad"),
        install: Install::Aur("mullvad-vpn-bin", "sudo systemctl enable --now mullvad-daemon"),
        sign_in: SignIn::Code { label: "Account number", run: mullvad_login },
        sign_out: Some(&["mullvad", "account", "logout"]),
    },
    Provider {
        id: "surfshark",
        name: "Surfshark",
        desc: "A WireGuard file from your Surfshark account",
        tool: None,
        install: Install::None,
        sign_in: SignIn::File(Some("https://my.surfshark.com/vpn/manual-setup/main/wireguard")),
        sign_out: None,
    },
    Provider {
        id: "windscribe",
        name: "Windscribe",
        desc: "A WireGuard file from your Windscribe account",
        tool: None,
        install: Install::None,
        sign_in: SignIn::File(Some("https://windscribe.com/getconfig/wireguard")),
        sign_out: None,
    },
    Provider {
        id: "proton-file",
        name: "Proton VPN (WireGuard)",
        desc: "A WireGuard file from your Proton account, no app needed",
        tool: None,
        install: Install::None,
        sign_in: SignIn::File(Some("https://account.protonvpn.com/downloads")),
        sign_out: None,
    },
    Provider {
        id: "other",
        name: "Other",
        desc: "Any WireGuard (.conf) or OpenVPN (.ovpn) file",
        tool: None,
        install: Install::None,
        sign_in: SignIn::File(None),
        sign_out: None,
    },
];

/// ExpressVPN: the code goes in a private file (it reads it from there), removed after.
fn express_login(code: &str) -> anyhow::Result<()> {
    let dir = gtk::glib::user_runtime_dir();
    let path = dir.join(format!("nyx-expressvpn-{}", std::process::id()));
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
        f.write_all(code.trim().as_bytes())?;
    }
    let r = std::process::Command::new("expressvpnctl").arg("login").arg(&path).output();
    let _ = std::fs::remove_file(&path);
    let out = r?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    // Lets the tile connect without ExpressVPN's own window open.
    let _ = std::process::Command::new("expressvpnctl").args(["background", "enable"]).output();
    Ok(())
}

/// Mullvad: the account number on stdin.
fn mullvad_login(number: &str) -> anyhow::Result<()> {
    let mut child = std::process::Command::new("mullvad")
        .args(["account", "login"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        writeln!(stdin, "{}", number.trim().replace(' ', ""))?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// NetworkManager's VPN and WireGuard connections (Tailscale's aside): (name, uuid, kind).
fn nm_vpns() -> Vec<(String, String, String)> {
    let Some(out) = cmd::output(&["nmcli", "-t", "-f", "NAME,UUID,TYPE", "connection", "show"]) else { return Vec::new() };
    out.lines()
        .filter_map(|l| {
            // `nmcli -t` escapes `:` as `\:`.
            let mut f = Vec::new();
            let mut cur = String::new();
            let mut esc = false;
            for c in l.chars() {
                match (esc, c) {
                    (true, c) => {
                        cur.push(c);
                        esc = false;
                    }
                    (false, '\\') => esc = true,
                    (false, ':') => f.push(std::mem::take(&mut cur)),
                    (false, c) => cur.push(c),
                }
            }
            f.push(cur);
            let (name, uuid, kind) = (f.first()?, f.get(1)?, f.get(2)?);
            // Tailscale's and NordVPN's own tunnels aren't services of their own.
            ((kind == "wireguard" || kind == "vpn") && !name.to_lowercase().contains("tailscale") && name != "nordlynx")
                .then(|| (name.clone(), uuid.clone(), if kind == "wireguard" { "WireGuard".into() } else { "OpenVPN".into() }))
        })
        .collect()
}

/// `~/.config/nyx/vpn.toml`: which VPN the Control Center shows.
fn config_path() -> std::path::PathBuf {
    gtk::glib::user_config_dir().join("nyx/vpn.toml")
}

fn chosen() -> Option<String> {
    let text = std::fs::read_to_string(config_path()).ok()?;
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == "default").then(|| v.trim().trim_matches('"').to_string())
    })
}

fn set_chosen(id: &str) {
    let text = format!("# The VPN the Control Center shows. Set in Settings › Online Accounts › VPN.\ndefault = \"{id}\"\n");
    if let Err(e) = cmd::atomic_write(&config_path(), text) {
        window::toast(&format!("Couldn't save: {e}"));
    }
}

/// The VPN group on the Online Accounts page.
pub fn build(page: &Page) {
    let g = page.group("VPN");
    g.note("The Control Center's VPN tile shows the one chosen here.");
    // Every service set up, as (id, name), the way the Control Center names them.
    let mut services: Vec<(String, String)> = PROVIDERS
        .iter()
        .filter(|p| p.tool.is_some_and(cmd::present))
        .map(|p| (p.id.to_string(), if p.id == "pia" { "Private Internet Access".to_string() } else { p.name.to_string() }))
        .collect();
    services.extend(nm_vpns().into_iter().map(|(name, uuid, _)| (format!("nm:{uuid}"), name)));
    if services.len() > 1 {
        let current = chosen().filter(|c| services.iter().any(|(id, _)| id == c)).unwrap_or_else(|| services[0].0.clone());
        let (r, _) = widgets::choice_row("Show in the Control Center", "The VPN its tile connects.", services.clone(), &current, |id| {
            set_chosen(&id)
        });
        g.add(&r);
    }
    let mut any = false;
    for p in PROVIDERS.iter().filter(|p| p.tool.is_some_and(cmd::present)) {
        any = true;
        let controls = widgets::hbox(6);
        if let Some(args) = p.sign_out {
            let name = p.name;
            let b = widgets::confirm_button("Sign out", "Click again to sign out", move |_| {
                cmd::run_async(args, move |r| match r {
                    Ok(_) => window::toast(&format!("Signed out of {name}")),
                    Err(e) => window::toast(&format!("{name}: {e}")),
                });
            });
            controls.append(&b);
        }
        let prov = *p;
        let b = gtk::Button::with_label("Sign in");
        b.set_tooltip_text(Some(&format!("Sign in to {} again", p.name)));
        b.connect_clicked(move |_| sign_in(prov));
        controls.append(&b);
        g.add(&widgets::row(p.name, "Installed; connect from the Control Center", Some(controls.upcast_ref())));
    }
    for (name, uuid, kind) in nm_vpns() {
        any = true;
        let n2 = name.clone();
        let del = widgets::confirm_button("Remove", "Click again to remove", move |_| {
            let n3 = n2.clone();
            cmd::run_async(&["nmcli", "connection", "delete", "uuid", &uuid], move |r| match r {
                Ok(_) => {
                    window::toast(&format!("Removed {n3}"));
                    window::rebuild("online-accounts");
                }
                Err(e) => window::toast(&format!("Couldn't remove {n3}: {e}")),
            });
        });
        g.add(&widgets::row(&name, &format!("{kind} connection"), Some(del.upcast_ref())));
    }
    if !any {
        g.add(&widgets::row("No VPN set up", "Add NordVPN, ExpressVPN, Proton, PIA, Mullvad or a WireGuard/OpenVPN file.", None));
    }
    let (r, _) = widgets::button_row("Add a VPN", "Choose a service, then install and sign in, or import its file.", "Add…", |_| {
        show_add(true)
    });
    g.add(&r);
    widgets::keywords("vpn nordvpn expressvpn proton pia private internet access mullvad surfshark windscribe wireguard openvpn");
}

/// Choosing a service to add.
pub fn add_page(page: &Page) {
    let back = gtk::Button::from_icon_name("go-previous-symbolic");
    back.set_tooltip_text(Some("Back to accounts"));
    back.add_css_class("flat");
    back.add_css_class("back-button");
    back.set_halign(gtk::Align::Start);
    back.connect_clicked(|_| show_add(false));
    page.body.append(&back);
    let g = page.group("Add a VPN");
    g.note("Services with their own app are installed and signed in here; others come as a file from your account page.");
    for p in PROVIDERS {
        let installed = p.tool.is_some_and(cmd::present);
        let prov = *p;
        let label = match (p.install, installed) {
            (Install::None, _) => "Import…",
            (_, true) => "Sign in",
            (Install::Aur(..), false) => "Install",
            (Install::Page(_), false) => "Get it",
        };
        let b = gtk::Button::with_label(label);
        b.connect_clicked(move |_| set_up(prov));
        let desc = if installed { format!("{} · installed", p.desc) } else { p.desc.to_string() };
        g.add(&widgets::row(p.name, &desc, Some(b.upcast_ref())));
    }
}

fn set_up(p: Provider) {
    let installed = p.tool.is_some_and(cmd::present);
    match (p.install, installed) {
        (Install::None, _) => sign_in(p),
        (_, true) => sign_in(p),
        (Install::Aur(pkg, then), false) => {
            // In a terminal: the AUR build and sudo need one. Sign in afterwards.
            let command = format!("omarchy-pkg-aur-add {pkg} && {then} && echo && echo 'Installed. Back in Settings, choose Sign in.'");
            super::accounts::terminal(&command);
            window::toast(&format!("Installing {} in a terminal; sign in when it's done", p.name));
        }
        (Install::Page(url), false) => {
            cmd::spawn(&["xdg-open", url]);
            window::toast(&format!("Install {} from its page, then come back and sign in", p.name));
        }
    }
}

fn sign_in(p: Provider) {
    match p.sign_in {
        SignIn::Terminal(command) => super::accounts::terminal(command),
        SignIn::App(path) => {
            cmd::spawn(&[path]);
            // The tile can then connect without the app's window open.
            if p.id == "pia" {
                cmd::spawn(&["piactl", "background", "enable"]);
            }
        }
        SignIn::Code { label, run } => {
            let name = p.name;
            crate::dialog::ask(
                &format!("Sign in to {name}"),
                &format!("Your {} from {name}'s website.", label.to_lowercase()),
                vec![crate::dialog::Field::secret(label)],
                "Sign in",
                None,
                move |values, _| {
                    let code = values.first().cloned().unwrap_or_default();
                    if code.trim().is_empty() {
                        return Some(format!("Type your {}", label.to_lowercase()));
                    }
                    match run(&code) {
                        Ok(()) => {
                            window::toast(&format!("Signed in to {name}"));
                            window::rebuild("online-accounts");
                            None
                        }
                        Err(e) => Some(format!("{e:#}")),
                    }
                },
            );
        }
        SignIn::File(page) => import_file(p.name, page),
    }
}

/// Pick a WireGuard or OpenVPN file and import it into NetworkManager.
fn import_file(provider: &'static str, page: Option<&'static str>) {
    if let Some(url) = page {
        cmd::spawn(&["xdg-open", url]);
    }
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("WireGuard or OpenVPN files"));
    filter.add_suffix("conf");
    filter.add_suffix("ovpn");
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title(format!("Choose the {provider} file"))
        .accept_label("Import")
        .filters(&filters)
        .modal(true)
        .build();
    dialog.open(window::window().as_ref(), gtk::gio::Cancellable::NONE, move |res| {
        let Ok(file) = res else { return };
        let Some(path) = file.path() else { return };
        let openvpn = path.extension().is_some_and(|e| e == "ovpn");
        if openvpn && !std::path::Path::new("/usr/lib/NetworkManager/VPN/nm-openvpn-service.name").exists() {
            super::accounts::terminal("omarchy-pkg-add networkmanager-openvpn && echo && echo 'Done. Import the file again in Settings.'");
            window::toast("OpenVPN files need NetworkManager's OpenVPN plugin: installing it in a terminal");
            return;
        }
        let kind = if openvpn { "openvpn" } else { "wireguard" };
        let p = path.to_string_lossy().to_string();
        cmd::run_async(&["nmcli", "connection", "import", "type", kind, "file", &p], move |r| match r {
            Ok(out) => {
                // "Connection 'wg0' (uuid) successfully added." Name it after the provider.
                if provider != "Other"
                    && let Some(uuid) = out.split(['(', ')']).nth(1)
                {
                    let name = format!("{provider} · {}", path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
                    cmd::spawn(&["nmcli", "connection", "modify", "uuid", uuid, "connection.id", &name]);
                    // Not on by itself: the Control Center connects it.
                    cmd::spawn(&["nmcli", "connection", "modify", "uuid", uuid, "connection.autoconnect", "no"]);
                }
                window::toast("Imported. Connect it from the Control Center's VPN tile");
                show_add(false);
            }
            Err(e) => window::toast(&format!("Couldn't import: {e}")),
        });
    });
}
