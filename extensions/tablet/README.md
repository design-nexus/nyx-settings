# Drawing tablets

Wacom, XP-Pen, Huion and other tablets in Settings: which screen the pen draws on, the
part of the tablet used, left-handed and mouse-like use. The settings are written to
`~/.config/hypr/nyx-tablet.lua` and applied through Hyprland at once.

## Pretend tablet

`TABLET_MOCK=wacom-intuos-s` makes `pages`, `describe` and `set` use a made-up Wacom
Intuos S (152 × 95 mm, four buttons, OpenTabletDriver installed) on one 2560×1600
screen, `eDP-1`, for checking pages without a tablet. `set` only saves the pretend
settings (`tablet-mock.json` in `SETTINGS_EXTENSION_STATE`); nothing writes Hyprland's
config or talks to Hyprland while it's on.
