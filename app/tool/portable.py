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
from typing import NamedTuple
import urllib.request

APP_PROGRAM = "CanoaUI"

# appimagetool and the AppImage runtime it embeds are programs, so the deps branch cannot carry them: they are toolchain,
# named with their digests by the `app appimagetool` and `app appimage-runtime` rows of tools/sdk.lock, put in
# target/appimage-tools by `python app/tool/portable.py fetch` (the only code here that uses the network) and checked against
# those digests by every build, which never fetches.
LOCK = Path("tools") / "sdk.lock"
LINUX_PLATFORMS = {"x86_64": "linux-x64", "aarch64": "linux-arm64"}


class Pin(NamedTuple):
    name: str
    sha256: str
    url: str


def appimage_pins(root: Path, arch: str) -> tuple[Pin, Pin]:
    """The pinned appimagetool and runtime for `arch`, from the SDK lock's rows `GROUP NAME VERSION PLATFORM SHA256 URL`."""
    found: dict[str, Pin] = {}
    for line in (root / LOCK).read_text(encoding="utf-8").splitlines():
        row = line.split()
        if len(row) == 6 and row[0] == "app" and row[1] in ("appimagetool", "appimage-runtime") and row[3] == LINUX_PLATFORMS.get(arch):
            found[row[1]] = Pin(row[5].rsplit("/", 1)[-1], row[4], row[5])
    if len(found) != 2:
        raise RuntimeError(f"no pinned appimagetool and runtime for {arch} in {LOCK}")
    return found["appimagetool"], found["appimage-runtime"]


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


def pinned_tool(url: str, sha256: str, dest: Path) -> Path:
    """`dest` when it is the pinned file; an error naming how to get it when it is missing or differs. Never downloads."""
    if not dest.is_file() or sha256_file(dest) != sha256:
        raise RuntimeError(
            f"{dest} is not the pinned file (SHA-256 {sha256}, from {url}): "
            f"install it with `python app/tool/portable.py fetch`, the one step that uses the network")
    return dest


def fetch_tools(root: Path, arch: str | None = None) -> list[Path]:
    """Download appimagetool and the runtime for `arch` into target/appimage-tools, each checked against its pin."""
    tools = root / "target" / "appimage-tools"
    return [fetch_pinned(pin.url, pin.sha256, tools / pin.name) for pin in appimage_pins(root, arch or machine())]


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
    # Cargo reads the launcher's static-runtime setting (crates/canoa-launch/.cargo/config.toml) only from that folder.
    crate = root / "crates" / "canoa-launch"
    subprocess.run(
        [cargo, "run", "--release", "--locked", "-p", "canoa-launch", "--bin", "canoa-pack", "--",
         str(folder), version, str(payload)],
        check=True, cwd=crate,
    )
    env = dict(os.environ, CANOA_LAUNCH_PAYLOAD=str(payload))
    subprocess.run(
        [cargo, "build", "--release", "--locked", "-p", "canoa-launch", "--bin", "canoa-launch"],
        check=True, cwd=crate, env=env,
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
    """Make out_dir/Canoa-<arch>.AppImage from Flutter's Linux release bundle, with appimagetool installed by `fetch`."""
    arch = arch or machine()
    tools = root / "target" / "appimage-tools"
    tool_pin, runtime_pin = appimage_pins(root, arch)
    tool = pinned_tool(tool_pin.url, tool_pin.sha256, tools / tool_pin.name)
    runtime = pinned_tool(runtime_pin.url, runtime_pin.sha256, tools / runtime_pin.name)
    tool.chmod(0o755)
    appdir = root / "target" / "appimage" / "Canoa.AppDir"
    make_appdir(bundle, appdir, root / "app" / "assets" / "icons" / "canoa.svg")
    out_dir.mkdir(parents=True, exist_ok=True)
    image = out_dir / f"Canoa-{arch}.AppImage"
    image.unlink(missing_ok=True)
    # appimagetool is an AppImage itself; running it extracted needs no FUSE. Its scratch space is ours alone, not /tmp.
    private = root / "target" / "appimage-tmp"
    private.mkdir(parents=True, exist_ok=True)
    private.chmod(0o700)
    env = dict(os.environ, ARCH=arch, APPIMAGE_EXTRACT_AND_RUN="1", TMPDIR=str(private))
    subprocess.run(
        [str(tool), "--no-appstream", "--runtime-file", str(runtime), str(appdir), str(image)],
        check=True, env=env,
    )
    return image


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description="Install the pinned AppImage tools a Linux `--portable` build reads.")
    parser.add_argument("command", choices=("fetch",))
    parser.add_argument("--arch", choices=sorted(LINUX_PLATFORMS), help="default: this machine's")
    options = parser.parse_args()
    try:
        for installed in fetch_tools(Path(__file__).resolve().parents[2], options.arch):
            print(installed)
    except (RuntimeError, OSError) as error:
        raise SystemExit(f"portable.py: {error}")
