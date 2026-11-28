"""Tests of tools/dev.py: no network, every tree a temporary one."""
import argparse
import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import dev  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
COMMIT = "0123456789abcdef0123456789abcdef01234567"


def fake_repo(base: Path) -> Path:
    """The files of the repository dev.py reads."""
    (base / "tools").mkdir(parents=True)
    (base / "app" / "tool").mkdir(parents=True)
    (base / "tools" / "flutter.pin").write_text(f"# c\nFLUTTER_COMMIT={COMMIT}\nFLUTTER_VERSION=1.0\n", encoding="utf-8")
    (base / "tools" / "agent_skills.txt").write_text((REPO / "tools" / "agent_skills.txt").read_text(encoding="utf-8"), encoding="utf-8")
    (base / "app" / "tool" / "envfile.py").write_text((REPO / "app" / "tool" / "envfile.py").read_text(encoding="utf-8"), encoding="utf-8")
    (base / "app" / "pubspec.lock").write_bytes(b"lock\n")
    return base


def fake_sdk(prefix: Path, tools: bool = True) -> Path:
    sdk = prefix / "git" / "flutter" / COMMIT[:12]
    (sdk / "bin" / "cache" / "dart-sdk" / "bin").mkdir(parents=True)
    dev.dart_program(sdk).write_text("", encoding="utf-8")
    dev.flutter_script(sdk).write_text("", encoding="utf-8")
    if tools:
        (sdk / "bin" / "cache" / "flutter_tools.snapshot").write_text("", encoding="utf-8")
        (sdk / "bin" / "cache" / "flutter_tools.stamp").write_text("", encoding="utf-8")
    return sdk


class Case(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name)
        self.root = fake_repo(self.base / "repo")
        self.prefix = self.base / "sdk"


class EnvFile(Case):
    def test_the_persons_values_are_kept_and_missing_keys_added(self):
        env = self.root / ".env"
        env.write_text("PUB_CACHE=/mine\nOTHER=1", encoding="utf-8")
        notes = dev.merge_env(env, {"PUB_CACHE": "/ours", "FLUTTER_ROOT": "/sdk"}, {})
        self.assertEqual(env.read_text(encoding="utf-8"), "PUB_CACHE=/mine\nOTHER=1\nFLUTTER_ROOT=/sdk\n")
        self.assertTrue(any("kept your PUB_CACHE=/mine" in n for n in notes))
        self.assertTrue(any("added FLUTTER_ROOT" in n for n in notes))

    def test_a_second_merge_changes_nothing(self):
        env = self.root / ".env"
        dev.merge_env(env, {"FLUTTER_ROOT": "/sdk"}, {})
        first = env.read_bytes()
        dev.merge_env(env, {"FLUTTER_ROOT": "/sdk"}, {})
        self.assertEqual(env.read_bytes(), first)
        self.assertNotIn(b"\r", first)


class Env(Case):
    def values(self, environ=None):
        sdk = fake_sdk(self.prefix)
        return sdk, dev.env_values(self.root, environ or {}, self.prefix)

    def test_values_and_the_sdk_first_on_path(self):
        sdk, (values, paths) = self.values()
        self.assertEqual(values["FLUTTER_ROOT"], str(sdk))
        self.assertEqual(values["CANOA_OFFICE_JS"], "cdn")
        self.assertTrue(values["CANOA_WEB_DIST"].endswith("tests/site") or values["CANOA_WEB_DIST"].endswith("tests\\site"))
        self.assertEqual(paths[0], str(sdk / "bin"))

    def test_deps_replaces_the_office_js_setting(self):
        (self.root / ".env").write_text("CANOA_DEPS=/deps\nPUB_CACHE=/pub\n", encoding="utf-8")
        _, (values, _) = self.values()
        self.assertEqual(values["CANOA_DEPS"], "/deps")
        self.assertEqual(values["PUB_CACHE"], "/pub")
        self.assertNotIn("CANOA_OFFICE_JS", values)

    def test_each_shell(self):
        values, paths = {"FLUTTER_ROOT": "/s dk", "CANOA_OFFICE_JS": "cdn"}, ["/s dk/bin", "/s dk/dart"]
        bash = dev.env_lines(values, paths, "bash")
        self.assertEqual(bash[0], "export FLUTTER_ROOT='/s dk'")
        self.assertEqual(bash[-1], "export PATH='/s dk/bin:/s dk/dart':\"$PATH\"")
        pwsh = dev.env_lines(values, paths, "pwsh")
        self.assertEqual(pwsh[0], "$env:FLUTTER_ROOT = '/s dk'")
        self.assertTrue(pwsh[-1].startswith("$env:PATH = '") and pwsh[-1].endswith("$env:PATH"))
        cmd = dev.env_lines(values, paths, "cmd")
        self.assertEqual(cmd[1], 'set "CANOA_OFFICE_JS=cdn"')
        self.assertTrue(cmd[-1].endswith(';%PATH%"'))
        with self.assertRaises(dev.Problem):
            dev.env_lines(values, paths, "fish")

    def test_a_windows_path_in_bash_is_spelled_for_git_bash(self):
        self.assertEqual(dev.posix("C:\\Users\\a b"), "/c/Users/a b")
        bash = dev.env_lines({"X": "C:\\a b"}, ["C:\\sdk\\bin"], "bash")
        self.assertEqual(bash, ["export X='/c/a b'", 'export PATH=/c/sdk/bin:"$PATH"'])

    def test_quotes_in_a_value_are_escaped(self):
        self.assertIn("'it'\"'\"'s'", dev.env_lines({"X": "it's"}, [], "bash")[0])
        self.assertEqual(dev.env_lines({"X": "it's"}, [], "pwsh")[0], "$env:X = 'it''s'")


class Agents(Case):
    def test_mcp_json_names_the_pinned_dart_and_the_disabled_tools(self):
        sdk = fake_sdk(self.prefix)
        config = dev.mcp_config(sdk, {"mcpServers": {"mine": {"command": "x"}}, "other": 1})
        server = config["mcpServers"]["dart"]
        self.assertEqual(server["command"], str(sdk / "bin" / "cache" / "dart-sdk" / "bin" / Path(dev.dart_program(sdk)).name))
        self.assertEqual(server["args"], ["mcp-server", "--flutter-sdk", str(sdk), "--disable", "pub,pub_dev_search,create_project"])
        self.assertEqual(config["mcpServers"]["mine"], {"command": "x"})
        self.assertEqual(config["other"], 1)

    def test_codex_block_is_replaced_not_repeated(self):
        sdk = fake_sdk(self.prefix)
        once = dev.merge_codex("model = 'x'", dev.codex_block(sdk))
        twice = dev.merge_codex(once, dev.codex_block(sdk))
        self.assertEqual(once, twice)
        self.assertEqual(twice.count("[mcp_servers.dart]"), 1)
        self.assertTrue(twice.startswith("model = 'x'\n"))
        self.assertIn("'--disable', 'pub,pub_dev_search,create_project'", twice)

    def test_skill_list_parses(self):
        rows = dev.parse_skills((REPO / "tools" / "agent_skills.txt").read_text(encoding="utf-8"))
        self.assertEqual(len(rows), 7)
        self.assertIn(("flutter/agent-plugins", "main", "flutter-add-widget-test"), rows)
        self.assertIn(("dart-lang/skills", "main", "dart-add-unit-test"), rows)
        self.assertEqual(dev.parse_skills("# c\n\na/b main x # note\n"), [("a/b", "main", "x")])

    def test_a_bad_skill_line_names_its_number(self):
        with self.assertRaisesRegex(dev.Problem, r"agent_skills.txt:2"):
            dev.parse_skills("a/b main x\nnot a line\n")

    def test_skills_are_copied_from_a_sparse_clone(self):
        calls = []

        def run(command, cwd, env):
            calls.append(list(command))
            if command[1] == "clone":
                clone = Path(command[-1])
                for name in ("one", "two"):
                    (clone / "skills" / name).mkdir(parents=True)
                    (clone / "skills" / name / "SKILL.md").write_text(name, encoding="utf-8")
            return 0

        names = dev.fetch_skills([("o/r", "main", "one"), ("o/r", "main", "two")], self.root / ".claude" / "skills", run)
        self.assertEqual(names, ["one", "two"])
        self.assertEqual(len(calls), 2)
        self.assertIn("skills/one", calls[1])
        self.assertIn("--depth", calls[0])
        self.assertEqual((self.root / ".claude" / "skills" / "two" / "SKILL.md").read_text(encoding="utf-8"), "two")

    def test_write_agents_without_a_fetch_writes_both_server_files(self):
        sdk = fake_sdk(self.prefix)
        dev.write_agents(self.root, sdk, lambda *a: 0, fetch=False)
        self.assertEqual(json.loads((self.root / ".mcp.json").read_text(encoding="utf-8"))["mcpServers"]["dart"]["args"][0], "mcp-server")
        self.assertIn("[mcp_servers.dart]", (self.root / ".codex" / "config.toml").read_text(encoding="utf-8"))


class Check(Case):
    def args(self):
        return argparse.Namespace(prefix=str(self.prefix))

    def run_check(self, exit_code=0, environ=None):
        calls = []

        def run(command, cwd, env):
            calls.append(list(command))
            return exit_code

        err = io.StringIO()
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
            code = dev.check(self.root, self.args(), run, environ or {})
        return code, err.getvalue(), calls

    def test_a_complete_tree_passes(self):
        fake_sdk(self.prefix)
        code, err, calls = self.run_check()
        self.assertEqual((code, err), (0, ""))
        self.assertEqual(sum("precache-check" in c for c in calls), 1)
        self.assertEqual(sum("--check" in c for c in calls), 1)

    def test_no_sdk_fails(self):
        code, err, _ = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("is not there", err)

    def test_a_missing_tool_snapshot_fails(self):
        fake_sdk(self.prefix, tools=False)
        code, err, _ = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("flutter_tools.snapshot", err)

    def test_the_installers_verdict_and_the_pin_check_count(self):
        fake_sdk(self.prefix)
        code, err, _ = self.run_check(exit_code=1)
        self.assertEqual(code, 1)
        self.assertIn("sdk install --check", err)
        self.assertIn("not the SDK tools/sdk.lock names", err)

    def test_an_env_key_naming_a_missing_folder_fails(self):
        fake_sdk(self.prefix)
        (self.root / ".env").write_text(f"PUB_CACHE={self.base / 'nowhere'}\n", encoding="utf-8")
        code, err, _ = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn(".env: PUB_CACHE", err)

    def test_agent_files_are_checked_only_when_present(self):
        sdk = fake_sdk(self.prefix)
        (self.root / ".mcp.json").write_text(json.dumps({"mcpServers": {"dart": {"command": str(self.base / "gone")}}}), encoding="utf-8")
        (self.root / ".claude" / "skills").mkdir(parents=True)
        code, err, _ = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn(".mcp.json", err)
        self.assertIn(".claude/skills/flutter-add-widget-test", err)
        dev.write_agents(self.root, sdk, lambda *a: 0, fetch=False)
        for _, _, name in dev.parse_skills((self.root / "tools" / "agent_skills.txt").read_text(encoding="utf-8")):
            (self.root / ".claude" / "skills" / name).mkdir()
            (self.root / ".claude" / "skills" / name / "SKILL.md").write_text("", encoding="utf-8")
        self.assertEqual(self.run_check()[0], 0)


class Setup(Case):
    def run_setup(self, tools_built=True, lock_changes=False, **options):
        sdk = fake_sdk(self.prefix, tools=tools_built)
        calls = []

        def run(command, cwd, env):
            calls.append((list(command), cwd))
            if command[-1] == "--version":
                (sdk / "bin" / "cache" / "flutter_tools.snapshot").write_text("", encoding="utf-8")
                (sdk / "bin" / "cache" / "flutter_tools.stamp").write_text("", encoding="utf-8")
            if lock_changes and "pub" in command:
                (self.root / "app" / "pubspec.lock").write_bytes(b"changed\n")
            return 0

        args = argparse.Namespace(prefix=str(self.prefix), platforms="web", deps=None, pub_cache=None, agents=False, **options)
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
            try:
                code = dev.setup(self.root, args, run, {})
            except dev.Problem as problem:
                code = str(problem)
        return code, calls

    def test_setup_installs_records_builds_the_tool_and_resolves(self):
        code, calls = self.run_setup(tools_built=False)
        self.assertEqual(code, 0)
        install = calls[0][0]
        self.assertEqual(install[3:5], ["sdk", "install"])
        self.assertIn("core,app", install)
        self.assertEqual(install[install.index("--precache") + 1], "web")
        self.assertTrue(calls[1][0][-1] == "--version")
        self.assertEqual(calls[2][0][-2:], ["get", "--enforce-lockfile"])
        self.assertEqual(calls[2][1], self.root / "app")
        self.assertIn(f"FLUTTER_ROOT={self.prefix / 'git' / 'flutter' / COMMIT[:12]}", (self.root / ".env").read_text(encoding="utf-8"))

    def test_a_built_tool_is_not_built_again(self):
        _, calls = self.run_setup()
        self.assertFalse(any(c[0][-1] == "--version" for c in calls))

    def test_a_changed_lockfile_is_restored_and_fails(self):
        code, _ = self.run_setup(lock_changes=True)
        self.assertIn("pubspec.lock", code)
        self.assertEqual((self.root / "app" / "pubspec.lock").read_bytes(), b"lock\n")


class Whole(unittest.TestCase):
    def test_the_real_pin_gives_a_folder_below_the_prefix(self):
        folder = dev.pinned_flutter(REPO, Path("/p"))
        self.assertEqual(folder.parent, Path("/p/git/flutter"))
        self.assertEqual(len(folder.name), 12)

    def test_help_runs(self):
        with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(SystemExit) as stop:
            dev.main(["env", "--help"])
        self.assertEqual(stop.exception.code, 0)


if __name__ == "__main__":
    unittest.main()
