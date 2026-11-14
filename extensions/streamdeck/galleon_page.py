"""The Galleon 100 SD's Stream Deck page in style A, for nyx-deck running the deck:
the deck drawn in the pane (click a key to edit it underneath), the switch,
brightness, profiles and pages on the left, and switching by app, the top screen
and dials, and the look as expandable rows. It edits nyx-deck's copy of the
config (galleon_config) and moves you over from galleon-deck: import its files
(a copy), turn it off, remove it, and allow the deck and typing.

Settings draws this when SETTINGS_KINDS has `preview`; otherwise the old
galleon-deck page in settings-streamdeck is shown, unchanged."""

import os
import re
import shlex
import shutil
import signal
import subprocess
import time

import galleon_config as gc

HOME = os.path.expanduser("~")
NYX_UNIT = "nyx-deck.service"
UDEV_RULE = "/etc/udev/rules.d/71-nyx-galleon.rules"
MODULES_LOAD = "/etc/modules-load.d/nyx-galleon.conf"
GALLEON_RULE = "/etc/udev/rules.d/71-galleon-deck.rules"
GALLEON_MODULES = "/etc/modules-load.d/galleon-deck.conf"
RULE_TEXT = (
    "# Written by Settings: lets the signed-in user run the Galleon 100 SD's deck (1b1c:2b18)\n"
    "# and type through a virtual keyboard.\n"
    'KERNEL=="hidraw*", SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1b1c", ATTRS{idProduct}=="2b18", MODE="0660", TAG+="uaccess"\n'
    'KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"\n'
)
NEEDS = ["python-pillow", "playerctl"]

ACTIONS = [
    ["none", "Nothing"], ["key", "A key"], ["keys", "A shortcut"], ["sequence", "A sequence of keys"],
    ["mouse", "Mouse steps"], ["repeat", "Repeat a key"], ["exec", "Run a command"], ["media", "Media"],
    ["page", "Go to a page"], ["profile", "Switch profile"],
]
ACTION_EXTRAS = ("confirm", "hold", "step_ms", "loop", "repeat_ms")
MEDIA = [["play-pause", "Play / pause"], ["next", "Next"], ["previous", "Previous"], ["stop", "Stop"]]
VOLUME_BAR = [["always", "Always"], ["changing", "While it changes"], ["never", "Never"]]
TURNS = [["volume", "Volume"], ["pages", "Pages"], ["profiles", "Profiles"], ["none", "Nothing"]]
PRESSES = [["mute", "Mute"], ["play-pause", "Play / pause"], ["start-page", "Start page"], ["none", "Nothing"]]
HOLD_TURNS = [["profiles", "Profiles"], ["none", "Nothing"]]
BOOT = [["crt", "CRT power-on"], ["gif", "Your own GIF"], ["off", "Nothing"]]
SHAPES = [["rounded", "Rounded"], ["hud", "Cut corners"]]
THEME_COLOURS = [("key_bg", "Keys"), ("fg", "Text"), ("accent", "Accent")]

# The pane draws GTK icons, not the deck's Nerd Font glyphs: these are the ones
# galleon-deck and its examples use, and the rest show what the key does.
GLYPH_ICONS = {
    0xF018D: "utilities-terminal-symbolic", 0xF024B: "folder-symbolic", 0xF059F: "web-browser-symbolic",
    0xF040A: "media-playback-start-symbolic", 0xF040E: "media-playback-start-symbolic",
    0xF03E4: "media-playback-pause-symbolic", 0xF04AD: "media-skip-forward-symbolic",
    0xF04AE: "media-skip-backward-symbolic", 0xF04DB: "media-playback-stop-symbolic",
    0xF0100: "camera-photo-symbolic", 0xF036C: "audio-input-microphone-symbolic",
    0xF036D: "microphone-sensitivity-muted-symbolic", 0xF0311: "input-keyboard-symbolic",
    0xF004D: "go-previous-symbolic", 0xF0141: "go-previous-symbolic", 0xF0142: "go-next-symbolic",
    0xF02DC: "user-home-symbolic", 0xF057E: "audio-volume-high-symbolic", 0xF075F: "audio-volume-muted-symbolic",
    0xF033E: "system-lock-screen-symbolic", 0xF003B: "view-app-grid-symbolic", 0xF0594: "weather-clear-night-symbolic",
    0xF0E09: "preferences-desktop-wallpaper-symbolic", 0xF00DE: "display-brightness-symbolic",
    0xF00E0: "display-brightness-symbolic",
}
ACTION_ICONS = {
    "key": "input-keyboard-symbolic", "keys": "preferences-desktop-keyboard-shortcuts-symbolic",
    "sequence": "media-playlist-consecutive-symbolic", "mouse": "input-mouse-symbolic",
    "repeat": "media-playlist-repeat-symbolic", "exec": "system-run-symbolic", "page": "go-next-symbolic",
    "profile": "view-list-symbolic",
}
MEDIA_ICONS = {"play-pause": "media-playback-start-symbolic", "next": "media-skip-forward-symbolic",
               "previous": "media-skip-backward-symbolic", "stop": "media-playback-stop-symbolic"}


def supported():
    return "preview" in (os.environ.get("SETTINGS_KINDS") or "").split(" ")


def esc(text):
    return str(text).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


# ------------------------------------------------------------ environment

class Env:
    """Where the page reads and writes: the real files, services and desktop."""

    pretend = False

    def __init__(self, state, jobs):
        self.state = state
        self.jobs = jobs
        self.nyx = gc.nyx_paths()
        self.legacy = gc.legacy_paths()
        self.nyx_config = gc.nyx_config()

    def unit(self, unit):
        def ask(verb):
            try:
                return subprocess.run(["systemctl", "--user", verb, unit], capture_output=True, text=True, timeout=5).stdout.strip()
            except (OSError, subprocess.TimeoutExpired):
                return ""
        return ask("is-enabled") == "enabled", ask("is-active") == "active"

    def systemctl(self, *args):
        r = subprocess.run(["systemctl", "--user", *args], capture_output=True, text=True, timeout=30)
        if r.returncode != 0:
            raise SystemExit((r.stderr or r.stdout).strip() or "systemctl failed")

    def deck_present(self):
        return self.jobs.deck_present()

    def galleon_deck_installed(self):
        return any(os.path.exists(p) for p in (os.path.join(HOME, ".local/bin/galleon-deck"), "/usr/bin/galleon-deck",
                                               os.path.join(gc.config_home(), "systemd/user", gc.GALLEON_UNIT)))

    def galleon_deck_running(self):
        return gc.galleon_deck_running()

    def access_ready(self):
        return ((os.path.exists(UDEV_RULE) or os.path.exists(GALLEON_RULE))
                and (os.path.exists(MODULES_LOAD) or os.path.exists(GALLEON_MODULES)))

    def missing(self):
        out = []
        if subprocess.run(["/usr/bin/python3", "-c", "import PIL"], capture_output=True).returncode != 0:
            out.append("python-pillow")
        if not shutil.which("playerctl"):
            out.append("playerctl")
        return out

    def deck_state(self):
        return gc.read_json(gc.state_file())

    def request(self, profile, page=None):
        gc.write_json(gc.request_file(), {"profile": profile, **({"page": page} if page else {})})

    def windows(self):
        """(class, title) of the open windows, for a new switching rule."""
        import json
        try:
            out = subprocess.run(["hyprctl", "clients", "-j"], capture_output=True, text=True, timeout=2).stdout
            return [(c.get("class", ""), c.get("title", "")) for c in json.loads(out or "[]") if c.get("class")]
        except (OSError, ValueError, subprocess.TimeoutExpired):
            return []

    def now(self, fmt):
        return time.strftime(fmt), time.strftime("%a %d %b")

    def playing(self):
        if not shutil.which("playerctl"):
            return ""
        try:
            if subprocess.run(["playerctl", "status"], capture_output=True, text=True, timeout=1).stdout.strip() != "Playing":
                return ""
            return subprocess.run(["playerctl", "metadata", "title"], capture_output=True, text=True, timeout=1).stdout.strip()
        except (OSError, subprocess.TimeoutExpired):
            return ""

    def wallpaper_theme(self):
        image = os.path.join(HOME, ".config/omarchy/current/background")
        argv = ["/usr/bin/python3", os.path.join(os.path.dirname(os.path.realpath(__file__)), "nyx-deck"), "--theme-from-wallpaper"]
        r = subprocess.run(argv + ([os.path.realpath(image)] if os.path.exists(image) else []), capture_output=True, text=True, timeout=60)
        if r.returncode != 0:
            lines = (r.stderr or r.stdout).strip().splitlines()
            raise SystemExit(lines[-1] if lines else "Couldn't make a theme from the wallpaper")

    def write_unit(self):
        import elgato
        os.makedirs(os.path.dirname(elgato.UNIT_FILE), exist_ok=True)
        with open(elgato.UNIT_FILE, "w") as f:
            f.write(elgato.unit_text())
        subprocess.run(["systemctl", "--user", "daemon-reload"], capture_output=True)

    def run_nyx_deck(self):
        self.write_unit()
        self.systemctl("enable", NYX_UNIT)
        self.systemctl("restart", NYX_UNIT)

    def galleon_deck_off(self):
        subprocess.run(["systemctl", "--user", "disable", "--now", gc.GALLEON_UNIT], capture_output=True, timeout=30)
        try:
            with open(os.path.join(gc.runtime_dir(), "galleon-deck.pid")) as f:
                os.kill(int(f.read().strip()), signal.SIGTERM)
        except (OSError, ValueError):
            pass

    def galleon_deck_on(self):
        self.systemctl("enable", "--now", gc.GALLEON_UNIT)

    def terminal(self, job, script):
        self.jobs.start_terminal(job, script)

    def job_row(self, key, title, desc, label, job, doing, icon):
        row = self.jobs.step_row(key, title, desc, label, job, doing)
        row["icon"] = icon
        return row

    def jobs_running(self):
        return any((self.jobs.job_state(j) or ("",))[0] == "running" for j in ("galleon-needs", "galleon-access", "galleon-remove"))


def recall(env, name):
    try:
        with open(os.path.join(env.state, "galleon-" + name)) as f:
            return f.read().strip()
    except OSError:
        return ""


def remember(env, name, value):
    os.makedirs(env.state, exist_ok=True)
    with open(os.path.join(env.state, "galleon-" + name), "w") as f:
        f.write(str(value))


# ------------------------------------------------------------------ reading

def config(env):
    return gc.read_toml(env.nyx.config)


def profile_doc(env, name):
    return gc.read_toml(env.nyx.profile(name)) if name else {}


def page_names(prof):
    return [p.get("name", f"page {i + 1}") for i, p in enumerate(prof.get("page", []))]


def current(env):
    """The profile and page being edited: the one picked here, else what the deck shows, else the start ones."""
    names = gc.profile_names(env.nyx)
    deck = env.deck_state() or {}
    profile = recall(env, "profile")
    if profile not in names:
        profile = deck.get("profile")
    if profile not in names:
        start = config(env).get("start_profile")
        profile = start if start in names else (names[0] if names else "")
    prof = profile_doc(env, profile)
    pages = page_names(prof)
    page = recall(env, "page")
    if page not in pages and deck.get("profile") == profile:
        page = deck.get("page")
    if page not in pages:
        start = prof.get("start_page")
        page = start if start in pages else (pages[0] if pages else "")
    return profile, prof, page


def page_keys(prof, page):
    for p in prof.get("page", []):
        if p.get("name") == page:
            keys = [dict(k) for k in p.get("keys", [])][:gc.KEYS]
            return keys + [{} for _ in range(gc.KEYS - len(keys))]
    return [{} for _ in range(gc.KEYS)]


def selected(env):
    try:
        return max(0, min(gc.KEYS - 1, int(recall(env, "key") or 0)))
    except ValueError:
        return 0


def key_icon(spec):
    """A GTK icon for the pane: the glyph's, or what the key does. A plain text key has none, as on the deck."""
    icon = spec.get("icon")
    if isinstance(icon, str) and len(icon) == 1 and ord(icon) in GLYPH_ICONS:
        return GLYPH_ICONS[ord(icon)]
    action = gc.action_of(spec)
    if not (icon or spec.get("image")) and spec.get("label") and action in ("key", "keys", "none"):
        return ""
    if action == "media":
        return MEDIA_ICONS.get(spec.get("media"), "media-playback-start-symbolic")
    if action == "page" and spec.get("page") == "back":
        return "go-previous-symbolic"
    return ACTION_ICONS.get(action, "")


def key_label(spec):
    action = gc.action_of(spec)
    return str(spec.get("label") or (spec.get(action, "") if action in ("key", "keys", "page", "profile", "repeat") else ""))


def glyph_code(icon):
    """How an icon glyph is shown for editing: its code point, like F059F."""
    if isinstance(icon, str) and len(icon) == 1 and ord(icon) > 0xE000:
        return f"{ord(icon):X}"
    return icon or ""


def parse_glyph(text):
    text = text.strip()
    m = re.fullmatch(r"(?:U\+|\\U|\\u|0x)?([0-9A-Fa-f]{4,6})", text)
    return chr(int(m.group(1), 16)) if m else text


def rule_title(rule):
    if rule.get("name"):
        return rule["name"]
    pattern = rule.get("class") or rule.get("title") or ""
    return re.sub(r"\\(.)", r"\1", pattern.strip("^$")) or "Any window"


def rule_desc(rule):
    if rule.get("class"):
        cls = re.sub(r"\\(.)", r"\1", rule["class"].strip("^$"))
        return f"Window class <tt>{esc(cls)}</tt>"
    return f"Window title <tt>{esc(rule.get('title', ''))}</tt>"


def legacy_counts(env):
    profiles = len(gc.profile_names(env.legacy))
    themes = len([p for p in os.listdir(env.legacy.themes) if p.endswith(".toml")]) if os.path.isdir(env.legacy.themes) else 0
    return profiles, themes


def plural(n, word):
    return f"{n} {word}" + ("" if n == 1 else "s")


# ------------------------------------------------------------------ the page

def describe(env):
    nyx_profiles = gc.profile_names(env.nyx)
    legacy_here = os.path.exists(env.legacy.config) or bool(gc.profile_names(env.legacy))
    if not nyx_profiles and not legacy_here:
        write_starter(env)
        nyx_profiles = gc.profile_names(env.nyx)
    on = gc.galleon_enabled(env.nyx_config)
    nyx_enabled, nyx_active = env.unit(NYX_UNIT)
    galleon_running = env.galleon_deck_running()
    banners, groups = [], []
    if not env.deck_present():
        banners.append({"text": "The Stream Deck isn't connected right now. Changes are saved and apply when it is.", "warning": False})
    if galleon_running:
        banners.append({"text": "<b>galleon-deck is running.</b> Settings runs the deck itself now: bring your profiles over, then turn galleon-deck off.",
                        "warning": True})
    setup = setup_rows(env)
    if setup:
        groups.append({"title": "Set up", "rows": setup})

    run = {"kind": "switch", "key": "g-run", "title": "Run the Stream Deck", "icon": "system-shutdown-symbolic",
           "desc": "Off, the keys type numbers like a numpad", "value": on and nyx_enabled, "refresh": True,
           "keywords": "service nyx-deck on off numpad"}
    if on and galleon_running:
        run["tag"] = "Waiting for galleon-deck"
    elif on and not nyx_active:
        run["tag"] = "Not running"
    cfg = config(env)
    groups.append({"title": "Stream Deck", "rows": [run, {
        "kind": "slider", "key": "g-brightness", "title": "Brightness", "icon": "display-brightness-symbolic",
        "value": cfg.get("brightness", 70), "min": 10, "max": 100, "step": 5, "unit": "%", "on_release": True,
        "keywords": "screen lcd brightness"}]})

    if nyx_profiles:
        profile, prof, page = current(env)
        groups.append(profiles_group(env, cfg, profile, prof, page))
        groups.append({"title": "More", "rows": [auto_row(env, cfg, profile), screen_row(cfg), look_row(env, cfg, profile, prof)]})
    if legacy_here or env.galleon_deck_installed():
        groups.append(move_group(env, nyx_profiles, galleon_running))
    groups.append({"title": "Device access", "rows": [access_row(env)]})

    page_desc = {"banners": banners, "groups": groups, "poll": 2 if env.jobs_running() else 0}
    if not nyx_profiles:
        page_desc["preview"] = legacy_preview(env)
    elif on and galleon_running:
        page_desc["preview"] = preview(env, cfg, False, "nyx-deck takes the deck once galleon-deck is off.")
    elif on and nyx_active:
        page_desc["preview"] = preview(env, cfg, True, "Every change shows on the deck within a second.")
    else:
        page_desc["preview"] = preview(env, cfg, False, "Turn on Run the Stream Deck to see these on the deck.")
    return page_desc


def setup_rows(env):
    missing = env.missing()
    if not missing:
        return []
    return [env.job_row("g-needs", "Install what it needs",
                        f"<tt>{' '.join(missing)}</tt>, for the pictures on the keys and what's playing. Opens a terminal.",
                        "Install…", "galleon-needs", "Installing", "system-software-install-symbolic")]


def profiles_group(env, cfg, profile, prof, page):
    names = gc.profile_names(env.nyx)
    pages = page_names(prof)
    count = plural(len(pages), "page") + (" · the last is the deck's own settings page" if prof.get("settings_page", True) else "")
    manage = [
        {"kind": "entry", "key": "g-new-profile", "title": "New profile", "icon": "list-add-symbolic", "value": "",
         "placeholder": "Profile name"},
        {"kind": "entry", "key": "g-rename-profile", "title": "Rename this profile", "icon": "document-edit-symbolic",
         "value": profile},
        {"kind": "choice", "key": "g-start-profile", "title": "Start with", "icon": "go-home-symbolic",
         "desc": "The profile the deck shows when it starts", "options": [[n, n] for n in names],
         "value": cfg.get("start_profile", names[0] if names else "")},
        {"kind": "entry", "key": "g-rename-page", "title": "Rename this page", "icon": "document-edit-symbolic", "value": page},
        {"kind": "choice", "key": "g-start-page", "title": "Start page", "icon": "go-home-symbolic",
         "desc": "Where the right dial's press goes back to", "options": [[p, p] for p in pages],
         "value": prof.get("start_page", pages[0] if pages else "")},
        {"kind": "buttons", "key": "g-move-page", "title": "Move this page", "icon": "view-sort-ascending-symbolic",
         "options": [["earlier", "Earlier"], ["later", "Later"]]},
        {"kind": "switch", "key": "g-settings-page", "title": "The deck's own settings page", "icon": "emblem-system-symbolic",
         "desc": "Themes, brightness and profiles, last", "value": prof.get("settings_page", True) is not False},
        {"kind": "button", "key": "g-remove-page", "title": "Remove this page", "icon": "user-trash-symbolic",
         "label": "Remove", "confirm": f"Remove “{page}”?", "destructive": True, "disabled": len(pages) < 2},
        {"kind": "button", "key": "g-remove-profile", "title": "Remove this profile", "icon": "user-trash-symbolic",
         "label": "Remove", "confirm": f"Remove “{profile}”?", "destructive": True, "disabled": len(names) < 2},
    ]
    return {"title": "Profiles", "rows": [
        {"kind": "choice", "key": "g-profile", "title": "Profile", "icon": "view-list-symbolic", "desc": "The deck switches too",
         "options": [[n, n] for n in names], "value": profile, "refresh": True},
        {"kind": "choice", "key": "g-page", "title": "Page", "icon": "view-grid-symbolic", "desc": count,
         "options": [[p, p] for p in pages], "value": page, "refresh": True},
        {"kind": "entry", "key": "g-add-page", "title": "Add a page", "icon": "list-add-symbolic", "value": "",
         "placeholder": "Page name"},
        {"kind": "disclosure", "key": "g-manage", "title": "Manage profiles", "icon": "document-edit-symbolic",
         "desc": f"Add, rename, reorder · {plural(len(names), 'profile')}", "rows": manage},
    ]}


def auto_row(env, cfg, profile):
    auto = cfg.get("auto_switch", {})
    names = gc.profile_names(env.nyx)
    rules = auto.get("rules", [])
    own = [(name, rule) for name in names for rule in profile_doc(env, name).get("auto_switch", [])]
    summary = ", ".join(f"{rule_title(r)} → {r.get('profile')}" for r in rules[:2]) or "No rules yet"
    on = auto.get("enabled", True) is not False
    inner = [
        {"kind": "switch", "key": "g-auto", "title": "Switch profiles by the focused app", "icon": "window-restore-symbolic",
         "desc": "On Hyprland", "value": on},
        {"kind": "choice", "key": "g-auto-fallback", "title": "When none matches", "icon": "go-home-symbolic",
         "options": [["", "Stay on the profile"]] + [[n, f"Go back to {n}"] for n in names], "value": auto.get("fallback", "")},
    ]
    for n, rule in enumerate(rules):
        inner.append({"kind": "choice", "key": f"g-rule:{n}", "title": rule_title(rule), "icon": "application-x-executable-symbolic",
                      "desc": rule_desc(rule), "options": [[p, p] for p in names] + [["-", "Remove this rule"]],
                      "value": rule.get("profile", ""), "refresh": True})
    for name, rule in own:
        inner.append({"kind": "info", "title": rule_title(rule), "icon": "application-x-addon-symbolic",
                      "desc": f"From the {esc(name)} profile's own rules", "value": name})
    known = {r.get("class") for r in rules} | {r.get("class") for _, r in own}
    windows, seen = [], set()
    for cls, title in env.windows():
        pattern = f"^{re.escape(cls)}$"
        if cls in seen or pattern in known:
            continue
        seen.add(cls)
        windows.append([cls, title if len(title) <= 30 else title[:29] + "…"])
    inner.append({"kind": "choice", "key": "g-rule-add", "title": "Add a rule", "icon": "list-add-symbolic",
                  "desc": f"Pick an open app's window; it switches to {esc(profile)}",
                  "options": [["", "Pick a window…"]] + windows, "value": "", "refresh": True, "disabled": not windows})
    return {"kind": "disclosure", "key": "g-more-auto", "title": "Switch by app", "icon": "window-restore-symbolic",
            "desc": summary if on else "Off", "rows": inner, "keywords": "auto switch window class game focus"}


def screen_row(cfg):
    screen = cfg.get("screen", {})
    dials = cfg.get("dials", {})

    def dial(key):
        import galleon
        return dials.get(key, galleon.DIAL_DEFAULTS[key])
    inner = [
        {"kind": "switch", "key": "g-screen-profile", "title": "Profile and page", "icon": "view-list-symbolic",
         "value": screen.get("profile", True) is not False},
        {"kind": "entry", "key": "g-clock", "title": "Clock and date", "icon": "preferences-system-time-symbolic",
         "value": cfg.get("clock_format", "%H:%M"), "placeholder": "%H:%M",
         "desc": "<tt>%H:%M</tt> is 24-hour, <tt>%I:%M %p</tt> 12-hour"},
        {"kind": "switch", "key": "g-screen-playing", "title": "What's playing", "icon": "media-playback-start-symbolic",
         "desc": "From any player", "value": screen.get("playing", True) is not False},
        {"kind": "choice", "key": "g-screen-volume", "title": "Volume bar", "icon": "audio-volume-high-symbolic",
         "desc": "A bar across the screen", "options": VOLUME_BAR, "value": screen.get("volume", "always")},
        {"kind": "choice", "key": "g-dial:left_turn", "title": "Left dial: turn", "icon": "object-rotate-right-symbolic",
         "options": TURNS, "value": dial("left_turn")},
        {"kind": "choice", "key": "g-dial:left_press", "title": "Left dial: press", "icon": "media-record-symbolic",
         "options": PRESSES, "value": dial("left_press")},
        {"kind": "slider", "key": "g-volume-step", "title": "Volume step", "icon": "audio-volume-medium-symbolic",
         "desc": "Each click", "value": cfg.get("volume_step", 2), "min": 1, "max": 10, "unit": "%", "on_release": True},
        {"kind": "choice", "key": "g-dial:right_turn", "title": "Right dial: turn", "icon": "object-rotate-right-symbolic",
         "options": TURNS, "value": dial("right_turn")},
        {"kind": "choice", "key": "g-dial:right_press", "title": "Right dial: press", "icon": "media-record-symbolic",
         "options": PRESSES, "value": dial("right_press")},
        {"kind": "choice", "key": "g-dial:right_hold_turn", "title": "Right dial: hold and turn", "icon": "view-list-symbolic",
         "options": HOLD_TURNS, "value": dial("right_hold_turn")},
    ]
    shown = ["Clock"] + (["what's playing"] if screen.get("playing", True) is not False else [])
    turns = {"volume": "volume", "pages": "pages", "profiles": "profiles"}
    dial_line = " and ".join(t for t in (turns.get(dial("left_turn")), turns.get(dial("right_turn"))) if t)
    return {"kind": "disclosure", "key": "g-more-screen", "title": "Top screen and dials", "icon": "preferences-system-time-symbolic",
            "desc": ", ".join(shown) + (f" · {dial_line}" if dial_line else ""), "rows": inner,
            "keywords": "lcd clock volume dial knob media now playing"}


def look_row(env, cfg, profile, prof):
    themes = gc.theme_names(env.nyx)
    theme = gc.profile_theme(env.nyx, prof)
    over = prof.get("theme_overrides", {})
    boot = cfg.get("boot", {})
    boot_mode = "off" if boot.get("enabled", True) is False else ("gif" if boot.get("gif") else "crt")
    inner = [
        {"kind": "choice", "key": "g-theme", "title": "Theme", "icon": "preferences-desktop-appearance-symbolic",
         "desc": f"For the {esc(profile)} profile", "options": [[t, "HUD" if t == "hud" else t.capitalize()] for t in themes],
         "value": prof.get("theme", gc.DEFAULT_THEME), "refresh": True},
        {"kind": "button", "key": "g-wallpaper-theme", "title": "Make one from the wallpaper",
         "icon": "preferences-desktop-wallpaper-symbolic", "desc": "Made again when the theme changes", "label": "Make"},
        {"kind": "segmented", "key": "g-key-style", "title": "Key shape", "icon": "view-grid-symbolic", "options": SHAPES,
         "value": "hud" if theme.get("key_style") == "hud" else "rounded"},
    ]
    for field, title in THEME_COLOURS:
        inner.append({"kind": "colour", "key": f"g-colour:{field}", "title": title, "icon": "color-select-symbolic",
                      "compact": True, "value": gc.colour(theme.get(field, ""), theme)})
    if any(k in over for k, _ in THEME_COLOURS):
        inner.append({"kind": "button", "key": "g-colour-reset", "title": "The theme's own colours", "icon": "edit-undo-symbolic",
                      "label": "Reset"})
    inner.append({"kind": "choice", "key": "g-boot", "title": "When you sign in or plug it in", "icon": "system-reboot-symbolic",
                  "options": BOOT, "value": boot_mode, "refresh": True})
    if boot_mode == "gif":
        inner.append({"kind": "entry", "key": "g-boot-gif", "title": "Your own animation", "icon": "image-x-generic-symbolic",
                      "desc": "A GIF, APNG or WebP", "value": boot.get("gif", ""), "placeholder": "~/Videos/intro.gif"})
    inner.append({"kind": "switch", "key": "g-transition", "title": "Profile switch", "icon": "media-playlist-shuffle-symbolic",
                  "desc": "A short glitch, then the new profile's logo",
                  "value": cfg.get("transition", {}).get("enabled", True) is not False})
    shape = "cut corners" if theme.get("key_style") == "hud" else "rounded keys"
    name = prof.get("theme", gc.DEFAULT_THEME)
    return {"kind": "disclosure", "key": "g-more-look", "title": "Look", "icon": "preferences-desktop-appearance-symbolic",
            "desc": f"{'HUD' if name == 'hud' else name.capitalize()}, {shape}" + (", the boot animation" if boot_mode != "off" else ""),
            "rows": inner, "keywords": "theme colours wallpaper animation boot"}


def move_group(env, nyx_profiles, galleon_running):
    rows = []
    legacy_profiles, legacy_themes = legacy_counts(env)
    imported = recall(env, "imported")
    if legacy_profiles:
        where = "<tt>~/.config/galleon-deck</tt>"
        desc = (f"{plural(legacy_profiles, 'profile')} and {plural(legacy_themes, 'theme')} in {where}; it's left as it is"
                if not imported else f"Copied on {esc(imported)}; again replaces this copy (the old one is kept aside)")
        rows.append({"kind": "button", "key": "g-import", "title": "Import profiles and themes", "icon": "folder-download-symbolic",
                     "desc": desc, "label": "Import again" if imported else "Import",
                     **({"confirm": "Replace this copy?"} if nyx_profiles else {})})
    if env.galleon_deck_installed():
        enabled, _ = env.unit(gc.GALLEON_UNIT)
        if galleon_running or enabled:
            rows.append({"kind": "button", "key": "g-galleon-off", "title": "Turn galleon-deck off", "icon": "system-shutdown-symbolic",
                         "desc": "Stops its service so only one program drives the deck", "label": "Turn off"})
        else:
            rows.append({"kind": "button", "key": "g-galleon-on", "title": "Turn galleon-deck back on", "icon": "system-shutdown-symbolic",
                         "desc": "Turns Run the Stream Deck off, so galleon-deck drives the deck again", "label": "Turn on"})
        rows.append(env.job_row("g-galleon-remove", "Remove galleon-deck",
                                "Its program and service; your files and its fonts stay. Allows the deck for nyx-deck in the same step.",
                                "Remove", "galleon-remove", "Removing", "user-trash-symbolic"))
    return {"title": "Move from galleon-deck", "rows": rows}


def access_row(env):
    if env.access_ready():
        return {"kind": "info", "title": "Allow the deck and typing", "icon": "channel-secure-symbolic",
                "desc": "The deck and the virtual keyboard can be used by you", "value": "Allowed"}
    return env.job_row("g-access", "Allow the deck and typing",
                       f"Installs <tt>{UDEV_RULE}</tt> and loads <tt>uinput</tt>. Asks for your password once, in a terminal.",
                       "Allow…", "galleon-access", "Allowing", "channel-secure-symbolic")


# ------------------------------------------------------------------- the pane

def screen_lines(env, cfg, profile, page):
    clock, date = env.now(cfg.get("clock_format", "%H:%M"))
    lines = [f"{profile} · {page}" if cfg.get("screen", {}).get("profile", True) is not False else "", clock, date]
    playing = env.playing() if cfg.get("screen", {}).get("playing", True) is not False else ""
    if playing:
        lines.append(f"▶ {playing}")
    return lines


def deck_block(env, cfg, profile, prof, page, keys, sel):
    return {"kind": "deck", "key": "g-pick", "columns": gc.COLUMNS, "selected": sel, "dials": 2,
            "keys": [{"icon": key_icon(k), "label": key_label(k)} for k in keys],
            "screen": screen_lines(env, cfg, profile, page)}


def preview(env, cfg, live, foot):
    profile, prof, page = current(env)
    keys = page_keys(prof, page)
    sel = selected(env)
    shown = env.deck_state() or {}
    line = f"{profile} profile" + (" · on the deck now" if live and shown.get("profile") == profile else "")
    return {"title": f"Keys on “{page}”", "line": line, "blocks": [
        deck_block(env, cfg, profile, prof, page, keys, sel),
        {"kind": "rows", "title": f"Key {sel + 1} · {key_label(keys[sel]) or 'empty'}", "rows": key_editor(env, prof, page, keys[sel])},
    ], "foot": foot, "foot_icon": "emblem-ok-symbolic" if live else "dialog-information-symbolic"}


def legacy_preview(env):
    """Before the import: galleon-deck's start profile, to show what will come over."""
    cfg = gc.read_toml(env.legacy.config)
    names = gc.profile_names(env.legacy)
    if not names:
        return {"title": "Nothing to show yet", "blocks": [{"kind": "caption", "text": "galleon-deck has no profiles to import."}]}
    profile = cfg.get("start_profile") if cfg.get("start_profile") in names else names[0]
    prof = gc.read_toml(env.legacy.profile(profile))
    pages = page_names(prof)
    page = prof.get("start_page") if prof.get("start_page") in pages else (pages[0] if pages else "")
    keys = page_keys(prof, page)
    block = deck_block(env, cfg, profile, prof, page, keys, None)
    block.pop("key")
    return {"title": f"galleon-deck's “{profile}”", "line": "Import it to edit it here", "blocks": [block],
            "foot": "Importing copies the files; galleon-deck's own are left as they are.", "foot_icon": "dialog-information-symbolic"}


def slider(key, title, value, top, desc=""):
    return {"kind": "slider", "key": key, "title": title, "value": value, "min": 0, "max": top, "step": 5,
            "unit": " ms", "on_release": True, **({"desc": desc} if desc else {})}


def key_editor(env, prof, page, spec):
    action = gc.action_of(spec)
    value = spec.get(action, "")
    rows = [
        {"kind": "entry", "key": "g-key:label", "title": "Label", "value": str(spec.get("label", "")), "placeholder": "Text on the key"},
        {"kind": "entry", "key": "g-key:icon", "title": "Icon", "value": glyph_code(spec.get("icon")), "placeholder": "F059F",
         "desc": "A Nerd Font icon's code"},
        {"kind": "entry", "key": "g-key:image", "title": "Picture", "value": str(spec.get("image", "")),
         "placeholder": "icons/obs.png", "desc": "Instead of the icon; beside the config, or a full path"},
        {"kind": "colour", "key": "g-key:bg", "title": "Key colour", "compact": True,
         "value": gc.colour(spec.get("bg", "key_bg"), gc.profile_theme(env.nyx, prof))},
        {"kind": "colour", "key": "g-key:fg", "title": "Text colour", "compact": True,
         "value": gc.colour(spec.get("fg", "fg"), gc.profile_theme(env.nyx, prof))},
    ]
    if "bg" in spec or "fg" in spec:
        rows.append({"kind": "button", "key": "g-key:colours", "title": "The theme's colours", "label": "Reset"})
    rows.append({"kind": "choice", "key": "g-key:action", "title": "Does", "options": ACTIONS, "value": action, "refresh": True})
    if action == "key":
        rows.append({"kind": "entry", "key": "g-key:key", "title": "Key", "value": value, "placeholder": "ENTER, F5, KP7",
                     "desc": "Held while you hold the deck key"})
    elif action == "keys":
        rows.append({"kind": "entry", "key": "g-key:keys", "title": "Shortcut", "value": value, "placeholder": "SUPER+SHIFT+S"})
    elif action in ("sequence", "mouse"):
        if action == "sequence":
            rows.append({"kind": "entry", "key": "g-key:hold", "title": "Hold throughout", "value": spec.get("hold", ""),
                         "placeholder": "CTRL"})
        noted = "#" in str(value)
        rows.append({"kind": "entry", "key": f"g-key:{action}", "title": "Steps", "value": " ".join(gc.step_words(value)),
                     "placeholder": "S S W D" if action == "sequence" else "CLICK WAIT:200 MOVE:100,-50",
                     "desc": "Its notes go when you change the steps" if noted else
                     ("Keys, combos (CTRL+V), CLICK, MOVE:x,y, SCROLL:n or WAIT:ms" if action == "sequence" else
                      "CLICK, RIGHTCLICK, MOVE:x,y, SCROLL:n, WAIT:ms")})
        rows.append(slider("g-key:step_ms", "Between steps", spec.get("step_ms", 40), 500))
        rows.append({"kind": "switch", "key": "g-key:loop", "title": "Repeat until pressed again", "value": bool(spec.get("loop")),
                     "rows": [slider("g-key:repeat_ms", "Between runs", spec.get("repeat_ms", 100), 2000)]})
    elif action == "repeat":
        rows.append({"kind": "entry", "key": "g-key:repeat", "title": "Key or shortcut", "value": value, "placeholder": "SPACE",
                     "desc": "Tapped until you press the deck key again"})
        rows.append(slider("g-key:repeat_ms", "Between presses", spec.get("repeat_ms", 100), 2000))
    elif action == "exec":
        rows.append({"kind": "entry", "key": "g-key:exec", "title": "Command", "value": value, "placeholder": "firefox"})
        rows.append({"kind": "switch", "key": "g-key:confirm", "title": "Show whether it worked",
                     "desc": "A ✓ or ✗ on the key when it finishes", "value": bool(spec.get("confirm"))})
    elif action == "media":
        rows.append({"kind": "choice", "key": "g-key:media", "title": "Media", "options": MEDIA, "value": value})
    elif action == "page":
        rows.append({"kind": "choice", "key": "g-key:page", "title": "Page", "value": value,
                     "options": [["back", "Back"]] + [[p, p] for p in page_names(prof) if p != page]})
    elif action == "profile":
        rows.append({"kind": "choice", "key": "g-key:profile", "title": "Profile", "value": value,
                     "options": [[p, p] for p in gc.profile_names(env.nyx)]})
    rows.append({"kind": "entry", "key": "g-key:note", "title": "Note", "value": str(spec.get("note", "")),
                 "placeholder": "Only for you; not on the deck"})
    rows.append({"kind": "choice", "key": "g-key:move", "title": "Swap with", "value": "",
                 "options": [["", "Another key…"]] + [[str(i), f"Key {i + 1}"] for i in range(gc.KEYS)], "refresh": True})
    return rows


# ---------------------------------------------------------------- starting

STARTER_CONFIG = """# The Galleon 100 SD's deck, run by nyx-deck. galleon-deck's format: its
# profiles, themes and add-on profiles work here as they are.
start_profile = "desktop"
brightness = 70
volume_step = 2
clock_format = "%H:%M"
"""
STARTER_PROFILE = {
    "theme": "amber", "start_page": "numpad",
    "page": [
        {"name": "numpad", "keys": [
            {"label": "7", "key": "KP7"}, {"label": "8", "key": "KP8"}, {"label": "9", "key": "KP9"},
            {"label": "4", "key": "KP4"}, {"label": "5", "key": "KP5"}, {"label": "6", "key": "KP6"},
            {"label": "1", "key": "KP1"}, {"label": "2", "key": "KP2"}, {"label": "3", "key": "KP3"},
            {"label": "0", "key": "KP0"}, {"label": ".", "key": "KPDOT"},
            {"icon": "\U000F0311", "key": "KPENTER", "fg": "accent"}]},
        {"name": "apps", "keys": [
            {"icon": "\U000F018D", "label": "Terminal", "exec": "xdg-terminal-exec"},
            {"icon": "\U000F024B", "label": "Files", "exec": "nautilus --new-window"},
            {"icon": "\U000F059F", "label": "Browser", "exec": "omarchy-launch-browser"},
            {"icon": "\U000F040E", "label": "Play", "media": "play-pause"},
            {"icon": "\U000F04AD", "label": "Next", "media": "next"},
            {"icon": "\U000F0100", "label": "Shot", "exec": "omarchy-cmd-screenshot"},
            {}, {}, {}, {}, {},
            {"icon": "\U000F004D", "label": "back", "page": "back", "fg": "accent"}]},
    ],
}


def write_starter(env):
    os.makedirs(env.nyx.profiles, exist_ok=True)
    if not os.path.exists(env.nyx.config):
        gc.write_atomic(env.nyx.config, STARTER_CONFIG)
    gc.write_profile(env.nyx.profile("desktop"), STARTER_PROFILE)


# --------------------------------------------------------------------- set

def edit_profile(env, name, change):
    path = env.nyx.profile(name)
    prof = gc.read_toml(path)
    change(prof)
    gc.write_profile(path, prof)


def find_page(prof, name):
    for p in prof.get("page", []):
        if p.get("name") == name:
            return p
    raise SystemExit(f"No page called {name}")


def check_name(name, taken, what):
    name = name.strip()
    if not name:
        return None
    if "/" in name or name.startswith(".") or name == "settings":
        raise SystemExit(f"“{name}” can't be a {what} name")
    if name in taken:
        raise SystemExit(f"There's already a {what} called “{name}”")
    return name


def set_cfg(env, table, key, value, remove=False):
    if not os.path.exists(env.nyx.config):
        gc.write_atomic(env.nyx.config, STARTER_CONFIG)
    gc.set_value(env.nyx.config, table, key, value, remove=remove)


def set_value(env, key, value):
    if key == "g-run":
        return set_run(env, value == "true")
    if key == "g-needs":
        if not shutil.which("pacman") and not env.pretend:
            raise SystemExit("Install these yourself: " + " ".join(NEEDS))
        env.terminal("galleon-needs", "sudo pacman -S --needed " + " ".join(NEEDS))
        return {"toast": "Installing in a terminal: type your password there", "refresh": True}
    if key == "g-access":
        env.terminal("galleon-access", f"sudo sh -c {shlex.quote(access_script())}")
        return {"toast": "Allowing access in a terminal: type your password there", "refresh": True}
    if key == "g-import":
        return import_legacy(env)
    if key == "g-galleon-off":
        env.galleon_deck_off()
        return {"toast": "galleon-deck is off" + ("" if gc.galleon_enabled(env.nyx_config) else ": turn on Run the Stream Deck"),
                "refresh": True}
    if key == "g-galleon-on":
        set_run(env, False)
        env.galleon_deck_on()
        return {"toast": "galleon-deck drives the deck again", "refresh": True}
    if key == "g-galleon-remove":
        env.terminal("galleon-remove", remove_script())
        return {"toast": "Removing galleon-deck in a terminal: type your password there", "refresh": True}
    if key == "g-brightness":
        set_cfg(env, None, "brightness", int(round(float(value))))
        return None
    if key == "g-volume-step":
        set_cfg(env, None, "volume_step", int(round(float(value))))
        return None
    if key == "g-clock":
        set_cfg(env, None, "clock_format", value or "%H:%M")
        return None
    if key in ("g-screen-profile", "g-screen-playing"):
        set_cfg(env, "screen", key.split("-")[-1], value == "true")
        return None
    if key == "g-screen-volume":
        set_cfg(env, "screen", "volume", value)
        return None
    if key.startswith("g-dial:"):
        import galleon
        field = key.split(":", 1)[1]
        if field not in galleon.DIAL_DEFAULTS:
            raise SystemExit(f"unknown setting {key}")
        set_cfg(env, "dials", field, value)
        return None
    if key == "g-transition":
        set_cfg(env, "transition", "enabled", value == "true")
        return None
    if key == "g-boot":
        set_cfg(env, "boot", "enabled", value != "off")
        if value != "gif":
            set_cfg(env, "boot", "gif", "", remove=True)
        return {"refresh": True}
    if key == "g-boot-gif":
        set_cfg(env, "boot", "gif", value, remove=not value)
        return None
    if key == "g-auto":
        set_cfg(env, "auto_switch", "enabled", value == "true")
        return {"refresh": True}
    if key == "g-auto-fallback":
        set_cfg(env, "auto_switch", "fallback", value, remove=not value)
        return None
    if key.startswith("g-rule:") or key == "g-rule-add":
        return set_rule(env, key, value)
    if key == "g-start-profile":
        set_cfg(env, None, "start_profile", value)
        return None
    if key == "g-new-profile":
        name = check_name(value, gc.profile_names(env.nyx), "profile")
        if not name:
            return None
        gc.write_profile(env.nyx.profile(name), {"theme": gc.DEFAULT_THEME, "page": [{"name": "Page 1", "keys": []}]})
        remember(env, "profile", name)
        remember(env, "page", "")
        return {"refresh": True}

    profile, prof, page = current(env)
    if not profile:
        raise SystemExit("There's no profile to change yet")
    if key == "g-profile":
        remember(env, "profile", value)
        remember(env, "page", "")
        if gc.galleon_enabled(env.nyx_config):
            env.request(value)
        return {"refresh": True}
    if key == "g-page":
        remember(env, "page", value)
        if gc.galleon_enabled(env.nyx_config):
            env.request(profile, value)
        return {"refresh": True}
    if key == "g-pick":
        index = int(value)
        if not 0 <= index < gc.KEYS:
            raise SystemExit("no such key")
        remember(env, "key", index)
        return {"refresh": True}
    if key == "g-add-page":
        name = check_name(value, page_names(prof), "page")
        if not name:
            return None
        edit_profile(env, profile, lambda d: d.setdefault("page", []).append({"name": name, "keys": []}))
        remember(env, "page", name)
        return {"refresh": True}
    if key == "g-rename-profile":
        return rename_profile(env, profile, value)
    if key == "g-rename-page":
        return rename_page(env, profile, page, value)
    if key == "g-start-page":
        edit_profile(env, profile, lambda d: d.__setitem__("start_page", value))
        return None
    if key == "g-settings-page":
        def change(d):
            if value == "true":
                d.pop("settings_page", None)
            else:
                d["settings_page"] = False
        edit_profile(env, profile, change)
        return {"refresh": True}
    if key == "g-move-page":
        def move(d):
            pages = d.get("page", [])
            i = page_names(d).index(page)
            j = i - 1 if value == "earlier" else i + 1
            if 0 <= j < len(pages):
                pages[i], pages[j] = pages[j], pages[i]
        edit_profile(env, profile, move)
        return {"refresh": True}
    if key == "g-remove-page":
        if len(page_names(prof)) < 2:
            raise SystemExit("A profile needs at least one page")
        edit_profile(env, profile, lambda d: d.__setitem__("page", [p for p in d["page"] if p.get("name") != page]))
        if prof.get("start_page") == page:
            edit_profile(env, profile, lambda d: d.pop("start_page", None))
        remember(env, "page", "")
        return {"refresh": True}
    if key == "g-remove-profile":
        return remove_profile(env, profile)
    if key == "g-theme":
        edit_profile(env, profile, lambda d: d.__setitem__("theme", value))
        if value == gc.WALLPAPER and not os.path.exists(os.path.join(env.nyx.themes, "wallpaper.toml")):
            env.wallpaper_theme()
        return {"refresh": True}
    if key == "g-wallpaper-theme":
        env.wallpaper_theme()
        edit_profile(env, profile, lambda d: d.__setitem__("theme", gc.WALLPAPER))
        return {"toast": "Made a theme from the wallpaper", "refresh": True}
    if key == "g-key-style":
        def shape(d):
            over = d.setdefault("theme_overrides", {})
            base = gc.all_themes(env.nyx).get(d.get("theme", gc.DEFAULT_THEME), {})
            if value == base.get("key_style", "rounded"):
                over.pop("key_style", None)
            else:
                over["key_style"] = value
            if not over:
                d.pop("theme_overrides")
        edit_profile(env, profile, shape)
        return {"refresh": True}
    if key.startswith("g-colour:"):
        field = key.split(":", 1)[1]
        if field not in dict(THEME_COLOURS) or not re.fullmatch(r"#[0-9a-fA-F]{6}", value):
            raise SystemExit(f"unknown setting {key}")
        edit_profile(env, profile, lambda d: d.setdefault("theme_overrides", {}).__setitem__(field, value.lower()))
        return {"refresh": True}
    if key == "g-colour-reset":
        def reset(d):
            over = d.get("theme_overrides", {})
            for field, _ in THEME_COLOURS:
                over.pop(field, None)
            if not over:
                d.pop("theme_overrides", None)
        edit_profile(env, profile, reset)
        return {"refresh": True}
    if key.startswith("g-key:"):
        return set_key(env, profile, prof, page, key.split(":", 1)[1], value)
    raise SystemExit(f"unknown setting {key}")


def set_run(env, on):
    gc.set_value(env.nyx_config, None, "galleon", on)
    if on:
        env.run_nyx_deck()
        if env.galleon_deck_running():
            return {"toast": "On: nyx-deck takes the deck once galleon-deck is off", "refresh": True}
        return {"toast": "nyx-deck runs the deck now", "refresh": True}
    return {"toast": "Off: the keys type numbers again", "refresh": True}


def set_rule(env, key, value):
    if key == "g-rule-add":
        if not value:
            return None
        profile, _, _ = current(env)
        title = dict(env.windows()).get(value, "")
        rule = {"class": f"^{re.escape(value)}$", "profile": profile}
        if title:
            rule["name"] = title if len(title) <= 40 else title[:39] + "…"
        rules = config(env).get("auto_switch", {}).get("rules", []) + [rule]
        set_cfg(env, "auto_switch", "rules", rules)
        return {"refresh": True}
    n = int(key.split(":", 1)[1])
    rules = config(env).get("auto_switch", {}).get("rules", [])
    if not 0 <= n < len(rules):
        raise SystemExit("no such rule")
    if value == "-":
        del rules[n]
    elif value in gc.profile_names(env.nyx):
        rules[n]["profile"] = value
    else:
        raise SystemExit(f"No profile called {value}")
    set_cfg(env, "auto_switch", "rules", rules, remove=not rules)
    return {"refresh": True}


def rename_profile(env, old, new):
    new = check_name(new, gc.profile_names(env.nyx), "profile")
    if not new or new == old:
        return None
    os.rename(env.nyx.profile(old), env.nyx.profile(new))
    retarget_profile(env, old, new)
    remember(env, "profile", new)
    return {"refresh": True}


def retarget_profile(env, old, new):
    """Point the start profile, switching rules and profile keys at `new` (None: drop them)."""
    cfg = config(env)
    names = gc.profile_names(env.nyx)
    if cfg.get("start_profile") == old:
        set_cfg(env, None, "start_profile", new or (names[0] if names else ""))
    auto = cfg.get("auto_switch", {})
    if auto.get("fallback") == old:
        set_cfg(env, "auto_switch", "fallback", new or "", remove=not new)
    rules = auto.get("rules", [])
    if any(r.get("profile") == old for r in rules):
        rules = [{**r, "profile": new} if r.get("profile") == old else r for r in rules if new or r.get("profile") != old]
        set_cfg(env, "auto_switch", "rules", rules, remove=not rules)
    for name in names:
        def change(d):
            for p in d.get("page", []):
                for k in p.get("keys", []):
                    if k.get("profile") == old:
                        if new:
                            k["profile"] = new
                        else:
                            del k["profile"]
        prof = profile_doc(env, name)
        if any(k.get("profile") == old for p in prof.get("page", []) for k in p.get("keys", [])):
            edit_profile(env, name, change)


def rename_page(env, profile, old, new):
    prof = profile_doc(env, profile)
    new = check_name(new, page_names(prof), "page")
    if not new or new == old:
        return None

    def change(d):
        find_page(d, old)["name"] = new
        if d.get("start_page") == old:
            d["start_page"] = new
        for p in d.get("page", []):
            for k in p.get("keys", []):
                if k.get("page") == old:
                    k["page"] = new
    edit_profile(env, profile, change)
    remember(env, "page", new)
    return {"refresh": True}


def remove_profile(env, profile):
    if len(gc.profile_names(env.nyx)) < 2:
        raise SystemExit("The deck needs at least one profile")
    os.remove(env.nyx.profile(profile))
    retarget_profile(env, profile, None)
    remember(env, "profile", "")
    remember(env, "page", "")
    return {"refresh": True}


def set_key(env, profile, prof, page, field, value):
    index = selected(env)
    if field == "move":
        if not value:
            return None
        other = int(value)
        if not 0 <= other < gc.KEYS:
            raise SystemExit("no such key")

        def swap(d):
            p = find_page(d, page)
            keys = (list(p.get("keys", [])) + [{}] * gc.KEYS)[:gc.KEYS]
            keys[index], keys[other] = keys[other], keys[index]
            p["keys"] = keys
        edit_profile(env, profile, swap)
        remember(env, "key", other)
        return {"refresh": True}
    action = gc.action_of(page_keys(prof, page)[index])
    if field in ("key", "keys", "repeat") and not value.strip():
        raise SystemExit("Type a key name, like ENTER, F5 or CTRL+C")
    try:
        if field in ("key", "keys", "hold", "repeat") and value:
            gc.codes(value)
        elif field == "sequence":
            gc.steps(value)
        elif field == "mouse":
            gc.mouse_steps(value)
    except ValueError as e:
        raise SystemExit(f"{e}. Key names are Linux input names: ENTER, F5, KP7, CTRL+C") from None

    def change(d):
        p = find_page(d, page)
        keys = (list(p.get("keys", [])) + [{}] * gc.KEYS)[:gc.KEYS]
        spec = dict(keys[index])
        if field == "action":
            for f in gc.ACTION_FIELDS + ACTION_EXTRAS:
                spec.pop(f, None)
            others = [n for n in gc.profile_names(env.nyx) if n != profile]
            defaults = {"key": "ENTER", "keys": "CTRL+C", "sequence": "", "mouse": "CLICK", "repeat": "SPACE", "exec": "",
                        "media": "play-pause", "page": "back", "profile": (others or [profile])[0]}
            if value in defaults:
                spec[value] = defaults[value]
        elif field in ("label", "note", "exec", "image"):
            if value:
                spec[field] = value
            else:
                spec.pop(field, None)
        elif field == "hold":
            if value.strip():
                spec["hold"] = value.replace(" ", "").upper()
            else:
                spec.pop("hold", None)
        elif field == "icon":
            if value.strip():
                spec["icon"] = parse_glyph(value)
            else:
                spec.pop("icon", None)
        elif field in ("key", "keys", "repeat"):
            spec[field] = value.replace(" ", "").upper()
        elif field in ("sequence", "mouse"):
            spec[field] = " ".join(w.upper() for w in gc.step_words(value))
        elif field in ("media", "page", "profile"):
            spec[field] = value
        elif field in ("step_ms", "repeat_ms"):
            spec[field] = int(round(float(value)))
        elif field in ("confirm", "loop"):
            if value == "true":
                spec[field] = True
            else:
                spec.pop(field, None)
        elif field in ("bg", "fg"):
            if not re.fullmatch(r"#[0-9a-fA-F]{6}", value):
                raise SystemExit("A colour is #rrggbb")
            spec[field] = value.lower()
        elif field == "colours":
            spec.pop("bg", None)
            spec.pop("fg", None)
        else:
            raise SystemExit(f"unknown setting g-key:{field}")
        keys[index] = spec
        p["keys"] = keys
    if field not in ("action", "label", "note", "icon", "image", "bg", "fg", "colours", "confirm", "loop") and action == "none":
        raise SystemExit("Pick what the key does first")
    edit_profile(env, profile, change)
    return {"refresh": True} if field in ("action", "label", "icon", "image", "colours", "loop", "bg", "fg") else None


def import_legacy(env):
    """Copy galleon-deck's folder over nyx-deck's copy; galleon-deck's stays as it is."""
    if not os.path.isdir(env.legacy.root):
        raise SystemExit("There's no ~/.config/galleon-deck to import")
    try:
        gc.load_config(env.legacy)
    except ValueError as e:
        raise SystemExit(f"galleon-deck's files have a problem, so nothing was copied: {e}") from None
    aside = ""
    if os.path.exists(env.nyx.root):
        aside = env.nyx.root + time.strftime(".before-import-%Y%m%d-%H%M%S")
        os.rename(env.nyx.root, aside)
    shutil.copytree(env.legacy.root, env.nyx.root, symlinks=True)
    profiles, themes = legacy_counts(env)
    remember(env, "imported", time.strftime("%-d %b %Y, %H:%M"))
    remember(env, "profile", "")
    remember(env, "page", "")
    toast = f"Imported {plural(profiles, 'profile')} and {plural(themes, 'theme')}"
    if aside:
        toast += f"; the copy before is in {os.path.basename(aside)}"
    return {"toast": toast, "refresh": True}


def access_script():
    return (f"printf %s {shlex.quote(RULE_TEXT)} > {UDEV_RULE} && echo uinput > {MODULES_LOAD} && modprobe uinput && "
            "udevadm control --reload && udevadm trigger --subsystem-match=hidraw --subsystem-match=misc")


def remove_script():
    """galleon-deck's uninstall.sh without --purge, then nyx-deck's device access."""
    data = os.environ.get("XDG_DATA_HOME") or os.path.join(HOME, ".local/share")
    conf = gc.config_home()
    pid = os.path.join(gc.runtime_dir(), "galleon-deck.pid")
    user = " && ".join([
        f"(systemctl --user disable --now {gc.GALLEON_UNIT} 2>/dev/null || true)",
        f"rm -f {shlex.quote(os.path.join(conf, 'systemd/user', gc.GALLEON_UNIT))}",
        "systemctl --user daemon-reload",
        f"([ -f {shlex.quote(pid)} ] && kill \"$(cat {shlex.quote(pid)})\" 2>/dev/null || true)",
        "rm -f " + " ".join(shlex.quote(os.path.join(HOME, ".local/bin", b)) for b in ("galleon-deck", "galleon-deck-config", "galleon-addon")),
        f"rm -rf {shlex.quote(os.path.join(data, 'galleon-deck'))}",
        f"rm -f {shlex.quote(os.path.join(data, 'applications/io.github.galleondeck.Config.desktop'))} "
        f"{shlex.quote(os.path.join(conf, 'autostart/galleon-deck-autostart.desktop'))}",
    ])
    root = f"rm -f {GALLEON_RULE} {GALLEON_MODULES} && {access_script()}"
    return f"{user} && sudo sh -c {shlex.quote(root)}"
