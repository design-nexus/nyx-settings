# Headsets

One page per headset [HeadsetControl](https://github.com/Sapd/HeadsetControl) finds, and a
page for the Logitech ASTRO A50 X (USB `046d:0b0b`) while it's plugged in, run through
[Gearbench](https://github.com/gearbench/gearbench)'s command line, `gearbench-ctl`. Without
Gearbench the A50 X page says what it's for and links to its releases; it installs nothing.

Gearbench drives every Logitech G device it supports, so no other program should drive those
devices while it runs.

## Pretend headsets

`HEADSET_MOCK` swaps the hardware for pretend headsets, for testing without them. Nothing is
opened or run while it's set, and changes go to `mock/` in `SETTINGS_EXTENSION_STATE`.

| Value | Simulates |
| --- | --- |
| `arctis7` | HeadsetControl's `-o json` for a SteelSeries Arctis 7 |
| `a50x` | an ASTRO A50 X with Gearbench: `gearbench-ctl --json status`, `audio`, `audio-set`, `audio-eq`, `audio-mix`, and its sound devices |
| `a50x-no-gearbench` | an ASTRO A50 X plugged in, without Gearbench |

Several can be given, split by commas:

```sh
HEADSET_MOCK=arctis7,a50x SETTINGS_EXTENSION_STATE=/tmp/headset ./settings-headset pages
```

Tests: `python3 -m unittest test_headset.py`
