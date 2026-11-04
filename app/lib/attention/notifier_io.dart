import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_local_notifications_linux/flutter_local_notifications_linux.dart';
import 'package:flutter_local_notifications_platform_interface/flutter_local_notifications_platform_interface.dart';
import 'package:flutter_local_notifications_windows/flutter_local_notifications_windows.dart';
import 'package:window_manager/window_manager.dart';

import 'notifier.dart';

Notifier createNotifier() => DesktopNotifier();

/// Windows' toasts and Linux's freedesktop notifications. Nothing is prepared until the first one is shown, so a person who
/// never gets one leaves no trace: on Windows the first one registers Canoa's app id, `dev.boriken.canoa`, under the current
/// user's registry, which Windows needs to show a toast for an unpackaged program.
final class DesktopNotifier implements Notifier {
  static const appId = 'dev.boriken.canoa';

  /// Names the activation callback Windows calls back; Canoa's own, fixed so a click finds the same program.
  static const _guid = '6b8f5f3e-0c2a-4d7e-9a1b-4c0a6d2e8f17';

  void Function(String sessionId)? _onOpen;
  FlutterLocalNotificationsWindows? _windows;
  LinuxFlutterLocalNotificationsPlugin? _linux;
  bool? _ready;
  int _next = 1;

  @override
  Future<void> initialize(void Function(String sessionId) onOpen) async => _onOpen = onOpen;

  void _chosen(NotificationResponse response) {
    final session = response.payload;
    if (session != null && session.isNotEmpty) _onOpen?.call(session);
  }

  Future<bool> _prepare() async {
    if (_ready != null) return _ready!;
    try {
      if (Platform.isWindows) {
        _windows = FlutterLocalNotificationsWindows();
        _ready = await _windows!.initialize(
          settings: const WindowsInitializationSettings(appName: 'Canoa', appUserModelId: appId, guid: _guid),
          onDidReceiveNotificationResponse: _chosen,
        );
      } else if (Platform.isLinux) {
        _linux = LinuxFlutterLocalNotificationsPlugin();
        _ready = await _linux!.initialize(
          settings: const LinuxInitializationSettings(defaultActionName: 'Open'),
          onDidReceiveNotificationResponse: _chosen,
        ) ?? false;
      } else {
        _ready = false;
      }
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
    if (!await _prepare()) return;
    final id = _next++;
    try {
      if (_windows != null) {
        await _windows!.show(id: id, title: title, body: body, payload: sessionId);
      } else {
        await _linux?.show(id: id, title: title, body: body, payload: sessionId);
      }
    } on Object catch (error) {
      debugPrint('a notification was not shown: $error');
    }
  }
}
