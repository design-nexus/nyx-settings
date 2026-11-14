# Logitech for Settings

Logitech mice and keyboards as pages in Settings, over HID++ on `/dev/hidraw*`:
battery, pointer speed, scrolling, buttons, backlight, Easy-Switch, onboard profiles
and lighting. `settings-logitech pages | describe PAGE | set PAGE KEY VALUE | watch`.

## Gearbench

When Gearbench is set up (its daemon's socket `$XDG_RUNTIME_DIR/gearbench.sock` is
there, or `gearbench-ctl` and `gearbench.service` are installed), the devices it drives,
by the product ids its guide lists, are never opened or written: two programs driving
one device read each other's replies. They still get a page, with what udev knows of
them and a button that runs `gearbench-ctl gui`. A receiver with any of them behind it
is left alone too.

## Pretend devices

`LOGI_MOCK` runs everything against simulated receivers and devices instead of the
hardware, for testing pages without the devices:

```sh
LOGI_MOCK=mx-master-3s,mx-keys,g502x settings-logitech describe b034-8a215c03
```

- `mx-master-3s` and `mx-keys` sit on a Bolt receiver, `g502x` (G502 X LIGHTSPEED) on a
  Lightspeed receiver, `mx-anywhere-3s` on Bluetooth. Their readings match the Settings
  mockups.
- `:off` after a name makes that device paired but switched off (`mx-keys:off`).
- `denied` makes every node refuse to open, as before the udev rule is installed, so
  `pages` shows the setup page. Allowing access then does nothing.
- `gearbench` pretends [Gearbench](https://github.com/gearbench/gearbench) is set up, so
  the Lightspeed receiver and the G502 behind it are left alone and the G502 gets the
  short "Gearbench drives this device" page. Opening Gearbench then does nothing.
- What `set` changes is kept in `SETTINGS_EXTENSION_STATE` (or a folder in the temp
  directory), never in `~/.config/logi/devices.toml`, and "Keep settings applied" only
  writes a file there instead of touching systemd.

Nothing pretend happens unless `LOGI_MOCK` is set.
