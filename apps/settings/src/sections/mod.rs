//! Every page in the sidebar, in order.

use crate::widgets::Page;
use crate::{ext, paths};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

pub mod about;
pub mod account_icons;
pub mod accounts;
pub mod apps;
pub mod arrange;
pub mod assistant;
pub mod audio;
pub mod bar;
pub mod boot;
pub mod barlayout;
pub mod capture;
pub mod cleanup;
pub mod connectivity;
pub mod disks;
pub mod displays;
pub mod extensions;
pub mod home;
pub mod idle;
pub mod keybindings;
pub mod keyboard;
pub mod look;
pub mod mounts;
pub mod mouse;
pub mod network;
pub mod nightlight;
pub mod nyx;
pub mod online_accounts;
pub mod online_presets;
pub mod cloud;
pub mod plugins;
pub mod power;
pub mod printers;
pub mod region;
pub mod rules;
pub mod services;
pub mod software;
pub mod theme;
pub mod trackpad;
pub mod workspace_rules;
pub mod workspaces;

/// How a page is filled in.
#[derive(Clone)]
pub enum Build {
    Native(fn(&Page)),
    /// Drawn from what an extension describes.
    Extension(Rc<ext::PageRef>),
}

#[derive(Clone)]
pub struct Section {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    pub group: &'static str,
    pub description: &'static str,
    /// Extra words the search should find this page by.
    pub keywords: &'static str,
    /// Files the "Open config" button offers.
    pub files: fn() -> Vec<PathBuf>,
    pub build: Build,
    pub visible: fn() -> bool,
}

impl Section {
    pub fn run(&self, page: &Page) {
        match &self.build {
            Build::Native(f) => f(page),
            Build::Extension(r) => ext::render::build(page, self.id, r.clone()),
        }
    }

    pub fn config_files(&self) -> Vec<PathBuf> {
        match &self.build {
            Build::Native(_) => (self.files)(),
            Build::Extension(r) => r.ext.files(),
        }
    }
}

thread_local! {
    static INTERNED: RefCell<HashMap<String, &'static str>> = RefCell::default();
}

/// Section ids and titles are `&'static str`; extension ones are made once and reused.
fn intern(s: &str) -> &'static str {
    INTERNED.with(|m| *m.borrow_mut().entry(s.to_string()).or_insert_with(|| Box::leak(s.to_string().into_boxed_str())))
}

fn extension_group(category: &str) -> &'static str {
    match category {
        "Connections" => "Connections", "Personalization" => "Personalization",
        "Desktop" => "Desktop", "Input" => "Input", "Accounts" => "Accounts",
        "System" => "System", _ => "Hardware",
    }
}

fn extension_sections(fresh: bool) -> Vec<Section> {
    ext::all_pages(fresh)
        .into_iter()
        .map(|r| Section {
            id: intern(&r.section_id()),
            title: intern(&r.page.title),
            icon: intern(&r.page.icon),
            group: extension_group(&r.ext.manifest.category),
            description: intern(&r.page.description),
            keywords: intern(&format!("{} {}", r.ext.manifest.name, r.page.keywords)),
            files: Vec::new,
            build: Build::Extension(Rc::new(r)),
            visible: always,
        })
        .collect()
}

fn assistant_config() -> Vec<PathBuf> {
    vec![paths::config_home().join("nyx-assistant/config.toml")]
}

fn always() -> bool {
    true
}

fn hypr(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(|n| paths::hypr_dir().join(n)).collect()
}

pub fn all() -> Vec<Section> {
    all_with(false)
}

/// `fresh` asks every extension what it has now instead of using what it said last time.
pub fn all_with(fresh: bool) -> Vec<Section> {
    let mut list = vec![
        // No group: Home sits above the first heading.
        Section {
            id: "home",
            title: "Home",
            icon: "user-home-symbolic",
            group: "",
            description: "This computer at a glance: activity, storage and updates.",
            keywords: "overview system info stats cpu processor memory ram disk storage battery temperature network uptime updates packages aur firmware snapshot",
            files: Vec::new,
            build: Build::Native(home::build),
            visible: always,
        },
        // ----- Connections -----
        Section {
            id: "wifi",
            title: "Wi-Fi",
            icon: "network-wireless-symbolic",
            group: "Connections",
            description: "Join wireless networks, and see and forget saved ones.",
            keywords: "wifi wi-fi network wireless join password hidden ssid airplane forget",
            files: Vec::new,
            build: Build::Native(connectivity::build_wifi),
            visible: always,
        },
        Section {
            id: "bluetooth",
            title: "Bluetooth",
            icon: "bluetooth-symbolic",
            group: "Connections",
            description: "Find, pair and connect Bluetooth devices.",
            keywords: "bluetooth pair headphones earbuds speaker keyboard mouse controller connect forget",
            files: Vec::new,
            build: Build::Native(connectivity::build_bluetooth),
            visible: always,
        },
        Section {
            id: "network",
            title: "Network",
            icon: "network-wired-symbolic",
            group: "Connections",
            description: "DNS, Wi-Fi band, firewall, VPNs and a speed test.",
            keywords: "network dns firewall ufw vpn tailscale nordvpn speed test ethernet ip band wifi internet",
            files: Vec::new,
            build: Build::Native(network::build),
            visible: always,
        },
        Section {
            id: "mounts",
            title: "Mounts",
            icon: "folder-remote-symbolic",
            group: "Connections",
            description: "Drives at startup, shares from a NAS or Windows PC, and folders over SSH.",
            keywords: "mounts fstab nas samba smb cifs windows share nfs sshfs ssh network drive automount startup",
            files: || vec![std::path::PathBuf::from("/etc/fstab")],
            build: Build::Native(mounts::build),
            visible: always,
        },
        // ----- Hardware -----
        Section {
            id: "displays",
            title: "Displays",
            icon: "video-display-symbolic",
            group: "Hardware",
            description: "Resolution, refresh rate, scale, arrangement and rendering options for your displays.",
            keywords: "monitor screen resolution refresh hz scale hidpi position rotate brightness vrr adaptive sync freesync hardware software cursor direct scanout fullscreen igpu gpu performance",
            files: || hypr(&["monitors.lua", "settings.lua"]),
            build: Build::Native(displays::build),
            visible: always,
        },
        Section {
            id: "audio",
            title: "Sound",
            icon: "audio-speakers-symbolic",
            group: "Hardware",
            description: "Volume, devices, a 9-band equalizer and a preamp to make everything louder.",
            keywords: "audio volume loud quiet boost preamp equalizer eq bass treble speakers headphones microphone output input",
            files: crate::backend::audio::config_files,
            build: Build::Native(audio::build),
            visible: always,
        },
        Section {
            id: "power",
            title: "Power & Battery",
            icon: "battery-full-charged-symbolic",
            group: "Hardware",
            description: "Power profiles, brightness and keyboard backlight.",
            keywords: "battery power profile performance balanced saver brightness backlight charge",
            files: Vec::new,
            build: Build::Native(power::build),
            visible: always,
        },
        Section {
            id: "idle",
            title: "Lock Screen",
            icon: "system-lock-screen-symbolic",
            group: "Hardware",
            description: "When the screensaver starts, the screen locks and the computer sleeps.",
            keywords: "idle screensaver lock sleep suspend timeout stay awake idle inhibit",
            files: || vec![paths::shell_json()],
            build: Build::Native(idle::build),
            visible: always,
        },
        Section {
            id: "printers",
            title: "Printers",
            icon: "printer-symbolic",
            group: "Hardware",
            description: "Add printers, choose the default and print a test page.",
            keywords: "printers printing cups print scanner default test page airprint ipp",
            files: Vec::new,
            build: Build::Native(printers::build),
            visible: always,
        },
        Section {
            id: "disks",
            title: "Disks",
            icon: "drive-harddisk-symbolic",
            group: "Hardware",
            description: "Drives and partitions: format, resize, encrypt, check health, and free up space.",
            keywords: "disks drives partitions format resize shrink grow delete label encrypt luks usb ssd nvme health smart \
                       temperature wear cleanup space cache logs trash",
            files: Vec::new,
            build: Build::Native(disks::build),
            visible: always,
        },
        // ----- Personalization -----
        Section {
            id: "theme",
            title: "Appearance",
            icon: "preferences-desktop-appearance-symbolic",
            group: "Personalization",
            description: "Theme, wallpaper, fonts, and how this window looks.",
            keywords: "theme colors colours palette wallpaper background font dracula catppuccin tokyo night one dark nord gruvbox follow omarchy",
            files: || vec![paths::omarchy_colors(), paths::app_dir().join("settings.toml")],
            build: Build::Native(theme::build),
            visible: always,
        },
        Section {
            id: "look",
            title: "Window Style",
            icon: "focus-windows-symbolic",
            group: "Personalization",
            description: "Gaps, borders, rounding, transparency, blur, shadows and animations.",
            keywords: "look feel gaps border rounding radius opacity transparency blur shadow animation cursor layout dwindle master scrolling",
            files: || hypr(&["looknfeel.lua", "settings.lua"]),
            build: Build::Native(look::build),
            visible: always,
        },
        Section {
            id: "bar",
            title: "Bar & Notifications",
            icon: "preferences-system-notifications-symbolic",
            group: "Personalization",
            description: "Where the bar sits, how it looks, and how notifications behave.",
            keywords: "waybar panel top bottom transparent notifications do not disturb silence shell text size",
            files: || vec![paths::shell_json(), paths::omarchy_config().join("shell.toml")],
            build: Build::Native(bar::build),
            visible: always,
        },
        Section {
            id: "plugins",
            title: "Bar Plugins",
            icon: "package-x-generic-symbolic",
            group: "Personalization",
            description: "Omarchy shell plugins: turn them on or off and keep them updated.",
            keywords: "plugins extensions widgets bar update enable disable remove",
            files: || vec![paths::shell_json()],
            build: Build::Native(plugins::build),
            visible: always,
        },
        Section {
            id: "nightlight",
            title: "Night Light",
            icon: "night-light-symbolic",
            group: "Personalization",
            description: "Warmer colors in the evening with hyprsunset.",
            keywords: "hyprsunset blue light temperature warm evening schedule",
            files: || hypr(&["hyprsunset.conf"]),
            build: Build::Native(nightlight::build),
            visible: always,
        },
        // ----- Desktop -----
        Section {
            id: "workspaces",
            title: "Tiling & Workspaces",
            icon: "view-grid-symbolic",
            group: "Desktop",
            description: "How windows are placed, focused and grouped, and how workspaces behave.",
            keywords: "windows focus follows mouse layout dwindle master scrolling split workspace resize",
            files: || hypr(&["looknfeel.lua", "settings.lua"]),
            build: Build::Native(workspaces::build),
            visible: always,
        },
        Section {
            id: "keybindings",
            title: "Keyboard Shortcuts",
            icon: "settings-shortcuts-symbolic",
            group: "Desktop",
            description: "Every shortcut you have, plus your own. Turn any of them off.",
            keywords: "keybindings shortcuts keys hotkeys bind unbind super",
            files: || hypr(&["bindings.lua", "settings.lua"]),
            build: Build::Native(keybindings::build),
            visible: always,
        },
        Section {
            id: "rules",
            title: "Startup & Rules",
            icon: "view-list-symbolic",
            group: "Desktop",
            description: "Programs that start with the session, and rules for windows and the shell.",
            keywords: "rules window layer startup autostart float opacity workspace blur class title launch login",
            files: Vec::new,
            build: Build::Native(rules::build),
            visible: always,
        },
        Section {
            id: "capture",
            title: "Capture & Tools",
            icon: "camera-photo-symbolic",
            group: "Desktop",
            description: "Screenshots, screen recording, reminders, weather and dictation.",
            keywords: "screenshot screenshots record recording screencast ocr qr reminder timer weather voxtype dictation crash disk speed restart",
            files: Vec::new,
            build: Build::Native(capture::build),
            visible: always,
        },
        // ----- Input -----
        Section {
            id: "keyboard",
            title: "Keyboard",
            icon: "input-keyboard-symbolic",
            group: "Input",
            description: "Layouts, compose and caps lock, and key repeat.",
            keywords: "layout language us variant compose caps lock repeat rate delay numlock xkb",
            files: || hypr(&["input.lua", "settings.lua"]),
            build: Build::Native(keyboard::build),
            visible: always,
        },
        Section {
            id: "mouse",
            title: "Mouse",
            icon: "input-mouse-symbolic",
            group: "Input",
            description: "Pointer speed, acceleration, scrolling and per-device overrides.",
            keywords: "pointer sensitivity speed acceleration accel flat adaptive scroll natural left handed cursor",
            files: || hypr(&["input.lua", "settings.lua"]),
            build: Build::Native(mouse::build),
            visible: always,
        },
        Section {
            id: "trackpad",
            title: "Trackpad",
            icon: "input-touchpad-symbolic",
            group: "Input",
            description: "Tapping, scrolling, and two-, three- and four-finger gestures.",
            keywords: "touchpad gestures swipe pinch tap click fingers natural scrolling reverse invert drag",
            files: || hypr(&["input.lua", "settings.lua"]),
            build: Build::Native(trackpad::build),
            visible: always,
        },
        // ----- Accounts -----
        Section {
            id: "accounts",
            title: "Users",
            icon: "system-users-symbolic",
            group: "Accounts",
            description: "Users, groups, administrator access, SSH keys and sign-in security.",
            keywords: "accounts users groups accounts sudo admin administrator password login ssh keys fido2 fingerprint docker wheel security encryption",
            files: accounts::files,
            build: Build::Native(accounts::build),
            visible: always,
        },
        Section {
            id: "online-accounts",
            title: "Online Accounts",
            icon: "network-workgroup-symbolic",
            group: "Accounts",
            description: "Google, Microsoft, Nextcloud and other accounts used by Nyx apps.",
            keywords: "online accounts google microsoft nextcloud webdav imap oauth email calendar contacts notes storage cloud rclone drive dropbox onedrive sftp files goa",
            files: Vec::new,
            build: Build::Native(online_accounts::build),
            visible: always,
        },
        // ----- Assistant -----
        Section {
            id: "assistant",
            title: "Assistant",
            icon: "audio-input-microphone-symbolic",
            group: "Assistant",
            description: "Turn Nyx Assistant on or off, and how you talk to it.",
            keywords: "assistant voice ai echo wake word conversation start stop autonomy",
            files: assistant_config,
            build: Build::Native(assistant::general::build),
            visible: assistant::client::installed,
        },
        Section {
            id: "assistant-appearance",
            title: "Aura",
            icon: "focus-windows-symbolic",
            group: "Assistant",
            description: "Aura border, movement and displays.",
            keywords: "assistant aura glow border full edge flow appearance monitor",
            files: assistant_config,
            build: Build::Native(assistant::appearance::build),
            visible: assistant::client::installed,
        },
        Section {
            id: "assistant-voice",
            title: "Voice",
            icon: "audio-speakers-symbolic",
            group: "Assistant",
            description: "Speech recognition, talking over it, and its voice.",
            keywords: "assistant stt asr tts voice duplex barge interrupt microphone speaker kokoro",
            files: assistant_config,
            build: Build::Native(assistant::voice::build),
            visible: assistant::client::installed,
        },
        Section {
            id: "assistant-permissions",
            title: "Permissions",
            icon: "security-high-symbolic",
            group: "Assistant",
            description: "What the assistant may do on its own, after asking, or never.",
            keywords: "assistant permission allow ask deny files admin tool rule computer",
            files: assistant_config,
            build: Build::Native(assistant::permissions::build),
            visible: assistant::client::installed,
        },
        Section {
            id: "assistant-activity",
            title: "Activity",
            icon: "view-list-symbolic",
            group: "Assistant",
            description: "What the assistant is asking, doing and has done.",
            keywords: "assistant activity task approval audit log history",
            files: assistant_config,
            build: Build::Native(assistant::activity::build),
            visible: assistant::client::installed,
        },
        Section {
            id: "assistant-performance",
            title: "Performance",
            icon: "power-profile-performance-symbolic",
            group: "Assistant",
            description: "How fast it answers and what it holds in memory.",
            keywords: "assistant performance latency timing vram gpu",
            files: assistant_config,
            build: Build::Native(assistant::performance::build),
            visible: assistant::client::installed,
        },
        Section {
            id: "assistant-models",
            title: "Models",
            icon: "drive-harddisk-symbolic",
            group: "Assistant",
            description: "Choose and download the language model.",
            keywords: "assistant model llm gguf hugging face download gemma qwen",
            files: assistant_config,
            build: Build::Native(assistant::models::build),
            visible: assistant::client::installed,
        },
        // ----- System -----
        Section {
            id: "region",
            title: "Time & Language",
            icon: "preferences-system-time-symbolic",
            group: "System",
            description: "Time zone, clock, language and the computer's name.",
            keywords: "region date date time timezone clock ntp language locale hostname name region",
            files: Vec::new,
            build: Build::Native(region::build),
            visible: always,
        },
        Section {
            id: "boot",
            title: "Boot & Login",
            icon: "system-reboot-symbolic",
            group: "System",
            description: "Boot and login screens, hibernation, snapshots and the boot menu.",
            keywords: "boot login plymouth sddm hibernation hibernate swap snapshot restore limine direct boot efi",
            files: Vec::new,
            build: Build::Native(boot::build),
            visible: always,
        },
        Section {
            id: "apps",
            title: "Default Apps",
            icon: "system-run-symbolic",
            group: "System",
            description: "Which apps open links, files, folders, media, documents and mail.",
            keywords: "default browser terminal editor file manager image video music pdf mail xdg mime open with",
            files: || vec![paths::config_home().join("mimeapps.list")],
            build: Build::Native(apps::build),
            visible: always,
        },
        Section {
            id: "software",
            title: "Software",
            icon: "system-software-install-symbolic",
            group: "System",
            description: "Web apps, terminal apps, packages, installers and the update channel.",
            keywords: "software apps packages install uninstall webapp tui pacman yay aur channel updates preinstalls firmware keyring orphans",
            files: Vec::new,
            build: Build::Native(software::build),
            visible: always,
        },
        Section {
            id: "services",
            title: "Services",
            icon: "applications-system-symbolic",
            group: "System",
            description: "Background services: see what runs, start, stop and choose what starts at boot.",
            keywords: "services daemons systemd units start stop restart enable disable failed background",
            files: Vec::new,
            build: Build::Native(services::build),
            visible: always,
        },
        Section {
            id: "extensions",
            title: "Extensions",
            icon: "application-x-addon-symbolic",
            group: "System",
            description: "Additional device and app settings, installed from GitHub. Each extension adds pages in the appropriate category.",
            keywords: "extensions add-ons addons devices brands asus aura logitech obsbot webcam headset razer corsair install github",
            files: Vec::new,
            build: Build::Native(extensions::build),
            visible: always,
        },
        Section {
            id: "nyx",
            title: "Nyx apps",
            icon: "applications-other-symbolic",
            group: "System",
            description: "The look every Nyx app shares, and installing or updating the apps.",
            keywords: "nyx apps suite theme density text size install update remove music pdf chat",
            files: || vec![nyx_theme::suite_file()],
            build: Build::Native(nyx::build),
            visible: always,
        },
        Section {
            id: "about",
            title: "About",
            icon: "help-about-symbolic",
            group: "System",
            description: "Settings and Omarchy versions, updating them, and snapshots.",
            keywords: "update upgrade version snapshot about system info kernel cpu memory",
            files: Vec::new,
            build: Build::Native(about::build),
            visible: always,
        },
    ];
    for section in extension_sections(fresh) {
        let at = list.iter().rposition(|s| s.group == section.group).map(|i| i + 1).unwrap_or(list.len());
        list.insert(at, section);
    }
    list
}

mod native_accounts;
