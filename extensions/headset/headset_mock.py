"""Pretend headsets for settings-headset, on only while HEADSET_MOCK is set.

HEADSET_MOCK=arctis7            HeadsetControl's `-o json` for an Arctis 7
HEADSET_MOCK=a50x               an ASTRO A50 X with Gearbench: `gearbench-ctl --json status`,
                                `audio`, `audio-set`, `audio-eq`, `audio-mix`, and its sound devices
HEADSET_MOCK=a50x-no-gearbench  an ASTRO A50 X plugged in, without Gearbench

Several can be given, split by commas. Changes go to a JSON file in the state folder;
nothing here opens a device or runs a program.
"""

import json
import os
import subprocess

ARCTIS = {
    "status": "success",
    "device": "Arctis 7",
    "vendor": "SteelSeries",
    "product": "Arctis 7",
    "id_vendor": "0x1038",
    "id_product": "0x12ad",
    "capabilities": ["CAP_SIDETONE", "CAP_BATTERY_STATUS", "CAP_LIGHTS", "CAP_INACTIVE_TIME", "CAP_CHATMIX_STATUS",
                     "CAP_VOICE_PROMPTS", "CAP_ROTATE_TO_MUTE", "CAP_EQUALIZER_PRESET", "CAP_MICROPHONE_MUTE_LED_BRIGHTNESS",
                     "CAP_MICROPHONE_VOLUME", "CAP_VOLUME_LIMITER", "CAP_BT_WHEN_POWERED_ON"],
    "capabilities_str": ["sidetone", "battery", "lights", "inactive time", "chatmix", "voice prompts", "rotate to mute",
                         "equalizer preset", "microphone mute led brightness", "microphone volume", "volume limiter",
                         "bluetooth when powered on"],
    "battery": {"status": "BATTERY_AVAILABLE", "level": 64, "time_to_empty_min": 900},
    "chatmix": 40,
    "equalizer_presets_count": 4,
    "equalizer_presets": {"flat": [0] * 10, "bass_boost": [6, 5, 3, 1, 0, 0, 0, 0, 0, 0],
                          "focus": [-2, -1, 0, 1, 2, 3, 3, 2, 1, 0], "smiley": [4, 3, 1, -1, -2, -2, -1, 1, 3, 4]},
}

# What was last set on the Arctis, as in the mockup.
ARCTIS_SET = {"sidetone": 30, "eq": "1", "limiter": False, "mic-volume": 100, "rotate-mute": True, "voice": True,
              "lights": True, "mute-led": 3, "inactive": "30", "bt-on": False}

A50X = {
    "battery": 82, "answering": True, "balance": 7, "boom": True, "sidetone": 3, "noise_gate": "night", "volume": 14,
    "auto_sleep": 30, "deep_sleep": False, "brightness": 60, "tones": "minimal",
    "equalizers": {"headphones": "gaming", "microphone": "broadcast"},
    "mix": {"stream": 26, "game": 24, "voice": 19, "mic": 29, "bluetooth": 0},
    "muted": ["bluetooth"],
}

BANDS = [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000]
GAINS = {"headphones": [3, 2, 0, -1, -2, 0, 2, 4, 3, 1], "microphone": [-6, -3, 1, 2, 1, 0, 2, 3, 1, -2]}
PRESETS = {"headphones": ["default", "gaming", "media"], "microphone": ["default", "competition", "broadcast"]}
SOURCES = ["stream", "game", "voice", "mic", "bluetooth"]
CHOICES = {"noise-gate": ["off", "night", "home", "tournament"], "tones": ["none", "all", "minimal"],
           "auto-sleep": ["never", "0", "15", "30", "60"], "deep-sleep": ["on", "off"]}
RANGES = {"sidetone": (0, 6), "volume": (0, 21), "balance": (0, 12), "brightness": (0, 100)}


def names():
    return [m.strip() for m in (os.environ.get("HEADSET_MOCK") or "").split(",") if m.strip()]


def _load(state_dir, name, default):
    try:
        with open(os.path.join(state_dir, name)) as f:
            return {**default, **json.load(f)}
    except (OSError, ValueError):
        return json.loads(json.dumps(default))


def _save(state_dir, name, data):
    os.makedirs(state_dir, exist_ok=True)
    with open(os.path.join(state_dir, name), "w") as f:
        json.dump(data, f)


# ------------------------------------------------------------------ HeadsetControl

def headsetcontrol(args):
    devices = [ARCTIS] if "arctis7" in names() else []
    out = json.dumps({"name": "HeadsetControl", "version": "3.0.0", "api_version": "1.9",
                      "device_count": len(devices), "devices": devices}) if "-o" in args else ""
    return subprocess.CompletedProcess(["headsetcontrol", *args], 0, out, "")


def last_set(state_dir, pid):
    return _load(state_dir, f"{pid}.json", ARCTIS_SET if pid == "1038-12ad" else {})


def warning_on(state_dir):
    return _load(state_dir, "warning.json", {"on": True})["on"]


def set_warning(state_dir, on):
    _save(state_dir, "warning.json", {"on": on})


# ------------------------------------------------------------------ Gearbench

def plugged_a50x():
    return any(n.startswith("a50x") for n in names())


def gearbench_installed():
    return "a50x" in names()


def gearbench(args, state_dir):
    """What `gearbench-ctl --json ARGS` would print: (exit status, stdout, stderr)."""
    args = [a for a in args if a != "--json"]
    st = _load(state_dir, "a50x.json", A50X)
    cmd, rest = args[0], args[1:]
    if cmd == "status":
        return 0, json.dumps(_status(st)), ""
    if cmd == "audio":
        return 0, json.dumps(_audio(st)), ""
    if cmd == "gui":
        return 0, "", ""
    try:
        if cmd == "audio-set":
            _audio_set(st, *rest)
        elif cmd == "audio-eq":
            _audio_eq(st, *rest)
        elif cmd == "audio-mix":
            _audio_mix(st, *rest)
        else:
            return 2, "", f"gearbench-ctl: invalid choice: '{cmd}'"
    except (ValueError, TypeError, IndexError) as e:
        return 1, "", f"refused: {e}"
    _save(state_dir, "a50x.json", st)
    return 0, json.dumps(_audio(st)), ""


def _status(st):
    # Field names as `status --json` documents them in gearbench-ctl(1).
    return {
        "headsets": ["Astro A50 X"],
        "headsets_silent": [] if st["answering"] else ["Astro A50 X"],
        "headsets_attached": [{"name": "Astro A50 X", "device": "/dev/hidraw7", "route": "base station",
                               "battery_found": True, "battery": st["battery"], "battery_at": None,
                               "battery_warned": None, "answering": st["answering"]}],
        "problems": [],
        "focus": {"source": "processes"},
    }


def _audio(st):
    # gearbench-ctl(1) lists what `audio` reports but not its JSON field names; this is the
    # shape settings-headset's parse_audio() reads.
    return {
        "model": "Astro A50 X", "device": "/dev/hidraw7", "battery": st["battery"],
        "balance": st["balance"], "boom": st["boom"], "sidetone": st["sidetone"], "noise_gate": st["noise_gate"],
        "equalizers": {inst: {"preset": preset, "bands": [[f, g] for f, g in zip(BANDS, GAINS[inst])]}
                       for inst, preset in st["equalizers"].items()},
        "stream_mix": {s: {"level": st["mix"][s], "muted": s in st["muted"]} for s in SOURCES},
        "volume": st["volume"], "auto_sleep": st["auto_sleep"], "deep_sleep": st["deep_sleep"],
        "brightness": st["brightness"], "tones": st["tones"],
    }


def _audio_set(st, setting, value=None):
    key = setting.replace("-", "_")
    if setting in RANGES:
        lo, hi = RANGES[setting]
        n = int(value)
        if not lo <= n <= hi:
            raise ValueError(f"{setting} is {lo} to {hi}")
        st[key] = n
    elif setting in CHOICES:
        if value not in CHOICES[setting]:
            raise ValueError(f"{setting} is one of {', '.join(CHOICES[setting])}")
        if setting == "auto-sleep":
            st[key] = 0 if value in ("never", "0") else int(value)
        elif setting == "deep-sleep":
            st[key] = value == "on"
        else:
            st[key] = value
    else:
        raise ValueError(f"no setting {setting}")


def _audio_eq(st, inst, *opts):
    if inst not in PRESETS or list(opts[:1]) != ["--pick"] or len(opts) != 2 or opts[1] not in PRESETS[inst]:
        raise ValueError(f"{inst} takes --pick {' or '.join(PRESETS.get(inst, []))}")
    st["equalizers"][inst] = opts[1]


def _audio_mix(st, source, *opts):
    if source not in SOURCES:
        raise ValueError(f"no source {source}")
    if opts == ("--mute",) or opts == ("--unmute",):
        if source == "stream":
            raise ValueError("the stream port has no mute")
        st["muted"] = sorted(set(st["muted"]) - {source} | ({source} if opts[0] == "--mute" else set()))
        return
    n = int(opts[0])
    if not 0 <= n <= 32:
        raise ValueError("a level is 0 to 32")
    st["mix"][source] = n


def sound_devices():
    return [
        {"kind": "sink", "name": "A50 X Game", "default": True, "apps": ["Firefox", "Steam"]},
        {"kind": "sink", "name": "A50 X Chat", "default": False, "apps": ["Discord", "Signal", "Zoom"]},
        {"kind": "source", "name": "A50 X Microphone", "default": True, "apps": []},
    ]
