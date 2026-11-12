// The Noise method: coqui-peer-noise compiled to WebAssembly (app/web_noise, built by tool/build_noise.py), its key kept in
// this origin's IndexedDB, and the proof and PIN pairing run a step at a time.
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import 'package:web/web.dart' as web;

import 'peer_session.dart';
import 'subtle.dart' show requestResult;

/// The loader beside the page: `noise/canoa_noise.js` and the `.wasm` it fetches from its own directory.
const noiseLoader = 'noise/canoa_noise.js';

extension type _NoiseKey._(JSObject _) implements JSObject {
  external String get peer_id;
  external String to_pem();
  external void free();
}

extension type _NoiseSession._(JSObject _) implements JSObject {
  external JSUint8Array seal(JSUint8Array message);
  external JSUint8Array? open(JSUint8Array record);
  external JSUint8Array close();
  external void free();
}

extension type _NoiseRun._(JSObject _) implements JSObject {
  external JSArray<JSUint8Array> step(JSUint8Array? received);
  external String? get peer_id;
  external _NoiseSession? take_session();
  external void free();
}

/// The text of a JavaScript error thrown across the WebAssembly boundary, as a failure a person can read.
PeerFailure noiseFailure(Object error) {
  final text = error.toString();
  for (final prefix in const ['Error: ', 'Exception: ']) {
    if (text.startsWith(prefix)) return PeerFailure(text.substring(prefix.length));
  }
  return PeerFailure(text);
}

Future<JSObject>? _loading;

/// The module namespace once its WebAssembly is compiled, loaded once. [base] is the page the loader is resolved against.
Future<JSObject> loadNoise({Uri? base}) => _loading ??= () async {
  try {
    final url = (base ?? Uri.parse(web.document.baseURI)).resolve(noiseLoader);
    final module = await importModule(url.toString().toJS).toDart;
    await (module.callMethod<JSPromise<JSAny?>>('default'.toJS)).toDart;
    return module;
  } catch (error) {
    _loading = null;
    throw noiseFailure(error);
  }
}();

/// Why this page cannot run the Noise method, or null. Asked before any connection.
Future<String?> noiseMissing({Uri? base}) async {
  if (!(web.window as JSObject).hasProperty('indexedDB'.toJS).toDart) return 'IndexedDB storage is unavailable';
  try {
    await loadNoise(base: base);
    return null;
  } on PeerFailure catch (failure) {
    return 'the Noise module did not load (${failure.message})';
  }
}

JSObject _class(JSObject module, String name) => module[name]! as JSObject;

/// This page's Noise key. Its secret is held as PKCS#8 text in this origin's IndexedDB: unlike the WebCrypto key it can be
/// read by script in the page, since the Noise code that uses it runs in WebAssembly.
final class NoiseIdentity {
  NoiseIdentity._(this._key);

  final _NoiseKey _key;

  String get peerId => _key.peer_id;

  /// The key as text, for a store.
  String toPem() => _wrap(() => _key.to_pem());

  static T _wrap<T>(T Function() call) {
    try {
      return call();
    } catch (error) {
      throw noiseFailure(error);
    }
  }

  static Future<NoiseIdentity> generate({Uri? base}) async {
    final module = await loadNoise(base: base);
    return NoiseIdentity._(_wrap(() => _class(module, 'NoiseKey').callMethod<_NoiseKey>('generate'.toJS)));
  }

  static Future<NoiseIdentity> fromPem(String pem, {Uri? base}) async {
    final module = await loadNoise(base: base);
    return NoiseIdentity._(_wrap(() => _class(module, 'NoiseKey').callMethod<_NoiseKey>('from_pem'.toJS, pem.toJS)));
  }

  /// The identity kept in the origin-local IndexedDB database [name], created when absent.
  static Future<NoiseIdentity> loadOrCreate(String name, {Uri? base}) async {
    final opening = web.window.indexedDB.open(name, 1);
    opening.onupgradeneeded = ((web.Event _) => (opening.result! as web.IDBDatabase).createObjectStore('keys')).toJS;
    final db = (await requestResult(opening))! as web.IDBDatabase;
    Future<JSAny?> read() => requestResult(db.transaction('keys'.toJS).objectStore('keys').get('default'.toJS));
    try {
      var stored = await read();
      if (stored.isUndefinedOrNull) {
        final fresh = await generate(base: base);
        final record = JSObject()
          ..['method'] = 'noise'.toJS
          ..['peerId'] = fresh.peerId.toJS
          ..['pem'] = fresh.toPem().toJS;
        // Another tab may get there first: `add` then fails, and every tab uses the one stored.
        try {
          await requestResult(db.transaction('keys'.toJS, 'readwrite').objectStore('keys').add(record, 'default'.toJS));
        } catch (_) {}
        fresh._key.free();
        stored = await read();
      }
      if (!stored.isA<JSObject>()) throw const PeerFailure('A Noise identity is required');
      final object = stored as JSObject;
      final pem = object['pem'];
      final id = object['peerId'];
      if (!pem.isA<JSString>() || !id.isA<JSString>()) throw const PeerFailure('A Noise identity is required');
      final identity = await fromPem((pem as JSString).toDart, base: base);
      if (identity.peerId != (id as JSString).toDart) {
        identity._key.free();
        throw const PeerFailure('A Noise identity is required');
      }
      return identity;
    } finally {
      db.close();
    }
  }

  /// A proof of the agent [expectedPeerId], and no other.
  Future<NoiseRun> prove(String expectedPeerId, {Uri? base}) async {
    final module = await loadNoise(base: base);
    return NoiseRun._(
      _wrap(() => _class(module, 'NoiseRun').callMethod<_NoiseRun>('prove'.toJS, _key, expectedPeerId.toJS)),
    );
  }

  /// A pairing of this key by the typed [pin].
  Future<NoiseRun> pair(String pin, {Uri? base}) async {
    final module = await loadNoise(base: base);
    return NoiseRun._(_wrap(() => _class(module, 'NoiseRun').callMethod<_NoiseRun>('pair'.toJS, _key, pin.toJS)));
  }

  /// Drops the key from WebAssembly memory.
  void dispose() => _key.free();
}

/// What a step gives: the messages to send now, and once the run is over the id it proved or learned and, for a proof, the
/// session.
typedef NoiseStep = ({List<Uint8List> send, String? peerId, PeerSession? session});

/// A proof or a pairing, a step at a time.
final class NoiseRun {
  NoiseRun._(this._run);

  final _NoiseRun _run;
  bool _freed = false;

  NoiseStep step(Uint8List? received) {
    if (_freed) throw const PeerFailure('Peer proof expired or unknown');
    try {
      final sent = [for (final message in _run.step(received?.toJS).toDart) message.toDart];
      final peer = _run.peer_id;
      final session = peer == null ? null : _run.take_session();
      return (send: sent, peerId: peer, session: session == null ? null : NoiseSessionHandle._(session));
    } catch (error) {
      throw noiseFailure(error);
    }
  }

  /// Ends the run and drops its state.
  void abort() {
    if (_freed) return;
    _freed = true;
    _run.free();
  }
}

/// The Noise session a proof ends in.
final class NoiseSessionHandle implements PeerSession {
  NoiseSessionHandle._(this._session);

  _NoiseSession? _session;

  _NoiseSession get _live => _session ?? (throw const PeerFailure('Session closed or unknown'));

  @override
  Future<Uint8List> seal(Uint8List message) async {
    try {
      return _live.seal(message.toJS).toDart;
    } catch (error) {
      throw error is PeerFailure ? error : noiseFailure(error);
    }
  }

  @override
  Future<Uint8List?> open(Uint8List record) async {
    try {
      return _live.open(record.toJS)?.toDart;
    } catch (error) {
      throw error is PeerFailure ? error : noiseFailure(error);
    }
  }

  @override
  Future<Uint8List> close() async {
    try {
      return _live.close().toDart;
    } catch (error) {
      throw error is PeerFailure ? error : noiseFailure(error);
    }
  }

  @override
  void destroy() {
    _session?.free();
    _session = null;
  }
}
