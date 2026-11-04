import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:local_notifier/local_notifier.dart';
import 'package:window_manager/window_manager.dart';

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
      await localNotifier.setup(appName: 'Canoa');
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

  @override
  Future<bool> attended() async {
    try {
      return await windowManager.isVisible() && !await windowManager.isMinimized() && await windowManager.isFocused();
    } on Object {
      return true;
    }
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
