import '../config/canoa_config.dart';
import 'desktop_shell.dart';

DesktopShell createDesktopShell() => _NoShell();
class _NoShell implements DesktopShell {
  @override
  Future<void> Function()? beforeQuit;

  @override
  Future<void> initialize(CanoaConfig config, {required void Function() onSettings, void Function()? onHosting}) async {}
  @override
  Future<void> update(CanoaConfig config) async {}
  @override
  Future<void> open() async {}
  @override
  Future<void> dispose() async {}
  @override
  bool get hasTray => false;
  @override
  bool get isDesktop => false;
  @override
  Future<void> quit() async {}
  @override
  Future<void> showWaiting(int count) async {}
}
