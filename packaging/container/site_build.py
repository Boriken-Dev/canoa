#!/usr/bin/env python3
"""Build the web app the host serves, from the deps branch and a pub cache, with the network off.

  site_build.py --seed-git   put the app's git packages in the pub cache (the one step that fetches, from the
                             repositories app/pubspec.lock names)
  site_build.py              resolve the packages offline, build with --csp, and finish the site as app/tool/build.py does

Run by packaging/container/Containerfile; it needs CANOA_DEPS (the deps branch), PUB_CACHE and Flutter on PATH, and
app/web/noise already built (app/tool/build_noise.py).
"""
from __future__ import annotations

import hashlib
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "app" / "tool"))

import build  # noqa: E402
import offline_pub  # noqa: E402


def run(*command: str, cwd: Path | None = None) -> None:
    subprocess.run(command, check=True, cwd=cwd)


def seed_git(cache: Path) -> None:
    """Each git package of the lock as pub caches it: a bare clone named by the URL's hash, and a checkout at the revision."""
    lock = (ROOT / "app" / "pubspec.lock").read_text(encoding="utf-8")
    for package in offline_pub.parse_lock(lock):
        if package.source != "git":
            continue
        bare = cache / "git" / "cache" / f"{package.repo}-{hashlib.sha1(package.url.encode()).hexdigest()}"
        checkout = cache / "git" / f"{package.repo}-{package.resolved_ref}"
        if checkout.is_dir():
            continue
        run("git", "clone", "--quiet", "--mirror", package.url, str(bare))
        run("git", "clone", "--quiet", "--no-checkout", str(bare), str(checkout))
        run("git", "-C", str(checkout), "checkout", "--quiet", package.resolved_ref)
        print(f"{package.name}: {package.url} at {package.resolved_ref[:12]}")


def build_site() -> Path:
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
    cache = Path(os.environ["PUB_CACHE"])
    if "--seed-git" in sys.argv[1:]:
        seed_git(cache)
        return
    try:
        dist = build_site()
    except (RuntimeError, subprocess.CalledProcessError) as error:
        sys.exit(f"site_build.py: {error}")
    print(f"Site in {dist}")


if __name__ == "__main__":
    main()
