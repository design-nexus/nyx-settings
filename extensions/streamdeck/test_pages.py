"""Tests for the Stream Deck pages in style A (galleon_page.py, elgato.py's describe_a)
and the pretend decks. Run: python3 -m unittest test_pages.py (scratch folders only)."""

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.realpath(__file__))
sys.path.insert(0, HERE)
import galleon_config as gc  # noqa: E402
import galleon_page  # noqa: E402

SAMPLE = os.path.join(HERE, "testdata", "galleon-deck")
KINDS = "info switch slider choice segmented buttons entry button colour chips curve disclosure camera meter preview icon"


def run(tmp, *args, kinds=True, **env):
    e = {k: v for k, v in os.environ.items() if k not in ("SETTINGS_KINDS", "CORSAIR_MOCK", "ELGATO_MOCK")}
    e.update(HOME=tmp, XDG_CONFIG_HOME=os.path.join(tmp, "conf"), XDG_RUNTIME_DIR=os.path.join(tmp, "run"),
             SETTINGS_EXTENSION_STATE=os.path.join(tmp, "state"), CORSAIR_MOCK="galleon-100-sd", ELGATO_MOCK="mk2", **env)
    if kinds:
        e["SETTINGS_KINDS"] = KINDS
    r = subprocess.run([os.path.join(HERE, "settings-streamdeck"), *args], capture_output=True, text=True, env=e)
    if r.returncode != 0:
        raise AssertionError(r.stderr)
    return json.loads(r.stdout) if r.stdout.strip() else None


def left_rows(page):
    for g in page["groups"]:
        for row in g["rows"]:
            yield row
            yield from row.get("rows", [])


class Pretend(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def test_preview_only_when_settings_can_draw_it(self):
        for page in ("deck", "elgato"):
            self.assertIn("preview", run(self.tmp, "describe", page), page)
            self.assertNotIn("preview", run(self.tmp, "describe", page, kinds=False), page)

    def test_old_galleon_page_without_preview(self):
        titles = [g["title"] for g in run(self.tmp, "describe", "deck", kinds=False)["groups"]]
        self.assertEqual(titles[0], "Set up")
        self.assertNotIn("Move from galleon-deck", titles)

    def test_every_left_row_has_an_icon(self):
        for page in ("deck", "elgato"):
            for row in left_rows(run(self.tmp, "describe", page)):
                self.assertTrue(row.get("icon"), f"{page}: {row['title']}")

    def test_galleon_page_as_in_the_mockup(self):
        d = run(self.tmp, "describe", "deck")
        self.assertEqual([g["title"] for g in d["groups"]],
                         ["Stream Deck", "Profiles", "More", "Move from galleon-deck", "Device access"])
        self.assertTrue(d["banners"][0]["warning"])
        deck, editor = d["preview"]["blocks"]
        self.assertEqual((deck["kind"], deck["columns"], deck["dials"], deck["selected"], len(deck["keys"])), ("deck", 3, 2, 2, 12))
        self.assertEqual(deck["screen"][:3], ["Work · Page 1", "14:32", "Fri 13 Nov"])
        self.assertEqual(deck["keys"][2], {"icon": "web-browser-symbolic", "label": "Browser"})
        self.assertEqual(editor["title"], "Key 3 · Browser")
        fields = {r["key"]: r.get("value") for r in editor["rows"]}
        self.assertEqual((fields["g-key:action"], fields["g-key:exec"]), ("exec", "firefox"))

    def test_picking_a_key_and_a_sequence(self):
        run(self.tmp, "set", "deck", "g-profile", "Gaming")
        run(self.tmp, "set", "deck", "g-pick", "0")
        editor = run(self.tmp, "describe", "deck")["preview"]["blocks"][1]
        fields = {r["key"]: r.get("value") for r in editor["rows"]}
        self.assertEqual((fields["g-key:action"], fields["g-key:hold"], fields["g-key:sequence"]), ("sequence", "CTRL", "S S W D"))
        self.assertIn("g-key:step_ms", fields)
        with self.assertRaises(AssertionError):
            run(self.tmp, "set", "deck", "g-key:sequence", "S NOPE")

    def test_moving_over_changes_only_pretend_state(self):
        run(self.tmp, "set", "deck", "g-galleon-off", "")
        d = run(self.tmp, "describe", "deck")
        self.assertEqual(d["banners"], [])
        self.assertEqual(d["preview"]["foot"], "Every change shows on the deck within a second.")
        self.assertTrue(os.path.isdir(os.path.join(self.tmp, "state", "pretend")))
        self.assertFalse(os.path.exists(os.path.join(self.tmp, "conf", "nyx-deck")))

    def test_elgato_page_as_in_the_mockup(self):
        d = run(self.tmp, "describe", "elgato")
        self.assertEqual([g["title"] for g in d["groups"]], ["Stream Deck", "Profile"])
        deck, editor = d["preview"]["blocks"]
        self.assertEqual((deck["columns"], deck["selected"], len(deck["keys"])), (5, 2, 15))
        self.assertEqual(d["preview"]["title"], "Keys on “Main”")
        self.assertEqual(editor["title"], "Key 3 · Browser")
        run(self.tmp, "set", "elgato", "elgato-pick", "13")
        self.assertEqual(run(self.tmp, "describe", "elgato")["preview"]["blocks"][1]["title"], "Key 14 · Lock")
        run(self.tmp, "set", "elgato", "elgato-run", "false")
        self.assertFalse(run(self.tmp, "describe", "elgato")["groups"][0]["rows"][0]["value"])


class Jobs:
    @staticmethod
    def deck_present():
        return True


class Quiet(galleon_page.Env):
    """The real page on scratch folders, with the services and desktop answered here."""

    def __init__(self, state):
        super().__init__(state, Jobs)
        self.units = {galleon_page.NYX_UNIT: [False, False], gc.GALLEON_UNIT: [True, True]}
        self.installed = True
        self.started = []

    def unit(self, unit):
        return tuple(self.units.get(unit, [False, False]))

    def galleon_deck_running(self):
        return self.units[gc.GALLEON_UNIT][1]

    def galleon_deck_installed(self):
        return self.installed

    def access_ready(self):
        return True

    def missing(self):
        return []

    def deck_state(self):
        return None

    def request(self, profile, page=None):
        self.started.append(("request", profile, page))

    def windows(self):
        return [("helldivers2.exe", "HELLDIVERS 2")]

    def playing(self):
        return ""

    def run_nyx_deck(self):
        self.started.append("nyx-deck")
        self.units[galleon_page.NYX_UNIT] = [True, True]

    def galleon_deck_off(self):
        self.units[gc.GALLEON_UNIT] = [False, False]

    def terminal(self, job, script):
        self.started.append((job, script))

    def job_row(self, key, title, desc, label, job, doing, icon):
        return {"kind": "button", "key": key, "title": title, "label": label, "icon": icon}

    def jobs_running(self):
        return False


def digest(root):
    h = hashlib.sha256()
    for base, _, files in sorted(os.walk(root)):
        for name in sorted(files):
            with open(os.path.join(base, name), "rb") as f:
                h.update(name.encode() + f.read())
    return h.hexdigest()


class Page(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.saved = os.environ.get("XDG_CONFIG_HOME")
        os.environ["XDG_CONFIG_HOME"] = os.path.join(self.tmp, "conf")
        shutil.copytree(SAMPLE, os.path.join(self.tmp, "conf", "galleon-deck"))
        self.env = Quiet(os.path.join(self.tmp, "state"))

    def tearDown(self):
        if self.saved is None:
            os.environ.pop("XDG_CONFIG_HOME", None)
        else:
            os.environ["XDG_CONFIG_HOME"] = self.saved
        shutil.rmtree(self.tmp)

    def set(self, key, value=""):
        return galleon_page.set_value(self.env, key, value)

    def editor(self):
        return {r["key"]: r for r in galleon_page.describe(self.env)["preview"]["blocks"][1]["rows"]}

    def test_before_the_import_it_shows_galleon_decks_profile(self):
        d = galleon_page.describe(self.env)
        self.assertEqual([g["title"] for g in d["groups"]], ["Stream Deck", "Move from galleon-deck", "Device access"])
        self.assertEqual(d["preview"]["title"], "galleon-deck's “desktop”")
        self.assertNotIn("key", d["preview"]["blocks"][0], "nothing to click before the import")
        self.assertFalse(os.path.exists(self.env.nyx.root), "describe writes nothing")

    def test_import_copies_and_leaves_galleon_deck_alone(self):
        before = digest(self.env.legacy.root)
        reply = self.set("g-import")
        self.assertIn("Imported 2 profiles", reply["toast"])
        self.assertEqual(digest(self.env.legacy.root), before)
        self.assertEqual(digest(self.env.nyx.root), before, "the same files, unchanged")
        self.assertEqual(sorted(gc.load_config(self.env.nyx)["profiles"]), ["desktop", "gaming"])
        self.set("g-key:label", "Seven")
        self.assertEqual(digest(self.env.legacy.root), before)
        self.set("g-import")
        self.assertEqual(len([n for n in os.listdir(os.path.dirname(self.env.nyx.root)) if ".before-import-" in n]), 1)

    def test_run_switch_turns_on_nyx_deck(self):
        self.set("g-import")
        self.assertFalse(gc.galleon_enabled(self.env.nyx_config))
        reply = self.set("g-run", "true")
        self.assertTrue(gc.galleon_enabled(self.env.nyx_config))
        self.assertIn("nyx-deck", self.env.started)
        self.assertIn("once galleon-deck is off", reply["toast"])
        run_row = galleon_page.describe(self.env)["groups"][0]["rows"][0]
        self.assertEqual(run_row["tag"], "Waiting for galleon-deck")
        self.set("g-galleon-off")
        self.assertNotIn("tag", galleon_page.describe(self.env)["groups"][0]["rows"][0])
        self.set("g-run", "false")
        self.assertFalse(gc.galleon_enabled(self.env.nyx_config))

    def test_nothing_to_import_starts_a_profile(self):
        shutil.rmtree(self.env.legacy.root)
        self.env.installed = False
        self.env.units[gc.GALLEON_UNIT] = [False, False]
        d = galleon_page.describe(self.env)
        self.assertEqual(gc.profile_names(self.env.nyx), ["desktop"])
        gc.load_config(self.env.nyx)
        self.assertNotIn("Move from galleon-deck", [g["title"] for g in d["groups"]])

    def test_every_action_has_its_fields(self):
        self.set("g-import")
        expect = {"key": "g-key:key", "keys": "g-key:keys", "sequence": "g-key:sequence", "mouse": "g-key:mouse",
                  "repeat": "g-key:repeat", "exec": "g-key:exec", "media": "g-key:media", "page": "g-key:page",
                  "profile": "g-key:profile"}
        for action, field in expect.items():
            self.set("g-key:action", action)
            self.assertIn(field, self.editor(), action)
            gc.load_config(self.env.nyx)
        self.set("g-key:action", "none")
        self.assertEqual(galleon_page.page_keys(gc.read_toml(self.env.nyx.profile("desktop")), "numpad")[0], {"label": "7"})

    def test_key_edits(self):
        self.set("g-import")
        self.set("g-pick", "3")
        self.set("g-key:action", "exec")
        self.set("g-key:exec", "firefox")
        self.set("g-key:confirm", "true")
        self.set("g-key:icon", "F059F")
        self.set("g-key:bg", "#3B1C5A")
        self.set("g-key:image", "icons/web.png")
        spec = galleon_page.page_keys(gc.read_toml(self.env.nyx.profile("desktop")), "numpad")[3]
        self.assertEqual(spec, {"label": "4", "exec": "firefox", "confirm": True, "icon": chr(0xF059F), "bg": "#3b1c5a",
                                "image": "icons/web.png"})
        self.assertEqual(self.editor()["g-key:icon"]["value"], "F059F")
        self.set("g-key:move", "0")
        keys = galleon_page.page_keys(gc.read_toml(self.env.nyx.profile("desktop")), "numpad")
        self.assertEqual((keys[0]["label"], keys[3]["label"]), ("4", "7"))
        with open(self.env.nyx.profile("desktop")) as f:
            self.assertIn("# Desktop profile", f.read())
        self.set("g-key:action", "key")
        with self.assertRaises(SystemExit):
            self.set("g-key:key", "NOSUCH")
        with self.assertRaises(SystemExit):
            self.set("g-key:key", "")
        with self.assertRaises(SystemExit):
            self.set("g-pick", "12")

    def test_profiles_and_rules_stay_consistent(self):
        self.set("g-import")
        self.set("g-rule-add", "helldivers2.exe")
        rules = gc.read_toml(self.env.nyx.config)["auto_switch"]["rules"]
        self.assertEqual(rules[-1], {"class": "^helldivers2\\.exe$", "profile": "desktop", "name": "HELLDIVERS 2"})
        self.set("g-profile", "gaming")
        self.set("g-rename-profile", "games")
        cfg = gc.load_config(self.env.nyx)
        self.assertEqual(cfg["auto_switch"]["rules"][0]["profile"], "games")
        self.assertEqual(cfg["profiles"]["desktop"]["page"][0]["keys"][0]["label"], "7")
        self.set("g-remove-profile")
        cfg = gc.load_config(self.env.nyx)
        self.assertEqual(list(cfg["profiles"]), ["desktop"])
        self.assertTrue(all(r["profile"] == "desktop" for r in cfg["auto_switch"]["rules"]))
        with self.assertRaises(SystemExit):
            self.set("g-remove-profile")

    def test_pages_and_look(self):
        self.set("g-import")
        self.set("g-add-page", "Streaming")
        self.set("g-rename-page", "Live")
        self.set("g-move-page", "earlier")
        self.assertEqual(galleon_page.page_names(gc.read_toml(self.env.nyx.profile("desktop")))[-2:], ["Live", "media"])
        self.set("g-key-style", "hud")
        self.set("g-colour:accent", "#7ee0ff")
        prof = gc.read_toml(self.env.nyx.profile("desktop"))
        self.assertEqual(prof["theme_overrides"], {"key_style": "hud", "accent": "#7ee0ff"})
        self.set("g-colour-reset")
        self.set("g-key-style", "rounded")
        self.assertNotIn("theme_overrides", gc.read_toml(self.env.nyx.profile("desktop")))
        self.set("g-boot", "gif")
        self.set("g-boot-gif", "~/intro.gif")
        self.set("g-screen-volume", "changing")
        self.set("g-dial:right_turn", "volume")
        cfg = gc.load_config(self.env.nyx)
        self.assertEqual((cfg["boot"]["gif"], cfg["screen"]["volume"], cfg["dials"]["right_turn"]), ("~/intro.gif", "changing", "volume"))

    def test_access_and_removal_go_through_a_terminal(self):
        self.set("g-access")
        self.set("g-galleon-remove")
        jobs = [j for j in self.env.started if isinstance(j, tuple) and j[0].startswith("galleon-")]
        self.assertEqual([j[0] for j in jobs], ["galleon-access", "galleon-remove"])
        self.assertIn(galleon_page.UDEV_RULE, jobs[0][1])
        self.assertNotIn("galleon-deck/config", jobs[1][1].replace("galleon-deck-config", ""), "your files stay")
        self.assertIn("sudo sh -c", jobs[1][1])


if __name__ == "__main__":
    unittest.main()
