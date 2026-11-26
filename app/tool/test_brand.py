import json
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

import brand
import build

APP = Path(__file__).resolve().parents[1]
ROOT = APP.parent
TEST_BRAND = APP / "test" / "fixtures" / "brand" / "brand.json"
AGENTUI = ROOT / "crates" / "canoa" / "tests" / "brands" / "agentui" / "brand.json"


class BrandFileTests(unittest.TestCase):
    def test_the_default_brand_is_canoas(self):
        default = brand.load()
        self.assertEqual((default.name, default.executable, default.env_prefix), ("Canoa", "CanoaUI", "CANOA"))
        self.assertEqual(default.source, brand.DEFAULT_FILE)
        self.assertEqual(default.org, "dev.boriken")

    def test_a_test_brand_changes_every_name(self):
        other = brand.load(TEST_BRAND)
        self.assertEqual((other.name, other.executable, other.env_prefix, other.org), ("Pirogue", "PirogueUI", "PIROGUE", "org.example"))
        defines = other.defines()
        self.assertIn("--dart-define=BRAND_NAME=Pirogue", defines)
        self.assertIn("--dart-define=BRAND_ENV_PREFIX=PIROGUE", defines)
        self.assertIn("--dart-define=BRAND_ACCENT=#C0392B", defines)
        mark = json.loads(next(d for d in defines if d.startswith("--dart-define=BRAND_MARK=")).split("=", 2)[2])
        self.assertEqual(set(mark), {"hull", "smallWeight", "cut", "paddle", "spiral", "spiralWidth"})
        self.assertEqual(other.environment(), {"BRAND_FILE": str(TEST_BRAND.resolve())})
        self.assertIn("--dart-define=BRAND_EXECUTABLE=PirogueUI", defines)
        self.assertIn("--dart-define=BRAND_PROGRAM=pirogue", defines)
        self.assertIn("--dart-define=BRAND_HOME_DIR=.pirogue", defines)

    def test_a_brand_that_names_nothing_else_takes_its_names_from_its_name(self):
        other = brand.load(TEST_BRAND)
        self.assertEqual((other.program, other.config_file, other.folder, other.home_dir), ("pirogue", "pirogue.cfg", "Pirogue", ".pirogue"))

    def test_the_default_brand_spells_out_what_canoa_has_always_used(self):
        default = brand.load()
        self.assertEqual((default.program, default.config_file, default.folder, default.home_dir), ("canoa", "canoa.cfg", "Canoa", ".canoa"))
        written = json.loads(brand.DEFAULT_FILE.read_text(encoding="utf-8"))
        for key in brand.NAMES:
            self.assertEqual(written[key], getattr(default, key))

    def test_the_example_brand_names_each_place_itself(self):
        example = brand.load(AGENTUI)
        self.assertEqual((example.program, example.config_file, example.folder, example.home_dir, example.env_prefix),
                         ("agents", "agents.cfg", "agents", ".agentui", "AGENTUI"))
        self.assertEqual(dict(line.split("=", 1) for line in subprocess.run(
            [__import__("sys").executable, "-I", str(APP / "tool" / "brand.py"), "--brand", str(AGENTUI), "vars"],
            check=True, capture_output=True, text=True).stdout.splitlines())["CONFIG_FILE"], "agents.cfg")

    def test_a_brand_file_that_is_wrong_says_which_key(self):
        good = json.loads(TEST_BRAND.read_text(encoding="utf-8"))
        for key, value in (("env_prefix", "lower"), ("accent", "red"), ("id", "NoDots"), ("executable", "has space"),
                           ("name", 'say "hi"'), ("icon", "missing.svg"), ("extra", "x"),
                           ("config_file", ""), ("config_file", "a/b.cfg"), ("config_file", "a\\b.cfg"), ("config_file", "..cfg"),
                           ("config_file", "x:y.cfg"), ("config_file", "nul.cfg"), ("folder", "agents."), ("folder", " agents"),
                           ("folder", "CON"), ("folder", "a*b"), ("home_dir", ".a\nb"), ("home_dir", "../x"),
                           ("program", "Agents"), ("program", "1agents"), ("program", "has space")):
            with self.subTest(key):
                with tempfile.TemporaryDirectory() as tmp:
                    folder = Path(tmp)
                    (folder / "mark.svg").write_bytes((TEST_BRAND.parent / "mark.svg").read_bytes())
                    (folder / "brand.json").write_text(json.dumps({**good, key: value}), encoding="utf-8")
                    with self.assertRaises(brand.BrandError) as caught:
                        brand.load(folder / "brand.json")
                    self.assertIn(key, str(caught.exception))

    def test_a_name_that_is_one_path_component_is_taken(self):
        good = json.loads(TEST_BRAND.read_text(encoding="utf-8"))
        for key, value in (("config_file", "agents.cfg"), ("config_file", "agents.json"), ("folder", "agents"), ("folder", "Agent UI"),
                           ("home_dir", ".agentui"), ("home_dir", "agentui"), ("program", "agent-ui")):
            with self.subTest(key + value):
                with tempfile.TemporaryDirectory() as tmp:
                    folder = Path(tmp)
                    (folder / "mark.svg").write_bytes((TEST_BRAND.parent / "mark.svg").read_bytes())
                    (folder / "brand.json").write_text(json.dumps({**good, key: value}), encoding="utf-8")
                    self.assertEqual(getattr(brand.load(folder / "brand.json"), key), value)

    def test_the_hosts_copy_is_the_brand(self):
        copy = ROOT / "crates" / "canoa" / "brand"
        for name in ("brand.json", "assets/icons/canoa.svg"):
            original = (APP / name).read_bytes().replace(b"\r\n", b"\n")
            self.assertEqual((copy / name).read_bytes().replace(b"\r\n", b"\n"), original, f"copy app/{name} over crates/canoa/brand/{name}")

class RunnerTests(unittest.TestCase):
    def test_a_brand_names_the_runner(self):
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp)
            for name, text in {
                "windows/CMakeLists.txt": 'set(BINARY_NAME "canoa")\n',
                "linux/CMakeLists.txt": 'set(BINARY_NAME "canoa")\n',
                "windows/runner/Runner.rc": '"FileDescription", "canoa"\n"InternalName", "canoa"\n"OriginalFilename", "canoa.exe"\n"ProductName", "canoa"\n',
                "windows/runner/main.cpp": 'window.Create(L"canoa", origin, size)\n',
                "linux/runner/my_application.cc": '#include "flutter/generated_plugin_registrant.h"\n_title(header_bar, "canoa"); _title(window, "canoa");\nfl_register_plugins(FL_PLUGIN_REGISTRY(view));\n',
                "windows/runner/flutter_window.cpp": '#include "flutter/generated_plugin_registrant.h"\nRegisterPlugins(flutter_controller_->engine());\n',
            }.items():
                path = app / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text, encoding="utf-8", newline="\n")
            script = APP / "tool" / "name_runner.sh"
            env = {"BRAND_FILE": str(TEST_BRAND), "PATH": str(Path(build.sh_program()).parent)}
            subprocess.run([build.sh_program(), str(script), str(app)], check=True, env=env)
            self.assertIn('BINARY_NAME "PirogueUI"', (app / "windows/CMakeLists.txt").read_text(encoding="utf-8"))
            self.assertIn('"ProductName", "Pirogue"', (app / "windows/runner/Runner.rc").read_text(encoding="utf-8"))
            self.assertIn('"OriginalFilename", "PirogueUI.exe"', (app / "windows/runner/Runner.rc").read_text(encoding="utf-8"))
            self.assertIn('Create(L"Pirogue"', (app / "windows/runner/main.cpp").read_text(encoding="utf-8"))
            self.assertIn('_title(window, "Pirogue")', (app / "linux/runner/my_application.cc").read_text(encoding="utf-8"))
            # Run again, and with Canoa's own brand: the same files take either.
            subprocess.run([build.sh_program(), str(script), str(app)], check=True, env={**env, "BRAND_FILE": str(brand.DEFAULT_FILE)})
            self.assertIn('BINARY_NAME "CanoaUI"', (app / "windows/CMakeLists.txt").read_text(encoding="utf-8"))

    def test_the_built_program_is_the_brands(self):
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp)
            found = app / "build" / "windows" / "x64" / "runner" / "Release" / "PirogueUI.exe"
            found.parent.mkdir(parents=True)
            found.write_bytes(b"")
            saved = build.BRAND
            try:
                build.BRAND = brand.load(TEST_BRAND)
                self.assertEqual(build.built_executable("windows", False, app), found)
            finally:
                build.BRAND = saved


class MarkTests(unittest.TestCase):
    def test_a_brand_mark_is_drawn_in_its_accent(self):
        try:
            from PIL import Image
        except ImportError:
            self.skipTest("Pillow is not installed")
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "icons"
            subprocess.run(
                [__import__("sys").executable, str(ROOT / "tools" / "canoa_mark.py"), str(ROOT), "--brand", str(TEST_BRAND), "--out", str(out)],
                check=True, stdout=subprocess.DEVNULL,
            )
            tray = out / "app" / "assets" / "icons" / "canoa_tray.png"
            ico = out / "app" / "assets" / "icons" / "canoa.ico"
            self.assertTrue(ico.is_file())
            with Image.open(tray).convert("RGBA") as image:
                solid = [pixel for pixel in image.getdata() if pixel[3] == 255]
            self.assertTrue(solid)
            self.assertEqual({pixel[:3] for pixel in solid}, {(0xC0, 0x39, 0x2B)})


if __name__ == "__main__":
    unittest.main()
