import '../config/canoa_config.dart';
import 'desktop_shell_stub.dart'
    if (dart.library.io) 'desktop_shell_io.dart' as platform;

abstract interface class DesktopShell {
  Future<void> initialize(CanoaConfig config, {required void Function() onSettings});
  Future<void> update(CanoaConfig config);
  Future<void> open();
  Future<void> dispose();
}
DesktopShell createDesktopShell() => platform.createDesktopShell();
