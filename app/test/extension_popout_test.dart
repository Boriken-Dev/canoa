// A session popped out of the extension's side panel: the browser's window opened by the worker over the panel's own model, the tab
// binding held by one client at a time, and a window that asks for nothing the panel could not.
import 'dart:async';

import 'package:canoa/extension/tab_bridge.dart';
import 'package:canoa/extension/tab_owner_banner.dart';
import 'package:canoa/extension/tab_shares.dart';
import 'package:canoa/popout/extension_windows.dart';
import 'package:canoa/popout/popout_app.dart';
import 'package:canoa/popout/popout_hub.dart';
import 'package:canoa/popout/popout_launch.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp_chat/experimental.dart' show ChatSessionView, MemoryViewLink;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show PermissionCard;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_tab_bridge.dart';
import 'fixtures/sample_render.dart' show samplesDir;
import 'fixtures/scripted_agent.dart';

/// The worker's windows, in memory: what the panel asks for and the window ids the browser reports gone.
final class FakeWindowBridge implements PopoutBridge {
  FakeWindowBridge({this.client = 'panel'});

  @override
  final String client;
  final opened = <({String session, String link, String title, String look, String density, int windowId})>[];
  final calls = <String>[];
  final _gone = StreamController<int>.broadcast();
  String? refuse;
  var _next = 40;

  @override
  Future<int> open({required String session, required String link, required String title, required String look, required String density}) async {
    if (refuse != null) throw StateError(refuse!);
    final windowId = _next++;
    opened.add((session: session, link: link, title: title, look: look, density: density, windowId: windowId));
    return windowId;
  }

  @override
  Future<void> focus(String session) async => calls.add('focus $session');

  @override
  Future<void> close(String session) async => calls.add('close $session');

  @override
  Stream<int> get closed => _gone.stream;

  void goneByUser(int windowId) => _gone.add(windowId);
}

final class _Links {
  final made = <String, MemoryViewLink>{};
  final far = <String, MemoryViewLink>{};

  MemoryViewLink call(String id) {
    final (owner, other) = MemoryViewLink.pair();
    far[id] = other;
    return made[id] = owner;
  }
}

OpenTab _tab(int id, String title) => OpenTab(id: id, title: title, url: 'https://site$id.example/');

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 120)));
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 50));
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump(const Duration(milliseconds: 300));
}

void main() {
  group('windows opened through the extension\'s worker', () {
    test('open names the session, the channel and the look, and the window is a link', () async {
      final bridge = FakeWindowBridge();
      final links = _Links();
      var count = 0;
      final windows = ExtensionPopoutWindows(bridge, linkFor: links.call, newLinkId: () => 'id${count++}');
      expect(windows.available, isTrue);
      final window = await windows.open(title: 'Fix the build', look: 'canoa', density: 'verbose', session: 'ws-1');
      final asked = bridge.opened.single;
      expect((asked.session, asked.link, asked.title, asked.look, asked.density), ('ws-1', 'id0', 'Fix the build', 'canoa', 'verbose'));
      expect(window.link, same(links.made['id0']));
    });

    test('only the panel opens windows: a popped-out window is not offered the way to open another', () {
      final windows = ExtensionPopoutWindows(FakeWindowBridge(client: 'window:${'ab' * 16}'), linkFor: _Links().call, newLinkId: () => 'x');
      expect(windows.available, isFalse);
    });

    test('a refusal throws what the worker said, and leaves no link behind', () async {
      final bridge = FakeWindowBridge()..refuse = 'Invalid link';
      final links = _Links();
      final windows = ExtensionPopoutWindows(bridge, linkFor: links.call, newLinkId: () => 'id');
      await expectLater(
        windows.open(title: 't', look: 'canoa', density: 'normal', session: 's'),
        throwsA(isA<StateError>().having((e) => e.message, 'message', contains('Invalid link'))),
      );
      expect(links.made['id']!.isClosed, isTrue);
      await expectLater(windows.open(title: 't', look: 'canoa', density: 'normal'), throwsStateError);
    });

    test('the browser reporting the window gone completes it, and another window going does not', () async {
      final bridge = FakeWindowBridge();
      final windows = ExtensionPopoutWindows(bridge, linkFor: _Links().call, newLinkId: () => 'id');
      final window = await windows.open(title: 't', look: 'canoa', density: 'normal', session: 's');
      var closed = false;
      unawaited(window.closed.then((_) => closed = true));
      bridge.goneByUser(7);
      await Future<void>.delayed(Duration.zero);
      expect(closed, isFalse);
      bridge.goneByUser(bridge.opened.single.windowId);
      await Future<void>.delayed(Duration.zero);
      expect(closed, isTrue);
    });

    test('focus and close are the worker\'s, by session, and closing ends the link', () async {
      final bridge = FakeWindowBridge();
      final links = _Links();
      final windows = ExtensionPopoutWindows(bridge, linkFor: links.call, newLinkId: () => 'id');
      final window = await windows.open(title: 't', look: 'canoa', density: 'normal', session: 'ws-9');
      await window.focus();
      await window.close();
      expect(bridge.calls, ['focus ws-9', 'close ws-9']);
      expect(links.made['id']!.isClosed, isTrue);
      await window.closed;
    });
  });

  test('a launch carries the session\'s key through its address or its arguments, and without one carries none', () {
    final launch = PopoutLaunch(link: 'ab' * 16, title: 'T', session: 'ws-1');
    expect(PopoutLaunch.decode(launch.encode())?.session, 'ws-1');
    expect(PopoutLaunch.decode(PopoutLaunch(link: 'x', title: 'T').encode())?.session, isNull);
  });

  testWidgets('a session popped out of the panel keeps its turn and its request, answers once from the window, and docks back', (tester) async {
    await tester.binding.setSurfaceSize(const Size(900, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final store = SessionStore();
    addTearDown(store.dispose);
    final agent = ScriptedAgent();
    final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
    final bridge = FakeWindowBridge();
    final links = _Links();
    final hub = PopoutHub(store, ExtensionPopoutWindows(bridge, linkFor: links.call, newLinkId: () => 'link'), shareInterval: const Duration(milliseconds: 10));
    addTearDown(hub.dispose);
    unawaited(store.sendText('Fix addNumbers'));
    agent.update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Reading calc.js.'}});
    agent.ask(5, 'Edit a.txt');
    await _settle(tester);
    expect(session.status, SessionStatus.working);

    await tester.runAsync(() => hub.popOut(session));
    expect(bridge.opened.single.session, sessionKeyOf(session), reason: 'the worker keeps the tab binding by this key');
    final view = await tester.runAsync(() => ChatSessionView.attach(links.far['link']!, clientId: 'window-1', name: 'the pop-out window')) as ChatSessionView;
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: _Probe(view))));
    await _settle(tester);
    expect(view.session.pendingPermissions, hasLength(1), reason: 'the request that waited is shown in the window');
    expect(session.status, SessionStatus.working, reason: 'popping out does not end the turn');

    await tester.tap(find.byKey(const Key('permission-primary')));
    await _settle(tester);
    expect(agent.answers().map((m) => m['id']), [5], reason: 'the agent hears one answer');
    expect(session.pendingPermissions, isEmpty);
    expect(session.status, SessionStatus.working);

    await tester.runAsync(() => hub.dockBack(session));
    expect(bridge.calls, ['close ${sessionKeyOf(session)}']);
    expect(hub.isPoppedOut(session), isFalse);
    expect(session.status, SessionStatus.working, reason: 'the turn goes on in the panel after docking back');
  });

  testWidgets('the window shows who holds the tab under its header, with the conversation and its request below', (tester) async {
    await tester.binding.setSurfaceSize(const Size(560, 780));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final store = SessionStore();
    addTearDown(store.dispose);
    final agent = ScriptedAgent();
    final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
    final links = _Links();
    final hub = PopoutHub(store, ExtensionPopoutWindows(FakeWindowBridge(), linkFor: links.call, newLinkId: () => 'link'), shareInterval: const Duration(milliseconds: 10));
    addTearDown(hub.dispose);
    unawaited(store.sendText('Fix addNumbers'));
    agent.update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Reading calc.js.'}});
    agent.ask(5, 'Edit a.txt');
    await _settle(tester);
    await tester.runAsync(() => hub.popOut(session));
    final view = await tester.runAsync(() => ChatSessionView.attach(links.far['link']!, clientId: 'window-1', name: 'the pop-out window')) as ChatSessionView;
    final windowClient = 'window:${'ab' * 16}';
    final bridge = FakeTabBridge(front: 2, client: windowClient)
      ..open(_tab(1, 'Docs'))
      ..open(_tab(2, 'Mail'))
      ..bind('ws-1', 1);
    final launch = PopoutLaunch(link: 'ab' * 16, title: session.title, session: 'ws-1');
    await tester.pumpWidget(PopoutApp(launch: launch, view: view, onClose: () {}, banner: TabOwnerBanner(tabs: TabShares(bridge), sessionKey: 'ws-1')));
    await _settle(tester);
    expect(find.text('"Docs" is with the side panel'), findsOneWidget);
    expect(find.byType(PermissionCard), findsOneWidget);
    if (samplesDir != null) {
      await tester.pump();
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('extension_popout_window.png'));
    }
  });

  group('who holds the tab binding', () {
    final windowClient = 'window:${'ab' * 16}';

    Future<(FakeTabBridge, TabShares)> pump(WidgetTester tester, {required String client, String key = 'ws-1'}) async {
      final bridge = FakeTabBridge(front: 2, client: client)
        ..open(_tab(1, 'Docs'))
        ..open(_tab(2, 'Mail'));
      bridge.bind(key, 1);
      final shares = TabShares(bridge);
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: TabOwnerBanner(tabs: shares, sessionKey: key))));
      await tester.pump();
      await tester.pump();
      return (bridge, shares);
    }

    testWidgets('a window says the panel holds the tab, and Move here hands it over with no resume', (tester) async {
      final (bridge, _) = await pump(tester, client: windowClient);
      expect(find.text('"Docs" is with the side panel'), findsOneWidget);
      bridge.shares['ws-1'] = {1};
      await tester.tap(find.text('Move here'));
      await tester.pump();
      await tester.pump();
      expect(bridge.calls, ['move ws-1 2'], reason: 'one move: no attach, no detach, no new session');
      expect(bridge.owners['ws-1'], windowClient);
      expect(bridge.bound['ws-1'], 2);
      expect(bridge.shares['ws-1'], isNull, reason: 'what the old tab shared ends; nothing is shared on the new one');
      expect(find.byKey(const Key('move-here')), findsNothing, reason: 'the window holds it now and it is the tab in front');
    });

    testWidgets('a window that already holds the tab in front offers nothing, and neither does a session with no tab', (tester) async {
      final bridge = FakeTabBridge(front: 1, client: windowClient)..open(_tab(1, 'Docs'));
      bridge.bind('ws-1', 1, owner: windowClient);
      final shares = TabShares(bridge);
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: Column(children: [
        TabOwnerBanner(tabs: shares, sessionKey: 'ws-1'),
        TabOwnerBanner(tabs: TabShares(bridge), sessionKey: 'nothing'),
      ]))));
      await tester.pump();
      await tester.pump();
      expect(find.byKey(const Key('move-here')), findsNothing);
    });

    testWidgets('the panel, when a window holds the tab, says so, and takes it back with one move', (tester) async {
      final bridge = FakeTabBridge(front: 1)..open(_tab(1, 'Docs'));
      bridge.bind('ws-1', 1, owner: windowClient);
      bridge.shares['ws-1'] = {1};
      final shares = TabShares(bridge);
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: TabOwnerBanner(tabs: shares, sessionKey: 'ws-1'))));
      await tester.pump();
      await tester.pump();
      expect(find.text('"Docs" is with its own window'), findsOneWidget);
      await tester.tap(find.text('Move here'));
      await tester.pump();
      await tester.pump();
      expect(bridge.owners['ws-1'], panelClient);
      expect(bridge.shares['ws-1'], {1}, reason: 'the same tab changing hands keeps what it shared: the panel already had that');
      expect(shares.dropsFor('ws-1'), isEmpty);
    });
  });
}

/// A window's view of a conversation, as the permission card in it.
class _Probe extends StatefulWidget {
  const _Probe(this.view);
  final ChatSessionView view;

  @override
  State<_Probe> createState() => _ProbeState();
}

class _ProbeState extends State<_Probe> {
  @override
  void initState() {
    super.initState();
    widget.view.session.addListener(() => mounted ? setState(() {}) : null);
  }

  @override
  Widget build(BuildContext context) {
    final waiting = widget.view.session.pendingPermissions.firstOrNull;
    if (waiting == null) return const Text('nothing waits');
    return PermissionCard(
      agent: widget.view.session.agent,
      title: waiting.title,
      detail: waiting.detail,
      options: waiting.options,
      bypassPassed: waiting.bypassPassed,
      forPerson: waiting.forPerson,
      more: 0,
      host: 'Canoa',
      onChoose: (option) => widget.view.choosePermission(waiting, option),
      onCancel: () => widget.view.choosePermission(waiting, null),
    );
  }
}
