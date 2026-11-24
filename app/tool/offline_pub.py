"""Where build.py finds the SDK and the pub cache, and whether pub can resolve from the deps branch.

The deps branch mirrors the hosted packages, every crate the locks name (crates/ and its siblings), and the app's git
packages as source trees under pub/git/<repository>-<revision>/. Pub finds a git package whose checkout folder is in
its cache without cloning, but it also wants a bare repository of the remote in which the locked revision resolves, and
git's packs cannot be carried. Each checkout is therefore seeded with a bare repository that holds one empty commit and a
replace ref from the locked revision to it: `git rev-list <revision>` succeeds, pub fetches nothing, and the lockfile is
resolved unchanged with `--enforce-lockfile`. The bare repository is only an answer to that question: the sources are the
checkout's.
"""
from __future__ import annotations

import hashlib
import os
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping, MutableMapping

# The only keys read from .env and from the environment.
KEYS = ("FLUTTER_ROOT", "FLUTTER", "PUB_CACHE", "CANOA_DEPS", "CARGO_HOME", "RUSTUP_HOME")
HOSTED_URL = "https://pub.dev"
HOSTED_DIR = "pub.dev"

_LINE = re.compile(r"^(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$")
_VAR = re.compile(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}")


def parse_env(text: str, environ: Mapping[str, str]) -> dict[str, str]:
    """KEY=VALUE lines of a .env file, for the known keys only; `${VAR}` is the process environment's, nothing else."""
    values: dict[str, str] = {}
    for raw in text.splitlines():
        match = _LINE.match(raw.strip())
        if match is None or match.group(1) not in KEYS:
            continue
        key, value = match.group(1), match.group(2).strip()
        if value[:1] in ("'", '"') and value.count(value[0]) >= 2:
            quote = value[0]
            value = value[1:value.index(quote, 1)]
            if quote == "'":
                values[key] = value
                continue
        else:
            value = re.split(r"\s+#", value, maxsplit=1)[0].strip()
        values[key] = _VAR.sub(lambda m: environ.get(m.group(1), ""), value)
    return values


def load_settings(root: Path, environ: Mapping[str, str]) -> dict[str, str]:
    """The repo's .env, with the process environment winning over it."""
    path = root / ".env"
    settings = parse_env(path.read_text(encoding="utf-8"), environ) if path.is_file() else {}
    settings.update({key: environ[key] for key in KEYS if environ.get(key)})
    return settings


def apply_settings(settings: Mapping[str, str], environ: MutableMapping[str, str]) -> None:
    """Make the settings the environment every tool this script starts sees; SDK and cargo bins go first on PATH."""
    for key in ("PUB_CACHE", "CARGO_HOME", "RUSTUP_HOME"):
        if key in settings:
            environ[key] = settings[key]
    bins = []
    flutter = settings.get("FLUTTER_ROOT") or settings.get("FLUTTER")
    if flutter:
        bins.append(str(Path(flutter) / "bin"))
    if "CARGO_HOME" in settings:
        bins.append(str(Path(settings["CARGO_HOME"]) / "bin"))
    if bins:
        environ["PATH"] = os.pathsep.join([*bins, environ.get("PATH", "")])


@dataclass(frozen=True)
class Package:
    name: str
    source: str = ""
    version: str = ""
    sha256: str = ""
    url: str = ""
    resolved_ref: str = ""

    @property
    def repo(self) -> str:
        """The name pub gives a git dependency's folders in its cache."""
        return self.url.rstrip("/").rsplit("/", 1)[-1].removesuffix(".git")


def parse_lock(text: str) -> list[Package]:
    """The packages of a pubspec.lock, read by indentation (the file's shape is fixed by pub)."""
    packages: list[Package] = []
    fields: dict[str, str] = {}
    name = ""

    def flush() -> None:
        if name:
            packages.append(Package(
                name, fields.get("source", ""), fields.get("version", ""), fields.get("sha256", ""),
                fields.get("url", ""), fields.get("resolved-ref", ""),
            ))

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


@dataclass(frozen=True)
class Plan:
    cache: Path | None  # the seeded pub cache to build with, or None when the build goes online
    reason: str | None  # why it cannot go offline
    cargo_home: Path | None = None  # the cargo home whose config builds from deps' crates/, when Cargo.lock is covered


def _mirror_crates():
    """tools/mirror_crates.py, the one place that knows what a crates/ must hold (tools/ is not a package)."""
    tools = str(Path(__file__).resolve().parents[2] / "tools")
    if tools not in sys.path:
        sys.path.insert(0, tools)
    import mirror_crates

    return mirror_crates


def crates_plan(cargo_lock: Path, deps: Path, out: Path | None = None) -> tuple[Path | None, str | None]:
    """A cargo home that builds from deps/crates, or the reason deps/crates does not cover Cargo.lock."""
    crates = _mirror_crates()
    try:
        lock = crates.read_lock(cargo_lock)
        found = crates.problems(lock, crates.read_dir(deps / "crates", deep=False), deep=False)
        if found:
            more = f" (and {len(found) - 1} more)" if len(found) > 1 else ""
            return None, f"the deps branch's crates/ does not cover Cargo.lock: {found[0]}{more}"
        return crates.use(deps, lock, cargo_lock.read_bytes(), out), None
    except crates.Problem as problem:
        return None, f"the deps branch's crates/ does not cover Cargo.lock: {problem}"


def default_pub_cache(environ: Mapping[str, str]) -> Path:
    if environ.get("PUB_CACHE"):
        return Path(environ["PUB_CACHE"])
    if os.name == "nt":
        return Path(environ.get("LOCALAPPDATA", str(Path.home() / "AppData" / "Local"))) / "Pub" / "Cache"
    return Path.home() / ".pub-cache"


def _git(root: Path, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True)


def find_deps(root: Path, settings: Mapping[str, str]) -> Path | None:
    """CANOA_DEPS, else a detached worktree of the deps branch beside the repository, added when the branch is local."""
    if settings.get("CANOA_DEPS"):
        path = Path(settings["CANOA_DEPS"])
        return path if (path / "pubspec.lock").is_file() else None
    ref = next((r for r in ("origin/deps", "deps") if _git(root, "rev-parse", "--verify", "-q", r).returncode == 0), None)
    if ref is None:
        return None
    path = root.with_name(root.name + "-deps")
    if path.is_dir():
        # Move an earlier worktree to the branch's tip; its files are only ever read.
        _git(path, "checkout", "-q", "--detach", ref)
    elif _git(root, "worktree", "add", "--detach", str(path), ref).returncode != 0:
        return None
    return path if (path / "pubspec.lock").is_file() else None


def _copy_missing(source: Path, target: Path) -> None:
    if source.is_dir() and not target.exists():
        shutil.copytree(source, target)
    elif source.is_file() and not target.exists():
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)


def _git_source(package: Package, deps: Path, user_cache: Path) -> Path | None:
    """The folder holding a git package's checkout: the deps branch's, else one already in the user's pub cache."""
    name = f"{package.repo}-{package.resolved_ref}"
    return next((d for d in (deps / "pub" / "git" / name, user_cache / "git" / name) if d.is_dir()), None)


def _seed_bare(target: Path, package: Package) -> None:
    """The bare repository pub keeps for a remote, in which the package's locked revision resolves (see the top)."""
    bare = target / "git" / "cache" / f"{package.repo}-{hashlib.sha1(package.url.encode()).hexdigest()}"
    if bare.exists():
        return
    identity = dict(os.environ, GIT_AUTHOR_NAME="canoa", GIT_AUTHOR_EMAIL="canoa@localhost",
                    GIT_COMMITTER_NAME="canoa", GIT_COMMITTER_EMAIL="canoa@localhost")

    def git(*args: str) -> str:
        done = subprocess.run(["git", "-C", str(bare), *args], capture_output=True, text=True, env=identity, input="")
        if done.returncode != 0:
            raise RuntimeError(f"git {' '.join(args)} in {bare}: {done.stderr.strip()}")
        return done.stdout.strip()

    bare.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["git", "init", "--bare", "--quiet", str(bare)], check=True)
    empty = git("hash-object", "-t", "tree", "-w", "--stdin")
    git("update-ref", f"refs/replace/{package.resolved_ref}", git("commit-tree", empty, "-m", "seed"))


def _list_packages(checkout: Path) -> None:
    """The file pub reads in a git checkout to learn which folders hold packages (it writes it after cloning).

    A checkout carried as a source tree has no .git, so the list is made from the pubspec.yaml files in it.
    """
    listing = checkout / ".git" / "pub-packages"
    if listing.exists():
        return
    folders = sorted(p.parent.relative_to(checkout).as_posix() for p in checkout.rglob("pubspec.yaml"))
    listing.parent.mkdir(exist_ok=True)
    listing.write_text("\n".join(folders), encoding="utf-8", newline="")


def plan_from(
    app_lock: str, deps: Path | None, user_cache: Path, target: Path,
    cargo_lock: Path | None = None, cargo_home: Path | None = None,
) -> Plan:
    """Check the deps checkout against the app's lock (and Cargo.lock) and seed `target` with what pub needs.

    The user's pub cache is only read.
    """
    if deps is None:
        return Plan(None, "no deps checkout (set CANOA_DEPS, or fetch the deps branch)")
    deps_lock = deps / "pubspec.lock"
    wanted = [p for p in parse_lock(app_lock) if p.source in ("hosted", "git")]
    have = {p.name: p for p in parse_lock(deps_lock.read_text(encoding="utf-8"))} if deps_lock.is_file() else {}
    hosted = [p for p in wanted if p.source == "hosted"]
    differs = [p.name for p in hosted
               if (q := have.get(p.name)) is None or (q.version, q.sha256) != (p.version, p.sha256)]
    if differs or any(p.url != HOSTED_URL for p in hosted):
        names = ", ".join(sorted(differs)[:5]) + (" ..." if len(differs) > 5 else "")
        return Plan(None, f"the deps branch's pubspec.lock differs from app/pubspec.lock ({names or 'a non-default host'})")
    mirror = deps / "pub" / "hosted" / HOSTED_DIR
    absent = [f"{p.name}-{p.version}" for p in hosted if not (mirror / f"{p.name}-{p.version}").is_dir()]
    if absent:
        return Plan(None, f"the deps branch lacks hosted packages: {', '.join(absent[:5])}")
    git = [p for p in wanted if p.source == "git"]
    sources = {p.name: _git_source(p, deps, user_cache) for p in git}
    missing = [p for p in git if sources[p.name] is None]
    if missing:
        listing = ", ".join(f"{p.name} {p.resolved_ref[:12]}" for p in missing)
        return Plan(None, f"git dependencies are neither on the deps branch (pub/git/) nor in the pub cache {user_cache}: {listing}")
    home = None
    if cargo_lock is not None:
        home, reason = crates_plan(cargo_lock, deps, cargo_home)
        if reason:
            return Plan(None, reason)
    for p in hosted:
        _copy_missing(mirror / f"{p.name}-{p.version}", target / "hosted" / HOSTED_DIR / f"{p.name}-{p.version}")
        _copy_missing(mirror / ".cache" / f"{p.name}-versions.json",
                      target / "hosted" / HOSTED_DIR / ".cache" / f"{p.name}-versions.json")
        _copy_missing(deps / "pub" / "hosted-hashes" / HOSTED_DIR / f"{p.name}-{p.version}.sha256",
                      target / "hosted-hashes" / HOSTED_DIR / f"{p.name}-{p.version}.sha256")
    for p in git:
        source = sources[p.name]
        checkout = target / "git" / f"{p.repo}-{p.resolved_ref}"
        _copy_missing(source, checkout)
        if source.parent == deps / "pub" / "git":
            _list_packages(checkout)
            _seed_bare(target, p)
        if source.parent == user_cache / "git":
            for bare in (user_cache / "git" / "cache").glob(f"{p.repo}-*"):
                _copy_missing(bare, target / "git" / "cache" / bare.name)
    return Plan(target, None, home)


def plan(root: Path, settings: Mapping[str, str], environ: Mapping[str, str], lock: Path) -> Plan:
    """The offline plan for the app's pub packages and the workspace's crates (the repository's Cargo.lock)."""
    deps = find_deps(root, settings)
    cache = default_pub_cache(settings) if "PUB_CACHE" in settings else default_pub_cache(environ)
    return plan_from(lock.read_text(encoding="utf-8"), deps, cache, root / "target" / "offline-pub-cache",
                     cargo_lock=root / "Cargo.lock")
