// The WebCrypto method's key and the side that dials: a P-256 identity whose private key never leaves WebCrypto, and the
// initiator of the proof (hello, the answer to the challenge, then the acceptance).
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import 'package:web/web.dart' as web;

import 'peer_session.dart';
import 'subtle.dart';
import 'webcrypto_wire.dart';

const _unusable = 'A non-extractable P-256 identity is required';

/// This page's WebCrypto key: its SubjectPublicKeyInfo (base64url), the id that hashes to, and the key itself.
final class WebCryptoIdentity {
  const WebCryptoIdentity._(this.peerId, this.publicKey, this.privateKey);

  final String peerId;
  final String publicKey;
  final web.CryptoKey privateKey;

  /// The key's SubjectPublicKeyInfo.
  Uint8List get spki => un64(publicKey, spkiLength);

  /// The ECDSA (P-256, SHA-256) signature of [message], 64 bytes as IEEE P1363, made inside WebCrypto: the private key is used and
  /// never read. It signs what a caller gives it, so only code that is told what it signs may hold it.
  Future<Uint8List> sign(Uint8List message) async =>
      bytesOfBuffer(await subtle().sign(Algorithm(name: 'ECDSA', hash: 'SHA-256'), privateKey, js(message)).toDart);

  /// The stored object, which holds the same four members the key's first home did.
  JSObject toStored() => JSObject()
    ..['method'] = 'webcrypto'.toJS
    ..['peerId'] = peerId.toJS
    ..['publicKey'] = publicKey.toJS
    ..['privateKey'] = privateKey;

  /// A new identity whose private key cannot leave WebCrypto.
  static Future<WebCryptoIdentity> generate() async {
    final pair = (await subtle().generateKey(Algorithm(name: 'ECDSA', namedCurve: 'P-256'), false, usages(['sign', 'verify'])).toDart)!
        as KeyPair;
    final spki = bytesOfBuffer(await subtle().exportKey('spki', pair.publicKey).toDart);
    return WebCryptoIdentity._(await peerIdOf(spki), b64(spki), pair.privateKey);
  }

  /// [stored] as an identity, or a [PeerFailure] when it is not exactly such a key.
  static Future<WebCryptoIdentity> fromStored(JSAny? stored) async {
    if (stored.isUndefinedOrNull || !stored.isA<JSObject>()) throw const PeerFailure(_unusable);
    final object = stored as JSObject;
    final key = object['privateKey'];
    final publicKey = object['publicKey'];
    final peerId = object['peerId'];
    if (!publicKey.isA<JSString>() || !peerId.isA<JSString>() || !key.isA<web.CryptoKey>()) {
      throw const PeerFailure(_unusable);
    }
    final privateKey = key as web.CryptoKey;
    final encoded = (publicKey as JSString).toDart;
    final id = (peerId as JSString).toDart;
    final algorithm = privateKey.algorithm;
    final names = (privateKey.usages as JSArray<JSString>).toDart.map((usage) => usage.toDart);
    if (privateKey.type != 'private' ||
        privateKey.extractable ||
        (algorithm['name'] as JSString?)?.toDart != 'ECDSA' ||
        (algorithm['namedCurve'] as JSString?)?.toDart != 'P-256' ||
        !names.contains('sign') ||
        !isPeerId(id)) {
      throw const PeerFailure(_unusable);
    }
    await verifyKey(encoded);
    if (await peerIdOf(un64(encoded, spkiLength)) != id) throw const PeerFailure(_unusable);
    return WebCryptoIdentity._(id, encoded, privateKey);
  }

  /// The identity kept in the origin-local IndexedDB database [name], created when absent. The private key is stored as a
  /// non-extractable CryptoKey.
  static Future<WebCryptoIdentity> loadOrCreate(String name) async {
    final opening = web.window.indexedDB.open(name, 1);
    opening.onupgradeneeded = ((web.Event _) => (opening.result! as web.IDBDatabase).createObjectStore('keys')).toJS;
    final db = (await requestResult(opening))! as web.IDBDatabase;
    Future<JSAny?> read() => requestResult(db.transaction('keys'.toJS).objectStore('keys').get('default'.toJS));
    try {
      var stored = await read();
      if (stored.isUndefinedOrNull) {
        final fresh = await generate();
        // Another tab may get there first: `add` then fails, and every tab uses the one stored.
        try {
          await requestResult(db.transaction('keys'.toJS, 'readwrite').objectStore('keys').add(fresh.toStored(), 'default'.toJS));
        } catch (_) {}
        stored = await read();
      }
      return await fromStored(stored);
    } finally {
      db.close();
    }
  }
}

/// The side that dials, trusting only [expectedPeerId]: call [hello], then [answer] with the challenge, then [finish] with
/// the acceptance.
final class WebCryptoInitiator {
  WebCryptoInitiator._(this._identity, this._expectedPeerId, this._maxMessage, this._binding, this._hello, this._ephemeralKey);

  final WebCryptoIdentity _identity;
  final String _expectedPeerId;
  final int _maxMessage;
  final Uint8List _binding;
  final Offer _hello;
  web.CryptoKey? _ephemeralKey;
  var _state = 'start';
  Offer? _challenge;
  web.CryptoKey? _peerKey;

  /// [binding] is bytes of an outer channel both ends really read (a TLS exporter where each ends that TLS itself, never
  /// through a reverse proxy); the proof signs it.
  static Future<WebCryptoInitiator> create(
    WebCryptoIdentity identity,
    String expectedPeerId, {
    int maxMessage = defaultMaxMessage,
    Uint8List? binding,
  }) async {
    if (!isPeerId(expectedPeerId)) throw const PeerFailure('A pinned responder id is required');
    if (maxMessage < 0) throw const PeerFailure('maxMessage must be a whole number');
    final ephemeral = await newEphemeral();
    final offer = Offer(publicKey: identity.publicKey, nonce: newNonce(), ephemeral: ephemeral.encoded);
    return WebCryptoInitiator._(identity, expectedPeerId, maxMessage, Uint8List.fromList(binding ?? Uint8List(0)), offer, ephemeral.privateKey);
  }

  /// The first message.
  String hello() {
    if (_state != 'start') throw const PeerFailure('Unexpected hello');
    _state = 'challenge';
    return pack('hello', {'publicKey': _hello.publicKey, 'ephemeral': _hello.ephemeral, 'nonce': _hello.nonce});
  }

  /// Checks the responder's challenge and answers it with this side's proof.
  Future<String> answer(Uint8List raw) async {
    if (_state != 'challenge') throw const PeerFailure('Unexpected challenge');
    _state = 'failed';
    final message = parseMessage(raw, 'challenge', const ['publicKey', 'ephemeral', 'nonce', 'signature'], _maxMessage);
    if (message['publicKey'] == _identity.publicKey) throw const PeerFailure('Invalid peer message');
    final key = await verifyKey(message['publicKey']!);
    final answered = Offer(publicKey: message['publicKey']!, nonce: message['nonce']!, ephemeral: message['ephemeral']!);
    await ephemeralKey(answered.ephemeral);
    if (!await verifyFields(key, message['signature']!, 'server', _hello, answered, _binding)) {
      throw const PeerFailure('Responder key proof failed');
    }
    if (await peerIdOf(un64(answered.publicKey, spkiLength)) != _expectedPeerId) {
      throw const PeerFailure('Responder is not pinned');
    }
    final signature = await signFields(_identity.privateKey, 'client', _hello, answered, _binding);
    _challenge = answered;
    _peerKey = key;
    _state = 'accept';
    return pack('proof', {'signature': signature});
  }

  /// Checks the responder's acceptance and gives the proved id with the session both ends now hold.
  Future<({String peerId, PeerSession session})> finish(Uint8List raw) async {
    if (_state != 'accept') throw const PeerFailure('Unexpected acceptance');
    _state = 'failed';
    final message = parseMessage(raw, 'accept', const ['signature'], _maxMessage);
    if (!await verifyFields(_peerKey!, message['signature']!, 'accept', _hello, _challenge!, _binding)) {
      throw const PeerFailure('Responder acceptance proof failed');
    }
    final session = await createSession(
      ownEphemeral: _ephemeralKey!,
      peerEphemeral: _challenge!.ephemeral,
      hello: _hello,
      challenge: _challenge!,
      binding: _binding,
      initiator: true,
    );
    _ephemeralKey = null;
    _peerKey = null;
    _state = 'done';
    return (peerId: _expectedPeerId, session: session);
  }
}

