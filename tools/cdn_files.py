#!/usr/bin/env python3
"""The files Canoa's builds take from a CDN, pinned by SHA-256 in `cdn_files.json` and mirrored for an offline build.

Each entry is a host, a path on it and the digest of the file served there. Two steps use the list:

  mirror   (tools/mirror_deps.py, or this script) downloads every file into the deps branch's cdn/<host><path> and
           refuses one whose digest differs. The only step that uses the network.
  build    (app/tool/build.py for the fonts, crates/canoa-host/build.rs for Office.js) reads the mirror, checks the
           digests again and never fetches.

Office.js is a loader: it asks, from the folder it was loaded from, for one file of the Office application and platform
it runs in, for the strings of the person's language and for a few helpers. `pin-office` reads the loader the CDN serves
at `/1/office.js`, which names exactly those files, downloads them and rewrites the Office.js entries of the list; the
diff of the list is then the review. `/1/` is an alias Microsoft moves, so the digests, not the path, say which copy this is.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

LIST = Path(__file__).with_name("cdn_files.json")
OFFICE_HOST = "officeapis.public.onecdn.static.microsoft"
OFFICE_BASE = f"https://{OFFICE_HOST}/1/"
# The Office applications Canoa's add-in is made for; Outlook also has its own strings file.
OFFICE_APPLICATIONS = ("outlook", "word", "excel", "powerpoint")
_HOST = re.compile(r"^[a-z0-9]([a-z0-9.-]*[a-z0-9])?$")
_PATH = re.compile(r"^/[A-Za-z0-9._/-]+$")


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


def write_list(data: dict, path: Path = LIST) -> None:
    text = json.dumps(data, indent=1, ensure_ascii=False)
    path.write_text(text + "\n", encoding="utf-8", newline="\n")


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


README = """# CDN files

Files the builds would otherwise take from a CDN, mirrored here so no build or run needs the network for them. Each is pinned by
SHA-256 in `tools/cdn_files.json` on the source branch, and `manifest.json` records what is here. `tools/mirror_deps.py` fills this
folder and refuses a file whose digest differs; nothing else writes it, and no build fetches from the CDN.

| Folder | What | Read by |
| --- | --- | --- |
| `fonts.gstatic.com/s/` | The fonts Flutter's web engine falls back to for a glyph Roboto lacks (symbols, emoji, CJK, every script) | `app/tool/build.py`, which copies them into the web build |
| `officeapis.public.onecdn.static.microsoft/1/` | Office.js and the files it loads: the file of each Office application and platform, the strings of each language, helpers | `crates/canoa-host/build.rs`, which carries them in the host; Microsoft's license terms for Office.js apply to them |

Flutter's CanvasKit is not here: the web build bundles it (`--no-web-resources-cdn`).
"""


def office_names(loader: str) -> tuple[list[str], list[str]]:
    """The files Office.js's loader asks for: those every run needs, and the per-platform ones that may not exist."""
    versions = re.search(r"OSF\.HostSpecificFileVersionMap=(\{.*?\});OSF", loader)
    locales = re.search(r"OSF\.SupportedLocales=(\{.*?\});", loader)
    if versions is None or locales is None:
        raise CdnError("office.js no longer names its host files and locales where this tool reads them")
    by_application = json.loads(re.sub(r"([{,])([a-z0-9]+):", r'\1"\2":', versions.group(1)))
    languages = sorted(json.loads(locales.group(1)))
    always = ["office.js", "microsoftajax.js", "o15apptofilemappingtable.js"]
    always += [f"{language}/office_strings.js" for language in languages]
    always += [f"{language}/outlook_strings.js" for language in languages]
    platform = [
        f"{application}-{system}-{version}.js"
        for application in OFFICE_APPLICATIONS
        for system, version in sorted(by_application[application].items())
    ]
    return always, platform


def pin_office(data: dict, workers: int = 12) -> tuple[dict, dict[str, bytes], list[str]]:
    """The list with its Office.js entries replaced by what the CDN serves now, the files, and the names that are absent."""
    loader = fetch(OFFICE_BASE + "office.js")
    version = re.search(rb'FileVersion:"([0-9.]+)"', loader)
    always, platform = office_names(loader.decode("utf-8"))
    files: dict[str, bytes] = {"office.js": loader}

    def one(name: str) -> tuple[str, bytes | None]:
        try:
            return name, fetch(OFFICE_BASE + name)
        except urllib.error.HTTPError as error:
            if error.code == 404 and name in platform:
                return name, None
            raise

    with ThreadPoolExecutor(max_workers=workers) as pool:
        got = dict(pool.map(one, [name for name in always + platform if name != "office.js"]))
    absent = sorted(name for name, body in got.items() if body is None)
    files.update({name: body for name, body in got.items() if body is not None})
    kept = [entry for entry in data["files"] if entry["host"] != OFFICE_HOST]
    pinned = [
        dict(host=OFFICE_HOST, path="/1/" + name, sha256=digest(body), bytes=len(body), licence="Microsoft Office.js")
        for name, body in sorted(files.items())
    ]
    out = dict(data)
    out["files"] = kept + pinned
    out["office_js"] = dict(
        file_version=version.group(1).decode() if version else None,
        not_served=absent,
        not_carried=[
            "telemetry/oteljs_agave.js", "ariatelemetry/aria-web-telemetry.js",
            "webauth/webauth.browserauth*.js (Office's sign-in token calls, which Canoa's add-in does not make)",
            "officeconfig.json", "*.debug.js"],
    )
    return out, files, absent


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=("mirror", "pin-office"))
    parser.add_argument("deps", type=Path, help="mirror: the deps checkout to fill; pin-office: where to put the files")
    args = parser.parse_args()
    try:
        data = read_list()
        if args.command == "mirror":
            done = mirror(args.deps, data["files"])
            print(f"mirrored {len(done)} files, {sum(item['bytes'] for item in done)} bytes")
        else:
            data, files, absent = pin_office(data)
            for name, body in files.items():
                destination = args.deps / "cdn" / OFFICE_HOST / "1" / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(body)
            write_list(data)
            print(f"pinned {len(files)} Office.js files ({', '.join(absent) or 'none'} not served)")
    except (CdnError, OSError) as error:
        sys.exit(f"cdn_files.py: {error}")


if __name__ == "__main__":
    main()
