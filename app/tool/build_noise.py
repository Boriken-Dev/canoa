#!/usr/bin/env python3
"""Build the web app's Noise method (app/web_noise, coqui-peer-noise) to WebAssembly and its loader.

Writes canoa_noise.js and canoa_noise_bg.wasm to app/web/noise/ (generated, never committed), which `flutter build web`
copies into the page. Needs cargo with the wasm32-unknown-unknown target and a wasm-bindgen command line of exactly the
version app/web_noise/Cargo.toml pins.
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
CRATE = APP / "web_noise"
NAME = "canoa_noise_web"


def pinned_bindgen() -> str:
    match = re.search(r'^wasm-bindgen = "=([^"]+)"', (CRATE / "Cargo.toml").read_text(encoding="utf-8"), re.M)
    if match is None:
        raise RuntimeError("web_noise/Cargo.toml does not pin wasm-bindgen exactly")
    return match.group(1)


def require_tools(bindgen: str) -> str:
    for tool in ("cargo", "wasm-bindgen"):
        if shutil.which(tool) is None:
            raise RuntimeError(f"{tool} is not on PATH (cargo install wasm-bindgen-cli --version {bindgen} --locked)")
    found = subprocess.run(["wasm-bindgen", "--version"], check=True, capture_output=True, text=True).stdout.split()[-1]
    if found != bindgen:
        raise RuntimeError(f"wasm-bindgen {found} is installed and the crate pins {bindgen}: cargo install wasm-bindgen-cli --version {bindgen} --locked --force")
    return found


def build(out: Path, test_peer: bool) -> Path:
    require_tools(pinned_bindgen())
    target = Path(os.environ.get("CARGO_TARGET_DIR", CRATE / "target"))
    command = ["cargo", "build", "--release", "--locked", "--target", "wasm32-unknown-unknown", "--manifest-path", str(CRATE / "Cargo.toml")]
    if test_peer:
        command += ["--features", "test-peer"]
    subprocess.run(command, check=True)
    wasm = target / "wasm32-unknown-unknown" / "release" / f"{NAME}.wasm"
    if out.exists():
        shutil.rmtree(out)
    subprocess.run(["wasm-bindgen", "--target", "web", "--no-typescript", "--out-dir", str(out), str(wasm)], check=True)
    # The loader is named for the app, whatever the crate is called.
    for suffix in (".js", "_bg.wasm"):
        (out / f"{NAME}{suffix}").rename(out / f"canoa_noise{suffix}")
    glue = out / "canoa_noise.js"
    glue.write_text(glue.read_text(encoding="utf-8").replace(f"{NAME}_bg.wasm", "canoa_noise_bg.wasm"), encoding="utf-8", newline="\n")
    return out / "canoa_noise_bg.wasm"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=APP / "web" / "noise", help="where the loader and the wasm go")
    parser.add_argument("--test-peer", action="store_true", help="also build the responder the browser tests use")
    opts = parser.parse_args()
    try:
        wasm = build(opts.out, opts.test_peer)
    except (RuntimeError, subprocess.CalledProcessError) as error:
        sys.exit(f"build_noise.py: {error}")
    print(f"{wasm} {wasm.stat().st_size} bytes")


if __name__ == "__main__":
    main()
