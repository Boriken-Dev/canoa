#!/usr/bin/env python3
"""Build the Canoa Flutter UI for the PWA, an extension or the desktop."""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys

import offline_pub

# Not resolve(): a drive made with `subst` (how a long checkout path stays under MSBuild's limit) must stay as typed.
APP = Path(os.path.abspath(__file__)).parents[1]
ROOT = APP.parent


# The repo's .env and the process environment (which wins): the SDK, the pub cache and the deps checkout.
SETTINGS: dict[str, str] = {}


def configure_environment() -> None:
    SETTINGS.update(offline_pub.load_settings(ROOT, os.environ))
    offline_pub.apply_settings(SETTINGS, os.environ)


def flutter_program() -> str:
    # By its full name: on Windows it is flutter.bat, which starting a process by "flutter" does not find.
    flutter = shutil.which("flutter")
    if flutter is None:
        raise RuntimeError("Flutter is not on PATH")
    return flutter


# What `flutter create` leaves in app/ once a platform's runner exists.
RUNNER_MARKERS = {
    "windows": Path("windows/runner/CMakeLists.txt"),
    "linux": Path("linux/runner/CMakeLists.txt"),
    "macos": Path("macos/Runner.xcodeproj/project.pbxproj"),
}
HOST_PLATFORMS = {"win32": "windows", "linux": "linux", "darwin": "macos"}
LOCK = Path("app/pubspec.lock")


def host_platform() -> str:
    return HOST_PLATFORMS.get(sys.platform, "linux")


def require_tools() -> None:
    # The app's native/ plugin runs cargo to build the host library with the runner.
    for tool in ("flutter", "cargo"):
        if shutil.which(tool) is None:
            raise RuntimeError(f"{tool} is not on PATH (a desktop build needs flutter and cargo)")


def runner_missing(platform: str, app: Path = APP) -> bool:
    return not (app / RUNNER_MARKERS[platform]).is_file()


def sh_program() -> str:
    sh = shutil.which("sh")
    if sh is None:
        # Git for Windows keeps its sh beside sed, off the PATH of a PowerShell window.
        git = shutil.which("git")
        candidate = Path(git).resolve().parents[1] / "usr" / "bin" / "sh.exe" if git else None
        if candidate is None or not candidate.is_file():
            raise RuntimeError("sh is not on PATH (name_runner.sh needs it; Git for Windows has one)")
        sh = str(candidate)
    return sh


def ensure_runner(platform: str, app: Path = APP, offline: bool = False) -> None:
    """Generate the platform's runner when missing, then name it as tool/name_runner.sh does (safe to repeat)."""
    if runner_missing(platform, app):
        widget_test = app / "test" / "widget_test.dart"
        had_widget_test = widget_test.exists()
        subprocess.run(
            [flutter_program(), "create", "--org", "dev.boriken", f"--platforms={platform}",
             "--project-name", "canoa", *(["--offline"] if offline else []), "."],
            check=True, cwd=app,
        )
        if not had_widget_test:
            widget_test.unlink(missing_ok=True)
    sh = sh_program()
    # The script's sed sits beside sh in Git for Windows.
    env = dict(os.environ, PATH=str(Path(sh).parent) + os.pathsep + os.environ.get("PATH", ""))
    subprocess.run([sh, str(app / "tool" / "name_runner.sh"), str(app)], check=True, env=env)
    icon = app / "assets" / "icons" / "canoa.ico"
    target = app / "windows" / "runner" / "resources" / "app_icon.ico"
    if platform == "windows" and (not target.is_file() or target.read_bytes() != icon.read_bytes()):
        shutil.copyfile(icon, target)


def clear_stale_build(platform: str, app: Path = APP) -> bool:
    """Remove build/<platform> when it was configured before the runner's CMakeLists.txt last changed.

    A configure from before name_runner.sh renamed the executable keeps naming the old `canoa` target, and CMake
    then fails with `No target "canoa"`. The directory is generated output, so the next build recreates it.
    """
    lists = app / platform / "CMakeLists.txt"
    build = app / "build" / platform
    if not lists.is_file() or not build.is_dir():
        return False
    caches = list(build.glob("*/CMakeCache.txt")) + list(build.glob("CMakeCache.txt"))
    if caches and all(cache.stat().st_mtime >= lists.stat().st_mtime for cache in caches):
        return False
    shutil.rmtree(build)
    return True


def committed_lock(root: Path = ROOT) -> bytes:
    return subprocess.run(
        ["git", "-C", str(root), "show", f"HEAD:{LOCK.as_posix()}"],
        check=True, capture_output=True,
    ).stdout


def restore_lock(before: bytes | None, root: Path = ROOT) -> bool:
    """Put git's pubspec.lock back if a tool rewrote it; one already edited by hand is left alone."""
    lock = root / LOCK
    committed = committed_lock(root)
    if before != committed or lock.read_bytes() == committed:
        return False
    lock.write_bytes(committed)
    return True


def built_executable(platform: str, debug: bool, app: Path = APP) -> Path | None:
    mode = "Debug" if debug else "Release"
    patterns = {
        "windows": f"build/windows/*/runner/{mode}/CanoaUI.exe",
        "linux": f"build/linux/*/{mode.lower()}/bundle/CanoaUI",
        "macos": f"build/macos/Build/Products/{mode}/Canoa.app/Contents/MacOS/CanoaUI",
    }
    found = sorted(app.glob(patterns[platform]))
    return found[0] if found else None


def choose_pub(mode: str) -> Path | None:
    """The seeded pub cache of an offline build, or None for an online one (`mode` is auto, offline or online)."""
    if mode == "online":
        return None
    result = offline_pub.plan(ROOT, SETTINGS, os.environ, ROOT / LOCK)
    if result.reason is not None:
        if mode == "offline":
            raise RuntimeError(f"an offline build is not possible: {result.reason}")
        print(f"Building online: {result.reason}")
    return result.cache


def pub_get_offline(cache: Path, mode: str) -> bool:
    """Resolve from `cache` with the network off; False means go online (auto), an error means --offline failed."""
    try:
        subprocess.run([flutter_program(), "pub", "get", "--offline"], check=True, cwd=APP)
    except subprocess.CalledProcessError as error:
        if mode == "offline":
            raise RuntimeError(f"pub get --offline failed in {cache}") from error
        print(f"Building online: pub get --offline failed in {cache}")
        return False
    return True


def build_web_for_host(offline: bool = False) -> Path:
    """The web app the host serves, built with a CSP (without it the host's policy refuses the page's code)."""
    subprocess.run([sys.executable, str(APP / "tool" / "build_noise.py")], check=True)
    if not offline:
        subprocess.run([flutter_program(), "pub", "get"], check=True, cwd=APP)
    subprocess.run(
        [flutter_program(), "build", "web", "--release", "--csp", "--no-web-resources-cdn",
         *(["--no-pub"] if offline else [])],
        check=True, cwd=APP,
    )
    dist = APP / "build" / "web"
    if not (dist / "index.html").is_file():
        raise RuntimeError("flutter build web left no app/build/web/index.html")
    return dist


def package_portable(platform: str, exe: Path) -> Path:
    """One file for the platform from the built Release folder around `exe`; see portable.py."""
    import portable

    folder = exe.parent
    if platform == "windows":
        return portable.build_windows_launcher(folder, ROOT, ROOT / "target" / "canoa-windows")
    if platform == "linux":
        return portable.build_appimage(folder, ROOT, ROOT / "target" / "canoa-linux")
    raise RuntimeError("Canoa.app is already one bundle: ship build/macos/Build/Products/Release/Canoa.app")


def restore_env(key: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(key, None)
    else:
        os.environ[key] = value


def build_desktop(platform: str, debug: bool, run: bool, portable: bool = False, mode: str = "online") -> None:
    require_tools()
    if portable and platform == "macos":
        raise RuntimeError("Canoa.app is already one bundle: ship build/macos/Build/Products/Release/Canoa.app")
    env = None
    lock = ROOT / LOCK
    before = lock.read_bytes() if lock.is_file() else None
    flag = "--debug" if debug else "--release"
    user_cache = os.environ.get("PUB_CACHE")
    try:
        cache = choose_pub(mode)
        if cache is not None:
            os.environ["PUB_CACHE"] = str(cache)
        # The runner first: pub get is what writes a platform's plugin files, so it must run once the runner exists.
        ensure_runner(platform, offline=cache is not None)
        if cache is not None and not pub_get_offline(cache, mode):
            cache = None
            restore_env("PUB_CACHE", user_cache)
        no_pub = ["--no-pub"] if cache is not None else []
        if portable:
            # The built library carries the web app, which the plugin builds in when CANOA_WEB_DIST names it.
            env = dict(os.environ, CANOA_WEB_DIST=str(build_web_for_host(cache is not None)))
        if platform != "macos" and clear_stale_build(platform):
            print(f"Removed app/build/{platform}, configured before the runner was named")
        if run:
            subprocess.run([flutter_program(), "run", "-d", platform, flag, *no_pub], check=True, cwd=APP)
            return
        subprocess.run([flutter_program(), "build", platform, flag, *no_pub], check=True, cwd=APP, env=env)
    finally:
        restore_env("PUB_CACHE", user_cache)
        if restore_lock(before):
            print("Restored app/pubspec.lock")
    exe = built_executable(platform, debug)
    if exe is None:
        raise RuntimeError(f"flutter build {platform} left no CanoaUI under app/build")
    print(f"Built {exe}")
    if portable:
        print(f"Packaged {package_portable(platform, exe)}")


def flutter_args(kind: str, wasm: bool) -> list[str]:
    args = [flutter_program(), "build", "web", "--release", "--no-web-resources-cdn"]
    if wasm:
        args.append("--wasm")
    if kind == "chromium":
        args += ["--base-href", "/ui/", "--csp"]
    return args


def package_ui(source: Path, target: Path) -> None:
    if not (source / "index.html").is_file() or not (
        source / "flutter_bootstrap.js"
    ).is_file():
        raise FileNotFoundError("Flutter release bundle is incomplete")
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(source, target, ignore=shutil.ignore_patterns("*.map"))
    index = target / "index.html"
    html = index.read_text(encoding="utf-8")
    if "<head>" not in html:
        raise ValueError("Flutter index.html has no head element")
    html = html.replace("<head>", '<head>\n    <script src="../bridge.js"></script>\n    <script src="../acp-page.js"></script>\n    <script src="../acp-content.js"></script>', 1)
    index.write_text(html, encoding="utf-8")


def package_debug_extension(source: Path, destination: Path) -> None:
    """Make an isolated, debug-enabled unpacked extension distribution."""
    if not (source / "manifest.debug.json").is_file():
        raise FileNotFoundError("Elevated Chromium manifest is unavailable")
    if not (source / "ui" / "index.html").is_file():
        raise FileNotFoundError("Build the shared Flutter UI first")
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(
        source, destination,
        ignore=shutil.ignore_patterns(
            "*.test.mjs", "node_modules", "ui/*.map",
        ),
    )
    shutil.copyfile(
        destination / "manifest.debug.json",
        destination / "manifest.json",
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=("pwa", "chromium", "desktop"))
    parser.add_argument(
        "--wasm", action="store_true",
        help="Build Flutter web with --wasm (extension compatibility must be checked)",
    )
    parser.add_argument("--debug", action="store_true",
        help="chromium: package a separate extension with CDP permission; desktop: a debug build")
    parser.add_argument("--run", action="store_true", help="desktop: run the app with flutter run, not build it")
    parser.add_argument("--portable", action="store_true",
        help="desktop: also make the one-file form of a release build (Windows: the CanoaUI.exe launcher in "
             "target/canoa-windows; Linux: an AppImage in target/canoa-linux)")
    pub = parser.add_mutually_exclusive_group()
    pub.add_argument("--online", action="store_true",
        help="desktop: resolve packages from the network, without looking at the deps branch")
    pub.add_argument("--offline", action="store_true",
        help="desktop: resolve from the deps branch, and fail instead of going online (the default is offline, "
             "falling back to online with the reason when a dependency is missing)")
    parser.add_argument("--platform", choices=tuple(RUNNER_MARKERS),
        help="desktop: the platform (default: this machine's; Flutter cannot cross-build)")
    opts = parser.parse_args()
    configure_environment()
    if opts.debug and opts.kind not in ("chromium", "desktop"):
        parser.error("--debug is only valid for chromium and desktop")
    if (opts.run or opts.platform or opts.portable or opts.online or opts.offline) and opts.kind != "desktop":
        parser.error("--run, --platform, --portable, --online and --offline are only valid for desktop")
    if opts.portable and (opts.debug or opts.run):
        parser.error("--portable packages a release build: not with --debug or --run")
    if opts.kind == "desktop":
        platform = opts.platform or host_platform()
        if platform != host_platform():
            parser.error(f"a {platform} build needs a {platform} machine")
        try:
            build_desktop(platform, opts.debug, opts.run, opts.portable,
                          "online" if opts.online else "offline" if opts.offline else "auto")
        except (RuntimeError, subprocess.CalledProcessError) as error:
            sys.exit(f"build.py: {error}")
        return
    # The web build carries the Noise method as WebAssembly, built from app/web_noise into app/web/noise.
    subprocess.run([sys.executable, str(APP / "tool" / "build_noise.py")], check=True)
    subprocess.run([flutter_program(), "pub", "get"], check=True, cwd=APP)
    subprocess.run(flutter_args(opts.kind, opts.wasm), check=True, cwd=APP)
    if opts.kind == "chromium":
        package_ui(
            APP / "build" / "web",
            ROOT / "bindings" / "browser" / "chromium" / "ui",
        )
        print("Packaged Flutter UI into bindings/browser/chromium/ui")
        if opts.debug:
            package_debug_extension(
                ROOT / "bindings" / "browser" / "chromium",
                ROOT / "build" / "chromium-debug",
            )
            print("Elevated debugger extension in build/chromium-debug")
    else:
        print("Built standalone PWA in app/build/web")
        if opts.wasm:
            print("Configure hosting for Flutter Wasm, including required isolation headers.")


if __name__ == "__main__":
    main()
