// The host's secret prompts in the app: the offer in `initialize`, the form and the show on the proven connection, and the one rule
// that matters: what the person types goes to the host sealed, in the answer to the host's own request, and nowhere else.
import 'dart:convert';

import 'package:canoa/session_store.dart';
import 'package:coqui_acp/secret.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/secret_host.dart';

void main() {
  testWidgets('the app offers the extension in initialize only when it has an identity to bind answers with', (tester) async {
    final (host, _, _) = await shown(tester);
    final capabilities = (host.initialize!['params'] as Map)['clientCapabilities'] ?? (host.initialize!['params'] as Map)['capabilities'];
    expect(((capabilities as Map)['_meta'] as Map)[acpSecretsCapability], {'version': 1, 'methods': ['request', 'show'], 'bindings': [acpBindingEcdsa]});
    final (bare, _, _) = await shown(tester, withIdentity: false);
    expect(jsonEncode(bare.initialize), isNot(contains(acpSecretsCapability)));
  });

  testWidgets('a request is a form over the composer; what is typed reaches the host only, sealed, in the answer to its request', (tester) async {
    final (host, session, store) = await shown(tester);
    final asked = <AttentionKind>[];
    store.attention.listen((needs) => asked.add(needs.$2));
    host.ask(7, acpSecretRequestMethod, (await tester.runAsync(() => requestParams('r1')))!);
    await settle(tester);
    expect(find.byType(SecretRequestCard), findsOneWidget);
    expect(find.text('The deploy needs your registry login'), findsOneWidget);
    expect(find.text('Claude asks for a secret'), findsOneWidget);
    expect(asked, [AttentionKind.question]);
    expect(session.needsYou, isTrue);
    // The shared ACP client never saw it: nothing but the one answer went back for that id.
    expect(host.sent.where((message) => message['id'] == 7), isEmpty);

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
    // Everything this app ever sent on the connection, the agent's side included: the value is in none of it, in the clear.
    final wire = host.sent.map(jsonEncode).join('\n');
    expect(wire, isNot(contains(password)));
    expect(find.byType(SecretRequestCard), findsNothing);
    expect(session.pendingSecrets, isEmpty);
    expect(session.messages.map((message) => message.text).join('\n'), isNot(contains(password)));
    // The sealed answer opens for the host, and only for it.
    final reply = AcpSecretRequestReply.fromJson(result, (name) => name == 'password') as AcpSecretAccepted;
    final opened = await tester.runAsync(() => receivers['r1']!.openValues(
      AcpSecretVerifier(own: hostProver.id, known: [appProver.id]), appProver.id, reply.plain, reply.sealed, reply.auth, now,
    )) as AcpOpened;
    expect(opened.secrets['password']!.text, password);
    expect(opened.plain, {'user': 'ana'});
    // The only message that carries it is the answer to the request that asked.
    final carrying = host.sent.where((message) => jsonEncode(message).contains('"enc"')).toList();
    expect(carrying, hasLength(1));
    expect(carrying.single['id'], 7);
    expect(carrying.single['method'], isNull);
  });

  testWidgets('Decline and Cancel answer the host without values', (tester) async {
    final (host, _, _) = await shown(tester);
    host.ask(8, acpSecretRequestMethod, (await tester.runAsync(() => requestParams('r2')))!);
    await settle(tester);
    await tester.tap(find.descendant(of: find.byType(SecretRequestCard), matching: find.text('Decline')));
    await settle(tester);
    expect(host.answerTo(8)?['result'], {'action': 'decline'});
    host.ask(9, acpSecretRequestMethod, (await tester.runAsync(() => requestParams('r3')))!);
    await settle(tester);
    await tester.tap(find.descendant(of: find.byType(SecretRequestCard), matching: find.text('Cancel')));
    await settle(tester);
    expect(host.answerTo(9)?['result'], {'action': 'cancel'});
  });

  testWidgets('without an identity the host is cancelled, never asked of the person, and the agent\'s client still sees nothing', (tester) async {
    final (host, session, _) = await shown(tester, withIdentity: false);
    host.ask(10, acpSecretRequestMethod, (await tester.runAsync(() => requestParams('r4')))!);
    await settle(tester);
    expect(find.byType(SecretRequestCard), findsNothing);
    expect(session.pendingSecrets, isEmpty);
    expect(host.answerTo(10)?['error'], isNotNull);
  });

  testWidgets('a request for another session is cancelled and shows nothing', (tester) async {
    final (host, session, _) = await shown(tester);
    final params = await tester.runAsync(() => requestParams('r5')) as Map<String, Object?>;
    host.ask(11, acpSecretRequestMethod, {...params, 'sessionId': 'other'});
    await settle(tester);
    expect(session.pendingSecrets, isEmpty);
    expect(find.byType(SecretRequestCard), findsNothing);
    // The binding covers the session, so the host's answer is cancel and no form was drawn.
    expect(host.answerTo(11)?['result'], {'action': 'cancel'});
  });

  testWidgets('a show: opened after the person agrees, delivered sealed, masked until revealed, and closed with shown', (tester) async {
    final (host, session, _) = await shown(tester);
    final open = AcpSecretShowOpen(context: AcpSecretContext(sessionId: 's1', requestId: 'w1', message: 'Your new API key', schema: showSchema, values: '{"label":"ci"}'));
    host.ask(20, acpSecretShowOpenMethod, open.toJson());
    await settle(tester);
    expect(find.byType(SecretShowCard), findsOneWidget);
    expect(host.answerTo(20), isNull, reason: 'nothing is answered before the person says so');
    await tester.tap(find.text('Show'));
    await settle(tester);
    final ready = AcpSecretShowOpened.fromJson(host.answerTo(20)!['result']);
    expect(ready.action, 'ready');
    // The host checks the app's key, seals the value to it and binds the delivery.
    final collector = await tester.runAsync(() => AcpSecretVerifier(own: hostProver.id, known: [appProver.id])
        .collector(appProver.id, AcpSecretMethod.show, open.context, ready.recipient!, now)) as AcpSecretCollector;
    final sealed = {'key': await tester.runAsync(() => collector.seal('key', token)) as AcpSealed};
    final auth = await tester.runAsync(() => collector.authenticate(hostProver, open.context.values!, sealed, now)) as AcpSecretBinding;
    host.ask(21, acpSecretShowDeliverMethod, AcpSecretShowDeliver(sessionId: 's1', requestId: 'w1', sealed: sealed, auth: auth).toJson());
    await settle(tester);
    expect(find.byKey(const ValueKey('masked-key')), findsOneWidget);
    expect(find.text(token), findsNothing);
    await tester.tap(find.byKey(const ValueKey('reveal-key')));
    await tester.pump();
    expect(find.text(token), findsOneWidget);
    await tester.tap(find.text('Done'));
    await settle(tester);
    expect(host.answerTo(21)?['result'], {'action': 'shown'});
    expect(session.pendingSecrets, isEmpty);
    expect(find.text(token), findsNothing);
    // The value was never in anything this app sent.
    expect(host.sent.map(jsonEncode).join('\n'), isNot(contains(token)));
  });

  testWidgets('a delivery nobody asked for is refused, and a show the person declines ends with no card', (tester) async {
    final (host, session, _) = await shown(tester);
    host.ask(30, acpSecretShowDeliverMethod, {
      'sessionId': 's1',
      'requestId': 'nope',
      'values': {'key': {'enc': List.filled(43, 'A').join(), 'ct': List.filled(43, 'A').join()}},
      'auth': {'method': acpBindingEcdsa, 'key': 'AAAA', 'expires': now, 'audience': appProver.id.toString(), 'proof': 'AAAA'},
    });
    await settle(tester);
    expect(host.answerTo(30)?['error'], isNotNull);
    final open = AcpSecretShowOpen(context: AcpSecretContext(sessionId: 's1', requestId: 'w2', message: 'A key', schema: showSchema, values: '{"label":"ci"}'));
    host.ask(31, acpSecretShowOpenMethod, open.toJson());
    await settle(tester);
    await tester.tap(find.text('Decline'));
    await settle(tester);
    expect(host.answerTo(31)?['result'], {'action': 'decline'});
    expect(session.pendingSecrets, isEmpty);
  });
}
