// What a mode is when two parties hold it: Canoa answers requests only under its own Bypass; Shift+Tab goes on from where the
// last press went and skips what the agent refused; Canoa's Bypass sits on the agent's mode; the ledger keeps what the person
// or the agent set and not the agent's starting default.
import 'package:canoa/chat/modes.dart';
import 'package:canoa/drafts/draft_store.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

SessionStore _store([SessionLedger? ledger]) => SessionStore(includePreview: false, ledger: ledger ?? SessionLedger(MemoryLedgerBackend()));

const _allow = [
  {'optionId': 'allow-once', 'name': 'Yes', 'kind': 'allow_once'},
  {'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'},
];

Future<void> _started(WidgetTester tester, OptionsAgent agent, SessionStore store) async {
  await pumpApp(tester, store, startAgent: agent.start);
  await newSession(tester);
  await chooseSource(tester, 'Scripted');
  await openFolder(tester);
  await send(tester, 'Say hello');
}

Future<void> _asked(WidgetTester tester, OptionsAgent agent, int id,
    {List<Map<String, Object?>> options = _allow, Object? rawInput, String kind = 'edit'}) async {
  agent.ask(id, options, rawInput: rawInput, kind: kind);
  await settle(tester);
}

void main() {
  group('Canoa answers only under its own Bypass', () {
    testWidgets('not when the agent is in its own Bypass: what it still asks is the person\'s', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      final store = _store();
      await _started(tester, agent, store);
      await pick(tester, 'option-mode', 'Bypass');
      expect(agent.mode, 'bypass');
      expect(store.inBypass(store.selected!), isTrue, reason: 'shown as a status');
      await _asked(tester, agent, 11);
      expect(agent.answers[11], isNull);
      expect(store.selected!.pendingPermissions, hasLength(1));
      await finish(tester);
    });

    testWidgets('not for a choice still waiting for the next turn, and not until it is applied', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes)..holdTurns = true;
      final store = _store();
      await _started(tester, agent, store);
      await _asked(tester, agent, 12);
      await pick(tester, 'option-mode', 'Bypass');
      expect(find.text('Bypass (next turn)'), findsOneWidget);
      expect(store.inBypass(store.selected!), isFalse, reason: 'not yet in effect');
      expect(store.selected!.pendingPermissions, hasLength(1), reason: 'the request waiting stays the person\'s');
      expect(agent.answers[12], isNull);
      await finish(tester);
    });

    testWidgets('not when the agent says by itself it is in Bypass', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      final store = _store();
      await _started(tester, agent, store);
      agent.agentSets('bypass');
      await settle(tester);
      expect(store.selected!.bypass, isFalse);
      await _asked(tester, agent, 13);
      expect(agent.answers[13], isNull);
      expect(store.selected!.pendingPermissions, hasLength(1));
      await finish(tester);
    });

    testWidgets('and then never a question or a plan, nor a request with no allow', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.codexModes);
      final store = _store();
      await _started(tester, agent, store);
      await pick(tester, 'option-mode', 'Bypass');
      expect(store.selected!.bypass, isTrue);
      await _asked(tester, agent, 14, rawInput: {'questions': [{'question': 'Which one?'}]}, kind: 'other');
      await _asked(tester, agent, 15, kind: 'switch_mode', rawInput: {'plan': 'Do it'});
      await _asked(tester, agent, 16, options: const [{'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'}]);
      expect([for (final id in [14, 15, 16]) agent.answers[id]], [null, null, null]);
      expect(store.selected!.pendingPermissions, hasLength(3));
      await _asked(tester, agent, 17);
      expect(agent.answers[17]?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}});
      await finish(tester);
    });
  });

  group('Shift+Tab', () {
    Future<void> press(WidgetTester tester, SessionStore store) async {
      unawaited2(store.cycleMode(store.selected!));
      await settle(tester, 20);
    }

    testWidgets('leaves Canoa\'s own Bypass, into the next mode, which is sent', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.codexModes);
      final store = _store();
      await _started(tester, agent, store);
      await pick(tester, 'option-mode', 'Plan');
      await pick(tester, 'option-mode', 'Bypass');
      expect(store.selected!.bypass, isTrue);
      expect(agent.mode, 'plan');
      await press(tester, store);
      expect(store.selected!.bypass, isFalse);
      expect(agent.mode, 'manual', reason: 'from Bypass the cycle goes round to the first mode, which is a change the agent saw');
      await finish(tester);
    });

    testWidgets('and the menu, queued together, run in the order they were asked', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes)..holdChoices = true;
      final store = _store();
      await _started(tester, agent, store);
      final session = store.selected!;
      final plan = modeEntries(modeOptionIn(session.options)).firstWhere((entry) => entry.value == 'plan');
      unawaited2(store.cycleMode(session));
      unawaited2(store.chooseMode(session, plan));
      await settle(tester, 20);
      for (var i = 0; i < 3; i++) {
        agent.releaseChoices();
        await settle(tester);
      }
      expect(agent.mode, 'plan', reason: 'the menu\'s choice came after the press, and is the last word');
      await finish(tester);
    });

    testWidgets('skips a mode the agent refused, and does not ask for it again', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes)..refuseModes.add('accept_edits');
      final store = _store();
      await _started(tester, agent, store);
      await press(tester, store);
      expect(store.selected!.refusedModes, {'accept_edits'});
      expect(agent.mode, 'manual', reason: 'refused');
      await press(tester, store);
      expect(agent.mode, 'auto', reason: 'the next one, with Accept edits skipped');
      final asked = agent.requests.where((request) => request['method'] == 'session/set_config_option').length;
      await press(tester, store);
      await press(tester, store);
      await press(tester, store);
      await press(tester, store);
      final modes = [for (final request in agent.requests) if (request['method'] == 'session/set_config_option') (request['params'] as Map)['value']];
      expect(modes.skip(asked).contains('accept_edits'), isFalse);
      await finish(tester);
    });

    testWidgets('quick presses go on from the last one asked for, not from the mode still shown', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes)..holdChoices = true;
      final store = _store();
      await _started(tester, agent, store);
      final session = store.selected!;
      unawaited2(store.cycleMode(session));
      unawaited2(store.cycleMode(session));
      await settle(tester, 20);
      agent.releaseChoices();
      await settle(tester);
      agent.releaseChoices();
      await settle(tester);
      expect(agent.mode, 'auto', reason: 'manual, then accept edits, then auto');
      await finish(tester);
    });

    testWidgets('does nothing while the session is being opened again, and leaving Bypass then is still kept', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      ledger.record(
        peerId: 'local:Scripted', sessionId: 'o1', entry: 'Scripted', place: SessionPlace.local, reopenBy: 'session/load', cwd: '/w',
      );
      ledger.setMode('local:Scripted', 'o1', mode: 'plan', bypass: true);
      final agent = OptionsAgent(modes: OptionsAgent.codexModes)..holdLoad = true;
      final store = _store(ledger);
      await tester.runAsync(() async {
        final client = await agent.start(scripted);
        final opening = store.reopen(ledger.entries.single, client);
        await Future<void>.delayed(const Duration(milliseconds: 60));
        final session = store.sessions.single;
        expect(session.opening, isTrue, reason: 'reopen marks it while the agent has not loaded it');
        expect(store.canChangeMode(session), isFalse);
        await store.cycleMode(session);
        expect(agent.methods, isNot(contains('session/set_config_option')), reason: 'Shift+Tab did nothing');
        await store.leaveBypass(session);
        agent.releaseLoad();
        await opening;
        expect(session.bypass, isFalse, reason: 'leaving Bypass while it opened was not undone');
        expect(session.opening, isFalse);
        expect(ledger.entries.single.bypass, isFalse, reason: 'and the ledger keeps that');
        expect(agent.mode, 'plan', reason: 'the mode it was left in, once');
      });
    });
  });

  group('Canoa\'s own Bypass and the agent\'s mode', () {
    testWidgets('sits on it, says so, and leaving it returns to that mode with nothing sent', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.codexModes);
      final store = _store();
      await _started(tester, agent, store);
      await pick(tester, 'option-mode', 'Plan');
      final sent = agent.requests.length;
      await pick(tester, 'option-mode', 'Bypass');
      expect(agent.mode, 'plan');
      expect(find.text('Bypass over Plan'), findsOneWidget);
      await tester.tap(find.text('Leave Bypass'));
      await settle(tester);
      expect(store.selected!.bypass, isFalse);
      expect(store.modeOf(store.selected!), 'plan');
      expect(agent.requests.length, sent, reason: "Bypass is Canoa's own: neither choosing it nor leaving it reached the agent");
      await finish(tester);
    });
  });

  group('reopening a session kept in Canoa\'s own Bypass', () {
    testWidgets('keeps it Canoa\'s when the agent has a Bypass of its own by now, on the mode it sat on', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      ledger.record(
        peerId: 'local:Scripted', sessionId: 'o1', entry: 'Scripted', place: SessionPlace.local, reopenBy: 'session/load', cwd: '/w',
      );
      ledger.setMode('local:Scripted', 'o1', mode: 'plan', bypass: true);
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      final store = _store(ledger);
      await tester.runAsync(() async {
        final client = await agent.start(scripted);
        final session = await store.reopen(ledger.entries.single, client);
        expect(session.bypass, isTrue, reason: 'Canoa\'s own, not turned into the agent\'s');
        expect(agent.mode, 'plan');
        await store.sendText('go on');
        expect(agent.mode, 'plan', reason: 'a later send does not turn it into the agent\'s either');
      });
    });
  });

  group('the ledger', () {
    testWidgets('keeps a change the agent makes by itself, and not its starting default', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      final store = _store(ledger);
      await _started(tester, agent, store);
      expect(ledger.entries.single.mode, isNull, reason: 'the agent\'s default is not the person\'s choice');
      agent.agentSets('plan');
      await settle(tester);
      expect(ledger.entries.single.mode, 'plan');
      await finish(tester);
    });
  });

  group('the ledger, with something else chosen first', () {
    testWidgets('does not keep the agent\'s starting mode because a model was applied', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend())
        ..rememberSource('Scripted', options: OptionsAgentOptions.parsed, images: true, files: true);
      final agent = OptionsAgent();
      final store = _store(ledger);
      await pumpApp(tester, store, startAgent: agent.start);
      await newSession(tester);
      await chooseSource(tester, 'Scripted');
      await openFolder(tester);
      await pick(tester, 'option-model', 'Opus');
      await send(tester, 'Say hello');
      expect(agent.model, 'opus', reason: 'the pending choice was applied');
      expect(ledger.entries.single.mode, isNull, reason: 'its starting mode was nobody\'s choice');
      await finish(tester);
    });
  });

  group('a draft', () {
    test('that leaves Bypass is kept without it', () async {
      final backend = MemoryDraftBackend();
      final keeper = DraftKeeper(backend);
      final store = SessionStore(includePreview: false, drafts: keeper);
      final draft = store.createDraft(bypass: true);
      await keeper.flush();
      expect((await keeper.load()).single.record['bypass'], isTrue);
      await store.leaveBypass(draft);
      await keeper.flush();
      expect((await keeper.load()).single.record['bypass'], isNot(true));
    });
  });
}

/// A press whose end the test does not wait for.
void unawaited2(Future<void> future) {}

