#!/usr/bin/env python3
"""Pin the Office.js files Canoa's pane carries, from github.com/OfficeDev/office-js.

`office_js.json` names one commit of the repository, the SHA-256 of every file in its tree and the files the add-in's pane
loads. Two steps use it:

  deps     (python tools/build.py deps sync) carries that commit's whole tree, unmodified, on the deps branch under
           cdn/office-js/ as [[deps.tree]] `office-js` of tools/build.cfg: the license lets the library be distributed only
           in its original form. The only step that uses the network.
  build    (crates/canoa-host/build.rs) reads the files the pane loads, and LICENSE.md, from cdn/office-js/ and
           checks each against the list. It never fetches.

The pane loads, from the folder office.js is served from, the file of each Office application and platform, the strings of each
language, and two helpers, all named by the loader in dist/office.js; `pin` derives that set from the loader of the commit it pins:

    python tools/office_js.py pin [--commit SHA]
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import re
import sys
import tarfile
import urllib.request
from pathlib import Path

LIST = Path(__file__).with_name("office_js.json")
REPOSITORY = "OfficeDev/office-js"
# The Office applications Canoa's add-in is made for; Outlook also has its own strings file.
APPLICATIONS = ("outlook", "word", "excel", "powerpoint")
LICENSE = "LICENSE.md"
_COMMIT = re.compile(r"^[0-9a-f]{40}$")
_PATH = re.compile(r"^[A-Za-z0-9._@+-]+(/[A-Za-z0-9._@+-]+)*$")


class OfficeJsError(RuntimeError):
    pass


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_list(path: Path = LIST) -> dict:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not _COMMIT.match(data["commit"]):
        raise OfficeJsError("office_js.json: commit is not a full commit id")
    for name, sha in data["files"].items():
        if not _PATH.match(name) or ".." in name.split("/") or not re.fullmatch(r"[0-9a-f]{64}", sha):
            raise OfficeJsError(f"office_js.json: bad entry {name}")
    missing = [name for name in data["carried"] if name not in data["files"]]
    if missing or LICENSE not in data["carried"]:
        raise OfficeJsError(f"office_js.json: carried names not in the tree, or no license: {missing}")
    return data


def write_list(data: dict, path: Path = LIST) -> None:
    path.write_text(json.dumps(data, indent=1) + "\n", encoding="utf-8", newline="\n")


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "Canoa-Offline-Mirror"})
    with urllib.request.urlopen(request, timeout=300) as response:
        return response.read()


def head_of(branch: str = "release") -> str:
    """The commit a branch of the repository is at now."""
    found = json.loads(fetch(f"https://api.github.com/repos/{REPOSITORY}/commits/{branch}"))["sha"]
    if not _COMMIT.match(found):
        raise OfficeJsError("GitHub answered with no commit id")
    return found


def tree_of(commit: str) -> dict[str, bytes]:
    """Every file of the commit's tree, by path, from its source archive (no clone, so no pack file)."""
    archive = fetch(f"https://codeload.github.com/{REPOSITORY}/tar.gz/{commit}")
    files: dict[str, bytes] = {}
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
        for member in tar:
            if member.isdir() or member.name == "pax_global_header":
                continue
            parts = member.name.split("/")[1:]
            name = "/".join(parts)
            if not member.isfile() or not parts or not _PATH.match(name) or ".." in parts:
                raise OfficeJsError(f"the archive holds {member.name!r}, which is not a plain file")
            files[name] = tar.extractfile(member).read()
    return files


def carried_names(loader: str, present: set[str]) -> tuple[list[str], list[str]]:
    """The dist files the pane loads, and the ones the loader names that the tree does not hold."""
    versions = re.search(r"OSF\.HostSpecificFileVersionMap=(\{.*?\});OSF", loader)
    locales = re.search(r"OSF\.SupportedLocales=(\{.*?\});", loader)
    if versions is None or locales is None:
        raise OfficeJsError("office.js no longer names its host files and locales where this tool reads them")
    by_application = json.loads(re.sub(r"([{,])([a-z0-9]+):", r'\1"\2":', versions.group(1)))
    languages = sorted(json.loads(locales.group(1)))
    wanted = ["office.js", "microsoftajax.js", "o15apptofilemappingtable.js"]
    wanted += [
        f"{application}-{system}-{version}.js"
        for application in APPLICATIONS
        for system, version in sorted(by_application[application].items())
    ]
    wanted += [f"{language}/office_strings.js" for language in languages]
    wanted += [f"{language}/outlook_strings.js" for language in languages]
    found = [f"dist/{name}" for name in wanted if f"dist/{name}" in present]
    return found, [f"dist/{name}" for name in wanted if f"dist/{name}" not in present]


def pin(commit: str) -> tuple[dict, list[str]]:
    files = tree_of(commit)
    carried, absent = carried_names(files["dist/office.js"].decode("utf-8"), set(files))
    version = re.search(rb'FileVersion:"([0-9.]+)"', files["dist/office.js"])
    data = dict(
        repository=REPOSITORY,
        commit=commit,
        file_version=version.group(1).decode() if version else None,
        carried=sorted(carried) + [LICENSE],
        files={name: digest(body) for name, body in sorted(files.items())},
    )
    return data, absent


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=("pin",))
    parser.add_argument("--commit", help="the commit to pin (default: the head of the release branch)")
    args = parser.parse_args()
    try:
        data, absent = pin(args.commit or head_of())
        write_list(data)
        print(f"pinned {data['commit']}: {len(data['files'])} files, {len(data['carried'])} carried; not in the tree: {absent or 'none'}")
    except (OfficeJsError, OSError, KeyError) as error:
        sys.exit(f"office_js.py: {error}")


if __name__ == "__main__":
    main()
