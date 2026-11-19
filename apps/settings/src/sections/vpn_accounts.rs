//! VPN services, under Online Accounts and on Network: the ones set up, and adding one.
//! The Control Center's VPN tile then works with whatever is set up here.
//!
//! - Services with their own app (NordVPN, ExpressVPN, Mullvad, Proton VPN): installed
//!   here from the catalog (the System Manager asks for the password), then signed in
//!   the way that app does, one step at a time on the same row.
//! - Anything else (Surfshark, PIA, Windscribe, a work VPN, your own server): a
//!   WireGuard (`.conf`) or OpenVPN (`.ovpn`) file imported into NetworkManager.
//!
//! Secrets (an activation code, an account number) go to the app on stdin or in a
//! private temporary file removed straight after, never on a command line.

use crate::widgets::{self, Page};
use crate::{cmd, window};
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;

thread_local! {
    static ADDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// What a provider's row is doing now ("Installing…"), shown instead of its state.
    static BUSY: RefCell<HashMap<&'static str, String>> = RefCell::new(HashMap::new());
    /// Each installed provider's account, once asked: `Some(name)` signed in, `None` not.
    static ACCOUNTS: RefCell<HashMap<&'static str, Option<String>>> = RefCell::new(HashMap::new());
}

pub fn adding() -> bool {
    ADDING.with(|a| a.get())
}

fn show_add(on: bool) {
    ADDING.with(|a| a.set(on));
    window::rebuild("online-accounts");
}

/// Redraw the pages that show VPN services, after one is added or removed.
fn refresh() {
    window::rebuild_if_built("online-accounts");
    window::rebuild_if_built("network");
}

fn busy(id: &'static str) -> Option<String> {
    BUSY.with(|b| b.borrow().get(id).cloned())
}

fn set_busy(id: &'static str, what: Option<&str>) {
    BUSY.with(|b| match what {
        Some(w) => b.borrow_mut().insert(id, w.to_string()),
        None => b.borrow_mut().remove(id),
    });
    refresh();
}

/// How you sign in once the app is installed.
#[derive(Clone, Copy)]
enum SignIn {
    /// In the browser: the app prints the page to open, and its account shows once done.
    Browser { start: &'static [&'static str] },
    /// A code typed here, handed to the app; `help` is where the code is found.
    Code { label: &'static str, help: &'static str, run: fn(&str) -> anyhow::Result<()> },
    /// In the provider's own window.
    App(&'static str),
}

#[derive(Clone, Copy)]
struct Provider {
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    /// The catalog item that installs it (`nyx-system catalog install`).
    item: &'static str,
    /// Programs that show it's installed (any of them).
    tools: &'static [&'static str],
    sign_in: SignIn,
    /// The account it's signed in to, or `None`; unknown for apps that don't say.
    account: Option<fn() -> Option<String>>,
    sign_out: Option<fn() -> anyhow::Result<()>>,
}

const PROVIDERS: &[Provider] = &[
    Provider {
        id: "nord",
        name: "NordVPN",
        desc: "NordLynx, the fastest server, every country and city",
        item: "nordvpn",
        tools: &["nordvpn"],
        sign_in: SignIn::Browser { start: &["nordvpn", "login"] },
        account: Some(nord_account),
        sign_out: Some(|| run_quiet(&["nordvpn", "logout", "--persist-token"])),
    },
    Provider {
        id: "express",
        name: "ExpressVPN",
        desc: "Sign in with the activation code from your account",
        item: "expressvpn",
        tools: &["expressvpnctl", "expressvpn"],
        sign_in: SignIn::Code {
            label: "Activation code",
            help: "https://www.expressvpn.com/setup",
            run: express_login,
        },
        account: Some(express_account),
        sign_out: Some(express_logout),
    },
    Provider {
        id: "mullvad",
        name: "Mullvad",
        desc: "Sign in with your account number",
        item: "mullvad-vpn",
        tools: &["mullvad"],
        sign_in: SignIn::Code { label: "Account number", help: "https://mullvad.net/account", run: mullvad_login },
        account: Some(mullvad_account),
        sign_out: Some(|| run_quiet(&["mullvad", "account", "logout"])),
    },
    Provider {
        id: "proton",
        name: "Proton VPN",
        desc: "Sign in in Proton's window",
        item: "proton-vpn",
        tools: &["protonvpn-app", "protonvpn"],
        sign_in: SignIn::App("protonvpn-app"),
        account: None,
        sign_out: None,
    },
];

fn run_quiet(args: &[&str]) -> anyhow::Result<()> {
    let out = std::process::Command::new(args[0]).args(&args[1..]).output()?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// The first `Key: value` line among `keys` in a command's output.
fn field(args: &[&str], keys: &[&str]) -> Option<String> {
    let out = cmd::output(args)?;
    out.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        keys.iter().any(|key| k.trim().eq_ignore_ascii_case(key)).then(|| v.trim().to_string()).filter(|v| !v.is_empty())
    })
}

fn nord_account() -> Option<String> {
    field(&["nordvpn", "account"], &["Email Address", "Email"])
}

fn mullvad_account() -> Option<String> {
    field(&["mullvad", "account", "get"], &["Mullvad account"]).map(|n| format!("account {n}"))
}

/// ExpressVPN 4 (`expressvpnctl`) or 3 (`expressvpn`, as nixpkgs has it).
fn express_v4() -> bool {
    cmd::present("expressvpnctl")
}

fn express_account() -> Option<String> {
    if express_v4() {
        let out = cmd::output(&["expressvpnctl", "get", "activated"])?;
        (out.trim() == "true").then(|| "activated".to_string())
    } else {
        let out = cmd::output(&["expressvpn", "status"])?;
        (!out.to_lowercase().contains("not activated")).then(|| "activated".to_string())
    }
}

fn express_logout() -> anyhow::Result<()> {
    if express_v4() { run_quiet(&["expressvpnctl", "logout"]) } else { run_quiet(&["expressvpn", "logout"]) }
}

/// ExpressVPN: v4 reads the code from a private file (removed after); v3 asks for it,
/// then whether to share diagnostics (no), on stdin.
fn express_login(code: &str) -> anyhow::Result<()> {
    if !express_v4() {
        let mut child = std::process::Command::new("expressvpn")
            .arg("activate")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            writeln!(stdin, "{}\nn", code.trim())?;
        }
        let out = child.wait_with_output()?;
        if !out.status.success() {
            anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        return Ok(());
    }
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

fn read_config() -> toml::Table {
    std::fs::read_to_string(config_path()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

fn chosen() -> Option<String> {
    read_config().get("default").and_then(|v| v.as_str()).map(String::from)
}

/// Set the default VPN, keeping the rest of the file (the Control Center's favourite
/// and default servers).
fn set_chosen(id: &str) {
    let mut t = read_config();
    t.insert("default".into(), toml::Value::String(id.to_string()));
    let text = match toml::to_string(&t) {
        Ok(s) => s,
        Err(e) => return window::toast(&format!("Couldn't save: {e}")),
    };
    if let Err(e) = cmd::atomic_write(&config_path(), text) {
        window::toast(&format!("Couldn't save: {e}"));
    }
}

/// The VPN group on the Online Accounts page: which VPN the Control Center's tile
/// connects, and adding one. Signing in and out of each is under Network › VPN services.
/// The VPN group on the Online Accounts page: which VPN the Control Center's tile
/// connects, and adding one. Signing in and out of each is under Network › VPN services.
pub fn build(page: &Page) {
    let g = page.group("VPN");
    let services = installed();
    if services.is_empty() {
        g.add(&widgets::icon_row(
            "security-high-symbolic",
            "No VPN yet",
            "NordVPN, ExpressVPN, Mullvad, Proton, or a WireGuard or OpenVPN file",
            None,
        ));
    } else {
        let current = chosen().filter(|c| services.iter().any(|(id, _)| id == c)).unwrap_or_else(|| services[0].0.clone());
        let name = services.iter().find(|(id, _)| *id == current).map(|(_, n)| n.clone()).unwrap_or_default();
        let control: gtk::Widget = if services.len() > 1 {
            let dd = widgets::dropdown(&services, &current);
            let options = services.clone();
            dd.connect_selected_notify(move |d| {
                if let Some((id, _)) = options.get(d.selected() as usize) {
                    set_chosen(id);
                    refresh();
                }
            });
            dd.upcast()
        } else {
            widgets::label(&name, "dim").upcast()
        };
        control.set_valign(gtk::Align::Center);
        g.add(&widgets::icon_row(
            "security-high-symbolic",
            "Control Center tile",
            &format!("Connects {}", gtk::glib::markup_escape_text(&name)),
            Some(&control),
        ));
    }
    g.add(&super::accounts::add_row("Add a VPN", || show_add(true)));
    widgets::keywords(
        "vpn control center tile nordvpn expressvpn proton mullvad surfshark pia windscribe wireguard openvpn",
    );
}

/// Every service set up, as (id, name), the way the Control Center names them.
fn installed() -> Vec<(String, String)> {
    let mut services: Vec<(String, String)> = PROVIDERS
        .iter()
        .filter(|p| has_app(p))
        .map(|p| (p.id.to_string(), p.name.to_string()))
        .collect();
    services.extend(nm_vpns().into_iter().map(|(name, uuid, _)| (format!("nm:{uuid}"), name)));
    services
}

/// The VPN services set up, as a page one level down (from Network).
pub fn subpage() {
    widgets::subpage("VPN services", 560, |p| {
        services(p, || {
            widgets::subpage("Add a VPN", 560, providers);
        })
    });
}

/// How many VPN services are set up.
pub fn count() -> usize {
    PROVIDERS.iter().filter(|p| has_app(p)).count() + nm_vpns().len()
}

/// NetworkManager's VPN connections, as (name, uuid), and which the Control Center uses
/// (`nm:<uuid>` or a service's id).
pub fn nm_connections() -> (Vec<(String, String)>, Option<String>) {
    (nm_vpns().into_iter().map(|(name, uuid, _)| (name, uuid)).collect(), chosen())
}

/// Each VPN set up, with signing in and out or removing it (Network › VPN services).

/// Each VPN set up, with signing in and out or removing it (Network › VPN services).
fn services(page: &Page, on_add: impl Fn() + 'static) {
    let g = page.group("VPN");
    g.note("The Control Center's VPN tile shows the one chosen here.");
    let services = installed();
    if services.len() > 1 {
        let current = chosen().filter(|c| services.iter().any(|(id, _)| id == c)).unwrap_or_else(|| services[0].0.clone());
        let (r, _) =
            widgets::choice_row("Show in the Control Center", "The VPN its tile connects.", services.clone(), &current, |id| {
                set_chosen(&id)
            });
        g.add(&r);
    }
    let mut any = false;
    for p in PROVIDERS.iter().filter(|p| has_app(p)) {
        any = true;
        g.add(&provider_row(*p));
    }
    for (name, uuid, kind) in nm_vpns() {
        any = true;
        let n2 = name.clone();
        let del = widgets::confirm_button("Remove", "Click again to remove", move |_| {
            let n3 = n2.clone();
            cmd::run_async(&["nmcli", "connection", "delete", "uuid", &uuid], move |r| match r {
                Ok(_) => {
                    window::toast(&format!("Removed {n3}"));
                    refresh();
                }
                Err(e) => window::toast(&format!("Couldn't remove {n3}: {e}")),
            });
        });
        g.add(&widgets::row(&name, &format!("{kind} connection"), Some(del.upcast_ref())));
    }
    if !any {
        g.add(&widgets::row("No VPN set up", "Add NordVPN, ExpressVPN, Mullvad, Proton, or a WireGuard or OpenVPN file.", None));
    }
    let (r, _) = widgets::button_row(
        "Add a VPN",
        "Install a VPN's app and sign in, or import a WireGuard or OpenVPN file.",
        "Add…",
        move |_| on_add(),
    );
    g.add(&r);
    widgets::keywords("vpn nordvpn expressvpn proton mullvad surfshark pia windscribe wireguard openvpn");
}

/// Choosing a service to add (Online Accounts).
pub fn add_page(page: &Page) {
    let back = gtk::Button::from_icon_name("go-previous-symbolic");
    back.set_tooltip_text(Some("Back to accounts"));
    back.add_css_class("flat");
    back.add_css_class("back-button");
    back.set_halign(gtk::Align::Start);
    back.connect_clicked(|_| show_add(false));
    page.body.append(&back);
    providers(page);
}

/// Every service there is to add: one row each, with the one thing to do next, and a
/// file for anything else.
fn providers(page: &Page) {
    let g = page.group("VPN apps");
    g.note("Installs the provider's own app, then signs you in. The Control Center's VPN tile connects it.");
    for p in PROVIDERS {
        g.add(&provider_row(*p));
    }
    let g = page.group("Anything else");
    let (r, _) = widgets::button_row(
        "Import a WireGuard or OpenVPN file",
        "Surfshark, PIA, Windscribe, Proton's WireGuard files, a work VPN or your own server: download the file from your account, then choose it here.",
        "Choose file…",
        |_| import_file(),
    );
    g.add(&r);
}

/// Where a provider is: installing or signing in now, not installed, signed in or not.
enum State {
    Busy(String),
    NotInstalled,
    Checking,
    SignedOut,
    SignedIn(String),
    /// Installed, by an app that doesn't say whether it's signed in.
    Installed,
}

fn state(p: &Provider) -> State {
    if let Some(what) = busy(p.id) {
        return State::Busy(what);
    }
    if !has_app(p) {
        return State::NotInstalled;
    }
    let Some(account) = p.account else { return State::Installed };
    match ACCOUNTS.with(|a| a.borrow().get(p.id).cloned()) {
        Some(Some(name)) => State::SignedIn(name),
        Some(None) => State::SignedOut,
        None => {
            // Asked off the main thread (the apps' daemons can take a moment).
            let id = p.id;
            cmd::background(account, move |name| {
                ACCOUNTS.with(|a| a.borrow_mut().insert(id, name));
                refresh();
            });
            State::Checking
        }
    }
}

/// Forget what was learnt about a provider's account, so the row asks again.
fn recheck(id: &'static str) {
    ACCOUNTS.with(|a| a.borrow_mut().remove(id));
}

fn provider_row(p: Provider) -> gtk::Box {
    let (desc, control): (String, gtk::Widget) = match state(&p) {
        State::Busy(what) => {
            let spinner = gtk::Spinner::new();
            spinner.start();
            (what, spinner.upcast())
        }
        State::NotInstalled => {
            let b = gtk::Button::with_label("Install");
            b.add_css_class("suggested-action");
            b.connect_clicked(move |_| install(p));
            (p.desc.to_string(), b.upcast())
        }
        State::Checking => (format!("{} · checking…", p.desc), gtk::Spinner::new().upcast()),
        State::SignedOut | State::Installed => {
            let b = gtk::Button::with_label("Sign in");
            b.add_css_class("suggested-action");
            b.connect_clicked(move |_| sign_in(p));
            ("Installed · not signed in".to_string(), b.upcast())
        }
        State::SignedIn(name) => {
            let controls = widgets::hbox(6);
            if let Some(sign_out) = p.sign_out {
                let b = widgets::confirm_button("Sign out", "Click again to sign out", move |_| {
                    cmd::background(sign_out, move |r| {
                        match r {
                            Ok(()) => window::toast(&format!("Signed out of {}", p.name)),
                            Err(e) => failed(&format!("Couldn't sign out of {}", p.name), &format!("{e:#}")),
                        }
                        recheck(p.id);
                        refresh();
                    });
                });
                controls.append(&b);
            }
            (format!("Signed in · {name} · connect from the Control Center"), controls.upcast())
        }
    };
    widgets::icon_row("security-high-symbolic", p.name, &gtk::glib::markup_escape_text(&desc), Some(&control))
}

/// Something failed: a dialog that stays until it's read (a toast goes before a long
/// message can be).
fn failed(title: &str, detail: &str) {
    gtk::AlertDialog::builder().message(title).detail(detail).modal(true).build().show(window::window().as_ref());
}

/// Whether the provider's app is installed.
fn has_app(p: &Provider) -> bool {
    p.tools.iter().any(|t| cmd::present(t))
}

/// Install the provider's app from the catalog (the System Manager asks for the
/// password), then go straight on to signing in.
fn install(p: Provider) {
    set_busy(p.id, Some("Installing… (asks for your password; takes a minute or two)"));
    cmd::run_async(&["nyx-system", "catalog", "install", p.item, "--yes"], move |r| {
        set_busy(p.id, None);
        match r {
            Ok(_) if has_app(&p) => {
                window::toast(&format!("{} installed. Now sign in.", p.name));
                recheck(p.id);
                sign_in(p);
            }
            Ok(_) => window::toast(&format!("{} installed; open Settings again if it doesn't show", p.name)),
            Err(e) => failed(&format!("{} couldn't be installed", p.name), &format!("{e:#}")),
        }
    });
}

fn sign_in(p: Provider) {
    match p.sign_in {
        SignIn::App(program) => {
            cmd::spawn(&[program]);
            window::toast(&format!("Sign in in {}'s window", p.name));
        }
        SignIn::Browser { start } => browser_sign_in(p, start),
        SignIn::Code { label, help, run } => {
            let name = p.name;
            let help_page = help;
            window::toast_action(&format!("Your {} is on your {name} account page", label.to_lowercase()), "Open it", move || {
                cmd::spawn(&["xdg-open", help_page])
            });
            crate::dialog::ask(
                &format!("Sign in to {name}"),
                &format!("Type the {} from your {name} account ({help}).", label.to_lowercase()),
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
                            window::toast(&format!("Signed in to {name}. Connect from the Control Center's VPN tile."));
                            recheck(p.id);
                            refresh();
                            None
                        }
                        Err(e) => Some(format!("{e:#}")),
                    }
                },
            );
        }
    }
}

/// Start the app's browser sign-in, open the page it names, and wait (up to five
/// minutes) for its account to show.
fn browser_sign_in(p: Provider, start: &'static [&'static str]) {
    set_busy(p.id, Some("Opening the sign-in page…"));
    cmd::background(
        move || {
            let out = std::process::Command::new(start[0]).args(&start[1..]).output().ok()?;
            let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            text.split_whitespace().find(|w| w.starts_with("https://")).map(String::from).or_else(|| {
                // Already signed in, or it opened the browser itself.
                Some(String::new())
            })
        },
        move |url| {
            if let Some(url) = url.filter(|u| !u.is_empty()) {
                cmd::spawn(&["xdg-open", &url]);
            }
            set_busy(p.id, Some("Finish signing in in your browser…"));
            wait_for_account(p, 150);
        },
    );
}

fn wait_for_account(p: Provider, tries: u32) {
    let Some(account) = p.account else { return set_busy(p.id, None) };
    gtk::glib::timeout_add_seconds_local_once(2, move || {
        cmd::background(account, move |name| match name {
            Some(name) => {
                ACCOUNTS.with(|a| a.borrow_mut().insert(p.id, Some(name)));
                set_busy(p.id, None);
                window::toast(&format!("Signed in to {}. Connect from the Control Center's VPN tile.", p.name));
            }
            None if tries > 1 => wait_for_account(p, tries - 1),
            None => {
                set_busy(p.id, None);
                window::toast(&format!("Still not signed in to {}; try Sign in again", p.name));
            }
        });
    });
}

/// NetworkManager's OpenVPN plugin, which OpenVPN files need.
fn openvpn_plugin() -> bool {
    ["/usr/lib/NetworkManager/VPN", "/etc/NetworkManager/VPN"]
        .iter()
        .any(|d| std::path::Path::new(d).join("nm-openvpn-service.name").exists())
}

/// Pick a WireGuard or OpenVPN file and import it into NetworkManager.
fn import_file() {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("WireGuard or OpenVPN files"));
    filter.add_suffix("conf");
    filter.add_suffix("ovpn");
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title("Choose a WireGuard or OpenVPN file")
        .accept_label("Import")
        .filters(&filters)
        .modal(true)
        .build();
    dialog.open(window::window().as_ref(), gtk::gio::Cancellable::NONE, move |res| {
        let Ok(file) = res else { return };
        let Some(path) = file.path() else { return };
        let openvpn = path.extension().is_some_and(|e| e == "ovpn");
        if openvpn && !openvpn_plugin() {
            window::toast("OpenVPN files need NetworkManager's OpenVPN plugin, which isn't installed");
            return;
        }
        let kind = if openvpn { "openvpn" } else { "wireguard" };
        let p = path.to_string_lossy().to_string();
        cmd::run_async(&["nmcli", "connection", "import", "type", kind, "file", &p], move |r| match r {
            Ok(out) => {
                // "Connection 'wg0' (uuid) successfully added." Not on by itself: the
                // Control Center connects it.
                if let Some(uuid) = out.split(['(', ')']).nth(1) {
                    cmd::spawn(&["nmcli", "connection", "modify", "uuid", uuid, "connection.autoconnect", "no"]);
                }
                window::toast("Imported. Connect it from the Control Center's VPN tile");
                refresh();
                if adding() {
                    show_add(false);
                }
            }
            Err(e) => failed("The file couldn't be imported", &format!("{e:#}")),
        });
    });
}
