#!/usr/bin/env python3
"""The Flutter SDK this repository builds with, named once in tools/flutter.pin.

    python tools/flutter_pin.py get KEY             print one value (FLUTTER_COMMIT, FLUTTER_UPSTREAM, FLUTTER_VERSION, FLUTTER_ENGINE, FLUTTER_IMAGE_TAG)
    python tools/flutter_pin.py check               fail when the Containerfile, the workflows or the docs name another SDK
    python tools/flutter_pin.py sdk DIR             fail when the SDK in DIR is not the pin
    python tools/flutter_pin.py install DIR [--platforms web,linux,windows] [--artifacts FILE]
                                                    clone the mirror at the pin into DIR and precache from the engine's release

An SDK matches when it is at the mirror's commit or at the upstream commit it mirrors, reports the pinned version and holds
the pinned engine. `install` needs git and network to github.com only: the engine, the Dart SDK and the web artifacts are the
assets of the mirror's release `engine-<hash>`, put back in the layout Flutter's storage has by flutter_artifacts.py of
the mirror's `images` branch (--artifacts names a copy of it) and served on loopback while `flutter precache` runs.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PIN = ROOT / "tools" / "flutter.pin"
MIRROR = "https://github.com/Boriken-Dev/flutter.git"
#: The branch of the mirror that holds the image tooling, flutter_artifacts.py among it.
IMAGES_BRANCH = "images"
KEYS = ("FLUTTER_COMMIT", "FLUTTER_UPSTREAM", "FLUTTER_VERSION", "FLUTTER_ENGINE")


def load(path: Path = PIN) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        key, _, value = line.partition("=")
        values[key.strip()] = value.strip()
    missing = [k for k in KEYS if not values.get(k)]
    if missing:
        raise ValueError(f"{path} lacks {', '.join(missing)}")
    for key in ("FLUTTER_COMMIT", "FLUTTER_UPSTREAM", "FLUTTER_ENGINE"):
        if not re.fullmatch(r"[0-9a-f]{40}", values[key]):
            raise ValueError(f"{path}: {key} is not a 40-digit commit hash")
    values["FLUTTER_IMAGE_TAG"] = values["FLUTTER_COMMIT"][:10]
    return values


def sdk_problems(sdk: Path, pin: dict[str, str]) -> list[str]:
    """What differs between the SDK in `sdk` and the pin; empty when it is the pin."""
    if not (sdk / "bin" / "internal").is_dir():
        return [f"{sdk} is not a Flutter SDK"]
    problems: list[str] = []
    head = subprocess.run(["git", "-C", str(sdk), "rev-parse", "HEAD"], capture_output=True, text=True)
    commit = head.stdout.strip()
    if head.returncode or commit not in (pin["FLUTTER_COMMIT"], pin["FLUTTER_UPSTREAM"]):
        problems.append(
            f"{sdk} is at {commit or 'no commit'}, not {pin['FLUTTER_COMMIT']} (the mirror) or {pin['FLUTTER_UPSTREAM']} (upstream)")
    cache = sdk / "bin" / "cache"
    stamp = cache / "engine.stamp"
    if not stamp.is_file():
        problems.append(f"{stamp} is missing: run flutter precache")
    elif stamp.read_text().strip() != pin["FLUTTER_ENGINE"]:
        problems.append(f"engine {stamp.read_text().strip()}, not {pin['FLUTTER_ENGINE']}")
    version = cache / "flutter.version.json"
    if not version.is_file():
        problems.append(f"{version} is missing: run flutter --version")
    else:
        found = json.loads(version.read_text(encoding="utf-8")).get("flutterVersion")
        if found != pin["FLUTTER_VERSION"]:
            problems.append(f"version {found}, not {pin['FLUTTER_VERSION']}")
    return problems


def consumer_problems(root: Path, pin: dict[str, str]) -> list[str]:
    """Where the Containerfile, the workflows and the docs disagree with the pin."""
    problems: list[str] = []
    containerfile = (root / "packaging/container/Containerfile").read_text(encoding="utf-8")
    tags = re.findall(r"^ARG FLUTTER_IMAGE_TAG=(\S*)", containerfile, re.M)
    if tags != [pin["FLUTTER_IMAGE_TAG"]]:
        problems.append(f"Containerfile: FLUTTER_IMAGE_TAG is {tags}, the pin's is {pin['FLUTTER_IMAGE_TAG']}")
    if not re.search(r"^FROM ghcr\.io/boriken-dev/flutter-web:\$\{FLUTTER_IMAGE_TAG\} AS flutter", containerfile, re.M):
        problems.append("Containerfile: the flutter stage is not FROM the flutter-web image of the pin")
    for name in ("FLUTTER_VERSION_TAG", "FLUTTER_STORAGE_BASE_URL"):
        if re.search(rf"^ARG {name}\b", containerfile, re.M):
            problems.append(f"Containerfile: still has {name}")
    action = (root / ".github/actions/flutter/action.yml").read_text(encoding="utf-8")
    if "tools/flutter_pin.py" not in action:
        problems.append("the flutter action does not read the pin")
    for workflow in sorted((root / ".github/workflows").glob("*.yml")):
        text = workflow.read_text(encoding="utf-8")
        if "flutter-action" in text or "flutter-version:" in text:
            problems.append(f"{workflow.name}: installs Flutter on its own, not with ./.github/actions/flutter")
        for found in re.findall(r"\b[0-9a-f]{40}\b", text):
            if found in (pin["FLUTTER_UPSTREAM"], pin["FLUTTER_COMMIT"]):
                problems.append(f"{workflow.name}: names the Flutter commit {found} itself")
    docs = (root / "docs/getting-started.md").read_text(encoding="utf-8")
    for needed in (pin["FLUTTER_VERSION"], pin["FLUTTER_UPSTREAM"][:10], pin["FLUTTER_COMMIT"][:10]):
        if needed not in docs:
            problems.append(f"docs/getting-started.md does not name {needed}")
    return problems


def install(directory: Path, platforms: list[str], artifacts: Path | None) -> None:
    pin = load()
    if directory.exists() and any(directory.iterdir()):
        raise SystemExit(f"{directory} is not empty")

    def git(*args: str, cwd: Path | None = None) -> None:
        subprocess.run(["git", "-c", "core.autocrlf=false", *args], check=True, cwd=cwd)

    scratch = Path(tempfile.mkdtemp(prefix="flutter_pin_"))
    try:
        if artifacts is None:
            git("clone", "--quiet", "--depth", "1", "--branch", IMAGES_BRANCH, MIRROR, str(scratch / "images"))
            artifacts = scratch / "images" / "tools" / "flutter_artifacts.py"
        git("clone", "--quiet", "--filter=blob:none", "--no-tags", "--branch", "main", "--single-branch", MIRROR, str(directory))
        git("checkout", "--quiet", "-B", "main", pin["FLUTTER_COMMIT"], cwd=directory)
        git("fetch", "--quiet", "--no-tags", "origin", "+refs/tags/*:refs/tags/*", cwd=directory)
        log = scratch / "served.log"
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]
        server = subprocess.Popen(
            [sys.executable, str(artifacts), "serve", pin["FLUTTER_ENGINE"], "--port", str(port), "--dir", str(scratch / "tree"),
             "--log", str(log)],
            stdout=subprocess.PIPE, text=True)
        try:
            assert server.stdout is not None
            for line in server.stdout:  # printed once the tree is complete
                if line.startswith("FLUTTER_STORAGE_BASE_URL="):
                    break
            else:
                raise SystemExit("the artifact server did not start")
            env = dict(os.environ, FLUTTER_STORAGE_BASE_URL=f"http://127.0.0.1:{port}", FLUTTER_GIT_URL=MIRROR,
                       FLUTTER_ROOT=str(directory), CI="true", TAR_OPTIONS="--no-same-owner")
            flutter = str(directory / "bin" / ("flutter.bat" if os.name == "nt" else "flutter"))
            subprocess.run([flutter, "config", "--no-analytics", "--no-cli-animations"], check=True, env=env, cwd=directory)
            subprocess.run([flutter, "precache", *(f"--{p}" for p in platforms)], check=True, env=env, cwd=directory)
            subprocess.run([flutter, "--version"], check=True, env=env, cwd=directory)
        finally:
            server.terminate()
            server.wait(timeout=30)
            time.sleep(0.2)
        misses = [line for line in log.read_text().splitlines() if line.startswith("MISS")] if log.exists() else []
        if misses:
            raise SystemExit("requests the release did not hold:\n" + "\n".join(misses))
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    problems = sdk_problems(directory, pin)
    if problems:
        raise SystemExit("\n".join(problems))
    print(f"flutter_pin: the SDK in {directory} is the pin")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("get").add_argument("key")
    sub.add_parser("check")
    sub.add_parser("sdk").add_argument("dir")
    p = sub.add_parser("install")
    p.add_argument("dir")
    p.add_argument("--platforms", default="web")
    p.add_argument("--artifacts")
    a = parser.parse_args()
    pin = load()
    if a.command == "get":
        print(pin[a.key])
        return
    if a.command == "install":
        install(Path(a.dir), [x for x in a.platforms.split(",") if x], Path(a.artifacts) if a.artifacts else None)
        return
    problems = consumer_problems(ROOT, pin) if a.command == "check" else sdk_problems(Path(a.dir), pin)
    for problem in problems:
        print(f"flutter_pin: {problem}", file=sys.stderr)
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
