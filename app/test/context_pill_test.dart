import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/surfaces/files_surface.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_session_agent.dart';

void main() {
  testWidgets('composer context pill shows fill percentage, opens details, and sends the command the agent reports', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final dir = Directory.systemTemp.createTempSync('canoa_ux_compact_');
    addTearDown(() => dir.deleteSync(recursive: true));

    final store = SessionStore();
    final agent = FakeAgent();
    await tester.runAsync(() => agent.start(store, cwd: dir.path));

    await tester.pumpWidget(CanoaApp(
      store: store,
      config: const CanoaConfig(density: ChatDensity.verbose),
      surfaces: [FilesSurface((_) => FakeFiles())],
    ));
    agent.update({'sessionUpdate': 'usage_update', 'used': 84000, 'size': 200000});
    agent.update({
      'sessionUpdate': 'available_commands_update',
      'availableCommands': [
        {'type': 'text', 'name': 'compact', 'description': 'Compact context'},
      ],
    });
    await settle(tester);
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('composer-context-pill')), findsOneWidget);
    expect(find.text('42% context'), findsOneWidget);

    await tester.tap(find.byKey(const Key('composer-context-pill')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('context-compact')), findsOneWidget);
    expect(find.byKey(const Key('context-details')), findsOneWidget);

    await tester.tap(find.byKey(const Key('context-compact')));
    await settle(tester);
    await tester.pumpAndSettle();
    final prompts = [for (final m in agent.sent) if (m['method'] == 'session/prompt') m];
    expect(prompts.length, 1);
    expect(jsonEncode(prompts.single['params']), contains('/compact'));
    store.dispose();
  });

  testWidgets('an agent that reports no compact command is offered no Compact', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final dir = Directory.systemTemp.createTempSync('canoa_ux_nocompact_');
    addTearDown(() => dir.deleteSync(recursive: true));

    final store = SessionStore();
    final agent = FakeAgent();
    await tester.runAsync(() => agent.start(store, cwd: dir.path));

    await tester.pumpWidget(CanoaApp(
      store: store,
      config: const CanoaConfig(density: ChatDensity.verbose),
      surfaces: [FilesSurface((_) => FakeFiles())],
    ));
    agent.update({'sessionUpdate': 'usage_update', 'used': 84000, 'size': 200000});
    agent.update({
      'sessionUpdate': 'available_commands_update',
      'availableCommands': [
        {'type': 'text', 'name': 'review', 'description': 'Review the changes'},
      ],
    });
    await settle(tester);
    await tester.pumpAndSettle();

    expect(find.text('42% context'), findsOneWidget);
    await tester.tap(find.byKey(const Key('composer-context-pill')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('context-compact')), findsNothing);
    expect(find.byKey(const Key('context-details')), findsOneWidget);
    store.dispose();
  });
}
