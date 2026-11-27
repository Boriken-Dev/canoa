#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# From Boriken-Dev/agent-overlays, overlays/release/repo/tools/public_api.py, at commit 11ed4ddf58fd3cabe76484d953071167c4e3c428.
# Copyright (c) 2026 Jose A.
"""Keeps checked-in public API snapshots equal to what cargo-public-api prints.

This file is copied into a repository as tools/public_api.py and configured by the `[public_api]`
table of tools/build.cfg. Standard library only, Python 3.11.0 or later.

    public_api.py check [--root DIR]    fail when a snapshot differs, is missing or has no package
    public_api.py update [--root DIR]   rewrite the snapshots

Keys of `[public_api]` (all optional):

    packages   strs  []                  packages to snapshot
    toolchain  str   ""                  rustup toolchain, passed as `cargo +TOOLCHAIN`; empty adds none
    sdk        str   ""                  "GROUP NAME": the toolchain version from the SDK lock file
                                         named by `[sdk] lock` (default tools/sdk.lock); not with toolchain
    layout     str   "dir"               "dir": PATH/PACKAGE.txt per package; "file": one file at PATH
    path       str   "tools/public-api"  snapshot directory, or file ("tools/public-api.txt" for "file")
    args       strs  ["--all-features", "-sss", "--color", "never"]   arguments after the package
    cargo      str   "cargo"             the cargo executable

Exit codes: 0 ok, 1 problems (one line each on stderr, prefixed `public_api.py:`), 2 usage.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any

NAME = "public_api.py"
CONFIG = "tools/build.cfg"
KEYS: dict[str, type] = {
    "packages": list,
    "toolchain": str,
    "sdk": str,
    "layout": str,
    "path": str,
    "args": list,
    "cargo": str,
}
DEFAULTS: dict[str, Any] = {
    "packages": [],
    "toolchain": "",
    "sdk": "",
    "layout": "dir",
    "path": None,
    "args": ["--all-features", "-sss", "--color", "never"],
    "cargo": "cargo",
}
DEFAULT_PATHS = {"dir": "tools/public-api", "file": "tools/public-api.txt"}


class Problem(Exception):
    """What stops a command; each line of its text is printed as one problem."""


def read_config(root: Path) -> dict[str, Any]:
    path = root / CONFIG
    if not path.is_file():
        return {}
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as error:
        raise Problem(f"build.cfg: {error}") from error


def settings(config: dict[str, Any]) -> dict[str, Any]:
    """The `[public_api]` table with defaults filled in; every fault is reported."""
    table = config.get("public_api", {})
    if not isinstance(table, dict):
        raise Problem("build.cfg: public_api: expected table")
    errors: list[str] = []
    for key, value in table.items():
        if key not in KEYS:
            errors.append(f"build.cfg: public_api.{key}: unknown key")
        elif not isinstance(value, KEYS[key]) or (
            KEYS[key] is list and not all(isinstance(v, str) for v in value)
        ):
            kind = "strs" if KEYS[key] is list else "str"
            errors.append(f"build.cfg: public_api.{key}: expected {kind}, found {type(value).__name__}")
    if errors:
        raise Problem("\n".join(errors))
    out = {**DEFAULTS, **table}
    if out["layout"] not in DEFAULT_PATHS:
        raise Problem(f"build.cfg: public_api.layout: expected \"dir\" or \"file\", found {out['layout']!r}")
    if out["path"] is None:
        out["path"] = DEFAULT_PATHS[out["layout"]]
    if out["toolchain"] and out["sdk"]:
        raise Problem("build.cfg: public_api: toolchain and sdk are alternatives; set only one")
    if out["layout"] == "file" and len(out["packages"]) > 1:
        raise Problem("build.cfg: public_api.packages: layout \"file\" takes at most one package")
    return out


def sdk_version(config: dict[str, Any], root: Path, item: str) -> str:
    """The version of the first SDK lock row whose group and name match `item` ("GROUP NAME")."""
    parts = item.split()
    if len(parts) != 2:
        raise Problem(f"build.cfg: public_api.sdk: expected \"GROUP NAME\", found {item!r}")
    sdk = config.get("sdk", {})
    lock = sdk.get("lock", "tools/sdk.lock") if isinstance(sdk, dict) else "tools/sdk.lock"
    if not isinstance(lock, str):
        raise Problem("build.cfg: sdk.lock: expected str")
    lock_path = root / lock
    if not lock_path.is_file():
        raise Problem(f"{lock}: SDK lock file not found")
    for line in lock_path.read_text(encoding="utf-8").splitlines():
        fields = line.split("#", 1)[0].split()
        if len(fields) >= 3 and fields[0] == parts[0] and fields[1] == parts[1]:
            return fields[2]
    raise Problem(f"{lock}: no row for {item}")


def run_command(cmd: list[str], cwd: Path) -> tuple[int, str]:
    """Runs a command and returns (exit code, stdout as text); a missing program is code 127."""
    try:
        done = subprocess.run(cmd, cwd=cwd, capture_output=True)
    except FileNotFoundError:
        return 127, ""
    return done.returncode, done.stdout.decode("utf-8", "replace")


def snapshot_text(stdout: str) -> str:
    return "\n".join(stdout.splitlines()) + "\n"


def targets(cfg: dict[str, Any]) -> list[tuple[str, str, Path]]:
    """(label, package or "", snapshot path relative to the root) for every snapshot."""
    packages = cfg["packages"]
    if cfg["layout"] == "file":
        package = packages[0] if packages else ""
        return [(package or cfg["path"], package, Path(cfg["path"]))]
    return [(p, p, Path(cfg["path"]) / f"{p}.txt") for p in packages]


def extract(cfg: dict[str, Any], toolchain: str, package: str, label: str, root: Path) -> str:
    cmd = [cfg["cargo"]]
    if toolchain:
        cmd.append(f"+{toolchain}")
    cmd += ["public-api", "--manifest-path", "Cargo.toml"]
    if package:
        cmd += ["--package", package]
    cmd += cfg["args"]
    code, stdout = run_command(cmd, root)
    if code != 0:
        raise Problem(f"Public API extraction failed for {label}")
    return snapshot_text(stdout)


def run(command: str, root: Path) -> int:
    config = read_config(root)
    cfg = settings(config)
    toolchain = cfg["toolchain"] or (sdk_version(config, root, cfg["sdk"]) if cfg["sdk"] else "")
    problems: list[str] = []
    for label, package, rel in targets(cfg):
        try:
            text = extract(cfg, toolchain, package, label, root)
        except Problem as error:
            problems.append(str(error))
            continue
        file = root / rel
        if command == "update":
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(text.encode("utf-8"))
            print(f"updated {label}")
        elif not file.is_file():
            problems.append(f"Missing public API snapshot {rel.as_posix()}; run public_api.py update")
        elif file.read_bytes() != text.encode("utf-8"):
            problems.append(
                f"Public API snapshot differs for {label}; run public_api.py update and review the diff"
            )
        else:
            print(f"verified {label}")
    if command == "check" and cfg["layout"] == "dir":
        owned = {rel.name for _, _, rel in targets(cfg)}
        directory = root / cfg["path"]
        if directory.is_dir():
            for file in sorted(directory.iterdir()):
                if file.is_file() and file.name not in owned:
                    problems.append(f"stale snapshot {(Path(cfg['path']) / file.name).as_posix()}")
    if problems:
        raise Problem("\n".join(problems))
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog=NAME, description="Public API snapshots of Rust packages.")
    parser.add_argument("command", choices=["check", "update"])
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    try:
        args = parser.parse_args(argv)
    except SystemExit as exit_:
        return 0 if exit_.code in (0, None) else 2
    try:
        return run(args.command, args.root.resolve())
    except Problem as error:
        for line in str(error).splitlines():
            print(f"{NAME}: {line}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
