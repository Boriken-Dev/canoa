import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import build
import offline_pub
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
    @patch("build.flutter_program", return_value="flutter")
    @patch("build.subprocess.run")
    def test_an_offline_build_creates_the_runner_offline(self, run, _f, _s):
        with tempfile.TemporaryDirectory() as tmp:
            build.ensure_runner("linux", Path(tmp), offline=True)
            self.assertIn("--offline", run.call_args_list[0].args[0])

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
        desktop.assert_called_once_with(host, True, True, False, "auto")
        with patch("sys.argv", ["build.py", "desktop", "--portable"]):
            build.main()
        desktop.assert_called_with(host, False, False, True, "auto")
        with patch("sys.argv", ["build.py", "desktop", "--online"]):
            build.main()
        desktop.assert_called_with(host, False, False, False, "online")
        with patch("sys.argv", ["build.py", "desktop", "--offline"]):
            build.main()
        desktop.assert_called_with(host, False, False, False, "offline")
        for bad in (["desktop", "--online", "--offline"], ["pwa", "--offline"]):
            with patch("sys.argv", ["build.py", *bad]), self.assertRaises(SystemExit):
                build.main()
        for bad in (["desktop", "--portable", "--debug"], ["desktop", "--portable", "--run"], ["pwa", "--portable"]):
            with patch("sys.argv", ["build.py", *bad]), self.assertRaises(SystemExit):
                build.main()
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

    @patch("build.restore_lock", return_value=False)
    @patch("build.package_portable", return_value=Path("target/canoa-windows/CanoaUI.exe"))
    @patch("build.build_web_for_host", return_value=Path("app/build/web"))
    @patch("build.built_executable", return_value=Path("CanoaUI.exe"))
    @patch("build.ensure_runner")
    @patch("build.flutter_program", return_value="flutter")
    @patch("build.require_tools")
    @patch("build.subprocess.run")
    def test_portable_builds_the_web_app_in_and_packages_the_release_folder(self, run, _t, _f, _e, _x, web, package, _r):
        build.build_desktop("windows", False, False, portable=True)
        web.assert_called_once()
        self.assertEqual(run.call_args.args[0], ["flutter", "build", "windows", "--release"])
        self.assertEqual(run.call_args.kwargs["env"]["CANOA_WEB_DIST"], str(Path("app/build/web")))
        package.assert_called_once_with("windows", Path("CanoaUI.exe"))

    @patch("build.require_tools")
    def test_portable_macos_points_at_the_bundle(self, _):
        with self.assertRaisesRegex(RuntimeError, "already one bundle"):
            build.build_desktop("macos", False, False, portable=True)


@patch("build.restore_lock", return_value=False)
@patch("build.built_executable", return_value=Path("CanoaUI.exe"))
@patch("build.ensure_runner")
@patch("build.flutter_program", return_value="flutter")
@patch("build.require_tools")
@patch("build.subprocess.run")
class PubModeTests(unittest.TestCase):
    def setUp(self):
        patcher = patch.dict("build.os.environ", {"PUB_CACHE": "/user/cache"})
        patcher.start()
        self.addCleanup(patcher.stop)

    def commands(self, run):
        return [call.args[0] for call in run.call_args_list]

    @patch("build.offline_pub.plan", return_value=offline_pub.Plan(Path("/seeded"), None))
    def test_offline_is_chosen_when_deps_matches(self, plan, run, _t, _f, ensure, *_):
        seen = []
        run.side_effect = lambda *a, **k: seen.append(build.os.environ["PUB_CACHE"])
        ensure.side_effect = lambda *a, **k: seen.append("runner")
        build.build_desktop("windows", False, False, mode="auto")
        self.assertEqual(self.commands(run), [
            ["flutter", "pub", "get", "--offline"], ["flutter", "build", "windows", "--release", "--no-pub"],
        ])
        seen = ["cache" if s == str(Path("/seeded")) else s for s in seen]
        self.assertEqual(seen, ["runner", "cache", "cache"], "the runner exists before pub get writes its plugin files")
        self.assertEqual(build.os.environ["PUB_CACHE"], "/user/cache", "the cache is put back")
        self.assertEqual(ensure.call_args.kwargs, {"offline": True})

    @patch("build.offline_pub.plan", return_value=offline_pub.Plan(Path("/seeded"), None, Path("/cargo_home")))
    def test_offline_points_cargo_at_deps_crates_with_the_network_off(self, plan, run, *_):
        seen = []
        run.side_effect = lambda *a, **k: seen.append(
            (build.os.environ.get("CARGO_HOME"), build.os.environ.get("CARGO_NET_OFFLINE")))
        with patch.dict("build.os.environ", {"CARGO_HOME": "/mine"}):
            build.build_desktop("windows", False, False, mode="auto")
            self.assertEqual(build.os.environ["CARGO_HOME"], "/mine")
        self.assertEqual(seen, [(str(Path("/cargo_home")), "true")] * 2)
        self.assertNotIn("CARGO_NET_OFFLINE", build.os.environ)

    @patch("build.offline_pub.plan", return_value=offline_pub.Plan(None, "the deps branch's crates/ does not cover Cargo.lock: crates/x-1.0.0: missing"))
    def test_crates_deps_does_not_cover_falls_back_online_with_cargo_untouched(self, plan, run, *_):
        seen = []
        run.side_effect = lambda *a, **k: seen.append(build.os.environ.get("CARGO_NET_OFFLINE"))
        with patch("builtins.print") as out:
            build.build_desktop("windows", False, False, mode="auto")
        self.assertEqual(seen, [None])
        self.assertIn("crates/x-1.0.0", str(out.call_args_list))

    @patch("build.offline_pub.plan", return_value=offline_pub.Plan(None, "git dependencies are not in the pub cache"))
    def test_a_missing_dependency_falls_back_online_and_says_why(self, plan, run, *_):
        with patch("builtins.print") as out:
            build.build_desktop("windows", False, False, mode="auto")
        self.assertEqual(self.commands(run), [["flutter", "build", "windows", "--release"]])
        said = " ".join(str(call.args[0]) for call in out.call_args_list)
        self.assertIn("online", said)
        self.assertIn("git dependencies are not in the pub cache", said)

    @patch("build.offline_pub.plan", return_value=offline_pub.Plan(None, "the deps branch's pubspec.lock differs"))
    def test_offline_mode_errors_instead_of_falling_back(self, plan, run, *_):
        with self.assertRaisesRegex(RuntimeError, "pubspec.lock differs"):
            build.build_desktop("windows", False, False, mode="offline")
        run.assert_not_called()

    @patch("build.offline_pub.plan", return_value=offline_pub.Plan(Path("/seeded"), None))
    def test_a_failed_offline_resolution_falls_back_in_auto_but_not_offline(self, plan, run, *_):
        def fail_offline(command, **_):
            if "--offline" in command:
                raise build.subprocess.CalledProcessError(1, command)

        run.side_effect = fail_offline
        with patch("builtins.print"):
            build.build_desktop("windows", False, False, mode="auto")
        self.assertEqual(self.commands(run)[-1], ["flutter", "build", "windows", "--release"])
        with self.assertRaisesRegex(RuntimeError, "offline"):
            build.build_desktop("windows", False, False, mode="offline")

    @patch("build.offline_pub.plan")
    def test_online_skips_deps(self, plan, run, *_):
        build.build_desktop("windows", False, False, mode="online")
        plan.assert_not_called()
        self.assertEqual(self.commands(run), [["flutter", "build", "windows", "--release"]])


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



    def test_extension_coqui_peer_methods_match_their_git_blobs(self):
        import hashlib
        import json
        vendor = Path(__file__).resolve().parents[2] / "bindings" / "browser" / "chromium" / "vendor"
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
