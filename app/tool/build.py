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
    index = target / "index.html"
    html = index.read_text(encoding="utf-8")
    if "<head>" not in html:
        raise ValueError("Flutter index.html has no head element")
    html = html.replace("<head>", '<head>\\n    <script src="../bridge.js"></script>', 1)
    index.write_text(html, encoding="utf-8")


def package_debug_extension(source: Path, destination: Path) -> None:
    """Make an isolated, debug-enabled unpacked extension distribution."""
    if not (source / "manifest.debug.json").is_file():
        raise FileNotFoundError("Elevated Chromium manifest is unavailable")
    if not (source / "ui" / "index.html").is_file():
        raise FileNotFoundError("Build the shared Flutter UI first")
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(
        source, destination,
        ignore=shutil.ignore_patterns(
            "*.test.mjs", "node_modules", "ui/*.map",
        ),
    )
    shutil.copyfile(
        destination / "manifest.debug.json",
        destination / "manifest.json",
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=("pwa", "chromium"))
    parser.add_argument(
        "--wasm", action="store_true",
        help="Build Flutter web with --wasm (extension compatibility must be checked)",
    )
    parser.add_argument("--debug", action="store_true",
        help="Package a separate Chromium debugger extension with CDP permission")
    opts = parser.parse_args()
    if opts.debug and opts.kind != "chromium":
        parser.error("--debug is only valid for chromium")
    subprocess.run(["flutter", "pub", "get"], check=True, cwd=APP)
    subprocess.run(flutter_args(opts.kind, opts.wasm), check=True, cwd=APP)
    if opts.kind == "chromium":
        package_ui(
            APP / "build" / "web",
            ROOT / "bindings" / "browser" / "chromium" / "ui",
        )
        print("Packaged Flutter UI into bindings/browser/chromium/ui")
        if opts.debug:
            package_debug_extension(
                ROOT / "bindings" / "browser" / "chromium",
                ROOT / "build" / "chromium-debug",
            )
            print("Elevated debugger extension in build/chromium-debug")
    else:
        print("Built standalone PWA in app/build/web")
        if opts.wasm:
            print("Configure hosting for Flutter Wasm, including required isolation headers.")


if __name__ == "__main__":
    main()
