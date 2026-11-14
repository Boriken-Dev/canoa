// Secret prompts on a connection this app's own Noise key proved: the answer is bound by the Diffie-Hellman of the two static keys,
// for that one host, and a connection a Noise key did not prove offers no Noise binding.
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/noise_secret.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp/secret.dart';
import 'package:coqui_acp/secret_unprotected.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/secret_host.dart';

final appSecret = List<int>.generate(32, (i) => i + 5);
final hostSecret = List<int>.generate(32, (i) => 90 - i);
final strangerSecret = List<int>.generate(32, (i) => i * 7 + 2);

/// What the app's Noise key does with a point: the one operation its non-extractable key is used for.
final agreeApp = x25519Agreement(appSecret);

Future<SecretIdentity?> identityFor(List<int> hostKey, {String? id, bool lie = false, Future<Uint8List> Function(Uint8List)? agree}) async {
  final hostSpki = await x25519Spki(hostKey);
  return noiseSecretIdentity(
    hostId: id ?? AcpIdentityId.of(hostSpki).toString(),
    hostSpki: lie ? await x25519Spki(strangerSecret) : hostSpki,
    spki: await x25519Spki(appSecret),
    agree: agree ?? agreeApp,
  );
}

/// A request as a host makes it: its receiver bound to its static key for the app.
Future<Map<String, Object?>> noiseRequest(AcpSecretProver host, AcpIdentityId app, String id) async {
  final context = AcpSecretContext(sessionId: 's1', requestId: id, message: 'The deploy needs your registry login', schema: schema);
  final receiver = await AcpSecretReceiver.create(AcpSecretMethod.request, context, host, app, now);
  receivers[id] = receiver;
  return AcpSecretRequestParams(context: context, recipient: receiver.recipient).toJson();
}

/// A connection to the stand-in host whose proof, as the app's own Noise key makes it, left [secrets] on the VerifiedPeer.
Future<(SecretHost, ChatSession, VerifiedAgent)> proven(WidgetTester tester, SessionStore store, String hostId, {NoiseSecrets? secrets}) async {
  final host = SecretHost();
  final (session, client) = await tester.runAsync(() async {
    final client = await VerifiedAgent.open(
      host,
      expectedPeerId: hostId,
      verifyPeer: (channel, expected) async {
        return VerifiedPeer(expected, channel, noise: secrets);
      },
      decider: store.askPermission,
      elicitor: store.askQuestion,
      secrets: store,
    );
    await client.initialize();
    await client.openSession(cwd: '/work');
    final session = store.addProviderSession(title: 'Deploy', agent: 'Claude', peerId: client.peerId, remoteSessionId: 's1');
    store.attachVerifiedClient(session.id, client, record: false);
    store.select(session.id);
    return (session, client);
  }) as (ChatSession, VerifiedAgent);
  return (host, session, client);
}

Object? offered(SecretHost host) {
  final params = host.initialize!['params'] as Map;
  final capabilities = (params['clientCapabilities'] ?? params['capabilities']) as Map;
  return (capabilities['_meta'] as Map?)?[acpSecretsCapability];
}

void main() {
  test('the identity is for the one host whose static key the proof gave, and agrees with no other point', () async {
    final identity = (await identityFor(hostSecret))!;
    expect(identity.offer.bindings, [acpBindingDh]);
    final appSpki = await x25519Spki(appSecret);
    expect(identity.prover.id, AcpIdentityId.of(appSpki));
    // The key the proof gave must be an X25519 key whose digest is the id the app pinned.
    expect(await identityFor(hostSecret, id: AcpIdentityId.of(await x25519Spki(strangerSecret)).toString()), isNull);
    expect(await identityFor(hostSecret, lie: true), isNull);
    expect(noiseSecretIdentity(hostId: 'sha256:${'a' * 64}', hostSpki: null, spki: appSpki, agree: agreeApp), isNull);
    expect(noiseSecretIdentity(hostId: 'not an id', hostSpki: Uint8List(44), spki: appSpki, agree: agreeApp), isNull);

    // The function the identity agrees with refuses every point but the host's: not even a host the verifier knows is agreed with.
    var asked = 0;
    final guarded = (await identityFor(hostSecret, agree: (point) {
      asked++;
      return agreeApp(point);
    }))!;
    final stranger = await x25519Prover(strangerSecret, appSpki);
    guarded.verifier.trust(stranger.id);
    final context = AcpSecretContext(sessionId: 's1', requestId: 'g1', message: 'login', schema: schema);
    final receiver = await AcpSecretReceiver.create(AcpSecretMethod.request, context, stranger, guarded.prover.id, now);
    await expectLater(
      guarded.verifier.collector(stranger.id, AcpSecretMethod.request, context, receiver.recipient, now),
      throwsA(isA<AcpSecretException>()),
    );
    expect(asked, 0, reason: 'the key never agreed with the stranger');
  });

  test('an answer bound by the static keys opens for the host, and for no other', () async {
    final identity = (await identityFor(hostSecret))!;
    final appSpki = await x25519Spki(appSecret);
    final host = await x25519Prover(hostSecret, appSpki);
    final context = AcpSecretContext(sessionId: 's1', requestId: 'n1', message: 'login', schema: schema);
    final receiver = await AcpSecretReceiver.create(AcpSecretMethod.request, context, host, identity.prover.id, now);
    identity.verifier.trust(host.id);
    final collector = await identity.verifier.collector(host.id, AcpSecretMethod.request, context, receiver.recipient, now);
    final sealed = {'password': await collector.seal('password', password)};
    const plain = '{"user":"ana"}';
    final auth = await collector.authenticate(identity.prover, plain, sealed, now);
    expect(auth.method, acpBindingDh);
    final hostVerifier = AcpSecretVerifier(
      own: host.id, known: [identity.prover.id], agreementSpki: await x25519Spki(hostSecret), agree: x25519Agreement(hostSecret),
    );
    expect((await receiver.openValues(hostVerifier, identity.prover.id, plain, sealed, auth, now)).secrets['password']!.text, password);
    // Another static key cannot verify what was made for this host's.
    final otherVerifier = AcpSecretVerifier(
      own: AcpIdentityId.of(await x25519Spki(strangerSecret)), known: [identity.prover.id], agreementSpki: await x25519Spki(strangerSecret), agree: x25519Agreement(strangerSecret),
    );
    await expectLater(
      receiver.openValues(otherVerifier, identity.prover.id, plain, sealed, auth, now),
      throwsA(isA<AcpSecretException>()),
    );
  });

  testWidgets('a connection a Noise key proved offers the Diffie-Hellman binding alone, and its form is sealed under it', (tester) async {
    tester.view.physicalSize = const Size(1100, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final store = SessionStore()..secretIdentity = identity();
    final app = (await tester.runAsync(() => identityFor(hostSecret)))!;
    final hostSpki = (await tester.runAsync(() => x25519Spki(hostSecret)))!;
    final (host, session, client) = await proven(tester, store, AcpIdentityId.of(hostSpki).toString(), secrets: (identity: app));
    expect(client.noiseSecrets, same(app));
    expect((offered(host)! as Map)['bindings'], [acpBindingDh], reason: "the store's WebCrypto binding is not offered on a Noise link");
    await tester.pumpWidget(CanoaApp(store: store));
    await tester.pump();
    final hostProver = (await tester.runAsync(() => x25519Prover(hostSecret, app.prover.spki)))!;
    host.ask(7, acpSecretRequestMethod, (await tester.runAsync(() => noiseRequest(hostProver, app.prover.id, 'n7')))!);
    await settle(tester);
    expect(find.byType(SecretRequestCard), findsOneWidget);
    await until(tester, find.byKey(const ValueKey('question-user')));
    await tester.enterText(find.byKey(const ValueKey('question-user')), 'ana');
    await tester.enterText(find.byKey(const ValueKey('secret-password')), password);
    await tester.pump();
    await tester.tap(find.text('Send sealed'));
    await settle(tester);
    final result = host.answerTo(7)!['result'] as Map;
    expect(result['action'], 'accept');
    expect(result['auth']['method'], acpBindingDh);
    expect(host.sent.map(jsonEncode).join('\n'), isNot(contains(password)));
    expect(session.messages.map((message) => message.text).join('\n'), isNot(contains(password)));
    final reply = AcpSecretRequestReply.fromJson(result, (name) => name == 'password') as AcpSecretAccepted;
    final hostVerifier = AcpSecretVerifier(
      own: hostProver.id, known: [app.prover.id], agreementSpki: hostSpki, agree: x25519Agreement(hostSecret),
    );
    final opened = await tester.runAsync(() => receivers['n7']!.openValues(hostVerifier, app.prover.id, reply.plain, reply.sealed, reply.auth, now)) as AcpOpened;
    expect(opened.secrets['password']!.text, password);
  });

  testWidgets('a request bound by a key that is not the host the app proved is not drawn', (tester) async {
    tester.view.physicalSize = const Size(1100, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final store = SessionStore();
    final app = (await tester.runAsync(() => identityFor(hostSecret)))!;
    final hostSpki = (await tester.runAsync(() => x25519Spki(hostSecret)))!;
    final (host, session, _) = await proven(tester, store, AcpIdentityId.of(hostSpki).toString(), secrets: (identity: app));
    await tester.pumpWidget(CanoaApp(store: store));
    await tester.pump();
    // Whatever relays holds a static key of its own: its binding names the app, but it is not the host that was pinned.
    final relay = (await tester.runAsync(() => x25519Prover(strangerSecret, app.prover.spki)))!;
    host.ask(8, acpSecretRequestMethod, (await tester.runAsync(() => noiseRequest(relay, app.prover.id, 'n8')))!);
    await settle(tester);
    expect(find.byKey(const ValueKey('secret-password')), findsNothing);
    expect(session.pendingSecrets, isEmpty);
    final answer = host.answerTo(8)!;
    expect(answer['error'] != null || (answer['result'] as Map)['action'] == 'cancel', isTrue);
  });

  testWidgets('a Noise proof that gave no usable key offers the host nothing, and a link no Noise key proved offers no Noise binding', (tester) async {
    final store = SessionStore()..secretIdentity = identity();
    tester.view.physicalSize = const Size(1100, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final (none, noSession, noKey) = await proven(tester, store, 'sha256:${'b' * 64}', secrets: (identity: null));
    expect(noKey.noiseSecrets, isNull);
    await tester.pumpWidget(CanoaApp(store: store));
    await tester.pump();
    // Even a request bound under the store's own WebCrypto identity is not drawn: a Noise link never falls back to it.
    none.ask(9, acpSecretRequestMethod, (await tester.runAsync(() => requestParams('n9')))!);
    await settle(tester);
    expect(noSession.pendingSecrets, isEmpty);
    expect(find.byKey(const ValueKey('secret-password')), findsNothing);
    expect(jsonEncode(none.sent), isNot(contains('"enc"')));
    final refused = none.answerTo(9);
    expect(refused == null || refused['error'] != null, isTrue);
    expect(jsonEncode(none.initialize), isNot(contains(acpSecretsCapability)), reason: "not even the store's WebCrypto binding");
    // A link a provider, or anything but this app's Noise key, proved has no Noise binding to give.
    final (other, _, relayed) = await proven(tester, store, 'sha256:${'c' * 64}');
    expect(relayed.noiseSecrets, isNull);
    expect((offered(other)! as Map)['bindings'], isNot(contains(acpBindingDh)));
  });
}
