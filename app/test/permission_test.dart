// The agent's own answers to a permission request, and full access: a choice the person makes, shown while it lasts, and never
// an answer to a request that offers no way to allow.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/local_agent.dart';
import 'package:canoa/chat/permission_card.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/settings_dialog.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// An agent in memory, speaking v1, whose requests offer what a test gives them.
final class _Agent {
  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];
  late VerifiedAgent client;

  Future<ChatSession> start(SessionStore store, {String label = 'Scripted'}) async {
    client = VerifiedAgent.fromChild(
      label: label,
      messages: incoming.stream,
      decider: store.askPermission,
      close: () async {},
      send: (line) async {
        final message = jsonDecode(line) as Map<String, dynamic>;
        sent.add(message);
        if (message['method'] == 'initialize') {
          reply(message['id'], {'protocolVersion': 1, 'agentCapabilities': <String, Object?>{}});
        } else if (message['method'] == 'session/new') {
          reply(message['id'], {'sessionId': 's1'});
        } else if (message['id'] != null && message['method'] != null && message['method'] != 'session/prompt') {
          reply(message['id'], <String, Object?>{});
        }
      },
    );
    await client.initialize();
    final opened = await client.openSession(cwd: '/work');
    final session = store.addProviderSession(title: label, agent: label, peerId: client.peerId, remoteSessionId: opened.id);
    store.attachVerifiedClient(session.id, client);
    store.select(session.id);
    return session;
  }

  void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));

  void ask(int id, String title, List<Map<String, Object?>> options) => incoming.add(jsonEncode({
    'jsonrpc': '2.0', 'id': id, 'method': 'session/request_permission',
    'params': {
      'sessionId': 's1',
      'toolCall': {'toolCallId': 't$id', 'title': title, 'kind': 'edit', 'rawInput': {'file_path': 'a.txt'}},
      'options': options,
    },
  }));

  Map<String, dynamic>? answerTo(int id) => sent.where((message) => message['id'] == id && message['method'] == null).firstOrNull;
}

/// What Claude's adapter offers for an edit, and what ACP's other kind refuses for good.
const _all = [
  {'optionId': 'allow-once', 'name': 'Yes', 'kind': 'allow_once'},
  {'optionId': 'allow-always', 'name': 'Yes, and don\'t ask again for edits', 'kind': 'allow_always'},
  {'optionId': 'reject-always', 'name': 'No, never', 'kind': 'reject_always'},
  {'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'},
];
const _noAllow = [
  {'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'},
  {'optionId': 'reject-always', 'name': 'No, never', 'kind': 'reject_always'},
];
const _alwaysOnly = [
  {'optionId': 'allow-always', 'name': 'Always', 'kind': 'allow_always'},
  {'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'},
];

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump();
}

Future<(_Agent, ChatSession, SessionStore)> _shown(WidgetTester tester, {CanoaConfig config = const CanoaConfig(), SessionStore? store}) async {
  await tester.binding.setSurfaceSize(const Size(1100, 900));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final kept = store ?? SessionStore(includePreview: false, fullAccessAgents: config.fullAccess);
  final agent = _Agent();
  final session = await tester.runAsync(() => agent.start(kept)) as ChatSession;
  await tester.pumpWidget(CanoaApp(store: kept, config: config));
  await tester.pump();
  return (agent, session, kept);
}

Finder _choice(String name) => find.descendant(of: find.byType(PermissionCard), matching: find.widgetWithText(PermissionChoice, name));

void main() {
  group('the agent\'s own answers', () {
    testWidgets('each option shows with the agent\'s name, refusals first, and the one chosen reaches the agent', (tester) async {
      final (agent, session, _) = await _shown(tester);
      agent.ask(5, 'Edit a.txt', _all);
      await _settle(tester);
      final names = [for (final choice in tester.widgetList<PermissionChoice>(find.byType(PermissionChoice))) choice.option.name];
      expect(names, ['No', 'No, never', 'Yes', 'Yes, and don\'t ask again for edits']);
      expect(find.textContaining('remembered by Scripted, not by Canoa'), findsOneWidget);
      await tester.tap(_choice('Yes, and don\'t ask again for edits'));
      await _settle(tester);
      expect(agent.answerTo(5)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-always'}});
      expect(session.pendingPermissions, isEmpty);
    });

    testWidgets('a refusal for good is the agent\'s option too', (tester) async {
      final (agent, _, _) = await _shown(tester);
      agent.ask(6, 'Edit a.txt', _all);
      await _settle(tester);
      await tester.tap(_choice('No, never'));
      await _settle(tester);
      expect(agent.answerTo(6)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'reject-always'}});
    });

    testWidgets('an option with no name is named by its kind, and one of a kind Canoa does not know is offered as it came', (tester) async {
      final (agent, _, _) = await _shown(tester);
      agent.ask(7, 'Edit a.txt', [
        {'optionId': 'a', 'kind': 'allow_once'},
        {'optionId': 'r', 'kind': 'reject_once'},
        {'optionId': 'v', 'name': 'Keep planning', 'kind': '_vendor'},
      ]);
      await _settle(tester);
      expect(_choice('Allow once'), findsOneWidget);
      expect(_choice('Refuse once'), findsOneWidget);
      expect(_choice('Keep planning'), findsOneWidget);
      expect(find.textContaining('not by Canoa'), findsNothing, reason: 'nothing here is remembered');
      await tester.tap(_choice('Keep planning'));
      await _settle(tester);
      expect(agent.answerTo(7)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'v'}});
    });

    testWidgets('a request with no option Canoa can read can only be cancelled', (tester) async {
      final (agent, _, _) = await _shown(tester);
      agent.ask(8, 'Edit a.txt', [{'optionId': 7}]);
      await _settle(tester);
      expect(find.byType(PermissionChoice), findsNothing);
      await tester.tap(find.descendant(of: find.byType(PermissionCard), matching: find.text('Cancel the request')));
      await _settle(tester);
      expect(agent.answerTo(8)?['result'], {'outcome': {'outcome': 'cancelled'}});
    });
  });

  group('full access', () {
    testWidgets('is off by default: a request shows, and no banner does', (tester) async {
      expect(const CanoaConfig().fullAccess, isEmpty);
      expect(CanoaConfig.parse('').fullAccess, isEmpty);
      final (agent, session, store) = await _shown(tester);
      expect(session.fullAccess, isNull);
      agent.ask(1, 'Edit a.txt', _all);
      await _settle(tester);
      expect(find.byType(PermissionCard), findsOneWidget);
      expect(find.byKey(const Key('full-access-banner')), findsNothing);
      expect(store.weakenings, isEmpty);
    });

    testWidgets('turned on for a conversation, after saying what it does, answers allow once with no card', (tester) async {
      final (agent, session, store) = await _shown(tester);
      // Full access is beside the agent's mode, under the message.
      await tester.tap(find.byKey(const Key('option-mode')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Full access…'));
      await tester.pumpAndSettle();
      expect(find.textContaining('without asking you'), findsOneWidget);
      await tester.tap(find.text('Turn on full access'));
      await tester.pumpAndSettle();
      expect(session.fullAccess, FullAccess.session);
      expect(store.weakenings.single, contains('full access'));
      expect(find.byKey(const Key('full-access-banner')), findsOneWidget);
      expect(find.descendant(of: find.byType(ListTile), matching: find.textContaining('Full access')), findsOneWidget,
          reason: 'the session row says so too');

      agent.ask(2, 'Edit a.txt', _all);
      await _settle(tester);
      expect(find.byType(PermissionCard), findsNothing);
      expect(agent.answerTo(2)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}},
          reason: 'never the option the agent would remember');
      expect(session.messages.last.text, contains('Full access allowed'));
    });

    testWidgets('never answers a request that offers no allow, or only one the agent would remember', (tester) async {
      final (agent, session, store) = await _shown(tester);
      store.setFullAccess(session, true);
      await tester.pump();
      agent.ask(3, 'Edit a.txt', _noAllow);
      await _settle(tester);
      expect(find.byType(PermissionCard), findsOneWidget);
      expect(agent.answerTo(3), isNull);
      await tester.tap(_choice('No'));
      await _settle(tester);
      agent.ask(4, 'Edit b.txt', _alwaysOnly);
      await _settle(tester);
      expect(find.byType(PermissionCard), findsOneWidget);
      expect(agent.answerTo(4), isNull);
      expect(find.textContaining('Full access answers only'), findsOneWidget);
    });

    testWidgets('is turned off in one action, and the next request shows', (tester) async {
      final (agent, session, store) = await _shown(tester);
      store.setFullAccess(session, true);
      await tester.pump();
      await tester.tap(find.descendant(of: find.byKey(const Key('full-access-banner')), matching: find.text('Turn off')));
      await tester.pump();
      expect(session.fullAccess, isNull);
      expect(find.byKey(const Key('full-access-banner')), findsNothing);
      expect(find.descendant(of: find.byType(ListTile), matching: find.textContaining('Full access')), findsNothing);
      agent.ask(9, 'Edit a.txt', _all);
      await _settle(tester);
      expect(find.byType(PermissionCard), findsOneWidget);
      expect(agent.answerTo(9), isNull);
    });

    testWidgets('per agent, from canoa.cfg: its sessions start with it, logged, and turning it off there stops it for the agent', (tester) async {
      final config = CanoaConfig.parse('[permissions]\nfull_access = ["Scripted"]\n');
      expect(config.fullAccess, {'Scripted'});
      expect(CanoaConfig.parse(config.encode()).fullAccess, {'Scripted'});
      CanoaConfig? saved;
      final store = SessionStore(includePreview: false, fullAccessAgents: config.fullAccess);
      await tester.binding.setSurfaceSize(const Size(1100, 900));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final agent = _Agent();
      final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
      await tester.pumpWidget(CanoaApp(store: store, config: config, writeConfig: (changed) async => saved = changed));
      await tester.pump();
      expect(session.fullAccess, FullAccess.agent);
      expect(store.weakenings.single, contains('Scripted'));
      agent.ask(10, 'Edit a.txt', _all);
      await _settle(tester);
      expect(agent.answerTo(10)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}});

      await tester.tap(find.descendant(of: find.byKey(const Key('full-access-banner')), matching: find.text('Turn off')));
      await _settle(tester);
      expect(session.fullAccess, isNull);
      expect(store.fullAccessAgents, isEmpty);
      expect(saved?.fullAccess, isEmpty, reason: 'canoa.cfg no longer names the agent');
    });

    testWidgets('Settings turns it on for an agent only after saying what it does', (tester) async {
      await tester.binding.setSurfaceSize(const Size(900, 1400));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      CanoaConfig? saved;
      await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
        onPressed: () async => saved = await showDialog<CanoaConfig>(
          context: context,
          builder: (_) => const SettingsDialog(config: CanoaConfig(), agents: ['Claude', 'Codex']),
        ),
        child: const Text('open'),
      ))));
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
      expect(find.text('Off for every agent'), findsOneWidget);
      await tester.tap(find.text('Full access'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(SwitchListTile, 'Claude'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Cancel').last);
      await tester.pumpAndSettle();
      expect(tester.widget<SwitchListTile>(find.widgetWithText(SwitchListTile, 'Claude')).value, isFalse);
      await tester.tap(find.widgetWithText(SwitchListTile, 'Claude'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Turn on full access'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      expect(saved?.fullAccess, {'Claude'});
    });

    testWidgets('a request reaches only the conversation of the agent that sent it, whatever session id it chose', (tester) async {
      final store = SessionStore(includePreview: false);
      final first = _Agent();
      final second = _Agent();
      final a = await tester.runAsync(() => first.start(store, label: 'First')) as ChatSession;
      final b = await tester.runAsync(() => second.start(store, label: 'Second')) as ChatSession;
      expect(a.remoteSessionId, b.remoteSessionId, reason: 'both agents chose s1');
      store.setFullAccess(a, true);
      second.ask(11, 'Edit a.txt', _all);
      await _settle(tester);
      expect(second.answerTo(11), isNull, reason: 'the first conversation\'s full access is not the second\'s');
      expect(b.pendingPermissions, hasLength(1));
      expect(a.pendingPermissions, isEmpty);
      expect(a.toolEntries, isEmpty, reason: 'its tool is drawn in its own conversation');
      expect(b.toolEntries.keys, ['t11']);
    });

    testWidgets('an agent\'s setting reaches only sessions of the program run under that name', (tester) async {
      final store = SessionStore(includePreview: false, fullAccessAgents: const {'Claude'});
      final agent = _Agent();
      // A connection whose agent calls itself Claude, which is not the program this app runs as Claude.
      agent.client = VerifiedAgent.fromChild(
        label: 'Elsewhere',
        messages: agent.incoming.stream,
        decider: store.askPermission,
        close: () async {},
        send: (line) async {
          final message = jsonDecode(line) as Map<String, dynamic>;
          if (message['method'] == 'initialize') {
            agent.reply(message['id'], {'protocolVersion': 1, 'agentCapabilities': <String, Object?>{}});
          } else if (message['method'] == 'session/new') {
            agent.reply(message['id'], {'sessionId': 's1'});
          }
        },
      );
      await tester.runAsync(() async {
        await agent.client.initialize();
        await agent.client.openSession(cwd: '/work');
      });
      final session = store.addProviderSession(title: 'T', agent: 'Claude', peerId: agent.client.peerId, remoteSessionId: 's1');
      store.attachVerifiedClient(session.id, agent.client);
      expect(session.fullAccess, isNull);
      store.setAgentFullAccess('Claude', false);
      store.setAgentFullAccess('Claude', true);
      expect(session.fullAccess, isNull);
      expect(store.weakenings, hasLength(1), reason: 'turning it on for the agent is logged, and reaches no session here');
    });

    testWidgets('what an agent or a person named is shown and logged without what could disguise it', (tester) async {
      final store = SessionStore(includePreview: false);
      final agent = _Agent();
      final session = await tester.runAsync(() => agent.start(store, label: 'Scripted')) as ChatSession;
      session.title = 'evil\u202Etxt\nweakened: nothing';
      store.setFullAccess(session, true);
      expect(store.weakenings.single, isNot(contains('\n')));
      expect(store.weakenings.single, isNot(contains('\u202E')));
    });

    testWidgets('questions past the most a conversation holds are cancelled and said', (tester) async {
      final (agent, session, _) = await _shown(tester);
      for (var id = 100; id < 100 + maxPendingPermissions + 2; id++) {
        agent.ask(id, 'Edit a.txt', _all);
      }
      await _settle(tester);
      expect(session.pendingPermissions, hasLength(maxPendingPermissions));
      expect(agent.answerTo(100 + maxPendingPermissions)?['result'], {'outcome': {'outcome': 'cancelled'}});
      expect(session.messages.where((message) => message.text.contains('more questions than')), isNotEmpty);
    });

    test('canoa.cfg refuses a full_access that is not a list of names', () {
      expect(() => CanoaConfig.parse('[permissions]\nfull_access = "Claude"\n'), throwsFormatException);
      expect(() => CanoaConfig.parse('[permissions]\nfull_access = [1]\n'), throwsFormatException);
      expect(() => CanoaConfig.parse('[permissions]\nask = false\n'), throwsFormatException);
    });
  });

  // The stand-in the coding-task test runs, as a real child: its requests offer every kind, and it remembers an "always" itself.
  final node = Platform.environment['CANOA_TEST_NODE'] ?? 'node';
  test('against the scripted agent, an always answer is the agent\'s to remember', () async {
    final dir = Directory.systemTemp.createTempSync('canoa-perm-');
    // The agent works in it, and Windows keeps it until the process has gone.
    addTearDown(() async {
      for (var tries = 0; ; tries++) {
        try {
          return dir.deleteSync(recursive: true);
        } on FileSystemException {
          if (tries > 100) rethrow;
          await Future<void>.delayed(const Duration(milliseconds: 100));
        }
      }
    });
    final store = SessionStore(includePreview: false);
    final client = await startLocalAgent(
      AgentCommand(label: 'Scripted', program: node, args: ['${Directory.current.path}/integration_test/scripted_agent.mjs', '--state=${dir.path}/s.json']),
      decider: store.askPermission,
      workingDirectory: dir.path,
    );
    addTearDown(client.close);
    final opened = await client.openSession(cwd: dir.path);
    final session = store.addProviderSession(title: 'S', agent: 'Scripted', peerId: client.peerId, remoteSessionId: opened.id);
    store.attachVerifiedClient(session.id, client);
    store.select(session.id);
    Future<void> until(bool Function() done) async {
      final deadline = DateTime.now().add(const Duration(seconds: 30));
      while (!done()) {
        if (DateTime.now().isAfter(deadline)) fail('timed out');
        await Future<void>.delayed(const Duration(milliseconds: 20));
      }
    }

    // A v1 prompt's answer is the end of its turn.
    final first = store.sendText('Rename addNumbers to sum');
    await until(() => session.pendingPermissions.isNotEmpty);
    final kinds = {for (final option in session.pendingPermissions.first.options) option.kind};
    expect(kinds, {'allow_once', 'allow_always', 'reject_once', 'reject_always'});
    store.choosePermission(session, session.pendingPermissions.first.options.firstWhere((option) => option.kind == 'allow_always'));
    await first;
    var asked = false;
    void watch() => asked |= session.pendingPermissions.isNotEmpty;
    store.addListener(watch);
    await store.sendText('Go on with the rename');
    store.removeListener(watch);
    expect(session.messages.any((message) => message.text == 'Renamed everywhere.'), isTrue);
    expect(asked, isFalse, reason: 'the agent remembered the edit, and asked nothing');
  }, skip: _runs(node) ? false : 'needs node', timeout: const Timeout(Duration(minutes: 1)));
}

bool _runs(String program) {
  try {
    return Process.runSync(program, ['--version'], runInShell: true).exitCode == 0;
  } on ProcessException {
    return false;
  }
}
