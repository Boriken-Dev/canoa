# One file per platform

Canoa ships a desktop app as one file where the platform allows it.

| Platform | File | Made by |
| --- | --- | --- |
| Windows | `CanoaUI.exe`, a launcher that carries the built app folder | `python app/tool/build.py desktop --portable` |
| Linux | `Canoa-<arch>.AppImage` | the same command on Linux |
| macOS | `Canoa.app`, already one bundle | `python app/tool/build.py desktop`, then ship `app/build/macos/Build/Products/Release/Canoa.app` |

Flutter cannot cross-build, so each file is built on its own platform. Nothing generated here is committed: the app archive,
the launcher and the AppImage are build output under `target/`.

## Windows: the launcher

`crates/canoa-launch` builds `CanoaUI.exe`. The packaging step builds the app in Release, packs its folder into a deflated
zip whose first entry is a manifest (every file's path, size and SHA-256, and the version), and builds the launcher with that
archive and its SHA-256 compiled in. Copy the one file anywhere and start it:

1. It checks the embedded archive against its SHA-256 and reads the manifest.
2. `%LOCALAPPDATA%\Canoa\<version>\` is where the app runs from. When that folder is absent, or is not exactly the manifest
   (a file changed or missing, one more file, a link inside), the launcher unpacks to a temporary sibling, checks every file
   against the manifest as it is written, and renames the folder into place. Otherwise it reuses the folder, after hashing
   every file again, so a second start unpacks nothing and a file changed since is put back.
3. It removes the folders it made earlier beyond the current version and the newest other one.
4. It starts `CanoaUI.exe` from that folder with its own arguments, with that folder as the working folder (the app reads no
   file by a path relative to it; each relative entry of `CANOA_CONFIG` is made absolute first, and `CANOA_INSTALLED_DIR` names the launcher's own folder, where an installed `canoa.cfg` is read from), waits, and exits with the app's status.

The launcher links the C runtime in and loads what it still imports from System32 alone, so a runtime or system DLL dropped
beside it (a download folder) is not picked up before it can check anything.

It refuses, and runs nothing, when `%LOCALAPPDATA%\Canoa` or the version folder is a symbolic link, a junction or another
reparse point, is not a folder, or is owned by someone else (the current user, Administrators and the system account are accepted: Canoa started from an elevated window leaves Administrators-owned files); it never starts a file the manifest does not list, and a
folder holding one is replaced. It deletes only a folder that holds a manifest copy naming that same folder, and its own
temporary folders. A launch that fails says why on standard error and in a message box.

The version is the workspace's `version`. A rebuild that keeps the version replaces the folder in place (the old folder is
renamed away first, which fails while Canoa is running from it, and the message says so); a new version unpacks beside it.

What the launcher cannot do: it protects against another user, a damaged or stale copy and a file planted beside the app's own
libraries. It does not protect against code already running as the same user, which can change the launcher too. Its integrity
check detects a damaged copy of the exe; only a code signature says who made it.

### Release recipe

Needs Flutter, Rust and Python on PATH, and the Visual Studio C++ workload. A long checkout path breaks MSBuild (`FTK1011`), so
map the checkout to a drive and build from it; Python is run as typed, not resolved to the real path, so the mapping holds:

```powershell
subst L: C:\path\to\canoa
cd L:\app
python tool\build.py desktop --portable
cd \; subst L: /d
```

The step builds the web app with a content security policy (the host's policy refuses a page without it), builds the app with
that web app inside `canoa_ffi.dll` (`CANOA_WEB_DIST`), packs the Release folder, builds the launcher around the archive and
writes `target/canoa-windows/CanoaUI.exe`, replacing the folder that used to be there. No `canoa.exe` is needed or shipped.

Check it before it leaves:

```powershell
$env:LOCALAPPDATA = "$env:TEMP\canoa-check"          # a throwaway profile, not yours
New-Item -ItemType Directory -Force $env:TEMP\only | Out-Null
Copy-Item target\canoa-windows\CanoaUI.exe $env:TEMP\only\
(Start-Process $env:TEMP\only\CanoaUI.exe '--canoa-child peers' -Wait -PassThru -WindowStyle Hidden).ExitCode   # 2, no window
Test-Path $env:LOCALAPPDATA\Canoa\*\canoa_ffi.dll                 # True
```

### SmartScreen and signing

The file is not signed. A copy downloaded with a browser carries the Mark of the Web, and Windows then shows Microsoft Defender
SmartScreen's "Windows protected your PC" with the publisher as "Unknown publisher"; "More info", then "Run anyway" starts it. A
machine in Smart App Control's enforcement mode blocks an unsigned program it has no reputation for and offers no such button.
Defender may also flag a program that unpacks and starts another; a signature and downloads over time are what remove those
warnings. A copy that arrives without the Mark of the Web (a file share, a build the same machine made) starts without either
prompt. Signing is the owner's decision; nothing here signs. The launcher is built to survive it: the archive is compiled in,
not appended, so signing the finished exe does not invalidate it.

## Linux: the AppImage

`python app/tool/build.py desktop --portable` on Linux builds the release bundle (`flutter build linux`, with the web app inside
the host library), lays it out as an AppDir (the bundle under `usr/bin`, `AppRun`, `canoa.desktop`, the icon) and runs
`appimagetool` to write `target/canoa-linux/Canoa-<arch>.AppImage`. appimagetool and the AppImage runtime are downloaded into
`target/appimage-tools/` and each is checked against a SHA-256 pinned in `app/tool/portable.py`; a download that differs is
deleted and the build stops. They run with `APPIMAGE_EXTRACT_AND_RUN=1`, so the build needs no FUSE. The Flutter Linux build needs
`clang`, `cmake`, `ninja`, `pkg-config` and the GTK 3 development files.

To run it: `chmod +x Canoa-x86_64.AppImage && ./Canoa-x86_64.AppImage`. It mounts itself through FUSE 2 (`libfuse2`, or `fuse-libs`
on Fedora); where that is missing, extract it into a folder only you can use and run it from there:
`mkdir -m 700 ~/canoa-app && cd ~/canoa-app && ../Canoa-x86_64.AppImage --appimage-extract && squashfs-root/AppRun`. On a shared
machine prefer that to `--appimage-extract-and-run`, which unpacks under `/tmp`. The build gives appimagetool a private
`target/appimage-tmp` (mode 0700) for the same reason. The AppImage is not signed either.

## macOS

`Canoa.app` is the bundle Flutter builds. An unsigned, un-notarized copy that arrives from the network is stopped by Gatekeeper
until the person opens it with Control-click, Open (or removes the quarantine attribute); signing and notarization need an Apple
developer account and are the owner's decision.
