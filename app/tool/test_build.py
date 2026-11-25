import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import build
import offline_pub
from build import flutter_args, package_ui, package_debug_extension


class DesktopTests(unittest.TestCase):
    def setUp(self):
        # A deps checkout the environment names is not looked for, which would run git; the SDK is not inspected.
        for patcher in (patch.dict("build.os.environ", {"CANOA_DEPS": "/deps"}), patch("build.require_pinned_sdk")):
            patcher.start()
            self.addCleanup(patcher.stop)

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
        desktop.assert_called_once_with(host, True, True, False, False, "bundled")
        with patch("sys.argv", ["build.py", "desktop", "--portable"]):
            build.main()
        desktop.assert_called_with(host, False, False, True, False, "bundled")
        with patch("sys.argv", ["build.py", "desktop", "--online"]):
            build.main()
        desktop.assert_called_with(host, False, False, False, True, "bundled")
        with patch("sys.argv", ["build.py", "desktop", "--office-js", "cdn"]):
            build.main()
        desktop.assert_called_with(host, False, False, False, False, "cdn")
        for bad in (["desktop", "--offline"], ["pwa", "--offline"], ["pwa", "--office-js", "cdn"],
                    ["desktop", "--office-js", "elsewhere"]):
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
        build.build_desktop("windows", False, False, online=True)
        self.assertEqual(run.call_args.args[0], ["flutter", "build", "windows", "--release", *build.BRAND.defines()])
        build.build_desktop("windows", True, True, online=True)
        self.assertEqual(run.call_args.args[0], ["flutter", "run", "-d", "windows", "--debug", *build.BRAND.defines()])
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
        build.build_desktop("windows", False, False, portable=True, online=True)
        web.assert_called_once_with(None)
        self.assertEqual(run.call_args.args[0], ["flutter", "build", "windows", "--release", *build.BRAND.defines()])
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
        for patcher in (patch.dict("build.os.environ", {"PUB_CACHE": "/user/cache", "CANOA_DEPS": "/deps"}),
                        patch("build.require_pinned_sdk")):
            patcher.start()
            self.addCleanup(patcher.stop)

    def commands(self, run):
        return [call.args[0] for call in run.call_args_list]

    @patch("build.offline_pub.find_deps", return_value=Path("/deps"))
    @patch("build.offline_pub.plan", return_value=offline_pub.Plan(Path("/seeded"), None))
    def test_offline_is_the_default_and_builds_without_resolving_again(self, plan, _d, run, _t, _f, ensure, *_):
        seen = []
        run.side_effect = lambda *a, **k: seen.append(build.os.environ["PUB_CACHE"])
        ensure.side_effect = lambda *a, **k: seen.append("runner")
        build.build_desktop("windows", False, False)
        self.assertEqual(self.commands(run), [
            ["flutter", "pub", "get", "--offline"], ["flutter", "build", "windows", "--release", "--no-pub", *build.BRAND.defines()],
        ])
        seen = ["cache" if s == str(Path("/seeded")) else s for s in seen]
        self.assertEqual(seen, ["runner", "cache", "cache"], "the runner exists before pub get writes its plugin files")
        self.assertEqual(build.os.environ["PUB_CACHE"], "/user/cache", "the cache is put back")
        self.assertEqual(ensure.call_args.kwargs, {"offline": True})

    @patch("build.offline_pub.find_deps", return_value=Path("/deps"))
    @patch("build.offline_pub.plan", return_value=offline_pub.Plan(Path("/seeded"), None, Path("/cargo_home")))
    def test_offline_points_cargo_at_deps_crates_and_turns_the_network_off(self, plan, _d, run, *_):
        seen = []
        run.side_effect = lambda *a, **k: seen.append(tuple(build.os.environ.get(key) for key in (
            "CARGO_HOME", "CARGO_NET_OFFLINE", "FLUTTER_STORAGE_BASE_URL", "FLUTTER_SUPPRESS_ANALYTICS")))
        with patch.dict("build.os.environ", {"CARGO_HOME": "/mine"}):
            build.build_desktop("windows", False, False)
            self.assertEqual(build.os.environ["CARGO_HOME"], "/mine")
        self.assertEqual(seen, [(str(Path("/cargo_home")), "true", build.NO_STORAGE, "true")] * 2)
        for key in ("CARGO_NET_OFFLINE", "FLUTTER_STORAGE_BASE_URL", "FLUTTER_SUPPRESS_ANALYTICS"):
            self.assertNotIn(key, build.os.environ)

    @patch("build.offline_pub.plan", return_value=offline_pub.Plan(None, "the deps branch's crates/ does not cover Cargo.lock: crates/x-1.0.0: missing"))
    def test_crates_the_deps_branch_lacks_stop_the_build_and_name_the_fix(self, plan, run, *_):
        with self.assertRaisesRegex(RuntimeError, r"crates/x-1\.0\.0.*mirror_crates\.py sync"):
            build.build_desktop("windows", False, False)
        run.assert_not_called()

    @patch("build.offline_pub.plan", return_value=offline_pub.Plan(None, "git dependencies are not in the pub cache"))
    def test_a_missing_dependency_stops_the_build_instead_of_going_online(self, plan, run, *_):
        with self.assertRaisesRegex(RuntimeError, "git dependencies are not in the pub cache"):
            build.build_desktop("windows", False, False)
        run.assert_not_called()

    @patch("build.offline_pub.find_deps", return_value=Path("/deps"))
    @patch("build.offline_pub.plan", return_value=offline_pub.Plan(Path("/seeded"), None))
    def test_a_failed_offline_resolution_stops_the_build(self, plan, _d, run, *_):
        def fail_offline(command, **_):
            if "--offline" in command:
                raise build.subprocess.CalledProcessError(1, command)

        run.side_effect = fail_offline
        with self.assertRaisesRegex(RuntimeError, "pub get --offline failed"):
            build.build_desktop("windows", False, False)
        self.assertEqual(self.commands(run), [["flutter", "pub", "get", "--offline"]])

    @patch("build.offline_pub.plan")
    def test_online_skips_deps(self, plan, run, *_):
        build.build_desktop("windows", False, False, online=True)
        plan.assert_not_called()
        self.assertEqual(self.commands(run), [["flutter", "build", "windows", "--release", *build.BRAND.defines()]])
        self.assertNotIn("FLUTTER_STORAGE_BASE_URL", build.os.environ)


@patch("build.build_noise")
@patch("build.finish_web")
@patch("build.flutter_program", return_value="flutter")
@patch("build.subprocess.run")
class WebModeTests(unittest.TestCase):
    """The PWA, the extension and the Office pane resolve like the desktop: from the deps branch, and never online unasked."""

    def setUp(self):
        for patcher in (patch.dict("build.os.environ", {"PUB_CACHE": "/user/cache", "CANOA_DEPS": "/deps"}),
                        patch("build.require_pinned_sdk")):
            patcher.start()
            self.addCleanup(patcher.stop)

    def commands(self, run):
        return [call.args[0] for call in run.call_args_list]

    @patch("build.offline_pub.find_deps", return_value=Path("/deps"))
    @patch("build.offline_pub.plan", return_value=offline_pub.Plan(Path("/seeded"), None))
    def test_the_pwa_resolves_offline_and_builds_without_resolving_again(self, plan, _d, run, _f, _fin, noise):
        for kind in ("pwa", "chromium"):
            run.reset_mock()
            with patch("sys.argv", ["build.py", kind]), patch("build.package_ui"):
                build.main()
            commands = self.commands(run)
            self.assertEqual(commands[0], ["flutter", "pub", "get", "--offline"])
            self.assertIn("--no-pub", commands[1])
            self.assertEqual(noise.call_args.args[0].deps, Path("/deps"), "the Noise wasm is built from the same deps checkout")

    @patch("build.offline_pub.plan", return_value=offline_pub.Plan(None, "the deps branch lacks hosted packages: web-1.1.1"))
    def test_a_deps_branch_that_lacks_a_package_stops_every_web_build(self, plan, run, _f, _fin, noise):
        for kind in ("pwa", "chromium", "office"):
            with patch("sys.argv", ["build.py", kind]), self.assertRaises(SystemExit) as stopped:
                build.main()
            self.assertIn("lacks hosted packages", str(stopped.exception))
        run.assert_not_called()
        noise.assert_not_called()

    @patch("build.offline_pub.plan")
    def test_online_is_the_one_way_to_resolve_from_the_network(self, plan, run, _f, _fin, noise):
        with patch("sys.argv", ["build.py", "pwa", "--online"]):
            build.main()
        plan.assert_not_called()
        self.assertEqual(self.commands(run)[0], ["flutter", "pub", "get"])
        self.assertEqual(noise.call_args.args, (None,))

    @patch("build.offline_pub.find_deps", return_value=Path("/deps"))
    @patch("build.offline_pub.plan", return_value=offline_pub.Plan(Path("/seeded"), None))
    def test_the_host_web_build_resolves_offline_too(self, plan, _d, run, _f, _fin, noise):
        with patch.object(Path, "is_file", return_value=True):
            build.build_web_for_host(build.Offline(Path("/deps"), Path("/seeded"), None))
        commands = self.commands(run)
        self.assertEqual(commands[0], ["flutter", "pub", "get", "--offline"])
        self.assertIn("--no-pub", commands[1])


class PinnedSdkTests(unittest.TestCase):
    def test_an_sdk_that_is_not_the_pin_stops_the_build_and_names_the_mirror_install(self):
        with tempfile.TemporaryDirectory() as tmp, patch("build.flutter_program", return_value=str(Path(tmp) / "bin" / "flutter")):
            with self.assertRaisesRegex(RuntimeError, r"(?s)is not a Flutter SDK.*flutter_pin\.py install"):
                build.require_pinned_sdk("web")

    def test_an_sdk_without_the_platforms_engine_stops_the_build(self):
        with tempfile.TemporaryDirectory() as tmp, patch("build.flutter_program", return_value=str(Path(tmp) / "bin" / "flutter")), \
                patch.object(build.flutter_pin, "sdk_problems", return_value=[]):
            with self.assertRaisesRegex(RuntimeError, "not precached for linux"):
                build.require_pinned_sdk("linux")

    def test_an_offline_build_has_no_storage_to_download_from(self):
        self.assertEqual(build.network_off(None), {})
        with patch.dict("build.os.environ", {"FLUTTER_STORAGE_BASE_URL": "http://127.0.0.1:5000"}):
            changes = build.network_off(build.Offline(Path("/d"), Path("/c"), None))
        self.assertEqual(changes["FLUTTER_STORAGE_BASE_URL"], "http://127.0.0.1:5000", "a loopback mirror the person serves is kept")


class PackagingTests(unittest.TestCase):
    @patch("build.shutil.which", return_value="/bin/flutter")
    def test_web_wasm_flag(self, _):
        self.assertIn("--wasm", flutter_args("pwa", True))
        self.assertNotIn("--no-pub", flutter_args("pwa", False))
        self.assertIn("--no-pub", flutter_args("pwa", False, build.Offline(Path("/d"), Path("/c"), None)))
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
        vendor = Path(__file__).resolve().parents[2] / "clients" / "browser" / "chromium" / "vendor"
        meta = json.loads((vendor / "COQUI-SOURCE.json").read_text())
        self.assertEqual(sorted(meta), ["coqui-peer-noise.mjs", "coqui-peer-webcrypto.mjs"])
        for name, pinned in meta.items():
            source = (vendor / name).read_bytes()
            self.assertEqual(
                hashlib.sha1(f"blob {len(source)}\0".encode() + source).hexdigest(),
                pinned["git_blob_sha1"],
                name,
            )

class FinishWebTests(unittest.TestCase):
    def test_the_service_worker_goes_and_a_missing_font_stops_the_build(self):
        with tempfile.TemporaryDirectory() as tmp:
            dist = Path(tmp)
            (dist / "flutter_service_worker.js").write_text("")
            with patch.object(build.fallback_fonts, "read_list", return_value={"a/v1/b.woff2": "0" * 64}), \
                    patch.object(build.fallback_fonts, "check_engine"), \
                    patch.object(build.offline_pub, "find_deps", return_value=None):
                with self.assertRaisesRegex(RuntimeError, "not all in the deps mirror"):
                    build.finish_web(dist)
            self.assertFalse((dist / "flutter_service_worker.js").exists())


class OfficeJsTests(unittest.TestCase):
    def test_the_cdn_choice_names_no_deps_checkout(self):
        self.assertEqual(build.office_js_environment("cdn"), {"CANOA_OFFICE_JS": "cdn"})

    def test_the_default_carries_office_js_from_the_deps_checkout_found(self):
        with patch.dict("os.environ", {}, clear=False) as env, patch("build.offline_pub.find_deps", return_value=Path("/deps")):
            env.pop("CANOA_DEPS", None)
            self.assertEqual(
                build.office_js_environment("bundled"),
                {"CANOA_OFFICE_JS": "bundled", "CANOA_DEPS": str(Path("/deps"))},
            )

    def test_a_deps_checkout_the_environment_names_is_left_as_it_is(self):
        with patch.dict("os.environ", {"CANOA_DEPS": "/mine"}), patch("build.offline_pub.find_deps") as find:
            self.assertEqual(build.office_js_environment("bundled"), {"CANOA_OFFICE_JS": "bundled"})
            find.assert_not_called()


class OfficeSiteTests(unittest.TestCase):
    def make(self, root: Path, body: bytes = b"office"):
        import hashlib
        import json

        web = root / "web"
        (web / "icons").mkdir(parents=True)
        (web / "index.html").write_text("<base href=\"/office/app/\">")
        (web / "main.dart.js.map").write_text("map")
        (web / "icons" / "Icon-192.png").write_bytes(b"png")
        pane = root / "taskpane"
        pane.mkdir()
        (pane / "taskpane.html").write_text('<script src="@OFFICE_JS@"></script>\n')
        (pane / "taskpane.js").write_text("export {};\n")
        mirror = root / "mirror"
        (mirror / "dist").mkdir(parents=True)
        (mirror / "dist" / "office.js").write_bytes(body)
        (mirror / "LICENSE.md").write_bytes(b"license")
        listing = root / "office_js.json"
        listing.write_text(json.dumps({
            "carried": ["dist/office.js", "LICENSE.md"],
            "files": {"dist/office.js": hashlib.sha256(b"office").hexdigest(), "LICENSE.md": hashlib.sha256(b"license").hexdigest()},
        }))
        return web, pane, mirror, listing

    def test_the_site_holds_the_pages_the_app_the_library_and_the_icons_under_office(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            web, pane, mirror, listing = self.make(root)
            target = build.package_office(web, root / "out", mirror, pane, listing)
            office = target / "office"
            self.assertEqual((office / "taskpane.html").read_text(), '<script src="js/office.js"></script>\n')
            self.assertTrue((office / "taskpane.js").is_file())
            self.assertTrue((office / "app" / "index.html").is_file())
            self.assertFalse((office / "app" / "main.dart.js.map").exists())
            self.assertEqual((office / "js" / "office.js").read_bytes(), b"office")
            self.assertEqual((office / "js" / "LICENSE.md").read_bytes(), b"license")
            for size in build.ICON_SIZES:
                self.assertTrue((office / f"icon-{size}.png").is_file())

    def test_a_library_file_that_is_not_the_pinned_one_stops_the_build(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            web, pane, mirror, listing = self.make(root, body=b"tampered")
            with self.assertRaisesRegex(RuntimeError, "pins"):
                build.package_office(web, root / "out", mirror, pane, listing)

    def test_a_library_file_missing_from_the_mirror_stops_the_build(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            web, pane, mirror, listing = self.make(root)
            (mirror / "LICENSE.md").unlink()
            with self.assertRaisesRegex(RuntimeError, "not in the Office.js mirror"):
                build.package_office(web, root / "out", mirror, pane, listing)

    def test_the_real_pages_name_only_what_the_site_carries(self):
        import re

        pane = build.ROOT / "clients" / "office" / "taskpane"
        for page in pane.glob("*.html"):
            for source in re.findall(r'(?:src|href)="([^"]+)"', page.read_text(encoding="utf-8")):
                self.assertTrue(source == "@OFFICE_JS@" or (pane / source).is_file(), f"{page.name}: {source}")


if __name__ == "__main__":
    unittest.main()
