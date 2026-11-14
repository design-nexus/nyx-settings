# Stream Deck

The Corsair Galleon 100 SD's built-in Stream Deck (USB `1b1c:2b18`) in Settings: its
12 screen keys, top screen and two dials. Elgato Stream Decks too (`elgato.py`).

## Galleon 100 SD, run by galleon-deck (today)

The deck is driven by [galleon-deck](https://github.com/NLMP-DDHS/galleon-deck)
(MIT), which runs as a user service (`galleon-deck.service`) and reads TOML files in
`~/.config/galleon-deck`. This extension fetches galleon-deck at a fixed commit and,
from the Stream Deck page:

- installs its packages and adds its device rule and the `uinput` module in a terminal
  (`sudo`, so you type your password there and can answer pacman), and runs its
  installer in the background; the page shows progress, and the last lines of output
  if a step fails;
- turns the service on or off, restarts it, or opens galleon-deck's own app (icons,
  images, themes, window rules);
- edits brightness, the start profile, clock format, animations and the volume
  step, and each profile's theme, start page and keys (label and action: a key, a
  shortcut, a command, media, a page or a profile).

Config is written with tomlkit, so comments in the files are kept; galleon-deck
picks up changes within a second. Profiles using the **Wallpaper** theme are
regenerated from the Omarchy wallpaper whenever the theme changes.

That page is what Settings shows when it can't draw the preview pane.

## Galleon 100 SD, run by nyx-deck (off until you turn it on)

`nyx-deck` (the service beside this file that runs Elgato decks) can run the Galleon's
deck itself, so galleon-deck isn't needed. `galleon.py` speaks the deck's protocol
(from node-elgato-stream-deck 7.5's "Galleon K100 SD", as galleon-deck uses it): a
keep-alive feature report every 500 ms puts it in host mode, keys and the top screen
take JPEGs, and key and dial reports come back; it types through its own uinput
keyboard and mouse. The behaviour is ported from galleon-deck (MIT; the notice is
in `galleon.py`, `galleon_config.py` and `galleon_draw.py`).

It reads galleon-deck's format from `~/.config/nyx-deck/galleon/` (`config.toml`,
`profiles/*.toml`, `themes/*.toml`), so galleon-deck's folder imports as it is. It
adds `[screen]` (`profile`, `playing`, `volume` = always / changing / never) and
`[dials]` (`left_turn`, `left_press`, `right_turn`, `right_press`,
`right_hold_turn`) to `config.toml`, and `enabled` under `[auto_switch]`;
galleon-deck ignores them.

It's off until `galleon = true` is in `~/.config/nyx-deck/config.toml`, and nyx-deck
never opens the deck while galleon-deck is running (its service, or its PID file).
When nyx-deck lets go, the keyboard goes back to its own numpad.

The page Settings draws with the preview pane (`galleon_page.py`): the switch,
brightness, profiles and pages on the left; the deck in the pane, where clicking a
key opens its editor underneath; **Switch by app**, **Top screen and dials** and
**Look** as expandable rows; and **Move from galleon-deck** (import a copy, turn
galleon-deck off or back on, remove it) and **Device access**
(`/etc/udev/rules.d/71-nyx-galleon.rules` and `uinput`, in a terminal).

To try it: on the Stream Deck page, **Import** under Move from galleon-deck, **Turn
off** galleon-deck, then turn on **Run the Stream Deck**. To go back: turn **Run the
Stream Deck** off, then **Turn on** under Move from galleon-deck.

It needs `python-pillow` (pictures) and `playerctl` (what's playing); switching by
app follows Hyprland only.

## Pretend decks

For showing the pages without the hardware. Nothing is real while these are set:
the files, services and desktop answers are pretend ones kept in
`$SETTINGS_EXTENSION_STATE/pretend`, and `set` changes only those.

- `CORSAIR_MOCK=galleon-100-sd`: the Galleon's deck with the mockup's profiles (Work,
  Gaming, Blender, Star Citizen), a galleon-deck folder to import that galleon-deck
  is still running from, and nyx-deck turned on and waiting for it.
- `ELGATO_MOCK=mk2` (or `xl`, `plus`, `mini`, `neo`): an Elgato deck with the
  mockup's Default profile.

```sh
SETTINGS_EXTENSION_STATE=$(mktemp -d) CORSAIR_MOCK=galleon-100-sd ELGATO_MOCK=mk2 \
SETTINGS_KINDS="preview icon" ./settings-streamdeck describe deck
```

## Tests

`python3 -m unittest test_galleon.py test_pages.py test_elgato.py test_settings_streamdeck.py`.
The driver is tested against a simulated deck; nothing opens hidraw or uinput. The
drawing tests need Pillow, and the old page's edits need tomlkit
(`GALLEON_EXAMPLES=testdata/galleon-deck` runs them on galleon-deck's examples).
