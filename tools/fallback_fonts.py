#!/usr/bin/env python3
"""Rewrite the fallback fonts of tools/cdn_files.json from the Flutter SDK's web engine.

The engine looks for a glyph its bundled fonts lack (symbols, arrows, emoji, CJK) in `fontFallbackBaseUrl` + a path from its
own manifest, `font_fallback_data.dart` in the SDK's web engine sources. The index lists every such path under
fonts.gstatic.com with the SHA-256 of the file Google serves for it; `python tools/build.py deps sync` mirrors them and
`flutter fonts-check` and `fonts-install` hold the build to the index. After a Flutter upgrade:

    python tools/fallback_fonts.py pin [DIR]

downloads each file the SDK on PATH asks for (or takes it from DIR/cdn/fonts.gstatic.com/s/, a deps checkout) and
rewrites the font entries, leaving every other entry of the index as it is. The diff of the index is then the review.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import sys
import urllib.request
from pathlib import Path

LIST = Path(__file__).with_name("cdn_files.json")
HOST = "fonts.gstatic.com"
PREFIX = "/s/"
LICENCE = "OFL-1.1"
ENGINE_DATA = Path("bin/cache/flutter_web_sdk/lib/_engine/engine/font_fallback_data.dart")
_ENTRY = re.compile(r"NotoFont\(\s*'[^']+',\s*'([^']+)'")
_SAFE = re.compile(r"^[a-z0-9]+/v[0-9]+/[A-Za-z0-9._-]+\.woff2$")


class FontError(RuntimeError):
    pass


def flutter_root() -> Path:
    flutter = shutil.which("flutter")
    if flutter is None:
        raise FontError("Flutter is not on PATH")
    return Path(flutter).resolve().parents[1]


def engine_paths(root: Path) -> list[str]:
    """The font paths the SDK's engine requests, in its order."""
    data = root / ENGINE_DATA
    if not data.is_file():
        raise FontError(f"{data} is missing: run the SDK once (flutter precache --web)")
    paths = _ENTRY.findall(data.read_text(encoding="utf-8"))
    if not paths:
        raise FontError(f"{data} lists no fonts")
    bad = [name for name in paths if not _SAFE.match(name)]
    if bad:
        raise FontError(f"{data} names a font that is not a plain woff2 path: {bad[0]}")
    return paths


def _download(name: str) -> bytes:
    request = urllib.request.Request(f"https://{HOST}{PREFIX}{name}", headers={"User-Agent": "Canoa-Offline-Mirror"})
    with urllib.request.urlopen(request, timeout=90) as response:
        return response.read()


def pin(root: Path, deps: Path | None, index: Path = LIST) -> tuple[int, int, int]:
    """Rewrite the font entries of `index` from `root`'s engine; the counts of fonts, new ones and dropped ones."""
    data = json.loads(index.read_text(encoding="utf-8"))
    is_font = lambda e: e["host"] == HOST and e["path"].startswith(PREFIX)  # noqa: E731
    others = [e for e in data["files"] if not is_font(e)]
    old = {e["path"][len(PREFIX):] for e in data["files"] if is_font(e)}
    fonts = []
    for name in engine_paths(root):
        local = None if deps is None else deps / "cdn" / HOST / PREFIX.strip("/") / name
        body = local.read_bytes() if local is not None and local.is_file() else _download(name)
        fonts.append(dict(host=HOST, path=PREFIX + name, sha256=hashlib.sha256(body).hexdigest(), bytes=len(body), licence=LICENCE))
    data["files"] = sorted(others + fonts, key=lambda e: (e["host"], e["path"]))
    index.write_text(json.dumps(data, indent=1) + "\n", encoding="utf-8", newline="\n")
    names = {e["path"][len(PREFIX):] for e in fonts}
    return len(fonts), len(names - old), len(old - names)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=("pin",))
    parser.add_argument("deps", nargs="?", type=Path, help="a deps checkout to take fonts from before downloading them")
    args = parser.parse_args()
    try:
        count, new, dropped = pin(flutter_root(), args.deps)
    except (FontError, OSError) as error:
        sys.exit(f"fallback_fonts.py: {error}")
    print(f"pinned {count} fonts ({new} new, {dropped} dropped)")


if __name__ == "__main__":
    main()
