#!/usr/bin/env python3
"""Syncs the JS SDK modules embedded by canoa-host into crates/canoa-host/extensions/sdk/.

Usage:
    python tools/sync_host_sdk.py          # copy changed files
    python tools/sync_host_sdk.py --check  # fail if any copy is missing or differs
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC_DIR = ROOT / "bindings" / "js" / "canoa-sdk" / "src"
DST_DIR = ROOT / "crates" / "canoa-host" / "extensions" / "sdk"
FILES = [
    "kinds.js",
    "model.js",
    "profile.js",
    "secret.js",
    "source.js",
    "speech.js",
    "wait.js",
]


def main() -> int:
    parser = argparse.ArgumentParser(description="Sync embedded canoa-sdk JS files into canoa-host.")
    parser.add_argument("--check", action="store_true", help="Exit 1 if any copy differs or is missing.")
    args = parser.parse_args()

    stale: list[str] = []
    if not args.check:
        DST_DIR.mkdir(parents=True, exist_ok=True)
    for name in FILES:
        src = SRC_DIR / name
        dst = DST_DIR / name
        want = src.read_bytes()
        have = dst.read_bytes() if dst.is_file() else None
        if have != want:
            stale.append(name)
            if not args.check:
                dst.write_bytes(want)
                print(f"synced {dst.relative_to(ROOT)}")

    if args.check:
        if stale:
            print(
                f"crates/canoa-host/extensions/sdk is out of date ({', '.join(stale)}); "
                "run python tools/sync_host_sdk.py",
                file=sys.stderr,
            )
            return 1
        print(f"ok ({len(FILES)} sdk files match)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
