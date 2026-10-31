import 'dart:js_interop';

@JS('canoaBridge')
external JSObject? get _canoaBridge;

@JS('canoaBridge.claimPendingContext')
external JSPromise<JSString?> _claim();

Future<String?> claimExtensionPageContext() async {
  if (_canoaBridge == null) return null;
  try {
    return (await _claim().toDart)?.toDart;
  } catch (_) {
    return null;
  }
}
