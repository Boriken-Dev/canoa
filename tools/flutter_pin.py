#!/usr/bin/env python3
"""The Flutter SDK this repository builds with, named once in tools/flutter.pin.

    python tools/flutter_pin.py get KEY             print one value (FLUTTER_COMMIT, FLUTTER_UPSTREAM, FLUTTER_VERSION, FLUTTER_ENGINE, FLUTTER_IMAGE_TAG)
    python tools/flutter_pin.py check               fail when the Containerfile, the workflows or the docs name another SDK

tools/build.py holds the same facts as rows of tools/sdk.lock and checks them against this file: `sdk check` for the
files, `flutter precache-check --flutter-root DIR --platform P` for an SDK on disk, `sdk install --groups app` to clone the
mirror at the pin and precache it from the engine's release.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PIN = ROOT / "tools" / "flutter.pin"
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


def consumer_problems(root: Path, pin: dict[str, str]) -> list[str]:
    """Where the Containerfile, the workflows and the docs disagree with the pin.

    The Containerfile's image tag is held to tools/sdk.lock by `build.py sdk check`; this reads what that cannot.
    """
    problems: list[str] = []
    containerfile = (root / "packaging/container/Containerfile").read_text(encoding="utf-8")
    if not re.search(r"^FROM ghcr\.io/boriken-dev/flutter-web:\$\{FLUTTER_IMAGE_TAG\} AS flutter", containerfile, re.M):
        problems.append("Containerfile: the flutter stage is not FROM the flutter-web image of the pin")
    for name in ("FLUTTER_VERSION_TAG", "FLUTTER_STORAGE_BASE_URL"):
        if re.search(rf"^ARG {name}\b", containerfile, re.M):
            problems.append(f"Containerfile: still has {name}")
    action = (root / ".github/actions/flutter/action.yml").read_text(encoding="utf-8")
    if "tools/build.py" not in action:
        problems.append("the flutter action does not install the SDK of tools/sdk.lock")
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


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("get").add_argument("key")
    sub.add_parser("check")
    a = parser.parse_args()
    pin = load()
    if a.command == "get":
        print(pin[a.key])
        return
    problems = consumer_problems(ROOT, pin)
    for problem in problems:
        print(f"flutter_pin: {problem}", file=sys.stderr)
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
