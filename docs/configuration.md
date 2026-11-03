# Desktop and Canoa.cfg

Canoa uses the **same deployment configuration convention** as AI6's Outer Office: a user-level TOML file ending in .cfg, with the environment override taking priority.

The default template is [examples/config/Canoa.cfg](https://github.com/Boriken-Dev/canoa/blob/main/examples/config/Canoa.cfg).

| Platform | Config location |
| --- | --- |
| Windows | %APPDATA%\Canoa\Canoa.cfg |
| macOS | ~/Library/Application Support/Canoa/Canoa.cfg |
| Linux | $XDG_CONFIG_HOME/Canoa/Canoa.cfg or ~/.config/Canoa/Canoa.cfg |
| Override | CANOA_CONFIG |
| PWA / extension | Canoa.cfg alongside the deployed index.html, read-only |

Configuration tables:

- [desktop] start_hidden, close_to_tray, minimize_to_tray
- [network] prefer_webtransport, connect_timeout (seconds, between 1 and 120)
- [assets] cdn = "none" or "local" (deployment option; font hosting remains a separate build concern)
- [host] what `canoa-host` serves: [host.agents.NAME] and [host.providers.NAME], described in [the host](host.md). The app keeps this table as it is when it saves its settings.

Do not store identity private keys, tokens or passwords in Canoa.cfg.

## Windows, macOS and Linux tray behavior

Canoa uses a desktop tray controller and the shared Flutter app under root app/. The app can start without a visible main window, remain alive when its window closes, and expose Open Canoa, Hosting, Settings, Hide and Exit commands via the notification area. Closing the window leaves the app, its conversations, its own agents and `canoa-host` running; Exit, or **Quit Canoa** in Settings, ends them.

What has been run, by platform:

- **Windows 11 (arm64)**: the icon is in the notification area (in its overflow, until the person pins it), named Canoa. Closing the window hides it while the host keeps serving, and the real extension in Edge ran a turn through that host with the window closed. Two clicks on the icon within half a second, Windows' default double-click time, open the window; that decision is tested through the plugin's events, and a double click by mouse was not run.
- **Linux (Fedora 44 under WSLg, arm64)**: tray_manager draws the icon as a StatusNotifier item, which passes no clicks on, so **Open Canoa** in its menu opens the chat. The app looks for a StatusNotifier host on the session bus (`org.kde.StatusNotifierWatcher`). With one, closing the window hides it, and picking Open Canoa from the menu over D-Bus, as a panel does, brings it back. WSLg's desktop has none, so there the app shows no icon, and closing the window minimizes it instead of hiding it where nothing could bring it back; WSLg does not report the window as minimized.
- **macOS**: not run.

The Settings action brings up the main window and opens the Canoa settings dialog. Saving writes the same Canoa.cfg file, not an unrelated preferences database. A system-hosted web application cannot overwrite its deployment Canoa.cfg.

The default launch is visible, with close-to-tray enabled. The start-hidden option takes effect on the next launch.

## Building desktop runners

This repository keeps the canonical Flutter code in app/. Flutter's platform runner directories must be generated before native desktop builds:

    cd app
    flutter create --org dev.boriken --platforms=windows,linux,macos .
    flutter pub get
    flutter build windows  # use linux or macos on that platform

On Linux the tray and window packages (leanflutter's `tray_manager` and `window_manager`) link system libraries that no Dart package carries, so a build needs them installed, offline too:

    sudo apt-get install libayatana-appindicator3-dev libgtk-3-dev libx11-dev libxi-dev

On Fedora the package is `libayatana-appindicator-gtk3-devel`. Its version 0.6 marks a function tray_manager calls as
deprecated, and the generated runner builds with `-Werror`, so build with
`CXXFLAGS=-Wno-error=deprecated-declarations` there (Fedora 44).

A desktop build carries `canoa-host` beside the app's executable, where the app finds it: `cargo build --release -p
canoa-host`, then copy `target/release/canoa-host` (`.exe` on Windows) into the bundle's directory, as the desktop
workflow does.

`app/integration_test/desktop_test.dart` runs the app on a real desktop. On a Linux session with no tray, run
`python3 integration_test/sni_watcher.py` first to have one: it is a minimal StatusNotifier watcher that draws nothing,
and the test picks the menu's entries over D-Bus.

The `deps` branch mirrors those packages' Dart side and names these in its `manifest.json`. The desktop CI workflow generates the appropriate runner on each OS with Boriken Dev's reverse-DNS namespace and builds it. Windows, macOS and Linux compilation have been verified in GitHub Actions; the tray on a full Linux desktop panel (KDE, GNOME with its AppIndicator extension) has not been run. Platform-native plugins and tray host support are required. A standard Flutter Web/PWA build does not create a system notification icon.

## Limitations

Canoa is still an early-stage app. The tray controller preserves the process, in-memory sessions and the host while its window is hidden; it does not replace a native background service, persist sessions across system restarts, or automatically install a startup entry.
