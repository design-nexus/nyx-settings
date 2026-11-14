"""Tests for the Webcams extension: python3 test_webcam.py"""

import importlib.machinery
import importlib.util
import os
import shutil
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TMP = tempfile.mkdtemp()
os.environ["XDG_CONFIG_HOME"] = TMP
os.environ["SETTINGS_KINDS"] = "info switch camera meter"

import obsbot  # noqa: E402
import webcam_mock  # noqa: E402

loader = importlib.machinery.SourceFileLoader("webcam", os.path.join(HERE, "settings-webcam"))
spec = importlib.util.spec_from_loader("webcam", loader)
webcam = importlib.util.module_from_spec(spec)
loader.exec_module(webcam)


class Obsbot(unittest.TestCase):
    def test_crc(self):
        self.assertEqual(obsbot.crc16_usb(b"123456789"), 0xB4C8)  # CRC-16/USB check value
        self.assertEqual(obsbot.crc16_usb(bytes([0xAA, 0x25, 0xA5, 0, 0x0C, 0, 0, 0, 0x0A, 0x02, 0xC2, 0xA0])), 0xEF5F)

    def test_frames_match_captures(self):
        # Frames OBSBOT's own software sends, as captured by Tiny4Linux / obsbot-tiny3-linux.
        wake = obsbot.frame(obsbot.FLAG_SET, 0x00A5, obsbot.RCV_CAMERA, obsbot.CMD_POWER, bytes(4))
        self.assertEqual(wake[:20].hex(), "aa25a5000c005fef0a02c2a00400be0700000000")
        sleep = obsbot.frame(obsbot.FLAG_SET, 0x0042, obsbot.RCV_CAMERA, obsbot.CMD_POWER, bytes([1, 0, 0, 0]))
        self.assertEqual(sleep[:20].hex(), "aa2542000c00ea630a02c2a00400bffb01000000")
        self.assertEqual(len(wake), 60)
        self.assertFalse(any(wake[20:]))

    def test_status(self):
        block = bytearray(60)
        block[:14] = bytes([0x2E, 1, 0, 2, 0, 0, 0, 1, 0, 1, 120, 0, 0, 1])
        self.assertEqual(obsbot.parse_status(bytes(block)), (False, 120))
        block[2] = 1
        block[10:12] = (600).to_bytes(2, "little")
        self.assertEqual(obsbot.parse_status(bytes(block)), (True, 600))


class Options(unittest.TestCase):
    def setUp(self):
        shutil.rmtree(os.path.join(TMP, "settings"), ignore_errors=True)

    def test_round_trip(self):
        webcam.save_options({"mic:alsa_input.usb-Remo_OBSBOT": {"default": True, "no_suspend": False},
                             "camera:usb-obsbot-video-index0": {"keep_awake": True}})
        opts = webcam.load_options()
        self.assertTrue(webcam.option(opts, "mic:alsa_input.usb-Remo_OBSBOT", "default"))
        self.assertFalse(webcam.option(opts, "mic:alsa_input.usb-Remo_OBSBOT", "no_suspend"))
        self.assertTrue(webcam.guard_wanted(opts))
        self.assertFalse(webcam.guard_wanted({"mic:x": {"no_suspend": True}}), "a rule file needs no service")

    def test_wireplumber_rule(self):
        r = webcam.wireplumber_rule(["alsa_input.usb-A", "alsa_input.usb-B"])
        self.assertIn('node.name = "alsa_input.usb-A"', r)
        self.assertIn('node.name = "alsa_input.usb-B"', r)
        self.assertIn("session.suspend-timeout-seconds = 0", r)
        self.assertTrue(r.startswith("## Written by the Settings Webcams extension"))


class Mics(unittest.TestCase):
    OBJECTS = [
        {"id": 50, "type": "PipeWire:Interface:Device",
         "info": {"props": {"device.vendor.id": "0x3564", "device.product.id": "0xff04"}}},
        {"id": 51, "type": "PipeWire:Interface:Device",
         "info": {"props": {"device.vendor.id": "0x8086", "device.product.id": "0x7728"}}},
        {"id": 90, "type": "PipeWire:Interface:Node",
         "info": {"props": {"media.class": "Audio/Source", "device.id": 50,
                            "node.name": "alsa_input.usb-Remo_Tech_OBSBOT_Tiny_3_Lite-02.analog-stereo",
                            "node.description": "OBSBOT Tiny 3 Lite Analog Stereo"}}},
        {"id": 91, "type": "PipeWire:Interface:Node",
         "info": {"props": {"media.class": "Audio/Source", "device.id": 51, "node.name": "alsa_input.pci"}}},
        {"id": 92, "type": "PipeWire:Interface:Node",
         "info": {"props": {"media.class": "Audio/Sink", "device.id": 50, "node.name": "alsa_output.usb-Remo"}}},
    ]

    def test_finds_the_cameras_mic(self):
        m = webcam.mics_for(("3564", "ff04"), self.OBJECTS)
        self.assertEqual([x[0] for x in m], [90])
        self.assertEqual(webcam.mics_for(("1234", "5678"), self.OBJECTS), [])
        self.assertEqual(webcam.mics_for(None, self.OBJECTS), [])


class Preview(unittest.TestCase):
    def test_in_page_or_window(self):
        self.assertEqual(webcam.preview_group("/dev/video0")["rows"][0]["kind"], "camera")
        old = webcam.KINDS
        webcam.KINDS = set()
        try:
            self.assertEqual(webcam.preview_group("/dev/video0")["rows"][0]["key"], "@preview-window")
        finally:
            webcam.KINDS = old


PANE_KINDS = set("info switch slider choice segmented buttons entry button colour chips curve disclosure camera meter preview icon".split())


class Pretend(unittest.TestCase):
    """The pages against WEBCAM_MOCK=obsbot-tiny-3-lite."""

    def setUp(self):
        self.state = tempfile.mkdtemp()
        self.env = {k: os.environ.get(k) for k in ("WEBCAM_MOCK", "SETTINGS_EXTENSION_STATE")}
        os.environ.update(WEBCAM_MOCK="obsbot-tiny-3-lite", SETTINGS_EXTENSION_STATE=self.state)
        loader = importlib.machinery.SourceFileLoader("webcam_pretend", os.path.join(HERE, "settings-webcam"))
        self.w = importlib.util.module_from_spec(importlib.util.spec_from_loader("webcam_pretend", loader))
        loader.exec_module(self.w)
        self.page = self.w.pages()[0]["id"]

    def tearDown(self):
        for k, v in self.env.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        shutil.rmtree(self.state, ignore_errors=True)

    def describe(self, kinds):
        self.w.KINDS = kinds
        return self.w.describe(self.page)

    def test_off_unless_asked(self):
        self.assertIs(webcam.MOCK, None)
        self.assertIs(webcam.ob, obsbot)
        os.environ["WEBCAM_MOCK"] = "nope"
        with self.assertRaises(SystemExit):
            webcam_mock.profile()

    def test_pane_page(self):
        p = self.describe(PANE_KINDS)
        self.assertNotIn("subtitle", p)
        self.assertEqual([g["title"] for g in p["groups"]], ["Picture", "Camera", "OBSBOT", "Microphone", ""])
        for g in p["groups"]:
            for r in g["rows"]:
                self.assertTrue(r.get("icon"), f"{r.get('key') or r['title']} has no icon")
        blocks = p["preview"]["blocks"]
        self.assertEqual([b["kind"] for b in blocks], ["rows", "rows", "caption"])
        camera, frame, centre, goto, save = blocks[0]["rows"]
        self.assertEqual((camera["kind"], camera["device"]), ("camera", webcam_mock.DEVICE))
        self.assertEqual((frame["kind"], frame["value"]), ("segmented", "half"))
        self.assertEqual([o[1] for o in goto["options"]], ["Go to 1", "Go to 2", "Go to 3"])
        self.assertEqual((centre["key"], save["key"]), ("@centre", "@save"))
        self.assertEqual((blocks[1]["rows"][0]["kind"], blocks[1]["rows"][0]["source"]), ("meter", webcam_mock.MIC))
        rows = {r.get("key"): r for g in p["groups"] for r in g["rows"]}
        self.assertTrue(rows["white_balance_temperature"]["disabled"])
        self.assertEqual(rows["power_line_frequency"]["kind"], "segmented")
        self.assertEqual(rows["@power"]["desc"], "Awake · sleeps by itself: off")

    def test_old_page_without_preview(self):
        p = self.describe({"info", "switch", "camera", "meter"})
        self.assertNotIn("preview", p)
        self.assertEqual(p["subtitle"], f"<tt>{webcam_mock.DEVICE}</tt>")
        self.assertEqual(p["groups"][0]["rows"][0]["kind"], "camera")
        self.assertFalse(any("icon" in r for g in p["groups"] for r in g["rows"]))
        self.assertEqual(p["banners"][0]["text"], "AI tracking modes use OBSBOT's own commands and aren't in Settings yet.")

    def test_changes_stay_pretend(self):
        self.w.set_value(self.page, "white_balance_automatic", "false")
        self.w.set_value(self.page, "brightness", "70.4")
        self.w.set_value(self.page, "@goto", "3")
        self.w.set_value(self.page, "@power", "sleep")
        self.w.set_value(self.page, "@mic-default", "true")
        rows = {r.get("key"): r for g in self.describe(PANE_KINDS)["groups"] for r in g["rows"]}
        self.assertEqual(rows["brightness"]["value"], 70)
        self.assertNotIn("disabled", rows["white_balance_temperature"])
        self.assertEqual(rows["pan_absolute"]["value"], 36000)
        self.assertTrue(rows["@power"]["desc"].startswith("Asleep"))
        self.assertTrue(rows["@mic-default"]["value"])
        self.assertTrue(self.w.OPTIONS.startswith(self.state))
        self.assertFalse(os.path.exists(os.path.join(TMP, "wireplumber")))
        with self.assertRaises(SystemExit):
            self.w.set_value(self.page, "power_line_frequency", "7")

    def test_controls_parse_like_v4l2_ctl(self):
        groups = webcam.parse_controls(webcam_mock._list_ctrls(webcam_mock._fresh()["controls"]))
        self.assertEqual([g for g, _ in groups], ["Picture", "Camera"])
        ctrls = {c["name"]: c for _, cs in groups for c in cs}
        self.assertEqual(len(ctrls), len(webcam_mock.CONTROLS))
        self.assertIn("inactive", ctrls["white_balance_temperature"]["flags"])
        self.assertEqual(ctrls["power_line_frequency"]["menu"], [("0", "Disabled"), ("1", "50 Hz"), ("2", "60 Hz")])


if __name__ == "__main__":
    try:
        unittest.main(verbosity=2)
    finally:
        shutil.rmtree(TMP, ignore_errors=True)
