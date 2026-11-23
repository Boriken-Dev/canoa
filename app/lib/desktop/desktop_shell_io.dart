import 'dart:async';
import 'dart:io';
import 'package:flutter/foundation.dart';
import 'package:flutter/painting.dart' show Color;
import 'package:tray_manager/tray_manager.dart';
import 'package:window_manager/window_manager.dart';
import '../brand.dart';
import '../config/canoa_config.dart';
import '../l10n/app_text.dart';
import 'desktop_shell.dart';
import 'login_start.dart';
import 'tray_icon.dart';

DesktopShell createDesktopShell() => TrayShell();

/// Whether something on this Linux session's bus shows StatusNotifier items, which is how its tray icon is drawn. A desktop
/// with none, such as WSLg's, would leave a window hidden to the tray with no way back.
Future<bool> linuxShowsTrayIcons() async {
  try {
    final asked = await Process.run('gdbus', [
      'call', '--session', '--dest', 'org.freedesktop.DBus', '--object-path', '/org/freedesktop/DBus',
      '--method', 'org.freedesktop.DBus.NameHasOwner', 'org.kde.StatusNotifierWatcher',
    ]).timeout(const Duration(seconds: 3));
    return asked.exitCode == 0 && (asked.stdout as String).contains('true');
  } on Object {
    return false;
  }
}

/// The window and its tray icon. On Windows a double click on the icon opens the chat; on Linux, whose StatusNotifier
/// icons pass no clicks on, the menu's Open does.
class TrayShell with WindowListener, TrayListener implements DesktopShell, TrayAppearance {
  TrayShell({
    @visibleForTesting Future<bool> Function()? trayAvailable,
    @visibleForTesting bool? desktop,
    @visibleForTesting DateTime Function()? clock,
    @visibleForTesting this._loginStart,
    @visibleForTesting Future<String> Function(Color accent)? iconFile,
  }) : _iconFile = iconFile ?? ((accent) => writeTrayIcon(accent, ico: Platform.isWindows)),
       _trayAvailable = trayAvailable ?? (() async => !Platform.isLinux || await linuxShowsTrayIcons()),
       _desktop = desktop ?? (Platform.isWindows || Platform.isMacOS || Platform.isLinux),
       _clock = clock ?? DateTime.now;

  /// Two clicks this close together are a double click: Windows' own default.
  static const Duration doubleClick = Duration(milliseconds: 500);

  final Future<bool> Function() _trayAvailable;
  final bool _desktop;
  final DateTime Function() _clock;
  final LoginStart? _loginStart;
  final Future<String> Function(Color accent) _iconFile;
  AppText _text = AppText.base;
  Color _accent = Color(Brand.accentArgb);
  CanoaConfig config = const CanoaConfig();
  void Function()? onSettings, onHosting;
  bool ready = false, quitting = false;
  bool _hasTray = false;
  @override
  Future<void> Function()? beforeQuit;
  DateTime? lastClick;

  /// The tray's menu, once it has one.
  @visibleForTesting
  Menu? menu;

  @override
  bool get isDesktop => _desktop;

  @override
  bool get hasTray => _hasTray;

  @override
  Future<void> initialize(CanoaConfig settings, {required void Function() onSettings, void Function()? onHosting, bool background = false}) async {
    config = settings;
    this.onSettings = onSettings;
    this.onHosting = onHosting;
    if (!_desktop) return;
    await windowManager.ensureInitialized();
    // The generated runner titles the window with the project's name, `canoa`.
    await windowManager.setTitle(Brand.name);
    await windowManager.setPreventClose(true);
    windowManager.addListener(this);
    _hasTray = await _trayAvailable();
    if (_hasTray) {
      trayManager.addListener(this);
      // The mark in the look's accent, drawn when asked: Windows loads the tray icon with LoadImage(IMAGE_ICON), which takes no
      // PNG, and picks the .ico's entry for the display's small-icon size; Linux and macOS take the one PNG.
      await trayManager.setIcon(await _iconFile(_accent));
      // A StatusNotifier icon on Linux has no tooltip, and tray_manager has no such call there.
      if (!Platform.isLinux) await trayManager.setToolTip(_tooltip(0));
      await trayManager.setContextMenu(menu = _menu(0));
    }
    ready = true;
    // Sign-in's own entry asks for no window, whatever the person chose for a start they made themselves.
    if (config.startHidden || background) await _putAway();
    await _applyLoginStart(config);
  }

  /// The tray's menu, whose first item says how many sessions wait for the person.
  Menu _menu(int waiting) => Menu(items: [
    MenuItem(key: 'open', label: waiting == 0 ? _text('tray.open') : _text('tray.open_waiting', {'count': waiting})),
    MenuItem(key: 'hosting', label: _text('tray.hosting')),
    MenuItem(key: 'settings', label: _text('tray.settings')),
    MenuItem(key: 'hide', label: _text('tray.hide')),
    MenuItem.separator(),
    MenuItem(key: 'quit', label: _text('tray.quit')),
  ]);

  String _tooltip(int waiting) => waiting == 0 ? _text('tray.tooltip') : _text('tray.tooltip_waiting', {'count': waiting});

  @override
  Future<void> applyAppearance({required AppText text, required Color accent}) async {
    final recolour = accent != _accent;
    if (text == _text && !recolour) return;
    _text = text;
    _accent = accent;
    if (!ready || !_hasTray) return;
    if (recolour) await trayManager.setIcon(await _iconFile(accent));
    if (!Platform.isLinux) await trayManager.setToolTip(_tooltip(waiting));
    await trayManager.setContextMenu(menu = _menu(waiting));
  }

  /// How many sessions the tray last said wait for the person.
  @visibleForTesting
  int waiting = 0;

  @override
  Future<void> showWaiting(int count) async {
    if (count == waiting) return;
    waiting = count;
    if (!ready || !_hasTray) return;
    // A StatusNotifier icon on Linux has no tooltip, so there the menu says it.
    if (!Platform.isLinux) await trayManager.setToolTip(_tooltip(count));
    await trayManager.setContextMenu(menu = _menu(count));
  }

  /// Out of the way, where the person can bring it back from: the tray, or the taskbar without one.
  Future<void> _putAway() => _hasTray ? windowManager.hide() : windowManager.minimize();

  @override
  Future<void> update(CanoaConfig settings) async {
    final before = config.startAtLogin;
    config = settings;
    if (ready && settings.startAtLogin != before) await _applyLoginStart(settings);
  }

  /// Makes the computer hold what the person chose; nothing is registered until the first run or the person chose.
  Future<void> _applyLoginStart(CanoaConfig settings) async {
    final chosen = settings.startAtLogin;
    if (chosen != null) await setStartAtLogin(chosen);
  }

  @override
  Future<String?> setStartAtLogin(bool on) async {
    if (!_desktop) return 'This is not a desktop window.';
    final entry = _loginStart ?? systemLoginStart();
    if (entry == null) return 'This computer has no place to register a start at sign-in.';
    try {
      return await entry.set(on);
    } on Object catch (error) {
      return '$error';
    }
  }
  @override
  Future<void> open() async {
    if (!ready) return;
    await windowManager.show();
    await windowManager.restore();
    await windowManager.focus();
  }
  @override
  void onTrayIconMouseDown() {
    // tray_manager reports each click, and no double click.
    final now = _clock();
    if (lastClick != null && now.difference(lastClick!) <= doubleClick) {
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
  void onTrayMenuItemClick(MenuItem menuItem) {
    switch (menuItem.key) {
      case 'open': unawaited(open());
      case 'hosting': unawaited(open().then((_) => onHosting?.call()));
      case 'settings': unawaited(open().then((_) => onSettings?.call()));
      case 'hide': unawaited(windowManager.hide());
      case 'quit': unawaited(quit());
    }
  }
  @override
  void onWindowClose() {
    if (quitting) return;
    if (config.closeToTray) {
      unawaited(_putAway());
    } else {
      unawaited(quit());
    }
  }
  @override
  void onWindowMinimize() {
    if (config.minimizeToTray && _hasTray) unawaited(windowManager.hide());
  }
  @override
  Future<void> quit() async {
    if (quitting) return;
    quitting = true;
    try {
      await beforeQuit?.call();
    } finally {
      await dispose();
      await windowManager.destroy();
    }
  }
  @override
  Future<void> dispose() async {
    if (!ready) return;
    ready = false;
    windowManager.removeListener(this);
    if (_hasTray) {
      trayManager.removeListener(this);
      await trayManager.destroy();
    }
  }
}
