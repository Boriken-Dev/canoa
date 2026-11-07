// The mode beside the composer's Send, bottom left: each harness's own modes in one control, changed by its menu and by
// Shift+Tab with nothing asked, Bypass shown as a status, the session's mode kept across reopen and restart, and Settings'
// default for a new session.
import 'package:canoa/chat/agent_events.dart' show Choice, ConfigOption, parseConfigOptions;
import 'package:canoa/chat/modes.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/settings_dialog.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

SessionStore _store(SessionLedger ledger) => SessionStore(includePreview: false, ledger: ledger);

Future<void> _draft(WidgetTester tester) async {
  await newSession(tester);
  await chooseSource(tester, 'Scripted');
  await openFolder(tester);
}

Future<void> _shiftTab(WidgetTester tester) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.tab);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
  await settle(tester);
}

void main() {
  group('the control', () {
    testWidgets('sits at the composer\'s bottom left, before the add button', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      await pumpApp(tester, _store(SessionLedger(MemoryLedgerBackend())), startAgent: agent.start);
      await _draft(tester);
      await send(tester, 'Say hello');
      final mode = tester.getTopLeft(find.byKey(const Key('option-mode')));
      final plus = tester.getTopLeft(find.byKey(const Key('composer-plus')));
      final box = tester.getRect(find.byKey(const Key('composer')));
      expect(mode.dx, lessThan(plus.dx));
      expect(mode.dy, greaterThan(box.center.dy), reason: 'under the message, not over it');
      await finish(tester);
    });

    testWidgets('shows Claude\'s five modes and Codex\'s four with Canoa\'s own Bypass beside them', (tester) async {
      for (final (modes, native) in [(OptionsAgent.claudeModes, true), (OptionsAgent.codexModes, false)]) {
        final agent = OptionsAgent(modes: modes);
        final store = _store(SessionLedger(MemoryLedgerBackend()));
        await pumpApp(tester, store, startAgent: agent.start);
        await _draft(tester);
        await send(tester, 'Say hello');
        expect(await menuOf(tester, 'option-mode'), ['Manual', 'Accept edits', 'Auto', 'Plan', 'Bypass']);
        await pick(tester, 'option-mode', 'Bypass');
        expect(store.selected?.bypass, !native, reason: native ? 'the agent has its own' : 'Canoa provides it');
        expect(agent.mode, native ? 'bypass' : 'manual');
        await finish(tester);
        await tester.pumpWidget(const SizedBox());
      }
    });

    testWidgets('an agent with no mode of its own still offers Bypass', (tester) async {
      final agent = OptionsAgent(modes: const []);
      await pumpApp(tester, _store(SessionLedger(MemoryLedgerBackend())), startAgent: agent.start);
      await _draft(tester);
      await send(tester, 'Say hello');
      expect(await menuOf(tester, 'option-mode'), ['Bypass']);
      await finish(tester);
    });
  });

  group('the row', () {
    testWidgets('stays on one line at desktop width, a new session too: mode, Add, then model and effort by Send', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      await pumpApp(tester, _store(SessionLedger(MemoryLedgerBackend())), startAgent: agent.start, size: const Size(1280, 800));
      await _draft(tester);
      Offset centre(String key) => tester.getCenter(find.byKey(Key(key)));
      void inOneRow(List<Offset> points) {
        for (final point in points) {
          expect((point.dy - points.first.dy).abs(), lessThan(4), reason: 'one line: $points');
        }
      }
      // Before the session starts the model and effort show once the source has been seen; Add and the mode are always there.
      inOneRow([centre('option-mode'), centre('composer-plus'), tester.getCenter(find.byTooltip('Send'))]);
      expect(centre('option-mode').dx, lessThan(centre('composer-plus').dx));
      await send(tester, 'Say hello');
      inOneRow([centre('option-mode'), centre('composer-plus'), centre('option-model'), centre('option-effort'), tester.getCenter(find.byTooltip('Send'))]);
      expect(centre('composer-plus').dx, lessThan(centre('option-model').dx));
      expect(centre('option-effort').dx, lessThan(tester.getCenter(find.byTooltip('Send')).dx));
      expect(centre('option-model').dx, greaterThan(tester.getCenter(find.byKey(const Key('composer'))).dx), reason: 'on the right');
      await finish(tester);
    });

    testWidgets('may wrap at phone width, with Send still at the end', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      await pumpApp(tester, _store(SessionLedger(MemoryLedgerBackend())), startAgent: agent.start, size: const Size(390, 844));
      await _draft(tester);
      await send(tester, 'Say hello');
      expect(tester.takeException(), isNull);
      expect(find.byKey(const Key('option-mode')), findsOneWidget);
      await finish(tester);
    });
  });

  group('changing it', () {
    testWidgets('by the menu reaches the agent, and by Shift+Tab goes on through the modes and round', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      final store = _store(SessionLedger(MemoryLedgerBackend()));
      await pumpApp(tester, store, startAgent: agent.start);
      await _draft(tester);
      await send(tester, 'Say hello');
      await pick(tester, 'option-mode', 'Plan');
      expect(agent.mode, 'plan');
      await tester.tap(find.byType(TextField).last);
      await settle(tester);
      await _shiftTab(tester);
      expect(agent.mode, 'bypass', reason: 'the translators\' order is manual, accept edits, auto, plan, bypass');
      await _shiftTab(tester);
      expect(agent.mode, 'manual');
      await _shiftTab(tester);
      expect(agent.mode, 'accept_edits');
      expect(find.byType(AlertDialog), findsNothing);
      await finish(tester);
    });

    testWidgets('to Bypass asks nothing and shows as a status, never in the colour of an error', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      final store = _store(SessionLedger(MemoryLedgerBackend()));
      await pumpApp(tester, store, startAgent: agent.start);
      await _draft(tester);
      await send(tester, 'Say hello');
      await tester.tap(find.byKey(const Key('option-mode')));
      await frames(tester);
      await tester.tap(find.text('Bypass').last);
      await frames(tester);
      expect(find.byType(AlertDialog), findsNothing, reason: 'nothing is asked');
      expect(agent.mode, 'bypass');
      expect(find.byKey(const Key('bypass-banner')), findsOneWidget);
      final scheme = Theme.of(tester.element(find.byKey(const Key('option-mode')))).colorScheme;
      final label = tester.widget<Text>(find.descendant(of: find.byKey(const Key('option-mode')), matching: find.text('Bypass')));
      expect(label.style?.color, scheme.tertiary);
      expect(label.style?.color, isNot(scheme.error));
      final banner = tester.widget<Material>(find.byKey(const Key('bypass-banner')));
      expect(banner.color, scheme.tertiaryContainer);
      // Leaving it is one action, into the mode that asks the most.
      await tester.tap(find.text('Leave Bypass'));
      await settle(tester);
      expect(agent.mode, 'manual');
      expect(find.byKey(const Key('bypass-banner')), findsNothing);
      await finish(tester);
    });

    test('Shift+Tab\'s order is the translators\' where the ids are theirs, and the agent\'s own otherwise', () {
      final claude = modeEntries(parseConfigOptions(OptionsAgent(modes: OptionsAgent.claudeModes).options)!.first)..shuffle();
      var at = nextMode(claude, 'manual')!;
      expect(at.value, 'accept_edits');
      expect(nextMode(claude, 'bypass')!.value, 'manual');
      at = nextMode(modeEntries(null), null)!;
      expect((at.value, at.own), ('bypass', true));
      final own = modeEntries(const ConfigOption(id: 'mode', label: 'Mode', current: 'b', choices: [Choice('b', 'B'), Choice('a', 'A')]));
      expect([for (final entry in own) entry.value], ['b', 'a', 'bypass']);
      expect(nextMode(own, 'b')!.value, 'a');
    });
  });

  group('kept', () {
    testWidgets('in the ledger across reopen and restart, the agent put back in it before anything is sent', (tester) async {
      final backend = MemoryLedgerBackend();
      var ledger = SessionLedger(backend);
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      final store = _store(ledger);
      await pumpApp(tester, store, startAgent: agent.start);
      await _draft(tester);
      await send(tester, 'Say hello');
      await pick(tester, 'option-mode', 'Plan');
      expect(ledger.entries.single.mode, 'plan');
      expect(ledger.entries.single.bypass, isFalse);
      await finish(tester);
      await tester.runAsync(ledger.flush);
      await tester.pumpWidget(const SizedBox());

      // The app starts again on the same file: a new store, a new agent that starts in its own default.
      ledger = SessionLedger(backend);
      await tester.runAsync(ledger.load);
      final entry = ledger.entries.single;
      expect(entry.mode, 'plan');
      final second = OptionsAgent(modes: OptionsAgent.claudeModes);
      final reopened = _store(ledger);
      final client = await tester.runAsync(() => second.start(scripted, decider: reopened.askPermission)) as dynamic;
      final session = await tester.runAsync(() => reopened.reopen(entry, client)) as ChatSession;
      expect(second.mode, 'plan');
      expect(reopened.modeOf(session), 'plan');
      // Nothing reaches the agent before its mode does: the first prompt comes after the choice.
      await tester.runAsync(() => reopened.sendText('go on'));
      expect(second.methods, containsAllInOrder(['session/load', 'session/set_config_option', 'session/prompt']));
      expect(second.methods.where((method) => method == 'session/set_config_option'), hasLength(1), reason: 'applied once');
    });

    testWidgets('Canoa\'s own Bypass is kept too, and a reopened session is in it', (tester) async {
      final backend = MemoryLedgerBackend();
      var ledger = SessionLedger(backend);
      final agent = OptionsAgent(modes: OptionsAgent.codexModes);
      final store = _store(ledger);
      await pumpApp(tester, store, startAgent: agent.start);
      await _draft(tester);
      await send(tester, 'Say hello');
      await pick(tester, 'option-mode', 'Plan');
      await pick(tester, 'option-mode', 'Bypass');
      // Bypass is Canoa's and sits on Plan, which the ledger keeps beside it.
      expect((ledger.entries.single.bypass, ledger.entries.single.mode), (true, 'plan'));
      expect(agent.mode, 'plan');
      await finish(tester);
      await tester.runAsync(ledger.flush);
      await tester.pumpWidget(const SizedBox());

      ledger = SessionLedger(backend);
      await tester.runAsync(ledger.load);
      final second = OptionsAgent(modes: OptionsAgent.codexModes);
      final reopened = _store(ledger);
      final client = await tester.runAsync(() => second.start(scripted, decider: reopened.askPermission)) as dynamic;
      final session = await tester.runAsync(() => reopened.reopen(ledger.entries.single, client)) as ChatSession;
      expect(session.bypass, isTrue);
      expect(second.mode, 'plan', reason: 'the agent is put back in the mode Bypass sat on');
      // It still answers: a request that offers an allow is answered with no card, after the reopen.
      second.ask(7, [
        {'optionId': 'allow-once', 'name': 'Yes', 'kind': 'allow_once'},
        {'optionId': 'reject', 'name': 'No', 'kind': 'reject_once'},
      ]);
      await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 60)));
      expect(second.answers[7]?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}});
      expect(session.pendingPermissions, isEmpty);
    });
  });

  group('the default for a new session', () {
    testWidgets('is Settings\' mode, put in the new session before it starts', (tester) async {
      final ledger = SessionLedger(MemoryLedgerBackend())
        ..rememberSource('Scripted', options: parseConfigOptions(OptionsAgent(modes: OptionsAgent.claudeModes).options)!, images: true, files: true);
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      final store = _store(ledger)..defaultMode = 'plan';
      await pumpApp(tester, store, startAgent: agent.start);
      await _draft(tester);
      await settle(tester);
      expect(store.modeOf(store.selected!), 'plan');
      await send(tester, 'Say hello');
      expect(agent.mode, 'plan');
      await finish(tester);
    });

    testWidgets('Bypass as the default puts a session with an agent that has none in Canoa\'s own', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.codexModes);
      final store = _store(SessionLedger(MemoryLedgerBackend()))..defaultMode = 'bypass';
      await pumpApp(tester, store, startAgent: agent.start);
      await _draft(tester);
      await settle(tester);
      expect(store.selected?.bypass, isTrue);
      await finish(tester);
    });

    testWidgets('Settings holds it as one setting, asks nothing, and has no Full access of its own', (tester) async {
      await tester.binding.setSurfaceSize(const Size(900, 1400));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      CanoaConfig? saved;
      await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => TextButton(
        onPressed: () async => saved = await showDialog<CanoaConfig>(context: context, builder: (_) => const SettingsDialog(config: CanoaConfig())),
        child: const Text('open'),
      ))));
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
      expect(find.text('Full access'), findsNothing);
      expect(find.byKey(const Key('settings-full-access')), findsNothing);
      await tester.tap(find.byKey(const Key('settings-default-mode')).last);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Bypass').last);
      await tester.pumpAndSettle();
      expect(find.byType(AlertDialog), findsOneWidget, reason: 'only Settings itself');
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      expect(saved?.defaultMode, 'bypass');
      expect(CanoaConfig.parse(saved!.encode()).defaultMode, 'bypass');
    });

    test('canoa.cfg reads, checks and writes it back, and reads an older full_access list as nothing', () {
      expect(const CanoaConfig().defaultMode, isNull);
      final config = CanoaConfig.parse('[permissions]\ndefault_mode = "accept_edits"\n');
      expect(config.defaultMode, 'accept_edits');
      expect(CanoaConfig.parse(config.encode()).defaultMode, 'accept_edits');
      expect(() => CanoaConfig.parse('[permissions]\ndefault_mode = "yolo"\n'), throwsFormatException);
      expect(() => CanoaConfig.parse('[permissions]\ndefault_mode = 3\n'), throwsFormatException);
      final older = CanoaConfig.parse('[permissions]\nfull_access = ["Claude"]\n');
      expect(older.defaultMode, isNull);
      expect(older.encode(), isNot(contains('full_access')));
    });
  });
}

