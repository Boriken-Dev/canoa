#!/usr/bin/env python3
"""The documents the shipped catalogs are made from, pinned by SHA-256 and mirrored on the deps branch.

An extension that registers a pinned catalog ships a snapshot of what its source answers (`extensions/*/catalogs/*.json`), and the
source reads that answer from a publisher: Hugging Face's file listing of a commit, a `voices.json`. `catalog_documents.json`
names each of those upstream documents, by the address that serves it and the digest of its bytes. Three steps use it:

  sync DEPS_DIR     download every document, check it against its pin and write it to DEPS_DIR/catalogs/<path>, so an offline build
                    can regenerate each snapshot from CANOA_DEPS and check it (the extensions' ignored `snapshot_from_deps` tests)
  check DEPS_DIR    fail, naming what is missing, changed or stale: every file is hashed again
  refresh           (online, the owner's) read every document again and write its size and digest into the list, for the day a
                    pinned catalog is moved to a new commit

Exits 0, or 1 with one line per problem. Needs Python 3.11 or later.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import urllib.request
from pathlib import Path

LIST = Path(__file__).with_name("catalog_documents.json")
FOLDER = "catalogs"
_PATH = re.compile(r"^[A-Za-z0-9._@+-]+(/[A-Za-z0-9._@+-]+)*$")
_HEX = re.compile(r"^[0-9a-f]{64}$")


class CatalogError(RuntimeError):
    pass


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_list(path: Path = LIST) -> list[dict]:
    entries = json.loads(path.read_text(encoding="utf-8"))["documents"]
    for entry in entries:
        check_entry(entry)
    return entries


def check_entry(entry: dict) -> None:
    path = entry.get("path", "")
    if not _PATH.match(path) or ".." in path.split("/"):
        raise CatalogError(f"{path!r} is not a plain relative path")
    if not str(entry.get("url", "")).startswith("https://"):
        raise CatalogError(f"{path}: the address is not https")
    if not _HEX.match(str(entry.get("sha256", ""))):
        raise CatalogError(f"{path}: the digest is not 64 hexadecimal digits")


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "canoa-catalog-documents"})
    with urllib.request.urlopen(request, timeout=120) as reply:
        return reply.read()


def mirror(deps: Path, entries: list[dict]) -> list[str]:
    """Writes every document under `deps/catalogs/`, fetching what is missing or changed; says what it wrote."""
    wrote = []
    for entry in entries:
        target = deps / FOLDER / entry["path"]
        if target.is_file() and digest(target.read_bytes()) == entry["sha256"]:
            continue
        data = fetch(entry["url"])
        if digest(data) != entry["sha256"]:
            raise CatalogError(f"{entry['path']}: {entry['url']} answered {digest(data)}, not the pinned {entry['sha256']}")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        wrote.append(entry["path"])
    stale = problems(deps, entries, only_stale=True)
    for line in stale:
        path = deps / FOLDER / line.split(": ", 1)[0]
        path.unlink(missing_ok=True)
    return wrote


def problems(deps: Path, entries: list[dict], only_stale: bool = False) -> list[str]:
    found = []
    wanted = {entry["path"]: entry for entry in entries}
    root = deps / FOLDER
    if root.is_dir():
        for file in sorted(root.rglob("*")):
            if file.is_file() and file.relative_to(root).as_posix() not in wanted:
                found.append(f"{file.relative_to(root).as_posix()}: stale, no entry of the list names it")
    if only_stale:
        return found
    for path, entry in wanted.items():
        target = root / path
        if not target.is_file():
            found.append(f"{path}: missing")
        elif digest(target.read_bytes()) != entry["sha256"]:
            found.append(f"{path}: changed, its digest is not the pinned one")
    return found


def refresh(entries: list[dict], path: Path = LIST) -> None:
    for entry in entries:
        data = fetch(entry["url"])
        entry["sha256"] = digest(data)
        entry["bytes"] = len(data)
    path.write_text(json.dumps({"documents": entries}, indent=1) + "\n", encoding="utf-8", newline="\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("command", choices=["sync", "check", "refresh"])
    parser.add_argument("deps", nargs="?", help="the deps checkout, for sync and check")
    args = parser.parse_args()
    entries = read_list()
    if args.command == "refresh":
        refresh(entries)
        return 0
    if not args.deps:
        parser.error(f"{args.command} needs the deps folder")
    deps = Path(args.deps)
    if args.command == "sync":
        mirror(deps, entries)
    found = problems(deps, entries)
    for line in found:
        print(line)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
