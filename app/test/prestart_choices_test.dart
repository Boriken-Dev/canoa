// The models and efforts of a source are asked once it is chosen, before any session starts: the composer says it is listing them
// while the host answers, then offers them; and a source that has accounts is asked as the account chosen, again when the choice
// changes.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/chat/session_options.dart' show OptionRole, PendingChoice;
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/source_choices.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/source_choices.dart';

SessionStore _store() => SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));

String _pill(WidgetTester tester, String key) => [
  for (final element in find.descendant(of: find.byKey(Key(key)), matching: find.byType(Text)).evaluate()) (element.widget as Text).data,
].join();

void main() {
  const claude = AgentSource.hosted('Claude', harness: 'claude', local: true);

  testWidgets('the composer says it is listing the models while the host answers, then offers them', (tester) async {
    final answer = Completer<SourceChoices?>();
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    await pumpApp(tester, _store(), linked: agent.openLink, askChoices: (source) => source.label == 'Claude' ? answer.future : Future.value(fixtureChoices(source.label)));
    await newSession(tester);
    await chooseSource(tester, 'Claude');
    await openFolder(tester);
    await tester.pump();
    expect(_pill(tester, 'option-model'), 'Listing models…');
    expect(find.text('Model: after start'), findsNothing);
    answer.complete(fixtureChoices('Claude'));
    await tester.pump(const Duration(milliseconds: 50));
    await tester.pump(const Duration(milliseconds: 50));
    expect(_pill(tester, 'option-model'), isNot(anyOf('Listing models…', 'Model: after start')));
    expect(agent.starts, isEmpty, reason: 'nothing started to list them');
    await finish(tester);
  });

  test('a draft is asked as the account chosen, and asked again when the choice changes', () async {
    final asked = <String?>[];
    final store = SessionStore()
      ..choicesLoader = ((source) async {
        asked.add(null);
        return fixtureChoices(source.label);
      })
      ..accountChoicesLoader = ((source, account) async {
        asked.add(account);
        return fixtureChoices(source.label);
      })
      ..defaultAccountOf = (source) => 'first';
    final draft = store.createDraft(source: claude);
    await Future<void>.delayed(const Duration(milliseconds: 20));
    expect(asked, ['first'], reason: 'the first usable account is the one a session would start on');
    store.setAccount(draft, 'second');
    await Future<void>.delayed(const Duration(milliseconds: 20));
    expect(asked, ['first', 'second']);
    store.setAccount(draft, 'second');
    await Future<void>.delayed(const Duration(milliseconds: 20));
    expect(asked, ['first', 'second'], reason: 'the same choice asks nothing more');
    expect(store.choicesOf(claude), isNotNull);
  });

  test('a model chosen on one account is taken back when the account chosen does not list it', () async {
    final big = SourceChoices.parse(jsonEncode(claudeAnswer));
    final small = SourceChoices.parse(jsonEncode({
      'known': true,
      'models': [
        {'id': 'haiku', 'name': 'Haiku', 'efforts': <String>[]},
      ],
    }));
    final store = SessionStore()
      ..choicesLoader = ((source) async => big)
      ..accountChoicesLoader = ((source, account) async => account == 'small' ? small : big)
      ..defaultAccountOf = (source) => 'big';
    final draft = store.createDraft(source: claude);
    await Future<void>.delayed(const Duration(milliseconds: 20));
    draft.chat.holdChoice(const PendingChoice('model', OptionRole.model, 'opus', byPerson: true));
    store.setAccount(draft, 'small');
    await Future<void>.delayed(const Duration(milliseconds: 20));
    expect(draft.pending.values.where((choice) => choice.role == OptionRole.model), isEmpty, reason: 'the small account lists no opus');
    draft.chat.holdChoice(const PendingChoice('model', OptionRole.model, 'haiku', byPerson: true));
    store.setAccount(draft, 'big');
    await Future<void>.delayed(const Duration(milliseconds: 20));
    expect(draft.pending.values.where((choice) => choice.role == OptionRole.model).map((choice) => choice.value), ['haiku'], reason: 'the big account lists it too');
  });

  test('a host that cannot ask as an account is asked as the machine\'s own login', () async {
    final asked = <String?>[];
    final store = SessionStore()
      ..choicesLoader = ((source) async {
        asked.add(null);
        return fixtureChoices(source.label);
      })
      ..accountChoicesLoader = ((source, account) async => null)
      ..defaultAccountOf = (source) => 'first';
    store.createDraft(source: claude);
    await Future<void>.delayed(const Duration(milliseconds: 20));
    expect(asked, [null]);
    expect(store.choicesOf(claude)!.known, isTrue);
  });
}
