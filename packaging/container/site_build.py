#!/usr/bin/env python3
"""Build the web app the host serves, from the deps branch and a pub cache, with the network off.

Resolve the packages offline (the hosted ones and the git ones, from the deps branch), build with --csp, and finish the
site as app/tool/build.py does.

Run by packaging/container/Containerfile; it needs CANOA_DEPS (the deps branch), PUB_CACHE and Flutter on PATH, and
app/web/noise already built (app/tool/build_noise.py). The SDK must be the one tools/flutter.pin names.
"""
from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "app" / "tool"))
sys.path.insert(0, str(ROOT / "tools"))

import build  # noqa: E402
import flutter_pin  # noqa: E402
import offline_pub  # noqa: E402


def run(*command: str, cwd: Path | None = None) -> None:
    subprocess.run(command, check=True, cwd=cwd)


def build_site() -> Path:
    problems = flutter_pin.sdk_problems(Path(os.environ["FLUTTER_ROOT"]), flutter_pin.load())
    if problems:
        raise RuntimeError("the Flutter SDK is not tools/flutter.pin's: " + "; ".join(problems))
    build.configure_environment()
    plan = build.choose_pub("offline")
    assert plan is not None and plan.cache is not None
    os.environ["PUB_CACHE"] = str(plan.cache)
    flutter = build.flutter_program()
    run(flutter, "pub", "get", "--offline", "--enforce-lockfile", cwd=build.APP)
    run(flutter, "build", "web", "--release", "--csp", "--no-web-resources-cdn", "--no-pub", *build.BRAND.defines(), cwd=build.APP)
    dist = build.APP / "build" / "web"
    if not (dist / "index.html").is_file() or not (dist / "main.dart.js").is_file():
        raise RuntimeError("flutter build web left no index.html and main.dart.js")
    build.finish_web(dist)
    return dist


def main() -> None:
    try:
        dist = build_site()
    except (RuntimeError, subprocess.CalledProcessError) as error:
        sys.exit(f"site_build.py: {error}")
    print(f"Site in {dist}")


if __name__ == "__main__":
    main()
