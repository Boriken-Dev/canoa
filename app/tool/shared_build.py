"""What Canoa's builds ask of the shared tools/build.py: the deps checkout, the offline environment, the SDK checks.

tools/build.py is a copy of the one in Boriken-Dev/agent-overlays, configured by tools/build.cfg; it knows the deps
branch, the pub cache and cargo home a build reads, and what a Flutter checkout must hold. Each function runs one of its
commands and returns what the build needs, or raises RuntimeError with the tool's own words.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(os.path.abspath(__file__)).parents[2]
TOOL = ROOT / "tools" / "build.py"

# What a build says when the deps branch cannot supply it: the command that fills the mirror, never a fetch.
MIRROR_HINT = "refresh the deps branch (python tools/build.py deps refresh) and fetch it, or pass --online to resolve from the network"


def run_tool(*args: str) -> subprocess.CompletedProcess:
    """`python tools/build.py ARGS`, output captured; the caller reads the return code."""
    return subprocess.run([sys.executable, str(TOOL), "--root", str(ROOT), *args], capture_output=True, text=True,
                          encoding="utf-8", errors="replace")


def tool_text(*args: str) -> str:
    """The standard output of one command of the shared tool, or its message as an error."""
    done = run_tool(*args)
    if done.returncode:
        raise RuntimeError((done.stderr.strip() or done.stdout.strip() or f"tools/build.py {' '.join(args)} failed")
                           .removeprefix("build.py: "))
    return done.stdout


def deps_checkout(explicit: str | os.PathLike | None = None) -> Path | None:
    """The deps checkout: `explicit`, else CANOA_DEPS, else the worktree beside the repository (added when the branch is local).

    None when there is none or it holds no manifest.json.
    """
    done = run_tool("deps", "path", *(["--deps", str(explicit)] if explicit else []))
    lines = done.stdout.strip().splitlines()
    if done.returncode or not lines:
        return None
    path = Path(lines[-1])
    return path if (path / "manifest.json").is_file() else None


def offline_environment(deps: Path) -> dict[str, str]:
    """`offline env --json` for `deps`: the variables an offline build runs under, plus `cache` and `cargo_home`."""
    return json.loads(tool_text("offline", "env", "--deps", str(deps), "--json"))


def vendor_config(deps: Path, folder: str) -> Path:
    """The cargo config that builds from deps/FOLDER alone; an error names what the folder lacks."""
    return Path(json.loads(tool_text("deps", "use", "--deps", str(deps), "--vendor", folder, "--json"))["config"])


def offline_command(deps: Path, vendor: str, command: list[str]) -> list[str]:
    """`command` wrapped to run under the offline plan of one vendor folder (its cargo home, no network)."""
    return [sys.executable, str(TOOL), "--root", str(ROOT), "offline", "run", "--deps", str(deps), "--vendor", vendor, "--", *command]


def precache_check(sdk: Path, platform: str) -> None:
    """Fails unless the Flutter checkout `sdk` is the locked one, precached for `platform` (web, windows, linux, macos)."""
    tool_text("flutter", "precache-check", "--flutter-root", str(sdk), "--platform", platform)


def fonts_check(sdk: Path) -> None:
    """Fails unless the fonts the SDK's engine asks for are exactly the ones the CDN index pins."""
    tool_text("flutter", "fonts-check", "--flutter-root", str(sdk))


def fonts_install(dist: Path, deps: Path) -> str:
    """Copies the fallback fonts from the deps checkout into a web build, verifying each; returns the tool's summary line."""
    return tool_text("flutter", "fonts-install", str(dist), "--deps", str(deps)).strip()
