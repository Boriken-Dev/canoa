// The Noise method: coqui-peer-noise compiled to WebAssembly (app/web_noise, built by tool/build_noise.py), and this origin's
// non-extractable WebCrypto X25519 key, which the WebAssembly never holds: a proof or PIN pairing is run a step at a time,
// and WebCrypto computes the one Diffie-Hellman that uses the private key between two calls into the module.
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import 'package:web/web.dart' as web;

import 'peer_session.dart';
import 'subtle.dart';

/// The loader beside the page: `noise/canoa_noise.js` and the `.wasm` it fetches from its own directory.
const noiseLoader = 'noise/canoa_noise.js';

const _unusable = 'A non-extractable X25519 identity is required';
// RFC 8410's SubjectPublicKeyInfo of an id-X25519 key is this prefix and the 32 key bytes.
const _spkiPrefix = <int>[0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x03, 0x21, 0x00];
const _keyLength = 32;
final _peerIdPattern = RegExp(r'^sha256:[a-f0-9]{64}$');

extension type _NoiseKey._(JSObject _) implements JSObject {
  external String get peerId;
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
  external JSUint8Array? get pendingDh;
  external JSArray<JSUint8Array> provideDh(JSUint8Array shared);
  external String? get peerId;
  external JSUint8Array? get peerKey;
  external String? get failure;
  external _NoiseSession? takeSession();
  external void free();
}

/// The text of a JavaScript error thrown across the WebAssembly boundary, as a failure a person can read.
PeerFailure noiseFailure(Object error) {
  if (error is PeerFailure) return error;
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

Algorithm _x25519([web.CryptoKey? peer]) => Algorithm(name: 'X25519', public: peer);

/// Whether this browser's WebCrypto computes X25519 on a non-extractable key. Without it Noise is unavailable, never weaker.
Future<bool> _hasX25519() async {
  try {
    final pair = (await subtle().generateKey(_x25519(), false, usages(['deriveBits'])).toDart)! as KeyPair;
    final other = (await subtle().generateKey(_x25519(), false, usages(['deriveBits'])).toDart)! as KeyPair;
    final shared = await deriveBits(_x25519(other.publicKey), pair.privateKey, _keyLength);
    return shared.length == _keyLength;
  } catch (_) {
    return false;
  }
}

Future<bool>? _x25519Works;

/// Why this page cannot run the Noise method, or null. Asked before any connection.
Future<String?> noiseMissing({Uri? base}) async {
  if (!(web.window as JSObject).hasProperty('indexedDB'.toJS).toDart) return 'IndexedDB storage is unavailable';
  if (subtleMissing() != null) return subtleMissing();
  if (!await (_x25519Works ??= _hasX25519())) {
    return "this browser's WebCrypto has no X25519, which the Noise method needs";
  }
  try {
    await loadNoise(base: base);
    return null;
  } on PeerFailure catch (failure) {
    return 'the Noise module did not load (${failure.message})';
  }
}

JSObject _class(JSObject module, String name) => module[name]! as JSObject;

/// This page's Noise key: an X25519 key whose private half cannot leave WebCrypto. The WebAssembly knows only the public half.
final class NoiseIdentity {
  NoiseIdentity._(this._key, this.privateKey, this._spki);

  final _NoiseKey _key;
  final Uint8List _spki;

  /// The private key, as WebCrypto holds it: `extractable` is false and it cannot be exported.
  final web.CryptoKey privateKey;

  String get peerId => _key.peerId;

  /// This identity's SubjectPublicKeyInfo: the public half only.
  Uint8List get spki => Uint8List.fromList(_spki);

  static T _wrap<T>(T Function() call) {
    try {
      return call();
    } catch (error) {
      throw noiseFailure(error);
    }
  }

  static Future<NoiseIdentity> _of(Uint8List raw, web.CryptoKey privateKey, {Uri? base}) async {
    final module = await loadNoise(base: base);
    final key = _wrap(() => _class(module, 'NoiseKey').callMethod<_NoiseKey>('fromPublicKey'.toJS, raw.toJS));
    return NoiseIdentity._(key, privateKey, Uint8List.fromList([..._spkiPrefix, ...raw]));
  }

  /// A new identity whose private key cannot leave WebCrypto.
  static Future<NoiseIdentity> generate({Uri? base}) async {
    final pair = (await subtle().generateKey(_x25519(), false, usages(['deriveBits'])).toDart)! as KeyPair;
    final raw = bytesOfBuffer(await subtle().exportKey('raw', pair.publicKey).toDart);
    return _of(raw, pair.privateKey, base: base);
  }

  /// [stored] as an identity, or a [PeerFailure] when it is not exactly such a key: the object holds a non-extractable X25519
  /// private key and the id of the public key it names.
  static Future<NoiseIdentity> fromStored(JSAny? stored, {Uri? base}) async {
    if (stored.isUndefinedOrNull || !stored.isA<JSObject>()) throw const PeerFailure(_unusable);
    final object = stored as JSObject;
    final key = object['privateKey'];
    final publicKey = object['publicKey'];
    final peerId = object['peerId'];
    if (!publicKey.isA<JSString>() || !peerId.isA<JSString>() || !key.isA<web.CryptoKey>()) {
      throw const PeerFailure(_unusable);
    }
    final privateKey = key as web.CryptoKey;
    final algorithm = privateKey.algorithm;
    final names = (privateKey.usages as JSArray<JSString>).toDart.map((usage) => usage.toDart);
    final id = (peerId as JSString).toDart;
    if (privateKey.type != 'private' ||
        privateKey.extractable ||
        (algorithm['name'] as JSString?)?.toDart != 'X25519' ||
        !names.contains('deriveBits') ||
        !_peerIdPattern.hasMatch(id)) {
      throw const PeerFailure(_unusable);
    }
    final Uint8List spki;
    try {
      spki = base64Url.decode(base64Url.normalize((publicKey as JSString).toDart));
    } on FormatException {
      throw const PeerFailure(_unusable);
    }
    if (spki.length != _spkiPrefix.length + _keyLength) throw const PeerFailure(_unusable);
    for (var i = 0; i < _spkiPrefix.length; i++) {
      if (spki[i] != _spkiPrefix[i]) throw const PeerFailure(_unusable);
    }
    final identity = await _of(Uint8List.sublistView(spki, _spkiPrefix.length), privateKey, base: base);
    if (identity.peerId != id) {
      identity._key.free();
      throw const PeerFailure(_unusable);
    }
    return identity;
  }

  /// The identity kept in the origin-local IndexedDB database [name], created when absent. The record is the one the
  /// extension's JavaScript keeps: its private key is a non-extractable CryptoKey.
  static Future<NoiseIdentity> loadOrCreate(String name, {Uri? base}) async {
    final opening = web.window.indexedDB.open(name, 1);
    opening.onupgradeneeded = ((web.Event _) => (opening.result! as web.IDBDatabase).createObjectStore('keys')).toJS;
    final db = (await requestResult(opening))! as web.IDBDatabase;
    Future<JSAny?> read() => requestResult(db.transaction('keys'.toJS).objectStore('keys').get('default'.toJS));
    try {
      var stored = await read();
      if (stored.isUndefinedOrNull) {
        final fresh = await generate(base: base);
        // Another tab may get there first: `add` then fails, and every tab uses the one stored.
        try {
          await requestResult(db.transaction('keys'.toJS, 'readwrite').objectStore('keys').add(fresh.toStored(), 'default'.toJS));
        } catch (_) {}
        fresh._key.free();
        stored = await read();
      }
      return await fromStored(stored, base: base);
    } finally {
      db.close();
    }
  }

  /// The stored object: the four members the extension's JavaScript keeps.
  JSObject toStored() => JSObject()
    ..['method'] = 'noise'.toJS
    ..['peerId'] = peerId.toJS
    ..['publicKey'] = b64(_spki).toJS
    ..['privateKey'] = privateKey;

  /// X25519 of the private key with [point] (32 bytes): WebCrypto refuses a result of all zeros.
  Future<Uint8List> _agree(Uint8List point) async {
    try {
      final peer = await subtle().importKey('raw', js(point), _x25519(), false, usages(const [])).toDart;
      return await deriveBits(_x25519(peer), privateKey, _keyLength);
    } catch (_) {
      throw const PeerFailure('The static key could not compute the handshake');
    }
  }

  /// X25519 of the private key with the one point of a secret binding's other end, which the caller has already checked is the host
  /// the connection proved. The key stays in WebCrypto.
  Future<Uint8List> agreeSecret(Uint8List point) => _agree(point);

  /// A proof of the agent [expectedPeerId], and no other.
  Future<NoiseRun> prove(String expectedPeerId, {Uri? base}) async {
    final module = await loadNoise(base: base);
    return NoiseRun._(
      _wrap(() => _class(module, 'NoiseRun').callMethod<_NoiseRun>('prove'.toJS, _key, expectedPeerId.toJS)),
      this,
    );
  }

  /// A pairing of this key by the typed [pin].
  Future<NoiseRun> pair(String pin, {Uri? base}) async {
    final module = await loadNoise(base: base);
    return NoiseRun._(_wrap(() => _class(module, 'NoiseRun').callMethod<_NoiseRun>('pair'.toJS, _key, pin.toJS)), this);
  }

  /// Drops the public key from WebAssembly memory.
  void dispose() => _key.free();
}

/// What a step gives: the messages to send now, and once the run is over the id it proved or learned and, for a proof, the peer's static
/// public key (SubjectPublicKeyInfo) and the
/// session. A [failure] is a pairing refused: the messages are the refusal to send first, then the run is over.
typedef NoiseStep = ({List<Uint8List> send, String? peerId, Uint8List? peerKey, PeerSession? session, String? failure});

/// A proof or a pairing, a step at a time.
final class NoiseRun {
  NoiseRun._(this._run, this._identity);

  final _NoiseRun _run;
  final NoiseIdentity _identity;
  bool _freed = false;

  Future<NoiseStep> step(Uint8List? received) async {
    if (_freed) throw const PeerFailure('Peer proof expired or unknown');
    try {
      final sent = [for (final message in _run.step(received?.toJS).toDart) message.toDart];
      final point = _run.pendingDh;
      if (point != null) {
        // The one place the private key is used: WebCrypto computes it, and the module goes on with the result.
        final shared = await _identity._agree(point.toDart);
        try {
          sent.addAll([for (final message in _run.provideDh(shared.toJS).toDart) message.toDart]);
        } finally {
          shared.fillRange(0, shared.length, 0);
        }
      }
      final peer = _run.peerId;
      final session = peer == null ? null : _run.takeSession();
      return (
        send: sent,
        peerId: peer,
        peerKey: peer == null ? null : _run.peerKey?.toDart,
        session: session == null ? null : NoiseSessionHandle._(session),
        failure: _run.failure,
      );
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
      throw noiseFailure(error);
    }
  }

  @override
  Future<Uint8List?> open(Uint8List record) async {
    try {
      return _live.open(record.toJS)?.toDart;
    } catch (error) {
      throw noiseFailure(error);
    }
  }

  @override
  Future<Uint8List> close() async {
    try {
      return _live.close().toDart;
    } catch (error) {
      throw noiseFailure(error);
    }
  }

  @override
  void destroy() {
    _session?.free();
    _session = null;
  }
}
