import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'notifier.dart';

Notifier createNotifier() => WebNotifier();

@JS('Notification')
extension type _Notification._(JSObject _) implements JSObject {
  external factory _Notification(String title, _Options options);
  external static String get permission;
  external static JSPromise<JSString> requestPermission();
  external set onclick(JSFunction? handler);
  external void close();
}

extension type _Options._(JSObject _) implements JSObject {
  external factory _Options({String body, String tag});
}

@JS('document.visibilityState')
external String get _visibility;

@JS('document.hasFocus')
external bool _hasFocus();

@JS('window.focus')
external void _focusWindow();

/// The browser's Notification API, once the person allowed it from a click of theirs. Nothing here registers a service worker:
/// a notification is shown by the page while it is open, and choosing one focuses the page.
final class WebNotifier implements Notifier {
  void Function(String sessionId)? _onOpen;

  static bool get _supported => globalContext.has('Notification');

  @override
  Future<void> initialize(void Function(String sessionId) onOpen) async => _onOpen = onOpen;

  @override
  Future<bool> get available async => _supported && _Notification.permission == 'granted';

  @override
  Future<bool> requestPermission() async {
    if (!_supported || _Notification.permission == 'denied') return false;
    return (await _Notification.requestPermission().toDart).toDart == 'granted';
  }

  @override
  Future<bool> attended() async => _visibility == 'visible' && _hasFocus();

  @override
  Future<void> show({required String title, required String body, required String sessionId}) async {
    if (!await available) return;
    final shown = _Notification(title, _Options(body: body, tag: 'canoa-$sessionId'));
    shown.onclick = (() {
      _focusWindow();
      shown.close();
      _onOpen?.call(sessionId);
    }).toJS;
  }
}
