# Configuration: defaults, your changes, devices and capabilities

How Nyx Settings and the desktop's components decide what a setting is, what shows,
and what happens to a device's settings when it's unplugged. Handoff 09 set this up;
the audit behind it is in
[nyxos/docs/audit](https://github.com/nyxos-project/nyxos/tree/main/docs/audit)
(`settings-audit.md`, `desktop-defaults.md`).

## Who owns what

Settings is the interface, not the owner. Each setting has one owner, the only program
that writes its authoritative value:

| Settings | Owner | Where they live |
| --- | --- | --- |
| Hyprland look, input, workspaces, keys, rules, displays | Settings | `~/.config/nyx-settings/state.json` → `~/.config/hypr/settings.lua` |
| Dock | Nyx Dock | `dock` domain |
| Control Center layout | Nyx Control Center | `control-center` domain |
| Bar layout | Nyx Bar | `bar` domain |
| Switcher | Nyx Switcher | `switcher` domain |
| Shell behaviour (focus, per-screen components, overlays, shortcuts) | Shell Core | `shell` domain, through Shell Core's socket |
| Notifications | nyx-notifyd | `notifications.db`, over the `notifications` domain's defaults |
| Theme | Theme Engine | `~/.config/nyx/theme.toml`, over the `appearance` domain's `theme` |
| Idle and lock times | NyxOS idle | `idle` domain |
| System settings (hostname, packages, services, users…) | System Manager | `/etc/nyxos/overrides/system.json`, through `nyx-system` |
| App-specific settings (Chat personas, Vault…) | each app | each app |

## Domains and layers

A domain is a file name in three layers, merged key by key (`nyx-config`, in
nyx-system):

1. `/etc/nyxos/defaults/DOMAIN.toml`: NyxOS's defaults, from `nyxos/defaults/` and
   the NixOS modules. Read-only.
2. `/etc/nyxos/overrides/DOMAIN.toml`: an administrator's defaults for everyone,
   written by the System Manager (`SetDefault`, polkit).
3. `~/.config/nyx/DOMAIN.toml`: the user's, holding **only what they changed**.

Below all three is the component's own built-in value. On Omarchy only the user's layer
exists.

What wins, highest first:

1. the user's own value;
2. a device's own setting (a display's scale, a mouse's speed);
3. the administrator's;
4. NyxOS's default;
5. the component's built-in value.

A component reads its domain with `nyx_config::Store`:

```rust
const DOMAIN: nyx_config::Domain =
    nyx_config::Domain { name: "dock", version: 1, migrations: &[nyx_config::drop_defaults] };
let store = nyx_config::Store::new(DOMAIN, nyx_config::table_of(&Config::default()));
let loaded = store.load();                                  // every layer, migrated
let (config, problems) = nyx_config::parse::<Config>(&loaded); // a bad key falls back alone
store.save(&nyx_config::table_of(&config))?;                 // only what differs
let _watch = store.watch(|| { /* any layer changed */ })?;   // feature "watch"
```

- **Writing only what differs** means a value the user never touched follows NyxOS's
  default when an update changes it. Values the user set stay.
- **Schema.** The user's file carries `schema = N`. An older file is copied to
  `DOMAIN.toml.vN.bak` and migrated; `drop_defaults` turns a file that held every
  setting into one with only the changes. The component owns its schema: Settings and
  `nyx plugin` set or reset single keys through `nyx_config::Layers` and never stamp
  one.
- **Broken files.** A user file that isn't TOML is kept as `DOMAIN.toml.broken-<time>`,
  and the other layers still load. A defaults file that's broken is reported and
  skipped.
- **Watching.** `Store::watch` sees all three folders, and `/etc/static`, which a NixOS
  switch replaces without touching `/etc/nyxos/defaults/*` themselves.
- **Reset** removes the user's key: `Store::reset(key)`, or `nyx-system config reset
  DOMAIN KEY` from a terminal. `Store::explain(key)` says what's in effect, where it
  comes from, and what a reset leaves. The Assistant asks the same through Settings'
  read-only `settings.value` tool.

Settings' own `state.json` (schema 2) and `settings.toml` (schema 1) follow the same
rules: backed up before an upgrade, set aside when broken.

### Hyprland

NyxOS's Hyprland defaults are data, `nyxos/defaults/hyprland.toml`, rendered into each
user's `hyprland.lua` by `modules/home/hyprland.nix` and installed as
`/etc/nyxos/defaults/hyprland.toml`. Settings' `settings.lua` loads last, so its values
win, and a row's reset removes the key from `state.json`.

Hyprland refuses a gesture that an earlier one shadows. So Settings reads the shipped
`[[gestures]]` and unsets each one on the same fingers and direction before setting its
own.

## Capabilities

`nyx-capability` (nyx-core) answers what this computer can do right now. Each
capability has an id (`program:nyx-dock`, `service:user:nyx-notifyd`,
`bus:system:org.bluez`, `hw:touchpad`, `daemon:shell-core`, `daemon:audio`,
`hw:fingerprint`…) and a state:

| State | Page or control |
| --- | --- |
| Available, Inactive (can be started from the page) | shown |
| TemporarilyUnavailable, PermissionRestricted | shown, dimmed, with the reason |
| NotInstalled, Unsupported | hidden |

A state is runtime information only. An unavailable capability makes a preference
inapplicable for now; it never deletes it.

Nothing polls. A provider names what can change its answer: folders (inotify, including
systemd's `units/` folder for services), D-Bus `NameOwnerChanged`, udev subsystems, or a
command's lines (`pactl subscribe`). It's probed again only when one of those fires.

In Settings:

- **Pages.** Each `Section` lists its `needs`. Only sidebar entries that come or go are
  redrawn. A page whose capability goes while it's open stays, saying so, with its
  settings kept.
- **Controls.** `capability::bind(widget, &[ids])` makes a control follow a capability.
  `capability::rebuild_on(&[ids], page)` redraws a page whose rows depend on hardware
  coming and going (a fingerprint reader, a keyboard light).
- **Live pages** (Wi-Fi, Bluetooth, Sound, Power, Services, Displays) redraw when
  NetworkManager, BlueZ, PipeWire, udev or systemd announce a change (`live.rs`).
  Pages nothing announces changes for are still checked every 4 seconds.
- **Search** only finds pages in the sidebar.

## Remembered devices

A device the user configured keeps its settings while unplugged, and they come back
when it returns. Never-configured devices aren't remembered.

| Device | Identity | Settings and their owner | Restored by |
| --- | --- | --- | --- |
| Display | make, model and serial (`stable_id`) | `state.json` `monitors`, the dock's `[monitors]`, Shell Core's `monitors.screens` | Hyprland's `desc:` monitor rules |
| Mouse | Hyprland device name | `state.json` `devices` | `hl.device` by name |
| Sound device | PipeWire `node.name` | `~/.config/nyx-settings/audio-devices.toml` | a WirePlumber rule (`wireplumber.conf.d/51-nyx-settings.conf`) |
| Bluetooth | BlueZ address | none in Settings: BlueZ keeps the pairing and lists paired devices, connected or not | BlueZ |
| Drive | filesystem UUID, only with a configured mount | the mount table / `system.json` | the mount |

- **The index.** `nyx-devices` keeps `~/.config/nyx/devices.json`: each device's name,
  what its subsystem matches it by, and when and on which port it was last seen. The
  settings themselves stay with their owners, so there's no second copy of them.
- **Displays kept by port.** Displays with no identity (virtual outputs), and two
  identical displays without a serial, keep their settings by port; the Displays page
  says so. Entries from before Settings knew displays by identity move over the first
  time the display is seen.
- **Pages.** Displays, Mouse and Sound have a folded *Remembered* group. There the
  settings can be changed while the device is away. Live actions (Identify, brightness,
  a test sound) need the device connected.
- **Forget.** It goes through every owner (`nyx_devices::Owner`). If one fails, the
  device stays remembered rather than half forgotten. Bluetooth keeps Disconnect and
  Unpair apart.
- **Settings that didn't take.** A connected display whose saved settings Hyprland
  didn't apply says which ones.

## Theme

`theme.toml` names a system theme only when the user chose one. Otherwise the theme
engine takes `theme` from the `appearance` domain (an administrator's, else NyxOS's),
else Catppuccin Mocha. A theme written at a user's first login is taken out once, and
Reset goes back to following the default. `nyx-themed` watches the appearance layers,
so a new default arrives at once.

## Tests

- `nyx-config`: precedence, reset to a new default, migration with backup, broken
  files, watch.
- `nyx-capability`: registry changes, sources, fake sysfs.
- `nyx-devices`: identity, ambiguity, forget.
- Each component: its domain, including copies of the reference laptop's files.
- Settings: `cargo test --bin nyx-settings`. The Shell Core client is tested against a
  running Shell Core with
  `NYX_SHELL_SOCKET=… cargo test --bin nyx-settings shell_core_live -- --ignored`.
