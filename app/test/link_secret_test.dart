// Secret prompts on the link to the host in this app's own process: the app and the host share one process and one identity, so the
// answer is bound by the secret the host minted for the link, which never travels. Only that link takes the binding.
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/acp/client.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp/secret.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/secret_host.dart';

final linkSecret = Uint8List.fromList(List<int>.generate(32, (i) => 200 - i));
final otherSecret = Uint8List.fromList(List<int>.generate(32, (i) => i * 3 + 1));

/// The host's end of the link: its prover and the identity it makes requests for, from the secret it minted.
Future<({AcpLinkKey key, AcpSecretProver host, AcpIdentityId app})> hostEnd(Uint8List secret) async {
  final key = await AcpLinkKey.derive(secret);
  return (key: key, host: AcpSecretProver.linked(key, AcpLinkRole.host), app: key.identity(AcpLinkRole.app));
}

/// A connection to a stand-in host on the in-process link, with [linkSecrets] when the app has the secret, attached to a conversation.
Future<(SecretHost, ChatSession, VerifiedAgent)> linked(WidgetTester tester, SessionStore store, {LinkSecretIdentity? linkSecrets}) async {
  final host = SecretHost();
  final (session, client) = await tester.runAsync(() async {
    final client = VerifiedAgent.fromChild(
      label: 'Claude', messages: host.messages, send: host.send, close: host.close,
      decider: store.askPermission, elicitor: store.askQuestion, secrets: store, linkSecrets: linkSecrets,
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

Future<Map<String, Object?>> linkRequest(AcpSecretProver host, AcpIdentityId app, String id) async {
  final context = AcpSecretContext(sessionId: 's1', requestId: id, message: 'The deploy needs your registry login', schema: schema);
  final receiver = await AcpSecretReceiver.create(AcpSecretMethod.request, context, host, app, now);
  receivers[id] = receiver;
  return AcpSecretRequestParams(context: context, recipient: receiver.recipient).toJson();
}

void main() {
  testWidgets('the link offers only its own binding, and the app needs no identity of its own for it', (tester) async {
    tester.view.physicalSize = const Size(1100, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final store = SessionStore();
    expect(store.secretIdentity, isNull);
    final linkSecrets = await tester.runAsync(() => linkSecretIdentity(linkSecret)) as LinkSecretIdentity;
    final (host, _, _) = await linked(tester, store, linkSecrets: linkSecrets);
    final params = host.initialize!['params'] as Map;
    final capabilities = (params['clientCapabilities'] ?? params['capabilities']) as Map;
    expect((capabilities['_meta'] as Map)[acpSecretsCapability], {'version': 1, 'methods': ['request', 'show'], 'bindings': [acpBindingLink]});
    // A program this app started that has no such secret offers nothing.
    final (bare, _, _) = await linked(tester, SessionStore());
    expect(jsonEncode(bare.initialize), isNot(contains(acpSecretsCapability)));
  });

  testWidgets('a request on the link is a form; what is typed reaches the host sealed, bound by the link\'s secret', (tester) async {
    tester.view.physicalSize = const Size(1100, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final store = SessionStore();
    final linkSecrets = await tester.runAsync(() => linkSecretIdentity(linkSecret)) as LinkSecretIdentity;
    final (host, session, _) = await linked(tester, store, linkSecrets: linkSecrets);
    await tester.pumpWidget(CanoaApp(store: store));
    await tester.pump();
    final end = (await tester.runAsync(() => hostEnd(linkSecret)))!;
    expect(linkSecrets.host, end.key.identity(AcpLinkRole.host), reason: 'both ends name the host the same');
    host.ask(7, acpSecretRequestMethod, (await tester.runAsync(() => linkRequest(end.host, end.app, 'l1')))!);
    await settle(tester);
    expect(find.byType(SecretRequestCard), findsOneWidget);
    expect(host.sent.where((message) => message['id'] == 7), isEmpty, reason: 'the shared client never saw it');

    await until(tester, find.byKey(const ValueKey('question-user')));
    await tester.enterText(find.byKey(const ValueKey('question-user')), 'ana');
    await tester.enterText(find.byKey(const ValueKey('secret-password')), password);
    await tester.pump();
    await tester.tap(find.text('Send sealed'));
    await settle(tester);

    final answer = host.answerTo(7)!;
    expect(answer['error'], isNull);
    final result = answer['result'] as Map;
    expect(result['action'], 'accept');
    expect(result['auth']['method'], acpBindingLink);
    expect(host.sent.map(jsonEncode).join('\n'), isNot(contains(password)));
    expect(session.pendingSecrets, isEmpty);
    expect(session.messages.map((message) => message.text).join('\n'), isNot(contains(password)));
    // The sealed answer opens for the host holding the secret, and for no other end of another link.
    final reply = AcpSecretRequestReply.fromJson(result, (name) => name == 'password') as AcpSecretAccepted;
    final opened = await tester.runAsync(() => receivers['l1']!.openValues(
      AcpSecretVerifier(own: end.host.id, link: end.key, linkRole: AcpLinkRole.host), end.app, reply.plain, reply.sealed, reply.auth, now,
    )) as AcpOpened;
    expect(opened.secrets['password']!.text, password);
    final other = (await tester.runAsync(() => hostEnd(otherSecret)))!;
    await tester.runAsync(() async {
      try {
        await receivers['l1']!.openValues(
          AcpSecretVerifier(own: other.host.id, link: other.key, linkRole: AcpLinkRole.host), other.app, reply.plain, reply.sealed, reply.auth, now,
        );
        fail('opened under another link');
      } on AcpSecretException {
        // refused
      }
    });
  });

  testWidgets('a program with no link secret, and a request bound under another link\'s secret, are cancelled or refused', (tester) async {
    tester.view.physicalSize = const Size(1100, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final store = SessionStore();
    // The store's own identity is a web identity for hosts it has proven; the link's program never uses it.
    store.secretIdentity = identity();
    final (host, session, _) = await linked(tester, store);
    final end = (await tester.runAsync(() => hostEnd(linkSecret)))!;
    host.ask(9, acpSecretRequestMethod, (await tester.runAsync(() => linkRequest(end.host, end.app, 'l2')))!);
    await settle(tester);
    expect(session.pendingSecrets, isEmpty);
    expect(host.answerTo(9)?['error'], isNotNull, reason: 'a program with no host identity can ask for no secret');

    // With the link's secret, a host that bound under another secret is not drawn: the card does not take its key.
    final linkSecrets = await tester.runAsync(() => linkSecretIdentity(linkSecret)) as LinkSecretIdentity;
    final other = (await tester.runAsync(() => hostEnd(otherSecret)))!;
    final store2 = SessionStore();
    final (second, withSecret, _) = await linked(tester, store2, linkSecrets: linkSecrets);
    await tester.pumpWidget(CanoaApp(store: store2));
    await tester.pump();
    second.ask(10, acpSecretRequestMethod, (await tester.runAsync(() => linkRequest(other.host, other.app, 'l3')))!);
    await settle(tester);
    expect(find.byKey(const ValueKey('secret-password')), findsNothing, reason: 'no form is drawn for a key bound under another secret');
    expect(second.sent.map(jsonEncode).join(' '), isNot(contains('"enc"')), reason: 'nothing was sealed for it');
    expect(withSecret.pendingSecrets, isEmpty, reason: 'nothing stays pending for it');
    final answer = second.answerTo(10);
    expect(answer, isNotNull, reason: 'the host got an answer');
    expect(answer!['error'] != null || (answer['result'] as Map)['action'] == 'cancel', isTrue,
        reason: 'an error or a cancel, never an answer that carries values');
  });
}
