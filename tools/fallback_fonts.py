#!/usr/bin/env python3
"""The fallback fonts Flutter's web engine asks for, pinned by SHA-256 and mirrored for an offline build.

The engine looks for a glyph its bundled fonts lack (symbols, arrows, emoji, CJK) in `fontFallbackBaseUrl` + a path
from its own manifest, `font_fallback_data.dart` in the SDK's web engine sources. `fallback_fonts.sha256` lists every
such path with the digest of the file Google serves for it. Two steps use that list:

  mirror   (tools/mirror_deps.py, or this script) downloads the files into the deps branch's
           cdn/fonts.gstatic.com/s/ and refuses a file whose digest differs. The only step that uses the network.
  install  (app/tool/build.py) copies them from the deps checkout into build/web/assets/fonts/fallback/, where the
           page asks for them, and fails on a missing or changed file. It never fetches.

`pin` rewrites the list from the SDK's manifest after a Flutter upgrade; the diff of the list is then the review.
"""
from __future__ import annotations

import argparse
import hashlib
import re
import shutil
import subprocess
import sys
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

LIST = Path(__file__).with_name("fallback_fonts.sha256")
HOST = "fonts.gstatic.com"
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
    return paths


def read_list(path: Path = LIST) -> dict[str, str]:
    pinned: dict[str, str] = {}
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        digest, _, name = line.partition("  ")
        if not re.fullmatch(r"[0-9a-f]{64}", digest) or not _SAFE.match(name):
            raise FontError(f"{path}:{number}: not '<sha256>  <path>'")
        pinned[name] = digest
    return pinned


def write_list(pinned: dict[str, str], path: Path = LIST) -> None:
    head = "# SHA-256 of each fallback font the Flutter web engine requests, and its path under fonts.gstatic.com/s/.\n"
    path.write_text(head + "".join(f"{pinned[name]}  {name}\n" for name in sorted(pinned)), encoding="utf-8", newline="\n")


def check_engine(pinned: dict[str, str], root: Path) -> None:
    """The list must name exactly what this SDK's engine requests."""
    wanted = set(engine_paths(root))
    if wanted != set(pinned):
        extra, absent = sorted(set(pinned) - wanted), sorted(wanted - set(pinned))
        raise FontError(
            f"fallback_fonts.sha256 does not match this Flutter's engine ({len(absent)} unlisted, {len(extra)} not requested"
            f"{'; first unlisted ' + absent[0] if absent else ''}): run tools/fallback_fonts.py pin")


def _digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _download(name: str) -> bytes:
    request = urllib.request.Request(f"https://{HOST}/s/{name}", headers={"User-Agent": "Canoa-Offline-Mirror"})
    with urllib.request.urlopen(request, timeout=90) as response:
        return response.read()


def mirror(deps: Path, pinned: dict[str, str]) -> int:
    """Put every pinned file in deps/cdn/fonts.gstatic.com/s/, downloading what is missing; the byte total."""
    base = deps / "cdn" / HOST / "s"

    def one(item: tuple[str, str]) -> int:
        name, digest = item
        target = base / name
        if target.is_file() and _digest(target.read_bytes()) == digest:
            return target.stat().st_size
        data = _download(name)
        if _digest(data) != digest:
            raise FontError(f"{name} changed upstream: its SHA-256 is {_digest(data)}, the list pins {digest}")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        return len(data)

    with ThreadPoolExecutor(max_workers=12) as pool:
        return sum(pool.map(one, pinned.items()))


def install(deps: Path | None, dist: Path, pinned: dict[str, str]) -> int:
    """Copy the pinned files from the deps checkout into dist/assets/fonts/fallback/; never touches the network."""
    base = None if deps is None else deps / "cdn" / HOST / "s"
    problems = []
    total = 0
    for name, digest in pinned.items():
        source = None if base is None else base / name
        if source is None or not source.is_file():
            problems.append(f"{name}: not in the deps mirror")
            continue
        data = source.read_bytes()
        if _digest(data) != digest:
            problems.append(f"{name}: its SHA-256 differs from the list")
            continue
        target = dist / "assets" / "fonts" / "fallback" / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        total += len(data)
    if problems:
        more = f" (and {len(problems) - 3} more)" if len(problems) > 3 else ""
        raise FontError(
            "the fallback fonts are not all in the deps mirror: " + "; ".join(problems[:3]) + more
            + ". Refresh the deps branch (tools/mirror_deps.py, or tools/fallback_fonts.py mirror <deps checkout>)")
    return total


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=("mirror", "pin"))
    parser.add_argument("deps", nargs="?", type=Path, help="mirror: the deps checkout to fill; pin: where to download to check")
    args = parser.parse_args()
    try:
        root = flutter_root()
        if args.command == "pin":
            if args.deps is None:
                parser.error("pin needs a directory to download into")
            old = read_list() if LIST.is_file() else {}
            found: dict[str, str] = {}
            for name in engine_paths(root):
                target = args.deps / "cdn" / HOST / "s" / name
                data = target.read_bytes() if target.is_file() else _download(name)
                found[name] = _digest(data)
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
            write_list(found)
            print(f"pinned {len(found)} fonts ({len(set(found) - set(old))} new, {len(set(old) - set(found))} dropped)")
        else:
            if args.deps is None:
                parser.error("mirror needs the deps checkout")
            pinned = read_list()
            check_engine(pinned, root)
            print(f"mirrored {len(pinned)} fonts, {mirror(args.deps, pinned)} bytes")
    except FontError as error:
        sys.exit(f"fallback_fonts.py: {error}")


if __name__ == "__main__":
    main()
