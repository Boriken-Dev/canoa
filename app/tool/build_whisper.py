#!/usr/bin/env python3
"""Build the web app's Whisper (app/web_whisper, canoa-whisper-candle's engine) to WebAssembly and its loader.

Writes canoa_whisper.js and canoa_whisper_bg.wasm to app/web/whisper/ (generated, never committed), which `flutter build web`
copies into the page beside whisper_worker.js, the web worker that loads them. Needs cargo with the wasm32-unknown-unknown
target and a wasm-bindgen command line of exactly the version app/web_whisper/Cargo.toml pins.

Nothing comes from crates.io or GitHub: the crates are DEPS/crates-web_whisper, DEPS being --deps, CANOA_DEPS or the deps
branch's worktree beside the repository, and the command line is installed as build_noise.py's --install-bindgen does. Without
a deps checkout the build stops; only --online, for refreshing the mirror, reads crates.io and GitHub.
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

APP = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_noise  # noqa: E402
import shared_build  # noqa: E402

CRATE = APP / "web_whisper"
BINDGEN = "wasm-bindgen-cli"
NAME = "canoa_whisper_web"


def pinned_bindgen() -> str:
    match = re.search(r'^wasm-bindgen = "=([^"]+)"', (CRATE / "Cargo.toml").read_text(encoding="utf-8"), re.M)
    if match is None:
        raise RuntimeError("web_whisper/Cargo.toml does not pin wasm-bindgen exactly")
    return match.group(1)


def install_bindgen(bindgen: str, deps: Path) -> None:
    config = shared_build.vendor_config(deps, "crates-wasm-bindgen-cli")
    subprocess.run(["cargo", "--config", str(config), "install", "--offline", "--locked", BINDGEN, "--version", bindgen], check=True)


def require_tools(bindgen: str, deps: Path | None) -> str:
    if shutil.which("cargo") is None:
        raise RuntimeError("cargo is not on PATH")
    build_noise.require_wasm_target()
    how = (f"python app/tool/build_noise.py --deps {deps} --install-bindgen" if deps
           else f"cargo install {BINDGEN} --version {bindgen} --locked")
    if shutil.which("wasm-bindgen") is None:
        raise RuntimeError(f"wasm-bindgen is not on PATH ({how})")
    found = subprocess.run(["wasm-bindgen", "--version"], check=True, capture_output=True, text=True).stdout.split()[-1]
    if found != bindgen:
        raise RuntimeError(f"wasm-bindgen {found} is installed and the crate pins {bindgen}: {how} --force")
    return found


def build(out: Path, deps: Path | None = None) -> Path:
    require_tools(pinned_bindgen(), deps)
    target = Path(os.environ.get("CARGO_TARGET_DIR", CRATE / "target"))
    command = ["cargo", "build", "--release", "--locked", "--target", "wasm32-unknown-unknown", "--manifest-path", str(CRATE / "Cargo.toml")]
    if deps is not None:
        command = shared_build.offline_command(deps, "crates-web_whisper", [*command, "--offline"])
    subprocess.run(command, check=True)
    wasm = target / "wasm32-unknown-unknown" / "release" / f"{NAME}.wasm"
    if out.exists():
        shutil.rmtree(out)
    subprocess.run(["wasm-bindgen", "--target", "web", "--no-typescript", "--out-dir", str(out), str(wasm)], check=True)
    # The loader is named for the app, whatever the crate is called.
    for suffix in (".js", "_bg.wasm"):
        (out / f"{NAME}{suffix}").rename(out / f"canoa_whisper{suffix}")
    glue = out / "canoa_whisper.js"
    glue.write_text(glue.read_text(encoding="utf-8").replace(f"{NAME}_bg.wasm", "canoa_whisper_bg.wasm"), encoding="utf-8", newline="\n")
    return out / "canoa_whisper_bg.wasm"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, help="where the loader and the wasm go (default web/whisper)")
    parser.add_argument("--deps", type=Path,
                        help="a checkout of the deps branch: build from its crates, with the network off (default: CANOA_DEPS, "
                             "else the deps branch's worktree beside the repository)")
    parser.add_argument("--online", action="store_true",
                        help="build from crates.io and GitHub instead of the deps branch, to refresh the mirror")
    opts = parser.parse_args()
    if opts.online and opts.deps:
        parser.error("--online reads no deps branch: not with --deps")
    if not opts.online and opts.deps is None:
        # CANOA_DEPS is a default, read only when the build reads the deps branch: it never refuses --online.
        env = os.environ.get("CANOA_DEPS")
        opts.deps = Path(env) if env else shared_build.deps_checkout()
        if opts.deps is None:
            sys.exit("build_whisper.py: no deps checkout (set CANOA_DEPS or --deps, or fetch the deps branch; "
                     "--online builds from crates.io and GitHub to refresh the mirror)")
    try:
        wasm = build(opts.out or (APP / "web" / "whisper"), opts.deps)
    except (RuntimeError, subprocess.CalledProcessError) as error:
        sys.exit(f"build_whisper.py: {error}")
    print(f"{wasm} {wasm.stat().st_size} bytes")


if __name__ == "__main__":
    main()
