import hashlib
import http.server
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

import portable


class VersionTests(unittest.TestCase):
    def test_the_workspace_version_is_read_from_cargo_toml(self):
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "Cargo.toml").write_text('[workspace.package]\nversion = "1.2.3-x"\n')
            self.assertEqual(portable.workspace_version(Path(tmp)), "1.2.3-x")
        root = Path(__file__).resolve().parents[2]
        self.assertRegex(portable.workspace_version(root), r"^\d+\.\d+\.\d+")


class AppDirTests(unittest.TestCase):
    def test_the_bundle_becomes_an_appdir_with_its_entry_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            bundle = tmp / "bundle"
            (bundle / "lib").mkdir(parents=True)
            (bundle / "CanoaUI").write_text("elf")
            (bundle / "lib" / "libcanoa_ffi.so").write_text("lib")
            icon = tmp / "canoa.svg"
            icon.write_text("<svg/>")
            appdir = tmp / "Canoa.AppDir"
            appdir.mkdir()
            (appdir / "stale").write_text("left from an earlier build")
            portable.make_appdir(bundle, appdir, icon)
            self.assertFalse((appdir / "stale").exists())
            self.assertTrue((appdir / "usr/bin/CanoaUI").is_file())
            self.assertTrue((appdir / "usr/bin/lib/libcanoa_ffi.so").is_file())
            self.assertTrue((appdir / "canoa.svg").is_file())
            self.assertIn("Exec=CanoaUI", (appdir / "canoa.desktop").read_text())
            self.assertIn("Icon=canoa", (appdir / "canoa.desktop").read_text())
            run = (appdir / "AppRun").read_bytes()
            self.assertTrue(run.startswith(b"#!/bin/sh\n") and b"\r" not in run)

    def test_a_folder_without_the_program_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(RuntimeError, "holds no CanoaUI"):
                portable.make_appdir(Path(tmp), Path(tmp) / "out", Path(tmp) / "i.svg")


class _Handler(http.server.BaseHTTPRequestHandler):
    body = b"tool bytes"

    def do_GET(self):  # noqa: N802
        self.send_response(200)
        self.end_headers()
        self.wfile.write(self.body)

    def log_message(self, *args):
        pass


class FetchTests(unittest.TestCase):
    def serve(self):
        server = http.server.HTTPServer(("127.0.0.1", 0), _Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        return f"http://127.0.0.1:{server.server_port}/tool"

    def test_a_download_with_the_pinned_digest_is_kept_and_not_fetched_again(self):
        url = self.serve()
        pinned = hashlib.sha256(_Handler.body).hexdigest()
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp) / "tools" / "tool"
            self.assertEqual(portable.fetch_pinned(url, pinned, dest), dest)
            self.assertEqual(dest.read_bytes(), _Handler.body)
            with patch("portable.urllib.request.urlopen", side_effect=AssertionError("fetched again")):
                portable.fetch_pinned(url, pinned, dest)

    def test_a_download_with_another_digest_is_refused_and_leaves_nothing(self):
        url = self.serve()
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp) / "tool"
            with self.assertRaisesRegex(RuntimeError, "not the pinned"):
                portable.fetch_pinned(url, "0" * 64, dest)
            self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_a_cached_file_that_no_longer_matches_is_fetched_again(self):
        url = self.serve()
        pinned = hashlib.sha256(_Handler.body).hexdigest()
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp) / "tool"
            dest.write_bytes(b"tampered")
            portable.fetch_pinned(url, pinned, dest)
            self.assertEqual(dest.read_bytes(), _Handler.body)


class PinTests(unittest.TestCase):
    def test_every_pinned_tool_has_a_runtime_and_a_digest(self):
        self.assertEqual(portable.APPIMAGETOOL.keys(), portable.RUNTIME.keys())
        for table in (portable.APPIMAGETOOL, portable.RUNTIME):
            for _name, digest in table.values():
                self.assertRegex(digest, r"^[0-9a-f]{64}$")
        self.assertTrue(portable.APPIMAGETOOL_RELEASE.startswith("https://"))
        self.assertTrue(portable.RUNTIME_RELEASE.startswith("https://"))


class AppImageBuildTests(unittest.TestCase):
    @patch("portable.subprocess.run")
    @patch("portable.make_appdir")
    @patch("portable.fetch_pinned", side_effect=lambda url, sha, dest: dest)
    def test_appimagetool_gets_a_private_temporary_folder_under_target(self, _fetch, _appdir, run):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "target" / "appimage-tmp").mkdir(parents=True, mode=0o755)
            with patch.object(Path, "chmod"):
                portable.build_appimage(root / "bundle", root, root / "out", "x86_64")
            env = run.call_args.kwargs["env"]
            private = root / "target" / "appimage-tmp"
            self.assertEqual(env["TMPDIR"], str(private))
            self.assertTrue(private.is_dir())
        # The folder is made 0700 even when it already existed with wider rights.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "target" / "appimage-tmp").mkdir(parents=True, mode=0o755)
            with patch("portable.subprocess.run"), patch("portable.make_appdir"), patch(
                "portable.fetch_pinned", side_effect=lambda url, sha, dest: dest
            ), patch("portable.Path.chmod", autospec=True) as chmod:
                portable.build_appimage(root / "bundle", root, root / "out", "x86_64")
            modes = {(call.args[0].name, call.args[1]) for call in chmod.call_args_list}
            self.assertIn(("appimage-tmp", 0o700), modes)


class WindowsLauncherTests(unittest.TestCase):
    def test_a_folder_without_the_program_is_refused_before_anything_is_built(self):
        with tempfile.TemporaryDirectory() as tmp, patch("portable.subprocess.run") as run:
            with self.assertRaisesRegex(RuntimeError, "holds no CanoaUI.exe"):
                portable.build_windows_launcher(Path(tmp), Path(tmp), Path(tmp) / "out", "1.0.0")
            run.assert_not_called()

    @patch("portable.cargo_program", return_value="cargo")
    @patch("portable.subprocess.run")
    def test_it_packs_then_builds_the_launcher_around_the_archive(self, run, _cargo):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            folder = tmp / "Release"
            folder.mkdir()
            (folder / "CanoaUI.exe").write_text("exe")
            (tmp / "target" / "release").mkdir(parents=True)
            (tmp / "target" / "release" / "canoa-launch.exe").write_text("launcher")
            out = tmp / "target" / "canoa-windows"
            out.mkdir()
            (out / "stale.dll").write_text("the old folder form")
            exe = portable.build_windows_launcher(folder, tmp, out, "4.5.6")
            pack, build = run.call_args_list
            self.assertIn("canoa-pack", pack.args[0])
            self.assertEqual(pack.args[0][-3:], [str(folder), "4.5.6", str(tmp / "target" / "portable" / "payload.zip")])
            self.assertEqual(build.kwargs["env"]["CANOA_LAUNCH_PAYLOAD"], str(tmp / "target" / "portable" / "payload.zip"))
            # From the crate's folder, where cargo reads the static-runtime setting that is scoped to it.
            for call in (pack, build):
                self.assertEqual(call.kwargs["cwd"], tmp / "crates" / "canoa-launch")
            self.assertEqual(exe, out / "CanoaUI.exe")
            self.assertEqual(sorted(p.name for p in out.iterdir()), ["CanoaUI.exe"], "one file replaces the folder")
            self.assertEqual(exe.read_text(), "launcher")


if __name__ == "__main__":
    unittest.main()
