"""The Corsair Galleon 100 SD's built-in Stream Deck (USB 1b1c:2b18), run by nyx-deck.

The protocol, from node-elgato-stream-deck's "Galleon K100 SD" (v7.5.0) as
galleon-deck uses it:

 - feature report 03 27 every 500 ms is a keep-alive that puts the deck in host
   mode. Until it arrives the firmware runs its own numpad and ignores every
   other command; once the pings stop it goes back to the numpad by itself.
 - feature 03 08 <0-100> sets the brightness.
 - output 02 07: a key image (160x160 JPEG) in 1024-byte packets, header
   02 07 <key> <last> <length LE16> <part LE16>, up to 1016 bytes each.
 - output 02 0c: the top screen (720x384 JPEG), header 02 0c <x> <y> <w> <h>
   (LE16 each) <last> <part LE16> <length LE16> 00, up to 1008 bytes each.
 - input 01 00 0c 00 + 12 bytes: key states. input 01 03 03 00 <kind> <left>
   <right>: kind 0 is dial presses, kind 1 signed clicks of rotation.

Device I/O goes through a transport (`Hidraw`: feature, write, read, close),
typing through `Keyboard` (a uinput device), pictures through galleon_draw and
sounds, media and commands through `System`; the tests swap in simulated ones.
nyx-deck only opens the deck when `galleon = true` is in its config.toml, and
lets go of it whenever galleon-deck is running.

The deck's behaviour is ported from galleon-deck (https://github.com/NLMP-DDHS/galleon-deck):

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

import array
import fcntl
import glob
import json
import os
import random
import re
import select
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time

import galleon_config as gc

VID, PID = "00001B1C", "00002B18"
KEYS = gc.KEYS
LCD_W, LCD_H = 720, 384
PING_INTERVAL = 0.5
PACKET = 1024
KEY_CHUNK = PACKET - 8
LCD_CHUNK = PACKET - 16
MIN_REPEAT_GAP = 0.001  # a repeat at 0 ms still yields, so it can't flood the desktop
VOLUME_SHOWN = 3.0  # seconds the volume bar stays up after a change, with volume = "changing"
KEYBOARD_NAME = "nyx-deck-galleon"

SCREEN_DEFAULTS = {"profile": True, "playing": True, "volume": "always"}
DIAL_DEFAULTS = {"left_turn": "volume", "left_press": "mute", "left_hold_turn": "none",
                 "right_turn": "pages", "right_press": "start-page", "right_hold_turn": "profiles"}

ICON_DIMMER, ICON_BRIGHTER = "\U000F00DE", "\U000F00E0"
ICON_PROFILE_ON, ICON_PROFILE = "\U000F0009", "\U000F0004"
ICON_WALLPAPER = "\U000F0E09"
ICON_PLAYING, ICON_PAUSED = "\U000F040A", "\U000F03E4"
ICON_DONE, ICON_FAILED = "\U000F012C", "\U000F0156"


def log(*args):
    print("galleon:", *args, file=sys.stderr, flush=True)


# --------------------------------------------------------------- protocol

PING = bytes([0x03, 0x27])


def brightness_report(pct):
    return bytes([0x03, 0x08, max(0, min(100, int(pct)))])


def key_packets(index, jpeg):
    packets, part, off = [], 0, 0
    while off < len(jpeg):
        chunk = jpeg[off:off + KEY_CHUNK]
        last = off + len(chunk) >= len(jpeg)
        hdr = bytes([0x02, 0x07, index, int(last)]) + len(chunk).to_bytes(2, "little") + part.to_bytes(2, "little")
        packets.append((hdr + chunk).ljust(PACKET, b"\0"))
        off += len(chunk)
        part += 1
    return packets


def lcd_packets(jpeg, x=0, y=0, w=LCD_W, h=LCD_H):
    packets, part, off = [], 0, 0
    while off < len(jpeg):
        chunk = jpeg[off:off + LCD_CHUNK]
        last = off + len(chunk) >= len(jpeg)
        hdr = (bytes([0x02, 0x0C]) + x.to_bytes(2, "little") + y.to_bytes(2, "little") + w.to_bytes(2, "little")
               + h.to_bytes(2, "little") + bytes([int(last)]) + part.to_bytes(2, "little")
               + len(chunk).to_bytes(2, "little") + b"\0")
        packets.append((hdr + chunk).ljust(PACKET, b"\0"))
        off += len(chunk)
        part += 1
    return packets


def parse_report(data):
    """("keys", [12 states]), ("press", left, right), ("turn", left, right) or None."""
    if len(data) < 5 or data[0] != 0x01:
        return None
    if data[1] == 0x00:
        return ("keys", [int(bool(s)) for s in data[4:4 + KEYS]])
    if data[1] == 0x03 and len(data) >= 7:
        a, b = data[5], data[6]
        if data[4] == 1:
            return ("turn", a - 256 if a > 127 else a, b - 256 if b > 127 else b)
        if data[4] == 0:
            return ("press", bool(a), bool(b))
    return None


class Hidraw:
    """The deck's interface-0 hidraw node."""

    def __init__(self, path):
        self.path = path
        self.fd = os.open(path, os.O_RDWR | os.O_NONBLOCK)

    def feature(self, payload):
        buf = array.array("B", list(payload) + [0] * (32 - len(payload)))
        fcntl.ioctl(self.fd, 0xC0000000 | (32 << 16) | (ord("H") << 8) | 0x06, buf, True)  # HIDIOCSFEATURE(32)

    def write(self, packet):
        os.write(self.fd, packet)

    def read(self, timeout):
        if not select.select([self.fd], [], [], max(0.0, timeout))[0]:
            return None
        try:
            return os.read(self.fd, PACKET)
        except BlockingIOError:
            return None

    def close(self):
        try:
            os.close(self.fd)
        except OSError:
            pass


def find_hidraw():
    """The deck's interface-0 hidraw node, or None if it isn't plugged in."""
    for node in sorted(glob.glob("/sys/class/hidraw/hidraw*")):
        try:
            with open(os.path.join(node, "device/uevent")) as f:
                uevent = f.read()
        except OSError:
            continue
        if f"{VID}:{PID}" in uevent and ":1.0/" in os.path.realpath(os.path.join(node, "device")):
            return "/dev/" + os.path.basename(node)
    return None


def open_hidraw():
    path = find_hidraw()
    return Hidraw(path) if path else None


class Deck:
    """The protocol over a transport. Pings and pictures can come from different
    threads, so every report is sent under one lock."""

    def __init__(self, transport):
        self.io = transport
        self.lock = threading.Lock()

    def ping(self):
        with self.lock:
            self.io.feature(PING)

    def brightness(self, pct):
        with self.lock:
            self.io.feature(brightness_report(pct))

    def _send(self, packets):
        for p in packets:
            with self.lock:  # per packet, so pings can come in between
                self.io.write(p)

    def key_image(self, index, jpeg):
        self._send(key_packets(index, jpeg))

    def lcd_image(self, jpeg):
        self._send(lcd_packets(jpeg))

    def read(self, timeout):
        return self.io.read(timeout)

    def close(self):
        self.io.close()


# ------------------------------------------------------------------ typing

EV_SYN, EV_KEY, EV_REL = 0, 1, 2
REL_X, REL_Y, REL_WHEEL = 0, 1, 8
UI_SET_EVBIT, UI_SET_KEYBIT, UI_SET_RELBIT = 0x40045564, 0x40045565, 0x40045566
UI_DEV_SETUP, UI_DEV_CREATE, UI_DEV_DESTROY = 0x405C5503, 0x5501, 0x5502
BUS_VIRTUAL = 0x06


class Uinput:
    """A virtual input device on /dev/uinput (keys, and relative axes for a mouse)."""

    def __init__(self, name, keys, rel=()):
        self.fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
        try:
            fcntl.ioctl(self.fd, UI_SET_EVBIT, EV_KEY)
            for code in keys:
                fcntl.ioctl(self.fd, UI_SET_KEYBIT, code)
            if rel:
                fcntl.ioctl(self.fd, UI_SET_EVBIT, EV_REL)
                for code in rel:
                    fcntl.ioctl(self.fd, UI_SET_RELBIT, code)
            fcntl.ioctl(self.fd, UI_DEV_SETUP, struct.pack("HHHH80sI", BUS_VIRTUAL, 0x1B1C, 0x2B18, 1, name.encode()[:79], 0))
            fcntl.ioctl(self.fd, UI_DEV_CREATE)
        except OSError:
            os.close(self.fd)
            raise

    def write(self, kind, code, value):
        os.write(self.fd, struct.pack("llHHi", 0, 0, kind, code, value))

    def syn(self):
        self.write(EV_SYN, 0, 0)

    def close(self):
        try:
            fcntl.ioctl(self.fd, UI_DEV_DESTROY)
            os.close(self.fd)
        except OSError:
            pass


def make_keyboard_device():
    keys = sorted({c for c in gc.key_codes().values() if 0 < c < 0x2FF})
    return Uinput(KEYBOARD_NAME, keys)


def make_mouse_device():
    ui = Uinput(KEYBOARD_NAME + "-mouse", [gc.BTN_LEFT, gc.BTN_RIGHT, gc.BTN_MIDDLE, gc.BTN_SIDE, gc.BTN_EXTRA],
                [REL_X, REL_Y, REL_WHEEL])
    time.sleep(0.2)  # give the desktop time to pick up the new device
    return ui


class Keyboard:
    """Keys and shortcuts typed through a virtual keyboard, and a mouse made on first use."""

    def __init__(self, make_keyboard=make_keyboard_device, make_mouse=make_mouse_device):
        self.make_keyboard, self.make_mouse = make_keyboard, make_mouse
        self.ui = None
        self.mouse_ui = None
        self.numlock_checked = False
        self.lock = threading.Lock()  # sequences type from their own thread

    def keyboard(self):
        if self.ui is None:
            self.ui = self.make_keyboard()
        return self.ui

    def press(self, codes):
        with self.lock:
            ui = self.keyboard()
            for c in codes:
                ui.write(EV_KEY, c, 1)
            ui.syn()

    def release(self, codes):
        with self.lock:
            ui = self.keyboard()
            for c in reversed(codes):
                ui.write(EV_KEY, c, 0)
            ui.syn()

    def tap(self, codes):
        self.press(codes)
        self.release(codes)

    def ensure_numlock(self):
        """Numpad keys type digits only with NumLock on, and Hyprland gives a
        virtual keyboard its own lock state."""
        if self.numlock_checked:
            return
        self.numlock_checked = True
        if self.ui is None:
            self.keyboard()
            time.sleep(0.3)  # until Hyprland lists the new keyboard
        try:
            for kb in json.loads(cmd_output(["hyprctl", "devices", "-j"]) or "{}").get("keyboards", []):
                if kb.get("name") == KEYBOARD_NAME and not kb.get("numLock"):
                    self.tap(gc.codes("NUMLOCK"))
        except ValueError:
            pass

    def mouse(self):
        if self.mouse_ui is None:
            self.mouse_ui = self.make_mouse()
        return self.mouse_ui

    def mouse_write(self, *events):
        ui = self.mouse()
        with self.lock:
            for kind, code, value in events:
                ui.write(kind, code, value)
            ui.syn()

    def click(self, button, count=1):
        for n in range(count):
            if n:
                time.sleep(0.02)
            self.mouse_write((EV_KEY, button, 1))
            time.sleep(0.02)  # some apps miss a press and release in the same instant
            self.mouse_write((EV_KEY, button, 0))

    def run_step(self, step, pause, stop=None):
        """Do one step, then wait out the gap to the next. `stop` cuts waits short."""
        def wait(seconds):
            if stop:
                stop.wait(seconds)
            elif seconds:
                time.sleep(seconds)
        if not isinstance(step, tuple):
            self.press(step)
            wait(pause)
            self.release(step)
        elif step[0] == "click":
            self.click(step[1], step[2])
        elif step[0] == "move":
            self.mouse_write((EV_REL, REL_X, step[1]), (EV_REL, REL_Y, step[2]))
        elif step[0] == "scroll":
            self.mouse_write((EV_REL, REL_WHEEL, step[1]))
        elif step[0] == "wait":
            wait(step[1] / 1000)
            return
        wait(pause)

    def type_sequence(self, steps, hold=(), step_ms=40, stop=None):
        """Each step in turn with `hold` held throughout (Helldivers stratagems:
        hold Ctrl, tap the arrows). Setting `stop` ends it after the current step."""
        pause = max(0, min(500, int(step_ms))) / 1000
        if hold:
            self.press(hold)
            time.sleep(pause)
        try:
            for step in steps:
                if stop and stop.is_set():
                    break
                self.run_step(step, pause, stop)
        finally:
            if hold:
                self.release(hold)

    def close(self):
        for ui in (self.ui, self.mouse_ui):
            if ui:
                ui.close()


# ------------------------------------------------------------ the desktop

def cmd_output(argv, timeout=1):
    try:
        return subprocess.run(argv, capture_output=True, text=True, timeout=timeout).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        return ""


def session_env():
    """Our environment with empty or missing variables filled in from the systemd
    user manager: the service can start before the compositor has exported them."""
    env = dict(os.environ)
    for line in cmd_output(["systemctl", "--user", "show-environment"], timeout=2).splitlines():
        name, sep, value = line.partition("=")
        if sep and value and not env.get(name):
            env[name] = value
    return env


class System:
    """Volume, media, commands and what's playing, on the real desktop."""

    def __init__(self):
        self._uwsm = None

    def audio(self):
        return "wpctl" if shutil.which("wpctl") else "pactl" if shutil.which("pactl") else None

    def volume(self):
        """(percent, muted) of the default output, or (None, False)."""
        backend = self.audio()
        try:
            if backend == "wpctl":
                out = cmd_output(["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"])  # "Volume: 0.42 [MUTED]"
                return round(float(out.split()[1]) * 100), "MUTED" in out
            if backend == "pactl":
                vol = cmd_output(["pactl", "get-sink-volume", "@DEFAULT_SINK@"])
                mute = cmd_output(["pactl", "get-sink-mute", "@DEFAULT_SINK@"])
                return int(re.search(r"(\d+)%", vol).group(1)), mute.endswith("yes")
        except (IndexError, ValueError, AttributeError):
            pass
        return None, False

    def change_volume(self, percent):
        backend = self.audio()
        if backend == "wpctl":
            cmd = ["wpctl", "set-volume"] + (["-l", "1.0"] if percent > 0 else [])
            subprocess.run(cmd + ["@DEFAULT_AUDIO_SINK@", f"{abs(percent)}%{'+' if percent > 0 else '-'}"], timeout=2)
        elif backend == "pactl":
            vol, _ = self.volume()
            target = max(0, min(100, (vol or 0) + percent))
            subprocess.run(["pactl", "set-sink-volume", "@DEFAULT_SINK@", f"{target}%"], timeout=2)

    def toggle_mute(self):
        backend = self.audio()
        if backend == "wpctl":
            subprocess.run(["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"], timeout=2)
        elif backend == "pactl":
            subprocess.run(["pactl", "set-sink-mute", "@DEFAULT_SINK@", "toggle"], timeout=2)

    def media(self, action):
        """Through playerctl; False when it isn't there (then the deck taps a media key)."""
        if not shutil.which("playerctl"):
            return False
        subprocess.Popen(["playerctl", action], start_new_session=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return True

    def now_playing(self):
        """(status, "artist — title"), status being Playing, Paused or ""."""
        if not shutil.which("playerctl"):
            return "", ""
        status = cmd_output(["playerctl", "status"])
        if status not in ("Playing", "Paused"):
            return "", ""
        return status, cmd_output(["playerctl", "metadata", "--format", "{{artist}} — {{title}}"]).strip(" —")

    def uwsm(self):
        if self._uwsm is None:
            self._uwsm = bool(shutil.which("uwsm")) and subprocess.run(
                ["systemctl", "--user", "is-active", "--quiet", "wayland-wm@*.service"]).returncode == 0
        return self._uwsm

    def run(self, cmd):
        """A shell command; under uwsm, launched through it so apps get the session's environment."""
        argv = ["sh", "-c", cmd]
        if os.environ.get("UWSM_WAIT_VARNAMES") or self.uwsm():
            argv = ["uwsm", "app", "--"] + argv
        return subprocess.Popen(argv, env=session_env(), start_new_session=True,
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def hyprland_socket():
    sig = os.environ.get("HYPRLAND_INSTANCE_SIGNATURE") or session_env().get("HYPRLAND_INSTANCE_SIGNATURE")
    if not sig:
        return None
    return os.path.join(gc.runtime_dir(), "hypr", sig, ".socket2.sock")


def focus_events(callback, running):
    """callback(class, title) when the focused window changes. Hyprland only: its
    socket2 `activewindow` events (galleon-deck also follows Sway, i3, niri and
    X11; nyx-deck doesn't)."""
    while running():
        path = hyprland_socket()
        if not path:
            time.sleep(5)
            continue
        try:
            current = json.loads(cmd_output(["hyprctl", "activewindow", "-j"]) or "{}")
            if current:
                callback(current.get("class", ""), current.get("title", ""))
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
                s.connect(path)
                for line in s.makefile(errors="replace"):
                    if not running():
                        return
                    event, _, payload = line.rstrip("\n").partition(">>")
                    if event == "activewindow":
                        cls, _, title = payload.partition(",")
                        callback(cls, title)
        except (OSError, ValueError) as e:
            log(f"window focus: {e}")
        time.sleep(3)


# -------------------------------------------------------------------- app

class App:
    """What the deck shows and does. Every outside effect goes through `open_deck`
    (returns a Deck or None), `kb`, `system` and `draw` (galleon_draw)."""

    def __init__(self, paths, open_deck, kb, system, draw, enabled=lambda: True,
                 blocked=gc.galleon_deck_running, publish=True):
        self.paths = paths
        self.open_deck, self.kb, self.system, self.draw = open_deck, kb, system, draw
        self.enabled, self.blocked, self.publish = enabled, blocked, publish
        self.sleep = time.sleep
        self.cfg = None
        self.cfg_stamp = None
        self.error = None
        self.profile = None
        self.page = 0
        self.history = []
        self.deck = None
        self.pending = None
        self.key_state = [0] * KEYS
        self.held = {}
        self.typing = threading.Event()
        self.cancellable = None
        self.repeating = {}
        self.dial_down = [False, False]
        self.dial_turned = [False, False]
        self.last_lcd = None
        self.lcd_dirty = True
        self.volume = (None, False)
        self.volume_changed = 0.0
        self.running = True
        self.lock = threading.RLock()
        self.auto_target = None
        self.published = None

    # config ----------------------------------------------------------------
    def reload_if_changed(self):
        stamp = gc.config_stamp(self.paths)
        if stamp == self.cfg_stamp:
            return
        self.cfg_stamp = stamp
        try:
            cfg = gc.load_config(self.paths)
        except (OSError, ValueError, re.error) as e:
            self.error = f"{e}"
            log("config:", self.error)
            self.lcd_dirty = True
            return
        first = self.cfg is None
        self.release_held()
        self.cfg, self.error = cfg, None
        if first or self.profile not in cfg["profiles"]:
            start = cfg.get("start_profile")
            self.profile = start if start in cfg["profiles"] else next(iter(cfg["profiles"]))
            self.page = self.start_page_index()
            self.history.clear()
        self.page = min(self.page, len(self.pages()) - 1)
        if self.deck:
            self.deck.brightness(cfg.get("brightness", 70))
            self.draw_keys()
        self.lcd_dirty = True

    def prof(self):
        return self.cfg["profiles"][self.profile] if self.cfg else None

    def pages(self):
        return self.prof()["page"] if self.cfg else [{"name": "-", "keys": [{}] * KEYS}]

    @property
    def theme(self):
        return self.prof()["theme"] if self.cfg else gc.THEME_DEFAULTS

    def screen(self):
        return {**SCREEN_DEFAULTS, **(self.cfg.get("screen", {}) if self.cfg else {})}

    def dials(self):
        return {**DIAL_DEFAULTS, **(self.cfg.get("dials", {}) if self.cfg else {})}

    def start_page_index(self):
        names = [p["name"] for p in self.pages()]
        start = self.prof().get("start_page") if self.cfg else None
        return names.index(start) if start in names else 0

    def settings_keys(self):
        """The generated settings page: themes, wallpaper, brightness, then profiles."""
        slots = KEYS - 3 - min(len(self.cfg["profiles"]), 3)
        keys = [{"set_theme": name} for name in self.cfg["themes"] if name != gc.WALLPAPER][:slots]
        keys.append({"wallpaper_theme": True})
        keys.append({"icon": ICON_DIMMER, "label": "dimmer", "brightness": -10})
        keys.append({"icon": ICON_BRIGHTER, "label": "brighter", "brightness": 10})
        for name in self.cfg["profiles"]:
            if len(keys) >= KEYS:
                break
            keys.append({"icon": ICON_PROFILE_ON if name == self.profile else ICON_PROFILE, "label": name,
                         "profile": name, **({"fg": "accent"} if name == self.profile else {})})
        return keys + [{}] * (KEYS - len(keys))

    def keys(self):
        page = self.pages()[self.page]
        return self.settings_keys() if page.get("settings") else page["keys"]

    # drawing ---------------------------------------------------------------
    def key_image(self, i, pressed=False):
        spec = self.keys()[i]
        pressed = pressed or i in self.repeating  # a running repeat stays lit
        if "wallpaper_theme" in spec:
            if gc.WALLPAPER not in self.cfg["themes"]:
                return self.draw.key({"icon": ICON_WALLPAPER, "label": "wallpaper"}, self.theme, pressed)
            spec = {"set_theme": gc.WALLPAPER}
        if "set_theme" in spec:
            name = spec["set_theme"]
            img = self.draw.theme_key(name, self.cfg["themes"][name], name == self.prof()["theme_name"])
            return self.draw.invert(img) if pressed else img
        return self.draw.key(spec, self.theme, pressed, self.paths)

    def draw_key(self, i, pressed=False):
        self.deck.key_image(i, self.draw.jpeg(self.key_image(i, pressed)))

    def draw_keys(self):
        for i in range(KEYS):
            self.draw_key(i)

    def lcd_state(self):
        now = time.localtime()
        screen = self.screen()
        state = {
            "profile": self.profile or "nyx-deck",
            "page": self.pages()[self.page]["name"],
            "page_index": self.page,
            "page_count": len(self.pages()),
            "clock": time.strftime(self.cfg.get("clock_format", "%H:%M") if self.cfg else "%H:%M", now),
            "date": time.strftime("%A %d %B", now).upper(),
            "error": self.error,
            "show_profile": screen["profile"] is not False,
        }
        if screen["volume"] == "always" or (screen["volume"] == "changing" and time.time() - self.volume_changed < VOLUME_SHOWN):
            state["volume"] = self.volume
        if screen["playing"] is not False:
            status, text = self.system.now_playing()
            if status:
                state["media"] = text
                state["media_icon"] = ICON_PLAYING if status == "Playing" else ICON_PAUSED
        return state

    def draw_lcd(self):
        state = self.lcd_state()
        key = repr(sorted(state.items()))
        if key != self.last_lcd or self.lcd_dirty:
            self.deck.lcd_image(self.draw.jpeg(self.draw.lcd(state, self.theme)))
            self.last_lcd, self.lcd_dirty = key, False

    def publish_state(self):
        """Tell Settings what the deck shows, so its page opens there."""
        if not self.cfg or not self.publish:
            return
        state = {"profile": self.profile, "page": self.pages()[self.page]["name"]}
        if state != self.published:
            try:
                gc.write_json(gc.state_file(), state)
                self.published = state
            except OSError as e:
                log(f"state file: {e}")

    def take_request(self):
        """Settings asked to show a profile (and page)."""
        if not self.publish:
            return
        req = gc.read_json(gc.request_file())
        if req is None:
            return
        try:
            os.remove(gc.request_file())
        except OSError:
            pass
        if req.get("profile"):
            self.set_profile(req["profile"])
        if req.get("page"):
            self.goto(req["page"])

    # navigation ------------------------------------------------------------
    def release_held(self):
        for i in list(self.held):
            self.kb.release(self.held.pop(i))
        for i in list(self.repeating):
            self.repeating.pop(i).set()

    def set_page(self, index, remember=True):
        index %= len(self.pages())
        if index == self.page:
            return
        self.release_held()
        if remember:
            self.history.append(self.page)
        self.page = index
        if self.deck:
            self.draw_keys()
        self.lcd_dirty = True

    def goto(self, name):
        names = [p["name"] for p in self.pages()]
        if name == "back":
            self.set_page(self.history.pop() if self.history else self.start_page_index(), remember=False)
        elif name in names:
            self.set_page(names.index(name))
        else:
            log(f"no page named {name!r}")

    def set_profile(self, name):
        if not self.cfg or name not in self.cfg["profiles"] or name == self.profile:
            return
        self.release_held()
        self.profile = name
        self.page = self.start_page_index()
        self.history.clear()
        self.transition()
        self.lcd_dirty = True

    def transition(self):
        """The new profile's logo resolves out of interference while its keys do
        the same. Any key or dial skips it."""
        if not self.deck:
            return
        settings = self.cfg.get("transition", {}) if self.cfg else {}
        if not settings.get("enabled", True):
            self.draw_keys()
            return
        speed = max(0.25, min(4.0, float(settings.get("speed", 1.0))))
        frames = max(2, round(9 / speed))
        hold = max(0.0, min(3.0, float(settings.get("hold", 0.5)))) / speed
        theme, rng = self.theme, random.Random()
        keys = [self.key_image(i) for i in range(KEYS)]
        logo_path = self.prof().get("logo")
        logo = self.draw.load_logo(self.paths.resolve(logo_path) if logo_path else None, theme, self.profile.upper())
        try:
            for f in range(frames):
                done = (f + 1) / frames
                amount = (1 - done) ** 1.5
                self.deck.lcd_image(self.draw.jpeg(self.draw.glitch(self.draw.screen_logo(theme, logo, done), rng, amount)))
                for i, img in enumerate(keys):
                    self.deck.key_image(i, self.draw.jpeg(self.draw.glitch(img, rng, amount)))
                if self.interrupted(0.05 / speed):
                    break
            else:
                self.draw_keys()
                if hold:
                    self.deck.lcd_image(self.draw.jpeg(self.draw.screen_logo(theme, logo, 1.0)))
                    self.interrupted(hold)
        except OSError as e:
            log(f"transition: {e}")
        self.draw_keys()
        self.last_lcd, self.lcd_dirty = None, True

    def interrupted(self, seconds):
        """Wait, unless a key or dial report arrives; the main loop handles that one."""
        data = self.deck.read(seconds)
        if data:
            self.pending = data
            return True
        return False

    def set_theme(self, name):
        gc.set_value(self.prof()["path"], None, "theme", name)
        self.reload_if_changed()

    def apply_wallpaper_theme(self):
        if self.draw.write_wallpaper_theme(self.paths, self.cfg.get("wallpaper_theme", {})):
            self.set_theme(gc.WALLPAPER)

    def follow_wallpaper(self):
        """With [wallpaper_theme] auto = true, make the theme again when the wallpaper changes."""
        settings = self.cfg.get("wallpaper_theme", {}) if self.cfg else {}
        if not settings.get("auto"):
            return
        current = self.cfg["themes"].get(gc.WALLPAPER, {})
        path = self.draw.wallpaper_path(settings)
        if path and (path != current.get("source") or int(os.stat(path).st_mtime) != current.get("source_mtime")):
            self.draw.write_wallpaper_theme(self.paths, settings, path)
            self.reload_if_changed()

    def set_brightness(self, delta):
        level = max(10, min(100, self.cfg.get("brightness", 70) + delta))
        gc.set_value(self.paths.config, None, "brightness", level)
        self.reload_if_changed()

    def on_focus(self, cls, title):
        """Only acts when the matching profile changes, so a profile picked by hand stays."""
        if not self.cfg or self.cfg.get("auto_switch", {}).get("enabled", True) is False:
            return
        target = self.cfg.get("auto_switch", {}).get("fallback")
        for rule in self.cfg["switch_rules"]:
            if re.search(rule.get("class", ""), cls) and re.search(rule.get("title", ""), title):
                target = rule["profile"]
                break
        with self.lock:
            if target != self.auto_target:
                self.auto_target = target
                if target:
                    self.set_profile(target)

    # input -----------------------------------------------------------------
    def key_down(self, i):
        spec = self.keys()[i]
        if not spec:
            return
        page, profile = self.page, self.profile
        try:
            if "key" in spec or "keys" in spec:
                if str(spec.get("key", "")).upper().startswith("KP"):
                    self.kb.ensure_numlock()
                codes = gc.codes(spec.get("key") or spec.get("keys"))
                self.kb.press(codes)
                self.held[i] = codes
            if "sequence" in spec or "mouse" in spec:
                if spec.get("loop"):
                    self.toggle_loop(i, spec)
                else:
                    self.play_sequence(spec)
            if "repeat" in spec:
                self.toggle_repeat(i, spec)
            if "exec" in spec:
                proc = self.system.run(spec["exec"])
                if spec.get("confirm"):
                    threading.Thread(target=self.confirm, args=(i, proc, page, profile), daemon=True).start()
            if "media" in spec:
                if not self.system.media(spec["media"]):
                    names = {"play-pause": "PLAYPAUSE", "next": "NEXTSONG", "previous": "PREVIOUSSONG", "stop": "STOPCD"}
                    self.kb.tap(gc.codes(names.get(spec["media"], "PLAYPAUSE")))
                self.lcd_dirty = True
            if "page" in spec:
                self.goto(spec["page"])
            if "profile" in spec:
                self.set_profile(spec["profile"])
            if "set_theme" in spec:
                self.set_theme(spec["set_theme"])
                return
            if "wallpaper_theme" in spec:
                self.apply_wallpaper_theme()
                return
            if "brightness" in spec:
                self.set_brightness(spec["brightness"])
        except (OSError, ValueError) as e:
            log(f"key {i}: {e}")
        if self.page == page and self.profile == profile and self.deck:
            self.draw_key(i, pressed=True)

    @staticmethod
    def sequence_of(spec):
        steps = gc.steps(spec["sequence"]) if "sequence" in spec else gc.mouse_steps(spec["mouse"])
        return steps, gc.codes(spec["hold"]) if spec.get("hold") else []

    def play_sequence(self, spec):
        """Typed in the background. A press while one is typing is dropped, except
        that the same key cancels one with a mouse or WAIT step; keys-only
        sequences always finish, so mashing a key can't cut a game combo short."""
        if self.typing.is_set():
            running = self.cancellable
            if running and running[0] == spec:
                running[1].set()
            return
        steps, hold = self.sequence_of(spec)
        stop = threading.Event() if any(isinstance(s, tuple) for s in steps) else None
        self.cancellable = (spec, stop) if stop else None
        self.typing.set()

        def typing():
            try:
                self.kb.type_sequence(steps, hold, spec.get("step_ms", 40), stop)
            except OSError as e:
                log(f"sequence: {e}")
            finally:
                self.cancellable = None
                self.typing.clear()
        threading.Thread(target=typing, daemon=True).start()

    def toggle_loop(self, i, spec):
        """The first press runs the sequence over and over; the next stops it."""
        if i in self.repeating:
            self.repeating.pop(i).set()
            return
        if self.typing.is_set():
            return
        steps, hold = self.sequence_of(spec)
        if not steps:
            return
        interval = max(0, min(10000, int(spec.get("repeat_ms", 100)))) / 1000
        stop = threading.Event()
        self.repeating[i] = stop
        self.typing.set()

        def loop():
            try:
                while not stop.is_set():
                    self.kb.type_sequence(steps, hold, spec.get("step_ms", 40), stop)
                    stop.wait(max(interval, MIN_REPEAT_GAP))
            except OSError as e:
                log(f"sequence: {e}")
            finally:
                if self.repeating.get(i) is stop:
                    self.repeating.pop(i, None)
                self.typing.clear()
        threading.Thread(target=loop, daemon=True).start()

    def toggle_repeat(self, i, spec):
        """The first press keeps tapping the key; the next stops it."""
        if i in self.repeating:
            self.repeating.pop(i).set()
            return
        codes = gc.codes(spec["repeat"])
        if str(spec["repeat"]).upper().startswith("KP"):
            self.kb.ensure_numlock()
        interval = max(0, min(10000, int(spec.get("repeat_ms", 100)))) / 1000
        stop = threading.Event()
        self.repeating[i] = stop

        def repeat():
            try:
                while not stop.is_set():
                    self.kb.press(codes)
                    if interval:  # games can miss a press and release in the same instant
                        time.sleep(0.02)
                    self.kb.release(codes)
                    stop.wait(max(interval, MIN_REPEAT_GAP))
            except OSError as e:
                log(f"repeat: {e}")
        threading.Thread(target=repeat, daemon=True).start()

    def confirm(self, i, proc, page, profile):
        """Flash the key with how its command went: a tick, or a cross."""
        ok = proc.wait() == 0
        time.sleep(0.15)
        th = self.theme
        spec = {"icon": ICON_DONE if ok else ICON_FAILED, "label": "done" if ok else "failed",
                "bg": th["accent"] if ok else gc.colour(th.get("frame", "#e0231a"), th), "fg": th["key_bg"],
                "border": th["key_bg"]}
        for _ in range(2):
            with self.lock:
                if not self.deck or (self.page, self.profile) != (page, profile):
                    return
                self.deck.key_image(i, self.draw.jpeg(self.draw.key(spec, th)))
            time.sleep(0.6)
            with self.lock:
                if self.deck and (self.page, self.profile) == (page, profile):
                    self.draw_key(i)
            time.sleep(0.2)

    def key_up(self, i):
        if i in self.held:
            self.kb.release(self.held.pop(i))
        if self.keys()[i] and self.deck:
            self.draw_key(i)

    def change_volume(self, clicks):
        self.system.change_volume(clicks * (self.cfg.get("volume_step", 2) if self.cfg else 2))
        self.volume = self.system.volume()
        self.volume_changed = time.time()

    def cycle_profile(self, direction):
        names = list(self.cfg["profiles"])
        self.set_profile(names[(names.index(self.profile) + direction) % len(names)])

    def dial_turn(self, action, clicks):
        if action == "volume":
            self.change_volume(clicks)
        elif action == "pages" and self.cfg:
            self.set_page(self.page + (1 if clicks > 0 else -1), remember=False)
        elif action == "profiles" and self.cfg:
            self.cycle_profile(1 if clicks > 0 else -1)

    def dial_press(self, action):
        if action == "mute":
            self.system.toggle_mute()
            self.volume = self.system.volume()
            self.volume_changed = time.time()
        elif action == "play-pause":
            if not self.system.media("play-pause"):
                self.kb.tap(gc.codes("PLAYPAUSE"))
        elif action == "start-page" and self.cfg:
            self.history.clear()
            self.set_page(self.start_page_index(), remember=False)

    def handle(self, data):
        report = parse_report(data)
        if not report:
            return
        if report[0] == "keys":
            states = report[1]
            for i, s in enumerate(states):
                if s and not self.key_state[i]:
                    self.key_down(i)
                elif not s and self.key_state[i]:
                    self.key_up(i)
            self.key_state = list(states)
            return
        dials = self.dials()
        if report[0] == "turn":
            for n, side in enumerate(("left", "right")):
                clicks = report[1 + n]
                if not clicks:
                    continue
                held = dials[f"{side}_hold_turn"]
                if self.dial_down[n] and held != "none":
                    self.dial_turned[n] = True
                    self.dial_turn(held, clicks)
                else:
                    self.dial_turn(dials[f"{side}_turn"], clicks)
        else:  # presses act on release, so press-and-turn works
            for n, side in enumerate(("left", "right")):
                down = report[1 + n]
                if down and not self.dial_down[n]:
                    self.dial_turned[n] = False
                elif not down and self.dial_down[n] and not self.dial_turned[n]:
                    self.dial_press(dials[f"{side}_press"])
                self.dial_down[n] = down
        self.lcd_dirty = True

    # lifecycle -------------------------------------------------------------
    def connect(self):
        try:
            transport = self.open_deck()
        except OSError as e:
            log(f"open: {e}")
            return False
        if not transport:
            return False
        self.deck = Deck(transport)
        try:
            self.deck.ping()
            self.key_state = [0] * KEYS
            self.dial_down = [False, False]
            self.last_lcd, self.lcd_dirty = None, True
            time.sleep(0.2)
            self.deck.brightness(self.cfg.get("brightness", 70) if self.cfg else 70)
            self.volume = self.system.volume()
            self.play_boot()
            self.draw_keys()
        except OSError as e:
            log(f"connect: {e}")
            self.disconnect()
            return False
        log("connected")
        return True

    def play_boot(self):
        """On sign-in and every replug. Any key or dial skips it."""
        boot = self.cfg.get("boot", {}) if self.cfg else {}
        if not boot.get("enabled", True):
            return
        blank = self.draw.jpeg(self.draw.key({}, self.theme))
        for i in range(KEYS):
            self.deck.key_image(i, blank)
        try:
            if boot.get("gif"):
                frames = self.draw.gif_frames(self.paths.resolve(boot["gif"]), self.theme)
            else:
                footer = f"DECK ONLINE // {self.profile.upper()}" if self.profile else "DECK ONLINE"
                logo = self.paths.resolve(boot["logo"]) if boot.get("logo") else None
                frames = self.draw.boot_frames(self.theme, boot, footer, self.key_image, logo)
            for img, keys, seconds in frames:
                due = time.time() + seconds
                self.deck.lcd_image(self.draw.jpeg(img))
                for i, k in keys.items():
                    if k == "real":
                        self.draw_key(i)
                    else:
                        self.deck.key_image(i, self.draw.jpeg(k))
                left = due - time.time()
                data = self.deck.read(left) if left > 0 else None
                if data and any(data[4:4 + KEYS]):
                    break
        except OSError as e:
            if not isinstance(e, BlockingIOError):
                raise
        except Exception as e:  # a broken GIF or logo mustn't stop the deck
            log(f"boot animation: {e}")
        while self.deck.read(0.05):
            pass  # what was pressed during the animation doesn't count
        self.last_lcd, self.lcd_dirty = None, True

    def disconnect(self):
        self.release_held()
        if self.deck:
            self.deck.close()
            self.deck = None

    def pinger(self):
        while self.running:
            deck = self.deck
            if deck:
                try:
                    deck.ping()
                except OSError:
                    pass  # the main loop notices and reconnects
            time.sleep(PING_INTERVAL)

    def tick(self):
        """Once a second: config, requests from Settings, volume, the top screen."""
        self.reload_if_changed()
        self.take_request()
        vol = self.system.volume()
        if vol != self.volume:
            self.volume, self.volume_changed = vol, time.time()
        if self.deck:
            self.draw_lcd()
        self.publish_state()

    def run(self, follow_focus=True):
        threading.Thread(target=self.pinger, daemon=True).start()
        if follow_focus:
            threading.Thread(target=focus_events, args=(self.on_focus, lambda: self.running), daemon=True).start()
        with self.lock:
            self.reload_if_changed()
        next_tick, next_check, ticks = 0.0, 0.0, 0
        while self.running:
            now = time.time()
            if not self.deck or now >= next_check:  # before every connect, and every 2 s while connected
                next_check = now + 2
                if not self.enabled():
                    break
                if self.blocked():
                    if self.deck:
                        log("galleon-deck is running: letting go of the deck")
                        with self.lock:
                            self.disconnect()
                    self.sleep(2)
                    continue
            if not self.deck:
                with self.lock:
                    connected = self.connect()
                if not connected:
                    self.sleep(2)
                    continue
            try:
                data, self.pending = self.pending or self.deck.read(max(0.0, min(0.1, next_tick - time.time()))), None
                with self.lock:
                    if data:
                        self.handle(data)
                    if time.time() >= next_tick:
                        next_tick = time.time() + 1
                        ticks += 1
                        if ticks % 10 == 0:
                            self.follow_wallpaper()
                        self.tick()
                    elif self.lcd_dirty and self.deck:
                        self.draw_lcd()
            except OSError as e:
                log(f"device error: {e}")
                with self.lock:
                    self.disconnect()
                self.sleep(1)
        self.running = False
        with self.lock:
            self.disconnect()


def serve(enabled):
    """Run the deck until `enabled()` turns false (nyx-deck's galleon setting)."""
    try:
        import galleon_draw
    except ImportError:
        log("the Galleon's deck needs python-pillow")
        time.sleep(60)
        return
    paths = gc.nyx_paths()
    app = App(paths, open_hidraw, Keyboard(), System(), galleon_draw, enabled=enabled)
    try:
        app.run()
    finally:
        app.kb.close()
