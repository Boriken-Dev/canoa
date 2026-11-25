#!/usr/bin/env python3
"""Build the Canoa Flutter UI for the PWA, an extension or the desktop."""
from __future__ import annotations

import argparse
import contextlib
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import brand as brand_file
import offline_pub

# tools/ is not a package: the fallback-font list and its checks live there, beside the step that mirrors the fonts.
sys.path.insert(0, str(Path(os.path.abspath(__file__)).parents[2] / "tools"))
import fallback_fonts
import flutter_pin

# Not resolve(): a drive made with `subst` (how a long checkout path stays under MSBuild's limit) must stay as typed.
APP = Path(os.path.abspath(__file__)).parents[1]
ROOT = APP.parent


# The brand this build carries: app/brand.json unless `--brand` or BRAND_FILE names another (set in main).
BRAND = brand_file.load()


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


def brand_icon(app: Path = APP, brand: brand_file.Brand | None = None) -> Path:
    """The Windows icon for the brand: the committed one for the default brand, else rendered from its mark.

    Rendering needs Pillow (tools/canoa_mark.py); the files go under app/build/brand, never into the tracked icons.
    """
    brand = brand or BRAND
    committed = app / "assets" / "icons" / "canoa.ico"
    if brand.source == brand_file.DEFAULT_FILE:
        return committed
    out = app / "build" / "brand" / brand.executable
    subprocess.run(
        [sys.executable, str(app.parent / "tools" / "canoa_mark.py"), str(app.parent), "--brand", str(brand.source), "--out", str(out)],
        check=True, stdout=subprocess.DEVNULL,
    )
    return out / "app" / "assets" / "icons" / "canoa.ico"


def ensure_runner(platform: str, app: Path = APP, offline: bool = False) -> None:
    """Generate the platform's runner when missing, then name it as tool/name_runner.sh does (safe to repeat)."""
    if runner_missing(platform, app):
        widget_test = app / "test" / "widget_test.dart"
        had_widget_test = widget_test.exists()
        subprocess.run(
            [flutter_program(), "create", "--org", BRAND.org, f"--platforms={platform}",
             "--project-name", "canoa", *(["--offline"] if offline else []), "."],
            check=True, cwd=app,
        )
        if not had_widget_test:
            widget_test.unlink(missing_ok=True)
    sh = sh_program()
    # The script's sed sits beside sh in Git for Windows.
    env = dict(os.environ, PATH=str(Path(sh).parent) + os.pathsep + os.environ.get("PATH", ""), BRAND_FILE=str(BRAND.source))
    subprocess.run([sh, str(app / "tool" / "name_runner.sh"), str(app)], check=True, env=env)
    icon = brand_icon(app)
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
        "windows": f"build/windows/*/runner/{mode}/{BRAND.executable}.exe",
        "linux": f"build/linux/*/{mode.lower()}/bundle/{BRAND.executable}",
        "macos": f"build/macos/Build/Products/{mode}/{BRAND.name}.app/Contents/MacOS/{BRAND.executable}",
    }
    found = sorted(app.glob(patterns[platform]))
    return found[0] if found else None


# What a build says when the deps branch cannot supply it: the commands that fill the mirror, never a fetch.
MIRROR_HINT = ("refresh the deps branch (tools/mirror_deps.py fills pub, fonts and Office.js, tools/mirror_crates.py sync the "
               "crates) and fetch it, or pass --online to resolve from the network")


@dataclass(frozen=True)
class Offline:
    """What an offline build reads: the deps checkout, the pub cache seeded from it and the cargo home that vendors Cargo.lock."""

    deps: Path
    cache: Path
    cargo_home: Path | None


def choose_pub(online: bool) -> Offline | None:
    """The offline plan: a seeded pub cache and a cargo home, or None when the build was asked to go online.

    A deps branch that cannot supply a build stops it with what is missing; there is no silent fall back to the network.
    """
    if online:
        return None
    result = offline_pub.plan(ROOT, SETTINGS, os.environ, ROOT / LOCK)
    if result.reason is not None or result.cache is None:
        raise RuntimeError(f"an offline build is not possible: {result.reason}; {MIRROR_HINT}")
    return Offline(offline_pub.find_deps(ROOT, SETTINGS), result.cache, result.cargo_home)


def engine_directories(platform: str) -> list[Path]:
    """The folders of the SDK's cache a build for `platform` reads, which a Flutter not precached for it would download."""
    import platform as host

    arch = "arm64" if host.machine().lower() in ("arm64", "aarch64") else "x64"
    cache = Path("bin") / "cache"
    return {
        "web": [cache / "flutter_web_sdk"],
        "windows": [cache / "artifacts" / "engine" / f"windows-{arch}"],
        "linux": [cache / "artifacts" / "engine" / f"linux-{arch}"],
        "macos": [cache / "artifacts" / "engine" / "darwin-x64"],
    }[platform]


def require_pinned_sdk(platform: str) -> None:
    """Stop unless the Flutter on PATH is tools/flutter.pin's and holds what a `platform` build reads.

    Flutter downloads whatever its cache lacks from FLUTTER_STORAGE_BASE_URL while it builds; the SDK comes from the mirror
    (Boriken-Dev/flutter, with its engine-<hash> release) already precached, and a build never asks for more.
    """
    sdk = Path(flutter_program()).resolve().parents[1]
    problems = flutter_pin.sdk_problems(sdk, flutter_pin.load())
    problems += [f"{sdk / folder} is missing: the SDK was not precached for {platform}"
                 for folder in engine_directories(platform) if not (sdk / folder).is_dir()]
    if problems:
        raise RuntimeError(
            "the Flutter SDK is not the one tools/flutter.pin names, precached for this build:\n  " + "\n  ".join(problems)
            + f"\nInstall it from the mirror with: python tools/flutter_pin.py install DIR --platforms {platform}")


@contextlib.contextmanager
def environment(changes: dict[str, str | None]):
    """Set (or, for None, remove) variables for the tools a build starts, and put them back afterwards."""
    saved = {key: os.environ.get(key) for key in changes}
    for key, value in changes.items():
        restore_env(key, value)
    try:
        yield
    finally:
        for key, value in saved.items():
            restore_env(key, value)


# Where Flutter's cache would download from if it lacked a file: nowhere, so a missing one fails instead of fetching.
NO_STORAGE = "http://127.0.0.1:9"


def network_off(offline: Offline | None) -> dict[str, str | None]:
    """The environment of an offline build: the seeded pub cache, the vendored cargo home, no telemetry and no storage URL."""
    if offline is None:
        return {}
    changes: dict[str, str | None] = {
        "PUB_CACHE": str(offline.cache),
        "FLUTTER_SUPPRESS_ANALYTICS": "true",
        "DART_SUPPRESS_ANALYTICS": "true",
        "CARGO_NET_OFFLINE": "true",
        "FLUTTER_STORAGE_BASE_URL": os.environ.get("FLUTTER_STORAGE_BASE_URL") or NO_STORAGE,
    }
    if offline.cargo_home is not None:
        changes["CARGO_HOME"] = str(offline.cargo_home)
    return changes


@contextlib.contextmanager
def build_environment(online: bool, platform: str):
    """The pinned SDK checked, the deps checkout read, the network off: what every kind of build runs inside.

    Yields the Offline plan, or None for `--online` (the one way to resolve from the network; the SDK is still the pin).
    """
    require_pinned_sdk(platform)
    offline = choose_pub(online)
    with environment(network_off(offline)):
        yield offline


def pub_get_offline(offline: Offline) -> None:
    """Resolve app/pubspec.lock from the seeded cache with the network off."""
    try:
        subprocess.run([flutter_program(), "pub", "get", "--offline"], check=True, cwd=APP)
    except subprocess.CalledProcessError as error:
        raise RuntimeError(f"pub get --offline failed in {offline.cache}") from error


def prepare_packages(offline: Offline | None) -> None:
    """`flutter pub get`, offline from the seeded cache, or online when the build was asked to be."""
    if offline is None:
        subprocess.run([flutter_program(), "pub", "get"], check=True, cwd=APP)
    else:
        pub_get_offline(offline)


def build_wasm(offline: Offline | None) -> None:
    """The web build's WebAssembly (the Noise method and Whisper), from the deps branch's crates-web_noise/ and crates-web_whisper/.

    `--online` reads crates.io and GitHub instead, to refresh the mirror.
    """
    where = ["--deps", str(offline.deps)] if offline else ["--online"]
    for script in ("build_noise.py", "build_whisper.py"):
        subprocess.run([sys.executable, str(APP / "tool" / script), *where], check=True)


def finish_web(dist: Path) -> None:
    """What a web build gets after Flutter: no service worker, and the fallback fonts the engine asks for.

    The fonts come from the deps checkout's mirror (tools/mirror_deps.py fills it); this never fetches. A missing or
    changed font stops the build, since a page without them cannot draw a symbol, an emoji or CJK.
    """
    (dist / "flutter_service_worker.js").unlink(missing_ok=True)
    pinned = fallback_fonts.read_list()
    try:
        fallback_fonts.check_engine(pinned, fallback_fonts.flutter_root())
        deps = offline_pub.find_deps(ROOT, SETTINGS)
        size = fallback_fonts.install(deps, dist, pinned)
    except fallback_fonts.FontError as error:
        raise RuntimeError(str(error)) from error
    print(f"Fallback fonts: {len(pinned)} files, {size} bytes, from the deps mirror")


def no_pub(offline: Offline | None) -> list[str]:
    """`flutter build` and `flutter run` resolve packages themselves unless told not to; an offline build resolved them already."""
    return ["--no-pub"] if offline is not None else []


def build_web_for_host(offline: Offline | None) -> Path:
    """The web app the host serves, built with a CSP (without it the host's policy refuses the page's code)."""
    build_wasm(offline)
    prepare_packages(offline)
    subprocess.run(
        [flutter_program(), "build", "web", "--release", "--csp", "--no-web-resources-cdn", *BRAND.defines(), *no_pub(offline)],
        check=True, cwd=APP,
    )
    dist = APP / "build" / "web"
    if not (dist / "index.html").is_file():
        raise RuntimeError("flutter build web left no app/build/web/index.html")
    finish_web(dist)
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


def office_js_environment(source: str) -> dict[str, str]:
    """What the host's build reads to carry Office.js: Microsoft's address for `cdn`, else the deps mirror's copy.

    The default (`bundled`) needs the deps checkout, which crates/canoa-host/build.rs names in its error when there is none.
    """
    if source == "cdn":
        return {"CANOA_OFFICE_JS": "cdn"}
    deps = None if os.environ.get("CANOA_DEPS") else offline_pub.find_deps(ROOT, SETTINGS)
    return {"CANOA_OFFICE_JS": "bundled", **({"CANOA_DEPS": str(deps)} if deps else {})}


def build_desktop(platform: str, debug: bool, run: bool, portable: bool = False, online: bool = False,
                  office_js: str = "bundled") -> None:
    require_tools()
    if portable and platform == "macos":
        raise RuntimeError(f"{BRAND.name}.app is already one bundle: ship build/macos/Build/Products/Release/{BRAND.name}.app")
    if portable and BRAND.executable != "CanoaUI":
        raise RuntimeError("--portable packs CanoaUI: the launcher and the AppImage do not follow a brand's executable name yet")
    lock = ROOT / LOCK
    before = lock.read_bytes() if lock.is_file() else None
    flag = "--debug" if debug else "--release"
    # The host's build reads the brand's environment-variable prefix; the app gets the rest as dart definitions.
    outer = {**BRAND.environment(), **office_js_environment(office_js)}
    exe = None
    try:
        with environment(outer), build_environment(online, platform) as offline:
            # The runner first: pub get is what writes a platform's plugin files, so it must run once the runner exists.
            ensure_runner(platform, offline=offline is not None)
            if offline is not None:
                pub_get_offline(offline)
            env = None
            if portable:
                # The built library carries the web app, which the plugin builds in when CANOA_WEB_DIST names it.
                env = dict(os.environ, CANOA_WEB_DIST=str(build_web_for_host(offline)))
            if platform != "macos" and clear_stale_build(platform):
                print(f"Removed app/build/{platform}, configured before the runner was named")
            if run:
                subprocess.run([flutter_program(), "run", "-d", platform, flag, *no_pub(offline), *BRAND.defines()], check=True, cwd=APP)
                return
            subprocess.run([flutter_program(), "build", platform, flag, *no_pub(offline), *BRAND.defines()], check=True, cwd=APP, env=env)
            exe = built_executable(platform, debug)
            if exe is None:
                raise RuntimeError(f"flutter build {platform} left no {BRAND.executable} under app/build")
            print(f"Built {exe}")
            if portable:
                # Still inside the offline environment: the launcher and the AppImage are built by cargo and from the mirror.
                print(f"Packaged {package_portable(platform, exe)}")
    finally:
        if restore_lock(before):
            print("Restored app/pubspec.lock")


ICON_SIZES = (16, 32, 64, 80)
OFFICE_JS_LIST = ROOT / "tools" / "office_js.json"


def office_js_folder() -> Path:
    """The mirror of the office-js repository: CANOA_OFFICE_JS_DIR, else cdn/office-js of the deps checkout."""
    own = os.environ.get("CANOA_OFFICE_JS_DIR")
    if own:
        return Path(own)
    deps = os.environ.get("CANOA_DEPS")
    found = Path(deps) if deps else offline_pub.find_deps(ROOT, SETTINGS)
    if found is None:
        raise RuntimeError("Office.js comes from the deps mirror: set CANOA_DEPS to a checkout of the deps branch")
    return Path(found) / "cdn" / "office-js"


def package_office(web: Path, target: Path, mirror: Path, taskpane: Path | None = None, listing: Path = OFFICE_JS_LIST) -> Path:
    """The add-in's pages as a static site: what a host serves under /office/, in one folder to publish at any origin.

    `web` is the app built with `--base-href /office/app/`. Office.js and the files it loads come from `mirror`, each checked
    against the SHA-256 `listing` pins; nothing is fetched. Every page is self-contained: its requests all go to its own origin.
    """
    taskpane = taskpane or ROOT / "clients" / "office" / "taskpane"
    if not (web / "index.html").is_file():
        raise RuntimeError(f"{web} holds no built app")
    pinned = json.loads(listing.read_text(encoding="utf-8"))
    if target.exists():
        shutil.rmtree(target)
    office = target / "office"
    shutil.copytree(web, office / "app", ignore=shutil.ignore_patterns("*.map"))
    for page in sorted(taskpane.iterdir()):
        if not page.is_file():
            continue
        if page.suffix == ".html":
            html = page.read_text(encoding="utf-8").replace("@OFFICE_JS@", "js/office.js")
            (office / page.name).write_text(html, encoding="utf-8", newline="")
        else:
            shutil.copyfile(page, office / page.name)
    for name in pinned["carried"]:
        source = mirror / name
        try:
            body = source.read_bytes()
        except OSError as error:
            raise RuntimeError(f"{name} is not in the Office.js mirror {mirror}: refresh the deps branch") from error
        seen = hashlib.sha256(body).hexdigest()
        if seen != pinned["files"][name]:
            raise RuntimeError(f"{source} is not the file {listing.name} pins (SHA-256 {seen})")
        destination = office / "js" / (name[len("dist/"):] if name.startswith("dist/") else name)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(body)
    icon = web / "icons" / "Icon-192.png"
    for size in ICON_SIZES:
        shutil.copyfile(icon, office / f"icon-{size}.png")
    return target


def build_office(online: bool = False) -> Path:
    """The pane's static site: the app built for /office/app/, with the pages and Office.js around it."""
    mirror = office_js_folder()
    with build_environment(online, "web") as offline:
        build_wasm(offline)
        prepare_packages(offline)
        subprocess.run(
            [flutter_program(), "build", "web", "--release", "--csp", "--no-web-resources-cdn", "--base-href", "/office/app/",
             *no_pub(offline)],
            check=True, cwd=APP,
        )
        dist = APP / "build" / "web"
        finish_web(dist)
    return package_office(dist, ROOT / "target" / "canoa-office", mirror)


def flutter_args(kind: str, wasm: bool, offline: Offline | None = None) -> list[str]:
    args = [flutter_program(), "build", "web", "--release", "--no-web-resources-cdn", *BRAND.defines(), *no_pub(offline)]
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
    parser.add_argument("kind", choices=("pwa", "chromium", "desktop", "office"))
    parser.add_argument("--brand", type=Path, help="the brand file: name, identifiers, environment-variable prefix, mark "
        "(default: app/brand.json, or the file BRAND_FILE names)")
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
    parser.add_argument("--online", action="store_true",
        help="resolve packages, crates and Flutter's own files from the network, without the deps branch (for refreshing the "
             "mirror; the default reads only the deps checkout and the pinned SDK, and stops naming what is missing)")
    parser.add_argument("--office-js", choices=("bundled", "cdn"), default="bundled",
        help="desktop: where the Office add-in's pages load Office.js from: the host's own copy, taken from the deps "
             "mirror (the default, nothing is fetched from Microsoft), or Microsoft's CDN")
    parser.add_argument("--platform", choices=tuple(RUNNER_MARKERS),
        help="desktop: the platform (default: this machine's; Flutter cannot cross-build)")
    opts = parser.parse_args()
    global BRAND
    try:
        BRAND = brand_file.load(opts.brand)
    except brand_file.BrandError as error:
        parser.error(str(error))
    configure_environment()
    if opts.debug and opts.kind not in ("chromium", "desktop"):
        parser.error("--debug is only valid for chromium and desktop")
    desktop_only = opts.run or opts.platform or opts.portable or opts.office_js != "bundled"
    if desktop_only and opts.kind != "desktop":
        parser.error("--run, --platform, --portable and --office-js are only valid for desktop")
    if opts.portable and (opts.debug or opts.run):
        parser.error("--portable packages a release build: not with --debug or --run")
    if opts.kind == "office":
        if opts.debug or opts.wasm:
            parser.error("office takes no --debug or --wasm")
        try:
            print(f"Built the Office pane's static site in {build_office(opts.online)}")
        except (RuntimeError, subprocess.CalledProcessError) as error:
            sys.exit(f"build.py: {error}")
        return
    if opts.kind == "desktop":
        platform = opts.platform or host_platform()
        if platform != host_platform():
            parser.error(f"a {platform} build needs a {platform} machine")
        try:
            build_desktop(platform, opts.debug, opts.run, opts.portable, opts.online, opts.office_js)
        except (RuntimeError, subprocess.CalledProcessError) as error:
            sys.exit(f"build.py: {error}")
        return
    # The web build carries the Noise method and Whisper as WebAssembly, built from app/web_noise into app/web/noise and from
    # app/web_whisper into app/web/whisper.
    try:
        with build_environment(opts.online, "web") as offline:
            build_wasm(offline)
            prepare_packages(offline)
            subprocess.run(flutter_args(opts.kind, opts.wasm, offline), check=True, cwd=APP)
            finish_web(APP / "build" / "web")
    except (RuntimeError, subprocess.CalledProcessError) as error:
        sys.exit(f"build.py: {error}")
    if opts.kind == "chromium":
        package_ui(
            APP / "build" / "web",
            ROOT / "clients" / "browser" / "chromium" / "ui",
        )
        print("Packaged Flutter UI into clients/browser/chromium/ui")
        if opts.debug:
            package_debug_extension(
                ROOT / "clients" / "browser" / "chromium",
                ROOT / "build" / "chromium-debug",
            )
            print("Elevated debugger extension in build/chromium-debug")
    else:
        print("Built standalone PWA in app/build/web")
        if opts.wasm:
            print("Configure hosting for Flutter Wasm, including required isolation headers.")


if __name__ == "__main__":
    main()
