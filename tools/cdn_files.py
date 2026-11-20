#!/usr/bin/env python3
"""The files Canoa's builds take from a CDN, pinned by SHA-256 in `cdn_files.json` and mirrored for an offline build.

Each entry is a host, a path on it and the digest of the file served there. Two steps use the list:

  mirror   (tools/mirror_deps.py, or this script) downloads every file into the deps branch's cdn/<host><path> and
           refuses one whose digest differs. The only step that uses the network.
  build    (app/tool/build.py) reads the mirror, checks the digests again and never fetches.

Office.js is not here: it is mirrored whole from its repository, see office_js.py.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

LIST = Path(__file__).with_name("cdn_files.json")
_HOST = re.compile(r"^[a-z0-9]([a-z0-9.-]*[a-z0-9])?$")
_PATH = re.compile(r"^/[A-Za-z0-9._/-]+$")

README = """# CDN files

Files the builds would otherwise take from a CDN or a repository, mirrored here so no build or run needs the network for them.
`manifest.json` records what is here; `tools/mirror_deps.py` fills this folder, refuses a file whose digest differs from the one
pinned on the source branch, and is the only thing that writes it. No build fetches from a CDN.

| Folder | What | Read by |
| --- | --- | --- |
| `fonts.gstatic.com/s/` | The fonts Flutter's web engine falls back to for a glyph Roboto lacks (symbols, emoji, CJK, every script), pinned by `tools/cdn_files.json` and `tools/fallback_fonts.sha256` | `app/tool/build.py`, which copies them into the web build |
| `office-js/` | The whole tree of one commit of github.com/OfficeDev/office-js (its commit and every file's SHA-256 are in `manifest.json`), unmodified, with its `LICENSE.md`. Nothing is renamed, filtered or rewritten, and nothing is trimmed to what compiles: the owner asked for the library as published, which Microsoft's license terms require of a copy | `crates/canoa-host/build.rs`, which carries the files the add-in's pane loads, and `LICENSE.md`, in the host |

Flutter's CanvasKit is not here: the web build bundles it (`--no-web-resources-cdn`).
"""


class CdnError(RuntimeError):
    pass


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def check_entry(entry: dict) -> None:
    host, path, sha = entry.get("host", ""), entry.get("path", ""), entry.get("sha256", "")
    if not _HOST.match(host) or not _PATH.match(path) or ".." in path.split("/") or "//" in path:
        raise CdnError(f"not a CDN file: {host}{path}")
    if not re.fullmatch(r"[0-9a-f]{64}", sha):
        raise CdnError(f"{host}{path}: not a SHA-256")


def read_list(path: Path = LIST) -> dict:
    data = json.loads(path.read_text(encoding="utf-8"))
    for entry in data["files"]:
        check_entry(entry)
    return data


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "Canoa-Offline-Mirror"})
    with urllib.request.urlopen(request, timeout=90) as response:
        return response.read()


def target(deps: Path, entry: dict) -> Path:
    return deps / "cdn" / entry["host"] / entry["path"].lstrip("/")


def mirror(deps: Path, entries: list[dict], workers: int = 12) -> list[dict]:
    """Put every entry's file under deps/cdn/<host><path>, downloading what is missing or changed.

    Returns the entries with the bytes of each file. A download whose digest differs from the list is refused.
    """

    def one(entry: dict) -> dict:
        destination = target(deps, entry)
        if destination.is_file() and digest(destination.read_bytes()) == entry["sha256"]:
            data = destination.read_bytes()
        else:
            data = fetch("https://" + entry["host"] + entry["path"])
            if digest(data) != entry["sha256"]:
                raise CdnError(
                    f"{entry['host']}{entry['path']} changed upstream: its SHA-256 is {digest(data)}, "
                    f"the list pins {entry['sha256']}")
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
        return dict(host=entry["host"], path=entry["path"], sha256=entry["sha256"], bytes=len(data))

    with ThreadPoolExecutor(max_workers=workers) as pool:
        return list(pool.map(one, entries))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("deps", type=Path, help="the deps checkout to fill")
    args = parser.parse_args()
    try:
        done = mirror(args.deps, read_list()["files"])
        print(f"mirrored {len(done)} files, {sum(item['bytes'] for item in done)} bytes")
    except (CdnError, OSError) as error:
        sys.exit(f"cdn_files.py: {error}")


if __name__ == "__main__":
    main()
