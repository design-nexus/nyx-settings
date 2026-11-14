"""Pretend webcams, for trying the pages without a camera.

WEBCAM_MOCK=obsbot-tiny-3-lite stands in for v4l2-ctl, the OBSBOT vendor
commands, pw-dump and wpctl. Everything it changes is kept in a JSON file in
SETTINGS_EXTENSION_STATE (or a scratch folder); nothing here opens a device,
writes WirePlumber rules or touches systemd.
"""

import json
import os
import re
import subprocess
import tempfile

PROFILES = ("obsbot-tiny-3-lite",)

# Paths nothing real can have, so Settings shows its own "no picture" and "no level".
DEVICE = "/dev/video-mock"
MIC = "settings-webcam-mock.obsbot-tiny-3-lite"
MIC_DESC = "OBSBOT Tiny 3 Lite microphone"
OTHER_MIC = "alsa_input.pci-0000_00_1f.3.analog-stereo"
STABLE = "usb-Remo_Tech_Co.__Ltd._OBSBOT_Tiny_3_Lite_MOCK0001-video-index0"
CARD = "OBSBOT Tiny 3 Lite: OBSBOT Tiny 3 Lite"
USB = ("3564", "ff04")

# (group, name, type, min, max, step, default, menu); values start at the default.
CONTROLS = [
    ("User Controls", "brightness", "int", 0, 100, 1, 50, None),
    ("User Controls", "contrast", "int", 0, 100, 1, 50, None),
    ("User Controls", "white_balance_automatic", "bool", 0, 1, 1, 1, None),
    ("User Controls", "white_balance_temperature", "int", 2000, 10000, 100, 5000, None),
    ("User Controls", "power_line_frequency", "menu", 0, 2, 1, 2, [(0, "Disabled"), (1, "50 Hz"), (2, "60 Hz")]),
    ("User Controls", "backlight_compensation", "bool", 0, 1, 1, 0, None),
    ("Camera Controls", "auto_exposure", "menu", 0, 3, 1, 3, [(1, "Manual Mode"), (3, "Aperture Priority Mode")]),
    ("Camera Controls", "exposure_time_absolute", "int", 1, 2500, 1, 156, None),
    ("Camera Controls", "focus_absolute", "int", 0, 100, 1, 0, None),
    ("Camera Controls", "focus_automatic_continuous", "bool", 0, 1, 1, 1, None),
    ("Camera Controls", "zoom_absolute", "int", 0, 100, 1, 0, None),
    ("Camera Controls", "pan_absolute", "int", -468000, 468000, 3600, 0, None),
    ("Camera Controls", "tilt_absolute", "int", -324000, 324000, 3600, 0, None),
]

# Which automatic setting makes each manual one unused: control -> (switch, value while unused).
INACTIVE_WHEN = {
    "white_balance_temperature": ("white_balance_automatic", 1),
    "exposure_time_absolute": ("auto_exposure", 3),
    "focus_absolute": ("focus_automatic_continuous", 1),
}


def profile():
    """The pretend camera asked for, or None when WEBCAM_MOCK isn't set."""
    name = (os.environ.get("WEBCAM_MOCK") or "").strip()
    if not name:
        return None
    if name not in PROFILES:
        raise SystemExit(f"WEBCAM_MOCK: no pretend camera called {name} (try {', '.join(PROFILES)})")
    return name


def state_dir():
    d = os.environ.get("SETTINGS_EXTENSION_STATE") or os.path.join(tempfile.gettempdir(), "nyx-webcam-mock")
    os.makedirs(d, exist_ok=True)
    return d


def _path():
    return os.path.join(state_dir(), "mock-camera.json")


def _fresh():
    values = {c[1]: c[6] for c in CONTROLS}
    values["zoom_absolute"] = 35  # "Half body", as in the mockup
    return {"controls": values, "asleep": False, "timer": 0, "default": OTHER_MIC, "muted": False}


def load():
    try:
        with open(_path()) as f:
            return json.load(f)
    except (OSError, ValueError):
        return _fresh()


def save(st):
    with open(_path(), "w") as f:
        json.dump(st, f)


def page_id():
    return re.sub(r"[^a-z0-9]+", "-", STABLE.lower()).strip("-")


def capture_nodes():
    return [(page_id(), DEVICE, CARD)]


def options_path():
    """The options file, seeded the first time like the mockup: kept awake, mic never
    idle, three saved positions."""
    path = os.path.join(state_dir(), "webcam.toml")
    if not os.path.exists(path):
        title = CARD.split(":", 1)[0]
        with open(path, "w") as f:
            f.write(f'[{json.dumps("camera:" + page_id())}]\nkeep_awake = true\n\n[{json.dumps("mic:" + MIC)}]\nno_suspend = true\n')
        positions = {title: {n: {"pan_absolute": p, "tilt_absolute": t, "zoom_absolute": z}
                             for n, (p, t, z) in {"1": (0, 0, 35), "2": (-36000, 0, 0), "3": (36000, -7200, 75)}.items()}}
        with open(os.path.join(state_dir(), "positions.json"), "w") as f:
            json.dump(positions, f)
    return path


# ------------------------------------------------------------- v4l2-ctl

def _list_ctrls(values):
    out, group = [], None
    for g, name, kind, lo, hi, step, default, menu in CONTROLS:
        if g != group:
            out += ["", g, ""]
            group = g
        v = values.get(name, default)
        if kind == "bool":
            fields = f"default={default} value={v}"
        elif kind == "menu":
            label = dict(menu).get(v, "")
            fields = f"min={lo} max={hi} default={default} value={v} ({label})"
        else:
            fields = f"min={lo} max={hi} step={step} default={default} value={v}"
        when = INACTIVE_WHEN.get(name)
        if when and values.get(when[0]) == when[1]:
            fields += " flags=inactive"
        out.append(f"{name:>31} 0x{0x00980900 + len(out):08x} {'(' + kind + ')':<8}: {fields}")
        for k, text in menu or []:
            out.append(f"\t\t\t\t{k}: {text}")
    return "\n".join(out) + "\n"


def v4l2_ctl(args):
    """Answers `v4l2-ctl -d DEV --list-ctrls-menus` and `-c a=1,b=2` like the real one."""
    args = list(args[1:])
    if args[:2] != ["-d", DEVICE]:
        return subprocess.CompletedProcess(args, 1, "", "Cannot open device: No such file or directory\n")
    st = load()
    if args[2:] == ["--list-ctrls-menus"]:
        return subprocess.CompletedProcess(args, 0, _list_ctrls(st["controls"]), "")
    if args[2:3] == ["-c"] and len(args) == 4:
        known = {c[1]: c for c in CONTROLS}
        for pair in args[3].split(","):
            name, _, value = pair.partition("=")
            c = known.get(name)
            if c is None:
                return subprocess.CompletedProcess(args, 1, "", f"unknown control '{name}'\n")
            n = int(value)
            if c[2] == "menu" and n not in dict(c[7]):
                return subprocess.CompletedProcess(args, 1, "", f"{name}: invalid value {n}\n")
            st["controls"][name] = max(c[3], min(c[4], n))
        save(st)
        return subprocess.CompletedProcess(args, 0, "", "")
    return subprocess.CompletedProcess(args, 1, "", "not something the pretend camera answers\n")


# ---------------------------------------------------------------- OBSBOT

def usb_ids(video_node):
    return USB if video_node == DEVICE else None


def is_tiny3(video_node):
    return video_node == DEVICE


class Camera:
    def __init__(self, video_node):
        if video_node != DEVICE:
            raise FileNotFoundError(2, "No such file or directory", video_node)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        pass

    def status(self):
        st = load()
        return st["asleep"], st["timer"]

    def set_auto_sleep(self, seconds):
        st = load()
        st["timer"] = int(seconds)
        save(st)
        return True

    def power(self, asleep):
        st = load()
        st["asleep"] = bool(asleep)
        save(st)
        return True


# ------------------------------------------------------- PipeWire, wpctl

def pw_dump():
    return [
        {"id": 50, "type": "PipeWire:Interface:Device",
         "info": {"props": {"device.vendor.id": "0x" + USB[0], "device.product.id": "0x" + USB[1]}}},
        {"id": 90, "type": "PipeWire:Interface:Node",
         "info": {"props": {"media.class": "Audio/Source", "device.id": 50, "node.name": MIC, "node.description": MIC_DESC}}},
    ]


def wpctl(args):
    st = load()
    out = ""
    if args[:2] == ["inspect", "@DEFAULT_AUDIO_SOURCE@"]:
        out = f'id 1, type PipeWire:Interface:Node\n  * node.name = "{st["default"]}"\n'
    elif args[:2] == ["get-volume", "90"]:
        out = "Volume: 1.00" + (" [MUTED]" if st["muted"] else "") + "\n"
    elif args[:2] == ["set-default", "90"]:
        st["default"] = MIC
    elif args[:3] == ["set-mute", "90", "0"]:
        st["muted"] = False
    save(st)
    return subprocess.CompletedProcess(args, 0, out, "")
