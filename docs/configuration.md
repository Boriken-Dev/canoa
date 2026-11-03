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

Canoa uses a desktop tray controller and the shared Flutter app under root app/. The app can start without a visible main window, remain alive when its window closes, and expose Open Canoa, Settings, Hide and Exit commands via the notification area. Double-click detection opens the main window on Windows/macOS. Linux StatusNotifierItem/AppIndicator implementations typically do not forward icon click events: Open Canoa remains accessible in their tray menu.

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

The `deps` branch mirrors those packages' Dart side and names these in its `manifest.json`. The desktop CI workflow generates the appropriate runner on each OS with Boriken Dev's reverse-DNS namespace and builds it. Windows, macOS and Linux compilation have been verified in GitHub Actions; human tray behavior still needs per-desktop testing. Platform-native plugins and tray host support are required. A standard Flutter Web/PWA build does not create a system notification icon.

## Limitations

Canoa is still an early-stage app. The tray controller preserves the process and in-memory sessions while its window is hidden; it does not replace a native background service, persist sessions across system restarts, or automatically install a startup entry.
