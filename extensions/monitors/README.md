# Monitors

External monitors in Settings, over DDC/CI with `ddcutil`: brightness, contrast,
colour preset, input, speaker volume and power, set on the monitor itself. One page
per monitor that answers; until the signed-in user may open `/dev/i2c-*`, a setup
page offers to install `/etc/udev/rules.d/45-nyx-ddc.rules`.

When Settings can draw a preview pane, the controls sit on the left and the monitor,
its current readings and its details on the right.

## Trying it without a monitor

`MONITORS_MOCK` answers like `ddcutil` for a pretend monitor, keeping its values in
`$SETTINGS_EXTENSION_STATE`. It never opens `/dev/i2c-*`, runs `ddcutil` or asks for a
password.

- `MONITORS_MOCK=dell-u2720q`: a DELL U2720Q on bus 4, as in the mockups.
- `MONITORS_MOCK=no-access`: the same monitor before the device rule is installed
  (the setup page). Pressing Allow pretends to install it, and the monitor's page
  appears.

```sh
MONITORS_MOCK=dell-u2720q SETTINGS_EXTENSION_STATE=/tmp/monitors-mock target/release/settings-monitors describe bus4
```

Tests: `cargo test`.
