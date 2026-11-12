// WebCrypto and IndexedDB through package:web: the pieces the peer methods share.
import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:math';
import 'dart:typed_data';

import 'package:web/web.dart' as web;

/// `crypto.subtle`, which a page has only in a secure context.
web.SubtleCrypto subtle() {
  final value = (web.window.crypto as JSObject)['subtle'];
  if (value.isUndefinedOrNull) throw StateError('Secure-context WebCrypto is required');
  return value as web.SubtleCrypto;
}

/// Why this page has no WebCrypto, or null.
String? subtleMissing() {
  final value = (web.window.crypto as JSObject)['subtle'];
  return value.isUndefinedOrNull ? 'WebCrypto needs a secure context (HTTPS or localhost)' : null;
}

/// A WebCrypto algorithm dictionary: only the members a call names are set.
extension type Algorithm._(JSObject _) implements JSObject {
  external factory Algorithm({
    required String name,
    String? namedCurve,
    String? hash,
    JSUint8Array? salt,
    JSUint8Array? info,
    JSUint8Array? iv,
    int? tagLength,
    web.CryptoKey? public,
  });
}

/// What `generateKey` answers for an asymmetric algorithm.
extension type KeyPair._(JSObject _) implements JSObject {
  external web.CryptoKey get publicKey;
  external web.CryptoKey get privateKey;
}

JSArray<JSString> usages(List<String> names) => [for (final name in names) name.toJS].toJS;

/// A copy of [bytes] as WebCrypto's input: a caller may reuse its buffer before the call settles.
JSUint8Array js(Uint8List bytes) => Uint8List.fromList(bytes).toJS;

Uint8List bytesOfBuffer(JSAny? buffer) => Uint8List.fromList((buffer! as JSArrayBuffer).toDart.asUint8List());

Future<Uint8List> digest256(Uint8List data) async =>
    bytesOfBuffer(await subtle().digest(Algorithm(name: 'SHA-256'), js(data)).toDart);

Future<Uint8List> deriveBits(Algorithm algorithm, web.CryptoKey key, int bytes) async =>
    Uint8List.fromList((await subtle().deriveBits(algorithm, key, bytes * 8).toDart).toDart.asUint8List());

Future<web.CryptoKey> importRaw(Uint8List bytes, Algorithm algorithm, List<String> keyUsages) =>
    subtle().importKey('raw', js(bytes), algorithm, false, usages(keyUsages)).toDart;

/// [count] bytes from the generator WebCrypto gives a page.
Uint8List randomBytes(int count) {
  final filled = Uint8List(count).toJS;
  web.window.crypto.getRandomValues(filled);
  return filled.toDart;
}

/// A fresh random token as lowercase hex.
String randomToken(int bytes) {
  final random = Random.secure();
  final buffer = StringBuffer();
  for (var i = 0; i < bytes; i++) {
    buffer.write(random.nextInt(256).toRadixString(16).padLeft(2, '0'));
  }
  return buffer.toString();
}

/// Base64url without padding.
String b64(Uint8List bytes) => base64Url.encode(bytes).replaceAll('=', '');

/// Waits for an `IDBRequest`.
Future<JSAny?> requestResult(web.IDBRequest request) {
  final done = Completer<JSAny?>();
  request.onsuccess = ((web.Event _) => done.complete(request.result)).toJS;
  request.onerror = ((web.Event _) => done.completeError(StateError(request.error?.message ?? 'IndexedDB failed'))).toJS;
  return done.future;
}
