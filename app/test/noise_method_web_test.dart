// The Noise method in a real browser: coqui-peer-noise as WebAssembly, the key a non-extractable WebCrypto X25519 key in
// IndexedDB, a proof against a responder of the same wire, and the checks around them.
//
// The module comes from tool/build_noise.py --test-peer, which adds the responder these tests need (into test/noise):
//   python tool/build_noise.py --test-peer
//   flutter test --platform chrome test/noise_method_web_test.dart   (CHROME_EXECUTABLE may name Edge or Chromium)
@TestOn('browser')
library;

import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import 'package:canoa/auth/web/noise_method.dart';
import 'package:canoa/auth/web/peer_session.dart';
import 'package:canoa/auth/web/subtle.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:web/web.dart' as web;

import 'fixtures/noise_responder.dart';

Uint8List text(String value) => Uint8List.fromList(utf8.encode(value));
String string(Uint8List bytes) => utf8.decode(bytes);

Matcher failure([Pattern? message]) => throwsA(
  message == null ? isA<PeerFailure>() : isA<PeerFailure>().having((e) => e.message, 'message', contains(message)),
);

Future<({PeerSession client, ServerSession server, String clientId, String serverId})> prove({String? pinned}) async {
  final client = await NoiseIdentity.generate();
  final side = await ResponderSide.create();
  final serverId = side.peerId;
  final server = side.admit(client.peerId);
  final run = await client.prove(pinned ?? serverId);
  var step = await run.step(null);
  expect(step.send.single, hasLength(32));
  final answers = server.step(step.send.single.toJS).toDart;
  step = await run.step(answers.single.toDart);
  expect(step.send.single, hasLength(64));
  expect(step.peerId, serverId);
  server.step(step.send.single.toJS);
  expect(server.peerId, client.peerId);
  return (client: step.session!, server: server.takeSession()!, clientId: client.peerId, serverId: serverId);
}

/// The record the extension's JavaScript keeps for a Noise key: the id is the SHA-256 of the SubjectPublicKeyInfo.
Future<JSObject> storedRecord({bool extractable = false, String? peerId}) async {
  final pair = (await subtle().generateKey(Algorithm(name: 'X25519'), extractable, usages(['deriveBits'])).toDart)! as KeyPair;
  final raw = bytesOfBuffer(await subtle().exportKey('raw', pair.publicKey).toDart);
  final spki = Uint8List.fromList([0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x03, 0x21, 0x00, ...raw]);
  final digest = await digest256(spki);
  final id = 'sha256:${[for (final byte in digest) byte.toRadixString(16).padLeft(2, '0')].join()}';
  return JSObject()
    ..['method'] = 'noise'.toJS
    ..['peerId'] = (peerId ?? id).toJS
    ..['publicKey'] = b64(spki).toJS
    ..['privateKey'] = pair.privateKey;
}

Future<void> keep(String database, JSObject record) async {
  final opening = web.window.indexedDB.open(database, 1);
  opening.onupgradeneeded = ((web.Event _) => (opening.result! as web.IDBDatabase).createObjectStore('keys')).toJS;
  final db = (await requestResult(opening))! as web.IDBDatabase;
  await requestResult(db.transaction('keys'.toJS, 'readwrite').objectStore('keys').add(record, 'default'.toJS));
  db.close();
}

Future<JSObject> kept(String database) async {
  final opening = web.window.indexedDB.open(database, 1);
  final db = (await requestResult(opening))! as web.IDBDatabase;
  final record = await requestResult(db.transaction('keys'.toJS).objectStore('keys').get('default'.toJS));
  db.close();
  return record! as JSObject;
}

void main() {
  test('the module loads, and the page can run the method', () async {
    expect(await noiseMissing(), isNull);
  });

  test('both ends prove their keys, then speak in sealed records, each direction apart', () async {
    final ends = await prove();
    expect(ends.clientId, matches(RegExp(r'^sha256:[a-f0-9]{64}$')));
    expect(ends.clientId, isNot(ends.serverId));
    final forward = await ends.client.seal(text('hello'));
    expect(string(ends.server.open(forward.toJS)!.toDart), 'hello');
    final back = ends.server.seal(text('back').toJS).toDart;
    expect(string((await ends.client.open(back))!), 'back');
    final close = await ends.client.close();
    expect(ends.server.open(close.toJS), isNull);
  });

  test('a record altered, replayed or out of order does not open', () async {
    final ends = await prove();
    final one = await ends.client.seal(text('one'));
    final two = await ends.client.seal(text('two'));
    expect(() => ends.server.open(two.toJS), throwsA(anything));
    expect(() => ends.server.open(one.toJS), throwsA(anything), reason: 'the session is over');

    final fresh = await prove();
    final sealed = await fresh.client.seal(text('intact'));
    sealed[sealed.length - 1] ^= 1;
    expect(() => fresh.server.open(sealed.toJS), throwsA(anything));
    await expectLater(fresh.client.open(sealed), failure());
  });

  test('the dialer trusts only the id it pinned', () async {
    final other = await NoiseIdentity.generate();
    await expectLater(prove(pinned: other.peerId), throwsA(anything));
  });

  test('a step out of place or a message that is not the next one ends the proof', () async {
    final client = await NoiseIdentity.generate();
    final id = (await ResponderSide.create()).peerId;
    final run = await client.prove(id);
    await run.step(null);
    await expectLater(run.step(null), failure('without a message'));
    final again = await client.prove(id);
    await again.step(null);
    await expectLater(again.step(Uint8List(96)), failure());
    await expectLater(again.step(Uint8List(96)), failure());
    run.abort();
    await expectLater(run.step(null), failure());
  });

  test('a destroyed session holds no key', () async {
    final ends = await prove();
    ends.client.destroy();
    await expectLater(ends.client.seal(text('x')), failure());
  });

  group('the pairing', () {
    test('refuses a PIN that is not six digits before anything is sent', () async {
      final client = await NoiseIdentity.generate();
      await expectLater(client.pair('abc'), failure());
      await expectLater(client.pair(''), failure());
    });

    test('sends its first message, and a reply that is not the handshake ends it', () async {
      final client = await NoiseIdentity.generate();
      final run = await client.pair('000000');
      expect((await run.step(null)).send.single, hasLength(32));
      await expectLater(run.step(Uint8List(96)), failure());
      await expectLater(run.step(Uint8List(96)), failure());
    });
  });

  group('the identity', () {
    test('is a non-extractable X25519 key that cannot be exported', () async {
      final identity = await NoiseIdentity.generate();
      expect(identity.privateKey.extractable, isFalse);
      expect(identity.privateKey.type, 'private');
      for (final format in ['raw', 'pkcs8', 'jwk']) {
        await expectLater(subtle().exportKey(format, identity.privateKey).toDart, throwsA(anything), reason: format);
      }
    });

    test('is kept by the origin as the extension keeps it, the same one every time it is asked for', () async {
      final name = 'canoa-noise-test-${DateTime.now().microsecondsSinceEpoch}';
      final first = await NoiseIdentity.loadOrCreate(name);
      final second = await NoiseIdentity.loadOrCreate(name);
      expect(second.peerId, first.peerId);
      expect(second.privateKey.extractable, isFalse);
      final record = await kept(name);
      expect([for (final member in ['method', 'peerId', 'publicKey', 'privateKey']) record.has(member)], everyElement(isTrue));
      expect((record['peerId']! as JSString).toDart, first.peerId);
      expect((await NoiseIdentity.loadOrCreate('$name-other')).peerId, isNot(first.peerId));
    });

    test('reuses a key an earlier version stored, with the id it had', () async {
      final name = 'canoa-noise-test-old-${DateTime.now().microsecondsSinceEpoch}';
      final old = await storedRecord();
      await keep(name, old);
      final identity = await NoiseIdentity.loadOrCreate(name);
      expect(identity.peerId, (old['peerId']! as JSString).toDart);
      expect(identity.privateKey.extractable, isFalse);
    });

    test('is refused unless it is a non-extractable X25519 key of the id it names', () async {
      expect((await NoiseIdentity.fromStored(await storedRecord())).privateKey.extractable, isFalse);
      await expectLater(NoiseIdentity.fromStored(await storedRecord(extractable: true)), failure());
      await expectLater(NoiseIdentity.fromStored(await storedRecord(peerId: 'sha256:${'0' * 64}')), failure());
      final good = await storedRecord();
      await expectLater(NoiseIdentity.fromStored(good..['publicKey'] = 'AAAA'.toJS), failure());
      final ecdh = (await subtle().generateKey(Algorithm(name: 'ECDH', namedCurve: 'P-256'), false, usages(['deriveBits'])).toDart)!
          as KeyPair;
      await expectLater(NoiseIdentity.fromStored((await storedRecord())..['privateKey'] = ecdh.privateKey), failure());
      await expectLater(NoiseIdentity.fromStored(JSObject()), failure());
      await expectLater(NoiseIdentity.fromStored(null), failure());
    });
  });
}
