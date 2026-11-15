import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'local_host.dart';

@JS('canoaBridge')
external JSObject? get _canoaBridge;

JSObject? get _host => _canoaBridge?.getProperty<JSObject?>('host'.toJS);

@JS('canoaBridge.host.status')
external JSPromise<JSString> _status();

@JS('canoaBridge.host.start')
external JSPromise<JSString> _start();

@JS('canoaBridge.host.pair')
external JSPromise<JSString> _pair(JSString pin);

@JS('canoaBridge.host.unpair')
external JSPromise<JSString> _unpair();

/// The extension's bridge to the local Canoa; null in any page the extension did not make, which has no `canoaBridge` at all.
LocalHostBridge? platformLocalHostBridge() => _host == null ? null : _WebLocalHostBridge();

final class _WebLocalHostBridge implements LocalHostBridge {
  Future<LocalHostView> _view(JSPromise<JSString> call) async => LocalHostView.fromJson((await call.toDart).toDart);

  @override
  Future<LocalHostView> status() => _view(_status());

  @override
  Future<LocalHostView> start() => _view(_start());

  @override
  Future<LocalHostView> pair(String pin) => _view(_pair(pin.toJS));

  @override
  Future<LocalHostView> unpair() => _view(_unpair());
}
