import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:local_notifier/local_notifier.dart';
import 'package:window_manager/window_manager.dart';

import '../brand.dart';
import 'notifier.dart';

Notifier createNotifier() => DesktopNotifier();

/// Windows' toasts and Linux's freedesktop notifications, through libnotify. Nothing is prepared until the first one is shown,
/// so a person who never gets one leaves no trace: on Windows the first one creates a Start menu shortcut named Canoa to this
/// program, which Windows needs to show a toast for a program not installed from a package.
final class DesktopNotifier implements Notifier {
  void Function(String sessionId)? _onOpen;
  bool? _ready;

  /// The notifications shown and not yet chosen or closed, which keep their callbacks alive.
  final Set<LocalNotification> _shown = {};

  @override
  Future<void> initialize(void Function(String sessionId) onOpen) async => _onOpen = onOpen;

  Future<bool> _prepare() async {
    if (_ready != null) return _ready!;
    try {
      await localNotifier.setup(appName: Brand.name);
      _ready = true;
    } on Object catch (error) {
      debugPrint('notifications are unavailable: $error');
      _ready = false;
    }
    return _ready!;
  }

  @override
  Future<bool> get available async => Platform.isWindows || Platform.isLinux;

  @override
  Future<bool> requestPermission() async => true;

  Future<bool> _attended() async {
    try {
      return await windowManager.isVisible() && !await windowManager.isMinimized() && await windowManager.isFocused();
    } on Object {
      return true;
    }
  }

  @override
  void watchAttention(void Function(bool attended) onChange) {
    final watcher = _Watcher(() async => onChange(await _attended()));
    try {
      windowManager.addListener(watcher);
    } on Object catch (error) {
      debugPrint('the window cannot be watched: $error');
    }
    unawaited(_attended().then(onChange));
  }

  @override
  Future<void> show({required String title, required String body, required String sessionId}) async {
    if (!await available || !await _prepare()) return;
    final notification = LocalNotification(title: title, body: body);
    notification
      ..onClick = () {
        _shown.remove(notification);
        _onOpen?.call(sessionId);
      }
      ..onClose = (_) => _shown.remove(notification);
    _shown.add(notification);
    try {
      await notification.show();
    } on Object catch (error) {
      _shown.remove(notification);
      debugPrint('a notification was not shown: $error');
    }
  }
}

/// Asks again whether the window is looked at each time its focus or its state changes.
final class _Watcher with WindowListener {
  _Watcher(this._changed);

  final void Function() _changed;

  @override
  void onWindowFocus() => _changed();

  @override
  void onWindowBlur() => _changed();

  @override
  void onWindowMinimize() => _changed();

  @override
  void onWindowRestore() => _changed();

  @override
  void onWindowEvent(String eventName) {
    if (eventName == 'show' || eventName == 'hide') _changed();
  }
}
