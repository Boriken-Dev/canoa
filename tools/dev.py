#!/usr/bin/env python3
"""Sets a clone up for development on the SDK every build of this repository uses.

    python tools/dev.py setup [--prefix DIR] [--platforms web,windows] [--deps DIR] [--pub-cache DIR] [--agents]
    python tools/dev.py env [--shell bash|pwsh|cmd]
    python tools/dev.py check
    python tools/dev.py agents

`setup` runs `tools/build.py sdk install` for the core and app groups (the Rust toolchains, the wasm target and the
pinned Flutter), records where they are in the repo's .env, builds the Flutter tool and runs `flutter pub get
--enforce-lockfile` in app/. `env` prints the lines that put that SDK first on PATH. `check` lists what is missing or
stale and exits 1 if anything is. `agents` (also `setup --agents`) writes the Dart MCP server entry for Claude Code
(.mcp.json) and Codex (.codex/config.toml) and copies the skills tools/agent_skills.txt names; all of it is generated
and ignored by git. Standard library only, Python 3.11 or newer.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Callable, Mapping, Sequence

ROOT = Path(__file__).resolve().parent.parent
WINDOWS = sys.platform == "win32"
GROUPS = "core,app"
MCP_DISABLED = "pub,pub_dev_search,create_project"
SKILLS_FILE = "tools/agent_skills.txt"
CODEX_BEGIN = "# begin tools/dev.py"
CODEX_END = "# end tools/dev.py"

# Runs a command and returns its exit code; tests replace it.
Run = Callable[[Sequence[str], Path, Mapping[str, str]], int]


class Problem(Exception):
    """A step that cannot go on; the message says what to do."""


def load_envfile(root: Path):
    """app/tool/envfile.py, the module build.py reads .env with: one parser for both."""
    spec = importlib.util.spec_from_file_location("canoa_envfile", root / "app" / "tool" / "envfile.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def read_pin(root: Path) -> dict[str, str]:
    """KEY=VALUE lines of tools/flutter.pin."""
    pin = {}
    for line in (root / "tools" / "flutter.pin").read_text(encoding="utf-8").splitlines():
        match = re.match(r"^([A-Z_]+)=(\S+)$", line)
        if match:
            pin[match.group(1)] = match.group(2)
    return pin


def default_prefix(environ: Mapping[str, str]) -> Path:
    """Where build.py puts the SDKs: $BUILD_SDK_HOME, else ~/.canoa-sdk (the name does not follow the checkout's folder)."""
    return Path(environ["BUILD_SDK_HOME"]) if environ.get("BUILD_SDK_HOME") else Path.home() / ".canoa-sdk"


def pinned_flutter(root: Path, prefix: Path) -> Path:
    """Where `sdk install` clones the Flutter commit tools/flutter.pin names."""
    return prefix / "git" / "flutter" / read_pin(root)["FLUTTER_COMMIT"][:12]


def flutter_script(sdk: Path) -> Path:
    return sdk / "bin" / ("flutter.bat" if WINDOWS else "flutter")


def dart_program(sdk: Path) -> Path:
    return sdk / "bin" / "cache" / "dart-sdk" / "bin" / ("dart.exe" if WINDOWS else "dart")


def settings(root: Path, environ: Mapping[str, str]) -> dict[str, str]:
    """.env with the process environment over it, as app/tool/build.py reads it."""
    return load_envfile(root).load_settings(root, environ)


def sdk_of(root: Path, environ: Mapping[str, str], prefix: Path | None = None) -> Path:
    """The Flutter SDK in use: FLUTTER_ROOT from .env or the environment, else the pinned one."""
    named = settings(root, environ).get("FLUTTER_ROOT")
    return Path(named) if named else pinned_flutter(root, prefix or default_prefix(environ))


# .env

def merge_env(path: Path, wanted: Mapping[str, str], environ: Mapping[str, str]) -> list[str]:
    """Add the wanted keys the file lacks; a key the person already set is left as it is. Returns one line per key."""
    envfile = load_envfile(path.parent)
    text = path.read_text(encoding="utf-8") if path.is_file() else ""
    have = envfile.parse_env(text, environ)
    notes, added = [], []
    for key, value in wanted.items():
        if key not in have:
            added.append(f"{key}={value}")
            notes.append(f".env: added {key}={value}")
        elif have[key] == value:
            notes.append(f".env: {key} is already {value}")
        else:
            notes.append(f".env: kept your {key}={have[key]} (this setup would use {value}; edit .env to change it)")
    if added:
        if text and not text.endswith("\n"):
            text += "\n"
        path.write_text(text + "".join(line + "\n" for line in added), encoding="utf-8", newline="\n")
    return notes


# env

def posix(path: str) -> str:
    """A Windows path as Git Bash spells it (C:\\a\\b as /c/a/b); other platforms' paths as they are."""
    match = re.match(r"^([A-Za-z]):[\\/](.*)$", path)
    if not match:
        return path.replace("\\", "/") if WINDOWS else path
    return "/" + match.group(1).lower() + "/" + match.group(2).replace("\\", "/")


def env_values(root: Path, environ: Mapping[str, str], prefix: Path | None = None) -> tuple[dict[str, str], list[str]]:
    """The variables to set and the directories to put first on PATH."""
    config = settings(root, environ)
    sdk = sdk_of(root, environ, prefix)
    values = {"FLUTTER_ROOT": str(sdk)}
    for key in ("PUB_CACHE", "CANOA_DEPS", "CARGO_HOME", "RUSTUP_HOME"):
        if config.get(key):
            values[key] = config[key]
    values["CANOA_WEB_DIST"] = str(root / "crates" / "canoa-host" / "tests" / "site")
    if not config.get("CANOA_DEPS"):
        values["CANOA_OFFICE_JS"] = "cdn"
    paths = [str(sdk / "bin"), str(dart_program(sdk).parent)]
    if config.get("CARGO_HOME"):
        paths.append(str(Path(config["CARGO_HOME"]) / "bin"))
    return values, paths


def env_lines(values: Mapping[str, str], paths: Sequence[str], shell: str) -> list[str]:
    if shell == "bash":
        quote = lambda text: shlex.quote(posix(text) if re.match(r"^[A-Za-z]:[\\/]", text) else text)
        lines = [f"export {key}={quote(value)}" for key, value in values.items()]
        lines.append("export PATH=" + shlex.quote(":".join(posix(p) for p in paths)) + ':"$PATH"')
    elif shell == "pwsh":
        quote = lambda text: "'" + text.replace("'", "''") + "'"
        lines = [f"$env:{key} = {quote(value)}" for key, value in values.items()]
        lines.append(f"$env:PATH = {quote(os.pathsep.join(paths))} + [IO.Path]::PathSeparator + $env:PATH")
    elif shell == "cmd":
        lines = [f'set "{key}={value}"' for key, value in values.items()]
        lines.append(f'set "PATH={";".join(paths)};%PATH%"')
    else:
        raise Problem(f"unknown shell {shell}; one of bash, pwsh, cmd")
    return lines


# agent tooling

def mcp_config(sdk: Path, existing: Mapping | None = None) -> dict:
    """.mcp.json: the Dart and Flutter MCP server on the pinned SDK, without the tools that reach pub.dev or make projects."""
    config = dict(existing or {})
    servers = dict(config.get("mcpServers", {}))
    servers["dart"] = {
        "command": str(dart_program(sdk)),
        "args": ["mcp-server", "--flutter-sdk", str(sdk), "--disable", MCP_DISABLED],
    }
    config["mcpServers"] = servers
    return config


def codex_block(sdk: Path) -> str:
    """The same server as a Codex config table; literal strings, so a Windows path needs no escapes."""
    args = ", ".join(f"'{a}'" for a in ("mcp-server", "--flutter-sdk", str(sdk), "--disable", MCP_DISABLED))
    return f"{CODEX_BEGIN}\n[mcp_servers.dart]\ncommand = '{dart_program(sdk)}'\nargs = [{args}]\n{CODEX_END}\n"


def merge_codex(text: str, block: str) -> str:
    """Replace the block between the markers, or append it."""
    pattern = re.compile(re.escape(CODEX_BEGIN) + r".*?" + re.escape(CODEX_END) + r"\n?", re.S)
    if pattern.search(text):
        return pattern.sub(lambda _: block, text)
    return text + ("\n" if text and not text.endswith("\n") else "") + block


def parse_skills(text: str) -> list[tuple[str, str, str]]:
    """Lines of `repository branch skill` (a GitHub owner/name, a branch, the folder under skills/); # starts a comment."""
    rows = []
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) != 3 or not re.fullmatch(r"[\w.-]+/[\w.-]+", parts[0]) or not re.fullmatch(r"[\w./-]+", parts[2]):
            raise Problem(f"{SKILLS_FILE}:{number}: want `owner/repository branch skill`, got `{raw.strip()}`")
        rows.append((parts[0], parts[1], parts[2]))
    return rows


def fetch_skills(rows: Sequence[tuple[str, str, str]], destination: Path, run: Run) -> list[str]:
    """Copy each skill's folder from the head of its branch into destination/<skill>; one sparse clone per repository."""
    destination.mkdir(parents=True, exist_ok=True)
    done = []
    for repo, branch in dict.fromkeys((r, b) for r, b, _ in rows):
        names = [name for r, b, name in rows if (r, b) == (repo, branch)]
        with tempfile.TemporaryDirectory(prefix="dev-skills-") as scratch:
            clone = Path(scratch) / "clone"
            steps = [
                ["git", "clone", "-q", "--depth", "1", "--filter=blob:none", "--sparse", "--branch", branch,
                 f"https://github.com/{repo}.git", str(clone)],
                ["git", "-C", str(clone), "sparse-checkout", "set", *(f"skills/{n}" for n in names)],
            ]
            for step in steps:
                if run(step, Path(scratch), os.environ) != 0:
                    raise Problem(f"{' '.join(step[:3])} failed for {repo}@{branch}")
            for name in names:
                source = clone / "skills" / name
                if not (source / "SKILL.md").is_file():
                    raise Problem(f"{repo}@{branch} has no skills/{name}/SKILL.md")
                target = destination / name
                if target.exists():
                    shutil.rmtree(target)
                shutil.copytree(source, target)
                done.append(name)
    return done


def write_agents(root: Path, sdk: Path, run: Run, fetch: bool = True) -> list[str]:
    """.mcp.json, .codex/config.toml and the skills; returns what was done, one line each."""
    notes = []
    mcp = root / ".mcp.json"
    existing = json.loads(mcp.read_text(encoding="utf-8")) if mcp.is_file() else None
    mcp.write_text(json.dumps(mcp_config(sdk, existing), indent=2) + "\n", encoding="utf-8", newline="\n")
    notes.append(f"wrote {mcp.name}: the dart server on {sdk}")
    codex = root / ".codex" / "config.toml"
    codex.parent.mkdir(exist_ok=True)
    codex.write_text(merge_codex(codex.read_text(encoding="utf-8") if codex.is_file() else "", codex_block(sdk)),
                     encoding="utf-8", newline="\n")
    notes.append("wrote .codex/config.toml (Codex reads it once you trust this project)")
    rows = parse_skills((root / SKILLS_FILE).read_text(encoding="utf-8"))
    if fetch:
        names = fetch_skills(rows, root / ".claude" / "skills", run)
        notes.append(f"copied {len(names)} skills into .claude/skills: {', '.join(names)}")
        if (root / ".agents").exists():
            notes.append(".agents/ exists here, so the skills are not copied into .agents/skills, which Codex reads: "
                         "copy them there yourself if you want them in Codex")
        else:
            fetch_skills(rows, root / ".agents" / "skills", run)
            notes.append("copied the same skills into .agents/skills for Codex")
    return notes


# running things

def subprocess_run(command: Sequence[str], cwd: Path, env: Mapping[str, str]) -> int:
    return subprocess.run(list(command), cwd=cwd, env=dict(env)).returncode


def build_py(root: Path, *args: str) -> list[str]:
    return [sys.executable, "-I", str(root / "tools" / "build.py"), *args]


def tool_env(root: Path, environ: Mapping[str, str], prefix: Path | None = None) -> dict[str, str]:
    """The process environment with the pinned SDK first on PATH, as every tool this script starts sees it."""
    values, paths = env_values(root, environ, prefix)
    env = dict(environ)
    env.update({k: v for k, v in values.items() if k != "CANOA_OFFICE_JS"})
    env["PATH"] = os.pathsep.join([*paths, environ.get("PATH", "")])
    return env


def tools_ready(sdk: Path) -> bool:
    cache = sdk / "bin" / "cache"
    return (cache / "flutter_tools.snapshot").is_file() and (cache / "flutter_tools.stamp").is_file()


# commands

def setup(root: Path, args: argparse.Namespace, run: Run = subprocess_run, environ: Mapping[str, str] = os.environ) -> int:
    prefix = Path(args.prefix) if args.prefix else default_prefix(environ)
    install = build_py(root, "sdk", "install", "--groups", GROUPS, "--prefix", str(prefix))
    if args.platforms:
        install += ["--precache", args.platforms]
    if args.deps:
        install += ["--deps", str(args.deps)]
    print(f"== sdk install into {prefix}", flush=True)
    if run(install, root, environ) != 0:
        raise Problem("build.py sdk install failed; read what it printed above")
    sdk = pinned_flutter(root, prefix)
    if not flutter_script(sdk).is_file():
        raise Problem(f"{sdk} has no Flutter after the install")
    wanted = {"FLUTTER_ROOT": str(sdk)}
    for key, folder in (("CARGO_HOME", prefix / "rust" / "cargo"), ("RUSTUP_HOME", prefix / "rust" / "rustup")):
        if folder.is_dir():
            wanted[key] = str(folder)
    if args.pub_cache or environ.get("PUB_CACHE"):
        wanted["PUB_CACHE"] = str(args.pub_cache or environ["PUB_CACHE"])
    if args.deps or environ.get("CANOA_DEPS"):
        wanted["CANOA_DEPS"] = str(args.deps or environ["CANOA_DEPS"])
    done = merge_env(root / ".env", wanted, environ)
    env = tool_env(root, environ, prefix)
    flutter = str(flutter_script(sdk))
    if not tools_ready(sdk):
        print("== building the Flutter tool (the one step of setup that reaches pub.dev)", flush=True)
        if run([flutter, "--version"], root, env) != 0 or not tools_ready(sdk):
            raise Problem(f"{sdk}/bin/cache has no flutter_tools.snapshot and flutter_tools.stamp after `flutter --version`")
        done.append("built bin/cache/flutter_tools.snapshot and its stamp")
    app = root / "app"
    lock = app / "pubspec.lock"
    before = lock.read_bytes()
    print("== flutter pub get --enforce-lockfile", flush=True)
    code = run([flutter, "pub", "get", "--enforce-lockfile"], app, env)
    if lock.read_bytes() != before:
        lock.write_bytes(before)
        raise Problem("flutter pub get changed app/pubspec.lock; it is restored. The SDK is not the one the lock was made with: "
                      "run `python tools/dev.py check`")
    if code != 0:
        raise Problem("flutter pub get --enforce-lockfile failed; read what it printed above")
    done.append("app's packages resolved against app/pubspec.lock")
    if args.agents:
        done += write_agents(root, sdk, run)
    print("\n".join(["", "Done:", *(f"  {line}" for line in done)]))
    print("\nNext: put the SDK on PATH in this shell, then build:")
    print('  eval "$(python tools/dev.py env)"                          # bash')
    print("  python tools/dev.py env --shell pwsh | Invoke-Expression   # PowerShell")
    print("  python app/tool/build.py pwa                               # or chromium, office, desktop")
    return 0


def check(root: Path, args: argparse.Namespace, run: Run = subprocess_run, environ: Mapping[str, str] = os.environ) -> int:
    prefix = Path(args.prefix) if args.prefix else default_prefix(environ)
    problems = []
    if run(build_py(root, "sdk", "install", "--check", "--groups", GROUPS, "--prefix", str(prefix)), root, environ) != 0:
        problems.append(f"sdk install --check: something is missing under {prefix} (above)")
    config = settings(root, environ)
    sdk = sdk_of(root, environ, prefix)
    expected = pinned_flutter(root, prefix)
    if config.get("FLUTTER_ROOT") and Path(config["FLUTTER_ROOT"]) != expected:
        print(f"note: FLUTTER_ROOT is {config['FLUTTER_ROOT']}, not the install under the prefix ({expected}); "
              "precache-check says whether it is the pinned one")
    if not sdk.is_dir():
        problems.append(f"the Flutter SDK {sdk} is not there")
    else:
        if run(build_py(root, "flutter", "precache-check", "--flutter-root", str(sdk)), root, environ) != 0:
            problems.append(f"{sdk} is not the SDK tools/sdk.lock names (above)")
        if not tools_ready(sdk):
            problems.append(f"{sdk}/bin/cache has no flutter_tools.snapshot and stamp: run `python tools/dev.py setup`")
    for key, value in config.items():
        if key != "FLUTTER" and not Path(value).exists():
            problems.append(f".env: {key} names {value}, which does not exist")
    mcp = root / ".mcp.json"
    if mcp.is_file():
        command = json.loads(mcp.read_text(encoding="utf-8")).get("mcpServers", {}).get("dart", {}).get("command")
        if not command or not Path(command).is_file():
            problems.append(f".mcp.json: the dart server's program {command} does not exist; run `python tools/dev.py agents`")
    skills = root / ".claude" / "skills"
    if skills.is_dir():
        for _, _, name in parse_skills((root / SKILLS_FILE).read_text(encoding="utf-8")):
            if not (skills / name / "SKILL.md").is_file():
                problems.append(f".claude/skills/{name} is missing; run `python tools/dev.py agents`")
    for problem in problems:
        print(f"dev.py: {problem}", file=sys.stderr)
    print("ok" if not problems else f"{len(problems)} problem(s)")
    return 1 if problems else 0


def agents(root: Path, args: argparse.Namespace, run: Run = subprocess_run, environ: Mapping[str, str] = os.environ) -> int:
    sdk = sdk_of(root, environ, Path(args.prefix) if args.prefix else None)
    if not dart_program(sdk).is_file():
        raise Problem(f"{dart_program(sdk)} does not exist: run `python tools/dev.py setup` first, or set FLUTTER_ROOT")
    print("\n".join(write_agents(root, sdk, run)))
    return 0


def env_command(root: Path, args: argparse.Namespace, environ: Mapping[str, str] = os.environ) -> int:
    values, paths = env_values(root, environ, Path(args.prefix) if args.prefix else None)
    print("\n".join(env_lines(values, paths, args.shell or ("pwsh" if WINDOWS else "bash"))))
    return 0


def parser() -> argparse.ArgumentParser:
    top = argparse.ArgumentParser(prog="dev.py", description=__doc__.split("\n\n")[0])
    sub = top.add_subparsers(dest="command", required=True)
    prefix = "where the SDKs are (default: $BUILD_SDK_HOME, else ~/.canoa-sdk)"
    one = sub.add_parser("setup", help="install the SDK, write .env, build the Flutter tool, resolve the app's packages")
    one.add_argument("--prefix", help=prefix)
    one.add_argument("--platforms", help="Flutter platforms to precache, comma separated (web, windows, linux, macos)")
    one.add_argument("--deps", help="a checkout of the deps branch; recorded as CANOA_DEPS")
    one.add_argument("--pub-cache", help="recorded as PUB_CACHE (default: $PUB_CACHE if set)")
    one.add_argument("--agents", action="store_true", help="also write the agent tooling (see `agents`)")
    two = sub.add_parser("env", help="print the lines that put the SDK on PATH and set the build's variables")
    two.add_argument("--shell", choices=("bash", "pwsh", "cmd"), help="default: pwsh on Windows, else bash")
    two.add_argument("--prefix", help=prefix)
    three = sub.add_parser("check", help="report what is missing or stale; exit 1 if anything is")
    three.add_argument("--prefix", help=prefix)
    four = sub.add_parser("agents", help="write .mcp.json, .codex/config.toml and .claude/skills (all ignored by git)")
    four.add_argument("--prefix", help=prefix)
    return top


def main(argv: Sequence[str] | None = None, root: Path = ROOT) -> int:
    args = parser().parse_args(argv)
    try:
        if args.command == "env":
            return env_command(root, args)
        return {"setup": setup, "check": check, "agents": agents}[args.command](root, args)
    except Problem as problem:
        print(f"dev.py: {problem}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
