#!/usr/bin/env python3
"""Build the shared Canoa Flutter UI for the PWA or an extension."""
from __future__ import annotations

import argparse
from pathlib import Path
import shutil
import subprocess

APP = Path(__file__).resolve().parents[1]
ROOT = APP.parent


def flutter_args(kind: str, wasm: bool) -> list[str]:
    flutter = shutil.which("flutter")
    if flutter is None:
        raise RuntimeError("Flutter is not on PATH")
    args = [flutter, "build", "web", "--release", "--no-web-resources-cdn"]
    if wasm:
        args.append("--wasm")
    if kind == "chromium":
        args += ["--base-href", "/ui/", "--csp"]
    return args


def package_ui(source: Path, target: Path) -> None:
    if not (source / "index.html").is_file() or not (
        source / "flutter_bootstrap.js"
    ).is_file():
        raise FileNotFoundError("Flutter release bundle is incomplete")
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(source, target, ignore=shutil.ignore_patterns("*.map"))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=("pwa", "chromium"))
    parser.add_argument(
        "--wasm", action="store_true",
        help="Build Flutter web with --wasm (extension compatibility must be checked)",
    )
    opts = parser.parse_args()
    subprocess.run(["flutter", "pub", "get"], check=True, cwd=APP)
    subprocess.run(flutter_args(opts.kind, opts.wasm), check=True, cwd=APP)
    if opts.kind == "chromium":
        package_ui(
            APP / "build" / "web",
            ROOT / "bindings" / "browser" / "chromium" / "ui",
        )
        print("Packaged Flutter UI into bindings/browser/chromium/ui")
    else:
        print("Built standalone PWA in app/build/web")
        if opts.wasm:
            print("Configure hosting for Flutter Wasm, including required isolation headers.")


if __name__ == "__main__":
    main()
