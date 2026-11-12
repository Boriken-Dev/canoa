// The Noise method in a real browser: coqui-peer-noise as WebAssembly, the key in IndexedDB, a proof against a responder of
// the same wire, and the checks around them.
//
// The module comes from tool/build_noise.py --test-peer, which adds the responder these tests need:
//   python tool/build_noise.py --out web/noise --test-peer
//   flutter test --platform chrome test/noise_method_web_test.dart   (CHROME_EXECUTABLE may name Edge or Chromium)
@TestOn('browser')
library;

import 'dart:convert';
import 'dart:js_interop';
import 'dart:typed_data';

import 'package:canoa/auth/web/noise_method.dart';
import 'package:canoa/auth/web/peer_session.dart';
import 'package:flutter_test/flutter_test.dart';

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
  var step = run.step(null);
  expect(step.send.single, hasLength(32));
  var answers = server.step(step.send.single.toJS).toDart;
  step = run.step(answers.single.toDart);
  expect(step.send.single, hasLength(64));
  expect(step.peerId, serverId);
  server.step(step.send.single.toJS);
  expect(server.peerId, client.peerId);
  return (
    client: step.session!,
    server: server.takeSession()!,
    clientId: client.peerId,
    serverId: serverId,
  );
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
    final run = await client.prove((await ResponderSide.create()).peerId);
    run.step(null);
    expect(() => run.step(null), throwsA(isA<PeerFailure>().having((e) => e.message, 'message', contains('without a message'))));
    final again = await client.prove((await ResponderSide.create()).peerId);
    again.step(null);
    expect(() => again.step(Uint8List(96)), throwsA(isA<PeerFailure>()));
    expect(() => again.step(Uint8List(96)), throwsA(isA<PeerFailure>()));
    expect(() => run.step(Uint8List(96)), throwsA(isA<PeerFailure>()));
    run.abort();
    expect(() => run.step(null), throwsA(isA<PeerFailure>()));
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
      expect(run.step(null).send.single, hasLength(32));
      expect(() => run.step(Uint8List(96)), throwsA(isA<PeerFailure>()));
      expect(() => run.step(Uint8List(96)), throwsA(isA<PeerFailure>()));
    });
  });

  group('the identity', () {
    test('is kept by the origin, the same one every time it is asked for', () async {
      final name = 'canoa-noise-test-${DateTime.now().microsecondsSinceEpoch}';
      final first = await NoiseIdentity.loadOrCreate(name);
      final second = await NoiseIdentity.loadOrCreate(name);
      expect(second.peerId, first.peerId);
      expect(first.toPem(), second.toPem());
      expect((await NoiseIdentity.loadOrCreate('$name-other')).peerId, isNot(first.peerId));
    });

    test('is refused unless it is a Noise key of the id it names', () async {
      await expectLater(NoiseIdentity.fromPem('not a key'), failure());
      final good = await NoiseIdentity.generate();
      expect((await NoiseIdentity.fromPem(good.toPem())).peerId, good.peerId);
    });
  });
}
