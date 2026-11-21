// UX improvements: human state labels, plain composer & dialog wording, New session like this, closed ledger items in
// search/palette, markdown-rendered plan approvals with taller scroll box, and unified/rebindable [shortcuts] including Esc & Ctrl+W.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/chat/conversation_view.dart';
import 'package:canoa/chat/palette.dart';
import 'package:canoa/chat/session_views.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/connect_dialog.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/main.dart';
import 'package:canoa/rail/session_groups.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/settings/app_sections.dart';
import 'package:canoa/settings/settings_controller.dart';
import 'package:canoa/shell/app_palette.dart';
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/shell/surface.dart';
import 'package:canoa/theme/look.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

final class _FakeAgent {
  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];

  Future<ChatSession> start(
    SessionStore store,
    String title, {
    bool keepWorking = false,
  }) async {
    final client = VerifiedAgent.fromChild(
      label: title,
      messages: incoming.stream,
      decider: store.askPermission,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        sent.add(message);
        final id = message['id'];
        if (id == null || message['method'] == null) return;
        switch (message['method']) {
          case 'initialize':
            reply(id, {'protocolVersion': 1, 'agentCapabilities': <String, Object?>{}});
          case 'session/new':
            reply(id, {'sessionId': 's-$title'});
          case 'session/prompt':
            if (!keepWorking) reply(id, <String, Object?>{});
          default:
            reply(id, <String, Object?>{});
        }
      },
    );
    await client.initialize();
    await client.openSession(cwd: '/work');
    final session = store.addProviderSession(
      title: title,
      agent: 'Claude',
      peerId: client.peerId,
      remoteSessionId: 's-$title',
    );
    store.attachVerifiedClient(session.id, client, record: false);
    return session;
  }

  void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));

  void askPlan(int id, String sessionId) => incoming.add(jsonEncode({
    'jsonrpc': '2.0',
    'id': id,
    'method': 'session/request_permission',
    'params': {
      'sessionId': sessionId,
      'toolCall': {
        'toolCallId': 'p$id',
        'kind': 'switch_mode',
        'title': 'Approve implementation plan',
        'rawInput': {'plan': '# Implementation Plan\n\n- Step one with `inline` code\n- Step two'},
      },
      'options': [
        {'optionId': 'allow', 'name': 'Allow', 'kind': 'allow_once'},
        {'optionId': 'deny', 'name': 'Deny', 'kind': 'reject_once'},
      ],
    },
  }));
}

void main() {
  testWidgets('ConversationView shows human row state in header, plain offline composer hint, and omits remote composer banner', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final store = SessionStore();
    final agent = _FakeAgent();
    final session = await tester.runAsync(() => agent.start(store, 'Remote one')) as ChatSession;
    store.select(session.id);

    await tester.pumpWidget(
      MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: Scaffold(
          body: ListenableBuilder(
            listenable: store,
            builder: (_, _) => ConversationView(
              store: store,
              narrow: false,
              handle: ConversationHandle(),
              layout: ShellLayout(),
              surfaces: WorkSurfaces(const []),
              welcome: const Text('Welcome'),
            ),
          ),
        ),
      ),
    );
    await tester.pump();

    expect(find.text(rowStateOf(session).word), findsOneWidget);
    expect(find.text('connected'), findsNothing);
    expect(find.textContaining('Remote sessions require verified peer authentication'), findsNothing);

    final offline = store.createPreviewSession(title: 'Offline preview', agent: 'Claude')
      ..status = SessionStatus.disconnected;
    store.select(offline.id);
    await tester.pump();
    expect(find.text('Offline'), findsOneWidget);
    expect(
      find.byWidgetPredicate((w) => w is TextField && w.decoration?.hintText == 'Connect to a host to chat'),
      findsOneWidget,
    );
  });

  testWidgets('Add connection dialog explains verified peer authentication once when adding a peer', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(
      MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: Scaffold(
          body: Builder(
            builder: (context) => TextButton(
              onPressed: () => unawaited(
                showAddConnectionDialog(
                  context,
                  onAdd: (_) async {},
                ),
              ),
              child: const Text('Add peer'),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Add peer'));
    await tester.pumpAndSettle();
    expect(find.textContaining('Remote sessions require verified peer authentication.'), findsOneWidget);
  });

  testWidgets('showAppPalette(sessionsOnly: true) lists closed ledger conversations and invokes onReopen', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    final store = SessionStore(ledger: ledger);
    ledger.record(
      peerId: 'local:claude',
      sessionId: 'closed-1',
      entry: 'Claude',
      reportedName: 'claude-code',
      place: SessionPlace.local,
      title: 'Earlier closed chat',
    );
    await tester.runAsync(() => ledger.flush());
    LedgerEntry? reopened;

    await tester.pumpWidget(
      MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: Scaffold(
          body: Builder(
            builder: (context) => TextButton(
              onPressed: () => unawaited(
                showAppPalette(
                  context,
                  store: store,
                  grouping: ValueNotifier(SessionGrouping.status),
                  conversation: ConversationHandle(),
                  onNewSession: () {},
                  onSettings: () {},
                  onStop: () {},
                  onReopen: (entry) async => reopened = entry,
                  sessionsOnly: true,
                ),
              ),
              child: const Text('Search sessions'),
            ),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Search sessions'));
    await tester.pumpAndSettle();
    expect(find.text('Earlier closed chat'), findsOneWidget);
    expect(find.textContaining('Earlier conversation'), findsOneWidget);
    await tester.tap(find.text('Earlier closed chat'));
    await tester.pumpAndSettle();
    expect(reopened?.sessionId, 'closed-1');
  });

  testWidgets('SessionPermissionCard renders a forPerson plan as AgentMarkdown inside 420 px container with plan labels', (tester) async {
    final store = SessionStore();
    final agent = _FakeAgent();
    final session = await tester.runAsync(() => agent.start(store, 'Plan')) as ChatSession;
    agent.askPlan(10, 's-Plan');
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 30)));
    await tester.pump();

    await tester.pumpWidget(
      MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: Scaffold(body: SessionPermissionCard(store: store, session: session)),
      ),
    );
    await tester.pump();

    expect(find.text('Plan approval'), findsOneWidget);
    expect(find.byType(AgentMarkdown), findsOneWidget);
    expect(find.textContaining('Approve plan'), findsOneWidget);
    expect(find.textContaining('Keep editing'), findsOneWidget);
    final detail = tester.widget<Container>(find.byKey(const Key('permission-detail')));
    expect(detail.constraints?.maxHeight, 420);

    await tester.tap(find.byKey(const Key('answer-allow_once')));
    await tester.pump();
    expect(session.pendingPermissions, isEmpty);
  });

  test('[shortcuts] parses, encodes, and overrides shortcutHelpFor', () {
    const raw = '''
[shortcuts]
new_session = "Ctrl+Shift+N"
close_session = "Ctrl+Shift+W"
''';
    final parsed = CanoaConfig.parse(raw);
    expect(parsed.shortcuts, {'new_session': 'Ctrl+Shift+N', 'close_session': 'Ctrl+Shift+W'});
    expect(parsed.encode(), contains('[shortcuts]'));
    expect(parsed.encode(), contains('"new_session" = "Ctrl+Shift+N"'));
    final reparsed = CanoaConfig.parse(parsed.encode());
    expect(reparsed.shortcuts, parsed.shortcuts);

    final help = shortcutHelpFor(parsed.shortcuts);
    expect(help.any((line) => line.keys == 'Ctrl+Shift+N' && line.what == 'Start a new session'), isTrue);
    expect(help.any((line) => line.keys == 'Esc' && line.what == 'Stop the working turn'), isTrue);
    expect(parseShortcutActivator('Esc')?.trigger, LogicalKeyboardKey.escape);
    expect(parseShortcutActivator('Ctrl+Shift+K')?.shift, isTrue);
  });

  testWidgets('Esc stops a working turn and Ctrl+W closes active session', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final store = SessionStore();
    final a1 = _FakeAgent();
    final a2 = _FakeAgent();
    final s1 = await tester.runAsync(() => a1.start(store, 'First', keepWorking: true)) as ChatSession;
    await tester.runAsync(() => a2.start(store, 'Second'));
    store.select(s1.id);

    await tester.pumpWidget(CanoaApp(store: store));
    await tester.pump();

    unawaited(store.sendText('keep running'));
    await tester.pump();
    expect(s1.status, SessionStatus.working);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    expect(a1.sent.any((m) => m['method'] == 'session/cancel'), isTrue);

    // Ctrl+W closes the active conversation.
    final before = store.sessions.length;
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyW);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));
    expect(store.sessions.length, before - 1);
  });

  testWidgets('ShortcutsSection edits and resets overrides', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1280, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final controller = SettingsController(config: const CanoaConfig(), apply: (c) async => c);
    await tester.pumpWidget(
      MaterialApp(
        theme: themeFor(LookId.canoa, Brightness.dark),
        home: Scaffold(body: SingleChildScrollView(child: ShortcutsSection(controller: controller))),
      ),
    );
    await tester.pump();
    await tester.tap(find.byKey(const Key('shortcut-edit-close_session')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('shortcut-input-close_session')), 'Ctrl+Shift+W');
    await tester.tap(find.byKey(const Key('shortcut-save-close_session')));
    await tester.pumpAndSettle();
    expect(controller.config.shortcuts['close_session'], 'Ctrl+Shift+W');
    await tester.ensureVisible(find.byKey(const Key('shortcuts-reset')));
    await tester.tap(find.byKey(const Key('shortcuts-reset')));
    await tester.pumpAndSettle();
    expect(controller.config.shortcuts, isEmpty);
  });
}
