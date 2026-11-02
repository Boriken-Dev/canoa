import 'dart:async';
import 'dart:io';
import 'package:tray_manager/tray_manager.dart';
import 'package:window_manager/window_manager.dart';
import '../config/canoa_config.dart';
import 'desktop_shell.dart';

DesktopShell createDesktopShell() => _Shell();
class _Shell with WindowListener, TrayListener implements DesktopShell {
  CanoaConfig config = const CanoaConfig();
  void Function()? onSettings;
  bool ready = false, quitting = false;
  @override
  Future<void> Function()? beforeQuit;
  DateTime? lastClick;
  bool get isDesktop => Platform.isWindows || Platform.isMacOS || Platform.isLinux;

  @override
  Future<void> initialize(CanoaConfig settings, {required void Function() onSettings}) async {
    config = settings;
    this.onSettings = onSettings;
    if (!isDesktop) return;
    await windowManager.ensureInitialized();
    await windowManager.setPreventClose(true);
    windowManager.addListener(this);
    trayManager.addListener(this);
    await trayManager.setIcon('assets/icons/canoa_tray.png');
    await trayManager.setToolTip('Canoa — agent connections');
    await trayManager.setContextMenu(Menu(items: [
      MenuItem(key: 'open', label: 'Open Canoa'),
      MenuItem(key: 'settings', label: 'Settings…'),
      MenuItem(key: 'hide', label: 'Hide to tray'),
      MenuItem.separator(),
      MenuItem(key: 'quit', label: 'Exit Canoa'),
    ]));
    ready = true;
    if (config.startHidden) await windowManager.hide();
  }
  @override
  Future<void> update(CanoaConfig settings) async { config = settings; }
  @override
  Future<void> open() async {
    if (!ready) return;
    await windowManager.show();
    await windowManager.restore();
    await windowManager.focus();
  }
  @override
  void onTrayIconMouseDown() {
    // Legacy tray_manager 0.5.x reports clicks but not double-click events.
    // Linux AppIndicator does not forward clicks; menu Open remains available.
    final now = DateTime.now();
    if (lastClick != null &&
        now.difference(lastClick!) <= const Duration(milliseconds: 450)) {
      lastClick = null;
      unawaited(open());
    } else {
      lastClick = now;
    }
  }
  @override
  void onTrayIconRightMouseDown() {
    unawaited(trayManager.popUpContextMenu());
  }
  @override
  void onTrayMenuItemClick(MenuItem item) {
    switch (item.key) {
      case 'open': unawaited(open());
      case 'settings': unawaited(open().then((_) => onSettings?.call()));
      case 'hide': unawaited(windowManager.hide());
      case 'quit': unawaited(_quit());
    }
  }
  @override
  void onWindowClose() {
    if (quitting) return;
    if (config.closeToTray) {
      unawaited(windowManager.hide());
    } else {
      unawaited(_quit());
    }
  }
  @override
  void onWindowMinimize() {
    if (config.minimizeToTray) unawaited(windowManager.hide());
  }
  Future<void> _quit() async {
    quitting = true;
    await beforeQuit?.call();
    await dispose();
    await windowManager.destroy();
  }
  @override
  Future<void> dispose() async {
    if (!ready) return;
    ready = false;
    trayManager.removeListener(this);
    windowManager.removeListener(this);
    await trayManager.destroy();
  }
}
