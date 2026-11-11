// "Always allow here": the card offers it with its scope on the button, a confirmed rule answers a matching request with allow
// once and nothing else, every use is said in the conversation, and Settings, Permissions lists the rules and revokes them.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/chat/permission_card.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/settings_dialog.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/main.dart';
import 'package:canoa/permissions/permission_rules.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// An agent in memory, speaking v1, whose requests are whatever a test says they are.
final class _Agent {
  final incoming = StreamController<String>();
  final sent = <Map<String, dynamic>>[];
  late VerifiedAgent client;

  Future<ChatSession> start(SessionStore store, {String label = 'Scripted', String folder = '/work/calc'}) async {
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
    final opened = await client.openSession(cwd: folder);
    final session = store.addProviderSession(title: label, agent: label, peerId: client.peerId, remoteSessionId: opened.id)..cwd = folder;
    store.attachVerifiedClient(session.id, client);
    store.select(session.id);
    return session;
  }

  void reply(Object? id, Object? result) => incoming.add(jsonEncode({'jsonrpc': '2.0', 'id': id, 'result': result}));

  void ask(int id, String title, {String kind = 'execute', Object? rawInput, List<Map<String, Object?>> options = _all}) =>
      incoming.add(jsonEncode({
        'jsonrpc': '2.0', 'id': id, 'method': 'session/request_permission',
        'params': {
          'sessionId': 's1',
          'toolCall': {'toolCallId': 't$id', 'title': title, 'kind': kind, 'rawInput': rawInput},
          'options': options,
        },
      }));

  void run(int id, String command, {List<Map<String, Object?>> options = _all}) =>
      ask(id, 'Run a command', rawInput: {'command': command}, options: options);

  void editFile(int id, String path, {List<Map<String, Object?>> options = _all}) =>
      ask(id, 'Edit a file', kind: 'edit', rawInput: {'file_path': path, 'old_string': 'a', 'new_string': 'b'}, options: options);

  void setMode(String value) => incoming.add(jsonEncode({
    'jsonrpc': '2.0', 'method': 'session/update',
    'params': {'sessionId': 's1', 'update': {'sessionUpdate': 'config_option_update', 'configOptions': [
      {'id': 'mode', 'name': 'Mode', 'category': 'mode', 'type': 'select', 'currentValue': value, 'options': [
        {'value': 'manual', 'name': 'Manual'}, {'value': 'accept_edits', 'name': 'Accept edits'},
      ]},
    ]}},
  }));

  Map<String, dynamic>? answerTo(int id) => sent.where((message) => message['id'] == id && message['method'] == null).firstOrNull;

  Object? chose(int id) => answerTo(id)?['result']?['outcome']?['optionId'];
}

const _all = [
  {'optionId': 'allow-always', 'name': 'Yes, and do not ask again', 'kind': 'allow_always'},
  {'optionId': 'allow-once', 'name': 'Yes', 'kind': 'allow_once'},
  {'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'},
];
const _alwaysOnly = [
  {'optionId': 'allow-always', 'name': 'Always', 'kind': 'allow_always'},
  {'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'},
];
const _noAllow = [
  {'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'},
];

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump();
}

final class _Shown {
  _Shown(this.agent, this.session, this.store, this.rules, this.backend);
  final _Agent agent;
  final ChatSession session;
  final SessionStore store;
  final PermissionRules rules;
  final MemoryLedgerBackend backend;
}

Future<_Shown> _shown(WidgetTester tester, {PermissionRules? rules, MemoryLedgerBackend? backend}) async {
  await tester.binding.setSurfaceSize(const Size(1100, 900));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final kept = backend ?? MemoryLedgerBackend();
  final list = rules ?? PermissionRules(kept);
  final store = SessionStore(includePreview: false, rules: list);
  final agent = _Agent();
  final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
  await tester.pumpWidget(CanoaApp(store: store, config: const CanoaConfig()));
  await tester.pump();
  return _Shown(agent, session, store, list, kept);
}

final _offer = find.byKey(const Key('always-allow-here'));

Future<void> _confirm(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('rule-confirm')));
  await _settle(tester);
}

void main() {
  group('the card', () {
    testWidgets('offers Always allow here with the folder on the button and says what it does', (tester) async {
      final s = await _shown(tester);
      s.agent.run(1, 'npm test');
      await _settle(tester);
      expect(find.descendant(of: _offer, matching: find.text('Always allow here: calc')), findsOneWidget);
      expect(find.textContaining('Canoa itself answers allow once'), findsOneWidget);
      expect(find.textContaining('in /work/calc'), findsWidgets);
      // The agent's own "always" is still offered beside it, and keeps being the agent's.
      expect(find.widgetWithText(PermissionChoice, 'Yes, and do not ask again'), findsOneWidget);
      expect(find.textContaining('remembered by Scripted, not by Canoa'), findsOneWidget);
    });

    testWidgets('offers it for no request that gives nothing to rule on, or that offers no allow once', (tester) async {
      final s = await _shown(tester);
      s.agent.ask(1, 'Read a file', kind: 'read', rawInput: {'file_path': 'a'});
      await _settle(tester);
      expect(find.byType(PermissionCard), findsOneWidget);
      expect(_offer, findsNothing, reason: 'a read has nothing a rule states');
      await tester.runAsync(() async => s.store.cancelPermission(s.session));
      s.agent.run(2, 'npm test', options: _alwaysOnly);
      await _settle(tester);
      expect(_offer, findsNothing, reason: 'with no allow once there is nothing Canoa could answer');
      s.store.cancelPermission(s.session);
      s.agent.run(3, 'npm test', options: _noAllow);
      await _settle(tester);
      expect(_offer, findsNothing);
      s.store.cancelPermission(s.session);
      s.agent.ask(4, 'Run', rawInput: {'command': 'ls', 'run_in_background': true});
      await _settle(tester);
      expect(_offer, findsNothing, reason: 'input that says more than a command is never ruled on');
      await _settle(tester);
    });

    testWidgets('is not offered where Canoa keeps no rules', (tester) async {
      await tester.binding.setSurfaceSize(const Size(1100, 900));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final store = SessionStore(includePreview: false);
      final agent = _Agent();
      await tester.runAsync(() => agent.start(store));
      await tester.pumpWidget(CanoaApp(store: store, config: const CanoaConfig()));
      agent.run(1, 'npm test');
      await _settle(tester);
      expect(find.byType(PermissionCard), findsOneWidget);
      expect(_offer, findsNothing);
    });
  });

  group('a rule for an exact command', () {
    testWidgets('is kept by confirming, answers this request with allow once, and says so', (tester) async {
      final s = await _shown(tester);
      s.agent.run(1, 'npm test');
      await _settle(tester);
      await tester.tap(_offer);
      await tester.pumpAndSettle();
      expect(find.text('Always allow here: calc'), findsWidgets);
      expect(find.byKey(const Key('rule-exact')), findsOneWidget);
      await _confirm(tester);
      expect(s.agent.chose(1), 'allow-once', reason: 'never the agent\'s always');
      expect(s.session.pendingPermissions, isEmpty);
      final rule = s.rules.rules.single;
      expect((rule.type, rule.value, rule.source, rule.folder, rule.tool), (RuleType.command, 'npm test', 'Scripted', '/work/calc', 'execute'));
      expect(s.session.messages.map((m) => m.text), contains(contains('Always allow here: Run: npm test, for Scripted in /work/calc')));
    });

    testWidgets('answers the same request later with allow once, with no card, and logs each use', (tester) async {
      final s = await _shown(tester);
      s.agent.run(1, 'npm test');
      await _settle(tester);
      await tester.tap(_offer);
      await tester.pumpAndSettle();
      await _confirm(tester);
      s.agent.run(2, 'npm test');
      await _settle(tester);
      s.agent.run(3, 'npm test');
      await _settle(tester);
      expect(find.byType(PermissionCard), findsNothing);
      expect(s.agent.chose(2), 'allow-once');
      expect(s.agent.chose(3), 'allow-once');
      expect(s.rules.rules.single.fired, 2);
      expect(s.rules.rules.single.lastUsed, isNotNull);
      final used = s.session.messages.where((m) => m.text.contains('A remembered rule allowed')).toList();
      expect(used, hasLength(2));
      expect(used.first.text, contains('Run: npm test'));
    });

    testWidgets('answers with allow once even when the agent lists its always first, and never when it offers no allow once',
        (tester) async {
      final s = await _shown(tester);
      s.rules.add(source: 'Scripted', folder: '/work/calc', kind: 'execute', type: RuleType.command, value: 'npm test');
      s.agent.run(1, 'npm test');
      await _settle(tester);
      expect(s.agent.chose(1), 'allow-once');
      s.agent.run(2, 'npm test', options: _alwaysOnly);
      await _settle(tester);
      expect(s.agent.answerTo(2), isNull, reason: 'a rule never takes an always');
      expect(find.byType(PermissionCard), findsOneWidget);
      s.store.cancelPermission(s.session);
      s.agent.run(3, 'npm test', options: _noAllow);
      await _settle(tester);
      expect(s.agent.answerTo(3), isNull);
      expect(s.rules.rules.single.fired, 1);
    });

    testWidgets('a near-miss command, another folder and another source still ask', (tester) async {
      final s = await _shown(tester);
      s.rules.add(source: 'Scripted', folder: '/work/calc', kind: 'execute', type: RuleType.command, value: 'npm test');
      for (final (i, command) in ['npm test -- a.js', 'npm test; rm -rf ~', 'npm  test', 'NPM test'].indexed) {
        s.agent.run(10 + i, command);
        await _settle(tester);
        expect(s.agent.answerTo(10 + i), isNull, reason: command);
        s.store.cancelPermission(s.session);
      }
      // Another folder and another source: their own sessions, same rule list.
      final other = _Agent(), elsewhere = _Agent();
      final inOther = await tester.runAsync(() => other.start(s.store, label: 'Scripted', folder: '/work/other')) as ChatSession;
      final fromOther = await tester.runAsync(() => elsewhere.start(s.store, label: 'Other', folder: '/work/calc')) as ChatSession;
      other.run(20, 'npm test');
      elsewhere.run(21, 'npm test');
      await _settle(tester);
      expect(other.answerTo(20), isNull, reason: 'another folder');
      expect(elsewhere.answerTo(21), isNull, reason: 'another source');
      expect(inOther.pendingPermissions, hasLength(1));
      expect(fromOther.pendingPermissions, hasLength(1));
      expect(s.rules.rules.single.fired, 0);
    });

    testWidgets('survives a restart: a new list over the same file still answers', (tester) async {
      final s = await _shown(tester);
      s.agent.run(1, 'npm test');
      await _settle(tester);
      await tester.tap(_offer);
      await tester.pumpAndSettle();
      await _confirm(tester);
      await tester.runAsync(s.rules.flush);
      final again = PermissionRules(s.backend);
      await tester.runAsync(again.load);
      expect(again.rules.single.value, 'npm test');
      final next = SessionStore(includePreview: false, rules: again);
      final agent = _Agent();
      await tester.runAsync(() => agent.start(next));
      agent.run(5, 'npm test');
      await _settle(tester);
      expect(agent.chose(5), 'allow-once');
    });
  });

  group('a rule for a prefix', () {
    Future<_Shown> openDialog(WidgetTester tester, String command) async {
      final s = await _shown(tester);
      s.agent.run(1, command);
      await _settle(tester);
      await tester.tap(_offer);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('rule-prefix')));
      await tester.pumpAndSettle();
      return s;
    }

    testWidgets('covers arguments after the edited prefix and nothing past it', (tester) async {
      final s = await openDialog(tester, 'npm test -- a.js');
      await tester.enterText(find.byKey(const Key('rule-value')), 'npm test');
      await _confirm(tester);
      expect(s.agent.chose(1), 'allow-once');
      expect(s.rules.rules.single.type, RuleType.prefix);
      s.agent.run(2, 'npm test --silent');
      await _settle(tester);
      expect(s.agent.chose(2), 'allow-once');
      for (final (i, command) in ['npm testing', 'npm test; rm -rf ~', 'npm test && x', 'npm', 'npm test\nrm', r'npm test $(id)'].indexed) {
        s.agent.run(30 + i, command);
        await _settle(tester);
        expect(s.agent.answerTo(30 + i), isNull, reason: command);
        s.store.cancelPermission(s.session);
      }
    });

    testWidgets('refuses a prefix that is not the start of the command, or holds a second command, and keeps nothing', (tester) async {
      final s = await openDialog(tester, 'npm test -- a.js');
      for (final bad in ['yarn', 'npm test -- a.js; rm', 'npm test ', '', 'npm test && curl x']) {
        await tester.enterText(find.byKey(const Key('rule-value')), bad);
        await tester.tap(find.byKey(const Key('rule-confirm')));
        await tester.pump();
        expect(find.byKey(const Key('rule-problem')), findsOneWidget, reason: '"$bad"');
        expect(s.rules.rules, isEmpty);
        expect(s.session.pendingPermissions, hasLength(1));
      }
      expect(s.agent.answerTo(1), isNull);
    });
  });

  group('a rule for changes to a file', () {
    testWidgets('is made for the file or a folder under the session\'s, and covers nothing outside it', (tester) async {
      final s = await _shown(tester);
      s.agent.editFile(1, '/work/calc/src/a.js');
      await _settle(tester);
      await tester.tap(_offer);
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('rule-value')), '/work/calc/src');
      await _confirm(tester);
      expect(s.agent.chose(1), 'allow-once');
      expect(s.rules.rules.single.type, RuleType.path);
      s.agent.editFile(2, '/work/calc/src/deep/b.js');
      await _settle(tester);
      expect(s.agent.chose(2), 'allow-once');
      for (final (i, path) in ['/work/calc/lib/a.js', '/work/calc/src/../lib/a.js', '/etc/passwd', '/work/calc/srcx/a.js'].indexed) {
        s.agent.editFile(40 + i, path);
        await _settle(tester);
        expect(s.agent.answerTo(40 + i), isNull, reason: path);
        s.store.cancelPermission(s.session);
      }
    });

    testWidgets('cannot be made wider than the session\'s folder', (tester) async {
      final s = await _shown(tester);
      s.agent.editFile(1, '/work/calc/src/a.js');
      await _settle(tester);
      await tester.tap(_offer);
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('rule-value')), '/work');
      await tester.tap(find.byKey(const Key('rule-confirm')));
      await tester.pump();
      expect(find.byKey(const Key('rule-problem')), findsOneWidget);
      expect(s.rules.rules, isEmpty);
    });
  });

  group('a rule for an MCP tool', () {
    testWidgets('is made for that tool on that server', (tester) async {
      final s = await _shown(tester);
      s.agent.ask(1, 'mcp__marker__mark', kind: 'other', rawInput: {'text': 'x'});
      await _settle(tester);
      await tester.tap(_offer);
      await tester.pumpAndSettle();
      expect(find.textContaining('Any use of the tool mark of marker'), findsOneWidget);
      await _confirm(tester);
      expect(s.agent.chose(1), 'allow-once');
      s.agent.ask(2, 'mcp__marker__mark', kind: 'other', rawInput: {'text': 'y'});
      s.agent.ask(3, 'mcp__marker__erase', kind: 'other', rawInput: {'text': 'y'});
      await _settle(tester);
      expect(s.agent.chose(2), 'allow-once');
      expect(s.agent.answerTo(3), isNull);
    });
  });

  group('Settings, Permissions', () {
    Future<void> open(WidgetTester tester, PermissionRules rules) async {
      await tester.binding.setSurfaceSize(const Size(1100, 1400));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: SettingsDialog(config: const CanoaConfig(), rules: rules))));
      await tester.tap(find.byKey(const Key('settings-permissions')));
      await tester.pumpAndSettle();
    }

    testWidgets('lists each rule with when it was made and how often it answered, and revokes it in one action', (tester) async {
      final rules = PermissionRules(MemoryLedgerBackend(), clock: () => DateTime.utc(2026, 11, 12, 9));
      final rule = rules.add(source: 'Scripted', folder: '/work/calc', kind: 'execute', type: RuleType.command, value: 'npm test')!;
      rules.recordUse(rule);
      rules.recordUse(rule);
      await open(tester, rules);
      expect(find.text('Run: npm test'), findsOneWidget);
      expect(find.textContaining('Scripted in /work/calc'), findsOneWidget);
      expect(find.textContaining('Made 2026-11-12; used 2 times'), findsOneWidget);
      await tester.tap(find.byKey(Key('revoke-${rule.id}')));
      await tester.pumpAndSettle();
      expect(rules.rules, isEmpty);
      expect(find.text('Run: npm test'), findsNothing);
      expect(find.textContaining('Nothing is allowed without asking'), findsOneWidget);
    });

    testWidgets('a revoked rule asks again', (tester) async {
      final s = await _shown(tester);
      final rule = s.rules.add(source: 'Scripted', folder: '/work/calc', kind: 'execute', type: RuleType.command, value: 'npm test')!;
      s.agent.run(1, 'npm test');
      await _settle(tester);
      expect(s.agent.chose(1), 'allow-once');
      s.rules.revoke(rule.id);
      s.agent.run(2, 'npm test');
      await _settle(tester);
      expect(s.agent.answerTo(2), isNull);
      expect(find.byType(PermissionCard), findsOneWidget);
    });
  });

  group('the agent\'s own always', () {
    testWidgets('is still offered, and a change of mode that follows it is said', (tester) async {
      final s = await _shown(tester);
      s.agent.setMode('manual');
      await _settle(tester);
      s.agent.editFile(1, '/work/calc/a.js');
      await _settle(tester);
      await tester.tap(find.widgetWithText(PermissionChoice, 'Yes, and do not ask again'));
      await _settle(tester);
      expect(s.agent.chose(1), 'allow-always');
      expect(s.rules.rules, isEmpty, reason: 'Canoa adds no rule on top of the agent\'s own');
      s.agent.setMode('accept_edits');
      await _settle(tester);
      expect(s.session.messages.map((m) => m.text), contains(contains('After your "always" answer, Scripted switched this session to Accept edits')));
    });

    testWidgets('a change of mode with no always before it is not blamed on one', (tester) async {
      final s = await _shown(tester);
      s.agent.setMode('manual');
      await _settle(tester);
      s.agent.setMode('accept_edits');
      await _settle(tester);
      expect(s.session.messages.where((m) => m.text.contains('After your "always"')), isEmpty);
    });
  });
}
