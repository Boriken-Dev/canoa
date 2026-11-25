"""No build in this folder goes online unless it is asked to with --online.

The mirror's own tools (tools/mirror_*.py, cdn_files.py, fallback_fonts.py, office_js.py, flutter_pin.py install) are the
only code that fetches; these tests hold the builds to the deps checkout and the pinned SDK, so a fallback to the network
that comes back fails here.
"""
import ast
import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import build
import build_noise
import offline_pub

HERE = Path(__file__).resolve().parent
NETWORK_MODULES = {"urllib", "http", "socket", "ssl", "ftplib", "smtplib", "requests", "aiohttp", "httpx"}
# The one build script that may reach the network, and only from its `fetch` command (see FetchIsOnlyCommandTests).
MAY_IMPORT_URLLIB = {"portable.py"}


def build_scripts():
    return [p for p in sorted(HERE.glob("*.py")) if not p.name.startswith("test_")]


class NoNetworkLibraryTests(unittest.TestCase):
    def test_a_build_script_imports_no_network_library(self):
        for path in build_scripts():
            tree = ast.parse(path.read_text(encoding="utf-8"))
            imported = {
                (node.module or "").split(".")[0] if isinstance(node, ast.ImportFrom) else alias.name.split(".")[0]
                for node in ast.walk(tree) if isinstance(node, (ast.Import, ast.ImportFrom))
                for alias in (node.names if isinstance(node, ast.Import) else [None])
            }
            allowed = {"urllib"} if path.name in MAY_IMPORT_URLLIB else set()
            self.assertEqual((imported & NETWORK_MODULES) - allowed, set(), f"{path.name} reaches the network by itself")

    def test_a_build_script_names_no_download_program_and_never_falls_back(self):
        for path in build_scripts():
            text = path.read_text(encoding="utf-8")
            self.assertIsNone(re.search(r"\b(curl|wget|Invoke-WebRequest)\b", text), f"{path.name} downloads with a program")
            self.assertNotIn("Building online", text, f"{path.name} falls back to the network")
            self.assertNotIn('"auto"', text, f"{path.name} has an automatic online mode")

    def test_the_appimage_tools_are_fetched_only_by_the_fetch_command(self):
        text = (HERE / "portable.py").read_text(encoding="utf-8")
        tree = ast.parse(text)
        callers = {
            function.name for function in ast.walk(tree) if isinstance(function, ast.FunctionDef)
            for call in ast.walk(function) if isinstance(call, ast.Call)
            and getattr(call.func, "id", None) == "fetch_pinned"
        }
        self.assertEqual(callers - {"fetch_pinned"}, {"fetch_tools"})
        appimage = next(n for n in ast.walk(tree) if isinstance(n, ast.FunctionDef) and n.name == "build_appimage")
        called = {getattr(c.func, "id", None) for c in ast.walk(appimage) if isinstance(c, ast.Call)}
        self.assertNotIn("fetch_pinned", called)
        self.assertNotIn("fetch_tools", called)


class RecordedBuild:
    """Runs a build with every subprocess recorded, and the environment each one would have seen."""

    def __init__(self):
        self.commands: list[list[str]] = []
        self.environments: list[dict[str, str]] = []

    def run(self, command, **kwargs):
        self.commands.append([str(part) for part in command])
        self.environments.append(dict(kwargs.get("env") or os.environ))
        return subprocess.CompletedProcess(command, 0, stdout="", stderr="")


class OfflineInvariantTests(unittest.TestCase):
    """Whatever kind is built, an offline run starts only commands that cannot reach the network."""

    def setUp(self):
        plan = offline_pub.Plan(Path("/seeded"), None, Path("/cargo_home"))
        for patcher in (
            patch.dict("build.os.environ", {"PUB_CACHE": "/user/cache", "CANOA_DEPS": "/deps"}),
            patch("build.require_pinned_sdk"),
            patch("build.require_tools"),
            patch("build.flutter_program", return_value="flutter"),
            patch("build.offline_pub.plan", return_value=plan),
            patch("build.offline_pub.find_deps", return_value=Path("/deps")),
            patch("build.finish_web"),
            patch("build.package_ui"),
            patch("build.package_office"),
            patch("build.ensure_runner"),
            patch("build.restore_lock", return_value=False),
            patch("build.built_executable", return_value=Path("CanoaUI.exe")),
            patch("build.office_js_folder", return_value=Path("/deps/cdn/office-js")),
        ):
            patcher.start()
            self.addCleanup(patcher.stop)
        self.recorded = RecordedBuild()
        patcher = patch("build.subprocess.run", side_effect=self.recorded.run)
        patcher.start()
        self.addCleanup(patcher.stop)

    def assert_offline(self):
        self.assertTrue(self.recorded.commands, "the build ran nothing")
        for command, env in zip(self.recorded.commands, self.recorded.environments):
            joined = " ".join(command)
            if command[:3] == ["flutter", "pub", "get"]:
                self.assertIn("--offline", command, joined)
            if command[:2] == ["flutter", "create"]:
                self.fail(f"{joined}: the runner is made by ensure_runner, which is patched here")
            if command[:2] in (["flutter", "build"], ["flutter", "run"]):
                self.assertIn("--no-pub", command, f"{joined} would resolve packages itself")
            if command[0] == "flutter":
                self.assertEqual(env.get("PUB_CACHE"), str(Path("/seeded")), joined)
                self.assertEqual(env.get("CARGO_NET_OFFLINE"), "true", joined)
                self.assertEqual(env.get("FLUTTER_SUPPRESS_ANALYTICS"), "true", joined)
                self.assertNotIn("googleapis", env.get("FLUTTER_STORAGE_BASE_URL", "googleapis-unset"), joined)
            if "build_noise.py" in joined:
                self.assertIn("--deps", command, joined)
                self.assertNotIn("--online", command, joined)

    def test_every_web_kind(self):
        for kind in ("pwa", "chromium", "office"):
            self.recorded.commands.clear()
            self.recorded.environments.clear()
            with patch("sys.argv", ["build.py", kind]):
                build.main()
            self.assert_offline()

    def test_the_desktop_build_and_its_run(self):
        for debug, run in ((False, False), (True, True)):
            self.recorded.commands.clear()
            self.recorded.environments.clear()
            build.build_desktop("windows", debug, run)
            self.assert_offline()

    def test_the_host_web_build(self):
        offline = build.Offline(Path("/deps"), Path("/seeded"), Path("/cargo_home"))
        with patch.object(Path, "is_file", return_value=True), build.environment(build.network_off(offline)):
            build.build_web_for_host(offline)
        self.assert_offline()

    def test_a_portable_build_keeps_the_offline_environment_for_packaging(self):
        seen = []
        with patch("build.build_web_for_host", return_value=Path("app/build/web")), \
                patch("build.package_portable", side_effect=lambda *a: seen.append(dict(os.environ)) or Path("x")):
            build.build_desktop("windows", False, False, portable=True)
        self.assertEqual(seen[0].get("CARGO_NET_OFFLINE"), "true")
        self.assertEqual(seen[0].get("CARGO_HOME"), str(Path("/cargo_home")))


class NoiseTests(unittest.TestCase):
    def test_without_a_deps_checkout_the_build_stops_and_does_not_reach_for_crates_io(self):
        with patch("sys.argv", ["build_noise.py"]), patch.dict("os.environ", {}, clear=False) as env, \
                patch("build_noise.offline_pub.find_deps", return_value=None), patch("build_noise.build") as built:
            env.pop("CANOA_DEPS", None)
            with self.assertRaises(SystemExit) as stopped:
                build_noise.main()
        self.assertIn("no deps checkout", str(stopped.exception))
        built.assert_not_called()

    def test_online_is_explicit_and_excludes_the_deps_options(self):
        with patch("sys.argv", ["build_noise.py", "--online", "--deps", "/d"]), self.assertRaises(SystemExit):
            build_noise.main()
        with patch("sys.argv", ["build_noise.py", "--online", "--install-bindgen"]), self.assertRaises(SystemExit):
            build_noise.main()
        with patch("sys.argv", ["build_noise.py", "--online"]), patch("build_noise.build", return_value=Path(__file__)) as built:
            build_noise.main()
        self.assertIsNone(built.call_args.args[2])

    def test_a_missing_wasm_target_is_named_not_installed(self):
        with tempfile.TemporaryDirectory() as tmp, patch("build_noise.subprocess.run") as run:
            run.return_value = subprocess.CompletedProcess([], 0, stdout=tmp + "\n")
            with self.assertRaisesRegex(RuntimeError, "rustup target add wasm32-unknown-unknown"):
                build_noise.require_wasm_target()


if __name__ == "__main__":
    unittest.main()
