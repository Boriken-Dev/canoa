// The panel's view of tabs, on a fake of the worker's bridge: what a session shares and with whom, the + menu and `@` lists, the
// notice when the browser ends a share, and moving a session to the tab in front.
import 'package:canoa/chat/composer.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/extension/composer_extension.dart';
import 'package:canoa/extension/tab_bridge.dart';
import 'package:canoa/extension/tab_shares.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_tab_bridge.dart';
import 'fixtures/sample_render.dart' show samplesDir;

final _peer = 'sha256:${'ab' * 32}';

OpenTab _tab(int id, String title) => OpenTab(id: id, title: title, url: 'https://site$id.example/');

final class _Rig {
  _Rig(this.bridge, this.store, this.session, this.shares, this.extension, this.input);

  final FakeTabBridge bridge;
  final SessionStore store;
  final ChatSession session;
  final TabShares shares;
  final TabsComposerExtension extension;
  final TextEditingController input;
}

Future<_Rig> _pump(WidgetTester tester, {String? peer, FakeTabBridge? given}) async {
  final bridge = given ??
      (FakeTabBridge(front: 1)
        ..open(_tab(1, 'Docs'))
        ..open(_tab(2, 'Mail'))
        ..open(_tab(3, 'Shop')));
  final store = SessionStore();
  final session = store.createDraft()
    ..peerId = peer ?? _peer
    ..agent = 'Desk';
  final shares = TabShares(bridge);
  final extension = TabsComposerExtension(shares, store: store);
  final input = TextEditingController();
  addTearDown(() {
    extension.dispose();
    input.dispose();
    store.dispose();
  });
  await tester.binding.setSurfaceSize(const Size(420, 1000));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(MaterialApp(
    debugShowCheckedModeBanner: false,
    theme: themeFor(const CanoaConfig().look, Brightness.dark),
    home: Scaffold(
      body: Align(
        alignment: Alignment.bottomCenter,
        child: SessionComposer(store: store, session: session, controller: input, kit: ComposerKit(extension: extension), onSend: () {}),
      ),
    ),
  ));
  await tester.pump();
  await tester.pump();
  return _Rig(bridge, store, session, shares, extension, input);
}

Future<void> _openPlus(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('composer-plus')));
  await tester.pump();
  await tester.pump();
}

/// With CANOA_SAMPLES_DIR set, a picture of the composer as the narrow panel draws it; otherwise nothing.
Future<void> _sample(WidgetTester tester, String name) async {
  if (samplesDir == null) return;
  await tester.binding.setSurfaceSize(const Size(380, 560));
  await tester.pump();
  await expectLater(find.byType(MaterialApp), matchesGoldenFile(name));
}

void main() {
  testWidgets('outside the extension the composer shows nothing about tabs', (tester) async {
    final store = SessionStore();
    final session = store.createDraft()
      ..peerId = _peer
      ..agent = 'Desk';
    addTearDown(store.dispose);
    final input = TextEditingController();
    await tester.pumpWidget(MaterialApp(
    debugShowCheckedModeBanner: false,
      theme: themeFor(const CanoaConfig().look, Brightness.dark),
      home: Scaffold(body: SessionComposer(store: store, session: session, controller: input, kit: const ComposerKit(), onSend: () {})),
    ));
    await _openPlus(tester);
    expect(find.textContaining('Sharing'), findsNothing);
    expect(find.byKey(const Key('share-tab-1')), findsNothing);
    expect(find.text('Add files or images…'), findsOneWidget);
  });

  testWidgets('a session with no paired agent has nothing to share with', (tester) async {
    await _pump(tester, peer: 'local:claude');
    await _openPlus(tester);
    expect(find.byKey(const Key('share-tab-1')), findsNothing);
    expect(find.textContaining('Sharing'), findsNothing);
  });

  testWidgets('the + menu lists the open tabs, and choosing one shares it with the agent\'s machine', (tester) async {
    final rig = await _pump(tester);
    expect(find.textContaining('Sharing'), findsNothing, reason: 'off until the person shares');
    await _openPlus(tester);
    expect(find.byKey(const Key('share-tab-1')), findsOneWidget);
    expect(find.byKey(const Key('share-tab-2')), findsOneWidget);
    expect(find.byKey(const Key('share-tab-3')), findsOneWidget);
    await tester.tap(find.byKey(const Key('share-tab-2')));
    await tester.pump();
    await tester.pump();
    final key = sessionKeyOf(rig.session);
    expect(rig.bridge.calls, contains('share $key 2 $_peer'));
    expect(find.text('Sharing "Mail" with Desk'), findsOneWidget);
    expect(rig.bridge.shares[key], {2});
  });

  testWidgets('samples of the panel: sharing, the + menu and @, and the notices', (tester) async {
    if (samplesDir == null) return;
    final rig = await _pump(tester);
    await rig.shares.toggle(rig.session, rig.bridge.tabs[1]!);
    await rig.shares.toggle(rig.session, rig.bridge.tabs[2]!);
    await tester.pump();
    await _sample(tester, 'extension_panel_sharing.png');
    await _openPlus(tester);
    await _sample(tester, 'extension_panel_plus_menu.png');
    await tester.tapAt(const Offset(5, 5));
    await tester.pump();
    rig.input.text = 'compare with @s';
    await tester.pump();
    await _sample(tester, 'extension_panel_at.png');
    rig.input.clear();
    rig.bridge.navigate(2, 'Elsewhere');
    rig.bridge.bind(sessionKeyOf(rig.session), 3);
    await rig.shares.refresh();
    await tester.pump();
    await _sample(tester, 'extension_panel_notices.png');
  });

  testWidgets('the person can share more than ten tabs: there is no cap', (tester) async {
    final bridge = FakeTabBridge(front: 1);
    for (var id = 1; id <= 14; id += 1) {
      bridge.open(_tab(id, 'Tab $id'));
    }
    final rig = await _pump(tester, given: bridge);
    final session = rig.session;
    for (var id = 1; id <= 14; id += 1) {
      await rig.shares.toggle(session, bridge.tabs[id]!);
    }
    await tester.pump();
    expect(rig.shares.sharedTabs(sessionKeyOf(session)).length, 14);
  });

  testWidgets('the x on a chip stops sharing, and says nothing was lost', (tester) async {
    final rig = await _pump(tester);
    await rig.shares.toggle(rig.session, rig.bridge.tabs[2]!);
    await tester.pump();
    expect(find.text('Sharing "Mail" with Desk'), findsOneWidget);
    await tester.tap(find.byTooltip('Stop sharing'));
    await tester.pump();
    await tester.pump();
    expect(find.textContaining('Sharing'), findsNothing);
    expect(find.textContaining('Stopped sharing'), findsNothing);
    expect(rig.bridge.shares[sessionKeyOf(rig.session)], isEmpty);
  });

  testWidgets('a tab that changes page ends its share, and the chip shows the drop', (tester) async {
    final rig = await _pump(tester);
    await rig.shares.toggle(rig.session, rig.bridge.tabs[2]!);
    await tester.pump();
    rig.bridge.navigate(2, 'Elsewhere');
    await tester.pump();
    await tester.pump();
    expect(find.text('Sharing "Mail" with Desk'), findsNothing);
    expect(find.text('Stopped sharing "Mail": the tab changed page or closed.'), findsOneWidget);
    await tester.tap(find.byTooltip('Dismiss'));
    await tester.pump();
    expect(find.textContaining('Stopped sharing'), findsNothing);
  });

  testWidgets('a closed tab does the same', (tester) async {
    final rig = await _pump(tester);
    await rig.shares.toggle(rig.session, rig.bridge.tabs[3]!);
    await tester.pump();
    rig.bridge.close(3);
    await tester.pump();
    await tester.pump();
    expect(find.text('Stopped sharing "Shop": the tab changed page or closed.'), findsOneWidget);
  });

  testWidgets('typing @ lists the open tabs, filters them, and choosing one shares it without the word in the message', (tester) async {
    final rig = await _pump(tester);
    rig.input.text = 'read this @ma';
    await tester.pump();
    expect(find.byKey(const Key('at-tab-2')), findsOneWidget);
    expect(find.byKey(const Key('at-tab-1')), findsNothing);
    await tester.tap(find.byKey(const Key('at-tab-2')));
    await tester.pump();
    await tester.pump();
    expect(rig.input.text, 'read this ');
    expect(find.text('Sharing "Mail" with Desk'), findsOneWidget);
    expect(find.byKey(const Key('at-tab-2')), findsNothing);
  });

  testWidgets('a session started in another tab offers Move here, which takes it to the tab in front', (tester) async {
    final bridge = FakeTabBridge(front: 1)
      ..open(_tab(1, 'Docs'))
      ..open(_tab(2, 'Mail'));
    final rig = await _pump(tester, given: bridge);
    final key = sessionKeyOf(rig.session);
    bridge.bind(key, 2);
    bridge.shares[key] = {2};
    await rig.shares.refresh();
    await tester.pump();
    expect(find.text('Started in "Mail"'), findsOneWidget);
    expect(find.text('Sharing "Mail" with Desk'), findsOneWidget);
    await tester.tap(find.text('Move here'));
    await tester.pump();
    await tester.pump();
    expect(bridge.bound[key], 1);
    expect(bridge.calls.where((call) => call.startsWith('move')), ['move $key 1'], reason: 'one move, not a resume');
    expect(bridge.owners[key], panelClient);
    expect(bridge.shares[key] ?? <int>{}, isEmpty, reason: 'what it shared in the old tab stops');
    expect(find.byKey(const Key('move-here')), findsNothing);
    expect(find.textContaining('Stopped sharing'), findsNothing, reason: 'the person did that');
  });

  testWidgets('a session whose tab a popped-out window holds offers Move here in the panel, which takes it back', (tester) async {
    final bridge = FakeTabBridge(front: 1)..open(_tab(1, 'Docs'));
    final rig = await _pump(tester, given: bridge);
    final key = sessionKeyOf(rig.session);
    bridge.bind(key, 1, owner: 'window:${'ab' * 16}');
    await rig.shares.refresh();
    await tester.pump();
    expect(find.text('"Docs" is with its own window'), findsOneWidget);
    await tester.tap(find.text('Move here'));
    await tester.pump();
    await tester.pump();
    expect(bridge.owners[key], panelClient);
    expect(find.byKey(const Key('move-here')), findsNothing);
  });

  testWidgets('a session with no tab is attached to the one in front the first time it is seen', (tester) async {
    final rig = await _pump(tester);
    expect(rig.bridge.bound[sessionKeyOf(rig.session)], 1);
    expect(find.byKey(const Key('move-here')), findsNothing);
  });

  testWidgets('a session closed here ends its shares', (tester) async {
    final rig = await _pump(tester);
    final key = sessionKeyOf(rig.session);
    await rig.shares.toggle(rig.session, rig.bridge.tabs[2]!);
    await tester.pump();
    await rig.store.close(rig.session);
    await tester.pump();
    await tester.pump();
    expect(rig.bridge.calls, contains('detach $key'));
    expect(rig.bridge.shares[key], isNull);
  });

  testWidgets('a refusal is said, and nothing is shown as shared', (tester) async {
    final rig = await _pump(tester);
    rig.bridge.refuse = 'Browser host access was refused';
    await _openPlus(tester);
    await tester.tap(find.byKey(const Key('share-tab-2')));
    await tester.pump();
    await tester.pump();
    expect(find.text('Browser host access was refused'), findsOneWidget);
    expect(find.textContaining('Sharing "'), findsNothing);
  });
}
