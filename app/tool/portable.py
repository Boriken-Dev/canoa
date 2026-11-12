"""One file per desktop platform: the Windows launcher around the built app folder, and the Linux AppImage.

`build.py desktop --portable` calls these after the Release build. macOS needs nothing: Canoa.app is already one bundle.
Nothing here writes into the repository's tracked files; the archive and the executables are build output under target/.
"""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tomllib
import urllib.request

APP_PROGRAM = "CanoaUI"

# appimagetool and the AppImage runtime it embeds, downloaded by a build and checked against these digests (a GitHub
# release's own, confirmed by hashing the download). Bumping either is a deliberate edit of this table.
APPIMAGETOOL_RELEASE = "https://github.com/AppImage/appimagetool/releases/download/1.9.1"
APPIMAGETOOL = {
    "x86_64": ("appimagetool-x86_64.AppImage", "ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"),
    "aarch64": ("appimagetool-aarch64.AppImage", "f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158"),
}
RUNTIME_RELEASE = "https://github.com/AppImage/type2-runtime/releases/download/20251108"
RUNTIME = {
    "x86_64": ("runtime-x86_64", "2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"),
    "aarch64": ("runtime-aarch64", "00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444"),
}

DESKTOP_FILE = """[Desktop Entry]
Type=Application
Name=Canoa
Comment=Chat with coding agents, on this computer or another
Exec=CanoaUI
Icon=canoa
Categories=Development;Utility;
Terminal=false
"""

# The bundle's executable finds its libraries through an $ORIGIN rpath, so this only has to run it from its own folder.
APP_RUN = """#!/bin/sh
here="$(dirname "$(readlink -f "$0")")"
exec "$here/usr/bin/CanoaUI" "$@"
"""


def workspace_version(root: Path) -> str:
    """The version the Cargo workspace declares, which names the unpacked folder."""
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    return manifest["workspace"]["package"]["version"]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def fetch_pinned(url: str, sha256: str, dest: Path) -> Path:
    """Download `url` to `dest` unless a file with that SHA-256 is already there; a download that differs is not kept."""
    if dest.is_file() and sha256_file(dest) == sha256:
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    partial = dest.with_name(dest.name + ".part")
    try:
        with urllib.request.urlopen(url, timeout=120) as response, partial.open("wb") as out:  # noqa: S310 (https, pinned)
            shutil.copyfileobj(response, out)
        got = sha256_file(partial)
        if got != sha256:
            raise RuntimeError(f"{url} has SHA-256 {got}, not the pinned {sha256}")
        os.replace(partial, dest)
    finally:
        partial.unlink(missing_ok=True)
    return dest


def cargo_program() -> str:
    cargo = shutil.which("cargo")
    if cargo is None:
        raise RuntimeError("cargo is not on PATH (the launcher is a Rust program)")
    return cargo


def build_windows_launcher(folder: Path, root: Path, out_dir: Path, version: str | None = None) -> Path:
    """Pack the built Release `folder`, build the launcher with the archive inside, and put it at out_dir/CanoaUI.exe.

    The archive goes in target/, never the tree. Returns the portable executable.
    """
    if not (folder / f"{APP_PROGRAM}.exe").is_file():
        raise RuntimeError(f"{folder} holds no {APP_PROGRAM}.exe")
    version = version or workspace_version(root)
    cargo = cargo_program()
    work = root / "target" / "portable"
    work.mkdir(parents=True, exist_ok=True)
    payload = work / "payload.zip"
    subprocess.run(
        [cargo, "run", "--release", "--locked", "-p", "canoa-launch", "--bin", "canoa-pack", "--",
         str(folder), version, str(payload)],
        check=True, cwd=root,
    )
    env = dict(os.environ, CANOA_LAUNCH_PAYLOAD=str(payload))
    subprocess.run(
        [cargo, "build", "--release", "--locked", "-p", "canoa-launch", "--bin", "canoa-launch"],
        check=True, cwd=root, env=env,
    )
    built = root / "target" / "release" / "canoa-launch.exe"
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True)
    exe = out_dir / f"{APP_PROGRAM}.exe"
    shutil.copyfile(built, exe)
    return exe


def make_appdir(bundle: Path, appdir: Path, icon: Path) -> None:
    """Lay Flutter's Linux bundle out as an AppDir: the bundle under usr/bin, AppRun, the desktop file and the icon."""
    if not (bundle / APP_PROGRAM).is_file():
        raise RuntimeError(f"{bundle} holds no {APP_PROGRAM}")
    if appdir.exists():
        shutil.rmtree(appdir)
    shutil.copytree(bundle, appdir / "usr" / "bin", symlinks=True)
    (appdir / "AppRun").write_bytes(APP_RUN.encode())
    (appdir / "AppRun").chmod(0o755)
    (appdir / "canoa.desktop").write_bytes(DESKTOP_FILE.encode())
    shutil.copyfile(icon, appdir / f"canoa{icon.suffix}")


def machine() -> str:
    import platform

    arch = platform.machine().lower()
    return {"amd64": "x86_64", "x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(arch, arch)


def build_appimage(bundle: Path, root: Path, out_dir: Path, arch: str | None = None) -> Path:
    """Make out_dir/Canoa-<arch>.AppImage from Flutter's Linux release bundle, with appimagetool fetched by digest."""
    arch = arch or machine()
    if arch not in APPIMAGETOOL:
        raise RuntimeError(f"no pinned appimagetool for {arch}")
    tools = root / "target" / "appimage-tools"
    tool_name, tool_sha = APPIMAGETOOL[arch]
    runtime_name, runtime_sha = RUNTIME[arch]
    tool = fetch_pinned(f"{APPIMAGETOOL_RELEASE}/{tool_name}", tool_sha, tools / tool_name)
    runtime = fetch_pinned(f"{RUNTIME_RELEASE}/{runtime_name}", runtime_sha, tools / runtime_name)
    tool.chmod(0o755)
    appdir = root / "target" / "appimage" / "Canoa.AppDir"
    make_appdir(bundle, appdir, root / "app" / "assets" / "icons" / "canoa.svg")
    out_dir.mkdir(parents=True, exist_ok=True)
    image = out_dir / f"Canoa-{arch}.AppImage"
    image.unlink(missing_ok=True)
    # appimagetool is an AppImage itself; running it extracted needs no FUSE.
    env = dict(os.environ, ARCH=arch, APPIMAGE_EXTRACT_AND_RUN="1")
    subprocess.run(
        [str(tool), "--no-appstream", "--runtime-file", str(runtime), str(appdir), str(image)],
        check=True, env=env,
    )
    return image
