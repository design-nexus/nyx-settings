# Omarchy Settings

A native control panel for [Omarchy](https://omarchy.org). One clean, searchable
window for everything you'd otherwise change by editing Lua files.

![Settings: Sound, with the 9-band equalizer and preamp](docs/audio.png)

```sh
curl -fsSL https://raw.githubusercontent.com/design-nexus/nyx-settings/main/install.sh | bash -s -- settings
```

## Features

- **Home**: Settings opens on an overview of this computer: model, Omarchy,
  kernel and Hyprland versions, and uptime; gauges for processor, memory, disk,
  battery and temperature; live charts of the last minute of processor, memory,
  network and per-core use; every disk and the installed package count. It also
  lists every update waiting to be installed (Omarchy, packages, the AUR and
  firmware) with a one-click **Update now**, and shows the count in the sidebar.
  Quick settings at the top switch Do not disturb and the power profile.
  To open on the last page shown instead, see Appearance → Settings window.

  ![Settings: Home](docs/home.png)

- **Organized and searchable**: pages are grouped into Connections, Hardware,
  Personalization, Desktop, Input, Accounts, Assistant (when Nyx Assistant is
  installed) and System. Each group of settings
  sits on one card, and less-used groups fold away until you open them.
  <kbd>Ctrl</kbd>+<kbd>F</kbd> searches every setting on
  every page, by its own words or everyday ones ("volume", "hotkeys",
  "wifi"), with a count per page in the sidebar; <kbd>↑</kbd>/<kbd>↓</kbd> go
  through the results. <kbd>Alt</kbd>+<kbd>←</kbd>/<kbd>→</kbd> (or the
  mouse's side buttons) go back and forward between pages.
- **Stays current**: Wi-Fi, Bluetooth, Sound, Power, Displays, Printers,
  Services and the assistant's Activity and Performance update by themselves while open.
- **Open config** on every page opens the underlying file in your default editor.
- **Your changes** (About) lists every option changed here, by page, to find
  or reset; **Export** and **Import** move all of Settings' settings to
  another computer as one file.
- **Apps follow the theme**: GTK, libadwaita, Qt and KDE windows and dialogs
  take the Omarchy theme's colors (through
  [hyprchroma](https://github.com/NobleDoodle/hyprchroma), if installed), plus
  the interface font and an icon theme that stays put when you change themes.
- **Units**: show temperatures in °C or °F (Appearance → Settings window).
- **Themes**: Dracula, Catppuccin (Mocha, Macchiato, Frappé, Latte), Tokyo Night
  (Night, Storm, Moon), One Dark Pro, Nord, Gruvbox, Rosé Pine, Everforest,
  Solarized and Kanagawa. Or **Follow Omarchy**, which restyles live whenever you
  change the desktop theme.
- **Sound**:
  - a **9-band equalizer** (31 Hz–8 kHz) with presets;
  - a separate **preamp** of up to +24 dB, with a limiter so loud audio doesn't
    distort — for when full volume is still too quiet;
  - **maximum volume** up to 150%, which the volume keys respect.
- **Trackpad**:
  - tap to click, and choosing which button a 2- or 3-finger tap presses;
  - 3-finger drag, and natural or reversed scrolling;
  - a **3- and 4-finger gesture editor**: pick what each swipe left/right/up/down
    and pinch in/out does, with **reverse left/right** and **reverse up/down**;
  - **looping workspace swipes**: the smooth, finger-tracking swipe continues
    from 5 back to 1, and from 1 to 5.
- **Pictures of what a setting does**: Tiling & Workspaces and Window Style
  open with a live preview of a small desktop that redraws as you change gaps,
  borders, rounding, opacity, dimming, shadows, the layout or the master width.
  The layout, where new windows go, how focus follows the mouse and the bar's
  position are chosen from picture cards, switches show a drawing of what they
  do, and Lock Screen shows when the screensaver and the lock start on a timeline.
- **Workspaces**: keep a number of workspaces always there, pin workspaces to a
  screen, and give one workspace its own layout (including floating), name,
  gaps or bare windows. The scratchpad is explained with its keys.
- **Disks**: every drive with a map of its partitions. Mount, rename, resize,
  format (ext4, btrfs, exFAT, FAT32, NTFS, optionally encrypted with LUKS2),
  check and repair, unlock, change a passphrase, delete, make new partitions in
  free space, start a new partition table, and safely remove a USB drive.
  Partitions in use are locked; the system drive and Windows partitions get a
  red warning, and anything that erases asks you to type the partition's name.
  Drive health shows temperature, wear, power-on hours and errors, with a
  warning on Home if a drive reports trouble. **Clean up** shows how full each
  filesystem is, the biggest folders in your home, and frees space from the
  package cache, unneeded packages, old logs, the Trash and thumbnails, each
  saying how much first. UDisks does the work and asks for your password.
- **Mounts**: mount a drive at a folder at every startup, add SMB shares (a NAS
  or a Windows PC; the password is kept in a root-only file), NFS shares and
  folders on another computer over SSH, each mounted when first opened so a
  missing network never holds up starting. Each one shows whether it's mounted.
- **Bar layout**: drag widgets to reorder them or move them between the left,
  center and right sections; add widgets, remove them, and add as many
  **spacers** as you like, each with its own width.
- **No hand-offs**: Wi-Fi (join, disconnect, forget, connection details),
  Bluetooth (find, pair, connect, forget), per-app volume and output,
  notification history with Do not disturb, and lock / suspend / log out /
  restart / shut down all work inside Settings instead of opening a panel.
- **Everything else**:
  - window style: gaps, borders, rounding, opacity, blur, shadows, animations,
    cursor;
  - tiling and workspaces, keyboard shortcuts (add your own, edit them, switch
    any off, all listed by kind with a filter, and a warning when keys are
    already taken), lock screen and idle, and a night light schedule;
  - keyboard layouts and Caps Lock, mouse settings (per-device too), and
    displays (with a 15-second "keep these settings?" safety revert; with
    two or more, drag them into place and **Identify** them);
  - Wi-Fi, Bluetooth, power profiles and brightness, default apps, bar plugins,
    and updates;
  - **users**: add and delete users, change passwords, make someone an
    administrator, lock accounts, manage groups, SSH keys and the SSH server,
    and set up a security key, fingerprint or sudoless Docker. Changing users
    and groups uses a small root helper, installed once from Settings →
    Users;
  - **network**: DNS provider, Wi-Fi band, firewall (ufw, with its rules listed), Tailscale, NordVPN,
    and a speed test;
  - **time & language**: time zone, automatic time, system language and
    the computer's name;
  - **boot & login**: boot and login screen themes, hibernation, snapshots and
    the boot menu;
  - **software**: web apps and terminal apps, package search, Omarchy's app
    installers, the update channel and maintenance tools;
  - **startup & rules**: programs that start with the session, plus window and
    layer rules;
  - **capture & tools**: screenshots, screen recording, reminders, weather and
    dictation;
  - **services and printers**: start, stop and enable background services, and
    add printers;
  - the lid, power button and hibernate delay (Power), the built-in screen
    (Displays), and touchpad / touchscreen switches (Trackpad).
- **Nyx Assistant**, when installed: start and stop it; wake word,
  conversation and how much it does on its own; speech recognition, talking over
  it and its voice (with a preview); what it may do (each capability, rules for
  single tools, apps it never touches); the approvals it's waiting for, its tasks
  step by step and its recent actions; how fast it answered; and its language
  models: switch, download recommended ones, or search Hugging Face.
- **Extensions** for more devices, installed from GitHub in one click
  (Settings → Extensions). Each adds its own pages under Hardware, only when
  matching hardware is found:
  - **ASUS** (needs `asusctl`): performance profiles, battery charge limit, a
    **fan curve graph** you drag (per profile and fan, with presets), firmware
    settings such as GPU mode and panel overdrive, and CPU/GPU power limits. Plus
    **Aura Lighting**: effects, an inline color picker, **Follow Omarchy theme**
    (keeps your effect across theme changes), which power states light the
    keyboard, the **Slash lightbar**, AniMe Matrix, XG Mobile light and drive lights.
  - **Logitech**: battery, pointer speed, scrolling, buttons, backlight,
    Easy-Switch and lighting for Logitech mice and keyboards.
  - **Headsets** (needs `headsetcontrol`): battery, sidetone, lights, EQ presets,
    auto power-off.
  - **Webcams** (needs `v4l2-ctl`): zoom, pan/tilt, focus, exposure, white
    balance, with framing presets for OBSBOT cameras.

  Anyone can write one: see
  [extensions](../../extensions).
- **Updates**: Settings checks GitHub once a day when it opens. A new version is
  offered on About (one click, then restart); extensions show "Update
  available" and can update together, or automatically if you switch that on.
  `nyx-settings --update` does the same from a terminal (`--check` only reports).
- **Keyboard backlight timeout** (any laptop with a keyboard backlight): turn it
  off after a set time without typing or touching the trackpad, and back on with
  the next key press.

| Trackpad gestures | Themes |
| --- | --- |
| ![Trackpad](docs/trackpad.png) | ![Theme](docs/theme.png) |
| **Window Style** | **Keyboard Shortcuts** |
| ![Window Style](docs/look.png) | ![Keyboard Shortcuts](docs/keybindings.png) |
| **Aura Lighting** (ASUS extension) | **ASUS** performance, fans and firmware (ASUS extension) |
| ![Aura Lighting](docs/aura.png) | ![ASUS](docs/asus.png) |
| **Bar layout** | |
| ![Bar layout](docs/bar.png) | |

## Install

**One line** (downloads the latest release, or builds from source if there
isn't a prebuilt one for your machine):

```sh
curl -fsSL https://raw.githubusercontent.com/design-nexus/nyx-settings/main/install.sh | bash -s -- settings
```

To also add a gear button to the Omarchy bar (left-click opens Settings,
right-click opens Sound):

```sh
curl -fsSL https://raw.githubusercontent.com/design-nexus/nyx-settings/main/install.sh | bash -s -- settings --gear
```

**From source:**

```sh
git clone https://github.com/design-nexus/nyx-settings.git
cd nyx-settings
./install.sh          # add --gear for the bar button
```

Everything installs into your home directory (`~/.local/bin/settings`, plus a
launcher entry and icon). Nothing needs root, except installing Rust if you
build from source and don't have it.

**Optional:** `sudo pacman -S lsp-plugins-lv2` adds the limiter that lets the
preamp go up to +24 dB safely. Without it, the preamp is capped lower.

### Requirements

Omarchy 4 (Hyprland 0.56+ with Lua config), GTK 4.18+, and PipeWire. These are
all part of a standard Omarchy install.

## Usage

Open **Settings** from the app launcher, or run `nyx-settings`.

| Command | What it does |
| --- | --- |
| `nyx-settings` | Open the window |
| `nyx-settings --section audio` | Open (or jump) to a page: `theme`, `look`, `trackpad`, `audio`, `displays`, `keybindings`, … |
| `nyx-settings --eq on\|off\|toggle\|status` | Turn the preamp and equalizer on or off, e.g. from a keybinding |
| `nyx-settings --volume raise\|lower\|+N\|-N` | Change volume, going past 100% up to the maximum set in Sound |
| `nyx-settings --kbd-timeout 30\|off` | Turn the keyboard backlight off after 30 idle seconds (or stop doing that) |
| `nyx-settings --apply` | Regenerate the Hyprland file from saved settings |

## How it changes your config

Settings never parses or rewrites your own config files:

- **Hyprland**: every value you change goes into one generated file,
  `~/.config/hypr/settings.lua`, which your `hyprland.lua` loads last. The
  `require` line is added the first time you change something. Each row has a
  reset button that hands the value back to your own config or Omarchy's
  default.
- **Saved state**: `~/.config/nyx-settings/state.json`. The generated file is always
  rebuilt from this. How NyxOS's defaults, your changes, remembered devices and
  what this computer can do fit together is in
  [docs/configuration.md](https://github.com/design-nexus/nyx-settings/blob/main/docs/configuration.md).
- **Keyboard backlight timeout**: a small user service, `settings-kbd-idle`, that
  asks the compositor when there's been no input (the Wayland idle-notify
  protocol). It exists only while a timeout is set.
- **Equalizer**: a PipeWire filter-chain running in its own client
  (`~/.config/pipewire/settings-eq.conf`, the `settings-eq` user service). Its
  sink feeds your speakers, and Omarchy's volume keys still control real
  loudness.
- **App theming**: Settings drives hyprchroma's own commands and never edits
  GTK or KDE files itself. The interface font and icon theme are the usual GNOME
  settings (`gsettings`). If you pick your own icons, a theme-set hook,
  `~/.config/omarchy/hooks/theme-set.d/50-settings-theme`, puts them back after
  each theme change. It also tells extensions that ask, so the ASUS extension
  can restore your keyboard lighting.
- **Extensions** live in `~/.local/share/nyx-settings/extensions/<id>` (a git
  checkout each). Settings runs each one's helper program and draws the page;
  each can be turned off without removing it (its pages are hidden), and
  `nyx-settings --ext list|install|update|enable|disable|remove` does the same from
  a terminal.
- **Nyx Assistant** settings go through its own `nyx-assistant config set`, which
  checks each value, keeps the comments in its `config.toml` and reloads it.
- **Omarchy features** (themes, fonts, the bar, idle, night light, default apps,
  plugins) are changed through Omarchy's own commands and config.

**Gestures:** Settings only defines gestures once you switch on *Manage swipe
gestures here*. If another file already defines gestures for the same fingers
and direction, Settings warns you and can switch those off (with a backup).
With *Loop around* on, Settings keeps two hidden placeholder workspaces, one
before 1 and one after the last. Swipes slide into them, then jump to the
other end of the loop.

## Custom themes

Drop a TOML file in `~/.config/nyx/themes/`:

```toml
name = "My Theme"
bg = "#1a1b26"
surface = "#16161e"
text = "#c0caf5"
dim_text = "#565f89"
accent = "#7aa2f7"
border = "#292e42"
muted = "#3b4261"
highlight = "#283457"
danger = "#f7768e"
glow = "#7aa2f7"
shadow = "#0f0f14"
light = false
```

## Uninstall

```sh
nyx uninstall settings
```

This removes the app, the keyboard backlight timeout service, and the theme-set
hook. These all run the app, so they can't stay behind without it. Your
nyx-settings are kept, so reinstalling brings the timeout back.

Add `-s -- --purge` to also remove everything Settings configured: its
Hyprland file and the line that loads it, the equalizer service, and its saved
settings.

## Development

```sh
cargo run -- --section trackpad
cargo test
cargo clippy -- -D warnings
```

It's built with Rust and gtk4-rs, and doesn't use libadwaita. Each sidebar page
is one file in `src/sections/`, and the Hyprland and audio logic lives in
`src/backend/`.

## License

[MIT](LICENSE) © Ken Smith
