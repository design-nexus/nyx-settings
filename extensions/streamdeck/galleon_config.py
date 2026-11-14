"""The Corsair Galleon 100 SD deck's config, shared by nyx-deck (which runs the deck)
and the Stream Deck page (which edits it). It is galleon-deck's format, so a
~/.config/galleon-deck folder imports as it is:

    config.toml          brightness, start_profile, volume_step, clock_format,
                         [auto_switch], [transition], [boot], [wallpaper_theme],
                         and nyx-deck's own [screen] and [dials] (galleon-deck
                         ignores them)
    profiles/<name>.toml theme, start_page, logo, settings_page, auto_switch,
                         [theme_overrides], [[page]] name + 12 keys
    themes/<name>.toml   colours over a built-in theme

Needs only the standard library: Settings uses it without Pillow or evdev.

The themes, key names, step syntax, config checks and the TOML writer are ported
from galleon-deck (https://github.com/NLMP-DDHS/galleon-deck):

    MIT License

    Copyright (c) 2026 The Galleon-Deck contributors

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all
    copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
    AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
    OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
    SOFTWARE.
"""

import glob
import json
import os
import re
import subprocess

try:
    import tomllib
except ImportError:  # Python 3.10
    import tomli as tomllib  # type: ignore

KEYS = 12
COLUMNS = 3
HOME = os.path.expanduser("~")


def config_home():
    return os.environ.get("XDG_CONFIG_HOME") or os.path.join(HOME, ".config")


def runtime_dir():
    return os.environ.get("XDG_RUNTIME_DIR") or "/tmp"


class Paths:
    """A galleon-deck-shaped config folder."""

    def __init__(self, root):
        self.root = root
        self.config = os.path.join(root, "config.toml")
        self.profiles = os.path.join(root, "profiles")
        self.themes = os.path.join(root, "themes")

    def profile(self, name):
        return os.path.join(self.profiles, name + ".toml")

    def resolve(self, path):
        """A path from the config: absolute, ~, or relative to the config folder."""
        path = os.path.expanduser(str(path))
        return path if os.path.isabs(path) else os.path.join(self.root, path)


def nyx_paths():
    """nyx-deck's copy, which it runs the Galleon from."""
    return Paths(os.path.join(config_home(), "nyx-deck", "galleon"))


def legacy_paths():
    """galleon-deck's own folder: only read (to import it), never written."""
    return Paths(os.path.join(config_home(), "galleon-deck"))


def nyx_config():
    """nyx-deck's config.toml; `galleon = true` there lets nyx-deck drive the Galleon."""
    return os.path.join(config_home(), "nyx-deck", "config.toml")


def read_toml(path):
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError):
        return {}


def galleon_enabled(path=None):
    return read_toml(path or nyx_config()).get("galleon", False) is True


# ---------------------------------------------------------------- themes

# key_style "hud" draws cut corners with ticks in the `tick` colour; there,
# `radius` is the size of the cut.
THEMES = {
    "amber": {
        "bg": "#1f1204", "key_bg": "#251505", "fg": "#f04a14", "accent": "#e0a93f",
        "dim": "#7f6a63", "border": "#5a2a0c", "frame": "#e0231a", "radius": 6, "border_width": 3,
        "font": "Quantico", "clock_font": "Quantico:bold",
    },
    "cyberpunk": {
        "bg": "#08040f", "key_bg": "#0d0718", "fg": "#00f0ff", "accent": "#ff00aa",
        "dim": "#6e6a86", "border": "#2a1f45", "radius": 18, "border_width": 3,
        "font": "JetBrains Mono", "clock_font": "JetBrains Mono:bold",
    },
    "synthwave": {
        "bg": "#12041f", "key_bg": "#1c0833", "fg": "#ff9e3d", "accent": "#ff2a6d",
        "dim": "#8a5fa8", "border": "#4a1d6e", "radius": 24, "border_width": 3,
        "font": "Orbitron", "clock_font": "Orbitron:bold",
    },
    "matrix": {
        "bg": "#000000", "key_bg": "#020a03", "fg": "#00ff41", "accent": "#b6ff5c",
        "dim": "#0f6b2a", "border": "#0c3d18", "radius": 4, "border_width": 2,
        "font": "FiraCode Nerd Font Mono", "clock_font": "FiraCode Nerd Font Mono:bold",
    },
    "arctic": {
        "bg": "#0b1018", "key_bg": "#121a26", "fg": "#d8e6f3", "accent": "#88c0d0",
        "dim": "#5b6b80", "border": "#26344a", "radius": 22, "border_width": 2,
        "font": "Rajdhani:semibold", "clock_font": "Rajdhani:bold",
    },
    "hud": {
        "bg": "#050505", "key_bg": "#0d0d0d", "fg": "#ff8c1a", "accent": "#19e6d4",
        "dim": "#6b6b6b", "border": "#333333", "radius": 2, "border_width": 2,
        "font": "Quantico", "clock_font": "Quantico:bold",
    },
    "mono": {
        "bg": "#000000", "key_bg": "#0a0a0a", "fg": "#f2f2f2", "accent": "#ffffff",
        "dim": "#777777", "border": "#2a2a2a", "radius": 20, "border_width": 2,
        "font": "Roboto", "clock_font": "Roboto:light",
    },
}
DEFAULT_THEME = "amber"
THEME_DEFAULTS = {**THEMES[DEFAULT_THEME], "icon_font": "Symbols Nerd Font", "key_style": "rounded", "tick": "accent"}
WALLPAPER = "wallpaper"


def all_themes(paths):
    themes = {name: {**THEME_DEFAULTS, **t} for name, t in THEMES.items()}
    for path in sorted(glob.glob(os.path.join(paths.themes, "*.toml"))):
        name = os.path.splitext(os.path.basename(path))[0]
        own = read_toml(path)
        if own:
            themes[name] = {**themes.get(name, THEME_DEFAULTS), **own}
    return themes


def theme_names(paths):
    """Built-in themes in their order, then your own, then the wallpaper one."""
    own = sorted(os.path.splitext(os.path.basename(p))[0] for p in glob.glob(os.path.join(paths.themes, "*.toml")))
    return list(THEMES) + [t for t in own if t not in THEMES and t != WALLPAPER] + [WALLPAPER]


def profile_theme(paths, prof):
    themes = all_themes(paths)
    name = prof.get("theme", DEFAULT_THEME)
    return {**themes.get(name, THEME_DEFAULTS), **prof.get("theme_overrides", {})}


def colour(value, theme):
    """A "#rrggbb" value, or the name of a theme colour ("accent", "dim", ...)."""
    return theme.get(value, value) if isinstance(value, str) and not value.startswith("#") else value


# ------------------------------------------------------------- key names

# Linux input names (without KEY_), used when neither python-evdev nor the
# kernel's input-event-codes.h is there to give the full list.
_BUILT_IN_KEYS = (
    "A=30 B=48 C=46 D=32 E=18 F=33 G=34 H=35 I=23 J=36 K=37 L=38 M=50 N=49 O=24 P=25 Q=16 R=19 S=31 T=20 "
    "U=22 V=47 W=17 X=45 Y=21 Z=44 0=11 1=2 2=3 3=4 4=5 5=6 6=7 7=8 8=9 9=10 F1=59 F2=60 F3=61 F4=62 F5=63 "
    "F6=64 F7=65 F8=66 F9=67 F10=68 F11=87 F12=88 F13=183 F14=184 F15=185 F16=186 F17=187 F18=188 F19=189 "
    "F20=190 F21=191 F22=192 F23=193 F24=194 LEFTCTRL=29 LEFTSHIFT=42 LEFTALT=56 LEFTMETA=125 RIGHTCTRL=97 "
    "RIGHTSHIFT=54 RIGHTALT=100 RIGHTMETA=126 ENTER=28 SPACE=57 TAB=15 BACKSPACE=14 ESC=1 DELETE=111 "
    "INSERT=110 UP=103 DOWN=108 LEFT=105 RIGHT=106 HOME=102 END=107 PAGEUP=104 PAGEDOWN=109 MINUS=12 "
    "EQUAL=13 LEFTBRACE=26 RIGHTBRACE=27 SEMICOLON=39 APOSTROPHE=40 GRAVE=41 BACKSLASH=43 COMMA=51 DOT=52 "
    "SLASH=53 102ND=86 KP0=82 KP1=79 KP2=80 KP3=81 KP4=75 KP5=76 KP6=77 KP7=71 KP8=72 KP9=73 KPDOT=83 "
    "KPENTER=96 KPPLUS=78 KPMINUS=74 KPASTERISK=55 KPSLASH=98 KPEQUAL=117 NUMLOCK=69 CAPSLOCK=58 "
    "SCROLLLOCK=70 SYSRQ=99 PAUSE=119 COMPOSE=127 MENU=139 PRINT=210 VOLUMEUP=115 VOLUMEDOWN=114 MUTE=113 "
    "MICMUTE=248 PLAYPAUSE=164 NEXTSONG=163 PREVIOUSSONG=165 STOPCD=166 BRIGHTNESSUP=225 BRIGHTNESSDOWN=224"
)
HEADER = "/usr/include/linux/input-event-codes.h"
_key_codes = None


def key_codes():
    """{"KEY_A": 30, ...}: every name the kernel knows, or the built-in list."""
    global _key_codes
    if _key_codes is None:
        codes = {}
        try:
            from evdev import ecodes
            codes = {k: v for k, v in ecodes.ecodes.items() if k.startswith("KEY_") and isinstance(v, int)}
        except ImportError:
            try:
                with open(HEADER) as f:
                    for m in re.finditer(r"#define\s+(KEY_\w+)\s+(0x[0-9a-fA-F]+|\d+)\b", f.read()):
                        codes.setdefault(m.group(1), int(m.group(2), 0))
            except OSError:
                pass
        if not codes:
            codes = {"KEY_" + k: int(v) for k, v in (p.split("=") for p in _BUILT_IN_KEYS.split())}
        _key_codes = codes
    return _key_codes


ALIASES = {"CTRL": "LEFTCTRL", "SHIFT": "LEFTSHIFT", "ALT": "LEFTALT", "SUPER": "LEFTMETA", "WIN": "LEFTMETA",
           "META": "LEFTMETA"}
BTN_LEFT, BTN_RIGHT, BTN_MIDDLE, BTN_SIDE, BTN_EXTRA = 272, 273, 274, 275, 276
MOUSE_CLICKS = {"CLICK": (BTN_LEFT, 1), "LEFTCLICK": (BTN_LEFT, 1), "DOUBLECLICK": (BTN_LEFT, 2),
                "RIGHTCLICK": (BTN_RIGHT, 1), "MIDDLECLICK": (BTN_MIDDLE, 1), "BACKCLICK": (BTN_SIDE, 1),
                "FORWARDCLICK": (BTN_EXTRA, 1)}
MAX_WAIT_MS = 60000
MAX_MOVE = 10000
MAX_SCROLL = 100


def codes(combo):
    """"CTRL+SHIFT+T" -> key codes, held in that order."""
    table = key_codes()
    out = []
    for name in str(combo).replace(" ", "").upper().split("+"):
        name = ALIASES.get(name, name)
        code = table.get("KEY_" + name)
        if code is None:
            raise ValueError(f"unknown key {name!r}")
        out.append(code)
    return out


def step(item):
    """One sequence step: a list of key codes, or ("click", button, count),
    ("move", x, y), ("scroll", notches) or ("wait", ms)."""
    name = str(item).replace(" ", "").upper()
    if name in MOUSE_CLICKS:
        return ("click", *MOUSE_CLICKS[name])
    m = re.fullmatch(r"(MOVE|SCROLL|WAIT):(.*)", name)
    if not m:
        return codes(item)
    kind, args = m.group(1).lower(), m.group(2)
    pattern, limit = {"move": (r"-?\d+,-?\d+", MAX_MOVE), "scroll": (r"-?\d+", MAX_SCROLL),
                      "wait": (r"\d+", MAX_WAIT_MS)}[kind]
    if not re.fullmatch(pattern, args):
        raise ValueError(f"bad {kind} step {str(item)!r}")
    values = [int(v) for v in args.split(",")]
    if any(abs(v) > limit for v in values):
        raise ValueError(f"{str(item)!r} is over the {kind} limit of {limit}")
    return (kind, *values)


def step_words(sequence):
    """"W S D" (or a list) -> the step words; a string can span lines, and
    anything after a # on a line is a note."""
    if isinstance(sequence, str):
        return [w for line in sequence.splitlines() for w in line.split("#", 1)[0].split()]
    return [str(w) for w in sequence]


def steps(sequence):
    return [step(w) for w in step_words(sequence)]


def mouse_steps(sequence):
    out = steps(sequence)
    if any(not isinstance(s, tuple) for s in out):
        raise ValueError("mouse actions take CLICK, MOVE:x,y, SCROLL:n and WAIT:ms, not keys")
    return out


# --------------------------------------------------------------- loading

# What a key does, in the order a key is checked.
ACTION_FIELDS = ("key", "keys", "sequence", "mouse", "repeat", "exec", "media", "page", "profile")


def action_of(spec):
    return next((f for f in ACTION_FIELDS if f in spec), "none")


def validate_pages(pages, where):
    if not pages:
        raise ValueError(f"{where}: no [[page]] entries")
    for p in pages:
        if "name" not in p:
            raise ValueError(f"{where}: a [[page]] has no name")
        keys = p.get("keys", [])
        if len(keys) > KEYS:
            raise ValueError(f"{where}: page {p['name']!r} has {len(keys)} keys; the deck has {KEYS}")
        p["keys"] = keys + [{}] * (KEYS - len(keys))
        for k in p["keys"]:
            for field in ("key", "keys", "hold", "repeat"):
                if field in k:
                    codes(k[field])
            if "sequence" in k:
                steps(k["sequence"])
            if "mouse" in k:
                mouse_steps(k["mouse"])


def profile_names(paths):
    return sorted(os.path.splitext(os.path.basename(p))[0] for p in glob.glob(os.path.join(paths.profiles, "*.toml")))


def load_config(paths):
    """The whole config, checked: raises ValueError with what's wrong and where."""
    try:
        with open(paths.config, "rb") as f:
            cfg = tomllib.load(f)
    except FileNotFoundError:
        cfg = {}
    except tomllib.TOMLDecodeError as e:
        raise ValueError(f"config.toml: {e}") from None
    themes = all_themes(paths)
    profiles = {}
    for name in profile_names(paths):
        try:
            with open(paths.profile(name), "rb") as f:
                prof = tomllib.load(f)
        except tomllib.TOMLDecodeError as e:
            raise ValueError(f"profiles/{name}.toml: {e}") from None
        validate_pages(prof.get("page", []), f"profiles/{name}.toml")
        for rule in prof.get("auto_switch", []):
            try:
                re.compile(rule.get("class", ""))
                re.compile(rule.get("title", ""))
            except (re.error, AttributeError) as e:
                raise ValueError(f"profiles/{name}.toml: auto_switch rule {rule!r}: {e}") from None
        theme_name = prof.get("theme", DEFAULT_THEME)
        if theme_name not in themes:
            raise ValueError(f"profiles/{name}.toml: unknown theme {theme_name!r} (have: {', '.join(themes)})")
        if prof.get("settings_page", True):
            prof["page"].append({"name": "settings", "keys": [{}] * KEYS, "settings": True})
        prof.update(name=name, path=paths.profile(name), theme_name=theme_name,
                    theme={**themes[theme_name], **prof.get("theme_overrides", {})})
        profiles[name] = prof
    if not profiles:
        raise ValueError("no profiles in profiles/*.toml")
    for rule in cfg.get("auto_switch", {}).get("rules", []):
        if rule.get("profile") not in profiles:
            raise ValueError(f"auto_switch rule points at unknown profile {rule.get('profile')!r}")
        re.compile(rule.get("class", ""))
        re.compile(rule.get("title", ""))
    cfg.update(profiles=profiles, themes=themes, switch_rules=switch_rules(cfg, profiles))
    return cfg


def switch_rules(cfg, profiles):
    """Each profile's own rules first (an add-on's profile brings its game's
    rule), then config.toml's."""
    rules = [{**rule, "profile": name} for name, prof in profiles.items() for rule in prof.get("auto_switch", [])]
    return rules + cfg.get("auto_switch", {}).get("rules", [])


def config_stamp(paths):
    files = [paths.config] + glob.glob(os.path.join(paths.profiles, "*.toml")) + glob.glob(os.path.join(paths.themes, "*.toml"))
    return max((os.stat(p).st_mtime for p in files if os.path.exists(p)), default=0), len(files)


# Settings and nyx-deck talk through two small files: the deck says what it shows,
# and Settings asks it to show a profile.
def state_file():
    return os.path.join(runtime_dir(), "nyx-deck-galleon.state")


def request_file():
    return os.path.join(runtime_dir(), "nyx-deck-galleon.request")


def read_json(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def write_json(path, value):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path + ".tmp", "w") as f:
        json.dump(value, f)
    os.replace(path + ".tmp", path)


# ------------------------------------------------------------ galleon-deck

GALLEON_UNIT = "galleon-deck.service"


def galleon_deck_running():
    """Whether galleon-deck has the deck (its service, or a copy started by hand)."""
    try:
        r = subprocess.run(["systemctl", "--user", "is-active", "--quiet", GALLEON_UNIT], timeout=3,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if r.returncode == 0:
            return True
    except (OSError, subprocess.TimeoutExpired):
        pass
    try:
        with open(os.path.join(runtime_dir(), "galleon-deck.pid")) as f:
            os.kill(int(f.read().strip()), 0)
        return True
    except (OSError, ValueError):
        return False


# ------------------------------------------------------------ TOML writing
# tomllib only reads. These write the small subset these files use, editing in
# place so hand-written comments survive.

def toml_str(text):
    out = ['"']
    for ch in str(text):
        o = ord(ch)
        if ch in '"\\':
            out.append("\\" + ch)
        elif ch == "\n":
            out.append("\\n")
        elif ch == "\t":
            out.append("\\t")
        elif o < 0x20 or o == 0x7F or 0xE000 <= o <= 0xF8FF:  # control, and icon glyphs (invisible in editors)
            out.append(f"\\u{o:04X}")
        elif o > 0xFFFF:
            out.append(f"\\U{o:08X}")
        else:
            out.append(ch)
    return "".join(out) + '"'


def toml_key(key):
    return key if re.fullmatch(r"[A-Za-z0-9_-]+", key) else toml_str(key)


def toml_value(value, multiline=False):
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return repr(value)
    if isinstance(value, str):
        return toml_str(value)
    if isinstance(value, dict):
        if not value:
            return "{}"
        return "{ " + ", ".join(f"{toml_key(k)} = {toml_value(v)}" for k, v in value.items()) + " }"
    if isinstance(value, (list, tuple)):
        if multiline and value:
            return "[\n" + "".join(f"  {toml_value(v)},\n" for v in value) + "]"
        return "[" + ", ".join(toml_value(v) for v in value) + "]"
    raise TypeError(f"can't write {type(value).__name__} to TOML")


def _depth(line):
    """Net bracket depth of a line, ignoring strings and comments."""
    depth, quote, i = 0, None, 0
    while i < len(line):
        ch = line[i]
        if quote:
            if ch == "\\" and quote == '"':
                i += 1
            elif ch == quote:
                quote = None
        elif ch in "\"'":
            quote = ch
        elif ch == "#":
            break
        elif ch in "[{":
            depth += 1
        elif ch in "]}":
            depth -= 1
        i += 1
    return depth


def _comment(line):
    """The trailing `# comment` of a line (outside strings), or ""."""
    quote, i = None, 0
    while i < len(line):
        ch = line[i]
        if quote:
            if ch == "\\" and quote == '"':
                i += 1
            elif ch == quote:
                quote = None
        elif ch in "\"'":
            quote = ch
        elif ch == "#":
            return line[i:]
        i += 1
    return ""


def write_atomic(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path + ".tmp", "w") as f:
        f.write(text)
    os.replace(path + ".tmp", path)


def set_value(path, table, key, value, remove=False):
    """Set (or remove) `key` in `[table]` (None = top level) of a TOML file, in place."""
    try:
        with open(path) as f:
            lines = f.read().split("\n")
    except FileNotFoundError:
        lines = []
    header = f"[{table}]" if table else None
    start = 0
    if header:
        start = next((i + 1 for i, line in enumerate(lines) if line.strip() == header), None)
        if start is None:
            if not remove:
                while lines and lines[-1] == "":
                    lines.pop()
                lines += ["", header, f"{key} = {toml_value(value, multiline=isinstance(value, list))}", ""]
                write_atomic(path, "\n".join(lines))
            return
    end = next((i for i in range(start, len(lines)) if re.match(r"\s*\[", lines[i]) and _depth(lines[i]) >= 0
                and re.match(r"\s*\[\[?[A-Za-z0-9_.\"-]+\]\]?\s*(#.*)?$", lines[i])), len(lines))
    new = [] if remove else f"{key} = {toml_value(value, multiline=isinstance(value, list))}".split("\n")
    for i in range(start, end):
        if re.match(rf"\s*{re.escape(key)}\s*=", lines[i]):
            j, depth = i, _depth(lines[i])
            while depth > 0 and j + 1 < len(lines):  # a value spanning lines
                j += 1
                depth += _depth(lines[j])
            comment = _comment(lines[i]) if j == i and len(new) == 1 else ""
            if comment:  # keep a trailing comment, aligned where it was
                col = len(lines[i]) - len(comment)
                new = [new[0].ljust(col) if len(new[0]) < col else new[0] + " "]
                new[0] += comment
            lines[i:j + 1] = new
            break
    else:
        if remove:
            return
        at = end
        while at > start and lines[at - 1].strip() == "":
            at -= 1
        lines[at:at] = new
    write_atomic(path, "\n".join(lines))


PROFILE_TOP = ("theme", "start_page", "logo")


def write_profile(path, profile):
    """Write a profile from a dict, keeping the file's leading comment block.
    Generated pages (the deck's settings page) aren't written."""
    head = []
    try:
        with open(path) as f:
            for line in f:
                if line.startswith("#") or not line.strip():
                    head.append(line.rstrip("\n"))
                else:
                    break
    except FileNotFoundError:
        head = ["# A Galleon 100 SD deck profile (galleon-deck's format), edited in Settings.", ""]
    while head and not head[-1].strip():
        head.pop()
    out = head + [""] if head else []
    for key in PROFILE_TOP:
        if profile.get(key):
            out.append(f"{key} = {toml_value(profile[key])}")
    if profile.get("settings_page") is False:
        out.append("settings_page = false")
    if profile.get("auto_switch"):
        out.append(f"auto_switch = {toml_value(profile['auto_switch'], multiline=True)}")
    known = set(PROFILE_TOP) | {"settings_page", "auto_switch", "theme_overrides", "page"}
    for key, value in profile.items():
        if key not in known and not isinstance(value, dict):
            out.append(f"{toml_key(key)} = {toml_value(value)}")
    if profile.get("theme_overrides"):
        out += ["", "[theme_overrides]"] + [f"{toml_key(k)} = {toml_value(v)}" for k, v in profile["theme_overrides"].items()]
    for page in profile.get("page", []):
        if page.get("settings"):
            continue
        keys = list(page.get("keys", []))
        while keys and not keys[-1]:
            keys.pop()
        out += ["", "[[page]]", f"name = {toml_value(page['name'])}"]
        out += [f"{toml_key(k)} = {toml_value(v)}" for k, v in page.items() if k not in ("name", "keys", "settings")]
        out.append("keys = [")
        out += [f"  {toml_value(k)}," for k in keys]
        out.append("]")
    write_atomic(path, "\n".join(out) + "\n")
