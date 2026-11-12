// The WebCrypto method in a real browser's WebCrypto and IndexedDB, against the other side of its own wire.
//
//   flutter test --platform chrome test/webcrypto_method_web_test.dart   (CHROME_EXECUTABLE may name Edge or Chromium)
@TestOn('browser')
library;

import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import 'package:canoa/auth/web/peer_session.dart';
import 'package:canoa/auth/web/subtle.dart';
import 'package:canoa/auth/web/webcrypto_method.dart';
import 'package:canoa/auth/web/webcrypto_wire.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:web/web.dart' as web;

import 'fixtures/webcrypto_responder.dart';

Uint8List text(String value) => Uint8List.fromList(utf8.encode(value));
String string(Uint8List bytes) => utf8.decode(bytes);

Matcher failure([String? message]) => throwsA(
  message == null ? isA<PeerFailure>() : isA<PeerFailure>().having((e) => e.message, 'message', message),
);

/// A client and a responder that admits it, and the session each ends with.
Future<({PeerSession client, PeerSession server, String clientId, String serverId})> prove({
  bool admit = true,
  String? pinned,
}) async {
  final clientKey = await WebCryptoIdentity.generate();
  final serverKey = await WebCryptoIdentity.generate();
  final initiator = await WebCryptoInitiator.create(clientKey, pinned ?? serverKey.peerId);
  final responder = WebCryptoResponder.create(serverKey, allowClient: (id) async => admit && id == clientKey.peerId);
  final challenge = await responder.challenge(text(initiator.hello()));
  final proof = await initiator.answer(text(challenge));
  final accepted = await responder.accept(text(proof));
  final finished = await initiator.finish(text(accepted.reply));
  expect(finished.peerId, serverKey.peerId);
  expect(accepted.peerId, clientKey.peerId);
  return (client: finished.session, server: accepted.session, clientId: clientKey.peerId, serverId: serverKey.peerId);
}

void main() {
  test('both ends prove their keys and then speak in sealed records, each direction apart', () async {
    final ends = await prove();
    expect(ends.clientId, matches(RegExp(r'^sha256:[a-f0-9]{64}$')));
    expect(ends.clientId, isNot(ends.serverId));

    final forward = await ends.client.seal(text('hello'));
    expect(string((await ends.server.open(forward))!), 'hello');
    final back = await ends.server.seal(text('back'));
    expect(string((await ends.client.open(back))!), 'back');
    // A record is bound to its direction: this side's own record does not open on this side.
    final mine = await ends.client.seal(text('mine'));
    await expectLater(ends.client.open(mine), failure());
  });

  test('a record that arrives out of order, twice or altered ends the session', () async {
    final ends = await prove();
    final first = await ends.client.seal(text('one'));
    final second = await ends.client.seal(text('two'));
    await expectLater(ends.server.open(second), failure());
    await expectLater(ends.server.open(first), failure(), reason: 'the session is over');

    final fresh = await prove();
    final record = await fresh.client.seal(text('once'));
    expect(string((await fresh.server.open(record))!), 'once');
    await expectLater(fresh.server.open(record), failure(), reason: 'a replay');

    final altered = await prove();
    final sealed = await altered.client.seal(text('intact'));
    sealed[sealed.length - 1] ^= 1;
    await expectLater(altered.server.open(sealed), failure());
  });

  test('a close record ends one direction, and the other goes on', () async {
    final ends = await prove();
    final close = await ends.client.close();
    expect(await ends.server.open(close), isNull);
    await expectLater(ends.client.seal(text('late')), failure());
    await expectLater(ends.client.close(), failure());
    final back = await ends.server.seal(text('still'));
    expect(string((await ends.client.open(back))!), 'still');
  });

  test('a destroyed session holds no key', () async {
    final ends = await prove();
    ends.client.destroy();
    await expectLater(ends.client.seal(text('x')), failure());
  });

  test('the initiator trusts only the id it pinned', () async {
    final other = await WebCryptoIdentity.generate();
    await expectLater(prove(pinned: other.peerId), failure('Responder is not pinned'));
    await expectLater(WebCryptoInitiator.create(await WebCryptoIdentity.generate(), 'not-an-id'), failure());
  });

  test('a responder that does not admit the client ends the exchange before an acceptance', () async {
    await expectLater(prove(admit: false), failure('Client is not paired'));
  });

  group('a message is refused unless it is exactly what the method packs', () {
    Future<(WebCryptoInitiator, String)> challenged() async {
      final client = await WebCryptoIdentity.generate();
      final server = await WebCryptoIdentity.generate();
      final initiator = await WebCryptoInitiator.create(client, server.peerId);
      final responder = WebCryptoResponder.create(server, allowClient: (_) async => true);
      return (initiator, await responder.challenge(text(initiator.hello())));
    }

    Future<void> refused(Uint8List message, [String why = 'Invalid peer message']) async {
      final (initiator, _) = await challenged();
      await expectLater(initiator.answer(message), failure(why));
    }

    test('a byte-order mark, whitespace, another member order or an extra member', () async {
      final (_, challenge) = await challenged();
      await refused(Uint8List.fromList([0xef, 0xbb, 0xbf, ...text(challenge)]));
      await refused(text(' $challenge'));
      final members = jsonDecode(challenge) as Map<String, Object?>;
      await refused(text(jsonEncode({...members, 'extra': 'AAAA'})));
      await refused(text(jsonEncode(Map.fromEntries(members.entries.toList().reversed))));
    });

    test('malformed UTF-8, JSON that is not an object, a wrong type and an oversized message', () async {
      await refused(Uint8List.fromList([0xc0, 0x80]));
      await refused(text('[]'));
      await refused(text('null'));
      final (_, challenge) = await challenged();
      await refused(text(challenge.replaceFirst('"type":"challenge"', '"type":"hello"')));
      await refused(Uint8List(defaultMaxMessage + 1)..fillRange(0, defaultMaxMessage + 1, 0x20), 'Oversized peer message');
    });

    test('a proof signed by the wrong key', () async {
      final (initiator, challenge) = await challenged();
      final members = (jsonDecode(challenge) as Map<String, Object?>).cast<String, String>();
      final forged = pack('challenge', {
        for (final key in ['publicKey', 'ephemeral', 'nonce']) key: members[key]!,
        'signature': 'A' * 86,
      });
      await expectLater(initiator.answer(text(forged)), failure('Responder key proof failed'));
    });

    test('a step out of place', () async {
      final (initiator, challenge) = await challenged();
      await expectLater(initiator.finish(text(challenge)), failure('Unexpected acceptance'));
      final fresh = await WebCryptoInitiator.create(
        await WebCryptoIdentity.generate(),
        (await WebCryptoIdentity.generate()).peerId,
      );
      await expectLater(fresh.answer(text(challenge)), failure('Unexpected challenge'));
      fresh.hello();
      expect(fresh.hello, throwsA(isA<PeerFailure>()));
    });
  });

  group('the identity', () {
    test('is kept by the origin, the same one every time it is asked for', () async {
      final name = 'canoa-webcrypto-test-${DateTime.now().microsecondsSinceEpoch}';
      final first = await WebCryptoIdentity.loadOrCreate(name);
      final second = await WebCryptoIdentity.loadOrCreate(name);
      expect(second.peerId, first.peerId);
      expect(first.privateKey.extractable, isFalse);
      expect(first.privateKey.type, 'private');
      final other = await WebCryptoIdentity.loadOrCreate('$name-other');
      expect(other.peerId, isNot(first.peerId));
    });

    test('is refused unless it is a non-extractable P-256 signing key of the id it names', () async {
      final good = await WebCryptoIdentity.generate();
      expect((await WebCryptoIdentity.fromStored(good.toStored())).peerId, good.peerId);

      final extractable =
          (await subtle().generateKey(Algorithm(name: 'ECDSA', namedCurve: 'P-256'), true, usages(['sign', 'verify'])).toDart)!
              as KeyPair;
      final stored = good.toStored()..['privateKey'] = extractable.privateKey;
      await expectLater(WebCryptoIdentity.fromStored(stored), failure());

      final wrongId = good.toStored()..['peerId'] = 'sha256:${'0' * 64}'.toJS;
      await expectLater(WebCryptoIdentity.fromStored(wrongId), failure());
      final noKey = JSObject()..['peerId'] = good.peerId.toJS;
      await expectLater(WebCryptoIdentity.fromStored(noKey), failure());
      await expectLater(WebCryptoIdentity.fromStored(null), failure());

      final ecdh =
          (await subtle().generateKey(Algorithm(name: 'ECDH', namedCurve: 'P-256'), false, usages(['deriveBits'])).toDart)!
              as KeyPair;
      final wrongKind = good.toStored()..['privateKey'] = ecdh.privateKey;
      await expectLater(WebCryptoIdentity.fromStored(wrongKind), failure());
    });

    test('needs a secure context, which a test page has', () {
      expect(subtleMissing(), isNull);
      expect(web.window.indexedDB, isNotNull);
    });
  });
}
