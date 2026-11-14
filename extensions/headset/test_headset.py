"""Tests for settings-headset. Run: python3 -m unittest test_headset.py"""

import importlib.machinery
import importlib.util
import os
import subprocess
import tempfile
import unittest
from unittest import mock

os.environ["SETTINGS_EXTENSION_STATE"] = tempfile.mkdtemp()
HERE = os.path.dirname(os.path.realpath(__file__))
loader = importlib.machinery.SourceFileLoader("headset", os.path.join(HERE, "settings-headset"))
spec = importlib.util.spec_from_loader("headset", loader)
h = importlib.util.module_from_spec(spec)
loader.exec_module(h)

KINDS = "info switch slider choice segmented buttons entry button colour chips curve disclosure camera meter preview icon"


def no_programs(*args, **kwargs):
    raise AssertionError(f"ran a program in pretend mode: {args}")


class Pretend(unittest.TestCase):
    """HEADSET_MOCK on, a fresh state folder, and nothing allowed to run."""
    mock_value = ""

    def setUp(self):
        env = {k: v for k, v in os.environ.items() if k not in ("SETTINGS_KINDS", "HEADSET_MOCK")}
        env["HEADSET_MOCK"] = self.mock_value
        patches = [mock.patch.dict(os.environ, env, clear=True), mock.patch.object(h, "STATE_DIR", tempfile.mkdtemp()),
                   mock.patch.object(h.subprocess, "run", no_programs), mock.patch.object(h.subprocess, "Popen", no_programs)]
        for p in patches:
            p.start()
            self.addCleanup(p.stop)

    def kinds(self):
        os.environ["SETTINGS_KINDS"] = KINDS

    def assert_icons(self, page):
        for g in page["groups"]:
            for r in g["rows"]:
                self.assertTrue(r.get("icon"), f"{g['title']}: {r.get('title')} has no icon")


class HeadsetControlPage(Pretend):
    mock_value = "arctis7"

    def test_old_page_without_settings_kinds(self):
        d = h.describe("1038-12ad")
        self.assertNotIn("preview", d)
        self.assertEqual([g["title"] for g in d["groups"]], ["Status", "Sound", "Headset"])
        self.assertFalse(any("icon" in r for g in d["groups"] for r in g["rows"]))

    def test_preview_with_settings_kinds(self):
        self.kinds()
        d = h.describe("1038-12ad")
        self.assertEqual([g["title"] for g in d["groups"]], ["Sound", "Headset"])
        self.assert_icons(d)
        blocks = d["preview"]["blocks"]
        self.assertEqual([b["kind"] for b in blocks], ["picture", "stats", "rows"])
        battery, chat, off = blocks[1]["items"]
        self.assertEqual((battery["value"], battery["sub"], battery["tone"]), ("64%", "About 15 hours", "good"))
        self.assertEqual((chat["value"], off["value"]), ("More game", "After 30 min"))
        self.assertEqual(blocks[2]["rows"][0]["key"], "battery-warning")
        self.assertTrue(blocks[2]["rows"][0]["icon"])

    def test_set_changes_only_the_pretend_state(self):
        self.kinds()
        h.set_value("1038-12ad", "sidetone", "70")
        h.set_value("1038-12ad", "battery-warning", "false")
        d = h.describe("1038-12ad")
        self.assertEqual(d["groups"][0]["rows"][0]["value"], 70)
        self.assertFalse(d["preview"]["blocks"][2]["rows"][0]["value"])
        self.assertTrue(os.path.exists(os.path.join(h.STATE_DIR, "mock", "1038-12ad.json")))


class A50X(Pretend):
    mock_value = "a50x"

    def test_page_listed(self):
        self.assertEqual([p["id"] for p in h.pages()], ["a50x"])

    def test_preview_with_settings_kinds(self):
        self.kinds()
        d = h.describe("a50x")
        self.assert_icons(d)
        self.assertEqual([g["title"] for g in d["groups"]],
                         ["Sound", "Microphone", "Equalizers", "Stream mix", "Headset and base station"])
        blocks = {b["kind"]: b for b in d["preview"]["blocks"]}
        self.assertEqual(blocks["picture"]["picture"], "headset")
        self.assertEqual([i["label"] for i in blocks["stats"]["items"]], ["Battery", "Balance", "Boom"])
        self.assertEqual([i["label"] for i in blocks["list"]["items"]], ["A50 X Game", "A50 X Chat", "A50 X Microphone"])
        self.assertIn("base station", blocks["banner"]["text"])
        self.assertIn("Logitech G device", d["preview"]["foot"])

    def test_no_preview_without_settings_kinds(self):
        d = h.describe("a50x")
        self.assertNotIn("preview", d)
        self.assertEqual(d["groups"][0]["title"], "Status")

    def test_set_goes_through_gearbench_ctl(self):
        self.kinds()
        h.set_value("a50x", "balance", "3")
        h.set_value("a50x", "eq-headphones", "media")
        self.assertEqual(h.set_value("a50x", "muted", "false,true,false,false"), {"refresh": True})
        a = h.parse_audio(h.gearbench("audio"))
        self.assertEqual((a["balance"], a["eq"]["headphones"], a["muted"]), (3, "media", {"voice"}))
        with self.assertRaises(SystemExit):
            h.set_value("a50x", "sidetone", "9")

    def test_open_gearbench_runs_nothing(self):
        self.assertIn("toast", h.set_value("a50x", "open-gearbench", ""))


class A50XWithoutGearbench(Pretend):
    mock_value = "a50x-no-gearbench"

    def test_setup_page(self):
        self.kinds()
        d = h.describe("a50x")
        self.assertIn("Gearbench", d["banners"][0]["text"])
        self.assertEqual([r["key"] for g in d["groups"] for r in g["rows"]], ["install-gearbench"])
        self.assert_icons(d)
        self.assertEqual(d["preview"]["title"], "A50 X found")
        self.assertIn("toast", h.set_value("a50x", "install-gearbench", ""))


class A50XDetection(unittest.TestCase):
    def usb(self, product):
        d = tempfile.mkdtemp()
        for name, value in (("idVendor", "046d"), ("idProduct", product)):
            with open(os.path.join(d, name), "w") as f:
                f.write(value + "\n")
        return d

    def test_only_the_a50x_usb_id(self):
        with mock.patch.dict(os.environ, {"HEADSET_MOCK": ""}), mock.patch.object(h, "devices", lambda: []):
            with mock.patch.object(h.glob, "glob", lambda _: [self.usb("0b0b")]):
                self.assertEqual([p["id"] for p in h.pages()], ["a50x"])
            with mock.patch.object(h.glob, "glob", lambda _: [self.usb("0b1c")]):
                self.assertEqual(h.pages(), [])


class Parsing(unittest.TestCase):
    def test_not_read_values(self):
        a = h.parse_audio({"volume": "not read", "settings": {"tones": "All", "auto-sleep": "never"},
                           "stream_mix": {"game": 10, "muted": ["mic"]}, "equalizers": {"headphones": "Custom"}})
        self.assertIsNone(a["volume"])
        self.assertEqual((a["tones"], a["auto_sleep"], a["eq"]["headphones"]), ("all", "never", "custom"))
        self.assertEqual((a["mix"]["game"], a["mix"]["voice"], a["muted"]), (10, None, {"mic"}))
        row = h.control({"kind": "slider", "key": "volume", "min": 0, "max": 21}, a["volume"])
        self.assertTrue(row["disabled"])

    def test_problems(self):
        self.assertEqual(h.problems({"problems": ["none"]}), [])
        self.assertEqual(h.problems({"problems": [{"keyboard": None, "message": "another program drives it"}]}),
                         ["another program drives it"])


class LowBattery(unittest.TestCase):
    def test_warns_once_then_again_after_charging(self):
        sent = []
        level = {"v": 10}
        devs = lambda: [{"device": "Arctis", "product": "Arctis 7", "id_vendor": "0x1038", "id_product": "0x12ad",  # noqa: E731
                         "battery": {"status": "BATTERY_AVAILABLE", "level": level["v"]}}]
        with mock.patch.object(h.subprocess, "run", lambda argv, **kw: sent.append(argv)), \
                mock.patch.object(h, "devices", devs), mock.patch.dict(os.environ, {"HEADSET_MOCK": ""}):
            h.battery_check()
            h.battery_check()
            self.assertEqual(len([a for a in sent if a[0] == "notify-send"]), 1)
            level["v"] = 90
            h.battery_check()
            level["v"] = 12
            h.battery_check()
            self.assertEqual(len([a for a in sent if a[0] == "notify-send"]), 2)


if __name__ == "__main__":
    unittest.main()
