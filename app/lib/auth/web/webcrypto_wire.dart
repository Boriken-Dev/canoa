// The WebCrypto method's wire and session (dev.boriken.coqui.peer-webcrypto/v1): a mutual P-256 key proof ending in an
// AES-GCM session, as coqui-peer-webcrypto's PROTOCOL.md specifies. Public data is compared with `==`; every secret
// comparison is WebCrypto's (signature verification, AEAD tags).
import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';
import 'dart:typed_data';

import 'package:web/web.dart' as web;

import 'peer_session.dart';
import 'subtle.dart';

const profile = 'dev.boriken.coqui.peer-webcrypto/v1';
const defaultMaxMessage = 3072;
// What one direction may carry before a new handshake: 2^38, against the 2^38.5 bytes of RFC 8446
// section 5.5's 2^24.5 full-size AES-GCM records. Each record counts its length and 32. Written out,
// since a shift is 32-bit when compiled to JavaScript.
const defaultSessionLimit = 274877906944;

const _version = 1;
const nonceLength = 32;
const pointLength = 65;
const _signatureLength = 64;
const spkiLength = 91;
const _innerData = 1;
const _innerClose = 2;
const _tagLength = 16;
// A close record: its one inner byte and the tag. It stays possible at the limit.
const _closeLength = 1 + _tagLength;
const _recordOverhead = 32;
// Records are numbered in 53 bits, the integers every Dart target holds exactly; the 2^38 byte limit ends a direction long
// before that many records.
const _lastSequence = 9007199254740991;
// A P-256 SubjectPublicKeyInfo holding an uncompressed point is this prefix and 64 coordinate bytes, so comparing it leaves
// no ASN.1 for two parsers to read differently.
const _spkiPrefix = <int>[
  0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, //
  0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04,
];
final _peerIdPattern = RegExp(r'^sha256:[a-f0-9]{64}$');
final _b64Pattern = RegExp(r'^[A-Za-z0-9_-]+$');

bool isPeerId(String? text) => text != null && _peerIdPattern.hasMatch(text);

Algorithm _ecdsa() => Algorithm(name: 'ECDSA', namedCurve: 'P-256');
Algorithm _ecdh() => Algorithm(name: 'ECDH', namedCurve: 'P-256');
Algorithm _sign() => Algorithm(name: 'ECDSA', hash: 'SHA-256');

const invalidMessage = 'Invalid peer message';

/// The bytes of base64url [text] of exactly [length] bytes in its one canonical encoding.
Uint8List un64(String? text, int length) {
  if (text == null || !_b64Pattern.hasMatch(text) || text.length != (length * 4 / 3).ceil()) {
    throw const PeerFailure(invalidMessage);
  }
  final Uint8List bytes;
  try {
    bytes = base64Url.decode(base64Url.normalize(text));
  } on FormatException {
    throw const PeerFailure(invalidMessage);
  }
  if (bytes.length != length || b64(bytes) != text) throw const PeerFailure(invalidMessage);
  return bytes;
}

String _hex(Uint8List bytes) => [for (final byte in bytes) byte.toRadixString(16).padLeft(2, '0')].join();

/// The peer id of a SubjectPublicKeyInfo: its SHA-256.
Future<String> peerIdOf(Uint8List spki) async => 'sha256:${_hex(await digest256(spki))}';

bool _hasSpkiShape(Uint8List der) {
  if (der.length != spkiLength) return false;
  for (var i = 0; i < _spkiPrefix.length; i++) {
    if (der[i] != _spkiPrefix[i]) return false;
  }
  return true;
}

/// The verification key of an encoded SubjectPublicKeyInfo.
Future<web.CryptoKey> verifyKey(String encoded) async {
  final der = un64(encoded, spkiLength);
  if (!_hasSpkiShape(der)) throw const PeerFailure(invalidMessage);
  try {
    return await subtle().importKey('spki', js(der), _ecdsa(), false, usages(['verify'])).toDart;
  } catch (_) {
    throw const PeerFailure(invalidMessage);
  }
}

Future<web.CryptoKey> ephemeralKey(String encoded) async {
  final point = un64(encoded, pointLength);
  if (point[0] != 4) throw const PeerFailure(invalidMessage);
  try {
    return await subtle().importKey('raw', js(point), _ecdh(), false, usages(const [])).toDart;
  } catch (_) {
    throw const PeerFailure(invalidMessage);
  }
}

Uint8List _lengthPrefixed(List<Uint8List> fields) {
  final output = Uint8List(fields.fold<int>(0, (total, field) => total + 4 + field.length));
  final view = ByteData.sublistView(output);
  var offset = 0;
  for (final field in fields) {
    view.setUint32(offset, field.length);
    offset += 4;
    output.setAll(offset, field);
    offset += field.length;
  }
  return output;
}

/// What each side puts in a hello or a challenge: its key, its nonce and its ephemeral point, each base64url.
final class Offer {
  const Offer({required this.publicKey, required this.nonce, required this.ephemeral});
  final String publicKey;
  final String nonce;
  final String ephemeral;
}

/// The length-prefixed fields signed, and with no [role] the session id's input.
Uint8List _encodeFields(String? role, Offer hello, Offer challenge, Uint8List binding) => _lengthPrefixed([
  utf8.encode(profile),
  if (role != null) utf8.encode(role),
  un64(hello.publicKey, spkiLength),
  un64(challenge.publicKey, spkiLength),
  un64(hello.nonce, nonceLength),
  un64(challenge.nonce, nonceLength),
  un64(hello.ephemeral, pointLength),
  un64(challenge.ephemeral, pointLength),
  binding,
]);

Future<String> signFields(web.CryptoKey key, String role, Offer hello, Offer challenge, Uint8List binding) async =>
    b64(bytesOfBuffer(await subtle().sign(_sign(), key, js(_encodeFields(role, hello, challenge, binding))).toDart));

Future<bool> verifyFields(
  web.CryptoKey key,
  String signature,
  String role,
  Offer hello,
  Offer challenge,
  Uint8List binding,
) async {
  final proof = un64(signature, _signatureLength);
  final verified = await subtle().verify(_sign(), key, js(proof), js(_encodeFields(role, hello, challenge, binding))).toDart;
  return (verified as JSBoolean).toDart;
}

/// One encoding only: a message is refused unless it is exactly what this gives for its values.
String pack(String type, Map<String, String> contents) =>
    jsonEncode({'profile': profile, 'version': _version, 'type': type, ...contents});

/// The [members] of a [type] message, which has to be exactly what [pack] gives for them.
Map<String, String> parseMessage(Uint8List raw, String type, List<String> members, int maxMessage) {
  // Dart's decoder drops a byte-order mark that JSON does not allow.
  if (raw.length >= 3 && raw[0] == 0xef && raw[1] == 0xbb && raw[2] == 0xbf) throw const PeerFailure(invalidMessage);
  final String text;
  try {
    text = utf8.decode(raw);
  } on FormatException {
    throw const PeerFailure(invalidMessage);
  }
  if (raw.length > maxMessage) throw const PeerFailure('Oversized peer message');
  final Object? value;
  try {
    value = jsonDecode(text);
  } on FormatException {
    throw const PeerFailure(invalidMessage);
  }
  if (value is! Map<String, Object?>) throw const PeerFailure(invalidMessage);
  final contents = <String, String>{};
  for (final member in members) {
    final field = value[member];
    // Base64url only: the two encoders then agree on every byte of a message that can be exact.
    if (field is! String || !_b64Pattern.hasMatch(field)) throw const PeerFailure(invalidMessage);
    contents[member] = field;
  }
  if (pack(type, contents) != text) throw const PeerFailure(invalidMessage);
  return contents;
}

Future<Uint8List> _hkdf(web.CryptoKey secret, Uint8List info, int bytes) =>
    deriveBits(Algorithm(name: 'HKDF', hash: 'SHA-256', salt: Uint8List(0).toJS, info: js(info)), secret, bytes);

/// A new ephemeral ECDH key: the private half stays in WebCrypto, the public point leaves base64url.
Future<({web.CryptoKey privateKey, String encoded})> newEphemeral() async {
  final pair = (await subtle().generateKey(_ecdh(), false, usages(['deriveBits'])).toDart)! as KeyPair;
  final point = bytesOfBuffer(await subtle().exportKey('raw', pair.publicKey).toDart);
  return (privateKey: pair.privateKey, encoded: b64(point));
}

String newNonce() => b64(randomBytes(nonceLength));

/// The session both ends hold after the handshake: an id, and keys only they can derive.
Future<PeerSession> createSession({
  required web.CryptoKey ownEphemeral,
  required String peerEphemeral,
  required Offer hello,
  required Offer challenge,
  required Uint8List binding,
  required bool initiator,
  int sessionLimit = defaultSessionLimit,
}) async {
  final id = await digest256(_encodeFields(null, hello, challenge, binding));
  final shared = await deriveBits(
    Algorithm(name: 'ECDH', public: await ephemeralKey(peerEphemeral)),
    ownEphemeral,
    32,
  );
  final master = await importRaw(shared, Algorithm(name: 'HKDF'), ['deriveBits']);
  shared.fillRange(0, shared.length, 0);
  final info = _lengthPrefixed([utf8.encode('$profile/keys'), id]);
  final material = await _hkdf(master, info, 64);
  Future<web.CryptoKey> aes(Uint8List bytes) => importRaw(bytes, Algorithm(name: 'AES-GCM'), ['encrypt', 'decrypt']);
  final initiatorKey = await aes(Uint8List.sublistView(material, 0, 32));
  final responderKey = await aes(Uint8List.sublistView(material, 32, 64));
  material.fillRange(0, material.length, 0);
  final status = _Status();
  return _Session(
    sending: _Direction(initiator ? initiatorKey : responderKey, 'encrypt', sessionLimit, status),
    receiving: _Direction(initiator ? responderKey : initiatorKey, 'decrypt', sessionLimit, status),
    status: status,
  );
}

final class _Status {
  bool failed = false;
  bool destroyed = false;
}

final class _Session implements PeerSession {
  _Session({required this.sending, required this.receiving, required this.status});

  final _Direction sending;
  final _Direction receiving;
  final _Status status;
  bool _closed = false;
  bool _peerClosed = false;

  @override
  Future<Uint8List> seal(Uint8List message) {
    final bytes = Uint8List.fromList(message);
    if (_closed) return Future.error(PeerFailure('This side already closed its direction'));
    final inner = Uint8List(bytes.length + 1)
      ..[0] = _innerData
      ..setAll(1, bytes);
    return sending.run(inner, inner.length + _tagLength);
  }

  @override
  Future<Uint8List> close() {
    if (_closed) return Future.error(PeerFailure('This side already closed its direction'));
    _closed = true;
    return sending.run(Uint8List.fromList([_innerClose]), 0);
  }

  @override
  Future<Uint8List?> open(Uint8List record) async {
    final bytes = Uint8List.fromList(record);
    final inner = await receiving.run(bytes, bytes.length > _closeLength ? bytes.length : 0);
    if (_peerClosed) {
      status.failed = true;
      throw PeerFailure('The session is over');
    }
    if (inner.isNotEmpty && inner[0] == _innerData) return Uint8List.sublistView(inner, 1);
    if (inner.length == 1 && inner[0] == _innerClose) {
      _peerClosed = true;
      return null;
    }
    status.failed = true;
    throw PeerFailure('A session record does not open');
  }

  @override
  void destroy() {
    status.destroyed = true;
    sending.key = null;
    receiving.key = null;
  }
}

/// One direction. Calls take their sequence number, and settle, in call order.
final class _Direction {
  _Direction(this.key, this._operation, this._limit, this._state);

  web.CryptoKey? key;
  final String _operation;
  final int _limit;
  final _Status _state;
  int _sequence = 0;
  int _charged = 0;
  Future<void> _queue = Future<void>.value();

  Future<Uint8List> run(Uint8List input, int length) {
    if (_state.destroyed) key = null;
    if (length > 0) {
      final total = _charged + length + _recordOverhead;
      if (total > _limit) return Future.error(PeerFailure('This direction reached its session limit'));
      _charged = total;
    }
    final number = _sequence;
    _sequence += 1;
    final result = _queue.then((_) async {
      final cipherKey = key;
      if (_state.failed || _state.destroyed || cipherKey == null || number >= _lastSequence) {
        throw PeerFailure('The session is over');
      }
      final iv = Uint8List(12);
      ByteData.sublistView(iv)
        ..setUint32(4, number ~/ 4294967296)
        ..setUint32(8, number % 4294967296);
      final algorithm = Algorithm(name: 'AES-GCM', iv: iv.toJS, tagLength: 128);
      try {
        final output = _operation == 'encrypt'
            ? await subtle().encrypt(algorithm, cipherKey, js(input)).toDart
            : await subtle().decrypt(algorithm, cipherKey, js(input)).toDart;
        return bytesOfBuffer(output);
      } catch (_) {
        _state.failed = true;
        throw PeerFailure('A session record does not open');
      }
    });
    _queue = result.then<void>((_) {}, onError: (Object _) {});
    return result;
  }
}
