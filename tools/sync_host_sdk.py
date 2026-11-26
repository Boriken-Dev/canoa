#!/usr/bin/env python3
"""Syncs the JS SDK modules embedded by canoa-host and by the extension crates that carry a script library.

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
# Each destination folder and the SDK modules it keeps: a crate may only include files from inside itself.
TARGETS = {
    ROOT / "crates" / "canoa-host" / "extensions" / "sdk": [
        "kinds.js",
        "model.js",
        "profile.js",
        "secret.js",
        "source.js",
        "speech.js",
        "wait.js",
    ],
    ROOT / "extensions" / "canoa-bitwarden" / "script" / "bitwarden" / "sdk": [
        "kinds.js",
        "model.js",
        "secret.js",
    ],
}


def main() -> int:
    parser = argparse.ArgumentParser(description="Sync embedded canoa-sdk JS files into the crates that carry them.")
    parser.add_argument("--check", action="store_true", help="Exit 1 if any copy differs or is missing.")
    args = parser.parse_args()

    stale: list[str] = []
    count = 0
    for dst_dir, names in TARGETS.items():
        if not args.check:
            dst_dir.mkdir(parents=True, exist_ok=True)
        for name in names:
            count += 1
            src = SRC_DIR / name
            dst = dst_dir / name
            want = src.read_bytes()
            have = dst.read_bytes() if dst.is_file() else None
            if have != want:
                stale.append(str(dst.relative_to(ROOT)))
                if not args.check:
                    dst.write_bytes(want)
                    print(f"synced {dst.relative_to(ROOT)}")

    if args.check:
        if stale:
            print(
                f"out of date: {', '.join(stale)}; run python tools/sync_host_sdk.py",
                file=sys.stderr,
            )
            return 1
        print(f"ok ({count} sdk files match)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
