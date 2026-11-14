"""Tests for the Galleon deck driver (galleon.py, galleon_config.py, galleon_draw.py)
against a simulated deck: nothing opens hidraw or uinput.
Run: python3 -m unittest test_galleon.py (the drawing tests need Pillow)."""

import os
import shutil
import struct
import sys
import tempfile
import threading
import time
import unittest

HERE = os.path.dirname(os.path.realpath(__file__))
sys.path.insert(0, HERE)
import galleon  # noqa: E402
import galleon_config as gc  # noqa: E402

SAMPLE = os.path.join(HERE, "testdata", "galleon-deck")
KP7, CTRL, S, W, D, ENTER, SPACE, NUMLOCK = 71, 29, 31, 17, 32, 28, 57, 69

try:
    import PIL  # noqa: F401
    HAVE_PIL = True
except ImportError:
    HAVE_PIL = False


class SimDeck:
    """A pretend Galleon on the other end of the transport: it keeps the reports
    sent to it, puts the key images and the screen back together, and hands out
    the key and dial reports queued with press(), turn() and push()."""

    def __init__(self):
        self.features = []
        self.packets = []
        self.inbox = []
        self.keys = {}
        self.screens = []
        self.parts = {}
        self.closed = False

    def feature(self, payload):
        self.features.append(bytes(payload))

    def write(self, pkt):
        assert len(pkt) == galleon.PACKET, len(pkt)
        self.packets.append(pkt)
        if pkt[:2] == b"\x02\x07":
            index, last = pkt[2], pkt[3]
            length = int.from_bytes(pkt[4:6], "little")
            part = int.from_bytes(pkt[6:8], "little")
            assert part == len(self.parts.get(index, [])), "parts arrive in order"
            self.parts.setdefault(index, []).append(pkt[8:8 + length])
            if last:
                self.keys[index] = b"".join(self.parts.pop(index))
        elif pkt[:2] == b"\x02\x0c":
            assert struct.unpack("<HHHH", pkt[2:10]) == (0, 0, 720, 384)
            last, part, length = pkt[10], int.from_bytes(pkt[11:13], "little"), int.from_bytes(pkt[13:15], "little")
            assert part == len(self.parts.get("lcd", []))
            self.parts.setdefault("lcd", []).append(pkt[16:16 + length])
            if last:
                self.screens.append(b"".join(self.parts.pop("lcd")))
        else:
            raise AssertionError(f"unknown output report {pkt[:4].hex()}")

    def read(self, timeout):
        if self.inbox:
            return self.inbox.pop(0)
        time.sleep(min(timeout, 0.1))
        return None

    def close(self):
        self.closed = True

    def press(self, *down):
        self.inbox.append(bytes([1, 0, 0x0C, 0]) + bytes(1 if i in down else 0 for i in range(12)) + bytes(48))

    def turn(self, left=0, right=0):
        self.inbox.append(bytes([1, 3, 3, 0, 1, left & 0xFF, right & 0xFF]) + bytes(57))

    def push(self, left=False, right=False):
        self.inbox.append(bytes([1, 3, 3, 0, 0, int(left), int(right)]) + bytes(57))

    def label(self, index):
        return eval(self.keys[index].decode())[1]


class FakeUi:
    def __init__(self, log):
        self.log = log

    def write(self, kind, code, value):
        self.log.append((kind, code, value))

    def syn(self):
        pass

    def close(self):
        pass


class FakeProc:
    def __init__(self, code):
        self.code = code

    def wait(self):
        return self.code


class FakeSystem:
    def __init__(self):
        self.calls = []
        self.vol = (40, False)

    def volume(self):
        return self.vol

    def change_volume(self, pct):
        self.calls.append(("volume", pct))
        self.vol = (self.vol[0] + pct, self.vol[1])

    def toggle_mute(self):
        self.calls.append(("mute",))

    def media(self, action):
        self.calls.append(("media", action))
        return True

    def now_playing(self):
        return "Playing", "M83 — Midnight City"

    def run(self, cmd):
        self.calls.append(("run", cmd))
        return FakeProc(0)


class FakeDraw:
    """galleon_draw without Pillow: pictures are tuples, and a JPEG is their repr."""

    def __init__(self):
        self.states = []

    def jpeg(self, img):
        return repr(img).encode()

    def key(self, spec, theme, pressed=False, paths=None):
        return ("key", spec.get("label", ""), pressed)

    def theme_key(self, name, theme, current):
        return ("theme", name, current)

    def invert(self, img):
        return ("inverted", img)

    def lcd(self, state, theme):
        self.states.append(state)
        return ("lcd", state["profile"], state["page"])

    def load_logo(self, path, theme, fallback):
        return fallback

    def glitch(self, img, rng, amount):
        return img

    def screen_logo(self, theme, logo, done):
        return ("logo", logo, done)

    def boot_frames(self, theme, boot, footer, real_keys, logo):
        yield ("boot", footer), {0: ("cipher", "", False)}, 0
        yield ("boot", "done"), {i: "real" for i in range(12)}, 0

    def gif_frames(self, path, theme):
        yield ("gif", path), {}, 0

    def wallpaper_path(self, settings):
        return None

    def write_wallpaper_theme(self, paths, settings, image=None):
        gc.write_atomic(os.path.join(paths.themes, "wallpaper.toml"), 'fg = "#ffffff"\n')
        return os.path.join(paths.themes, "wallpaper.toml")


def keys_down(log):
    return [(c, v) for kind, c, v in log if kind == galleon.EV_KEY]


class Driver(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        shutil.copytree(SAMPLE, os.path.join(self.tmp, "galleon"))
        self.paths = gc.Paths(os.path.join(self.tmp, "galleon"))
        self.sim = SimDeck()
        self.log = []
        kb = galleon.Keyboard(make_keyboard=lambda: FakeUi(self.log), make_mouse=lambda: FakeUi(self.log))
        kb.numlock_checked = True
        self.system = FakeSystem()
        self.draw = FakeDraw()
        self.opened = 0

        def open_deck():
            self.opened += 1
            return self.sim
        self.app = galleon.App(self.paths, open_deck, kb, self.system, self.draw, blocked=lambda: False, publish=False)
        self.app.reload_if_changed()

    def tearDown(self):
        self.app.release_held()
        shutil.rmtree(self.tmp)

    def connect(self):
        self.assertTrue(self.app.connect())
        self.sim.features.clear()

    def feed(self):
        """Handle the queued reports, as the main loop does (with the one a
        profile-switch animation set aside first)."""
        while self.sim.inbox or self.app.pending:
            data, self.app.pending = self.app.pending or self.sim.inbox.pop(0), None
            self.app.handle(data)

    def wait_typing(self):
        for _ in range(200):
            if not self.app.typing.is_set():
                return
            time.sleep(0.01)
        self.fail("the sequence didn't finish")

    def test_connect_pings_then_draws(self):
        self.assertTrue(self.app.connect())
        self.assertEqual(self.sim.features[:2], [bytes([3, 0x27]), bytes([3, 8, 70])])
        self.assertEqual(sorted(self.sim.keys), list(range(12)))
        self.assertEqual(self.sim.label(0), "7")
        self.assertEqual(self.sim.screens[0], repr(("boot", "DECK ONLINE // DESKTOP")).encode())

    def test_keep_alive_every_half_second(self):
        self.connect()
        t = threading.Thread(target=self.app.pinger, daemon=True)
        t.start()
        time.sleep(1.2)
        self.app.running = False
        t.join(1)
        pings = [f for f in self.sim.features if f == galleon.PING]
        self.assertIn(len(pings), (2, 3))

    def test_numpad_key_is_held_while_held(self):
        self.connect()
        self.sim.press(0)
        self.feed()
        self.assertEqual(keys_down(self.log), [(KP7, 1)])
        self.assertEqual(self.sim.keys[0], repr(("key", "7", True)).encode())
        self.sim.press()
        self.feed()
        self.assertEqual(keys_down(self.log), [(KP7, 1), (KP7, 0)])

    def test_page_and_back(self):
        self.connect()
        self.app.goto("apps")
        self.assertEqual(self.sim.label(0), "Terminal")
        self.sim.press(11)  # back
        self.feed()
        self.assertEqual(self.app.pages()[self.app.page]["name"], "numpad")

    def test_command_with_confirm_flash(self):
        self.app.set_profile("gaming")
        self.connect()
        self.sim.press(3)  # Shot: confirm = true
        self.feed()
        self.assertEqual(self.system.calls[-1], ("run", 'grim "$HOME/shot.png"'))
        time.sleep(0.4)
        self.assertEqual(self.sim.label(3), "done")

    def test_media(self):
        self.app.set_profile("gaming")
        self.connect()
        self.sim.press(6)
        self.feed()
        self.assertIn(("media", "play-pause"), self.system.calls)

    def test_sequence_holds_ctrl_throughout(self):
        self.app.set_profile("gaming")
        self.connect()
        self.sim.press(4)  # Resupply: hold CTRL, S S W D
        self.feed()
        self.wait_typing()
        steps = [(c, v) for c, v in keys_down(self.log)]
        self.assertEqual(steps[0], (CTRL, 1))
        self.assertEqual(steps[-1], (CTRL, 0))
        self.assertEqual([c for c, v in steps[1:-1] if v == 1], [S, S, W, D])

    def test_multiline_sequence_with_notes(self):
        self.app.set_profile("gaming")
        self.connect()
        self.sim.press(10)  # Paste next: CTRL+V, DOWN, with notes
        self.feed()
        self.wait_typing()
        self.assertEqual([c for c, v in keys_down(self.log) if v == 1], [CTRL, 47, 108])

    def test_repeat_toggles(self):
        self.app.set_profile("gaming")
        self.connect()
        self.sim.press(9)  # Auto jump: repeat SPACE every 250 ms
        self.sim.press()
        self.feed()
        time.sleep(0.4)
        self.sim.press(9)
        self.sim.press()
        self.feed()
        time.sleep(0.1)  # a tap already under way finishes
        taps = [v for c, v in keys_down(self.log) if c == SPACE]
        self.assertGreaterEqual(taps.count(1), 2)
        time.sleep(0.4)
        self.assertEqual(taps, [v for c, v in keys_down(self.log) if c == SPACE], "stopped")

    def test_mouse_loop(self):
        self.app.set_profile("gaming")
        self.connect()
        self.sim.press(5)  # Autoclick: CLICK, loop, every 200 ms
        self.sim.press()
        self.feed()
        time.sleep(0.5)
        self.sim.press(5)
        self.sim.press()
        self.feed()
        self.wait_typing()
        self.assertGreaterEqual(keys_down(self.log).count((gc.BTN_LEFT, 1)), 2)

    def test_dials(self):
        self.connect()
        self.sim.turn(left=2)
        self.feed()
        self.assertEqual(self.system.calls[-1], ("volume", 4))  # 2 clicks × volume_step 2
        self.sim.push(left=True)
        self.sim.push()
        self.feed()
        self.assertEqual(self.system.calls[-1], ("mute",))
        self.sim.turn(right=1)
        self.feed()
        self.assertEqual(self.app.pages()[self.app.page]["name"], "numpad ops")
        self.sim.push(right=True)
        self.sim.push()
        self.feed()
        self.assertEqual(self.app.pages()[self.app.page]["name"], "numpad", "press: the start page")

    def test_hold_and_turn_switches_profile(self):
        self.connect()
        self.sim.push(right=True)
        self.sim.turn(right=1)
        self.sim.push()
        self.feed()
        self.assertEqual(self.app.profile, "gaming")
        self.assertEqual(self.app.pages()[self.app.page]["name"], "game", "no start-page jump on release")

    def test_dials_follow_the_config(self):
        gc.set_value(self.paths.config, "dials", "left_turn", "pages")
        self.app.reload_if_changed()
        self.connect()
        self.sim.turn(left=1)
        self.feed()
        self.assertEqual(self.app.page, 1)
        self.assertNotIn(("volume", 2), self.system.calls)

    def test_settings_page_sets_a_theme(self):
        self.connect()
        self.app.set_page(len(self.app.pages()) - 1)
        self.assertEqual(self.sim.keys[0], repr(("theme", "amber", True)).encode())
        self.sim.press(1)  # cyberpunk
        self.feed()
        self.assertEqual(gc.read_toml(self.paths.profile("desktop"))["theme"], "cyberpunk")
        with open(self.paths.profile("desktop")) as f:
            self.assertIn("# Desktop profile", f.read())

    def test_auto_switch(self):
        self.app.on_focus("steam_app_570", "Dota 2")
        self.assertEqual(self.app.profile, "gaming")
        self.app.on_focus("firefox", "Mozilla Firefox")
        self.assertEqual(self.app.profile, "desktop", "the fallback")
        gc.set_value(self.paths.config, "auto_switch", "enabled", False)
        self.app.reload_if_changed()
        self.app.on_focus("gamescope", "")
        self.assertEqual(self.app.profile, "desktop")

    def test_top_screen(self):
        self.connect()
        self.app.draw_lcd()
        state = self.draw.states[-1]
        self.assertEqual((state["profile"], state["page"], state["page_count"]), ("desktop", "numpad", 5))
        self.assertEqual((state["media"], state["volume"]), ("M83 — Midnight City", (40, False)))
        gc.set_value(self.paths.config, "screen", "playing", False)
        gc.set_value(self.paths.config, "screen", "volume", "changing")
        self.app.reload_if_changed()
        self.app.volume_changed = 0
        self.assertNotIn("media", self.app.lcd_state())
        self.assertNotIn("volume", self.app.lcd_state())
        self.sim.turn(left=1)
        self.feed()
        self.assertEqual(self.app.lcd_state()["volume"], (42, False))

    def test_a_broken_config_shows_on_the_screen(self):
        with open(self.paths.profile("desktop"), "a") as f:
            f.write("\n[[page]]\nname = \"x\"\nkeys = [{ key = \"NOSUCHKEY\" }]\n")
        self.app.reload_if_changed()
        self.assertIn("NOSUCHKEY", self.app.lcd_state()["error"])
        self.assertEqual(self.app.profile, "desktop", "it keeps the last good config")

    def test_settings_asks_for_a_profile(self):
        saved = os.environ.get("XDG_RUNTIME_DIR")
        os.environ["XDG_RUNTIME_DIR"] = self.tmp
        try:
            self.app.publish = True
            self.app.tick()
            self.assertEqual(gc.read_json(gc.state_file()), {"profile": "desktop", "page": "numpad"})
            gc.write_json(gc.request_file(), {"profile": "gaming"})
            self.app.tick()
            self.assertEqual(self.app.profile, "gaming")
            self.assertFalse(os.path.exists(gc.request_file()))
        finally:
            if saved is None:
                os.environ.pop("XDG_RUNTIME_DIR")
            else:
                os.environ["XDG_RUNTIME_DIR"] = saved

    def test_never_opens_the_deck_while_galleon_deck_runs(self):
        self.app.blocked = lambda: True
        checks = []
        self.app.enabled = lambda: checks.append(1) or len(checks) < 5
        self.app.sleep = lambda s: None
        self.app.run(follow_focus=False)
        self.assertEqual(self.opened, 0)

    def test_lets_go_when_galleon_deck_starts(self):
        checks = []
        self.app.blocked = lambda: checks.append(1) or len(checks) > 1  # it starts after the first check
        self.app.enabled = lambda: len(checks) < 3
        self.app.sleep = lambda s: None
        self.app.run(follow_focus=False)
        self.assertTrue(self.sim.closed)
        self.assertEqual(self.opened, 1, "opened once, and not again while galleon-deck runs")

    def test_turned_off_stops_and_lets_go(self):
        self.app.enabled = lambda: False
        self.app.connect()
        self.app.run(follow_focus=False)
        self.assertTrue(self.sim.closed)
        self.assertIsNone(self.app.deck)


class Protocol(unittest.TestCase):
    def test_key_image_packets(self):
        jpeg = bytes(range(256)) * 10  # 2560 bytes: 1016 + 1016 + 528
        packets = galleon.key_packets(5, jpeg)
        self.assertEqual([len(p) for p in packets], [1024] * 3)
        self.assertEqual(packets[0][:8], bytes([2, 7, 5, 0]) + (1016).to_bytes(2, "little") + (0).to_bytes(2, "little"))
        self.assertEqual(packets[2][:8], bytes([2, 7, 5, 1]) + (528).to_bytes(2, "little") + (2).to_bytes(2, "little"))
        self.assertEqual(b"".join(p[8:8 + int.from_bytes(p[4:6], "little")] for p in packets), jpeg)

    def test_screen_packets(self):
        packets = galleon.lcd_packets(b"x" * 2000)
        self.assertEqual(packets[0][:16], bytes([2, 0x0C, 0, 0, 0, 0, 0xD0, 2, 0x80, 1, 0, 0, 0, 0xF0, 3, 0]))
        self.assertEqual(packets[1][10:15], bytes([1, 1, 0, 0xE0, 3]))

    def test_feature_reports(self):
        self.assertEqual(galleon.PING, b"\x03\x27")
        self.assertEqual(galleon.brightness_report(150), b"\x03\x08\x64")

    def test_input_reports(self):
        self.assertEqual(galleon.parse_report(bytes([1, 0, 12, 0] + [0, 1] + [0] * 10)), ("keys", [0, 1] + [0] * 10))
        self.assertEqual(galleon.parse_report(bytes([1, 3, 3, 0, 1, 0xFE, 3])), ("turn", -2, 3))
        self.assertEqual(galleon.parse_report(bytes([1, 3, 3, 0, 0, 1, 0])), ("press", True, False))
        self.assertIsNone(galleon.parse_report(b"\x02\x00\x00\x00\x00"))

    def test_uinput_structs(self):
        self.assertEqual(struct.calcsize("HHHH80sI"), 92)
        self.assertEqual(galleon.UI_DEV_SETUP, 0x40000000 | (92 << 16) | (ord("U") << 8) | 3)
        self.assertEqual(struct.calcsize("llHHi"), 24)


class Config(unittest.TestCase):
    def test_galleon_deck_folder_loads_as_it_is(self):
        cfg = gc.load_config(gc.Paths(SAMPLE))
        self.assertEqual(sorted(cfg["profiles"]), ["desktop", "gaming"])
        self.assertEqual(cfg["profiles"]["gaming"]["theme_name"], "hud")
        self.assertEqual(cfg["profiles"]["desktop"]["page"][-1]["name"], "settings")

    def test_profiles_write_back_the_same(self):
        tmp = tempfile.mkdtemp()
        try:
            for name in ("desktop", "gaming"):
                before = gc.read_toml(os.path.join(SAMPLE, "profiles", name + ".toml"))
                out = os.path.join(tmp, name + ".toml")
                shutil.copy(os.path.join(SAMPLE, "profiles", name + ".toml"), out)
                gc.write_profile(out, before)
                after = gc.read_toml(out)
                for page in before["page"]:
                    while page["keys"] and not page["keys"][-1]:
                        page["keys"].pop()
                self.assertEqual(after, before)
                with open(out) as f:
                    self.assertTrue(f.read().startswith("# "), "the leading comment stays")
        finally:
            shutil.rmtree(tmp)

    def test_set_value_keeps_comments(self):
        tmp = tempfile.mkdtemp()
        try:
            path = os.path.join(tmp, "config.toml")
            shutil.copy(os.path.join(SAMPLE, "config.toml"), path)
            gc.set_value(path, None, "volume_step", 5)
            gc.set_value(path, "screen", "volume", "changing")
            with open(path) as f:
                text = f.read()
            self.assertIn("volume_step = 5 # percent per dial click", text)
            cfg = gc.read_toml(path)
            self.assertEqual((cfg["volume_step"], cfg["screen"]["volume"], cfg["boot"]["title"]), (5, "changing", "GALLEON DECK"))
        finally:
            shutil.rmtree(tmp)

    def test_key_names_and_steps(self):
        self.assertEqual(gc.codes("ctrl+shift+t"), [29, 42, 20])
        self.assertEqual(gc.steps("RIGHTCLICK WAIT:200 MOVE:10,-5 SCROLL:-3 CTRL+V"),
                         [("click", gc.BTN_RIGHT, 1), ("wait", 200), ("move", 10, -5), ("scroll", -3), [29, 47]])
        with self.assertRaises(ValueError):
            gc.codes("HYPER")
        with self.assertRaises(ValueError):
            gc.mouse_steps("CLICK A")
        with self.assertRaises(ValueError):
            gc.steps("WAIT:999999")

    def test_built_in_key_names(self):
        saved, gc._key_codes = gc._key_codes, None
        header, gc.HEADER = gc.HEADER, "/nonexistent"
        try:
            import builtins
            real_import = builtins.__import__

            def no_evdev(name, *a, **k):
                if name == "evdev":
                    raise ImportError
                return real_import(name, *a, **k)
            builtins.__import__ = no_evdev
            try:
                self.assertEqual(gc.codes("KPENTER+NUMLOCK+F24"), [96, 69, 194])
            finally:
                builtins.__import__ = real_import
        finally:
            gc._key_codes, gc.HEADER = saved, header

    def test_off_unless_turned_on(self):
        tmp = tempfile.mkdtemp()
        try:
            path = os.path.join(tmp, "config.toml")
            self.assertFalse(gc.galleon_enabled(path))
            gc.set_value(path, None, "brightness", 60)
            self.assertFalse(gc.galleon_enabled(path))
            gc.set_value(path, None, "galleon", True)
            self.assertTrue(gc.galleon_enabled(path))
        finally:
            shutil.rmtree(tmp)


@unittest.skipUnless(HAVE_PIL, "drawing needs Pillow")
class Drawing(unittest.TestCase):
    def setUp(self):
        import galleon_draw
        self.d = galleon_draw
        self.theme = {**gc.THEME_DEFAULTS, **gc.THEMES["synthwave"]}

    def test_pictures_fit_the_deck(self):
        self.assertEqual(self.d.key({"label": "Files", "icon": chr(0xF024B)}, self.theme).size, (160, 160))
        self.assertEqual(self.d.key({"label": "7"}, {**self.theme, "key_style": "hud"}, pressed=True).size, (160, 160))
        state = {"profile": "Work", "page": "Page 1", "page_index": 0, "page_count": 3, "clock": "14:32",
                 "date": "FRIDAY 13 NOVEMBER", "media": "M83 — Midnight City", "media_icon": "", "volume": (45, False)}
        img = self.d.lcd(state, self.theme)
        self.assertEqual(img.size, (720, 384))
        self.assertEqual(self.d.jpeg(img)[:2], b"\xff\xd8")

    def test_boot_animation_ends_with_every_key(self):
        done = set()
        for screen, keys, seconds in self.d.boot_frames(self.theme, {"speed": 4.0}, "DECK ONLINE",
                                                        lambda i: self.d.key({"label": str(i)}, self.theme)):
            self.assertEqual(screen.size, (720, 384))
            done |= {i for i, k in keys.items() if k == "real"}
        self.assertEqual(done, set(range(12)))

    def test_the_driver_with_real_pictures(self):
        tmp = tempfile.mkdtemp()
        try:
            shutil.copytree(SAMPLE, os.path.join(tmp, "galleon"))
            sim = SimDeck()
            kb = galleon.Keyboard(make_keyboard=lambda: FakeUi([]), make_mouse=lambda: FakeUi([]))
            app = galleon.App(gc.Paths(os.path.join(tmp, "galleon")), lambda: sim, kb, FakeSystem(), self.d,
                              blocked=lambda: False, publish=False)
            app.reload_if_changed()
            self.assertTrue(app.connect())
            app.set_profile("gaming")
            app.set_page(len(app.pages()) - 1)
            app.draw_lcd()
            self.assertEqual(sorted(sim.keys), list(range(12)))
            self.assertTrue(all(j[:2] == b"\xff\xd8" for j in list(sim.keys.values()) + sim.screens))
            self.assertGreater(len(sim.screens), 10, "boot animation and the profile switch")
        finally:
            shutil.rmtree(tmp)

    def test_theme_from_an_image(self):
        from PIL import Image
        tmp = tempfile.mkdtemp()
        try:
            path = os.path.join(tmp, "wall.png")
            img = Image.new("RGB", (64, 64), "#203060")
            img.paste(Image.new("RGB", (32, 64), "#ff6020"), (0, 0))
            img.save(path)
            out = self.d.write_wallpaper_theme(gc.Paths(tmp), {}, path)
            theme = gc.read_toml(out)
            self.assertRegex(theme["fg"], r"^#[0-9a-f]{6}$")
            self.assertEqual(theme["source"], path)
        finally:
            shutil.rmtree(tmp)


if __name__ == "__main__":
    unittest.main()
