import '../config/canoa_config.dart';
import 'desktop_shell_stub.dart'
    if (dart.library.io) 'desktop_shell_io.dart' as platform;

abstract interface class DesktopShell {
  /// [onSettings] and [onHosting] open those dialogs, from the tray menu.
  Future<void> initialize(
    CanoaConfig config, {
    required void Function() onSettings,
    void Function()? onHosting,
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
}
DesktopShell createDesktopShell() => platform.createDesktopShell();
