#!/usr/bin/env python3
"""Build the web app's Noise method (app/web_noise, coqui-peer-noise) to WebAssembly and its loader.

Writes canoa_noise.js and canoa_noise_bg.wasm to app/web/noise/ (generated, never committed), which `flutter build web`
copies into the page. With --test-peer it builds a variant with a responder into app/test/noise/, for the browser tests.
Needs cargo with the wasm32-unknown-unknown target and a wasm-bindgen command line of exactly the version
app/web_noise/Cargo.toml pins.

Nothing comes from crates.io or GitHub: the crates are DEPS/crates-web_noise, DEPS being --deps, CANOA_DEPS or the deps
branch's worktree beside the repository, and --install-bindgen builds the command line from DEPS/crates-wasm-bindgen-cli with
`cargo install --offline --locked`, into CARGO_HOME's bin (or ~/.cargo/bin). Without a deps checkout the build stops; only
--online, for refreshing the mirror, reads crates.io and GitHub.
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

APP = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(Path(__file__).resolve().parent))
import offline_pub  # noqa: E402

CRATE = APP / "web_noise"
BINDGEN = "wasm-bindgen-cli"
NAME = "canoa_noise_web"


def pinned_bindgen() -> str:
    match = re.search(r'^wasm-bindgen = "=([^"]+)"', (CRATE / "Cargo.toml").read_text(encoding="utf-8"), re.M)
    if match is None:
        raise RuntimeError("web_noise/Cargo.toml does not pin wasm-bindgen exactly")
    return match.group(1)


def deps_config(deps: Path, folder: str) -> Path:
    """The cargo config that builds from deps/<folder> alone, or an error naming what the folder lacks."""
    crates = offline_pub._mirror_crates()
    try:
        vendor = next(v for v in crates.VENDORS if v.folder == folder)
        lock, text = crates.vendor_lock(vendor, deps)
        found = crates.problems(lock, crates.read_dir(deps / folder, deep=False), deep=False, folder=folder)
        if found:
            more = f" (and {len(found) - 1} more)" if len(found) > 1 else ""
            raise crates.Problem(found[0] + more)
        home = Path(tempfile.gettempdir()) / f"canoa-cargo-{folder}"
        return crates.use(deps, lock, text, home, folder=folder) / "config.toml"
    except crates.Problem as problem:
        raise RuntimeError(f"the deps branch's {folder}/ cannot build this: {problem}") from problem


def install_bindgen(bindgen: str, deps: Path) -> None:
    config = deps_config(deps, "crates-wasm-bindgen-cli")
    subprocess.run(["cargo", "--config", str(config), "install", "--offline", "--locked", BINDGEN, "--version", bindgen], check=True)


def require_wasm_target() -> None:
    """The wasm32 standard library is rustup's, installed once with the toolchain; a build never adds it."""
    sysroot = Path(subprocess.run(["rustc", "--print", "sysroot"], check=True, capture_output=True, text=True).stdout.strip())
    if not (sysroot / "lib" / "rustlib" / "wasm32-unknown-unknown").is_dir():
        raise RuntimeError("the wasm32-unknown-unknown target is not installed (rustup target add wasm32-unknown-unknown)")


def require_tools(bindgen: str, deps: Path | None) -> str:
    if shutil.which("cargo") is None:
        raise RuntimeError("cargo is not on PATH")
    require_wasm_target()
    how = (f"python app/tool/build_noise.py --deps {deps} --install-bindgen" if deps
           else f"cargo install {BINDGEN} --version {bindgen} --locked")
    if shutil.which("wasm-bindgen") is None:
        raise RuntimeError(f"wasm-bindgen is not on PATH ({how})")
    found = subprocess.run(["wasm-bindgen", "--version"], check=True, capture_output=True, text=True).stdout.split()[-1]
    if found != bindgen:
        raise RuntimeError(f"wasm-bindgen {found} is installed and the crate pins {bindgen}: {how} --force")
    return found


def build(out: Path, test_peer: bool, deps: Path | None = None) -> Path:
    require_tools(pinned_bindgen(), deps)
    target = Path(os.environ.get("CARGO_TARGET_DIR", CRATE / "target"))
    command = ["cargo", "build", "--release", "--locked", "--target", "wasm32-unknown-unknown", "--manifest-path", str(CRATE / "Cargo.toml")]
    if deps is not None:
        config = deps_config(deps, "crates-web_noise")
        command = [command[0], "--config", str(config), *command[1:], "--offline"]
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
    parser.add_argument("--out", type=Path, help="where the loader and the wasm go (default web/noise, or test/noise with --test-peer)")
    parser.add_argument("--test-peer", action="store_true", help="also build the responder the browser tests use, into test/noise only")
    parser.add_argument("--deps", type=Path, default=os.environ.get("CANOA_DEPS") or None,
                        help="a checkout of the deps branch: build from its crates, with the network off (default: CANOA_DEPS, "
                             "else the deps branch's worktree beside the repository)")
    parser.add_argument("--online", action="store_true",
                        help="build from crates.io and GitHub instead of the deps branch, to refresh the mirror")
    parser.add_argument("--install-bindgen", action="store_true",
                        help="install the wasm-bindgen command line from --deps, offline, and stop")
    opts = parser.parse_args()
    if opts.online and (opts.deps or opts.install_bindgen):
        parser.error("--online reads no deps branch: not with --deps or --install-bindgen")
    if not opts.online and opts.deps is None:
        opts.deps = offline_pub.find_deps(APP.parent, offline_pub.load_settings(APP.parent, os.environ))
        if opts.deps is None:
            sys.exit("build_noise.py: no deps checkout (set CANOA_DEPS or --deps, or fetch the deps branch; "
                     "--online builds from crates.io and GitHub to refresh the mirror)")
    if opts.install_bindgen:
        try:
            install_bindgen(pinned_bindgen(), opts.deps)
        except (RuntimeError, subprocess.CalledProcessError) as error:
            sys.exit(f"build_noise.py: {error}")
        return
    out = opts.out or (APP / ("test" if opts.test_peer else "web") / "noise")
    # The responder is for tests: a page that ships it would carry code no user of the app needs.
    if opts.test_peer and (APP / "web").resolve() in (out.resolve(), *out.resolve().parents):
        parser.error("--test-peer builds into test/noise, never into web/")
    try:
        wasm = build(out, opts.test_peer, opts.deps)
    except (RuntimeError, subprocess.CalledProcessError) as error:
        sys.exit(f"build_noise.py: {error}")
    print(f"{wasm} {wasm.stat().st_size} bytes")


if __name__ == "__main__":
    main()
