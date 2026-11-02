import 'package:coqui_acp/page.dart';
import 'package:canoa/connect_dialog.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

final class _FakeProvider implements AcpPage {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  testWidgets('shows Canoa multi-session chat shell', (tester) async {
    final store = SessionStore();
    await tester.pumpWidget(CanoaApp(store: store));
    expect(find.text('Canoa'), findsWidgets);
    expect(find.text('Getting started'), findsWidgets);
    expect(find.textContaining('Preview'), findsWidgets);
    expect(find.byIcon(Icons.add_comment_outlined), findsOneWidget);
    store.dispose();
  });

  group('agents from the browser', () {
    Future<void> open(WidgetTester tester, Future<AcpPage?> provider, UseBrowserAgents use) async {
      await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
        onPressed: () => showCanoaConnectDialog(context, (_) async {}, browserProvider: provider, onUseBrowserAgents: use),
        child: const Text('open'),
      ))));
      await tester.tap(find.text('open'));
      await tester.pump();
      await tester.pump();
    }

    const label = 'Use an agent from this browser';

    testWidgets('no provider, no button', (tester) async {
      await open(tester, Future.value(null), (_, {required cwd, required title}) async {});
      expect(find.text(label), findsNothing);
    });

    testWidgets('a provider offers it, and the dialog closes once the agents are in', (tester) async {
      AcpPage? used;
      final provider = _FakeProvider();
      await open(tester, Future.value(provider), (acp, {required cwd, required title}) async => used = acp);
      await tester.tap(find.text(label));
      await tester.pump();
      await tester.pump();
      expect(used, same(provider));
      expect(find.text(label), findsNothing);
    });

    testWidgets('a refusal stays in the dialog and says so', (tester) async {
      await open(tester, Future.value(_FakeProvider()), (_, {required cwd, required title}) async {
        throw const AcpPageFailure('NotAllowedError', 'this page may not do that');
      });
      await tester.tap(find.text(label));
      await tester.pump();
      await tester.pump();
      expect(find.textContaining('NotAllowedError'), findsOneWidget);
    });
  });
}
