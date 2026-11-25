#!/usr/bin/env python3
"""Repin the documents the shipped catalog snapshots are made from.

An extension that registers a pinned catalog ships a snapshot of what its source answers (`extensions/*/catalogs/*.json`), and the
source reads that answer from a publisher: Hugging Face's file listing of a commit, a `voices.json`. Each of those upstream documents
is an entry of tools/cdn_files.json (host `huggingface.co`, the address's path, the SHA-256 of its bytes), so `python tools/build.py
deps sync` mirrors it to the deps branch as cdn/huggingface.co/<path> and refuses a document that changed, and the extensions'
ignored `snapshot_from_deps` tests regenerate each snapshot from CANOA_DEPS with no fetch.

    python tools/catalog_documents.py refresh

(online, the owner's) reads every such document again and writes its size and digest into the index, for the day a pinned catalog is
moved to a new commit: change the entry's path to the new address first. The diff of the index is the review.
"""
from __future__ import annotations

import hashlib
import json
import sys
import urllib.request
from pathlib import Path

LIST = Path(__file__).with_name("cdn_files.json")
HOST = "huggingface.co"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "canoa-catalog-documents"})
    with urllib.request.urlopen(request, timeout=120) as reply:
        return reply.read()


def refresh(path: Path = LIST) -> list[str]:
    """Rewrite the digest and size of each catalog document of the index; the paths that changed."""
    data = json.loads(path.read_text(encoding="utf-8"))
    changed = []
    for entry in data["files"]:
        if entry["host"] != HOST:
            continue
        body = fetch(f"https://{entry['host']}{entry['path']}")
        if entry["sha256"] != digest(body):
            changed.append(entry["path"])
        entry["sha256"] = digest(body)
        entry["bytes"] = len(body)
    path.write_text(json.dumps(data, indent=1) + "\n", encoding="utf-8", newline="\n")
    return changed


def main() -> int:
    if sys.argv[1:] != ["refresh"]:
        print(__doc__)
        return 2
    changed = refresh()
    print(f"refreshed the catalog documents; changed: {changed or 'none'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
