#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# From Boriken-Dev/agent-overlays, overlays/release/repo/tools/build.py, at commit 28bb67e0a5226390dba012f4b73b22f5427d8fd1.
# Copyright (c) 2026 Jose A.
"""One build tool for a repository: vendored crates, scans, SDK lock, offline and release builds.

This file is copied into a repository as tools/build.py and configured by tools/build.cfg; a section the
configuration leaves out is off. Standard library only, Python 3.11.0 or later.

    build.py help [config]        the command surface, or every build.cfg key with its default
    build.py self check|update    keep this copy equal to the one it was taken from
    build.py deps ...             the deps branch: crates, pub packages and CDN files
    build.py scan ...             refuse archives, executables, keys and opaque binaries
    build.py sdk ...              the SDK lock: lint, agree with the repository, install
    build.py offline plan         what an offline build of the repository needs, as JSON
    build.py release ...          release binaries, a universal macOS binary, container images
    build.py container ...        build or test in a container from the deps branch, with .env overrides
    build.py flutter ...          plugin junctions and the pinned Flutter checkout

Exit codes: 0 ok, 1 problems (one line each on stderr, prefixed `build.py:`), 2 usage.
"""

from __future__ import annotations

import argparse
import collections
import fnmatch
import gzip
import hashlib
import io
import json
import math
import os
import platform
import posixpath
import re
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
import tomllib
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any, Callable, Iterator, NamedTuple

CONFIG = "tools/build.cfg"
SELF_NAME = "build.py"
UPSTREAM_DIR = "overlays/release/repo/tools"
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
IS_WINDOWS = os.name == "nt"


class Problem(Exception):
    """What stops a command; each line of its text is printed as one problem."""


class Usage(Exception):
    """A command line that cannot be run, which exits 2."""


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run_git(*args: str, cwd: Path | None = None, binary: bool = False) -> Any:
    """Runs git and returns its stdout stripped (bytes when `binary`); a failure is a Problem."""
    command = ["git", *(["-C", str(cwd)] if cwd else []), *args]
    try:
        done = subprocess.run(command, capture_output=True)
    except FileNotFoundError as error:
        raise Problem("git is not installed") from error
    if done.returncode != 0:
        raise Problem(f"git {' '.join(args)}: {done.stderr.decode('utf-8', 'replace').strip()}")
    return done.stdout if binary else done.stdout.decode("utf-8").strip()


def git_ok(*args: str, cwd: Path | None = None) -> bool:
    command = ["git", *(["-C", str(cwd)] if cwd else []), *args]
    return subprocess.run(command, capture_output=True).returncode == 0


# Network settings, replaced from `[net]` once the configuration is read.
NET = {"retries": 3, "timeout": 120, "user_agent": "build.py"}


def fetch(url: str) -> bytes:
    """The body of a URL, retried on a connection failure or a server error."""
    request = urllib.request.Request(url, headers={"User-Agent": NET["user_agent"]})
    failure: Exception | None = None
    for _ in range(max(1, NET["retries"])):
        try:
            with urllib.request.urlopen(request, timeout=NET["timeout"]) as response:
                return response.read()
        except urllib.error.HTTPError as error:
            if error.code < 500:
                raise Problem(f"{url}: {error}") from error
            failure = error
        except OSError as error:
            failure = error
    raise Problem(f"{url}: {failure}")


# --- configuration ---------------------------------------------------------------------------

REQUIRED = object()


class Key(NamedTuple):
    """One build.cfg key. `kind` is str, int, bool, strs (list of strings), map (string to string),
    table (fixed keys) or tables (a list of tables with fixed keys)."""

    kind: str
    default: Any
    doc: str
    keys: dict[str, "Key"] | None = None


class Table(dict):
    """A validated table whose keys read as attributes."""

    def __getattr__(self, name: str) -> Any:
        try:
            return self[name]
        except KeyError as error:
            raise AttributeError(name) from error


# Each section adds its own table here when it is defined below.
SCHEMA: dict[str, Key] = {}
SCHEMA["tools"] = Key(
    "table",
    None,
    "The files copied from the overlay into this repository.",
    {
        "files": Key("strs", ["build.py"], "Files under tools/ that `self check` holds equal to the overlay's."),
        "source": Key("str", "Boriken-Dev/agent-overlays", "The GitHub repository the files come from."),
    },
)
SCHEMA["net"] = Key(
    "table",
    None,
    "Downloads.",
    {
        "retries": Key("int", 3, "Attempts per download."),
        "timeout": Key("int", 120, "Seconds before one attempt is abandoned."),
        "user_agent": Key("str", "build.py", "The User-Agent header."),
    },
)


def default_of(key: Key) -> Any:
    if key.kind == "table":
        return Table({name: default_of(child) for name, child in (key.keys or {}).items()})
    if key.kind == "tables":
        return []
    if key.default is REQUIRED:
        return None
    return list(key.default) if isinstance(key.default, list) else dict(key.default) if isinstance(key.default, dict) else key.default


def _check(value: Any, key: Key, path: str, errors: list[str]) -> Any:
    kind = key.kind
    if kind == "str" and isinstance(value, str):
        return value
    if kind == "int" and isinstance(value, int) and not isinstance(value, bool):
        return value
    if kind == "bool" and isinstance(value, bool):
        return value
    if kind == "strs" and isinstance(value, list) and all(isinstance(v, str) for v in value):
        return list(value)
    if kind == "map" and isinstance(value, dict) and all(isinstance(v, str) for v in value.values()):
        return dict(value)
    if kind == "table" and isinstance(value, dict):
        return _check_table(value, key.keys or {}, path, errors)
    if kind == "tables" and isinstance(value, list) and all(isinstance(v, dict) for v in value):
        return [_check_table(v, key.keys or {}, f"{path}[{i}]", errors) for i, v in enumerate(value)]
    errors.append(f"{path}: expected {key.kind}, found {type(value).__name__}")
    return default_of(key)


def _check_table(table: dict, keys: dict[str, Key], path: str, errors: list[str]) -> Table:
    out = Table()
    for name in table:
        if name not in keys:
            errors.append(f"{path + '.' if path else ''}{name}: unknown key")
    for name, key in keys.items():
        where = f"{path + '.' if path else ''}{name}"
        if name in table:
            out[name] = _check(table[name], key, where, errors)
        elif key.default is REQUIRED:
            errors.append(f"{where}: required")
            out[name] = None
        else:
            out[name] = default_of(key)
    return out


def parse_config(text: str, name: str = "build.cfg") -> Table:
    """The configuration with every default filled in; unknown keys are refused with their path."""
    try:
        raw = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        raise Problem(f"{name}: {error}") from error
    errors: list[str] = []
    table = _check_table(raw, SCHEMA, "", errors)
    if errors:
        raise Problem("\n".join(f"{name}: {line}" for line in errors))
    NET.update({k: table.net[k] for k in ("retries", "timeout", "user_agent")})
    return table


def load_config(root: Path, path: Path | None = None) -> Table:
    path = path or root / CONFIG
    return parse_config(path.read_text(encoding="utf-8") if path.is_file() else "", path.name if path != root / CONFIG else CONFIG)


def config_help() -> str:
    """The table `parse_config` validates against: every key, its kind, default and meaning."""
    lines: list[str] = []

    def shown(key: Key) -> str:
        if key.default is REQUIRED:
            return "required"
        return repr(key.default) if key.kind not in ("table", "tables") else ""

    def walk(keys: dict[str, Key], path: str) -> None:
        for name, key in keys.items():
            where = f"{path}.{name}" if path else name
            if key.kind == "table":
                lines.append(f"[{where}]  {key.doc}")
                walk(key.keys or {}, where)
            elif key.kind == "tables":
                lines.append(f"[[{where}]]  {key.doc}")
                walk(key.keys or {}, where)
            else:
                lines.append(f"  {where}  ({key.kind}, {shown(key)})  {key.doc}")

    walk(SCHEMA, "")
    return "\n".join(lines)


class Ctx:
    """The repository a command runs on: its root and, read when first asked for, its configuration."""

    def __init__(self, root: Path, config: Path | None = None) -> None:
        self.root = root
        self.config = config
        self._cfg: Table | None = None
        self.quick = False
        self.pubspec_lock: Path | None = None

    @property
    def cfg(self) -> Table:
        if self._cfg is None:
            self._cfg = load_config(self.root, self.config)
        return self._cfg


# --- self: keep this copy equal to the overlay's ----------------------------------------------

HEADER = re.compile(rb"^# From (?P<repo>\S+), (?P<path>\S+), at commit (?P<commit>SOURCE|[0-9a-f]{40})\.$")


def header_of(data: bytes) -> re.Match[bytes] | None:
    lines = data.split(b"\n", 3)
    return HEADER.match(lines[2]) if len(lines) > 3 else None


def upstream(cfg: Table, commit: str, name: str, local: Path | None) -> bytes:
    """A file of the overlay's repository at a commit: from a local checkout, else from GitHub."""
    path = f"{UPSTREAM_DIR}/{name}"
    if local is not None:
        return run_git("show", f"{commit}:{path}", cwd=local, binary=True)
    return fetch(f"https://raw.githubusercontent.com/{cfg.tools.source}/{commit}/{path}")


def self_problems(root: Path, cfg: Table, local: Path | None = None) -> list[str]:
    """One line for each way the listed files differ from the overlay's, or are not what they claim."""
    found: list[str] = []
    listed = list(cfg.tools.files)
    commits: dict[str, list[str]] = {}
    copies: dict[str, bytes] = {}
    for name in listed:
        path = root / "tools" / name
        if not path.is_file():
            found.append(f"tools/{name}: listed in [tools] files, missing")
            continue
        data = path.read_bytes()
        match = header_of(data)
        if match is None:
            found.append(f"tools/{name}: line 3 is not '# From REPO, PATH, at commit COMMIT.'")
            continue
        if match["commit"] == b"SOURCE":
            found.append(f"tools/{name}: line 3 still says SOURCE; make the copy with `self update`")
            continue
        if match["repo"].decode() != cfg.tools.source or match["path"].decode() != f"{UPSTREAM_DIR}/{name}":
            found.append(f"tools/{name}: line 3 names {match['repo'].decode()}, {match['path'].decode()}")
            continue
        commits.setdefault(match["commit"].decode(), []).append(name)
        copies[name] = data
    tools = root / "tools"
    prefix = f"# From {cfg.tools.source}, {UPSTREAM_DIR}/".encode()
    if tools.is_dir():
        for path in sorted(p for p in tools.iterdir() if p.is_file() and p.name not in listed):
            if prefix in b"\n".join(path.read_bytes()[:2000].split(b"\n")[:4]):
                found.append(f"tools/{path.name}: has the overlay's header but is not in [tools] files")
    if len(commits) > 1:
        found.append("copies name different commits: " + "; ".join(f"{c[:12]} ({', '.join(n)})" for c, n in sorted(commits.items())))
    for commit, names in sorted(commits.items()):
        for name in names:
            try:
                expected = upstream(cfg, commit, name, local)
            except Problem as problem:
                found.append(f"tools/{name}: cannot read the overlay at {commit[:12]}: {problem}")
                continue
            lines = copies[name].split(b"\n")
            lines[2] = lines[2].replace(f"at commit {commit}.".encode(), b"at commit SOURCE.")
            actual = b"\n".join(lines)
            if actual != expected:
                theirs = expected.split(b"\n")
                where = next((i + 1 for i, (a, b) in enumerate(zip(actual.split(b"\n"), theirs)) if a != b), min(len(theirs), len(lines)) + 1)
                found.append(f"tools/{name}: differs from the overlay at {commit[:12]} (first at line {where})")
    return found


def resolve_commit(cfg: Table, rev: str, local: Path | None) -> str:
    if local is not None:
        return run_git("rev-parse", "--verify", f"{rev}^{{commit}}", cwd=local)
    if HEX40.match(rev):
        return rev
    listing = run_git("ls-remote", f"https://github.com/{cfg.tools.source}.git", rev, f"refs/heads/{rev}", f"refs/tags/{rev}^{{}}", f"refs/tags/{rev}")
    for row in listing.splitlines():
        commit, _, ref = row.partition("\t")
        if HEX40.match(commit) and (ref == rev or ref.endswith(f"/{rev}") or ref.endswith(f"/{rev}^{{}}")):
            return commit
    raise Problem(f"{cfg.tools.source} has no {rev}")


def self_check(ctx: Ctx, args: argparse.Namespace) -> int:
    found = self_problems(ctx.root, ctx.cfg, args.src)
    if found:
        raise Problem("\n".join(found))
    names = ", ".join(ctx.cfg.tools.files)
    print(f"ok: {names} equal the overlay's")
    return 0


def self_update(ctx: Ctx, args: argparse.Namespace) -> int:
    cfg = ctx.cfg
    commit = resolve_commit(cfg, args.rev, args.src)
    data = {name: upstream(cfg, commit, name, args.src) for name in cfg.tools.files}
    for name, body in data.items():
        lines = body.split(b"\n")
        if len(lines) < 4 or not (match := HEADER.match(lines[2])) or match["commit"] != b"SOURCE":
            raise Problem(f"the overlay's {name} has no 'at commit SOURCE.' on line 3")
        lines[2] = lines[2].replace(b"at commit SOURCE.", f"at commit {commit}.".encode())
        data[name] = b"\n".join(lines)
    tools = ctx.root / "tools"
    tools.mkdir(parents=True, exist_ok=True)
    for name, body in data.items():
        (tools / name).write_bytes(body)
        print(f"wrote tools/{name} at {commit[:12]}")
    found = self_problems(ctx.root, cfg, args.src)
    if found:
        raise Problem("\n".join(found))
    print(f"ok: {len(data)} file(s) at {commit}")
    return 0


def add_self(sub: argparse._SubParsersAction) -> None:
    parser = sub.add_parser("self", help="keep this copy equal to the overlay's")
    commands = parser.add_subparsers(dest="command", required=True, metavar="COMMAND")
    check = commands.add_parser("check", help="fail when a listed file differs from the overlay's at the commit it names")
    check.add_argument("--from", dest="src", type=Path, metavar="DIR", help="read a local checkout of the overlay's repository instead of GitHub")
    check.set_defaults(run=self_check)
    update = commands.add_parser("update", help="write every listed file at REV, then check")
    update.add_argument("rev", metavar="REV", help="a commit, branch or tag of the overlay's repository")
    update.add_argument("--from", dest="src", type=Path, metavar="DIR", help="read a local checkout (git -C DIR show) instead of GitHub")
    update.set_defaults(run=self_update)


# --- help --------------------------------------------------------------------------------------


def surface(parser: argparse.ArgumentParser, depth: int = 0) -> list[str]:
    """Every command with its arguments, one block each, read from the parser itself."""
    lines: list[str] = []
    for action in parser._actions:
        if isinstance(action, argparse._SubParsersAction):
            helps = {choice.dest: choice.help for choice in action._choices_actions}
            for name, child in action.choices.items():
                usage = child.format_usage().replace("usage: ", "", 1).strip()
                lines.append(f"{usage}\n    {helps.get(name) or child.description or ''}".rstrip())
                lines += surface(child, depth + 1)
    return lines


def help_command(ctx: Ctx, args: argparse.Namespace) -> int:
    if args.topic == "config":
        print(config_help())
    elif args.topic == "env":
        print(env_help())
    else:
        print(build_parser().format_help())
        print("\n".join(surface(build_parser())))
    return 0


def add_help(sub: argparse._SubParsersAction) -> None:
    parser = sub.add_parser("help", help="the command surface, `help config` for build.cfg, `help env` for the .env keys")
    parser.add_argument("topic", nargs="?", choices=("config", "env"), help="config: every build.cfg key with its default; env: every key of .env, .env.local and --env")
    parser.set_defaults(run=help_command)


# --- command line --------------------------------------------------------------------------------

SECTIONS: list[Callable[[argparse._SubParsersAction], None]] = [add_help, add_self]

# --- the classifier: what an archive scanner would refuse --------------------------------------

# Judged by a file's leading bytes: archives, encrypted data, executables, packs, keys.
FLAG_MAGIC = [
    (b"PK\x03\x04", "zip"), (b"PK\x05\x06", "zip"), (b"PK\x07\x08", "zip"),
    (b"\x1f\x8b", "gzip"), (b"BZh", "bzip2"), (b"\xfd7zXZ\x00", "xz"),
    (b"7z\xbc\xaf\x27\x1c", "7z"), (b"Rar!\x1a\x07", "rar"), (b"\x28\xb5\x2f\xfd", "zstd"),
    (b"\x04\x22\x4d\x18", "lz4"), (b"MSCF", "cab"), (b"Cr24", "crx"), (b"!<arch>", "ar/deb"),
    (b"\xed\xab\xee\xdb", "rpm"), (b"Salted__", "openssl-encrypted"),
    (b"age-encryption.org", "age-encrypted"), (b"-----BEGIN PGP MESSAGE", "pgp-encrypted"),
    (b"MZ", "windows executable"), (b"\x7fELF", "elf executable"),
    (b"\xcf\xfa\xed\xfe", "mach-o executable"), (b"\xfe\xed\xfa\xcf", "mach-o executable"),
    (b"\xca\xfe\xba\xbe", "mach-o/java class"), (b"PACK", "git pack"),
    (b"\xfftOc", "git pack index"), (b"RIDX", "git reverse index"),
    (b"\x30\x82", "der (certificate or key)"),
]  # fmt: skip
# Other kinds that are refused unless `[deps] allow` names the path.
NOTE_MAGIC = [(b"\x00asm", "wasm"), (b"%PDF", "pdf"), (b"SQLite format 3", "sqlite")]
MEDIA = [
    b"\x89PNG", b"GIF8", b"\xff\xd8\xff", b"RIFF", b"\x00\x00\x01\x00", b"wOFF", b"wOF2",
    b"\x00\x01\x00\x00", b"OTTO",
]  # fmt: skip
KEY_EXT = re.compile(r"\.(pfx|p12|jks|keystore|der|key)$", re.IGNORECASE)
# A whole PEM block with a body, not a label quoted in code or docs.
PRIVATE_KEY = re.compile(
    rb"-----BEGIN ([A-Z0-9-]+ )?PRIVATE KEY-----\r?\n(?:[A-Za-z0-9+/=]{16,}\r?\n){2,}"
    rb"[A-Za-z0-9+/=]*\r?\n?-----END ([A-Z0-9-]+ )?PRIVATE KEY-----"
)
# The markers of a PEM private key, which one dash fewer no longer reads as.
PEM_MARKER = re.compile(rb"-----(BEGIN|END) ((?:[A-Z0-9-]+ )?PRIVATE KEY)-----")
PEM_REPLACEMENT = rb"----\1 \2-----"


def entropy(data: bytes) -> float:
    counts = collections.Counter(data)
    return -sum(v / len(data) * math.log2(v / len(data)) for v in counts.values()) if data else 0.0


# Text in a single-byte encoding (ISO-8859-1, Windows-1252), or UTF-8 holding a stray such byte, reads
# as text to any scanner. It has no NUL, no control character but tab, LF, FF and CR, and at most this
# share of bytes from 0x80 up (prose in a Latin script is 1 to 5%; random bytes and code are far above).
LEGACY_TEXT_HIGH = 0.10
LEGACY_TEXT_CONTROLS = bytes([*range(0x09), 0x0B, *range(0x0E, 0x20), 0x7F])


def legacy_text(body: bytes) -> bool:
    """Whether body that is not UTF-8 is still text: mostly printable ASCII, no control characters."""
    if not body or body.translate(None, LEGACY_TEXT_CONTROLS) != body:
        return False
    return sum(b >= 0x80 for b in body) <= len(body) * LEGACY_TEXT_HIGH


def text_note(body: bytes) -> str | None:
    """A note for text that passes though it is not UTF-8, else None."""
    if body.isascii() or not legacy_text(body):
        return None
    try:
        body.decode("utf-8")
    except UnicodeDecodeError:
        return "text, not UTF-8"
    return None


def flagged_kind(body: bytes, name: str) -> str | None:
    """The kind of data a branch must not hold in a file's bytes, or None.

    Text and images pass. Archives, executables, keys, packs, wasm, pdf, sqlite and any other
    binary do not.
    """
    head = body[:600]
    if len(head) >= 262 and head[257:262] == b"ustar":
        return "tar"
    for magic, kind in FLAG_MAGIC:
        if head.startswith(magic):
            return kind
    if KEY_EXT.search(name):
        return "key or keystore file"
    for magic, kind in NOTE_MAGIC:
        if head.startswith(magic):
            return kind
    # ISO base media (HEIC, AVIF, MP4, MOV) names its brand after a 4-byte box size.
    if any(head.startswith(m) for m in MEDIA) or head[4:8] == b"ftyp":
        return None
    try:
        body.decode("utf-8")
        return "private key in text" if PRIVATE_KEY.search(body) else None
    except UnicodeDecodeError:
        pass
    if legacy_text(body):
        return "private key in text" if PRIVATE_KEY.search(body) else None
    return "opaque high-entropy data" if entropy(body[:65536]) > 7.5 else "unrecognised binary"


class Allow:
    """`[deps] allow`: entries `GLOB:KIND`, a kind of file that one path pattern may hold."""

    def __init__(self, entries: list[str] | None = None) -> None:
        self.rules: list[tuple[str, str]] = []
        for entry in entries or []:
            pattern, _, kind = entry.rpartition(":")
            if not pattern or not kind:
                raise Problem(f"deps.allow: {entry!r} is not GLOB:KIND")
            self.rules.append((pattern, kind))

    def permits(self, path: str, kind: str) -> bool:
        return any(kind == k and fnmatch.fnmatchcase(path, g) for g, k in self.rules)


# --- a Cargo.lock, a vendor directory and what makes one cover the other ----------------------------

CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
# The address crates.io's own index gives in its config.json.
CRATES_URL = "https://static.crates.io/crates/{name}/{name}-{version}.crate"
CHECKSUM = ".cargo-checksum.json"
CONFIG_MARK = "# Written by tools/build.py."
MANIFEST = "manifest.json"
MANIFEST_FORMAT = 1
# Directories at a package's top that compiling never reads, and at any depth the ones that hold
# only test data.
DROP_TOP = {
    "test", "tests", "example", "examples", "bench", "benches", "benchmark", "benchmarks", "fuzz",
    "testdata", "test-data", "test_data", "fixture", "fixtures",
}  # fmt: skip
DROP_ANY = {"testdata", "test-data", "test_data", "fixture", "fixtures"}
# What only a person working on the crate reads: documentation and CI directories at its top.
DROP_TOP |= {"doc", "docs", "ci"}
# Container files at any depth, and at the top of a package its developer scripts and tool dotfiles.
DEV_ANYWHERE = re.compile(r"^(containerfile|dockerfile)(\..+)?$|^\.dockerignore$|^docker-compose.*\.ya?ml$", re.IGNORECASE)
DEV_TOP = re.compile(
    r".+\.(sh|bat|cmd|ps1)$|^(justfile|flake\.nix|flake\.lock)$"
    r"|^\.(editorconfig|envrc|ignore|travis\.ya?ml|codecov\.ya?ml|gitlab-ci\.ya?ml|appveyor\.ya?ml)$",
    re.IGNORECASE,
)
INCLUDE = re.compile(r'include_(?:str|bytes)!\s*\(\s*"([^"]+)"\s*,?\s*\)|\binclude!\s*\(\s*"([^"]+)"\s*,?\s*\)')
INCLUDE_BYTES = re.compile(r'(\*\s*)?include_bytes!\s*\(\s*"([^"]+)"\s*,?\s*\)')
# A source file only tests, fuzzing or benchmarks compile; what it includes is not worth carrying.
TEST_ONLY = re.compile(r"(^|/)(fuzz|tests?|test_vectors|test-vectors|benches|bench|examples?)(\.rs)?(/|$)")
# The big-endian copy of a table; no target of ours has that byte order.
BIG_ENDIAN = re.compile(r"(^|[._-])bigendian([._-]|$)")
# A name Windows cannot create, which would make the checkout differ between platforms.
RESERVED = re.compile(r"^(con|prn|aux|nul|com[0-9]|lpt[0-9])(\..*)?$", re.IGNORECASE)


class Crate(NamedTuple):
    name: str
    version: str
    checksum: str

    @property
    def dir(self) -> str:
        return f"{self.name}-{self.version}"

    @property
    def file(self) -> str:
        return f"{self.name}-{self.version}.crate"


class Repo(NamedTuple):
    url: str
    commit: str

    @property
    def name(self) -> str:
        return self.url.rstrip("/").rsplit("/", 1)[-1].removesuffix(".git")


class Lock(NamedTuple):
    crates: list[Crate]
    # Each repository at a revision, with the (name, version) Cargo.lock takes from it.
    repos: dict[Repo, list[tuple[str, str]]]
    # Each git source as Cargo.lock spells it, which is what a [source] entry has to replace.
    sources: dict[str, Repo]


class Tree(NamedTuple):
    """What a crates/ holds: each package directory's files with their SHA-256, and what is loose."""

    # Directory name -> file path below it -> SHA-256, `.cargo-checksum.json` included (empty
    # when only the metadata was read).
    dirs: dict[str, dict[str, str]]
    # Directory name -> its parsed `.cargo-checksum.json`.
    sums: dict[str, dict]
    # Directory name -> first line of its Cargo.toml, which names a git package's commit.
    heads: dict[str, str]
    # Files directly in crates/, which a vendor directory does not have.
    loose: list[str]
    # Directory name -> the files in it that are not text or images, as "path: kind".
    binaries: dict[str, list[str]]


class Scan(NamedTuple):
    """A whole commit: its crates/, and every path or file a scanner would refuse."""

    tree: Tree
    flagged: list[str]


def read_lock(path: Path) -> Lock:
    lock = Lock([], {}, {})
    try:
        packages = tomllib.loads(path.read_text(encoding="utf-8")).get("package", [])
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise Problem(f"{path}: {error}") from error
    for package in packages:
        name, version, source = package["name"], package["version"], package.get("source")
        if source is None:
            continue
        if source == CRATES_IO:
            if "checksum" not in package:
                raise Problem(f"{path.name} gives {name} {version} no checksum")
            lock.crates.append(Crate(name, version, package["checksum"]))
        elif source.startswith("git+"):
            parts = urllib.parse.urlsplit(source.removeprefix("git+"))
            repo = Repo(f"{parts.scheme}://{parts.netloc}{parts.path}", parts.fragment)
            lock.sources[source] = repo
            lock.repos.setdefault(repo, []).append((name, version))
        else:
            raise Problem(f"{name} {version} comes from {source}, which only crates.io and git are")
    return lock


def wanted_packages(lock: Lock) -> dict[str, tuple[str | None, Repo | None]]:
    """Directory name -> (the registry checksum or None, the repository or None)."""
    wanted: dict[str, tuple[str | None, Repo | None]] = {}
    entries = [(c.dir, (c.checksum, None)) for c in lock.crates]
    entries += [(f"{n}-{v}", (None, r)) for r, packages in lock.repos.items() for n, v in packages]
    for name, entry in entries:
        if name in wanted:
            raise Problem(f"{name} is taken from more than one source; a vendor directory has one")
        wanted[name] = entry
    return wanted


class Adder:
    """Files into a Tree, whether they come from a directory or from a commit.

    `prefix` is what a path read here is below in the whole branch, for `[deps] allow`.
    """

    def __init__(self, tree: Tree, allow: Allow | None = None, prefix: str = "crates/") -> None:
        self.tree = tree
        self.allow = allow or Allow()
        self.prefix = prefix

    def add(self, path: str, data: bytes) -> None:
        directory, _, rest = path.partition("/")
        if not rest:
            self.tree.loose.append(directory)
            return
        self.tree.dirs.setdefault(directory, {})[rest] = sha256(data)
        if rest != CHECKSUM:
            kind = flagged_kind(data, rest.rsplit("/", 1)[-1])
            if kind and not self.allow.permits(self.prefix + path, kind):
                self.tree.binaries.setdefault(directory, []).append(f"{rest}: {kind}")
        if rest == CHECKSUM:
            try:
                self.tree.sums[directory] = json.loads(data)
            except ValueError:
                self.tree.sums[directory] = {}
        elif rest == "Cargo.toml":
            self.tree.heads[directory] = data.split(b"\n", 1)[0].decode("utf-8", "replace")


def empty_tree() -> Tree:
    return Tree({}, {}, {}, [], {})


def read_dir(crates: Path, deep: bool = True, allow: Allow | None = None) -> Tree:
    """A crates/ as it is on disk; `deep=False` hashes nothing and reads only what names a package."""
    tree = empty_tree()
    if not crates.is_dir():
        return tree
    adder = Adder(tree, allow)
    for entry in sorted(crates.iterdir()):
        if not entry.is_dir():
            tree.loose.append(entry.name)
            continue
        tree.dirs.setdefault(entry.name, {})
        try:
            tree.sums[entry.name] = json.loads((entry / CHECKSUM).read_bytes())
        except (OSError, ValueError):
            tree.sums[entry.name] = {}
        manifest = entry / "Cargo.toml"
        if manifest.is_file():
            with manifest.open("rb") as handle:
                tree.heads[entry.name] = handle.readline().decode("utf-8", "replace").rstrip("\r\n")
        if not deep:
            continue
        for path in sorted(entry.rglob("*")):
            if path.is_file():
                adder.add(f"{entry.name}/{path.relative_to(entry).as_posix()}", path.read_bytes())
    return tree


def package_problems(
    tree: Tree, name: str, checksum: str | None, repo: Repo | None, deep: bool = True
) -> list[str]:
    """What keeps one directory from being the package Cargo.lock names, for a line each."""
    where = f"crates/{name}"
    if name not in tree.dirs:
        return [f"{where}: missing, Cargo.lock names it"]
    sums = tree.sums.get(name)
    if not sums:
        return [f"{where}: no {CHECKSUM}"]
    found = []
    if sums.get("package") != checksum:
        found.append(f"{where}: package checksum {sums.get('package')}, Cargo.lock says {checksum}")
    if repo and f" at {repo.commit}" not in tree.heads.get(name, ""):
        found.append(f"{where}: not {repo.name} at {repo.commit}")
    if not deep:
        return found
    found += [f"{where}/{line}; run sync" for line in tree.binaries.get(name, [])]
    listed = sums.get("files", {})
    have = {p: d for p, d in tree.dirs[name].items() if p != CHECKSUM}
    for path in sorted(set(listed) - set(have)):
        found.append(f"{where}/{path}: listed in {CHECKSUM}, missing")
    for path in sorted(set(have) - set(listed)):
        found.append(f"{where}/{path}: not listed in {CHECKSUM}")
    for path in sorted(set(have) & set(listed)):
        if have[path] != listed[path]:
            found.append(f"{where}/{path}: SHA-256 {have[path]}, {CHECKSUM} says {listed[path]}")
    return found


def problems(lock: Lock, tree: Tree, deep: bool = True) -> list[str]:
    """One line for each thing keeping a crates/ from covering Cargo.lock, and nothing more."""
    found = []
    wanted = wanted_packages(lock)
    for name, (checksum, repo) in wanted.items():
        found += package_problems(tree, name, checksum, repo, deep)
    for name in sorted(set(tree.dirs) - set(wanted)):
        found.append(f"crates/{name}: stale, Cargo.lock does not name it")
    for name in sorted(tree.loose):
        found.append(f"crates/{name}: a file; a vendor directory holds only package directories")
    return found


# --- commits as data: every blob of a commit, with its mode ------------------------------------------


def batch_bodies(cwd: Path, shas: list[str]) -> Iterator[tuple[str, bytes]]:
    """Each object's bytes exactly as git stores them, in the order asked, from one `cat-file --batch`."""
    reader = subprocess.Popen(
        ["git", "-C", str(cwd), "cat-file", "--batch"], stdin=subprocess.PIPE, stdout=subprocess.PIPE
    )

    def feed() -> None:
        try:
            for sha in shas:
                reader.stdin.write(sha.encode() + b"\n")
        except OSError:
            pass
        finally:
            try:
                reader.stdin.close()
            except OSError:
                pass

    threading.Thread(target=feed, daemon=True).start()
    try:
        for sha in shas:
            header = reader.stdout.readline().split()
            if len(header) != 3:
                raise Problem(f"object {sha} is not in {cwd}")
            body = reader.stdout.read(int(header[2]))
            reader.stdout.read(1)
            yield sha, body
    finally:
        reader.kill()
        reader.wait()
        reader.stdout.close()


def commit_entries(checkout: Path, commit: str) -> list[tuple[str, str, str]]:
    """(path, mode, object id) of each blob of a commit's tree; raises when it is not there."""
    listing = subprocess.run(["git", "-C", str(checkout), "ls-tree", "-r", "-z", commit], capture_output=True)
    if listing.returncode != 0:
        raise Problem(f"commit {commit} is not in {checkout}")
    entries = []
    for entry in listing.stdout.split(b"\0"):
        if not entry:
            continue
        meta, _, raw = entry.partition(b"\t")
        mode, kind, sha = meta.decode().split()
        if kind == "blob":
            entries.append((raw.decode("utf-8", "surrogateescape"), mode, sha))
    return entries


def has_commit(checkout: Path, commit: str) -> bool:
    return git_ok("cat-file", "-e", f"{commit}^{{commit}}", cwd=checkout)


def scan_ref(checkout: Path, commit: str, allow: Allow | None = None) -> Scan | None:
    """A commit's crates/ and everything in it a scanner would refuse; None when it is not here."""
    allow = allow or Allow()
    if not has_commit(checkout, commit):
        return None
    tree = empty_tree()
    adder = Adder(tree, allow)
    flagged = []
    entries = commit_entries(checkout, commit)
    for (path, mode, _), (_, body) in zip(entries, batch_bodies(checkout, [e[2] for e in entries])):
        if any(part.lower() == ".git" for part in path.split("/")):
            flagged.append(f"{path}: a path below .git")
        if mode == "120000":
            flagged.append(f"{path}: a symbolic link")
        elif (kind := flagged_kind(body, path.rsplit("/", 1)[-1])) and not allow.permits(path, kind):
            flagged.append(f"{path}: {kind}")
        if path.startswith("crates/"):
            adder.add(path.removeprefix("crates/"), body)
    return Scan(tree, flagged)


# --- turning a package into a vendor directory ---------------------------------------------------------


def settings_crates_url(cfg: Table) -> str:
    return os.environ.get("BUILD_CRATES_URL") or cfg.deps.crates_url or CRATES_URL


def fetch_crate(crate: Crate, template: str = CRATES_URL) -> bytes:
    # A version's build metadata has a + that a URL path would otherwise leave ambiguous.
    version = urllib.parse.quote(crate.version, safe="")
    data = fetch(template.format(name=crate.name, version=version))
    if sha256(data) != crate.checksum:
        raise Problem(f"refused {crate.file}: SHA-256 {sha256(data)}, Cargo.lock says {crate.checksum}")
    return data


def archive_files(data: bytes, prefix: str) -> dict[str, bytes]:
    """The regular files of a .crate below its `<name>-<version>/` directory, by relative path."""
    files = {}
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
        for member in tar:
            if not member.isfile():
                continue
            if not member.name.startswith(prefix):
                raise Problem(f"{prefix.rstrip('/')}.crate holds {member.name}, outside {prefix}")
            files[member.name.removeprefix(prefix)] = tar.extractfile(member).read()
    return files


def unsafe_member(name: str) -> bool:
    return name.startswith(("/", "\\")) or bool(re.match(r"^[A-Za-z]:", name)) or ".." in name.replace("\\", "/").split("/")


def stripped(name: str, strip: int) -> list[str]:
    return [p for p in posixpath.normpath(name).split("/") if p not in ("", ".")][strip:]


def safe_extract(tar: tarfile.TarFile, dest: Path, links: str = "refuse", strip: int = 0) -> None:
    """Writes an archive member by member: regular files and directories, below `dest` only.

    A path that is absolute or climbs out with `..` is refused. A link is refused (`links =
    "refuse"`), left out (`"skip"`), or made when it points inside `dest` (`"inside"`, not on
    Windows). `strip` drops that many leading path parts, as `tar --strip-components` does.
    """
    base = dest.resolve()
    for member in tar:
        if unsafe_member(member.name):
            raise Problem(f"archive member {member.name} is outside the archive")
        parts = stripped(member.name, strip)
        if not parts:
            continue
        target = base.joinpath(*parts)
        if member.isdir():
            target.mkdir(parents=True, exist_ok=True)
        elif member.isfile():
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(tar.extractfile(member).read())
            if member.mode & 0o111 and os.name != "nt":
                target.chmod(target.stat().st_mode | 0o111)
        elif member.issym() or member.islnk():
            if links == "refuse":
                raise Problem(f"archive member {member.name} is a link; links are refused")
            if links == "inside" and os.name != "nt":
                if member.issym():
                    resolved = (target.parent / member.linkname).resolve()
                else:
                    resolved = base.joinpath(*stripped(member.linkname, strip))
                if base not in resolved.parents and resolved != base:
                    raise Problem(f"archive link {member.name} points outside the archive")
                target.parent.mkdir(parents=True, exist_ok=True)
                if not target.exists() and not target.is_symlink():
                    if member.issym():
                        os.symlink(member.linkname, target)
                    else:
                        shutil.copyfile(resolved, target)
        else:
            raise Problem(f"archive member {member.name} is not a regular file or directory")


def safe_unzip(archive: zipfile.ZipFile, dest: Path, strip: int = 0) -> None:
    """The same rules for a zip: no absolute or `..` path, no symbolic link."""
    base = dest.resolve()
    for info in archive.infolist():
        if unsafe_member(info.filename):
            raise Problem(f"archive member {info.filename} is outside the archive")
        if (info.external_attr >> 16) & 0o170000 == 0o120000:
            raise Problem(f"archive member {info.filename} is a link; links are refused")
        parts = stripped(info.filename, strip)
        if not parts:
            continue
        target = base.joinpath(*parts)
        if info.is_dir():
            target.mkdir(parents=True, exist_ok=True)
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(archive.read(info))


def keeps(path: str) -> bool:
    """Whether a package's file (by path below the package) is something compiling can read."""
    *dirs, name = path.split("/")
    if dirs and dirs[0].lower() in DROP_TOP:
        return False
    if any(d.lower() in DROP_ANY or d.startswith(".") for d in dirs):
        return False
    if DEV_ANYWHERE.match(name) or (not dirs and DEV_TOP.match(name)):
        return False
    return not (name.startswith(".git") or name == ".cargo-ok")


def drop_missing_targets(text: str, present: set[str]) -> str:
    """The manifest without the [[test]], [[bench]] and [[example]] tables whose file is not here.

    Cargo refuses a manifest naming a target it cannot find, even in a dependency.
    """
    try:
        data = tomllib.loads(text)
    except tomllib.TOMLDecodeError:
        return text
    folders = {"test": "tests", "bench": "benches", "example": "examples"}
    index = dict.fromkeys(folders, -1)
    out, skipping = [], False
    for line in text.split("\n"):
        header = re.match(r"^\[\[(test|bench|example)\]\]\s*(#.*)?$", line)
        if header:
            kind = header[1]
            index[kind] += 1
            target = data.get(kind, [])[index[kind]]
            name = target.get("name", "")
            path = target.get("path") or None
            options = [path] if path else [f"{folders[kind]}/{name}.rs", f"{folders[kind]}/{name}/main.rs"]
            skipping = not any(Path(p).as_posix() in present for p in options if p)
        elif re.match(r"^\[", line):
            skipping = False
        if not skipping:
            out.append(line)
    return "\n".join(out)


def inline_bytes(kept: dict[str, bytes], files: dict[str, bytes], refused: set[str]) -> None:
    """Writes the bytes of each refused file into the `include_bytes!` that reads it.

    The array literal has the type `include_bytes!` gives, so a crate that embeds a DER file or a
    table compiles without the binary file being in the branch. A file that only tests read, or
    the big-endian copy of a table, is left out and its `include_bytes!` as it was.
    """

    def literal(base: str):
        def replace(found: re.Match[str]) -> str:
            target = posixpath.normpath(posixpath.join(base, found[2]))
            if target not in refused or BIG_ENDIAN.search(target) or TEST_ONLY.search(base + "/"):
                return found[0]
            body = files[target]
            array = "[" + ",".join(f"0x{b:02x}u8" if i == 0 else f"0x{b:02x}" for i, b in enumerate(body)) + "]"
            # `*include_bytes!(..)` is the array itself; plain `include_bytes!(..)` a reference to it.
            return array if found[1] else "&" + array

        return replace

    for path in [p for p in kept if p.endswith(".rs") and not TEST_ONLY.search(p)]:
        try:
            text = kept[path].decode("utf-8")
        except UnicodeDecodeError:
            continue
        changed = INCLUDE_BYTES.sub(literal(posixpath.dirname(path)), text)
        if changed != text:
            kept[path] = changed.encode("utf-8")


def hex_lines(data: bytes) -> bytes:
    digits = data.hex()
    return ("\n".join(digits[i : i + 128] for i in range(0, len(digits), 128)) + "\n").encode("ascii")


RING_OBJECT = re.compile(r"^pregenerated/.+\.o$")
RING_USE = """            let obj_srcs = asm_srcs
                .iter()
                .map(|src| obj_path(generated_dir, src.as_path()))
                .collect::<Vec<_>>();"""
RING_FN = """
/// The object file this crate carries as `<name>.o.hex`, written below `out_dir` for the linker.
fn object_from_text(generated_dir: &Path, out_dir: &Path, src: &Path) -> PathBuf {
    let text = fs::read_to_string(format!("{}.hex", obj_path(generated_dir, src).display())).unwrap();
    let digits: Vec<u8> = text.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
    let bytes: Vec<u8> = digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect();
    let path = obj_path(out_dir, src);
    fs::write(&path, bytes).unwrap();
    path
}
"""


def patch_ring(name: str, kept: dict[str, bytes], refused: dict[str, bytes]) -> None:
    """Keeps ring's prebuilt nasm objects (linked on x86 Windows) as hex, decoded by build.rs."""
    objects = sorted(p for p in refused if RING_OBJECT.match(p))
    if not objects:
        return
    build = kept["build.rs"].decode("utf-8")
    if build.count(RING_USE) != 1:
        raise Problem(f"{name}: build.rs no longer links its objects as build.py expects")
    for path in objects:
        kept[f"{path}.hex"] = hex_lines(refused.pop(path))
    fixed = build.replace(
        RING_USE, RING_USE.replace("obj_path(generated_dir, src.as_path())", "object_from_text(generated_dir, out_dir, src.as_path())")
    )
    kept["build.rs"] = (fixed.rstrip("\n") + "\n" + RING_FN).encode("utf-8")


WINDOWS_CRATE = re.compile(r"^windows_[a-z0-9_]+$")
WINDOWS_BUILD = """fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    println!(
        "cargo:rustc-link-search=native={}",
        std::path::Path::new(&dir).join("lib").display()
    );
}
"""
WINDOWS_BUILD_TEXT = """use std::{env, fs, path::Path};

/// Writes each `lib/<name>.hex` below `OUT_DIR` as `<name>` and points the linker there.
fn main() {
    let dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let out = env::var("OUT_DIR").unwrap();
    for entry in fs::read_dir(Path::new(&dir).join("lib")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let Some(lib) = name.strip_suffix(".hex") else { continue };
        println!("cargo:rerun-if-changed={}", path.display());
        let text = fs::read_to_string(&path).unwrap();
        let digits: Vec<u8> = text.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
        let bytes: Vec<u8> = digits
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        fs::write(Path::new(&out).join(lib), bytes).unwrap();
    }
    println!("cargo:rustc-link-search=native={out}");
}
"""


def patch_windows_libraries(name: str, kept: dict[str, bytes], refused: dict[str, bytes]) -> None:
    """Keeps the import libraries of a windows_<arch>_<abi> crate as hex, decoded into OUT_DIR by build.rs."""
    libs = sorted(p for p in refused if p.startswith("lib/"))
    if not libs:
        return
    if kept.get("build.rs", b"").decode("utf-8", "replace").replace("\r\n", "\n") != WINDOWS_BUILD:
        raise Problem(f"{name}: build.rs no longer links its import libraries as build.py expects")
    for path in libs:
        kept[f"{path}.hex"] = hex_lines(refused.pop(path))
    kept["build.rs"] = WINDOWS_BUILD_TEXT.encode("utf-8")


def package_patch(crate: str) -> Callable[[str, dict[str, bytes], dict[str, bytes]], None] | None:
    """The patch for a crate whose build links binary files that cannot be inlined, by crate name."""
    if WINDOWS_CRATE.match(crate):
        return patch_windows_libraries
    return {"ring": patch_ring}.get(crate)


def include_closure(files: dict[str, bytes]) -> dict[str, bytes]:
    """The files compiling can read: `keeps`, and whatever a kept source file includes by path."""
    kept = {p: b for p, b in files.items() if keeps(p)}
    pending = [p for p in kept if p.endswith(".rs")]
    while pending:
        path = pending.pop()
        try:
            text = kept[path].decode("utf-8")
        except UnicodeDecodeError:
            continue
        for found in INCLUDE.finditer(text):
            target = posixpath.normpath(posixpath.join(posixpath.dirname(path), found[1] or found[2]))
            if target in files and target not in kept:
                kept[target] = files[target]
                if target.endswith(".rs"):
                    pending.append(target)
    return kept


def write_package(target: Path, everything: dict[str, bytes], checksum: str | None) -> None:
    """Writes a package directory with its .cargo-checksum.json, leaving out what a scanner refuses.

    The files map lists what was written, so cargo checks each file it reads against it.
    """
    files = include_closure(everything)
    seen: dict[str, str] = {}
    for path in files:
        if seen.setdefault(path.lower(), path) != path:
            raise Problem(f"{target.name} has {path} and {seen[path.lower()]}, one file on Windows")
        if any(RESERVED.match(part) for part in path.split("/")):
            raise Problem(f"{target.name}/{path} is a name Windows cannot create")
    kept, refused = {}, {}
    for path, body in files.items():
        name = path.rsplit("/", 1)[-1]
        if flagged_kind(body, name) == "private key in text":
            body = PEM_MARKER.sub(PEM_REPLACEMENT, body)
        (refused if flagged_kind(body, name) else kept)[path] = body
    if refused:
        inline_bytes(kept, files, set(refused))
        patch = package_patch(target.name.rsplit("-", 1)[0])
        if patch:
            patch(target.name, kept, refused)
    if "Cargo.toml" in kept:
        text = kept["Cargo.toml"].decode("utf-8")
        fixed = drop_missing_targets(text, set(kept))
        if fixed != text:
            kept["Cargo.toml"] = fixed.encode("utf-8")
    partial = target.with_name(target.name + ".partial")
    shutil.rmtree(partial, ignore_errors=True)
    for path, body in kept.items():
        (partial / path).parent.mkdir(parents=True, exist_ok=True)
        (partial / path).write_bytes(body)
    listed = {path: sha256(body) for path, body in sorted(kept.items())}
    (partial / CHECKSUM).write_bytes(json.dumps({"files": listed, "package": checksum}, separators=(",", ":")).encode())
    shutil.rmtree(target, ignore_errors=True)
    partial.rename(target)


def materialize_crate(crate: Crate, crates: Path, template: str = CRATES_URL) -> None:
    write_package(crates / crate.dir, archive_files(fetch_crate(crate, template), f"{crate.dir}/"), crate.checksum)


def export_commit(repo: Repo, into: Path) -> Path:
    """The tree of exactly this commit below `into`, with no history: git checks the hash it fetched."""
    work = into / "fetch"
    work.mkdir()
    run_git("init", "-q", cwd=work)
    run_git("fetch", "-q", "--depth", "1", repo.url, repo.commit, cwd=work)
    if run_git("rev-parse", "FETCH_HEAD", cwd=work) != repo.commit:
        raise Problem(f"{repo.url} did not give commit {repo.commit}")
    if git_ok("cat-file", "-e", "FETCH_HEAD:.gitmodules", cwd=work):
        raise Problem(f"{repo.name} has submodules, which a source tree would leave out")
    tree = into / "tree"
    tree.mkdir()
    made = subprocess.Popen(
        ["git", "-C", str(work), "-c", "core.autocrlf=false", "-c", "core.eol=lf", "archive", "--format=tar", "FETCH_HEAD"],
        stdout=subprocess.PIPE,
    )
    try:
        with tarfile.open(fileobj=made.stdout, mode="r|") as tar:
            safe_extract(tar, tree)
    finally:
        # git pads the archive to a whole record after its end marker; read it, or the pipe stays
        # full and git never exits (a Windows pipe holds 4 KiB).
        made.stdout.read()
        code = made.wait()
    if code:
        raise Problem(f"git archive of {repo.name} at {repo.commit} failed")
    shutil.rmtree(work, ignore_errors=True)
    return tree


def toml_value(value: object) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return str(value)
    if isinstance(value, str):
        # What JSON escapes is a TOML basic string too, as long as it writes no surrogate pairs.
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, list):
        return "[" + ", ".join(toml_value(item) for item in value) + "]"
    if isinstance(value, dict):
        pairs = (f"{toml_key(key)} = {toml_value(item)}" for key, item in value.items())
        return "{ " + ", ".join(pairs) + " }"
    raise Problem(f"cannot write {value!r} back as TOML")


def toml_key(key: str) -> str:
    return key if re.fullmatch(r"[A-Za-z0-9_-]+", key) else json.dumps(key, ensure_ascii=False)


def toml_table(table: dict, path: list[str], lines: list[str]) -> None:
    """Writes one table, its plain values before its subtables as TOML requires."""
    subtables = {key: item for key, item in table.items() if isinstance(item, dict)}
    if path:
        lines.append("[" + ".".join(toml_key(part) for part in path) + "]")
    for key, item in table.items():
        if key not in subtables:
            lines.append(f"{toml_key(key)} = {toml_value(item)}")
    for key, item in subtables.items():
        toml_table(item, [*path, key], lines)


def inherit_dependency(name: str, own: dict, workspace: dict, manifest: str) -> dict:
    """A `{ workspace = true }` dependency, with cargo's rules for what the member may add."""
    shared = workspace.get("dependencies", {}).get(name)
    if shared is None:
        raise Problem(f"{manifest} inherits dependency {name}, which its workspace does not list")
    merged = {"version": shared} if isinstance(shared, str) else dict(shared)
    features = list(merged.get("features", [])) + list(own.get("features", []))
    if features:
        merged["features"] = features
    if "optional" in own:
        merged["optional"] = own["optional"]
    for spelling in ("default-features", "default_features"):
        if spelling in own and "default-features" not in merged and "default_features" not in merged:
            merged[spelling] = own[spelling]
    return merged


def normalized_manifest(manifest: dict, root: dict, member_dir: Path, root_dir: Path, label: str) -> dict:
    """A workspace member's manifest with nothing left to inherit, as `cargo package` would write it.

    A directory source reads each package on its own, with no workspace above it. A `path`
    dependency stays: within a directory source it means this same source, wherever the path points.
    """
    workspace = root.get("workspace", {})
    shared = workspace.get("package", {})
    result = {k: v for k, v in manifest.items() if k not in ("workspace", "patch", "replace", "profile")}
    package = dict(result["package"])
    package.pop("workspace", None)
    for key, value in list(package.items()):
        if isinstance(value, dict) and value.get("workspace") is True:
            if key not in shared:
                raise Problem(f"{label} inherits package.{key}, which its workspace does not set")
            value = shared[key]
            if key in ("readme", "license-file") and isinstance(value, str):
                value = Path(os.path.relpath(root_dir / value, member_dir)).as_posix()
            package[key] = value
    result["package"] = package
    lints = result.get("lints")
    if isinstance(lints, dict) and lints.get("workspace") is True:
        result.pop("lints")
        if "lints" in workspace:
            result["lints"] = workspace["lints"]

    def inherit_tables(table: dict) -> None:
        for kind in ("dependencies", "dev-dependencies", "dev_dependencies", "build-dependencies", "build_dependencies"):
            if kind in table:
                table[kind] = {
                    name: (
                        inherit_dependency(name, value, workspace, label)
                        if isinstance(value, dict) and value.get("workspace") is True
                        else value
                    )
                    for name, value in table[kind].items()
                }

    inherit_tables(result)
    for target in result.get("target", {}).values():
        inherit_tables(target)
    return result


def materialize_repo(repo: Repo, wanted: list[tuple[str, str]], crates: Path) -> None:
    """Puts each package Cargo.lock takes from a repository in a directory of its own in crates/."""
    with tempfile.TemporaryDirectory(prefix="build-git-") as scratch:
        tree = export_commit(repo, Path(scratch))
        manifests: dict[Path, dict] = {}
        for path in tree.rglob("Cargo.toml"):
            if any(part.startswith(".") for part in path.relative_to(tree).parts[:-1]):
                continue
            try:
                manifests[path.parent] = tomllib.loads(path.read_text(encoding="utf-8"))
            except (tomllib.TOMLDecodeError, UnicodeDecodeError):
                # A fixture, or anything else cargo would also skip when it searches a git source.
                continue

        def workspace_root(member: Path) -> Path | None:
            for directory in (member, *member.parents):
                if directory == tree.parent:
                    return None
                if "workspace" in manifests.get(directory, {}):
                    return directory
            return None

        packages = {d for d, m in manifests.items() if "package" in m}
        for name, version in wanted:
            found = []
            for directory in packages:
                manifest = manifests[directory]
                package = manifest["package"]
                if package.get("name") != name:
                    continue
                root_dir = workspace_root(directory)
                root = manifests.get(root_dir, {}) if root_dir else {}
                shown = package.get("version")
                if isinstance(shown, dict):
                    shown = root.get("workspace", {}).get("package", {}).get("version")
                if shown == version:
                    found.append((directory, manifest, root_dir or directory, root))
            if len(found) != 1:
                raise Problem(f"{repo.name} at {repo.commit} has {len(found)} packages {name} {version}")
            directory, manifest, root_dir, root = found[0]
            label = f"{repo.name}/{directory.relative_to(tree).as_posix()}"
            written = normalized_manifest(manifest, root, directory, root_dir, label)
            nested = [d for d in packages if d != directory and directory in d.parents]
            files = {}
            for path in directory.rglob("*"):
                if path.is_file() and not any(n in path.parents for n in nested):
                    files[path.relative_to(directory).as_posix()] = path.read_bytes()
            lines = [f"# {label} at {repo.commit}, with what it inherits written out."]
            toml_table(written, [], lines)
            files["Cargo.toml"] = ("\n".join(lines) + "\n").encode("utf-8")
            # Git packages have no registry checksum.
            write_package(crates / f"{name}-{version}", files, None)


def untrimmed(crates: Path, tree: Tree, name: str) -> bool:
    """Whether a package on disk holds a file the current rules leave out and no kept source includes."""
    present = [p for p in tree.dirs.get(name, {}) if p != CHECKSUM]
    if all(keeps(p) for p in present):
        return False
    files = {p: (crates / name / p).read_bytes() for p in present}
    return set(include_closure(files)) != set(files)


def sync_crates(deps_dir: Path, lock: Lock, template: str, jobs: int) -> list[str]:
    """Makes deps_dir/crates cover the lock and hold nothing else; returns a line per change."""
    crates = deps_dir / "crates"
    wanted = wanted_packages(lock)
    crates.mkdir(parents=True, exist_ok=True)
    for entry in sorted(crates.iterdir()):
        if entry.name.endswith(".partial") and entry.is_dir():
            shutil.rmtree(entry)
        elif not entry.is_dir() or (entry.name not in wanted and not (entry / CHECKSUM).is_file()):
            raise Problem(f"{entry} was not made by this tool: is {crates} from a deps checkout?")
    tree = read_dir(crates)
    changes = []
    for name in sorted(set(tree.dirs) - set(wanted)):
        shutil.rmtree(crates / name)
        changes.append(f"removed {name}")
    stale = {n for n, (c, r) in wanted.items() if package_problems(tree, n, c, r) or untrimmed(crates, tree, n)}
    crate_jobs = [c for c in lock.crates if c.dir in stale]
    repo_jobs = [(r, p) for r, p in lock.repos.items() if any(f"{n}-{v}" in stale for n, v in p)]
    with ThreadPoolExecutor(max_workers=max(1, jobs)) as pool:
        for future in [pool.submit(materialize_crate, c, crates, template) for c in crate_jobs] + [
            pool.submit(materialize_repo, r, p, crates) for r, p in repo_jobs
        ]:
            future.result()
    return changes + [f"{'replaced' if n in tree.dirs else 'added'} {n}" for n in sorted(stale)]


# --- deps: the commands -------------------------------------------------------------------------------

SCHEMA["deps"] = Key(
    "table",
    None,
    "The deps branch: every crate Cargo.lock names, vendored as text, so a build needs no network.",
    {
        "branch": Key("str", "deps", "The orphan branch that holds the vendored files."),
        "source_name": Key("str", "vendored", "The name of the cargo source the written cargo config replaces crates.io with."),
        "crates_url": Key("str", "", "A mirror of crates.io's .crate files, with {name} and {version}; the environment variable BUILD_CRATES_URL wins."),
        "cargo_lock": Key("str", "Cargo.lock", "The lock the deps branch must cover, relative to the repository."),
        "checkout": Key("str", "", "Where the deps branch is checked out; the environment variable BUILD_DEPS wins. Empty: a detached worktree beside the repository, named REPO-deps."),
        "allow": Key("strs", [], "Files a scan lets through, as GLOB:KIND (for example cdn/www.gstatic.com/flutter-canvaskit/*:wasm)."),
        "jobs": Key("int", 8, "Packages fetched at once."),
        "pin": Key(
            "table",
            None,
            "Where main records the deps commit it was built with: a row of an SDK lock, `GROUP NAME COMMIT ...`.",
            {
                "lock": Key("str", "", "The lock file, relative to the repository; empty: main records no pin."),
                "item": Key("str", "", "The row, `GROUP NAME` (for example core deps)."),
            },
        ),
        "repin": Key(
            "tables",
            None,
            "A file on the deps branch that names the deps commit; refresh rewrites it to the new commit.",
            {
                "file": Key("str", REQUIRED, "The file, relative to the deps branch."),
                "pattern": Key("str", REQUIRED, "A regular expression (multi-line) with one group: the text to replace by the commit."),
            },
        ),
    },
)


def short(commit: str) -> str:
    return commit[:8]


def find_deps(ctx: Ctx, given: Path | None = None) -> Path:
    """The deps checkout: --deps, $BUILD_DEPS, `[deps] checkout`, else a worktree beside the repository.

    The worktree is added detached at origin/deps (or deps) when missing, and moved to that
    branch's tip when it is detached; one on a branch is its owner's and is left alone.
    """
    if given is not None:
        return given.resolve()
    if os.environ.get("BUILD_DEPS"):
        return Path(os.environ["BUILD_DEPS"]).resolve()
    cfg = ctx.cfg.deps
    if cfg.checkout:
        path = Path(cfg.checkout)
        return path if path.is_absolute() else (ctx.root / path).resolve()
    ref = next((r for r in (f"origin/{cfg.branch}", cfg.branch) if git_ok("rev-parse", "--verify", "-q", r, cwd=ctx.root)), None)
    if ref is None:
        raise Problem(f"no deps checkout: pass --deps, set BUILD_DEPS or [deps] checkout, or fetch the branch (git fetch origin {cfg.branch})")
    path = ctx.root.with_name(ctx.root.name + "-deps")
    if path.is_dir():
        if not git_ok("symbolic-ref", "-q", "HEAD", cwd=path):
            run_git("checkout", "-q", "--detach", ref, cwd=path)
    else:
        run_git("worktree", "add", "--detach", str(path), ref, cwd=ctx.root)
    return path


class DirView:
    """A deps checkout on disk, for the checks that ask whether a path is there."""

    def __init__(self, path: Path) -> None:
        self.path = path
        self.label = str(path)

    def exists(self, rel: str) -> bool:
        return (self.path / rel).exists()

    def read(self, rel: str) -> bytes | None:
        target = self.path / rel
        return target.read_bytes() if target.is_file() else None


class RefView:
    """A commit of the repository, for the same checks."""

    def __init__(self, root: Path, commit: str) -> None:
        self.root, self.commit = root, commit
        self.label = f"deps at {commit[:12]}"

    def exists(self, rel: str) -> bool:
        return git_ok("cat-file", "-e", f"{self.commit}:{rel}", cwd=self.root)

    def read(self, rel: str) -> bytes | None:
        done = subprocess.run(["git", "-C", str(self.root), "show", f"{self.commit}:{rel}"], capture_output=True)
        return done.stdout if done.returncode == 0 else None


# Extra coverage checks and sync steps, added by the sections below (pub, CDN).
DEPS_CHECKS: list[Callable[[Ctx, Any, Any], tuple[list[str], list[str]]]] = []


def manifest_problems(text: bytes | None, lock: Lock) -> tuple[list[str], list[str]]:
    """(problems, notes) about manifest.json: absent is a note, wrong is a problem."""
    if text is None:
        return [], [f"no {MANIFEST} on the deps branch; `deps refresh` writes one"]
    try:
        data = json.loads(text)
    except ValueError:
        return [f"{MANIFEST}: not JSON"], []
    found = []
    if data.get("format") != MANIFEST_FORMAT:
        found.append(f"{MANIFEST}: format {data.get('format')}, this tool reads {MANIFEST_FORMAT}")
    if data.get("crates") != len(lock.crates) or data.get("gitSources") != len(lock.repos):
        found.append(
            f"{MANIFEST}: says {data.get('crates')} crates and {data.get('gitSources')} git sources, "
            f"Cargo.lock names {len(lock.crates)} and {len(lock.repos)}"
        )
    return found, []


def write_manifest(deps_dir: Path, ctx: Ctx, lock: Lock, lock_path: Path, extra: dict) -> None:
    """manifest.json at the root of the deps branch. `sourceCommit` is where main stood when the
    contents last changed, so a commit that changes nothing else leaves the file as it was."""
    commit = run_git("rev-parse", "HEAD", cwd=ctx.root) if git_ok("rev-parse", "HEAD", cwd=ctx.root) else ""
    data = {
        "format": MANIFEST_FORMAT,
        "sourceCommit": commit,
        "cargoLockSha256": sha256(lock_path.read_bytes()),
        "crates": len(lock.crates),
        "gitSources": len(lock.repos),
        **extra,
    }
    path = deps_dir / MANIFEST
    try:
        old = json.loads(path.read_bytes())
    except (OSError, ValueError):
        old = None
    if isinstance(old, dict) and {**old, "sourceCommit": ""} == {**data, "sourceCommit": ""}:
        return
    path.write_bytes((json.dumps(data, indent=2, sort_keys=True) + "\n").encode("utf-8"))


def deps_sync_all(ctx: Ctx, deps_dir: Path, lock: Lock, lock_path: Path, args: argparse.Namespace | None = None) -> list[str]:
    """Crates, then the pub and CDN files when configured, then manifest.json; a line per change."""
    cfg = ctx.cfg.deps
    changes = sync_crates(deps_dir, lock, settings_crates_url(ctx.cfg), cfg.jobs)
    extra: dict = {}
    for sync in DEPS_SYNCS:
        more, info = sync(ctx, deps_dir, args)
        changes += more
        extra.update(info)
    write_manifest(deps_dir, ctx, lock, lock_path, extra)
    return changes


DEPS_SYNCS: list[Callable[[Ctx, Path, Any], tuple[list[str], dict]]] = []


def config_text(ctx: Ctx, lock: Lock, crates: Path) -> str:
    """The config `cargo vendor` prints: every source replaced by the one vendor directory."""
    name = ctx.cfg.deps.source_name
    lines = [
        f"{CONFIG_MARK} Every source Cargo.lock names, from a directory.",
        "[net]",
        "offline = true",
        "",
        "[source.crates-io]",
        f'replace-with = "{name}"',
        "",
        f"[source.{name}]",
        f"directory = {toml_value(crates.as_posix())}",
    ]
    for source in sorted(lock.sources):
        parts = urllib.parse.urlsplit(source.removeprefix("git+"))
        entry = [f"git = {toml_value(f'{parts.scheme}://{parts.netloc}{parts.path}')}"]
        entry += [f"{key} = {toml_value(value)}" for key, value in urllib.parse.parse_qsl(parts.query)]
        lines += ["", f"[source.{toml_value(source.partition('#')[0])}]", *entry, f'replace-with = "{name}"']
    return "\n".join(lines) + "\n"


def use_crates(
    ctx: Ctx, deps_dir: Path, lock: Lock, lock_path: Path, out: Path | None = None, repo_config: bool = False, full: bool = False
) -> dict:
    """Writes a cargo home whose config builds from deps_dir/crates, only when it covers the lock.

    The check reads each package's metadata; `full` also re-hashes every file.
    """
    crates = (deps_dir / "crates").resolve()
    found = problems(lock, read_dir(crates, full, Allow(ctx.cfg.deps.allow)), full)
    if found:
        raise Problem("; ".join(found[:5]) + f" ({len(found)} in all); run `deps sync` first")
    root = ctx.root.resolve()
    out = (out or Path(tempfile.gettempdir()) / f"build-crates-{sha256(lock_path.read_bytes())[:12]}").resolve()
    if out == root or root in out.parents:
        raise Problem(f"{out} is inside the repository; --out names a directory outside it")
    home = out / "cargo-home"
    config = home / "config.toml"
    if config.is_file() and not config.read_text(encoding="utf-8").startswith(CONFIG_MARK):
        raise Problem(f"{config} is not one this command wrote; it was left alone")
    home.mkdir(parents=True, exist_ok=True)
    text = config_text(ctx, lock, crates)
    config.write_bytes(text.encode("utf-8"))
    info = {"cargo_home": str(home), "config": str(config), "crates": len(lock.crates), "git_sources": len(lock.repos), "repo_config": None}
    if repo_config:
        target = root / ".cargo" / "config.toml"
        if target.exists() and not target.read_text(encoding="utf-8").startswith(CONFIG_MARK):
            raise Problem(f"{target} is not one this command wrote; it was left alone")
        target.parent.mkdir(exist_ok=True)
        target.write_bytes(text.encode("utf-8"))
        info["repo_config"] = str(target)
    return info


def ref_problems(ctx: Ctx, lock: Lock, commit: str) -> tuple[list[str], list[str]]:
    """What keeps a commit of the repository from being a deps branch covering the lock."""
    scan = scan_ref(ctx.root, commit, Allow(ctx.cfg.deps.allow))
    if scan is None:
        return [f"deps at {commit[:12]} is not here: git fetch --depth 1 origin {commit}"], []
    label = f"deps at {commit[:12]}"
    found = [f"{label}: {line}" for line in problems(lock, scan.tree)]
    found += [f"{label}: {line}; the branch holds no such file" for line in scan.flagged]
    view = RefView(ctx.root, commit)
    more, notes = manifest_problems(view.read(MANIFEST), lock)
    found += [f"{label}: {line}" for line in more]
    for check in DEPS_CHECKS:
        extra, extra_notes = check(ctx, view, lock)
        found += [f"{label}: {line}" for line in extra]
        notes += extra_notes
    return found, notes


def dir_problems(ctx: Ctx, deps: Path, lock: Lock, deep: bool = True) -> tuple[list[str], list[str]]:
    """What keeps a deps checkout from covering the lock: (problems, notes)."""
    found = problems(lock, read_dir(deps / "crates", deep, Allow(ctx.cfg.deps.allow)), deep)
    view = DirView(deps)
    more, notes = manifest_problems(view.read(MANIFEST), lock)
    found += more
    for check in DEPS_CHECKS:
        extra, extra_notes = check(ctx, view, lock)
        found += extra
        notes += extra_notes
    return found, notes


def lock_path_of(ctx: Ctx, args: argparse.Namespace) -> Path:
    ctx.pubspec_lock = getattr(args, "pubspec_lock", None)
    ctx.quick = bool(getattr(args, "quick", False))
    return args.cargo_lock.resolve() if args.cargo_lock else ctx.root / ctx.cfg.deps.cargo_lock


def deps_sync(ctx: Ctx, args: argparse.Namespace) -> int:
    path = lock_path_of(ctx, args)
    lock = read_lock(path)
    deps = find_deps(ctx, args.deps)
    changes = deps_sync_all(ctx, deps, lock, path, args)
    for change in changes:
        print(change)
    print(f"ok: {len(lock.crates)} crates, {len(lock.repos)} git sources, {len(changes)} changed")
    return 0


def deps_check(ctx: Ctx, args: argparse.Namespace) -> int:
    path = lock_path_of(ctx, args)
    lock = read_lock(path)
    if args.ref:
        if args.quick:
            raise Usage("--quick reads a checkout's metadata; --ref reads every file of a commit")
        found, notes = ref_problems(ctx, lock, args.ref)
    else:
        found, notes = dir_problems(ctx, find_deps(ctx, args.deps), lock, not args.quick)
    for note in notes:
        print(f"note: {note}")
    if found:
        raise Problem("\n".join(found))
    print(f"ok: {len(lock.crates)} crates, {len(lock.repos)} git sources cover {path.name}")
    return 0


def deps_use(ctx: Ctx, args: argparse.Namespace) -> int:
    path = lock_path_of(ctx, args)
    lock = read_lock(path)
    deps = find_deps(ctx, args.deps)
    info = use_crates(ctx, deps, lock, path, args.out, args.repo_config, args.full)
    if args.json:
        print(json.dumps(info, indent=2))
        return 0
    if info["repo_config"]:
        print(f"wrote {info['repo_config']}; delete it to build from the network again")
    home = info["cargo_home"]
    print(f"offline build ready: {info['crates']} crates, {info['git_sources']} git repositories: {home}")
    print(f"  sh:          export CARGO_HOME='{home}' CARGO_NET_OFFLINE=true")
    print(f"  PowerShell:  $env:CARGO_HOME = '{home}'; $env:CARGO_NET_OFFLINE = 'true'")
    print(f"  or one command, keeping your CARGO_HOME:  cargo --config '{info['config']}' build --locked")
    return 0


def deps_path(ctx: Ctx, args: argparse.Namespace) -> int:
    print(find_deps(ctx, args.deps))
    return 0


def pin_row(text: str, item: str) -> re.Match[str]:
    group, _, name = item.partition(" ")
    found = re.search(rf"^({re.escape(group)}[ \t]+{re.escape(name)}[ \t]+)(\S+)([ \t])", text, re.MULTILINE)
    if not found:
        raise Problem(f"no `{item}` row to pin the deps commit in")
    return found


def repin_match(found: re.Match[str], commit: str) -> str:
    """The match with its one group replaced by the commit (the whole match without a group)."""
    if not found.re.groups:
        return commit
    start = found.start()
    return found.group(0)[: found.start(1) - start] + commit + found.group(0)[found.end(1) - start :]


def deps_refresh(ctx: Ctx, args: argparse.Namespace) -> int:
    """Sync, then the commits that keep a pin true: deps' files, its repinned files, then main's lock.

    The lock pins the deps commit, and the files on deps naming that commit come after it, since a
    commit cannot hold its own hash.
    """
    cfg = ctx.cfg.deps
    lock_path = lock_path_of(ctx, args)
    lock = read_lock(lock_path)
    deps = find_deps(ctx, args.deps)
    if not git_ok("symbolic-ref", "-q", "HEAD", cwd=deps):
        raise Problem(f"{deps} is not on a branch; `deps init` makes one, or git -C {deps} checkout -B {cfg.branch} origin/{cfg.branch}")
    generated = ["crates", "pub", "cdn", MANIFEST]
    dirty = run_git("status", "--porcelain", "--", ".", *[f":!{g}" for g in generated], cwd=deps)
    if dirty:
        raise Problem(f"{deps} has changes outside {', '.join(generated)}: {dirty.splitlines()[0]}")
    changes = deps_sync_all(ctx, deps, lock, lock_path, args)
    for change in changes:
        print(change)
    source = run_git("rev-parse", "--short=8", "HEAD", cwd=ctx.root)
    run_git("add", "-A", "--", *[g for g in generated if (deps / g).exists()], cwd=deps)
    committed = bool(run_git("diff", "--cached", "--name-only", cwd=deps))
    if committed:
        body = "\n".join(changes) if len(changes) <= 40 else f"{len(changes)} files changed."
        run_git("commit", "-q", "-m", f"chore: crates/ for main at {source}", "-m", body or "no package changed", cwd=deps)
    pin_lock = ctx.root / cfg.pin.lock if cfg.pin.lock else None
    text = pinned = None
    if pin_lock:
        text = pin_lock.read_bytes().decode("utf-8")
        pinned = pin_row(text, cfg.pin.item)[2]
        scan = scan_ref(ctx.root, pinned, Allow(cfg.allow)) if has_commit(ctx.root, pinned) else None
        if not committed and scan is not None and not problems(lock, scan.tree) and not scan.flagged:
            print(f"current: deps {pinned} covers {lock_path.name}")
            return 0
    pin = run_git("rev-parse", "HEAD", cwd=deps)
    if not pin_lock and not committed:
        found, _ = ref_problems(ctx, lock, pin) if has_commit(ctx.root, pin) else ([f"deps {pin} is not in this repository"], [])
        if not found:
            print(f"current: deps {pin} covers {lock_path.name}")
            return 0
    repinned = []
    for rule in cfg.repin:
        path = deps / rule.file
        data = path.read_bytes().decode("utf-8")
        rewritten, count = re.subn(rule.pattern, lambda m: repin_match(m, pin), data, flags=re.MULTILINE)
        if not count:
            raise Problem(f"{rule.file} on the deps branch has no match for {rule.pattern}")
        path.write_bytes(rewritten.encode("utf-8"))
        repinned.append(rule.file)
    if repinned:
        run_git("add", "--", *repinned, cwd=deps)
        if run_git("diff", "--cached", "--name-only", cwd=deps):
            run_git("commit", "-q", "-m", f"chore: {', '.join(repinned)} pin deps at {short(pin)}", cwd=deps)
    # The pin is the commit that holds the vendored files; the repin commit above follows it.
    if not has_commit(ctx.root, pin):
        run_git("fetch", "-q", str(deps), "HEAD", cwd=ctx.root)
    if pin_lock:
        assert text is not None and pinned is not None
        row = pin_row(text, cfg.pin.item)
        pin_lock.write_bytes((text[: row.start(2)] + pin + text[row.end(2) :]).encode("utf-8"))
        run_git("commit", "-q", "-m", f"chore: {Path(cfg.pin.lock).name} pins deps at {short(pin)}", "--", cfg.pin.lock, cwd=ctx.root)
    found, _ = ref_problems(ctx, lock, pin)
    if found:
        raise Problem(f"deps {pin} does not cover {lock_path.name}: {found[0]}")
    print(f"pinned deps {pin}; push {deps}'s branch to {cfg.branch} first, not rebased, then this one")
    return 0


README_TEXT = """# deps

Everything a build of the main branch needs from the network, as text: `crates/` is a cargo vendor
directory with one `NAME-VERSION/` directory for each package Cargo.lock names, and `manifest.json`
says which commit and lock it was made from. Nothing here is edited by hand: `tools/build.py deps
refresh` on main rewrites it, and `tools/build.py deps check` holds it to Cargo.lock.

No archive, executable, key or opaque binary is kept: a file a build reads that is one is written
as text.
"""


def deps_init(ctx: Ctx, args: argparse.Namespace) -> int:
    cfg = ctx.cfg.deps
    path = (args.path or ctx.root.with_name(ctx.root.name + "-deps")).resolve()
    if path.exists() and any(path.iterdir()):
        raise Problem(f"{path} exists and is not empty")
    if any(git_ok("rev-parse", "--verify", "-q", f"{prefix}{cfg.branch}", cwd=ctx.root) for prefix in ("", "origin/")):
        raise Problem(f"the branch {cfg.branch} exists; `deps path` checks it out")
    empty = subprocess.run(["git", "-C", str(ctx.root), "hash-object", "-t", "tree", "-w", "--stdin"], input=b"", capture_output=True)
    if empty.returncode:
        raise Problem(f"git hash-object: {empty.stderr.decode(errors='replace').strip()}")
    first = run_git("commit-tree", empty.stdout.decode().strip(), "-m", "chore: start the deps branch", cwd=ctx.root)
    run_git("branch", cfg.branch, first, cwd=ctx.root)
    run_git("worktree", "add", str(path), cfg.branch, cwd=ctx.root)
    (path / "README.md").write_bytes(README_TEXT.encode("utf-8"))
    (path / ".gitattributes").write_bytes(b"* -text\n")
    (path / "crates").mkdir(exist_ok=True)
    run_git("add", "--", "README.md", ".gitattributes", cwd=path)
    run_git("commit", "-q", "-m", "chore: README and attributes for the deps branch", cwd=path)
    print(f"made branch {cfg.branch} in {path}")
    return 0


# --- scan: refuse what an archive scanner would -----------------------------------------------------------

SCAN_LIST = 25


def report_scan(found: dict[str, list[tuple[str, int, str]]], scanned: int, notes: list[tuple[str, int, str]] | None = None) -> int:
    flagged = 0
    if notes:
        print(f"NOTE text, not UTF-8: {len(notes)}")
        for sha, size, path in notes[:SCAN_LIST]:
            print(f"  {sha[:10]} {size:>10}  {path}")
        if len(notes) > SCAN_LIST:
            print(f"  ... {len(notes) - SCAN_LIST} more")
    for kind in sorted(found):
        items = found[kind]
        flagged += len(items)
        print(f"FLAG {kind}: {len(items)}")
        for sha, size, path in items[:SCAN_LIST]:
            print(f"  {sha[:10]} {size:>10}  {path}")
        if len(items) > SCAN_LIST:
            print(f"  ... {len(items) - SCAN_LIST} more")
    print(f"{'FAIL' if flagged else 'PASS'}: {flagged} flagged, {scanned} paths scanned")
    return 1 if flagged else 0


def scan_command(ctx: Ctx, args: argparse.Namespace) -> int:
    """`blob_scan.py`'s contract: what is flagged, then `PASS|FAIL: N flagged, M paths scanned`."""
    allow = Allow(ctx.cfg.deps.allow)
    found: dict[str, list[tuple[str, int, str]]] = collections.defaultdict(list)
    notes: list[tuple[str, int, str]] = []
    chosen = sum(bool(x) for x in (args.refs, args.all_refs, args.tree))
    if chosen > 1:
        raise Usage("name refs, --all-refs or --tree, not several")
    if args.tree:
        scanned = 0
        base = args.tree.resolve()
        for current, dirs, names in os.walk(base):
            dirs[:] = sorted(d for d in dirs if not (d == ".git" and Path(current) == base))
            for name in sorted(names):
                file = Path(current) / name
                rel = file.relative_to(base).as_posix()
                scanned += 1
                if file.is_symlink():
                    found["a symbolic link"].append(("-", 0, rel))
                    continue
                body = file.read_bytes()
                if any(part.lower() == ".git" for part in rel.split("/")):
                    found["a path below .git"].append((sha256(body), len(body), rel))
                kind = flagged_kind(body, name)
                if kind and not allow.permits(rel, kind):
                    found[kind].append((sha256(body), len(body), rel))
                elif not kind and text_note(body):
                    notes.append((sha256(body), len(body), rel))
        return report_scan(found, scanned, notes)
    if args.all_refs:
        names: dict[str, set[str]] = collections.defaultdict(set)
        listing = run_git("rev-list", "--objects", "--all", cwd=ctx.root, binary=True).decode("utf-8", "replace")
        for line in listing.splitlines():
            sha, _, path = line.partition(" ")
            if path:
                names[sha].add(path)
        shas = list(names)
        kinds = {}
        if shas:
            batch = subprocess.run(
                ["git", "-C", str(ctx.root), "cat-file", "--batch-check=%(objectname) %(objecttype)"],
                input=("\n".join(shas) + "\n").encode(),
                capture_output=True,
            )
            kinds = {row.split()[0]: row.split()[1] for row in batch.stdout.decode().splitlines() if row.strip()}
        blobs = [s for s in shas if kinds.get(s) == "blob"]
        for sha, body in batch_bodies(ctx.root, blobs):
            for path in sorted(names[sha]):
                if any(part.lower() == ".git" for part in path.split("/")):
                    found["a path below .git"].append((sha, len(body), path))
                kind = flagged_kind(body, path.rsplit("/", 1)[-1])
                if kind and not allow.permits(path, kind):
                    found[kind].append((sha, len(body), path))
                elif not kind and text_note(body):
                    notes.append((sha, len(body), path))
        return report_scan(found, len(names), notes)
    refs = args.refs or ["HEAD"]
    seen: set[tuple[str, str]] = set()
    scanned = 0
    for ref in refs:
        entries = commit_entries(ctx.root, ref)
        wanted = [e for e in entries if (e[2], e[0]) not in seen]
        seen.update((e[2], e[0]) for e in wanted)
        scanned += len(wanted)
        by_sha: dict[str, list[tuple[str, str]]] = collections.defaultdict(list)
        for path, mode, sha in wanted:
            by_sha[sha].append((path, mode))
        for sha, body in batch_bodies(ctx.root, list(by_sha)):
            for path, mode in by_sha[sha]:
                if any(part.lower() == ".git" for part in path.split("/")):
                    found["a path below .git"].append((sha, len(body), path))
                if mode == "120000":
                    found["a symbolic link"].append((sha, len(body), path))
                elif (kind := flagged_kind(body, path.rsplit("/", 1)[-1])) and not allow.permits(path, kind):
                    found[kind].append((sha, len(body), path))
                elif not kind and text_note(body):
                    notes.append((sha, len(body), path))
    return report_scan(found, scanned, notes)


def add_deps(sub: argparse._SubParsersAction) -> None:
    parser = sub.add_parser("deps", help="the deps branch: crates (and, when configured, pub packages and CDN files)")
    commands = parser.add_subparsers(dest="command", required=True, metavar="COMMAND")
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--cargo-lock", type=Path, metavar="FILE", help="read this Cargo.lock instead of the repository's")
    common.add_argument("--deps", type=Path, metavar="DIR", help="the deps checkout (default: `deps path`)")
    common.add_argument("--pubspec-lock", type=Path, metavar="FILE", help="read this pubspec.lock instead of [deps.pub] lock")
    pub_cache = argparse.ArgumentParser(add_help=False)
    pub_cache.add_argument("--pub-cache", type=Path, metavar="DIR", help="the pub cache the pub packages are copied from (default: [deps.pub] cache, $PUB_CACHE, the platform's)")
    sync = commands.add_parser("sync", parents=[common, pub_cache], help="make the deps checkout match Cargo.lock (and the pub packages and CDN files): fetch, verify, remove")
    sync.set_defaults(run=deps_sync)
    check = commands.add_parser("check", parents=[common], help="fail, naming what is missing, changed or stale")
    check.add_argument("--quick", action="store_true", help="read each package's metadata only, without re-hashing its files")
    check.add_argument("--ref", metavar="REF", help="check a commit of this repository instead of the checkout")
    check.set_defaults(run=deps_check)
    use = commands.add_parser("use", parents=[common], help="write a cargo home that replaces every source by the checkout's crates/")
    use.add_argument("--out", type=Path, metavar="DIR", help="where to write it (default: a directory in the system temp dir named for the lock)")
    use.add_argument("--repo-config", action="store_true", help="also write the repository's .cargo/config.toml")
    use.add_argument("--json", action="store_true", help="print the result as JSON")
    use.add_argument("--full", action="store_true", help="re-hash every file first (default: package metadata only)")
    use.set_defaults(run=deps_use)
    refresh = commands.add_parser("refresh", parents=[common, pub_cache], help="sync, commit on the deps branch, repin, then pin the commit on main")
    refresh.set_defaults(run=deps_refresh)
    where = commands.add_parser("path", parents=[common], help="print the deps checkout, adding the worktree when it is missing")
    where.set_defaults(run=deps_path)
    init = commands.add_parser("init", help="make the orphan deps branch in a new worktree")
    init.add_argument("--path", type=Path, metavar="DIR", help="the worktree (default: REPO-deps beside the repository)")
    init.set_defaults(run=deps_init)


def add_scan(sub: argparse._SubParsersAction) -> None:
    parser = sub.add_parser("scan", help="refuse archives, executables, keys and opaque binaries: PASS or FAIL")
    parser.add_argument("refs", nargs="*", metavar="REF", help="commits whose trees are scanned (default: HEAD)")
    parser.add_argument("--all-refs", action="store_true", help="every blob reachable from any ref, history included")
    parser.add_argument("--tree", type=Path, metavar="DIR", help="the files of a directory")
    parser.set_defaults(run=scan_command)


SECTIONS.extend([add_deps, add_scan])

# --- the SDK lock's rows (read here; `sdk` lints and installs them) -------------------------------------



class Item(NamedTuple):
    line: int
    group: str
    name: str
    version: str
    platform: str
    sha256: str
    url: str


def parse_lock_items(text: str, name: str = "sdk.lock") -> list[Item]:
    """The rows of an SDK lock: six space-separated fields, `#` for a comment."""
    items = []
    for number, row in enumerate(text.split("\n"), 1):
        row = row.strip()
        if not row or row.startswith("#"):
            continue
        fields = row.split()
        if len(fields) != 6:
            raise Problem(f"{name}:{number}: expected 6 fields, found {len(fields)}")
        items.append(Item(number, *fields))
    return items


def read_sdk_lock(ctx: Ctx) -> list[Item]:
    path = ctx.root / ctx.cfg.sdk.lock
    if not path.is_file():
        raise Problem(f"{ctx.cfg.sdk.lock}: no such file")
    return parse_lock_items(path.read_text(encoding="utf-8"), path.name)


# --- pub packages and CDN files on the deps branch ----------------------------------------------------------

# Directories at the top of a package that compilation never reads.
PUB_DROP = ["test", "tests", "example", "examples", "benchmark", "benchmarks", "doc", "docs", ".github"]
PUB_HOSTED_URL = "https://pub.dev"

SCHEMA["deps"].keys["pub"] = Key(
    "table",
    None,
    "Dart packages of the Flutter app on the deps branch (off when `lock` is empty).",
    {
        "lock": Key("str", "", "The app's pubspec.lock, relative to the repository."),
        "drop": Key("strs", PUB_DROP, "Directories at a package's top that are left out."),
        "require": Key("strs", [], "Packages that must be in the pub cache whatever pub resolves (for example a desktop build's tray and window packages)."),
        "git": Key("str", "checkout", "`checkout`: pub/git/REPO-REV/ without .git is on the branch; `record`: git packages are only listed in manifest.json."),
        "system": Key("strs", [], "System packages the require list links on Linux, recorded in manifest.json for whoever builds offline."),
        "cache": Key("str", "", "The pub cache `deps sync` copies from; empty: --pub-cache, else $PUB_CACHE, else the default of the platform."),
    },
)
SCHEMA["deps"].keys["cdn"] = Key(
    "table",
    None,
    "Files a Flutter build downloads from a CDN (off when `index` is empty).",
    {
        "index": Key("str", "", "A JSON file `{\"files\": [{\"host\", \"path\", \"sha256\"}]}`, relative to the repository; files go to cdn/HOST/PATH."),
        "hosts": Key("strs", [], "Only these hosts of the index; empty: all."),
    },
)
SCHEMA["flutter"] = Key(
    "table",
    None,
    "Flutter helpers.",
    {
        "app_dir": Key("str", "app", "The Flutter app, relative to the repository."),
        "version_item": Key("str", "app flutter-version", "The lock row naming the framework version `precache-check` expects."),
        "engine_item": Key("str", "app flutter-engine", "The lock row naming the engine revision."),
        "dart_item": Key("str", "app dart", "The lock row naming the Dart version."),
    },
)
SCHEMA["offline"] = Key(
    "table",
    None,
    "`offline plan`.",
    {"cache_dir": Key("str", "target/offline-pub-cache", "Where the pub cache an offline build uses is seeded, relative to the repository.")},
)


class Package(NamedTuple):
    name: str
    source: str
    version: str
    sha256: str
    url: str
    path: str
    resolved_ref: str

    @property
    def repo(self) -> str:
        """The name pub gives a git dependency's folders in its cache."""
        return self.url.rstrip("/").rsplit("/", 1)[-1].removesuffix(".git")

    @property
    def host(self) -> str:
        return self.url.removeprefix("https://").removeprefix("http://").rstrip("/")

    @property
    def key(self) -> str:
        """The package's directory name below pub/hosted/HOST or pub/git."""
        return f"{self.name}-{self.version}" if self.source == "hosted" else f"{self.repo}-{self.resolved_ref}"


def parse_pubspec_lock(text: str) -> list[Package]:
    """The packages of a pubspec.lock, read by indentation: the file pub writes has one shape."""
    packages: list[Package] = []
    fields: dict[str, str] = {}
    name = ""

    def flush() -> None:
        if name:
            packages.append(
                Package(
                    name, fields.get("source", ""), fields.get("version", ""), fields.get("sha256", ""),
                    fields.get("url", ""), fields.get("path", ""), fields.get("resolved-ref", ""),
                )
            )  # fmt: skip

    in_packages = False
    for line in text.splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if not line.startswith(" "):
            flush()
            name, fields = "", {}
            in_packages = line.startswith("packages:")
            continue
        if not in_packages:
            continue
        if line.startswith("  ") and not line.startswith("   "):
            flush()
            name, fields = line.strip().rstrip(":"), {}
            continue
        key, _, value = line.strip().partition(":")
        if value.strip():
            fields[key] = value.strip().strip('"')
    flush()
    return packages


def default_pub_cache(environ: dict[str, str] | os._Environ) -> Path:
    if environ.get("PUB_CACHE"):
        return Path(environ["PUB_CACHE"])
    if os.name == "nt":
        return Path(environ.get("LOCALAPPDATA", str(Path.home() / "AppData" / "Local"))) / "Pub" / "Cache"
    return Path.home() / ".pub-cache"


def pubspec_lock_of(ctx: Ctx) -> Path | None:
    chosen = getattr(ctx, "pubspec_lock", None)
    if chosen:
        return Path(chosen)
    return ctx.root / ctx.cfg.deps.pub.lock if ctx.cfg.deps.pub.lock else None


def cdn_files(ctx: Ctx) -> list[dict]:
    """The entries of the CDN index, with each path checked: absolute, no `..`."""
    cfg = ctx.cfg.deps.cdn
    if not cfg.index:
        return []
    path = ctx.root / cfg.index
    try:
        entries = json.loads(path.read_text(encoding="utf-8"))["files"]
    except (OSError, ValueError, KeyError) as error:
        raise Problem(f"{cfg.index}: {error}") from error
    chosen = []
    for entry in entries:
        if cfg.hosts and entry["host"] not in cfg.hosts:
            continue
        name = entry["path"]
        if not name.startswith("/") or ".." in Path(name).parts or "\\" in name:
            raise Problem(f"{cfg.index}: {name!r} is not an absolute path without ..")
        chosen.append(entry)
    return chosen


def copy_package(source: Path, target: Path, drop: list[str], allow: Allow, prefix: str, skip_git: bool = False) -> tuple[collections.Counter, list[dict]]:
    """Copies one pub package without the directories a build never reads, then deletes any file a
    scanner would refuse. Returns (counts by dropped name or kind, the deleted files)."""
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(source, target, ignore=shutil.ignore_patterns(".git") if skip_git else None)
    dropped: collections.Counter = collections.Counter()
    deleted: list[dict] = []
    for name in drop:
        directory = target / name
        if directory.is_dir():
            dropped[name] += sum(1 for f in directory.rglob("*") if f.is_file())
            shutil.rmtree(directory)
    for file in sorted(f for f in target.rglob("*") if f.is_file()):
        rel = f"{prefix}/{file.relative_to(target).as_posix()}"
        kind = flagged_kind(file.read_bytes(), file.name)
        if kind and not allow.permits(rel, kind):
            dropped[kind] += 1
            deleted.append({"path": rel, "kind": kind})
            file.unlink()
    return dropped, deleted


def sync_pub(ctx: Ctx, deps_dir: Path, args: Any) -> tuple[list[str], dict]:
    cfg = ctx.cfg.deps.pub
    lock_path = pubspec_lock_of(ctx)
    if lock_path is None:
        return [], {}
    packages = [p for p in parse_pubspec_lock(lock_path.read_text(encoding="utf-8")) if p.source in ("hosted", "git")]
    chosen = getattr(args, "pub_cache", None)
    cache = Path(chosen) if chosen else Path(cfg.cache) if cfg.cache else default_pub_cache(os.environ)
    allow = Allow(ctx.cfg.deps.allow)
    target = deps_dir / "pub"
    counts: collections.Counter = collections.Counter()
    deleted: list[dict] = []
    names: list[str] = []
    hosted = [p for p in packages if p.source == "hosted"]
    git = [p for p in packages if p.source == "git"]
    wanted_hosted = {(p.host, p.key) for p in hosted}
    for p in hosted:
        source = cache / "hosted" / p.host / p.key
        if not source.is_dir():
            raise Problem(f"pub cache {cache} has no hosted/{p.host}/{p.key}; run `flutter pub get` first")
        done = copy_package(source, target / "hosted" / p.host / p.key, cfg.drop, allow, f"pub/hosted/{p.host}/{p.key}")
        counts.update(done[0])
        deleted += done[1]
        names.append(p.key)
        for rel in (f"hosted-hashes/{p.host}/{p.key}.sha256", f"hosted/{p.host}/.cache/{p.name}-versions.json"):
            if (cache / rel).is_file():
                (target / rel).parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(cache / rel, target / rel)
    wanted_git: set[str] = set()
    if cfg.git == "checkout":
        for p in git:
            source = cache / "git" / p.key
            if not source.is_dir():
                raise Problem(f"pub cache {cache} has no git/{p.key}; run `flutter pub get` first")
            done = copy_package(source, target / "git" / p.key, cfg.drop, allow, f"pub/git/{p.key}", skip_git=True)
            counts.update(done[0])
            deleted += done[1]
            wanted_git.add(p.key)
    changes = []
    # What the lock no longer names goes.
    for base in ("hosted", "hosted-hashes"):
        for host_dir in sorted((target / base).glob("*")) if (target / base).is_dir() else []:
            for entry in sorted(host_dir.iterdir()):
                stem = entry.name.removesuffix(".sha256")
                if entry.name == ".cache":
                    for cached in sorted(entry.iterdir()):
                        if not any(cached.name == f"{p.name}-versions.json" and p.host == host_dir.name for p in hosted):
                            cached.unlink()
                            changes.append(f"removed pub/{base}/{host_dir.name}/.cache/{cached.name}")
                    continue
                if entry.name.startswith(".") or (host_dir.name, stem) in wanted_hosted:
                    continue
                shutil.rmtree(entry) if entry.is_dir() else entry.unlink()
                changes.append(f"removed pub/{base}/{host_dir.name}/{entry.name}")
    if (target / "git").is_dir():
        for entry in sorted((target / "git").iterdir()):
            if entry.name not in wanted_git:
                shutil.rmtree(entry) if entry.is_dir() else entry.unlink()
                changes.append(f"removed pub/git/{entry.name}")
    present = {p.name for p in hosted}
    absent = [name for name in cfg.require if name not in present]
    if absent:
        raise Problem("required packages missing from pubspec.lock: " + ", ".join(absent))
    changes.append(f"pub: {len(hosted)} hosted packages, {len(wanted_git)} git checkouts, {sum(counts.values())} files dropped")
    info = {
        "pub": {
            "packages": sorted(names),
            "gitDependencies": [
                {"package": p.name, "url": p.url, "path": p.path, "ref": p.resolved_ref, "mode": cfg.git} for p in git
            ],
            "dropped": {"directories": list(cfg.drop), "counts": dict(sorted(counts.items())), "deletedFlaggedFiles": deleted},
            "require": list(cfg.require),
            "systemPackages": list(cfg.system),
        }
    }
    return changes, info


def sync_cdn(ctx: Ctx, deps_dir: Path, args: Any) -> tuple[list[str], dict]:
    entries = cdn_files(ctx)
    if not ctx.cfg.deps.cdn.index:
        return [], {}
    changes, listed = [], []
    template = "https://{host}{path}"

    def copy(entry: dict) -> tuple[dict, bool]:
        destination = deps_dir / "cdn" / entry["host"] / entry["path"].lstrip("/")
        if destination.is_file() and sha256(destination.read_bytes()) == entry["sha256"]:
            return {"host": entry["host"], "path": entry["path"], "sha256": entry["sha256"], "bytes": destination.stat().st_size}, False
        data = fetch(template.format(host=entry["host"], path=entry["path"]))
        if sha256(data) != entry["sha256"]:
            raise Problem(f"CDN checksum mismatch for {entry['host']}{entry['path']}")
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
        return {"host": entry["host"], "path": entry["path"], "sha256": entry["sha256"], "bytes": len(data)}, True

    with ThreadPoolExecutor(max_workers=max(1, ctx.cfg.deps.jobs)) as pool:
        results = list(pool.map(copy, entries))
    wanted = set()
    for entry, (done, fetched) in zip(entries, results):
        listed.append(done)
        wanted.add((deps_dir / "cdn" / entry["host"] / entry["path"].lstrip("/")).resolve())
        if fetched:
            changes.append(f"added cdn/{entry['host']}{entry['path']}")
    cdn = deps_dir / "cdn"
    if cdn.is_dir():
        for file in sorted(f for f in cdn.rglob("*") if f.is_file()):
            if file.resolve() not in wanted:
                file.unlink()
                changes.append(f"removed cdn/{file.relative_to(cdn).as_posix()}")
    return changes, {"cdn": {"files": sorted(listed, key=lambda f: (f["host"], f["path"]))}}


def pub_check(ctx: Ctx, view: Any, lock: Lock) -> tuple[list[str], list[str]]:
    """Every package pubspec.lock names is on the branch, or the offline build stops there.

    A package from the Flutter SDK comes with it, and a path package is the checkout's. A hosted
    package is `pub/hosted/HOST/NAME-VERSION`. A git package is pub's checkout of its repository at
    the locked revision (`checkout`), or only listed in the manifest (`record`).
    """
    cfg = ctx.cfg.deps.pub
    lock_path = pubspec_lock_of(ctx)
    if lock_path is None:
        return [], []
    found: list[str] = []
    packages = parse_pubspec_lock(lock_path.read_text(encoding="utf-8"))
    if not packages:
        found.append(f"{lock_path.name} names no package")
    manifest = json.loads(view.read(MANIFEST) or b"{}") if cfg.git == "record" else {}
    recorded = {(g.get("package"), g.get("ref")) for g in manifest.get("pub", {}).get("gitDependencies", [])}
    for package in packages:
        if package.source == "hosted":
            where = f"pub/hosted/{package.host}/{package.key}"
            have = view.exists(where)
        elif package.source == "git" and cfg.git == "checkout":
            where = f"pub/git/{package.key}/{package.path}/pubspec.yaml".replace("//", "/")
            have = view.exists(where)
        elif package.source == "git":
            where = f"{MANIFEST} gitDependencies"
            have = (package.name, package.resolved_ref) in recorded
        else:
            continue
        if not have:
            found.append(f"{lock_path.name} names {package.name} {package.version}, but there is no {where}; refresh the deps branch")
    hosted_names = {p.name for p in packages if p.source == "hosted"}
    found += [f"required package {name} is not in {lock_path.name}" for name in cfg.require if name not in hosted_names]
    return found, []


def cdn_check(ctx: Ctx, view: Any, lock: Lock) -> tuple[list[str], list[str]]:
    if not ctx.cfg.deps.cdn.index:
        return [], []
    found = []
    deep = not getattr(ctx, "quick", False)
    for entry in cdn_files(ctx):
        where = f"cdn/{entry['host']}{entry['path']}"
        if not view.exists(where):
            found.append(f"{where}: missing, {ctx.cfg.deps.cdn.index} names it")
        elif deep and sha256(view.read(where) or b"") != entry["sha256"]:
            found.append(f"{where}: SHA-256 is not the index's {entry['sha256']}")
    return found, []


DEPS_SYNCS.extend([sync_pub, sync_cdn])
DEPS_CHECKS.extend([pub_check, cdn_check])


# --- offline plan: what an offline build of the repository needs -----------------------------------------------


def seed(source: Path, target: Path) -> None:
    if source.is_dir() and not target.exists():
        shutil.copytree(source, target)
    elif source.is_file() and not target.exists():
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)


def offline_plan(
    ctx: Ctx, deps: Path | None, user_cache: Path, cargo_lock: Path | None, pubspec_lock: Path | None, cargo_out: Path | None = None
) -> dict:
    """{cache, reason, cargo_home}: the seeded pub cache and cargo home, or why a build must go online.

    The user's pub cache is only read; the seeded one is below the repository's target directory.
    """
    plan: dict = {"cache": None, "reason": None, "cargo_home": None}
    if deps is None or not deps.is_dir():
        plan["reason"] = "no deps checkout (pass --deps, set BUILD_DEPS, or fetch the deps branch)"
        return plan
    cfg = ctx.cfg.deps.pub
    target = ctx.root / ctx.cfg.offline.cache_dir
    if pubspec_lock is not None and pubspec_lock.is_file():
        wanted = [p for p in parse_pubspec_lock(pubspec_lock.read_text(encoding="utf-8")) if p.source in ("hosted", "git")]
        hosted = [p for p in wanted if p.source == "hosted"]
        git = [p for p in wanted if p.source == "git"]
        if any(p.url != PUB_HOSTED_URL for p in hosted):
            plan["reason"] = "a hosted package comes from a host other than pub.dev"
            return plan
        absent = [p.key for p in hosted if not (deps / "pub" / "hosted" / p.host / p.key).is_dir()]
        if absent:
            plan["reason"] = f"the deps branch lacks hosted packages: {', '.join(absent[:5])}"
            return plan
        differs = []
        for p in hosted:
            recorded = deps / "pub" / "hosted-hashes" / p.host / f"{p.key}.sha256"
            if recorded.is_file() and recorded.read_text(encoding="utf-8").strip() != p.sha256:
                differs.append(p.key)
        if differs:
            plan["reason"] = f"the deps branch's package hashes differ from {pubspec_lock.name}: {', '.join(sorted(differs)[:5])}"
            return plan
        if cfg.git == "record":
            missing = [p for p in git if not (user_cache / "git" / p.key).is_dir()]
            if missing:
                listing = ", ".join(f"{p.name} {p.resolved_ref[:12]}" for p in missing)
                plan["reason"] = f"git dependencies are not in the pub cache {user_cache}: {listing}"
                return plan
        else:
            absent_git = [p.key for p in git if not (deps / "pub" / "git" / p.key).is_dir()]
            if absent_git:
                plan["reason"] = f"the deps branch lacks git checkouts: {', '.join(absent_git[:5])}"
                return plan
    if cargo_lock is not None and cargo_lock.is_file():
        lock = read_lock(cargo_lock)
        try:
            info = use_crates(ctx, deps, lock, cargo_lock, cargo_out)
        except Problem as problem:
            plan["reason"] = f"the deps branch's crates/ does not cover {cargo_lock.name}: {problem}"
            return plan
        plan["cargo_home"] = info["cargo_home"]
    if pubspec_lock is not None and pubspec_lock.is_file():
        for p in hosted:
            seed(deps / "pub" / "hosted" / p.host / p.key, target / "hosted" / p.host / p.key)
            seed(deps / "pub" / "hosted" / p.host / ".cache" / f"{p.name}-versions.json", target / "hosted" / p.host / ".cache" / f"{p.name}-versions.json")
            seed(deps / "pub" / "hosted-hashes" / p.host / f"{p.key}.sha256", target / "hosted-hashes" / p.host / f"{p.key}.sha256")
        for p in git:
            if cfg.git == "checkout":
                destination = target / "git" / p.key
                seed(deps / "pub" / "git" / p.key, destination)
                # Pub takes a git package from its checkout once the checkout has .git/pub-packages,
                # which pub fills in itself; git cannot carry a .git path, so the branch has none.
                (destination / ".git").mkdir(exist_ok=True)
                (destination / ".git" / "pub-packages").touch()
            else:
                seed(user_cache / "git" / p.key, target / "git" / p.key)
                for bare in (user_cache / "git" / "cache").glob(f"{p.repo}-*"):
                    seed(bare, target / "git" / "cache" / bare.name)
        plan["cache"] = str(target)
    return plan


def offline_command(ctx: Ctx, args: argparse.Namespace) -> int:
    cargo_lock = args.cargo_lock.resolve() if args.cargo_lock else ctx.root / ctx.cfg.deps.cargo_lock
    ctx.pubspec_lock = args.pubspec_lock
    try:
        deps = find_deps(ctx, args.deps)
    except Problem as problem:
        deps = None
        reason = str(problem)
    chosen = args.pub_cache
    user_cache = Path(chosen) if chosen else Path(ctx.cfg.deps.pub.cache) if ctx.cfg.deps.pub.cache else default_pub_cache(os.environ)
    plan = offline_plan(ctx, deps, user_cache, cargo_lock, pubspec_lock_of(ctx), args.out)
    if deps is None:
        plan["reason"] = reason
    if args.json:
        print(json.dumps(plan, indent=2))
    else:
        print(f"pub cache:  {plan['cache'] or '-'}")
        print(f"cargo home: {plan['cargo_home'] or '-'}")
        if plan["reason"]:
            print(f"offline is not possible: {plan['reason']}")
    return 0 if not plan["reason"] else 1


def add_offline(sub: argparse._SubParsersAction) -> None:
    parser = sub.add_parser("offline", help="what an offline build of the repository needs")
    commands = parser.add_subparsers(dest="command", required=True, metavar="COMMAND")
    plan = commands.add_parser("plan", help="seed the pub cache and write the cargo home from the deps checkout, or say why not")
    plan.add_argument("--json", action="store_true", help="print {cache, reason, cargo_home} as JSON")
    plan.add_argument("--deps", type=Path, metavar="DIR", help="the deps checkout (default: `deps path`)")
    plan.add_argument("--cargo-lock", type=Path, metavar="FILE", help="read this Cargo.lock instead of the repository's")
    plan.add_argument("--pubspec-lock", type=Path, metavar="FILE", help="read this pubspec.lock instead of [deps.pub] lock")
    plan.add_argument("--pub-cache", type=Path, metavar="DIR", help="the user's pub cache, only read (git packages recorded, not vendored, come from it)")
    plan.add_argument("--out", type=Path, metavar="DIR", help="where the cargo home is written (default: a temp directory named for the lock)")
    plan.set_defaults(run=offline_command)


# --- flutter: plugin junctions and the pinned checkout ---------------------------------------------------------


def make_junction(link: Path, target: Path) -> None:
    """A directory junction, which needs no privilege (a symbolic link needs Developer Mode)."""
    done = subprocess.run(["cmd", "/c", "mklink", "/J", str(link), str(target)], capture_output=True, text=True)
    if done.returncode != 0:
        raise Problem(f"mklink /J {link}: {done.stdout.strip() or done.stderr.strip()}")


def flutter_junctions(ctx: Ctx, args: argparse.Namespace) -> int:
    """Recreates the Windows plugin links `flutter build windows` needs, without Developer Mode.

    Flutter makes `windows/flutter/ephemeral/.plugin_symlinks/<plugin>` as symbolic links and
    skips any link that already exists, so a junction made here is accepted. Idempotent.
    """
    if not IS_WINDOWS:
        print("not Windows: no plugin links to make")
        return 0
    app = (args.app or ctx.root / ctx.cfg.flutter.app_dir).resolve()
    manifest = app / ".flutter-plugins-dependencies"
    if not manifest.is_file():
        raise Problem(f"no .flutter-plugins-dependencies in {app}: run `flutter pub get` first")
    links = app / "windows" / "flutter" / "ephemeral" / ".plugin_symlinks"
    links.mkdir(parents=True, exist_ok=True)
    plugins = json.loads(manifest.read_text(encoding="utf-8")).get("plugins", {}).get("windows") or []
    if not plugins:
        print("no Windows plugins declared")
        return 0
    created = 0
    for plugin in plugins:
        link = links / plugin["name"]
        if link.exists() or link.is_symlink():
            continue
        # The manifest stores a trailing separator and doubled separators.
        target = Path(os.path.normpath(plugin["path"]))
        if not target.exists():
            print(f"warning: {plugin['name']}: target missing, skipped: {target}", file=sys.stderr)
            continue
        make_junction(link, target)
        print(f"linked {plugin['name']} -> {target}")
        created += 1
    print(f"{created} junction(s) created; {len(plugins)} plugin(s) declared")
    return 0


def json_field(text: str, key: str) -> str:
    found = re.search(rf'"{re.escape(key)}"\s*:\s*"([^"]*)"', text)
    return found[1] if found else ""


def flutter_precache_check(ctx: Ctx, args: argparse.Namespace) -> int:
    """What Flutter reports about itself, against what the lock expects of that commit."""
    cfg = ctx.cfg.flutter
    root = args.flutter_root or (Path(os.environ["FLUTTER_ROOT"]) if os.environ.get("FLUTTER_ROOT") else None)
    if root is None:
        raise Usage("name the Flutter checkout with --flutter-root or $FLUTTER_ROOT")
    items = read_sdk_lock(ctx)

    def want(row: str) -> str:
        group, _, name = row.partition(" ")
        found = [i.version for i in items if i.group == group and i.name == name]
        if len(found) != 1:
            raise Problem(f"{ctx.cfg.sdk.lock}: expected one `{row}` row, found {len(found)}")
        return found[0]

    version_file = root / "bin" / "cache" / "flutter.version.json"
    stamp = root / "bin" / "cache" / "engine.stamp"
    if not version_file.is_file() or not stamp.is_file():
        raise Problem(f"{root}: no bin/cache/flutter.version.json or engine.stamp; run `flutter precache` there")
    text = version_file.read_text(encoding="utf-8")
    found = []
    got = json_field(text, "frameworkVersion")
    if got != want(cfg.version_item):
        found.append(f"flutter reports version {got}, the lock says {want(cfg.version_item)}")
    got = stamp.read_text(encoding="utf-8").strip()
    if got != want(cfg.engine_item):
        found.append(f"flutter's engine is {got}, the lock says {want(cfg.engine_item)}")
    got = json_field(text, "dartSdkVersion")
    if want(cfg.dart_item) not in got:
        found.append(f"flutter's Dart is {got}, the lock says {want(cfg.dart_item)}")
    if found:
        raise Problem("\n".join(found))
    print(f"ok: Flutter {want(cfg.version_item)}, engine {want(cfg.engine_item)[:12]}, Dart {want(cfg.dart_item)}")
    return 0


def add_flutter(sub: argparse._SubParsersAction) -> None:
    parser = sub.add_parser("flutter", help="plugin junctions on Windows and the pinned Flutter checkout")
    commands = parser.add_subparsers(dest="command", required=True, metavar="COMMAND")
    junctions = commands.add_parser("junctions", help="recreate the Windows plugin links flutter build windows needs (no privilege)")
    junctions.add_argument("--app", type=Path, metavar="DIR", help="the Flutter app (default: [flutter] app_dir)")
    junctions.set_defaults(run=flutter_junctions)
    check = commands.add_parser("precache-check", help="verify the checkout's version, engine and Dart against the SDK lock")
    check.add_argument("--flutter-root", type=Path, metavar="DIR", help="the Flutter checkout (default: $FLUTTER_ROOT)")
    check.set_defaults(run=flutter_precache_check)


SECTIONS.extend([add_offline, add_flutter])

# --- sdk: the lock, its agreement with the repository, and installing it ------------------------------------------

SDK_GROUPS = ["core", "app", "api", "docs", "test"]
SDK_PLATFORMS = ["windows-x64", "windows-arm64", "linux-x64", "linux-arm64", "macos-arm64", "macos-x64"]
TRIPLES = {
    "windows-x64": "x86_64-pc-windows-msvc", "windows-arm64": "aarch64-pc-windows-msvc",
    "linux-x64": "x86_64-unknown-linux-gnu", "linux-arm64": "aarch64-unknown-linux-gnu",
    "macos-arm64": "aarch64-apple-darwin", "macos-x64": "x86_64-apple-darwin",
}  # fmt: skip
# A name that means "whatever is newest" is not a pin.
FLOATING = re.compile(r"^(stable|beta|nightly|latest|lts|main|master|head|dev)$", re.IGNORECASE)

SCHEMA["sdk"] = Key(
    "table",
    None,
    "The SDK lock: every tool a build needs at an exact version, one row `GROUP NAME VERSION OS-ARCH|any SHA256|- URL|-` per line.",
    {
        "lock": Key("str", "tools/sdk.lock", "The lock file, relative to the repository."),
        "groups": Key("strs", SDK_GROUPS, "The groups a row may name besides `system`."),
        "platforms": Key("strs", SDK_PLATFORMS, "The platforms a row may name besides `any`."),
        "all_platforms": Key("strs", ["core rustup-init"], "Rows (`GROUP NAME`) that must have a line for every platform."),
        "max_bytes": Key("int", 20480, "The lock's size above which it is refused as not a text lock."),
        "install": Key("strs", ["core"], "The groups `sdk install` installs when --groups is not given."),
        "git_urls": Key("map", {}, "Item name to the URL it is cloned from, replacing the lock's (a mirror); $BUILD_SDK_GIT_URL_NAME wins."),
        "env": Key("map", {}, "Variables env.sh and env.ps1 set; a value may use {prefix} and {dir:GROUP NAME}."),
        "path": Key("strs", [], "Directories env.sh and env.ps1 put on PATH; the same templates."),
        "agree": Key(
            "tables",
            None,
            "A fact in the repository that must equal a row of the lock.",
            {
                "item": Key("str", REQUIRED, "The row, `GROUP NAME`."),
                "file": Key("str", REQUIRED, "The file that states the fact, relative to the repository."),
                "pattern": Key("str", REQUIRED, "A regular expression (multi-line); its first group, or the whole match, is the value."),
                "ref": Key("str", "", "Read the file at this git ref instead of the checkout; @pin is the deps commit [deps] pin names, @deps the --deps-ref."),
                "match": Key("str", "equal", "`equal`: every value is the row's; `prefix`: each value is the row's version or a dotted prefix of it; `set`: the values are exactly the rows'; `in`: one value is the row's."),
                "expect": Key("str", "{version}", "The row's side of the comparison, over {version}, {sha256}, {url}, {name} and {platform}."),
                "platform": Key("str", "", "Only the row for this platform (or `any`); empty: every row."),
                "first": Key("bool", False, "Only the first row of the item."),
            },
        ),
        "venv": Key(
            "tables",
            None,
            "A Python virtual environment installed from a hashed requirements file.",
            {"item": Key("str", REQUIRED, "The row it is named for, `GROUP NAME`."), "requirements": Key("str", REQUIRED, "The requirements file, relative to the repository.")},
        ),
        "post": Key(
            "tables",
            None,
            "A command run once in a git item's directory after it is cloned.",
            {
                "item": Key("str", REQUIRED, "The git row, `GROUP NAME`."),
                "run": Key("strs", REQUIRED, "The command, its first word relative to the item's directory."),
                "stamp": Key("str", ".build-post", "A file in the directory that records the command ran."),
            },
        ),
        "system": Key("table", None, "System packages, which are only checked.", {"print": Key("map", {}, "What to install, by os (linux, macos, windows): the text `--print-system` prints.")}),
    },
)


def split_item(row: str) -> tuple[str, str]:
    group, _, name = row.partition(" ")
    return group, name.strip()


def lint_lock(ctx: Ctx, raw: bytes, name: str) -> tuple[list[Item], list[str]]:
    """The rows of a lock and a line for each way they are not a well-formed lock."""
    cfg = ctx.cfg.sdk
    errors: list[str] = []

    def error(message: str, line: int | None = None) -> None:
        errors.append(f"{name}:{line}: {message}" if line else f"{name}: {message}")

    if len(raw) > cfg.max_bytes:
        error(f"{len(raw)} bytes, over the {cfg.max_bytes} a text lock should need")
    if b"\r" in raw:
        error("has CR line endings")
    items: list[Item] = []
    seen: dict[tuple[str, str, str, str], int] = {}
    groups = set(cfg.groups)
    for number, text in enumerate(raw.decode("utf-8").split("\n"), 1):
        text = text.strip()
        if not text or text.startswith("#"):
            continue
        fields = text.split()
        if len(fields) != 6:
            error(f"expected 6 fields, found {len(fields)}", number)
            continue
        item = Item(number, *fields)
        items.append(item)
        if item.group != "system" and item.group not in groups:
            error(f"unknown group {item.group}", number)
        if item.platform != "any" and item.platform not in cfg.platforms:
            error(f"unknown platform {item.platform}", number)
        if item.sha256 != "-" and not HEX64.match(item.sha256):
            error(f"hash of {item.name} is not 64 hex characters: {item.sha256}", number)
        if item.url != "-" and not re.match(r"^(https|docker)://\S+$", item.url):
            error(f"url of {item.name} is not https or docker: {item.url}", number)
        if FLOATING.match(item.version):
            error(f"{item.name} floats: {item.version}", number)
        if item.group == "system":
            if item.sha256 != "-" or item.url != "-":
                error(f"system item {item.name} has a hash or a url", number)
            if item.version not in groups:
                error(f"system item {item.name} names no group: {item.version}", number)
        elif item.url.startswith("https://"):
            if item.url.endswith(".git"):
                if not HEX40.match(item.version):
                    error(f"git item {item.name} is not at a commit: {item.version}", number)
            elif item.sha256 == "-":
                error(f"download {item.name} has no hash", number)
        if item.group == "core" and item.name == "rust" and not re.match(r"^\d+\.\d+\.\d+$", item.version):
            error(f"rust is not an exact version: {item.version}", number)
        key = (item.group, item.name, item.version, item.platform)
        if key in seen:
            error(f"same item as line {seen[key]}", number)
        seen[key] = number
    for row in cfg.all_platforms:
        group, item_name = split_item(row)
        have = {i.platform for i in items if i.group == group and i.name == item_name}
        missing = set(cfg.platforms) - have
        if missing:
            error(f"{group} {item_name} is missing {', '.join(sorted(missing))}")
    return items, errors


def read_at(ctx: Ctx, ref: str | None, path: str) -> str | None:
    """A file of the checkout, or of a commit; None when it is not there."""
    if ref is None:
        target = ctx.root / path
        return target.read_bytes().decode("utf-8") if target.is_file() else None
    done = subprocess.run(["git", "-C", str(ctx.root), "show", f"{ref}:{path}"], capture_output=True)
    return done.stdout.decode("utf-8") if done.returncode == 0 else None


def pin_commit(ctx: Ctx) -> tuple[str | None, int | None]:
    """(the deps commit `[deps] pin` names, its line); None when no pin is configured."""
    pin = ctx.cfg.deps.pin
    if not pin.lock:
        return None, None
    path = ctx.root / pin.lock
    if not path.is_file():
        raise Problem(f"[deps] pin: {pin.lock}: no such file")
    group, name = split_item(pin.item)
    rows = [i for i in parse_lock_items(path.read_text(encoding="utf-8"), path.name) if i.group == group and i.name == name]
    if len(rows) != 1:
        raise Problem(f"{pin.lock}: expected one `{pin.item}` row, found {len(rows)}")
    return rows[0].version, rows[0].line


def agree_problems(ctx: Ctx, items: list[Item], lock_name: str, pin: str | None, deps_ref: str, notes: list[str]) -> list[str]:
    found: list[str] = []
    for rule in ctx.cfg.sdk.agree:
        group, name = split_item(rule.item)
        rows = [i for i in items if i.group == group and i.name == name and (not rule.platform or i.platform in (rule.platform, "any"))]
        if rule.first:
            rows = rows[:1]
        if not rows:
            found.append(f"{lock_name}: agree {rule.item}: the lock has no such row")
            continue
        line = f"{lock_name}:{rows[0].line}"
        if rule.ref == "@pin" and pin is None:
            found.append(f"{line}: agree {rule.item}: ref @pin needs [deps] pin")
            continue
        ref = {"": None, "@pin": pin, "@deps": deps_ref}.get(rule.ref, rule.ref)
        where = rule.file + (f" at {ref[:12] if HEX40.match(ref or '') else ref}" if ref else "")
        text = read_at(ctx, ref, rule.file)
        if text is None:
            if ref:
                notes.append(f"no {rule.file} at {ref}: not checked")
            else:
                found.append(f"{line}: {rule.file}: no such file")
            continue
        expected = [rule.expect.format(version=r.version, sha256=r.sha256, url=r.url, name=r.name, platform=r.platform) for r in rows]
        values = [m.group(1) if m.re.groups else m.group(0) for m in re.finditer(rule.pattern, text, re.MULTILINE)]
        if not values:
            found.append(f"{line}: {where} has no match for {rule.pattern}")
            continue
        mode = rule.match
        if mode == "equal":
            if len(expected) != 1:
                found.append(f"{line}: agree {rule.item}: `equal` needs one row, the lock has {len(expected)}; use platform, first or set")
            elif any(v != expected[0] for v in values):
                found.append(f"{line}: {where} has {sorted(set(values))}, the lock {expected[0]}")
        elif mode == "prefix":
            for v in sorted(set(values)):
                if not any(e == v or e.startswith(v + ".") for e in expected):
                    found.append(f"{line}: {where} has {v}, which is not in the lock's {expected}")
        elif mode == "set":
            if set(values) != set(expected):
                found.append(f"{line}: {where} names {sorted(set(values))}, the lock {sorted(set(expected))}")
        elif mode == "in":
            if expected[0] not in values:
                found.append(f"{line}: {where} does not name {expected[0]}")
        else:
            found.append(f"{line}: agree {rule.item}: unknown match {mode!r}")
    return found


def upstream_sha256(item: Item) -> str:
    """What the item's publisher says its download hashes to (the only way to notice a well-formed
    hash that was changed)."""
    if item.name == "rustup-init":
        return fetch(item.url + ".sha256").decode("utf-8").split()[0]
    base = item.url.rsplit("/", 1)[1]
    if item.url.startswith("https://nodejs.org/dist/"):
        folder = item.url.rsplit("/", 1)[0]
        rows = fetch(folder + "/SHASUMS256.txt").decode("utf-8").splitlines()
        return {r.split()[1]: r.split()[0] for r in rows if len(r.split()) == 2}.get(base, "")
    found = re.match(r"^https://github\.com/([^/]+)/([^/]+)/releases/download/([^/]+)/", item.url)
    if found:
        release = json.loads(fetch(f"https://api.github.com/repos/{found[1]}/{found[2]}/releases/tags/{found[3]}"))
        for asset in release["assets"]:
            if asset["name"] == base:
                return str(asset.get("digest", "")).removeprefix("sha256:")
    return ""


def online_problems(items: list[Item], lock_name: str) -> list[str]:
    found = []
    for item in items:
        if item.group == "system" or not item.url.startswith("https://") or item.sha256 == "-" or item.url.endswith(".git"):
            continue
        known = item.name == "rustup-init" or item.url.startswith("https://nodejs.org/dist/") or item.url.startswith("https://github.com/")
        if not known:
            continue
        expected = upstream_sha256(item)
        if expected != item.sha256:
            found.append(f"{lock_name}:{item.line}: {item.name} {item.platform}: upstream says {expected}")
    return found


def sdk_check(ctx: Ctx, args: argparse.Namespace) -> int:
    cfg = ctx.cfg.sdk
    path = ctx.root / cfg.lock
    if not path.is_file():
        raise Problem(f"{cfg.lock}: no such file")
    items, found = lint_lock(ctx, path.read_bytes(), path.name)
    notes: list[str] = []
    pin, pin_line = pin_commit(ctx)
    if pin is not None:
        if not HEX40.match(pin):
            found.append(f"{path.name}:{pin_line}: the deps pin is not a commit: {pin}")
        else:
            lock_file = ctx.root / ctx.cfg.deps.cargo_lock
            more, extra = ref_problems(ctx, read_lock(lock_file), pin)
            found += [f"{ctx.cfg.deps.pin.lock}:{pin_line}: {line}" for line in more]
            notes += extra
    deps_ref = args.deps_ref or f"origin/{ctx.cfg.deps.branch}"
    found += agree_problems(ctx, items, path.name, pin, deps_ref, notes)
    if args.online:
        found += online_problems(items, path.name)
    for note in notes:
        print(f"note: {note}")
    if found:
        raise Problem("\n".join(found))
    print(f"ok: {len(items)} items in {path.name}")
    return 0


# --- sdk install --------------------------------------------------------------------------------------------------


def host_platform() -> str:
    """The machine's platform, `OS-ARCH`; $BUILD_SDK_PLATFORM names another.

    On Windows the process's own architecture can be an emulated one, so the machine's is read.
    """
    if os.environ.get("BUILD_SDK_PLATFORM"):
        return os.environ["BUILD_SDK_PLATFORM"]
    system = {"Windows": "windows", "Linux": "linux", "Darwin": "macos"}.get(platform.system())
    machine = (os.environ.get("PROCESSOR_ARCHITEW6432") or os.environ.get("PROCESSOR_ARCHITECTURE") or "") if system == "windows" else ""
    machine = (machine or platform.machine()).lower()
    arch = {"x86_64": "x64", "amd64": "x64", "aarch64": "arm64", "arm64": "arm64"}.get(machine)
    if system is None or arch is None:
        raise Problem(f"no SDK is listed for {platform.system()} on {machine or 'an unknown processor'}")
    return f"{system}-{arch}"


def run_tool(command: list[str], env: dict[str, str] | None = None, cwd: Path | None = None) -> None:
    """Runs a tool, its output shown; a non-zero exit is a Problem."""
    done = subprocess.run(command, env=env, cwd=cwd)
    if done.returncode != 0:
        raise Problem(f"{' '.join(str(c) for c in command)} exited with {done.returncode}")


def capture_tool(command: list[str], env: dict[str, str] | None = None) -> tuple[int, str]:
    """(exit code, stdout) of a tool, or (127, '') when it is not there."""
    try:
        done = subprocess.run(command, env=env, capture_output=True, text=True)
    except OSError:
        return 127, ""
    return done.returncode, done.stdout


def unpack(archive: Path, dest: Path, strip: int = 1) -> None:
    name = archive.name.lower()
    if name.endswith(".zip"):
        with zipfile.ZipFile(archive) as zf:
            safe_unzip(zf, dest, strip)
    elif name.endswith((".tar.gz", ".tgz", ".tar.xz", ".tar.bz2")):
        with tarfile.open(archive, "r:*") as tar:
            safe_extract(tar, dest, links="inside", strip=strip)
    else:
        raise Problem(f"cannot unpack {archive.name}: only .zip, .tar.gz, .tar.xz and .tar.bz2")


class Installer:
    """What `sdk install` does for one platform, prefix and set of groups."""

    def __init__(self, ctx: Ctx, items: list[Item], plat: str, prefix: Path, groups: list[str], check: bool, native: dict[str, tuple[str, str]] | None = None) -> None:
        self.ctx, self.plat, self.prefix, self.groups, self.check = ctx, plat, prefix, groups, check
        self.native = native or {}
        self.windows = plat.startswith("windows")
        self.triple = TRIPLES[plat]
        self.missing = False
        self.paths: list[str] = []
        self.dirs: dict[str, Path] = {}
        self.items = [i for i in items if i.platform in (plat, "any")]
        self.rustup_home = prefix / "rust" / "rustup"
        self.cargo_home = prefix / "rust" / "cargo"
        self.exe = ".exe" if self.windows else ""
        self.rustup = self.cargo_home / "bin" / f"rustup{self.exe}"
        self.tool_env = {**os.environ, "RUSTUP_HOME": str(self.rustup_home), "CARGO_HOME": str(self.cargo_home)}

    # reporting
    def report(self, what: str, state: str) -> None:
        print(f"{what:<32} {state}")

    def need(self, what: str, state: str) -> None:
        self.missing = True
        self.report(what, state)

    def rows(self, name: str, group: str | None = None) -> list[Item]:
        return [i for i in self.items if i.name == name and i.group != "system" and i.group in self.groups and (group is None or i.group == group)]

    # verified downloads
    def kept(self, item: Item) -> Path:
        return self.prefix / "dl" / f"{item.name}-{item.version}" / item.url.rsplit("/", 1)[1]

    def verify_kept(self, item: Item) -> None:
        """A kept copy that changed on disk is refused and removed rather than trusted."""
        path = self.kept(item)
        if path.is_file() and sha256(path.read_bytes()) != item.sha256:
            actual = sha256(path.read_bytes())
            path.unlink()
            raise Problem(f"refused and removed {path}\n  the lock says {item.sha256}\n  the file was  {actual}")

    def fetch_verified(self, item: Item) -> Path:
        self.verify_kept(item)
        path = self.kept(item)
        if path.is_file():
            return path
        self.report(f"{item.name} {item.version}", f"fetching {item.url}")
        self.prefix.mkdir(parents=True, exist_ok=True)
        scratch = Path(tempfile.mkdtemp(prefix="tmp.", dir=self.prefix))
        try:
            data = fetch(item.url)
            if sha256(data) != item.sha256:
                raise Problem(f"refused {item.url}\n  the lock says {item.sha256}\n  the download  {sha256(data)}")
            partial = scratch / path.name
            partial.write_bytes(data)
            path.parent.mkdir(parents=True, exist_ok=True)
            os.replace(partial, path)
        finally:
            shutil.rmtree(scratch, ignore_errors=True)
        return path

    # rust
    def rustup_ready(self, version: str) -> bool:
        if not self.rustup.is_file():
            return False
        code, out = capture_tool([str(self.rustup), "--version"], self.tool_env)
        return code == 0 and out.split()[1:2] == [version]

    def toolchain_ready(self, version: str, components: list[str], targets: list[str]) -> bool:
        """Asks only the prefix, so a run with nothing to do stays off the network."""
        if not self.rustup.is_file():
            return False
        code, out = capture_tool([str(self.rustup), "toolchain", "list"], self.tool_env)
        if code or not any(row.startswith(f"{version}-{self.triple}") for row in out.splitlines()):
            return False
        code, out = capture_tool([str(self.rustup), "component", "list", "--toolchain", version, "--installed"], self.tool_env)
        if code or not all(any(row.startswith(c) for row in out.splitlines()) for c in components):
            return False
        code, out = capture_tool([str(self.rustup), "target", "list", "--toolchain", version, "--installed"], self.tool_env)
        return not code and all(t in out.split() for t in targets)

    def rust(self) -> None:
        init = [i for i in self.rows("rustup-init")]
        toolchains = self.rows("rust")
        if not toolchains:
            return
        if len(init) != 1:
            raise Problem(f"the lock needs one rustup-init row for {self.plat}, it has {len(init)}")
        init = init[0]
        components = [i.version for i in self.rows("rust-component")]
        targets = [i.version for i in self.rows("rust-target")]
        numeric = [t.version for t in toolchains if re.match(r"^\d+\.\d+\.\d+$", t.version)]
        if self.check:
            if self.rustup_ready(init.version):
                self.report(f"rustup {init.version}", "ok")
            else:
                self.need(f"rustup {init.version}", f"missing, would fetch {init.url}")
        else:
            self.verify_kept(init)
            if self.rustup_ready(init.version):
                self.report(f"rustup {init.version}", "ok")
            else:
                file = self.fetch_verified(init)
                file.chmod(file.stat().st_mode | 0o111)
                run_tool([str(file), "-y", "--no-modify-path", "--default-toolchain", "none", "--profile", "minimal"], {**self.tool_env, "RUSTUP_INIT_SKIP_PATH_CHECK": "yes"})
                run_tool([str(self.rustup), "set", "auto-self-update", "disable"], self.tool_env)
                self.report(f"rustup {init.version}", "installed")
        for toolchain in toolchains:
            comps, tgts = (components, targets) if toolchain.version in numeric else ([], [])
            label = f"rust {toolchain.version}"
            if self.toolchain_ready(toolchain.version, comps, tgts):
                self.report(label, "ok")
            elif self.check:
                self.need(label, f"missing, rustup would fetch it with {' '.join(comps + tgts)}".rstrip())
            else:
                command = [str(self.rustup), "toolchain", "install", toolchain.version, "--profile", "minimal", "--no-self-update"]
                for c in comps:
                    command += ["-c", c]
                for t in tgts:
                    command += ["-t", t]
                run_tool(command, self.tool_env)
                self.report(label, "installed")
        if numeric and not self.check:
            code, out = capture_tool([str(self.rustup), "default"], self.tool_env)
            if not out.startswith(f"{numeric[0]}-{self.triple}"):
                run_tool([str(self.rustup), "default", f"{numeric[0]}-{self.triple}"], self.tool_env)
        for tool in [i for i in self.items if i.name.startswith("cargo-") and i.group in self.groups]:
            self.cargo_tool(tool, numeric[0] if numeric else None)
        self.paths.append(str(self.cargo_home / "bin"))

    def cargo_tool(self, tool: Item, stable: str | None) -> None:
        binary = self.cargo_home / "bin" / f"{tool.name}{self.exe}"
        code, out = capture_tool([str(binary), "--version"], self.tool_env) if binary.is_file() else (1, "")
        label = f"{tool.name} {tool.version}"
        if code == 0 and out.split()[1:2] == [tool.version]:
            self.report(label, "ok")
        elif self.check:
            self.need(label, "missing, cargo would compile it")
        else:
            if stable is None:
                raise Problem(f"{tool.name} needs a stable rust row to be installed with")
            run_tool([str(self.rustup), "run", stable, "cargo", "install", tool.name, "--version", tool.version, "--locked"], self.tool_env)
            self.report(label, "installed")

    # clang for ring on Windows ARM64
    def clang_dir(self) -> Path | None:
        """None when a clang is already on PATH or Visual Studio has one; else the locked LLVM's."""
        if shutil.which("clang") or "LLVM_DIR" in self.native:
            return None
        vswhere = Path(os.environ.get("ProgramFiles(x86)", "")) / "Microsoft Visual Studio" / "Installer" / "vswhere.exe"
        if vswhere.is_file():
            code, out = capture_tool([str(vswhere), "-products", "*", "-prerelease", "-requires", "Microsoft.VisualStudio.Component.VC.Llvm.Clang", "-property", "installationPath"])
            for line in out.splitlines():
                directory = Path(line.strip()) / "VC" / "Tools" / "Llvm" / "ARM64" / "bin"
                if (directory / "clang.exe").is_file():
                    return directory
        llvm = self.rows("llvm")
        return self.prefix / "llvm" / llvm[0].version / "bin" if llvm else None

    def llvm(self) -> None:
        rows = self.rows("llvm")
        if not rows or self.plat != "windows-arm64":
            return
        item = rows[0]
        directory = self.clang_dir()
        if directory is None or not str(directory).startswith(str(self.prefix / "llvm")):
            return
        label = f"llvm {item.version}"
        self.verify_kept(item)
        if (directory / "clang.exe").is_file():
            self.report(label, "ok")
        elif self.check:
            self.need(label, f"no clang found, would fetch {item.url}")
            return
        else:
            file = self.fetch_verified(item)
            partial = directory.parent.with_name(directory.parent.name + ".partial")
            shutil.rmtree(partial, ignore_errors=True)
            partial.mkdir(parents=True)
            unpack(file, partial, 1)
            shutil.rmtree(directory.parent, ignore_errors=True)
            os.replace(partial, directory.parent)
            self.report(label, "installed")
        self.paths.append(str(directory))

    # archives
    def archives(self) -> None:
        for item in self.items:
            if (
                item.group not in self.groups or item.group == "system" or not item.url.startswith("https://") or item.url.endswith(".git")
                or item.sha256 == "-" or item.name in ("rustup-init", "llvm") or item.name.startswith("cargo-")
            ):
                continue
            directory = self.prefix / item.name / item.version
            label = f"{item.name} {item.version}"
            self.dirs[f"{item.group} {item.name}"] = directory
            if directory.is_dir():
                self.verify_kept(item)
                self.report(label, "ok")
            elif self.check:
                self.need(label, f"missing, would fetch {item.url}")
                continue
            else:
                file = self.fetch_verified(item)
                partial = directory.with_name(directory.name + ".partial")
                shutil.rmtree(partial, ignore_errors=True)
                partial.mkdir(parents=True)
                unpack(file, partial, 1)
                os.replace(partial, directory)
                self.report(label, "installed")
            self.paths.append(str(directory / "bin" if (directory / "bin").is_dir() else directory))

    # git items
    def git_items(self) -> None:
        cfg = self.ctx.cfg.sdk
        for item in self.items:
            if item.group not in self.groups or item.group == "system" or not item.url.endswith(".git") or not item.url.startswith("https://"):
                continue
            url = os.environ.get(f"BUILD_SDK_GIT_URL_{re.sub(r'[^A-Za-z0-9]', '_', item.name).upper()}") or cfg.git_urls.get(item.name) or item.url
            directory = self.prefix / "git" / item.name / item.version[:12]
            self.dirs[f"{item.group} {item.name}"] = directory
            label = f"{item.name} {item.version[:12]}"
            ready = (directory / ".git").exists() and git_ok("rev-parse", "HEAD", cwd=directory) and run_git("rev-parse", "HEAD", cwd=directory) == item.version
            if ready:
                self.report(label, "ok")
            elif self.check:
                self.need(label, f"missing, would clone {url} at {item.version}")
                continue
            else:
                partial = directory.with_name(directory.name + ".partial")
                shutil.rmtree(partial, ignore_errors=True)
                partial.parent.mkdir(parents=True, exist_ok=True)
                self.report(label, f"cloning {url}")
                # A commit-only clone fetches just the trees and files of this revision, and keeps
                # the history a tool that reads tags and commits needs.
                run_git("-c", "core.autocrlf=false", "clone", "-q", "--filter=tree:0", "--no-checkout", url, str(partial))
                run_git("-C", str(partial), "checkout", "-q", "--detach", item.version)
                shutil.rmtree(directory, ignore_errors=True)
                os.replace(partial, directory)
                self.report(label, "installed")
            for post in cfg.post:
                if split_item(post.item) != (item.group, item.name):
                    continue
                stamp = directory / post.stamp
                if stamp.exists():
                    continue
                if self.check:
                    self.need(f"{item.name} {post.run[0]}", "has not run")
                    continue
                command = list(post.run)
                exe = directory / command[0]
                if self.windows:
                    exe = next((c for c in (exe, exe.with_suffix(".bat"), exe.with_suffix(".cmd"), exe.with_suffix(".exe")) if c.exists()), exe)
                run_tool([str(exe), *command[1:]], self.tool_env, cwd=directory)
                stamp.write_text("", encoding="utf-8")
                self.report(f"{item.name} {post.run[0]}", "ran")

    # Python virtual environments
    def venvs(self) -> None:
        for spec in self.ctx.cfg.sdk.venv:
            group, name = split_item(spec.item)
            rows = [i for i in self.items if i.group == group and i.name == name]
            if group not in self.groups or not rows:
                continue
            requirements = self.ctx.root / spec.requirements
            venv = self.prefix / "venv" / f"{name}-{rows[0].version}"
            scripts = venv / ("Scripts" if self.windows else "bin")
            copy = venv / "requirements.txt"
            label = f"{name} {rows[0].version}"
            if copy.is_file() and requirements.is_file() and copy.read_bytes() == requirements.read_bytes():
                self.report(label, "ok")
            elif self.check:
                self.need(label, "missing, pip would fetch what the requirements list, hash-checked")
                continue
            else:
                shutil.rmtree(venv, ignore_errors=True)
                venv.parent.mkdir(parents=True, exist_ok=True)
                run_tool([sys.executable, "-m", "venv", str(venv)])
                run_tool([str(scripts / f"python{self.exe}"), "-m", "pip", "install", "--quiet", "--disable-pip-version-check", "--require-hashes", "-r", str(requirements)])
                shutil.copyfile(requirements, copy)
                self.report(label, "installed")
            self.dirs[f"{group} {name}"] = venv
            self.paths.append(str(scripts))

    def docker_items(self) -> None:
        for item in self.items:
            if item.group in self.groups and item.url.startswith("docker://"):
                image = f"{item.url.removeprefix('docker://')}:{item.version}@sha256:{item.sha256}"
                self.report(f"{item.name} image", f"{image}, left to your container engine")

    # system items are only checked
    def system_found(self, name: str) -> bool:
        if name.startswith("pkg:"):
            return capture_tool(["pkg-config", "--exists", name[4:]])[0] == 0
        if name == "xcodebuild":
            return capture_tool(["xcodebuild", "-version"])[0] == 0
        if name == "docker-or-podman":
            return bool(shutil.which("docker") or shutil.which("podman"))
        if name == "python3":
            return bool(shutil.which("python3") or shutil.which("python") or shutil.which("py"))
        if name == "windows-sdk" and "WINDOWSSDKDIR" in self.native:
            return True
        if name.startswith("Microsoft.VisualStudio.") and "VSINSTALLDIR" in self.native:
            # A directory the user names replaces vswhere; the build proves which components it has.
            return True
        if name == "windows-sdk":
            kits = Path(os.environ.get("ProgramFiles(x86)", "C:/Program Files (x86)")) / "Windows Kits" / "10" / "Include"
            return kits.is_dir()
        if name.startswith("Microsoft.VisualStudio."):
            vswhere = Path(os.environ.get("ProgramFiles(x86)", "")) / "Microsoft Visual Studio" / "Installer" / "vswhere.exe"
            if not vswhere.is_file():
                return False
            code, out = capture_tool([str(vswhere), "-products", "*", "-prerelease", "-requires", name, "-property", "installationPath"])
            return bool(out.strip())
        return bool(shutil.which(name))

    def system(self) -> None:
        absent = False
        for item in self.items:
            if item.group != "system" or item.version not in self.groups:
                continue
            if self.system_found(item.name):
                self.report(f"system {item.name}", "found")
            else:
                absent = True
                self.report(f"system {item.name}", "MISSING; see --print-system")
        if absent:
            self.missing = self.missing or self.check
            print("build.py: some of the system group is missing. Nothing here installs it:", file=sys.stderr)
            print(system_text(self.ctx, self.plat), file=sys.stderr)

    # env.sh and env.ps1
    def expand(self, text: str) -> str:
        text = text.replace("{prefix}", str(self.prefix))
        return re.sub(r"\{dir:([^}]+)\}", lambda m: str(self.dirs.get(m[1].strip(), self.prefix / "missing" / m[1].strip())), text)

    def write_env(self) -> None:
        cfg = self.ctx.cfg.sdk
        variables = {"BUILD_SDK_HOME": str(self.prefix)}
        native, directories = native_environment(self.native)
        variables.update(native)
        if self.rustup.is_file():
            variables["RUSTUP_HOME"] = str(self.rustup_home)
            variables["CARGO_HOME"] = str(self.cargo_home)
        for key, value in cfg.env.items():
            variables[key] = self.expand(value)
        paths = directories + [self.expand(p) for p in cfg.path] + [p for p in reversed(self.paths)]
        paths = [p for p in dict.fromkeys(paths) if Path(p).exists()]
        for value in [*variables.values(), *paths]:
            if "'" in value:
                raise Problem(f"a value containing a single quote cannot be written to env.sh: {value}")
        sep = ";" if self.windows else ":"
        sh = ["# Written by tools/build.py. Source it: . " + str(self.prefix / "env.sh")]
        ps1 = ["# Written by tools/build.py. Source it: . " + str(self.prefix / "env.ps1")]
        for key, value in variables.items():
            sh.append(f"export {key}='{value}'")
            ps1.append(f"$env:{key} = '{value}'")
        if paths:
            sh.append("export PATH='" + ":".join(paths) + "':\"$PATH\"")
            ps1.append("$env:PATH = '" + sep.join(paths) + sep + "' + $env:PATH")
        for name, lines in (("env.sh", sh), ("env.ps1", ps1)):
            (self.prefix / f"{name}.partial").write_bytes(("\n".join(lines) + "\n").encode("utf-8"))
            os.replace(self.prefix / f"{name}.partial", self.prefix / name)
        self.report("env", f"{self.prefix / 'env.sh'}, {self.prefix / 'env.ps1'}")


def system_text(ctx: Ctx, plat: str) -> str:
    text = ctx.cfg.sdk.system.print.get(plat.split("-")[0])
    return text.rstrip("\n") if text else f"(no system package instructions are configured for {plat.split('-')[0]}; see [sdk.system] print)"


def sdk_install(ctx: Ctx, args: argparse.Namespace) -> int:
    cfg = ctx.cfg.sdk
    plat = args.platform or host_platform()
    if plat not in TRIPLES:
        raise Usage(f"unknown platform {plat}; one of {', '.join(sorted(TRIPLES))}")
    if args.print_system:
        print(system_text(ctx, plat))
        return 0
    groups = [g for g in (args.groups.split(",") if args.groups else cfg.install) if g]
    unknown = [g for g in groups if g not in cfg.groups]
    if unknown:
        raise Usage(f"unknown group {', '.join(unknown)}; the groups are {', '.join(cfg.groups)}")
    path = ctx.root / cfg.lock
    if not path.is_file():
        raise Problem(f"{cfg.lock}: no such file")
    items, errors = lint_lock(ctx, path.read_bytes(), path.name)
    if errors:
        raise Problem("\n".join(errors[:5]) + (f"\n({len(errors)} problems in all; run `sdk check`)" if len(errors) > 5 else ""))
    native = native_settings(ctx, os.environ, getattr(args, "env", None) or [])
    if args.prefix:
        prefix = Path(args.prefix)
    elif "SDK_HOME" in native:
        prefix = Path(native["SDK_HOME"][0])
    else:
        prefix = Path(os.environ["BUILD_SDK_HOME"]) if os.environ.get("BUILD_SDK_HOME") else Path.home() / f".{ctx.root.name}-sdk"
    prefix = prefix.resolve() if prefix.is_absolute() else (Path.cwd() / prefix).resolve()
    install = Installer(ctx, items, plat, prefix, groups + ["system"], args.check, native)
    if not args.check:
        (prefix / "dl").mkdir(parents=True, exist_ok=True)
    install.report("prefix", str(prefix) + (f" (SDK_HOME, {native['SDK_HOME'][1]})" if "SDK_HOME" in native and not args.prefix else ""))
    for key in NATIVE_VARIABLES:
        if key in native:
            install.report(key, f"{native[key][0]} ({native[key][1]})")
    install.report("platform", plat)
    install.rust()
    install.llvm()
    install.archives()
    install.git_items()
    install.venvs()
    install.docker_items()
    install.system()
    if args.check:
        return 1 if install.missing else 0
    install.write_env()
    return 0


def add_sdk(sub: argparse._SubParsersAction) -> None:
    parser = sub.add_parser("sdk", help="the SDK lock: lint it, hold the repository to it, install it")
    commands = parser.add_subparsers(dest="command", required=True, metavar="COMMAND")
    check = commands.add_parser("check", help="lint the lock, cover the deps pin, and compare each [[sdk.agree]] fact")
    check.add_argument("--online", action="store_true", help="also compare each download's hash with what its publisher says")
    check.add_argument("--deps-ref", metavar="REF", help="where @deps reads files (default: origin/ plus [deps] branch)")
    check.set_defaults(run=sdk_check)
    install = commands.add_parser("install", help="install what the lock names under one prefix, verified, by version")
    install.add_argument("--groups", metavar="A,B", help="the groups to install (default: [sdk] install)")
    install.add_argument("--prefix", metavar="DIR", help="where (default: SDK_HOME of `help env`, else $BUILD_SDK_HOME, else ~/.REPO-sdk)")
    install.add_argument("--check", action="store_true", help="install nothing: report what is missing and exit 1 if anything is")
    install.add_argument("--print-system", action="store_true", help="print the system packages to install, and nothing else")
    install.add_argument("--platform", metavar="OS-ARCH", help="act as this platform (default: this machine, or $BUILD_SDK_PLATFORM)")
    install.add_argument("--env", action="append", metavar="KEY=VALUE", help="set a native key of `help env` (SDK_HOME, VSINSTALLDIR, LLVM_DIR, ...); wins over the environment and the .env files")
    install.set_defaults(run=sdk_install)


SECTIONS.append(add_sdk)

# --- release: binaries, a universal macOS binary, container images -----------------------------------------------------

SCHEMA["release"] = Key(
    "table",
    None,
    "Release builds: one binary archive per target, a universal macOS binary, container images.",
    {
        "name": Key("str", "", "The name archives are made under; empty: the repository directory's."),
        "version_from": Key("str", "Cargo.toml", "The manifest whose [workspace.package] or [package] version names the release."),
        "toolchain": Key("str", "", "A rustup toolchain (for example 1.99.1) that cargo runs under; empty: the one on PATH."),
        "dist": Key("str", "dist", "Where archives and SHA256SUMS are written, relative to the repository."),
        "packages": Key(
            "tables",
            None,
            "What is built for each target.",
            {
                "package": Key("str", REQUIRED, "The cargo package."),
                "bin": Key("str", "", "The binary it builds; empty: the package's name."),
                "features": Key("strs", [], "Cargo features to enable."),
            },
        ),
        "target": Key(
            "tables",
            None,
            "A target, and the runner that builds it natively.",
            {
                "triple": Key("str", REQUIRED, "The Rust target triple."),
                "runner": Key("str", REQUIRED, "The CI runner label that builds it."),
                "rustflags": Key("strs", [], "Words of RUSTFLAGS (for example -C target-feature=+crt-static)."),
                "env": Key("map", {}, "Environment variables of the build; {triple} and {triple_} (hyphens as underscores) are replaced in names and values."),
                "archive": Key("str", "tar.gz", "`tar.gz` or `zip`."),
            },
        ),
        "universal": Key(
            "table",
            None,
            "A macOS binary for two architectures, joined with lipo.",
            {"macos": Key("strs", [], "The two Apple triples, both built on one runner.")},
        ),
        "image": Key(
            "table",
            None,
            "A container image, built natively on each architecture's runner.",
            {
                "file": Key("str", "", "The Containerfile, relative to the repository; empty: no image."),
                "context": Key("str", ".", "The build context, relative to the repository."),
                "name": Key("str", "", "The image name; empty: the release name."),
                "tool": Key("str", "", "`podman` or `docker`; empty: podman when found, else docker."),
                "from_deps": Key("bool", False, "Build as `container build` does (the deps branch as a build context, the .env keys), for a Containerfile made from the template."),
                "archs": Key("map", {}, "The architectures (amd64, arm64) and the runner that builds each."),
            },
        ),
    },
)


def release_name(ctx: Ctx) -> str:
    return ctx.cfg.release.name or ctx.root.name


def release_version(ctx: Ctx) -> str:
    """The release version: `[workspace.package] version`, else `[package] version`, of `version_from`."""
    path = ctx.root / ctx.cfg.release.version_from
    try:
        data = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise Problem(f"{ctx.cfg.release.version_from}: {error}") from error
    version = data.get("workspace", {}).get("package", {}).get("version") or data.get("package", {}).get("version")
    if not isinstance(version, str):
        raise Problem(f"{ctx.cfg.release.version_from} has no [workspace.package] or [package] version")
    return version


def target_of(ctx: Ctx, triple: str) -> Table:
    for target in ctx.cfg.release.target:
        if target.triple == triple:
            return target
    raise Problem(f"[[release.target]] has no {triple}; the targets are {', '.join(t.triple for t in ctx.cfg.release.target) or 'none'}")


def archive_name(ctx: Ctx, label: str, kind: str) -> str:
    if kind not in ("tar.gz", "zip"):
        raise Problem(f"archive {kind!r} is not tar.gz or zip")
    return f"{release_name(ctx)}-{release_version(ctx)}-{label}.{kind}"


def release_matrix(ctx: Ctx, args: argparse.Namespace) -> int:
    """The JSON a workflow's `fromJSON` reads: the runners come from the configuration alone."""
    cfg = ctx.cfg.release
    universal = [t for t in cfg.target if t.triple in cfg.universal.macos]
    plan = {
        "binaries": {"include": [{"target": t.triple, "runner": t.runner, "archive": t.archive} for t in cfg.target]},
        "universal": {"include": [{"name": "universal-apple-darwin", "runner": universal[0].runner, "targets": " ".join(cfg.universal.macos)}] if len(cfg.universal.macos) == 2 and universal else []},
        "images": {"include": [{"arch": arch, "runner": runner} for arch, runner in sorted(cfg.image.archs.items())] if cfg.image.file else []},
    }
    if len(cfg.universal.macos) not in (0, 2):
        raise Problem("[release.universal] macos names two triples, or none")
    for triple in cfg.universal.macos:
        target_of(ctx, triple)
    print(json.dumps(plan[args.section] if args.section else plan, indent=None if args.section else 2))
    return 0


def exe_suffix(triple: str) -> str:
    return ".exe" if "windows" in triple else ""


def update_sums(dist: Path, name: str, data: bytes) -> None:
    """Records an artifact in dist/SHA256SUMS (`HASH  NAME`, sorted), replacing an earlier line for it."""
    sums = dist / "SHA256SUMS"
    rows: dict[str, str] = {}
    if sums.is_file():
        for line in sums.read_text(encoding="utf-8").splitlines():
            digest, _, file = line.partition("  ")
            if digest and file:
                rows[file] = digest
    rows[name] = sha256(data)
    sums.write_bytes("".join(f"{rows[n]}  {n}\n" for n in sorted(rows)).encode("utf-8"))


def pack(dist: Path, name: str, folder: str, files: dict[str, Path], kind: str, executables: tuple[str, ...] = ()) -> Path:
    """An archive holding `folder/FILE` for each file, with no timestamps, owners or host paths, so
    the same inputs give the same bytes. The names in `executables` get the executable bit."""
    dist.mkdir(parents=True, exist_ok=True)
    out = dist / name
    if kind == "zip":
        with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as zf:
            for arc, path in sorted(files.items()):
                info = zipfile.ZipInfo(f"{folder}/{arc}", date_time=(1980, 1, 1, 0, 0, 0))
                info.compress_type = zipfile.ZIP_DEFLATED
                info.external_attr = (0o755 if arc in executables else 0o644) << 16
                zf.writestr(info, path.read_bytes())
    else:
        with open(out, "wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as zipped:
            with tarfile.open(fileobj=zipped, mode="w", format=tarfile.PAX_FORMAT) as tar:
                for arc, path in sorted(files.items()):
                    info = tarfile.TarInfo(f"{folder}/{arc}")
                    data = path.read_bytes()
                    info.size, info.mtime, info.mode = len(data), 0, 0o755 if arc in executables else 0o644
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    tar.addfile(info, io.BytesIO(data))
    return out


def extras(ctx: Ctx) -> dict[str, Path]:
    """The licence and readme files at the repository root, which ship beside the binaries."""
    return {p.name: p for p in sorted(ctx.root.iterdir()) if p.is_file() and re.match(r"^(LICENSE|LICENCE|COPYING|README)", p.name, re.IGNORECASE)}


def cargo_command(ctx: Ctx, config: Path | None, *args: str) -> list[str]:
    cfg = ctx.cfg.release
    return ["cargo", *([f"+{cfg.toolchain}"] if cfg.toolchain else []), *(["--config", str(config)] if config else []), *args]


def release_build(ctx: Ctx, args: argparse.Namespace) -> int:
    cfg = ctx.cfg.release
    target = target_of(ctx, args.target)
    if not cfg.packages:
        raise Problem("[[release.packages]] lists nothing to build")
    name = archive_name(ctx, target.triple, target.archive)
    config = None
    env = dict(os.environ)
    native = native_settings(ctx, os.environ, getattr(args, "env", None) or [])
    variables, directories = native_environment(native)
    env.update(variables)
    if directories:
        env["PATH"] = os.pathsep.join([*directories, env.get("PATH", "")])
    for key in NATIVE_KEYS:
        if key in native and key != "SDK_HOME":
            print(f"native: {key}={native[key][0]} ({native[key][1]})")
    if args.offline:
        lock_path = args.cargo_lock.resolve() if args.cargo_lock else ctx.root / ctx.cfg.deps.cargo_lock
        info = use_crates(ctx, find_deps(ctx, args.deps), read_lock(lock_path), lock_path, args.out)
        config = Path(info["config"])
        env["CARGO_NET_OFFLINE"] = "true"
    fill = lambda text: text.replace("{triple_}", target.triple.replace("-", "_")).replace("{triple}", target.triple)  # noqa: E731
    env.update({fill(k): fill(v) for k, v in target.env.items()})
    if target.rustflags:
        env["RUSTFLAGS"] = " ".join(target.rustflags)
    built: dict[str, Path] = {}
    for package in cfg.packages:
        command = cargo_command(ctx, config, "build", "--release", "--locked", "--target", target.triple, "-p", package.package)
        if package.bin:
            command += ["--bin", package.bin]
        if package.features:
            command += ["--features", ",".join(package.features)]
        run_tool(command, env, ctx.root)
        binary = (package.bin or package.package) + exe_suffix(target.triple)
        path = ctx.root / "target" / target.triple / "release" / binary
        if not path.is_file():
            raise Problem(f"cargo built no {path}")
        built[binary] = path
    folder = name.removesuffix("." + target.archive)
    dist = ctx.root / cfg.dist
    out = pack(dist, name, folder, {**extras(ctx), **built}, target.archive, tuple(built))
    update_sums(dist, name, out.read_bytes())
    print(f"built {out.relative_to(ctx.root).as_posix()}")
    return 0


def release_universal(ctx: Ctx, args: argparse.Namespace) -> int:
    """One macOS binary for both Apple architectures (lipo), packed as `universal-apple-darwin`."""
    cfg = ctx.cfg.release
    triples = cfg.universal.macos
    if len(triples) != 2:
        raise Problem("[release.universal] macos must name two triples")
    dist = ctx.root / cfg.dist
    name = archive_name(ctx, "universal-apple-darwin", "tar.gz")
    folder = name.removesuffix(".tar.gz")
    scratch = Path(tempfile.mkdtemp(prefix="build-universal-"))
    try:
        joined: dict[str, Path] = {}
        for package in cfg.packages:
            binary = package.bin or package.package
            parts = [ctx.root / "target" / t / "release" / binary for t in triples]
            for part in parts:
                if not part.is_file():
                    raise Problem(f"{part} is missing; build {part.parent.parent.name} first")
            out = scratch / binary
            run_tool(["lipo", "-create", "-output", str(out), *map(str, parts)])
            code, text = capture_tool(["lipo", "-info", str(out)])
            if code != 0 or "x86_64" not in text or "arm64" not in text:
                raise Problem(f"lipo -info of {binary} does not list both architectures: {text.strip()}")
            joined[binary] = out
        out = pack(dist, name, folder, {**extras(ctx), **joined}, "tar.gz", tuple(joined))
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    update_sums(dist, name, out.read_bytes())
    print(f"built {out.relative_to(ctx.root).as_posix()}")
    return 0


def release_image(ctx: Ctx, args: argparse.Namespace) -> int:
    cfg = ctx.cfg.release
    image = cfg.image
    if not image.file:
        raise Problem("[release.image] file is not set")
    if args.arch not in image.archs:
        raise Problem(f"[release.image] archs has no {args.arch}; it has {', '.join(sorted(image.archs)) or 'none'}")
    tool = image.tool or next((t for t in ("podman", "docker") if shutil.which(t)), "")
    if tool not in ("podman", "docker"):
        raise Problem("neither podman nor docker is installed")
    tag = f"{image.name or release_name(ctx)}:{release_version(ctx)}-{args.arch}"
    if image.from_deps:
        plan = container_plan(ctx, argparse.Namespace(env=[], tag=tag), "", os.environ, fetch=True, platform=f"linux/{args.arch}")
        tool = plan.engine
        if not shutil.which(tool):
            raise Problem(f"{tool} is not installed")
        run_tool(plan.command, {**os.environ, **plan.secrets}, ctx.root)
    else:
        run_tool([tool, "build", "--platform", f"linux/{args.arch}", "-f", str(ctx.root / image.file), "-t", tag, str(ctx.root / image.context)])
    dist = ctx.root / cfg.dist
    dist.mkdir(parents=True, exist_ok=True)
    out = dist / f"{release_name(ctx)}-{release_version(ctx)}-image-{args.arch}.tar"
    # podman writes an OCI archive; docker has no such format and writes its own.
    run_tool([tool, "save", *(["--format", "oci-archive"] if tool == "podman" else []), "-o", str(out), tag])
    update_sums(dist, out.name, out.read_bytes())
    print(f"built {out.relative_to(ctx.root).as_posix()}")
    return 0


def release_verify(ctx: Ctx, args: argparse.Namespace) -> int:
    """Each Linux binary in the archives is statically linked (`file` says so)."""
    cfg = ctx.cfg.release
    targets = [t for t in cfg.target if "linux" in t.triple and (not args.target or t.triple == args.target)]
    if args.target:
        target_of(ctx, args.target)
    if not targets:
        print("no Linux target to verify")
        return 0
    if not shutil.which("file"):
        raise Problem("the `file` command is not installed")
    dist = ctx.root / cfg.dist
    found = []
    for target in targets:
        archive = dist / archive_name(ctx, target.triple, target.archive)
        if not archive.is_file():
            found.append(f"{archive.relative_to(ctx.root).as_posix()}: missing; build {target.triple} first")
            continue
        scratch = Path(tempfile.mkdtemp(prefix="build-verify-"))
        try:
            if target.archive == "zip":
                with zipfile.ZipFile(archive) as zf:
                    safe_unzip(zf, scratch, 1)
            else:
                with tarfile.open(archive, "r:gz") as tar:
                    safe_extract(tar, scratch, strip=1)
            for package in cfg.packages:
                binary = scratch / (package.bin or package.package)
                code, text = capture_tool(["file", "--brief", str(binary)])
                if code != 0 or not re.search(r"statically linked|static-pie linked", text):
                    found.append(f"{target.triple}: {binary.name} is not statically linked: {text.strip() or 'file failed'}")
        finally:
            shutil.rmtree(scratch, ignore_errors=True)
    if found:
        raise Problem("\n".join(found))
    print(f"ok: {len(targets)} Linux target(s) statically linked")
    return 0


def add_release(sub: argparse._SubParsersAction) -> None:
    parser = sub.add_parser("release", help="release binaries, a universal macOS binary and container images")
    commands = parser.add_subparsers(dest="command", required=True, metavar="COMMAND")
    matrix = commands.add_parser("matrix", help="print the JSON a workflow's fromJSON reads: targets, runners, images")
    matrix.add_argument("--section", choices=("binaries", "universal", "images"), help="print only this section, on one line")
    matrix.set_defaults(run=release_matrix)
    build_ = commands.add_parser("build", help="cargo build --release --locked one target, pack it, and add it to SHA256SUMS")
    build_.add_argument("--target", required=True, metavar="TRIPLE", help="a [[release.target]] triple")
    build_.add_argument("--offline", action="store_true", help="build from the deps branch (`deps use`): no network")
    build_.add_argument("--deps", type=Path, metavar="DIR", help="the deps checkout for --offline (default: `deps path`)")
    build_.add_argument("--cargo-lock", type=Path, metavar="FILE", help="read this Cargo.lock for --offline")
    build_.add_argument("--out", type=Path, metavar="DIR", help="where --offline writes the cargo home")
    build_.add_argument("--env", action="append", metavar="KEY=VALUE", help="set a native key of `help env` (VSINSTALLDIR, LIBCLANG_PATH, ...); wins over the environment and the .env files")
    build_.set_defaults(run=release_build)
    universal = commands.add_parser("universal", help="join the two built Apple targets with lipo and pack the result")
    universal.set_defaults(run=release_universal)
    image = commands.add_parser("image", help="build the container image for one architecture and save it as an OCI archive")
    image.add_argument("--arch", required=True, metavar="ARCH", help="amd64 or arm64: a key of [release.image] archs")
    image.set_defaults(run=release_image)
    verify = commands.add_parser("verify", help="check that each Linux binary in dist/ is statically linked")
    verify.add_argument("--target", metavar="TRIPLE", help="only this target")
    verify.set_defaults(run=release_verify)


SECTIONS.append(add_release)

# --- container: builds from the deps branch, with .env overrides ---------------------------------------------------

ENV_FILES = (".env", ".env.local")
PINNED_IMAGE = re.compile(r"@sha256:[0-9a-f]{64}$")
CREDENTIALS = re.compile(r"(?P<scheme>[A-Za-z][A-Za-z0-9+.-]*://)[^/@\s]*@")


class EnvKey(NamedTuple):
    """One key of .env, .env.local, the process environment and --env.

    `kind` is url, str, image, toolchain, version, sha256, flag, engine, dir or file. A key that is `by_name` goes to
    the engine as `--build-arg KEY` with its value in the engine's environment, so the value is on no command line;
    so does any URL value that carries credentials. A key that is not `arg` is read by build.py alone.
    """

    kind: str
    doc: str
    by_name: bool = False
    arg: bool = True
    # Glob patterns, each of which must match something in a `dir`: what makes the directory the one meant.
    need: tuple[str, ...] = ()


# Keys for a build on this machine (`sdk install`, `release build`): where Visual Studio, the Windows SDK, clang and the SDK
# prefix are. The container build reports them when set and ignores them.
NATIVE_KEYS = ("VSINSTALLDIR", "VCTOOLSINSTALLDIR", "WINDOWSSDKDIR", "WINDOWSSDKVERSION", "LIBCLANG_PATH", "LLVM_DIR", "SDK_HOME")
NATIVE_VARIABLES = tuple(key for key in NATIVE_KEYS if key != "SDK_HOME")


ENV_KEYS: dict[str, EnvKey] = {
    "RUSTUP_DIST_SERVER": EnvKey("url", "Where rustup downloads toolchains from (rustup's own variable)."),
    "RUSTUP_UPDATE_ROOT": EnvKey("url", "Where rustup downloads its own updates from (rustup's own variable)."),
    "RUSTUP_INIT_BASE_URL": EnvKey("url", "Where rustup-init is fetched from, as BASE/VERSION/TRIPLE/rustup-init. The lock's hash is still checked."),
    "APT_MIRROR": EnvKey("url", "The package archive the SDK stage reads when it must install python3 (Debian's main archive only)."),
    "BASE_IMAGE": EnvKey("image", "The SDK stage's base image. Needs an @sha256: digest unless ALLOW_UNPINNED_IMAGE=1."),
    "RUNTIME_IMAGE": EnvKey("image", "The runtime stage's base image. Needs an @sha256: digest unless ALLOW_UNPINNED_IMAGE=1."),
    "ALLOW_UNPINNED_IMAGE": EnvKey("flag", "1 lets BASE_IMAGE and RUNTIME_IMAGE name an image without a digest; the run reports it.", arg=False),
    "RUST_VERSION": EnvKey("toolchain", "The toolchain the SDK stage installs: an exact version or nightly-YYYY-MM-DD."),
    "RUSTUP_VERSION": EnvKey("version", "The rustup-init version. Given with both RUSTUP_INIT_SHA256_* keys."),
    "RUSTUP_INIT_SHA256_X86_64": EnvKey("sha256", "The hash of the x86_64 rustup-init at RUSTUP_VERSION."),
    "RUSTUP_INIT_SHA256_AARCH64": EnvKey("sha256", "The hash of the aarch64 rustup-init at RUSTUP_VERSION."),
    "DEPS_DIR": EnvKey("dir", "A local checkout of the deps branch, used as it is (and checked against Cargo.lock) instead of fetching one.", arg=False),
    "DEPS_GIT_URL": EnvKey("str", "Where the deps commit is fetched from; default: the URL of the lock's deps row.", arg=False),
    "DEPS_REF": EnvKey("str", "A deps commit or ref other than the lock's; it is checked against Cargo.lock and reported.", arg=False),
    "HTTP_PROXY": EnvKey("url", "A proxy for the SDK stage's downloads (build argument only; the offline stages use none).", by_name=True),
    "HTTPS_PROXY": EnvKey("url", "A proxy for the SDK stage's https downloads.", by_name=True),
    "NO_PROXY": EnvKey("str", "Hosts the SDK stage reaches without the proxy."),
    "EXTRA_CA_CERT": EnvKey("file", "A PEM file added to the SDK stage's trust store, never to the runtime image."),
    "CONTAINER_ENGINE": EnvKey("engine", "podman or docker; default: [release.image] tool, else podman when found, else docker.", arg=False),
    "CONTAINER_PLATFORM": EnvKey("str", "The engine's --platform, for example linux/arm64.", arg=False),
    "MSVC_SPLAT_DIR": EnvKey(
        "dir",
        "An xwin-style splat of the MSVC CRT and the Windows SDK (crt/ and sdk/) for the Containerfile's cross stage, which mounts it read-only as the `msvc` build context. Microsoft licenses these files: they are never copied into an image layer that is kept or into the repository.",
        arg=False,
        need=("crt", "sdk"),
    ),
    "VSINSTALLDIR": EnvKey(
        "dir",
        "Native builds: the Visual Studio or Build Tools root, used instead of vswhere to find MSVC, and set for cargo (the cc crate reads it).",
        arg=False,
        need=("VC",),
    ),
    "VCTOOLSINSTALLDIR": EnvKey(
        "dir",
        "Native builds: the MSVC toolset directory (VC/Tools/MSVC/VERSION), set for cargo.",
        arg=False,
        need=("bin", "include", "lib"),
    ),
    "WINDOWSSDKDIR": EnvKey(
        "dir",
        "Native builds: the Windows SDK root (Windows Kits/10), used instead of the default location, and set for cargo.",
        arg=False,
        need=("Include", "Lib"),
    ),
    "WINDOWSSDKVERSION": EnvKey("sdkversion", "Native builds: the Windows SDK version (for example 10.0.26100.0), set for cargo; with WINDOWSSDKDIR it must be installed there.", arg=False),
    "LIBCLANG_PATH": EnvKey("dir", "Native builds: the directory of libclang (bindgen, clang-sys), set for cargo.", arg=False, need=("libclang.*",)),
    "LLVM_DIR": EnvKey(
        "dir",
        "Native builds: an LLVM installation (bin/clang): its bin comes first on PATH, `sdk install` fetches no LLVM, and LIBCLANG_PATH defaults to its bin when that holds libclang.",
        arg=False,
        need=("bin/clang*",),
    ),
    "SDK_HOME": EnvKey("home", "Native builds: where `sdk install` puts the SDKs (after --prefix, before $BUILD_SDK_HOME).", arg=False),
}


def hide_credentials(text: str) -> str:
    """The text with the user and password of every URL in it replaced."""
    return CREDENTIALS.sub(r"\g<scheme>***@", text)


def scrub(text: str, url: str) -> str:
    """The text without the user and password of the URL, also where git quotes them without the scheme."""
    found = CREDENTIALS.match(url)
    return text.replace(url[len(found["scheme"]) : found.end() - 1] + "@", "***@") if found else text


def parse_env_text(text: str, name: str) -> tuple[list[tuple[int, str, str]], list[str]]:
    """The (line, key, value) of an env file and one message for each line that is not KEY=VALUE.

    No interpolation: a value is the text after `=`, without a matching pair of quotes or a ` #` comment.
    """
    rows: list[tuple[int, str, str]] = []
    errors: list[str] = []
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, eq, value = line.removeprefix("export ").lstrip().partition("=")
        key, value = key.strip(), value.strip()
        if not eq or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key):
            errors.append(f"{name}:{number}: not KEY=VALUE")
            continue
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
            value = value[1:-1]
        else:
            value = re.sub(r"\s+#.*$", "", value)
        rows.append((number, key, value))
    return rows, errors


def read_env(ctx: Ctx, environ: Any, flags: list[str], only: tuple[str, ...] | None = None) -> dict[str, tuple[str, str]]:
    """{key: (value, where it came from)}: .env, then .env.local, then the process environment, then --env.

    A later layer wins. An unknown key in a file or a flag is refused, naming it; the environment is
    read for the known keys only. An empty value clears the key, which leaves the default.

    `only` is for a command that uses some keys (a native build): it reads those and leaves the rest of the
    files and flags alone, so a key of another tool in the same `.env` breaks nothing there.
    """
    values: dict[str, tuple[str, str]] = {}
    errors: list[str] = []
    wanted = ENV_KEYS if only is None else only
    for name in ENV_FILES:
        path = ctx.root / name
        if not path.is_file():
            continue
        rows, bad = parse_env_text(path.read_text(encoding="utf-8"), name)
        errors += bad if only is None else []
        for number, key, value in rows:
            if key in wanted:
                values[key] = (value, name)
            elif only is None:
                errors.append(f"{name}:{number}: unknown key {key}; `build.py help env` lists the keys")
    for key in wanted:
        if key in environ:
            values[key] = (environ[key], "environment")
    for flag in flags:
        key, eq, value = flag.partition("=")
        if eq and key in wanted:
            values[key] = (value, "--env")
        elif eq and only is None:
            errors.append(f"--env {key}: unknown key; `build.py help env` lists the keys")
        elif not eq:
            errors.append(f"--env {flag}: not KEY=VALUE")
    if errors:
        raise Problem("\n".join(errors))
    return {key: pair for key, pair in values.items() if pair[0] != ""}


def env_help() -> str:
    lines = []
    for key, spec in ENV_KEYS.items():
        how = "build argument" if spec.arg else "read by build.py"
        lines.append(f"  {key}  ({spec.kind}, {how}{', passed by name' if spec.by_name else ''})  {spec.doc}")
    head = f"Keys of {', '.join(ENV_FILES)}, the process environment and --env KEY=VALUE; a later one wins, an empty value clears a key."
    return "\n".join([head, *lines])


def env_problems(values: dict[str, tuple[str, str]]) -> list[str]:
    """What is wrong with the values themselves, each line naming the key and where it was set."""
    found: list[str] = []
    allowed = values.get("ALLOW_UNPINNED_IMAGE", ("", ""))[0] == "1"
    for key, (value, source) in values.items():
        kind, where = ENV_KEYS[key].kind, f"{key} ({source})"
        if kind == "url" and "://" not in value:
            found.append(f"{where}: not a URL")
        elif kind == "sha256" and not HEX64.match(value):
            found.append(f"{where}: not 64 lowercase hex characters")
        elif kind == "version" and not re.fullmatch(r"\d+\.\d+\.\d+", value):
            found.append(f"{where}: not an exact version (MAJOR.MINOR.PATCH): {value}")
        elif kind == "toolchain" and not re.fullmatch(r"\d+\.\d+\.\d+|nightly-\d{4}-\d{2}-\d{2}", value):
            found.append(f"{where}: not an exact version or nightly-YYYY-MM-DD: {value}")
        elif kind == "flag" and value != "1":
            found.append(f"{where}: must be 1, or left out")
        elif kind == "engine" and value not in ("podman", "docker"):
            found.append(f"{where}: must be podman or docker: {value}")
        elif kind == "dir" and not Path(value).is_dir():
            found.append(f"{where}: {value} is not a directory")
        elif kind == "dir" and (missing := [n for n in ENV_KEYS[key].need if not any(Path(value).glob(n))]):
            found.append(f"{where}: {value} holds no {', no '.join(missing)}, so it is not the directory meant")
        elif kind == "home" and Path(value).exists() and not Path(value).is_dir():
            found.append(f"{where}: {value} exists and is not a directory")
        elif kind == "sdkversion" and not re.fullmatch(r"\d+\.\d+\.\d+\.\d+[\\/]?", value):
            found.append(f"{where}: not a Windows SDK version (for example 10.0.26100.0): {value}")
        elif kind == "file" and not Path(value).is_file():
            found.append(f"{where}: {value} is not a file")
        elif kind == "image" and not PINNED_IMAGE.search(value) and not allowed:
            found.append(f"{where}: {hide_credentials(value)} has no @sha256: digest; pin it, or set ALLOW_UNPINNED_IMAGE=1 and accept that the build is not reproducible")
    if "WINDOWSSDKVERSION" in values and "WINDOWSSDKDIR" in values:
        version = values["WINDOWSSDKVERSION"][0].rstrip("\\/")
        if not (Path(values["WINDOWSSDKDIR"][0]) / "Lib" / version).is_dir():
            found.append(f"WINDOWSSDKVERSION ({values['WINDOWSSDKVERSION'][1]}): {version} is not installed under {values['WINDOWSSDKDIR'][0]} (WINDOWSSDKDIR, {values['WINDOWSSDKDIR'][1]})")
    if "DEPS_DIR" in values:
        found += [f"DEPS_DIR and {other} are exclusive" for other in ("DEPS_GIT_URL", "DEPS_REF") if other in values]
    versions = {"RUSTUP_VERSION", "RUSTUP_INIT_SHA256_X86_64", "RUSTUP_INIT_SHA256_AARCH64"}
    given = versions & set(values)
    if given and given != versions:
        found.append(f"{', '.join(sorted(given))} given without {', '.join(sorted(versions - given))}: a rustup version and both its hashes go together")
    return found


def native_settings(ctx: Ctx, environ: Any, flags: list[str]) -> dict[str, tuple[str, str]]:
    """The native keys (NATIVE_KEYS) of .env, .env.local, the environment and --env, each checked.

    Other keys of those files are left to `container`, so a wrong path fails here naming its key and where it was set."""
    values = read_env(ctx, environ, flags, only=NATIVE_KEYS)
    found = env_problems(values)
    if found:
        raise Problem("\n".join(found))
    return values


def native_environment(native: dict[str, tuple[str, str]]) -> tuple[dict[str, str], list[str]]:
    """(variables for cargo and the installed env files, directories to put first on PATH) for native keys.

    LLVM_DIR's bin comes first on PATH and is where libclang is looked for when LIBCLANG_PATH is not set."""
    variables = {key: native[key][0] for key in NATIVE_VARIABLES if key in native}
    directories: list[str] = []
    if "LLVM_DIR" in native:
        bin_dir = Path(native["LLVM_DIR"][0]) / "bin"
        directories.append(str(bin_dir))
        if "LIBCLANG_PATH" not in variables and any(bin_dir.glob("libclang.*")):
            variables["LIBCLANG_PATH"] = str(bin_dir)
    return variables, directories


SCHEMA["container"] = Key(
    "table",
    None,
    "`container build|test`: a Containerfile that builds from the deps branch, with .env overrides.",
    {
        "file": Key("str", "", "The Containerfile, relative to the repository; empty: [release.image] file."),
        "context": Key("str", "", "The build context, relative to the repository; empty: [release.image] context."),
        "name": Key("str", "", "The image name; empty: [release.image] name, else the release name."),
        "test_stage": Key("str", "test", "The stage `container test` builds."),
        "deps_item": Key("str", "deps crates", "The lock row whose version is the deps commit and whose url is where it is fetched from."),
        "rust_item": Key("str", "core rust", "The lock row whose first version is the default RUST_VERSION."),
        "rustup_item": Key("str", "core rustup-init", "The lock rows that are the default RUSTUP_VERSION and, for linux-x64 and linux-arm64, the default hashes."),
        "base_item": Key("str", "", "The lock row (url docker://NAME, a tag, a digest) of the SDK stage's base image: the default BASE_IMAGE; empty: not known."),
        "runtime_item": Key("str", "", "The lock row of the runtime stage's base image: the default RUNTIME_IMAGE; empty: not known."),
        "work_dir": Key("str", "target/container", "Where the deps clone and the empty CA directory go, relative to the repository."),
    },
)


def lock_rows(items: list[Item], spec: str, plat: str = "") -> list[Item]:
    group, name = split_item(spec)
    return [i for i in items if i.group == group and i.name == name and (not plat or i.platform in (plat, "any"))]


def container_defaults(ctx: Ctx, items: list[Item]) -> dict[str, str]:
    """What each key's value is when nothing sets it, from the lock; keys the lock does not decide are left out."""
    cfg = ctx.cfg.container
    defaults: dict[str, str] = {}
    rust = lock_rows(items, cfg.rust_item)
    if rust:
        defaults["RUST_VERSION"] = rust[0].version
    rustup = lock_rows(items, cfg.rustup_item)
    if rustup:
        defaults["RUSTUP_VERSION"] = rustup[0].version
    for key, plat in (("RUSTUP_INIT_SHA256_X86_64", "linux-x64"), ("RUSTUP_INIT_SHA256_AARCH64", "linux-arm64")):
        rows = lock_rows(items, cfg.rustup_item, plat)
        if rows:
            defaults[key] = rows[0].sha256
    for key, spec in (("BASE_IMAGE", cfg.base_item), ("RUNTIME_IMAGE", cfg.runtime_item)):
        rows = lock_rows(items, spec) if spec else []
        if rows and rows[0].url.startswith("docker://") and rows[0].sha256 != "-":
            defaults[key] = f"{rows[0].url.removeprefix('docker://')}:{rows[0].version}@sha256:{rows[0].sha256}"
    deps = lock_rows(items, cfg.deps_item)
    if deps and deps[0].url != "-":
        defaults["DEPS_GIT_URL"] = deps[0].url
    return defaults


def container_engine(ctx: Ctx, values: dict[str, tuple[str, str]]) -> str:
    if "CONTAINER_ENGINE" in values:
        return values["CONTAINER_ENGINE"][0]
    return ctx.cfg.release.image.tool or next((t for t in ("podman", "docker") if shutil.which(t)), "podman")


def head_of(path: Path) -> str:
    return run_git("rev-parse", "HEAD", cwd=path) if git_ok("rev-parse", "--verify", "-q", "HEAD", cwd=path) else "unknown"


def container_deps(ctx: Ctx, values: dict[str, tuple[str, str]], items: list[Item], fetch: bool, notes: list[str]) -> tuple[Path, str]:
    """(the deps checkout the build copies crates/ from, its commit).

    DEPS_DIR is used as it is. Otherwise the commit of the lock's deps row (or DEPS_REF) is fetched from that row's URL
    (or DEPS_GIT_URL) into the work directory; `fetch` false only says where. A checkout other than the lock's commit is
    read file by file against Cargo.lock; the lock's own commit is read by metadata, as `sdk check` covers it.
    """
    cfg = ctx.cfg.container
    lock = read_lock(ctx.root / ctx.cfg.deps.cargo_lock)
    if "DEPS_DIR" in values:
        deps = Path(values["DEPS_DIR"][0]).resolve()
        found, extra = dir_problems(ctx, deps, lock, True)
        notes += extra
        if found:
            raise Problem("\n".join(f"DEPS_DIR ({values['DEPS_DIR'][1]}): {line}" for line in found))
        return deps, head_of(deps)
    rows = lock_rows(items, cfg.deps_item)
    if len(rows) != 1:
        raise Problem(f"{ctx.cfg.sdk.lock}: expected one `{cfg.deps_item}` row to name the deps commit, found {len(rows)}; or set DEPS_DIR")
    url = values.get("DEPS_GIT_URL", (rows[0].url, ""))[0]
    ref = values.get("DEPS_REF", (rows[0].version, ""))[0]
    own = "DEPS_REF" not in values and "DEPS_GIT_URL" not in values
    key = sha256((url + "\0" + ref).encode())[:12]
    deps = ctx.root / cfg.work_dir / f"deps-{key}"
    if not fetch:
        if not own and HEX40.match(ref) and has_commit(ctx.root, ref):
            found, extra = ref_problems(ctx, lock, ref)
            notes += extra
            if found:
                raise Problem("\n".join(f"DEPS_REF: {line}" for line in found))
        else:
            notes.append(f"deps {ref[:12]} is fetched from {hide_credentials(url)} when the build runs; --print does not fetch it")
        return deps, ref
    deps.mkdir(parents=True, exist_ok=True)
    try:
        if not (deps / ".git").exists():
            run_git("init", "-q", cwd=deps)
        run_git("fetch", "-q", "--depth", "1", url, ref, cwd=deps)
        run_git("-c", "core.autocrlf=false", "checkout", "-q", "--detach", "FETCH_HEAD", cwd=deps)
    except Problem as problem:
        raise Problem(hide_credentials(scrub(str(problem), url))) from None
    commit = head_of(deps)
    if HEX40.match(ref) and commit != ref:
        raise Problem(f"{hide_credentials(url)}: fetching {ref} gave {commit}")
    found, extra = dir_problems(ctx, deps, lock, not own)
    notes += extra
    if found:
        raise Problem("\n".join(f"deps at {commit[:12]}: {line}" for line in found))
    return deps, commit


class ContainerPlan(NamedTuple):
    engine: str
    command: list[str]
    secrets: dict[str, str]
    non_default: list[tuple[str, str, str]]
    deps: Path
    commit: str
    notes: list[str]


def container_plan(ctx: Ctx, args: argparse.Namespace, stage: str, environ: Any, fetch: bool, platform: str = "") -> ContainerPlan:
    """The engine command for one build and what is reported about it; `fetch` false runs and writes nothing.

    `platform` is the engine's --platform and wins over CONTAINER_PLATFORM."""
    cfg, image = ctx.cfg.container, ctx.cfg.release.image
    file, context = cfg.file or image.file, cfg.context or image.context
    if not file:
        raise Problem("[container] file (or [release.image] file) is not set")
    values = read_env(ctx, environ, args.env or [])
    found = env_problems(values)
    if found:
        raise Problem("\n".join(found))
    lock_file = ctx.root / ctx.cfg.sdk.lock
    items = parse_lock_items(lock_file.read_text(encoding="utf-8"), lock_file.name) if lock_file.is_file() else []
    defaults = container_defaults(ctx, items)
    notes: list[str] = []
    deps, commit = container_deps(ctx, values, items, fetch, notes)
    for key in NATIVE_KEYS:
        if key in values:
            notes.append(f"{key} ({values[key][1]}) is for native builds (`sdk install`, `release build`); the container build ignores it")
    non_default = [
        (key, hide_credentials(values[key][0]), values[key][1])
        for key in ENV_KEYS
        if key in values and key not in NATIVE_KEYS and values[key][0] != defaults.get(key)
    ]
    for key in ("BASE_IMAGE", "RUNTIME_IMAGE"):
        if key in values and not PINNED_IMAGE.search(values[key][0]):
            notes.append(f"{key} has no digest (ALLOW_UNPINNED_IMAGE=1): the build is not reproducible")
    ca_file = Path(values["EXTRA_CA_CERT"][0]).resolve() if "EXTRA_CA_CERT" in values else None
    ca_dir = ca_file.parent if ca_file else ctx.root / cfg.work_dir / "ca-empty"
    if ca_file is None and fetch:
        ca_dir.mkdir(parents=True, exist_ok=True)
    # Always given, empty when MSVC_SPLAT_DIR is unset, so a cross stage built without it fails on its own message.
    msvc_dir = Path(values["MSVC_SPLAT_DIR"][0]).resolve() if "MSVC_SPLAT_DIR" in values else ctx.root / cfg.work_dir / "msvc-empty"
    if "MSVC_SPLAT_DIR" not in values and fetch:
        msvc_dir.mkdir(parents=True, exist_ok=True)
    build_args: list[str] = []
    secrets: dict[str, str] = {}
    for key, spec in ENV_KEYS.items():
        if key not in values or not spec.arg:
            continue
        value = ca_file.name if key == "EXTRA_CA_CERT" and ca_file else values[key][0]
        if spec.by_name or CREDENTIALS.search(value):
            build_args += ["--build-arg", key]
            secrets[key] = value
        else:
            build_args += ["--build-arg", f"{key}={value}"]
    labels = {"build.py.deps-commit": commit}
    if git_ok("rev-parse", "--verify", "-q", "HEAD", cwd=ctx.root):
        labels["org.opencontainers.image.revision"] = run_git("rev-parse", "HEAD", cwd=ctx.root)
    if non_default:
        labels["build.py.non-default"] = ",".join(key for key, _, _ in non_default)
    engine = container_engine(ctx, values)
    name = cfg.name or image.name or release_name(ctx)
    tag = args.tag or f"{name + '-test' if stage == cfg.test_stage else name}:{release_version(ctx)}"
    command = [engine, "build", "--file", str(ctx.root / file), *(["--target", stage] if stage else []), "--tag", tag]
    if platform or "CONTAINER_PLATFORM" in values:
        command += ["--platform", platform or values["CONTAINER_PLATFORM"][0]]
    command += ["--build-context", f"deps={deps}", "--build-context", f"ca={ca_dir}", "--build-context", f"msvc={msvc_dir}", *build_args]
    for label, text in labels.items():
        command += ["--label", f"{label}={text}"]
    command.append(str((ctx.root / context).resolve()))
    return ContainerPlan(engine, command, secrets, non_default, deps, commit, notes)


def container_run(ctx: Ctx, args: argparse.Namespace, stage: str) -> int:
    plan = container_plan(ctx, args, stage, os.environ, fetch=not args.print)
    if args.json:
        report = {
            "engine": plan.engine,
            "command": plan.command,
            "env": sorted(plan.secrets),
            "non_default": [{"key": k, "value": v, "source": s} for k, v, s in plan.non_default],
            "deps": {"dir": str(plan.deps), "commit": plan.commit},
            "notes": plan.notes,
        }
        print(json.dumps(report, indent=2))
    else:
        for key, value, source in plan.non_default:
            print(f"non-default: {key}={value} ({source})")
        if not plan.non_default:
            print("non-default: none")
        for note in plan.notes:
            print(f"note: {note}")
        print(f"deps: {plan.deps} at {plan.commit}")
        if args.print:
            print(f"engine: {plan.engine}; passed by name through the environment: {', '.join(sorted(plan.secrets)) or 'none'}")
            print(shlex.join(plan.command))
    if args.print:
        return 0
    if not shutil.which(plan.engine):
        raise Problem(f"{plan.engine} is not installed")
    run_tool(plan.command, {**os.environ, **plan.secrets}, ctx.root)
    return 0


def container_build(ctx: Ctx, args: argparse.Namespace) -> int:
    return container_run(ctx, args, args.target or "")


def container_test(ctx: Ctx, args: argparse.Namespace) -> int:
    return container_run(ctx, args, ctx.cfg.container.test_stage)


def add_container(sub: argparse._SubParsersAction) -> None:
    parser = sub.add_parser("container", help="build or test in a container: the deps branch is copied in, the build runs offline")
    commands = parser.add_subparsers(dest="command", required=True, metavar="COMMAND")
    for name, run, help_ in (
        ("build", container_build, "build the Containerfile (or --target STAGE) with the deps branch as the `deps` build context"),
        ("test", container_test, "build the test stage, which runs the workspace tests offline"),
    ):
        command = commands.add_parser(name, help=help_)
        if name == "build":
            command.add_argument("--target", metavar="STAGE", help="stop at this stage of the Containerfile (default: the last)")
        command.add_argument("--tag", metavar="T", help="the image tag (default: NAME:VERSION, NAME-test:VERSION for the test stage)")
        command.add_argument("--env", action="append", metavar="KEY=VALUE", help="set a key of `help env`; wins over the environment and the .env files")
        command.add_argument("--print", action="store_true", help="print the effective settings and the engine command; fetch nothing and run nothing")
        command.add_argument("--json", action="store_true", help="print the settings and the command as JSON")
        command.set_defaults(run=run)


SECTIONS.append(add_container)

# --- the other copied tools: public_api.py and apk.py read their own tables of this file ---------------------------

SCHEMA["public_api"] = Key(
    "table",
    None,
    "public_api.py: Rust public API snapshots.",
    {
        "packages": Key("strs", [], "The packages to snapshot."),
        "toolchain": Key("str", "", "A rustup toolchain, passed as `cargo +TOOLCHAIN`; empty adds none."),
        "sdk": Key("str", "", "`GROUP NAME`: the toolchain version is that row of the SDK lock; not with toolchain."),
        "layout": Key("str", "dir", "`dir`: PATH/PACKAGE.txt for each package; `file`: one file at PATH."),
        "path": Key("str", "", "The snapshot directory or file; empty: tools/public-api, or tools/public-api.txt for `file`."),
        "args": Key("strs", ["--all-features", "-sss", "--color", "never"], "Arguments of `cargo public-api` after the package."),
        "cargo": Key("str", "cargo", "The cargo executable."),
    },
)
SCHEMA["apk"] = Key(
    "table",
    None,
    "apk.py: checks of a built Android APK.",
    {
        "forbid": Key(
            "tables",
            None,
            "A pattern no entry of the APK may match; a hit is reported by its place, never by its text.",
            {
                "string": Key("str", REQUIRED, "A regular expression over the bytes of each entry."),
                "control_base64": Key("str", "", "Base64 of a sample the pattern must match: the check refuses to run when it does not (a check that cannot fail proves nothing)."),
                "name": Key("str", "", "A label for reports; empty: the index."),
            },
        ),
        "max_entry_bytes": Key("int", 268435456, "Entries larger than this are not scanned, and are listed."),
        "allow_flag": Key("str", "", "A command-line flag that reports forbidden hits without failing, for a build made on purpose (for example --allow-sideload-key)."),
        "permissions": Key(
            "table",
            None,
            "Permissions in the built manifest.",
            {"want": Key("strs", [], "Names that must be declared."), "unwanted": Key("strs", [], "Names that must not be.")},
        ),
        "abis": Key("strs", [], "The native ABIs the APK must ship, exactly; empty: not checked."),
        "libs": Key("strs", [], "Libraries every shipped ABI must carry."),
    },
)

# @@END-OF-SECTIONS@@


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog=SELF_NAME, description="One build tool for a repository.")
    parser.add_argument("--root", type=Path, help="the repository (default: the directory above this file's tools/)")
    parser.add_argument("--config", type=Path, metavar="FILE", help="read this configuration instead of ROOT/tools/build.cfg")
    sub = parser.add_subparsers(dest="section", required=True, metavar="SECTION")
    for add in SECTIONS:
        add(sub)
    return parser


def own_commit() -> str | None:
    """The commit line 3 of this file names, or None for the overlay's own copy (SOURCE)."""
    match = header_of(Path(__file__).read_bytes())
    return None if match is None or match["commit"] == b"SOURCE" else match["commit"].decode()


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        if args.root is None:
            if own_commit() is None and args.section != "help":
                parser.error("this is the overlay's own copy: pass --root DIR to name the repository")
            args.root = Path(__file__).resolve().parent.parent
        return args.run(Ctx(args.root.resolve(), args.config.resolve() if args.config else None), args)
    except Problem as problem:
        for line in str(problem).splitlines():
            print(f"{SELF_NAME}: {line}", file=sys.stderr)
        return 1
    except Usage as usage:
        parser.error(str(usage))
        return 2


if __name__ == "__main__":
    sys.exit(main())
