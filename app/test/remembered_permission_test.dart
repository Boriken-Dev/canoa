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
import 'package:canoa/sources/sources.dart';
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

  void ask(int id, String title, {String kind = 'execute', Object? rawInput, List<Map<String, Object?>> options = _all, String? toolName}) =>
      incoming.add(jsonEncode({
        'jsonrpc': '2.0', 'id': id, 'method': 'session/request_permission',
        'params': {
          'sessionId': 's1',
          'toolCall': {
            'toolCallId': 't$id', 'title': title, 'kind': kind, 'rawInput': rawInput,
            if (toolName != null) '_meta': {'claudeCode': {'toolName': toolName}},
          },
          'options': options,
        },
      }));

  void run(int id, String command, {List<Map<String, Object?>> options = _all, String? toolName}) =>
      ask(id, 'Run a command', rawInput: {'command': command}, options: options, toolName: toolName);

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
  for (var round = 0; round < 3; round++) {
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
    await tester.pump();
  }
}

final class _Shown {
  _Shown(this.agent, this.session, this.store, this.rules, this.backend);
  final _Agent agent;
  final ChatSession session;
  final SessionStore store;
  final PermissionRules rules;
  final MemoryLedgerBackend backend;
}

/// Links that lead nowhere else.
String? _plain(String path) => path;

Future<_Shown> _shown(WidgetTester tester, {PermissionRules? rules, MemoryLedgerBackend? backend, String mode = 'manual'}) async {
  await tester.binding.setSurfaceSize(const Size(1100, 900));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final kept = backend ?? MemoryLedgerBackend();
  final list = rules ?? PermissionRules(kept, real: _plain);
  final store = SessionStore(includePreview: false, rules: list);
  final agent = _Agent();
  final session = await tester.runAsync(() => agent.start(store)) as ChatSession;
  await tester.pumpWidget(CanoaApp(store: store, config: const CanoaConfig()));
  await tester.pump();
  agent.setMode(mode);
  await _settle(tester);
  return _Shown(agent, session, store, list, kept);
}

final _offer = find.byKey(const Key('always-allow-here'));

/// A rule as the dialog would make it for [s]'s session.
PermissionRule _add(_Shown s, {String kind = 'execute', required RuleType type, required String value, String toolName = ''}) =>
    s.rules.add(source: 'Scripted', peer: s.session.peerId ?? '', folder: '/work/calc', kind: kind, toolName: toolName, type: type, value: value)!;

/// Taps confirm and expects the dialog to stay, with its problem shown.
Future<void> _confirmKeeping(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('rule-confirm')));
  await tester.pump();
}

Future<void> _confirm(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('rule-confirm')));
  await _settle(tester);
}

/// A rules file whose reads wait while [hold] is set, as a slow disk does.
final class _GatedBackend implements LedgerBackend {
  String? document;
  bool hold = false;
  final _waiting = <Completer<void>>[];

  void release() {
    hold = false;
    for (final waiting in _waiting) {
      waiting.complete();
    }
    _waiting.clear();
  }

  @override
  String get location => 'a slow file';

  @override
  Future<String?> read() async {
    if (hold) {
      final waiting = Completer<void>();
      _waiting.add(waiting);
      await waiting.future;
    }
    return document;
  }

  @override
  Future<void> write(String document) async => this.document = document;
}

void raceTests() {
  group('a request waiting for the rules to be read again', () {
    Future<(_Shown, _GatedBackend)> gated(WidgetTester tester, {String mode = 'manual'}) async {
      final backend = _GatedBackend();
      final s = await _shown(tester, rules: PermissionRules(backend, real: _plain), mode: mode);
      _add(s, type: RuleType.command, value: 'npm test');
      await tester.runAsync(s.rules.settled);
      backend.hold = true;
      return (s, backend);
    }

    Future<void> done(WidgetTester tester, _Shown s, _GatedBackend backend) async {
      await tester.runAsync(() async => backend.release());
      await _settle(tester);
    }

    testWidgets('is dropped when the person presses Stop meanwhile, and not answered allow once after it', (tester) async {
      final (s, backend) = await gated(tester);
      s.agent.run(1, 'npm test');
      await _settle(tester);
      expect(s.session.pendingPermissions, isEmpty, reason: 'it waits on the read');
      await tester.runAsync(s.store.cancelActiveSession);
      await done(tester, s, backend);
      expect(s.agent.chose(1), isNull, reason: 'a turn that was stopped is not allowed to go on');
      expect(s.agent.answerTo(1)?['result'], {'outcome': {'outcome': 'cancelled'}});
      expect(s.rules.rules.single.fired, 0);
      expect(s.session.messages.where((m) => m.text.contains('A remembered rule allowed')), isEmpty);
    });

    testWidgets('is dropped when the conversation is closed meanwhile', (tester) async {
      final (s, backend) = await gated(tester);
      s.agent.run(1, 'npm test');
      await _settle(tester);
      await tester.runAsync(() => s.store.close(s.session));
      await done(tester, s, backend);
      expect(s.agent.chose(1), isNull);
      expect(s.rules.rules.single.fired, 0);
    });

    testWidgets('is not answered by a rule once the mode accepts edits, which it learns of meanwhile', (tester) async {
      final (s, backend) = await gated(tester);
      s.agent.run(1, 'npm test');
      await _settle(tester);
      s.agent.setMode('accept_edits');
      await _settle(tester);
      await done(tester, s, backend);
      expect(s.agent.answerTo(1), isNull);
      expect(s.session.pendingPermissions, hasLength(1), reason: 'it asks, as the mode now says');
      expect(s.rules.rules.single.fired, 0);
    });

    testWidgets('is answered by Bypass, not by the rule, when Bypass was chosen meanwhile', (tester) async {
      final (s, backend) = await gated(tester);
      s.agent.run(1, 'npm test');
      await _settle(tester);
      s.session.bypass = true;
      await done(tester, s, backend);
      expect(s.agent.chose(1), 'allow-once');
      expect(s.rules.rules.single.fired, 0, reason: 'Bypass answered it');
      expect(s.session.messages.where((m) => m.text.contains('Bypass allowed')), isNotEmpty);
    });

    testWidgets('keeps the cards in the order the requests came, whether or not they needed the read', (tester) async {
      final (s, backend) = await gated(tester);
      s.agent.run(1, 'cargo test');
      s.agent.ask(2, 'Read a file', kind: 'read', rawInput: {'file_path': '/work/calc/a'});
      s.agent.run(3, 'cargo build');
      await _settle(tester);
      expect(s.session.pendingPermissions, isEmpty, reason: 'the later ones do not pass the first');
      await done(tester, s, backend);
      expect([for (final asked in s.session.pendingPermissions) asked.toolCallId], ['t1', 't2', 't3']);
    });
  });
}

void toolRuleTests() {
  group('a rule for an MCP tool is held to the mode like a command', () {
    testWidgets('it does not apply while the mode accepts edits, and it is not offered', (tester) async {
      final s = await _shown(tester, mode: 'accept_edits');
      _add(s, kind: 'other', type: RuleType.mcp, value: 'mcp__marker__mark', toolName: 'mcp__marker__mark');
      s.agent.ask(1, 'mark', kind: 'other', rawInput: {'x': 1}, toolName: 'mcp__marker__mark');
      await _settle(tester);
      expect(s.agent.answerTo(1), isNull);
      expect(_offer, findsNothing);
    });

    testWidgets('it is never offered for a tool that executes', (tester) async {
      final s = await _shown(tester);
      s.agent.ask(1, 'run', kind: 'other', rawInput: {'x': 1}, toolName: 'mcp__sh__run_command');
      s.agent.ask(2, 'list', kind: 'execute', rawInput: {'x': 1}, toolName: 'mcp__marker__list');
      await _settle(tester);
      expect(s.session.pendingPermissions.map((asked) => asked.subject), [isNull, isNull]);
    });
  });
}

void main() {
  raceTests();
  toolRuleTests();
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
      _add(s, type: RuleType.command, value: 'npm test');
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
      _add(s, type: RuleType.command, value: 'npm test');
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
      other.setMode('manual');
      elsewhere.setMode('manual');
      await _settle(tester);
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
      await tester.runAsync(s.rules.settled);
      final again = PermissionRules(s.backend, real: _plain);
      await tester.runAsync(again.load);
      expect(again.rules.single.value, 'npm test');
      final next = SessionStore(includePreview: false, rules: again);
      final agent = _Agent();
      await tester.runAsync(() => agent.start(next));
      agent.setMode('manual');
      await _settle(tester);
      agent.run(5, 'npm test');
      await _settle(tester);
      expect(agent.chose(5), 'allow-once');
    });
  });

  group('there is no rule for a prefix', () {
    testWidgets('the dialog offers the exact command and nothing else', (tester) async {
      final s = await _shown(tester);
      s.agent.run(1, 'npm test -- a.js');
      await _settle(tester);
      await tester.tap(_offer);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('rule-prefix')), findsNothing);
      expect(find.byKey(const Key('rule-value')), findsNothing);
      expect(tester.widget<SelectableText>(find.byKey(const Key('rule-exact'))).data, 'npm test -- a.js');
      await _confirm(tester);
      expect(s.rules.rules.single.type, RuleType.command);
      s.agent.run(2, 'npm test -- a.js --bail');
      s.agent.run(3, 'npm test');
      await _settle(tester);
      expect(s.agent.answerTo(2), isNull);
      expect(s.agent.answerTo(3), isNull);
    });
  });

  group('the mode', () {
    testWidgets('applies no command rule while it accepts edits, and offers none', (tester) async {
      final s = await _shown(tester, mode: 'accept_edits');
      _add(s, type: RuleType.command, value: 'npm test');
      s.agent.run(1, 'npm test');
      await _settle(tester);
      expect(s.agent.answerTo(1), isNull);
      expect(find.byType(PermissionCard), findsOneWidget);
      expect(_offer, findsNothing);
      s.agent.setMode('manual');
      await _settle(tester);
      s.agent.run(2, 'npm test');
      await _settle(tester);
      expect(s.agent.chose(2), 'allow-once', reason: 'back in Manual the rule applies');
    });

    testWidgets('applies none in Canoa\'s own Bypass, in a mode it does not know, or before the agent has said', (tester) async {
      final s = await _shown(tester);
      _add(s, type: RuleType.command, value: 'npm test');
      s.session.bypass = true;
      s.agent.run(1, 'npm test', options: _noAllow);
      await _settle(tester);
      expect(s.agent.answerTo(1), isNull);
      s.store.cancelPermission(s.session);
      s.session.bypass = false;
      s.agent.setMode('accept_edits');
      await _settle(tester);
      s.agent.run(2, 'npm test');
      await _settle(tester);
      expect(s.agent.answerTo(2), isNull);
      final unsaid = await _shown(tester, mode: 'unknown-mode');
      _add(unsaid, type: RuleType.command, value: 'npm test');
      unsaid.agent.run(3, 'npm test');
      await _settle(tester);
      expect(unsaid.agent.answerTo(3), isNull);
    });

    testWidgets('does not hold back a tool or a file change', (tester) async {
      final s = await _shown(tester, mode: 'accept_edits');
      s.agent.editFile(1, '/work/calc/src/a.js');
      await _settle(tester);
      expect(_offer, findsOneWidget);
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

  group('where a file change is remembered', () {
    for (final place in [SessionPlace.host, SessionPlace.peer, SessionPlace.browser]) {
      testWidgets('not for a session on a ${place.name} place', (tester) async {
        final s = await _shown(tester);
        s.session.place = place;
        s.agent.editFile(1, '/work/calc/src/a.js');
        s.agent.run(2, 'npm test');
        await _settle(tester);
        final asked = s.session.pendingPermissions;
        expect(asked, hasLength(2));
        expect(asked.first.subject, isNull, reason: 'a file change has no rule here');
        expect(asked.last.subject, isA<CommandSubject>(), reason: 'a command still does');
      });
    }

    testWidgets('on this computer it has one', (tester) async {
      final s = await _shown(tester);
      s.agent.editFile(1, '/work/calc/src/a.js');
      await _settle(tester);
      expect(s.session.pendingPermissions.single.subject, isA<PathSubject>());
    });

    testWidgets('and a rule made for a local session does not answer for a hosted one', (tester) async {
      final s = await _shown(tester);
      _add(s, kind: 'edit', type: RuleType.path, value: '/work/calc/src');
      s.session.place = SessionPlace.host;
      s.agent.editFile(1, '/work/calc/src/a.js');
      await _settle(tester);
      expect(s.agent.answerTo(1), isNull);
    });

    testWidgets('not where links cannot be followed, as in a browser', (tester) async {
      final backend = MemoryLedgerBackend();
      final s = await _shown(tester, rules: PermissionRules(backend), backend: backend);
      s.agent.editFile(1, '/work/calc/src/a.js');
      await _settle(tester);
      expect(find.byType(PermissionCard), findsOneWidget);
      expect(_offer, findsNothing);
    });

    testWidgets('not where the session folder is home, holds it, or is Canoa\'s settings', (tester) async {
      final backend = MemoryLedgerBackend();
      final guarded = PermissionRules(backend, real: _plain, guards: const PermissionGuards(home: '/work/calc', config: '/cfg'));
      final s = await _shown(tester, rules: guarded, backend: backend);
      s.agent.editFile(1, '/work/calc/src/a.js');
      await _settle(tester);
      expect(find.byType(PermissionCard), findsOneWidget);
      expect(_offer, findsNothing);
      s.store.cancelPermission(s.session);
      final above = PermissionRules(MemoryLedgerBackend(), real: _plain, guards: const PermissionGuards(home: '/work/calc/home'));
      final t = await _shown(tester, rules: above);
      t.agent.editFile(1, '/work/calc/src/a.js');
      await _settle(tester);
      expect(_offer, findsNothing, reason: 'the folder holds home');
    });
  });

  group('a rule for an MCP tool', () {
    testWidgets('is made for that tool on that server', (tester) async {
      final s = await _shown(tester);
      s.agent.ask(1, 'mark', kind: 'other', rawInput: {'text': 'x'}, toolName: 'mcp__marker__mark');
      await _settle(tester);
      await tester.tap(_offer);
      await tester.pumpAndSettle();
      expect(find.textContaining('Any use of the tool mark of marker'), findsOneWidget);
      await _confirm(tester);
      expect(s.agent.chose(1), 'allow-once');
      s.agent.ask(2, 'mark', kind: 'other', rawInput: {'text': 'y'}, toolName: 'mcp__marker__mark');
      s.agent.ask(3, 'mark', kind: 'other', rawInput: {'text': 'y'}, toolName: 'mcp__marker__erase');
      s.agent.ask(4, 'mcp__marker__mark', kind: 'other', rawInput: {'text': 'y'});
      await _settle(tester);
      expect(s.agent.chose(2), 'allow-once');
      expect(s.agent.answerTo(3), isNull);
      expect(s.agent.answerTo(4), isNull, reason: 'a title is not a tool name');
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
      rules.recordUse(rule, 'npm test');
      rules.recordUse(rule, 'npm test');
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

    testWidgets('flags a rule the app did not write, and says when a write failed', (tester) async {
      final backend = MemoryLedgerBackend()
        ..document = '{"version":1,"rules":[{"id":"hand","source":"s","folder":"/f","tool":"execute","type":"command","value":"ls","made":"2026-11-12T00:00:00Z"}]}';
      final rules = PermissionRules(backend);
      await tester.runAsync(rules.load);
      rules.writeProblem = 'could not write the file: disk full';
      await open(tester, rules);
      expect(find.textContaining('Not made in this app'), findsOneWidget);
      expect(find.byKey(const Key('rules-write-problem')), findsOneWidget);
    });

    testWidgets('shows a rule an earlier version wrote as no longer supported, and revokes it', (tester) async {
      final backend = MemoryLedgerBackend()
        ..document = '{"version":1,"rules":[{"id":"old","source":"s","folder":"/f","tool":"execute","type":"prefix","value":"npm test","made":"2026-11-01T00:00:00Z","by":"canoa"}]}';
      final rules = PermissionRules(backend);
      await tester.runAsync(rules.load);
      await open(tester, rules);
      expect(find.textContaining('No longer supported (prefix): npm test'), findsOneWidget);
      await tester.tap(find.byKey(const Key('revoke-old')));
      await tester.pumpAndSettle();
      expect(rules.rules, isEmpty);
    });

    testWidgets('a rule revoked by another instance stops answering before the next request is matched', (tester) async {
      final s = await _shown(tester);
      final rule = _add(s, type: RuleType.command, value: 'npm test');
      await tester.runAsync(s.rules.settled);
      final other = PermissionRules(s.backend, real: _plain);
      await tester.runAsync(other.load);
      other.revoke(rule.id);
      await tester.runAsync(other.settled);
      s.agent.run(1, 'npm test');
      await _settle(tester);
      expect(s.agent.answerTo(1), isNull);
      expect(s.rules.rules, isEmpty);
    });

    testWidgets('a revoked rule asks again', (tester) async {
      final s = await _shown(tester);
      final rule = _add(s, type: RuleType.command, value: 'npm test');
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

  group('what the dialog refuses and says', () {
    testWidgets('says what a command rule runs and what a path rule never covers', (tester) async {
      final s = await _shown(tester);
      s.agent.run(1, 'npm test');
      await _settle(tester);
      await tester.tap(_offer);
      await tester.pumpAndSettle();
      expect(find.textContaining('runs whatever the files in this folder make it do'), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(s.session.pendingPermissions, hasLength(1), reason: 'cancelling keeps nothing and answers nothing');
      s.store.cancelPermission(s.session);
      s.agent.editFile(2, '/work/calc/src/a.js');
      await _settle(tester);
      await tester.tap(_offer);
      await tester.pumpAndSettle();
      expect(find.textContaining('Never a hidden file or folder, build scripts'), findsOneWidget);
    });

    testWidgets('refuses a path rule for version control, however it is asked for', (tester) async {
      final s = await _shown(tester);
      s.agent.editFile(1, '/work/calc/.git/hooks/pre-commit');
      await _settle(tester);
      await tester.tap(_offer);
      await tester.pumpAndSettle();
      await _confirmKeeping(tester);
      expect(find.byKey(const Key('rule-problem')), findsOneWidget);
      await tester.enterText(find.byKey(const Key('rule-value')), '/work/calc');
      await _confirmKeeping(tester);
      expect(find.byKey(const Key('rule-problem')), findsOneWidget, reason: 'a wider scope that holds it is refused too');
      expect(s.rules.rules, isEmpty);
    });

    testWidgets('offers no rule for a path that is relative, starts with ~, names a stream or is a network path', (tester) async {
      final s = await _shown(tester);
      var id = 1;
      for (final path in ['src/a.js', '~/a.js', '/work/calc/a.js:stream', r'\\server\share\a.js', 'C:a.js']) {
        s.agent.editFile(id, path);
        await _settle(tester);
        expect(_offer, findsNothing, reason: path);
        expect(find.byType(PermissionCard), findsOneWidget, reason: path);
        s.store.cancelPermission(s.session);
        id++;
      }
    });

    testWidgets('keeps commands and file changes apart for one source and folder', (tester) async {
      final s = await _shown(tester);
      _add(s, type: RuleType.command, value: 'npm test');
      s.agent.editFile(1, '/work/calc/src/a.js');
      await _settle(tester);
      await tester.tap(_offer);
      await tester.pumpAndSettle();
      await _confirmKeeping(tester);
      expect(find.byKey(const Key('rule-problem')), findsOneWidget);
      expect(find.textContaining('A rule for commands or tools already stands'), findsOneWidget);
      expect(s.rules.rules, hasLength(1));
      expect(s.agent.answerTo(1), isNull);
    });

    testWidgets('a rule made for Bash is not one for PowerShell', (tester) async {
      final s = await _shown(tester);
      s.agent.run(1, 'npm test', toolName: 'Bash');
      await _settle(tester);
      await tester.tap(_offer);
      await tester.pumpAndSettle();
      await _confirm(tester);
      expect(s.rules.rules.single.toolName, 'Bash');
      s.agent.run(2, 'npm test', toolName: 'Bash');
      s.agent.run(3, 'npm test', toolName: 'PowerShell');
      s.agent.run(4, 'npm test');
      await _settle(tester);
      expect(s.agent.chose(2), 'allow-once');
      expect(s.agent.answerTo(3), isNull);
      expect(s.agent.answerTo(4), isNull);
    });

    testWidgets('a rule is made for the request the dialog was opened for, not the one that is first by then', (tester) async {
      final s = await _shown(tester);
      s.agent.run(1, 'npm test');
      s.agent.run(2, 'cargo test');
      await _settle(tester);
      final opened = s.session.pendingPermissions.first;
      s.store.cancelPermission(s.session);
      expect(s.store.rememberHere(s.session, opened, RuleType.command, 'npm test'), contains('no longer waiting'));
      expect(s.rules.rules, isEmpty);
      expect(s.agent.answerTo(2), isNull, reason: 'the second request was not answered by a rule made for the first');
      expect(s.session.pendingPermissions.single.title, 'Run a command');
      final second = s.session.pendingPermissions.single;
      expect(s.store.rememberHere(s.session, second, RuleType.command, 'npm test'), isNotNull, reason: 'the scope is not the command asked');
      expect(s.store.rememberHere(s.session, second, RuleType.command, 'cargo test'), isNull);
      await _settle(tester);
      expect(s.agent.chose(2), 'allow-once');
    });

    testWidgets('two sources that share a label do not share rules', (tester) async {
      final s = await _shown(tester);
      expect(SessionStore.ruleSource(s.session), 'Scripted');
      s.session.source = const AgentSource.hosted('Scripted');
      expect(SessionStore.ruleSource(s.session), 'canoa/Scripted');
      s.rules.add(source: 'Scripted', peer: s.session.peerId ?? '', folder: '/work/calc', kind: 'execute', type: RuleType.command, value: 'npm test');
      s.agent.run(1, 'npm test');
      await _settle(tester);
      expect(s.agent.answerTo(1), isNull, reason: 'a preset\'s rule is not a hosted entry\'s');
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
