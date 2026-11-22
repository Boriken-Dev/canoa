import '../config/canoa_config.dart';
import 'desktop_shell_stub.dart'
    if (dart.library.io) 'desktop_shell_io.dart' as platform;

abstract interface class DesktopShell {
  /// [onSettings] and [onHosting] open those dialogs, from the tray menu. [background] starts with no window, as the entry that
  /// starts the app at sign-in asks.
  Future<void> initialize(
    CanoaConfig config, {
    required void Function() onSettings,
    void Function()? onHosting,
    bool background = false,
  });
  Future<void> update(CanoaConfig config);
  Future<void> open();
  Future<void> dispose();

  /// Whether a tray icon is shown. Without one, closing the window minimizes it, since nothing could bring it back.
  bool get hasTray;

  /// Whether this is a desktop window that [quit] can end.
  bool get isDesktop;

  /// Ends what the app started, then the app.
  Future<void> quit();

  /// Runs before the app quits, to end what it started.
  set beforeQuit(Future<void> Function()? ending);

  /// Registers the app to start when this person signs in, or removes that. Returns whether the computer now holds it.
  Future<bool> setStartAtLogin(bool on);

  /// Says on the tray how many sessions wait for the person's answer.
  Future<void> showWaiting(int count);
}
DesktopShell createDesktopShell() => platform.createDesktopShell();
