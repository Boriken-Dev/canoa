import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';

import 'tab_bridge.dart';

@JS('canoaBridge.tabs')
external JSObject? get _tabs;

@JS('canoaBridge.tabs.call')
external JSPromise<JSString> _call(JSString name, JSString args);

@JS('canoaBridge.client')
external JSString? get _client;

@JS('canoaBridge.tabs.watch')
external void _watch(JSFunction callback);

TabBridge? platformTabBridge() => _tabs == null ? null : _WebTabBridge();

final class _WebTabBridge implements TabBridge {
  _WebTabBridge() {
    _watch((() => _changes.add(null)).toJS);
  }

  final _changes = StreamController<void>.broadcast();

  @override
  String get client => _client?.toDart ?? panelClient;

  Future<String> _run(String name, List<Object?> args) async {
    try {
      return (await _call(name.toJS, jsonEncode(args).toJS).toDart).toDart;
    } on Object catch (error) {
      throw StateError(_reason(error));
    }
  }

  /// The page's own words for a refusal, without the JavaScript wrapper around them.
  static String _reason(Object error) => '$error'.replaceFirst(RegExp(r'^.*?Error: '), '');

  @override
  Future<TabSnapshot> snapshot() async => TabSnapshot.fromJson(await _run('snapshot', const []));

  @override
  Future<void> attach(String sessionKey, int tabId) => _run('attach', [sessionKey, tabId]);

  @override
  Future<void> move(String sessionKey, int tabId) => _run('move', [sessionKey, tabId]);

  @override
  Future<void> detach(String sessionKey) => _run('detach', [sessionKey]);

  @override
  Future<void> share(String sessionKey, int tabId, String peerId) => _run('share', [sessionKey, tabId, peerId]);

  @override
  Future<void> unshare(String sessionKey, int tabId) => _run('unshare', [sessionKey, tabId]);

  @override
  Stream<void> get changes => _changes.stream;
}
