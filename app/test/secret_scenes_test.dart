// The secret prompts as Canoa draws them over its composer, drawn with the test font like the app's other goldens. With
// CANOA_SAMPLES_DIR set the same scenes are written with real fonts, for the UI reference.
import 'dart:convert';

import 'package:coqui_acp/secret.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/secret_host.dart';

void main() {
  for (final (label, size) in [('desktop', const Size(1100, 900)), ('phone', const Size(390, 1100))]) {
    testWidgets('a request for a registry login, over the composer, $label', (tester) async {
      final (host, _, _) = await shown(tester, size: size);
      host.ask(7, acpSecretRequestMethod, (await tester.runAsync(() => requestParams('r1')))!);
      await until(tester, find.byKey(const ValueKey('question-user')));
      await tester.enterText(find.byKey(const ValueKey('question-user')), 'ana');
      await tester.enterText(find.byKey(const ValueKey('secret-password')), password);
      FocusManager.instance.primaryFocus?.unfocus();
      await tester.pump();
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/secret_request_$label.png'));
    });
  }

  testWidgets('a show of an API key, masked and then revealed, desktop', (tester) async {
    final (host, _, _) = await shown(tester, size: const Size(1100, 900));
    final open = AcpSecretShowOpen(context: AcpSecretContext(sessionId: 's1', requestId: 'w1', message: 'Your new API key', schema: showSchema, values: '{"label":"ci"}'));
    host.ask(20, acpSecretShowOpenMethod, open.toJson());
    await settle(tester);
    await tester.tap(find.text('Show'));
    await until(tester, find.text('Waiting for the values from the host.'));
    final ready = AcpSecretShowOpened.fromJson(host.answerTo(20)!['result']);
    final collector = await tester.runAsync(() => AcpSecretVerifier(own: hostProver.id, known: [appProver.id])
        .collector(appProver.id, AcpSecretMethod.show, open.context, ready.recipient!, now)) as AcpSecretCollector;
    final sealed = {'key': await tester.runAsync(() => collector.seal('key', token)) as AcpSealed};
    final auth = await tester.runAsync(() => collector.authenticate(hostProver, open.context.values!, sealed, now)) as AcpSecretBinding;
    host.ask(21, acpSecretShowDeliverMethod, AcpSecretShowDeliver(sessionId: 's1', requestId: 'w1', sealed: sealed, auth: auth).toJson());
    await until(tester, find.byKey(const ValueKey('masked-key')));
    expect(jsonEncode(host.sent), isNot(contains(token)));
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/secret_show_masked_desktop.png'));
    await tester.tap(find.byKey(const ValueKey('reveal-key')));
    await tester.pump();
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/secret_show_revealed_desktop.png'));
  });
}
