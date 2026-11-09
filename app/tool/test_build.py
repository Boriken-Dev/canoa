import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import build
from build import flutter_args, package_ui, package_debug_extension


class DesktopTests(unittest.TestCase):
    def test_runner_detection_follows_the_platform_marker(self):
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp)
            self.assertTrue(build.runner_missing("windows", app))
            (app / "windows" / "runner").mkdir(parents=True)
            (app / "windows" / "runner" / "CMakeLists.txt").write_text("")
            self.assertFalse(build.runner_missing("windows", app))
            self.assertTrue(build.runner_missing("linux", app))

    @patch("build.sh_program", return_value="/bin/sh")
    @patch("build.flutter_program", return_value="flutter")
    @patch("build.subprocess.run")
    def test_missing_runner_is_created_then_named_and_the_template_test_dropped(self, run, _f, _s):
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp)
            (app / "test").mkdir()

            def create(command, **_):
                if command[1] == "create":
                    (app / "test" / "widget_test.dart").write_text("")

            run.side_effect = create
            build.ensure_runner("linux", app)
            commands = [call.args[0] for call in run.call_args_list]
            self.assertEqual(
                commands[0],
                ["flutter", "create", "--org", "dev.boriken", "--platforms=linux", "--project-name", "canoa", "."],
            )
            self.assertEqual(commands[1][0], "/bin/sh")
            self.assertTrue(commands[1][1].endswith("name_runner.sh"))
            self.assertFalse((app / "test" / "widget_test.dart").exists())

    @patch("build.sh_program", return_value="/bin/sh")
    @patch("build.subprocess.run")
    def test_existing_runner_is_only_renamed_and_keeps_its_own_widget_test(self, run, _s):
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp)
            (app / "linux" / "runner").mkdir(parents=True)
            (app / "linux" / "runner" / "CMakeLists.txt").write_text("")
            (app / "test").mkdir()
            (app / "test" / "widget_test.dart").write_text("")
            build.ensure_runner("linux", app)
            self.assertEqual(run.call_count, 1)
            self.assertTrue((app / "test" / "widget_test.dart").exists())

    @patch("build.sh_program", return_value="/bin/sh")
    @patch("build.subprocess.run")
    def test_windows_gets_the_icon(self, _run, _s):
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp)
            (app / "windows" / "runner" / "resources").mkdir(parents=True)
            (app / "windows" / "runner" / "CMakeLists.txt").write_text("")
            (app / "windows" / "runner" / "resources" / "app_icon.ico").write_bytes(b"flutter")
            (app / "assets" / "icons").mkdir(parents=True)
            (app / "assets" / "icons" / "canoa.ico").write_bytes(b"canoa")
            build.ensure_runner("windows", app)
            self.assertEqual((app / "windows" / "runner" / "resources" / "app_icon.ico").read_bytes(), b"canoa")

    @patch("build.committed_lock", return_value=b"git\n")
    def test_lock_rewritten_by_a_tool_is_restored(self, _):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "app").mkdir()
            lock = root / "app" / "pubspec.lock"
            lock.write_bytes(b"older\n")
            self.assertTrue(build.restore_lock(b"git\n", root))
            self.assertEqual(lock.read_bytes(), b"git\n")
            self.assertFalse(build.restore_lock(b"git\n", root))

    @patch("build.committed_lock", return_value=b"git\n")
    def test_lock_edited_by_hand_is_left_alone(self, _):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "app").mkdir()
            lock = root / "app" / "pubspec.lock"
            lock.write_bytes(b"new dependency\n")
            self.assertFalse(build.restore_lock(b"new dependency\n", root))
            self.assertEqual(lock.read_bytes(), b"new dependency\n")

    def test_a_build_configured_before_the_runner_changed_is_removed(self):
        import os

        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp)
            (app / "windows").mkdir()
            lists = app / "windows" / "CMakeLists.txt"
            cache = app / "build" / "windows" / "arm64" / "CMakeCache.txt"
            cache.parent.mkdir(parents=True)
            lists.write_text("")
            cache.write_text("")
            os.utime(lists, (1000, 1000))
            os.utime(cache, (2000, 2000))
            self.assertFalse(build.clear_stale_build("windows", app))
            self.assertTrue(cache.exists())
            os.utime(lists, (3000, 3000))
            self.assertTrue(build.clear_stale_build("windows", app))
            self.assertFalse((app / "build" / "windows").exists())
            self.assertFalse(build.clear_stale_build("windows", app))

    def test_built_executable_is_found_by_mode(self):
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp)
            exe = app / "build" / "windows" / "arm64" / "runner" / "Release" / "CanoaUI.exe"
            exe.parent.mkdir(parents=True)
            exe.write_text("")
            self.assertEqual(build.built_executable("windows", False, app), exe)
            self.assertIsNone(build.built_executable("windows", True, app))

    @patch("build.shutil.which", side_effect=lambda name: None if name == "cargo" else "/bin/" + name)
    def test_missing_cargo_fails_clearly(self, _):
        with self.assertRaisesRegex(RuntimeError, "cargo is not on PATH"):
            build.require_tools()

    @patch("build.build_desktop")
    def test_arguments(self, desktop):
        host = build.host_platform()
        with patch("sys.argv", ["build.py", "desktop", "--debug", "--run"]):
            build.main()
        desktop.assert_called_once_with(host, True, True)
        other = "linux" if host != "linux" else "windows"
        with patch("sys.argv", ["build.py", "desktop", "--platform", other]), self.assertRaises(SystemExit):
            build.main()
        with patch("sys.argv", ["build.py", "pwa", "--run"]), self.assertRaises(SystemExit):
            build.main()

    @patch("build.restore_lock", return_value=True)
    @patch("build.built_executable", return_value=Path("CanoaUI.exe"))
    @patch("build.ensure_runner")
    @patch("build.flutter_program", return_value="flutter")
    @patch("build.require_tools")
    @patch("build.subprocess.run")
    def test_build_and_run_commands(self, run, _t, _f, _e, _x, restore):
        build.build_desktop("windows", False, False)
        self.assertEqual(run.call_args.args[0], ["flutter", "build", "windows", "--release"])
        build.build_desktop("windows", True, True)
        self.assertEqual(run.call_args.args[0], ["flutter", "run", "-d", "windows", "--debug"])
        self.assertEqual(restore.call_count, 2)


class PackagingTests(unittest.TestCase):
    @patch("build.shutil.which", return_value="/bin/flutter")
    def test_web_wasm_flag(self, _):
        self.assertIn("--wasm", flutter_args("pwa", True))
        self.assertNotIn("--wasm", flutter_args("pwa", False))
        self.assertIn("--base-href", flutter_args("chromium", False))

    def test_extension_assets_are_local(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)
            source = path / "web"
            source.mkdir()
            (source / "index.html").write_text("<html><head></head><body></body></html>")
            (source / "flutter_bootstrap.js").write_text("// bootstrap")
            (source / "main.dart.js").write_text("// code")
            (source / "main.dart.js.map").write_text("map")
            package_ui(source, path / "ui")
            self.assertTrue((path / "ui" / "index.html").exists())
            self.assertIn("../bridge.js", (path / "ui" / "index.html").read_text())
            self.assertFalse((path / "ui" / "main.dart.js.map").exists())

    def test_debug_extension_is_a_distinct_unpacked_build(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            source = base / "chromium"
            (source / "ui").mkdir(parents=True)
            (source / "ui" / "index.html").write_text("<html></html>")
            (source / "manifest.json").write_text('{"permissions":["tabs"]}')
            (source / "manifest.debug.json").write_text(
                '{"permissions":["tabs","debugger"]}'
            )
            package_debug_extension(source, base / "chromium-debug")
            self.assertIn(
                "debugger",
                (base / "chromium-debug" / "manifest.json").read_text(),
            )
            self.assertNotIn(
                "debugger", (source / "manifest.json").read_text(),
            )

    def test_broken_bundle_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(FileNotFoundError):
                package_ui(Path(tmp), Path(tmp) / "out")



    def test_pinned_coqui_peer_methods_match_their_git_blobs(self):
        import hashlib
        import json
        vendor = Path(__file__).resolve().parents[1] / "web" / "vendor"
        meta = json.loads((vendor / "COQUI-SOURCE.json").read_text())
        self.assertEqual(sorted(meta), ["coqui-peer-noise.mjs", "coqui-peer-webcrypto.mjs"])
        for name, pinned in meta.items():
            source = (vendor / name).read_bytes()
            self.assertEqual(
                hashlib.sha1(f"blob {len(source)}\0".encode() + source).hexdigest(),
                pinned["git_blob_sha1"],
                name,
            )

if __name__ == "__main__":
    unittest.main()
