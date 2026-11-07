// What a mode is when two parties hold it: Canoa answers requests only under its own Bypass; Shift+Tab goes on from where the
// last press went and skips what the agent refused; Canoa's Bypass sits on the agent's mode; the ledger keeps what the person
// or the agent set and not the agent's starting default.
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

    testWidgets('leaves Canoa\'s own Bypass', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.codexModes);
      final store = _store();
      await _started(tester, agent, store);
      await pick(tester, 'option-mode', 'Bypass');
      expect(store.selected!.bypass, isTrue);
      await press(tester, store);
      expect(store.selected!.bypass, isFalse);
      expect(agent.mode, 'manual');
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

    testWidgets('does nothing while the session is being opened again', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      final store = _store();
      await _started(tester, agent, store);
      final session = store.selected!..opening = true;
      expect(store.canChangeMode(session), isFalse);
      await store.cycleMode(session);
      expect(agent.mode, 'manual');
      session.opening = false;
      await finish(tester);
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
