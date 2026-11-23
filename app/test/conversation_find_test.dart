import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/surfaces/files_surface.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_session_agent.dart';

void main() {
  testWidgets('Find in conversation counts matches, steps through them and closes', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final dir = Directory.systemTemp.createTempSync('canoa_ux_find_');
    addTearDown(() => dir.deleteSync(recursive: true));

    final store = SessionStore();
    final agent = FakeAgent();
    await tester.runAsync(() => agent.start(store, cwd: dir.path));

    await tester.pumpWidget(CanoaApp(
      store: store,
      config: const CanoaConfig(density: ChatDensity.verbose),
      surfaces: [FilesSurface((_) => FakeFiles())],
    ));
    agent.update({'sessionUpdate': 'user_message_chunk', 'content': {'type': 'text', 'text': 'Check the alpha token.'}});
    agent.update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Found `alpha` inside **config**.'}});
    await settle(tester);
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('Find in conversation'));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('conversation-find-bar')), findsOneWidget);

    await tester.enterText(find.byKey(const Key('conversation-find-input')), 'alpha');
    await tester.pump();
    expect(find.text('1 of 2'), findsOneWidget);

    await tester.tap(find.byKey(const Key('conversation-find-next')));
    await tester.pump();
    expect(find.text('2 of 2'), findsOneWidget);

    await tester.tap(find.byKey(const Key('conversation-find-close')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('conversation-find-bar')), findsNothing);
  });
}
