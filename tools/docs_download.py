"""MkDocs hook: the Download button of the Quick Start, written from the latest GitHub release.

Enabled by `hooks:` in mkdocs.yml. A page whose Markdown holds `<!-- download -->` gets the button's HTML in its place, and the
theme override (`docs/overrides/main.html`) adds the inline style and script to that page only.

The release is the repository's latest, read from the GitHub API at build time (`repo_url` names the repository). Each asset is
sorted by its file name into Windows, Linux or macOS and an architecture, or the browser extension; an asset that is none of
those (a checksum, a signature) is left out. A link is written only for a file the release holds, at the address GitHub gave it.

With no release, or none the API can be asked about, the button says so and points at the build instructions, so the page never
links a file that is not there. `CANOA_DOCS_RELEASE` overrides the source: a path to a release's JSON (the API's own shape), or
`none` for no release and no network. An unreachable network is no release; any other failure of the API is a warning.
"""
from __future__ import annotations

import html
import json
import logging
import os
import re
import urllib.error
import urllib.request
from pathlib import Path

MARKER = "<!-- download -->"
HOSTS = ("https://github.com/",)
TIMEOUT_SECONDS = 15

log = logging.getLogger("mkdocs.hooks.docs_download")

_state: dict = {"release": None, "repo": "", "name": ""}

_IGNORED = re.compile(r"(\.sha256|\.sha512|\.sig|\.asc|\.pem|\.sbom|\.json|\.txt|\.md|sha256sums?|checksums?)(\.|$)", re.I)
_OS = (
    ("windows", re.compile(r"(\.exe|\.msi)$|windows|win64|win32|[-_.]win[-_.]", re.I)),
    ("linux", re.compile(r"\.appimage$|linux", re.I)),
    ("macos", re.compile(r"\.dmg$|macos|darwin|osx", re.I)),
)
_ARCH = (
    ("arm64", re.compile(r"arm64|aarch64", re.I)),
    ("x64", re.compile(r"x64|x86[-_]64|amd64|win64", re.I)),
)
_EXTENSION = re.compile(r"extension|chromium|browser", re.I)
OS_NAMES = {"windows": "Windows", "linux": "Linux", "macos": "macOS"}
ARCH_NAMES = {"x64": "x64", "arm64": "Arm64"}
ORDER = [
    ("windows", "x64"), ("windows", "arm64"), ("windows", None),
    ("linux", "x64"), ("linux", "arm64"), ("linux", None),
    ("macos", "arm64"), ("macos", "x64"), ("macos", None),
]


def classify(name: str) -> tuple[str, str | None] | None:
    """`(os, arch)` of a release file by its name, `("extension", None)` for the browser extension, `None` for anything else."""
    if _IGNORED.search(name):
        return None
    if name.lower().endswith(".zip"):
        return ("extension", None) if _EXTENSION.search(name) else None
    for os_name, pattern in _OS:
        if pattern.search(name):
            arch = next((a for a, p in _ARCH if p.search(name)), None)
            return os_name, arch
    return None


def options(release: dict) -> list[dict]:
    """The downloads a release offers, in the order the menu shows them: `key`, `label`, `file` and `url`."""
    found: dict[tuple[str, str | None], dict] = {}
    extension = None
    for asset in release.get("assets") or []:
        name = str(asset.get("name") or "")
        url = str(asset.get("browser_download_url") or "")
        if not name or not url.startswith(HOSTS):
            continue
        kind = classify(name)
        if kind is None:
            continue
        if kind[0] == "extension":
            extension = extension or {"key": "extension", "label": "Browser extension (Chromium, Edge)", "file": name, "url": url}
            continue
        label = OS_NAMES[kind[0]] + (f", {ARCH_NAMES[kind[1]]}" if kind[1] else "")
        found.setdefault(kind, {"key": kind[0] + (f"-{kind[1]}" if kind[1] else ""), "label": label, "file": name, "url": url})
    listed = [found[k] for k in ORDER if k in found]
    return listed + ([extension] if extension else [])


def _get(url: str) -> object:
    request = urllib.request.Request(url, headers={"Accept": "application/vnd.github+json", "User-Agent": "docs-build"})
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        request.add_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as reply:
        return json.load(reply)


def _fetch(repo: str) -> dict | None:
    """The newest release: GitHub's latest, else the newest of all, since `latest` leaves out a prerelease."""
    source = os.environ.get("CANOA_DOCS_RELEASE", "")
    if source == "none":
        return None
    if source:
        return json.loads(Path(source).read_text(encoding="utf-8"))
    api = f"https://api.github.com/repos/{repo}/releases"
    try:
        try:
            return _get(f"{api}/latest")
        except urllib.error.HTTPError as error:
            if error.code != 404:
                raise
        newest = _get(f"{api}?per_page=1")
        return newest[0] if newest else None
    except urllib.error.HTTPError as error:
        if error.code != 404:
            log.warning("the latest release of %s could not be read (HTTP %s): the Download button says there is none", repo, error.code)
    except (urllib.error.URLError, TimeoutError, OSError) as error:
        log.info("no network for the latest release of %s (%s): the Download button says there is none", repo, error)
    return None


def _repo(url: str) -> str:
    match = re.match(r"https://github\.com/([^/]+/[^/]+?)/?$", url or "")
    return match.group(1) if match else ""


def on_config(config):
    _state["repo"] = _repo(config.repo_url)
    _state["name"] = config.site_name
    release = _fetch(_state["repo"]) if _state["repo"] else None
    _state["release"] = release if release and not release.get("draft") else None
    return config


def _link(item: dict) -> str:
    return (
        f'<li><a href="{html.escape(item["url"], quote=True)}" data-platform="{item["key"]}">'
        f'{html.escape(item["label"])}<small>{html.escape(item["file"])}</small></a></li>'
    )


def render(release: dict | None, repo: str, name: str) -> str:
    """The button's HTML. One line per element and no blank line, so Markdown passes the block through whole."""
    site = f"https://github.com/{repo}"
    if not release:
        return "\n".join([
            '<div class="canoa-download" data-release="">',
            '<div class="canoa-download__split">',
            '<a class="md-button md-button--primary canoa-download__main" href="getting-started/">Build from source</a>',
            '<button type="button" class="canoa-download__arrow" aria-label="Every way to get it" aria-haspopup="true" aria-expanded="false"></button>',
            '<ul class="canoa-download__menu">',
            '<li><a href="getting-started/">Build the desktop app<small>Windows, Linux, macOS</small></a></li>',
            '<li><a href="getting-started/#web-app">Build the web app<small>served by the host</small></a></li>',
            '<li><a href="browser/">Build the browser extension<small>Chromium, Edge</small></a></li>',
            f'<li><a href="{html.escape(site, quote=True)}">Source code<small>{html.escape(repo)}</small></a></li>',
            "</ul>",
            "</div>",
            f'<p class="canoa-download__note">{html.escape(name)} has not published a release yet, so there is no file to download. '
            "Build it from source for now; this button offers the file for your system once a release exists.</p>",
            "</div>",
        ])
    items = options(release)
    tag = str(release.get("tag_name") or "")
    page = str(release.get("html_url") or "")
    page = page if page.startswith(HOSTS) else f"{site}/releases"
    entries = [_link(item) for item in items]
    entries.append(f'<li><a href="{html.escape(page, quote=True)}">All files of this release<small>{html.escape(tag)}</small></a></li>')
    entries.append('<li><a href="getting-started/">Build from source<small>every platform</small></a></li>')
    return "\n".join([
        f'<div class="canoa-download" data-release="{html.escape(tag, quote=True)}">',
        '<div class="canoa-download__split">',
        f'<a class="md-button md-button--primary canoa-download__main" href="{html.escape(page, quote=True)}">Download {html.escape(name)}</a>',
        '<button type="button" class="canoa-download__arrow" aria-label="Every way to get it" aria-haspopup="true" aria-expanded="false"></button>',
        '<ul class="canoa-download__menu">',
        *entries,
        "</ul>",
        "</div>",
        f'<p class="canoa-download__note">Version {html.escape(tag)}. The button offers the file for the system this page is opened on; '
        "the arrow lists every file.</p>",
        "</div>",
    ])


def on_page_markdown(markdown, page, config, files):
    if MARKER not in markdown:
        return markdown
    page.meta["download"] = True
    return markdown.replace(MARKER, render(_state["release"], _state["repo"], _state["name"]))
