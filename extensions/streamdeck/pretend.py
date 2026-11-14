"""Pretend decks, so both Stream Deck pages can be shown without the hardware.

CORSAIR_MOCK=galleon-100-sd  the Galleon 100 SD's deck, with the mockup's
    profiles (Work, Gaming, Blender, Star Citizen), a galleon-deck folder to
    import that galleon-deck is still running from, and pretend services.
ELGATO_MOCK=mk2  an Elgato Stream Deck MK.2 (also xl, plus, mini, neo) with the
    mockup's Default profile.

Everything lives in SETTINGS_EXTENSION_STATE/pretend; `set` changes only that.
Nothing here is used unless the variable is set."""

import json
import os

import galleon_config as gc
import galleon_page


def galleon_on():
    return "galleon-100-sd" in (os.environ.get("CORSAIR_MOCK") or "")


def elgato_on():
    return bool(os.environ.get("ELGATO_MOCK"))


def _load(path, default):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return default


def _save(path, value):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        json.dump(value, f, indent=1)


# ------------------------------------------------------------------ Galleon

def k(icon, label, **action):
    return {"icon": chr(icon), "label": label, **action}


WORK = [
    k(0xF018D, "Terminal", exec="xdg-terminal-exec"), k(0xF024B, "Files", exec="nautilus --new-window"),
    k(0xF059F, "Browser", exec="firefox"), k(0xF040A, "Play", media="play-pause"), k(0xF04AD, "Next", media="next"),
    k(0xF0100, "Shot", exec="omarchy-cmd-screenshot", confirm=True),
    k(0xF036D, "Mute", exec="wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle"),
    k(0xF0594, "DND", exec="makoctl mode -t do-not-disturb"), k(0xF033E, "Lock", exec="loginctl lock-session"),
    k(0xF003B, "Apps", exec="omarchy-launch-walker"), {}, k(0xF0142, "Page 2", page="Page 2"),
]
WORK_2 = [k(0xF0E09, "Wallpaper", exec="omarchy-theme-bg-next"), k(0xF057E, "Volume", keys="VOLUMEUP"),
          {}, {}, {}, {}, {}, {}, {}, {}, {}, k(0xF0141, "Back", page="back")]
GAME = [
    {"label": "Resupply", "hold": "CTRL", "sequence": "S S W D"},
    {"label": "Reinforce", "hold": "CTRL", "sequence": "W S D A W"},
    {"label": "Eagle", "hold": "CTRL", "sequence": "W D S S"},
    {"label": "Orbital", "hold": "CTRL", "sequence": "D D S D"},
    {"label": "Shield", "hold": "CTRL", "sequence": "S W A D A D"},
    {"label": "Sentry", "hold": "CTRL", "sequence": "S W D W A"},
    k(0xF036C, "Push to talk", key="V"), k(0xF0100, "Clip", keys="ALT+F10"),
    k(0xF057E, "Game vol", exec="wpctl set-volume @DEFAULT_AUDIO_SINK@ 60%"),
    k(0xF0594, "DND", exec="makoctl mode -t do-not-disturb"), {}, k(0xF0141, "Back", page="back"),
]
PROFILES = {
    "Work": {"theme": "synthwave", "start_page": "Page 1",
             "page": [{"name": "Page 1", "keys": WORK}, {"name": "Page 2", "keys": WORK_2}]},
    "Gaming": {"theme": "hud", "start_page": "Stratagems", "theme_overrides": {"key_style": "hud"},
               "page": [{"name": "Stratagems", "keys": GAME},
                        {"name": "Comms", "keys": [k(0xF036C, "Talk", key="V"), {"label": "Ping", "keys": "Z"}]}]},
    "Blender": {"theme": "arctic",
                "page": [{"name": "Tools", "keys": [{"label": "Grab", "key": "G"}, {"label": "Rotate", "key": "R"},
                                                    {"label": "Scale", "key": "S"}, {"label": "Render", "key": "F12"}]}]},
    "Star Citizen": {"theme": "cyberpunk", "auto_switch": [{"class": "^starcitizen\\.exe$", "name": "Star Citizen"}],
                     "page": [{"name": "Flight", "keys": [{"label": "Gear", "key": "N"}, {"label": "Quantum", "key": "B"},
                                                          {"label": "Lights", "key": "L"}]}]},
}
CONFIG = """# The Galleon 100 SD's deck (pretend).
start_profile = "Work"
brightness = 70
volume_step = 2
clock_format = "%H:%M"

[auto_switch]
fallback = "Work"
rules = [
  { class = "^helldivers2\\\\.exe$", profile = "Gaming", name = "Helldivers 2" },
  { class = "^blender$", profile = "Blender", name = "Blender" },
]

[boot]
enabled = true

[transition]
enabled = true
"""
THEMES = {"midnight": 'bg = "#0b0b1a"\nkey_bg = "#14142b"\nfg = "#c8c8ff"\naccent = "#7a7aff"\n',
          "wallpaper": 'bg = "#160c1f"\nkey_bg = "#1f1230"\nfg = "#ff9ec7"\naccent = "#7ee0ff"\n'}
WINDOWS = [["helldivers2.exe", "HELLDIVERS™ 2"], ["firefox", "Mozilla Firefox"], ["blender", "Blender 4.2"],
           ["com.mitchellh.ghostty", "ghostty"]]


def write_folder(paths):
    os.makedirs(paths.profiles, exist_ok=True)
    gc.write_atomic(paths.config, CONFIG)
    for name, prof in PROFILES.items():
        gc.write_profile(paths.profile(name), prof)
    for name, text in THEMES.items():
        gc.write_atomic(os.path.join(paths.themes, name + ".toml"), text)


class GalleonEnv(galleon_page.Env):
    pretend = True

    def __init__(self, state, jobs):
        root = os.path.join(state, "pretend")
        self.state = os.path.join(root, "page")
        self.jobs = jobs
        self.nyx = gc.Paths(os.path.join(root, "nyx-deck", "galleon"))
        self.legacy = gc.Paths(os.path.join(root, "galleon-deck"))
        self.nyx_config = os.path.join(root, "nyx-deck", "config.toml")
        self.services = os.path.join(root, "services.json")
        if not os.path.exists(self.services):
            write_folder(self.legacy)
            write_folder(self.nyx)
            gc.write_atomic(self.nyx_config, "galleon = true\n")
            _save(self.services, {"units": {"nyx-deck.service": [True, True], gc.GALLEON_UNIT: [True, True]},
                                  "galleon-deck installed": True, "access": True})
            galleon_page.remember(self, "key", 2)

    def get(self):
        return _load(self.services, {"units": {}})

    def put(self, s):
        _save(self.services, s)

    def unit(self, unit):
        return tuple(self.get()["units"].get(unit, [False, False]))

    def set_unit(self, unit, enabled, active):
        s = self.get()
        s["units"][unit] = [enabled, active]
        self.put(s)

    def systemctl(self, *args):
        pass

    def deck_present(self):
        return True

    def galleon_deck_installed(self):
        return self.get().get("galleon-deck installed", False)

    def galleon_deck_running(self):
        return self.unit(gc.GALLEON_UNIT)[1]

    def access_ready(self):
        return self.get().get("access", False)

    def missing(self):
        return []

    def deck_state(self):
        if gc.galleon_enabled(self.nyx_config) and self.unit("nyx-deck.service")[1] and not self.galleon_deck_running():
            return {"profile": "Work", "page": "Page 1"}
        return None

    def request(self, profile, page=None):
        pass

    def windows(self):
        return [tuple(w) for w in WINDOWS]

    def now(self, fmt):
        import time
        when = time.strptime("2026-11-13 14:32", "%Y-%m-%d %H:%M")
        return time.strftime(fmt, when), time.strftime("%a %d %b", when)

    def playing(self):
        return "Midnight City"

    def wallpaper_theme(self):
        gc.write_atomic(os.path.join(self.nyx.themes, "wallpaper.toml"), THEMES["wallpaper"])

    def write_unit(self):
        pass

    def run_nyx_deck(self):
        self.set_unit("nyx-deck.service", True, True)

    def galleon_deck_off(self):
        self.set_unit(gc.GALLEON_UNIT, False, False)

    def galleon_deck_on(self):
        self.set_unit(gc.GALLEON_UNIT, True, True)

    def terminal(self, job, script):
        s = self.get()
        if job == "galleon-access":
            s["access"] = True
        elif job == "galleon-remove":
            s["galleon-deck installed"] = False
            s["units"][gc.GALLEON_UNIT] = [False, False]
            s["access"] = True
        self.put(s)

    def job_row(self, key, title, desc, label, job, doing, icon):
        return {"kind": "button", "key": key, "title": title, "desc": desc, "label": label, "icon": icon}

    def jobs_running(self):
        return False


# ------------------------------------------------------------------- Elgato

ELGATO_MODELS = {"mk2": "0080", "xl": "006c", "plus": "0084", "mini": "0063", "neo": "009a"}
ELGATO_PROFILE = """start_page = "Main"

[[page]]
name = "Main"
keys = [
  { label = "Terminal", exec = "xdg-terminal-exec" },
  { label = "Files", exec = "nautilus --new-window" },
  { label = "Browser", exec = "firefox" },
  { label = "Play", media = "play-pause" },
  { label = "Next", media = "next" },
  { label = "Shot", exec = "omarchy-cmd-screenshot" },
  { label = "Mute", keys = "SUPER+M" },
  {}, {}, {},
  { label = "DND", exec = "makoctl mode -t do-not-disturb" },
  {}, {},
  { label = "Lock", exec = "loginctl lock-session" },
  { label = "Page 2", page = "Page 2" },
]

[[page]]
name = "Page 2"
keys = [
  { label = "Back", page = "Main" },
]
"""


def use_elgato(state):
    """Point elgato.py at pretend files and a pretend deck and service. Returns
    the state folder the page should use, and `set` for the pretend service."""
    import elgato
    root = os.path.join(state, "pretend", "elgato")
    page_state = os.path.join(root, "page")
    services = os.path.join(root, "services.json")
    elgato.CONF = os.path.join(root, "nyx-deck")
    elgato.CONFIG = os.path.join(elgato.CONF, "config.toml")
    elgato.PROFILES = os.path.join(elgato.CONF, "profiles")
    elgato.UDEV_RULE = os.path.join(root, "71-nyx-elgato.rules")
    if not os.path.exists(services):
        os.makedirs(elgato.PROFILES, exist_ok=True)
        gc.write_atomic(os.path.join(elgato.PROFILES, "Default.toml"), ELGATO_PROFILE)
        gc.write_atomic(elgato.CONFIG, 'start_profile = "Default"\nbrightness = 70\n')
        gc.write_atomic(elgato.UDEV_RULE, "")
        _save(services, {"running": True})
        elgato.remember(page_state, "key", "2")
    pid = ELGATO_MODELS.get(os.environ.get("ELGATO_MOCK", ""), "0080")
    elgato.present = lambda: [(pid, "PRETEND")]
    elgato.library_ready = lambda: True
    elgato.service_active = lambda: _load(services, {}).get("running", False)

    def pretend_set(key, value):
        if key == "elgato-run":
            _save(services, {"running": value == "true"})
            return {"refresh": True}
        if key in ("elgato-install", "elgato-access"):
            return {"toast": "Pretend deck: nothing to install", "refresh": True}
        return None
    return page_state, pretend_set
