// Loadout in the app: a new session starts with the overlays Loadout has on and tells its host which, the choice is kept with the
// session, and turning a default off reaches new sessions only. The agent is the test's, reached over a link as the host's own is.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/profile/report.dart' show overlaysMeta;
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

const overlaysText = '[overlays]\na = "https://example.invalid/a.git@main"\nb = "https://example.invalid/b.git@main"\n';

SessionStore _store() => SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));

String _pill(WidgetTester tester, String key) => find
    .descendant(of: find.byKey(Key(key)), matching: find.byType(Text))
    .evaluate()
    .map((element) => (element.widget as Text).data)
    .join();

Future<void> _startOnClaude(WidgetTester tester, OptionsAgent agent, int initializes) async {
  await newSession(tester);
  await chooseSource(tester, 'Claude');
  await openFolder(tester);
  await send(tester, 'hello');
  await until(tester, () => agent.requests.where((request) => request['method'] == 'initialize').length >= initializes, 'initialize');
}

Object? _overlaysAsked(OptionsAgent agent, int nth) =>
    (agent.requests.where((request) => request['method'] == 'initialize').toList()[nth]['params'] as Map)['_meta'][overlaysMeta];

void main() {
  testWidgets('a new session starts with the overlays that are on, tells the host which, and keeps them', (tester) async {
    final agent = OptionsAgent();
    final store = _store();
    await pumpApp(tester, store, linked: agent.openLink, config: CanoaConfig.parse('$overlaysText[loadout]\noff = ["b"]\n'));
    await newSession(tester);
    await chooseSource(tester, 'Claude');
    await openFolder(tester);
    expect(_pill(tester, 'overlays'), 'Overlays (1)', reason: 'b is off by default');
    await send(tester, 'hello');
    await until(tester, () => agent.requests.any((request) => request['method'] == 'initialize'), 'initialize');
    expect(_overlaysAsked(agent, 0), ['a']);
    expect(store.selected!.overlays, {'a'}, reason: 'what it started with is kept with the session');
    await finish(tester);
  });

  testWidgets('a person\'s choice in the composer wins over the defaults', (tester) async {
    final agent = OptionsAgent();
    final store = _store();
    await pumpApp(tester, store, linked: agent.openLink, config: CanoaConfig.parse('$overlaysText[loadout]\noff = ["b"]\n'));
    await newSession(tester);
    await chooseSource(tester, 'Claude');
    await openFolder(tester);
    await tester.tap(find.byKey(const Key('overlays')));
    await frames(tester);
    await tester.tap(find.byKey(const Key('overlay b')).last);
    await frames(tester);
    await tester.tap(find.byKey(const Key('overlay a')).last);
    await frames(tester);
    await tester.tapAt(const Offset(5, 5));
    await frames(tester);
    await send(tester, 'hello');
    await until(tester, () => agent.requests.any((request) => request['method'] == 'initialize'), 'initialize');
    expect(_overlaysAsked(agent, 0), ['b']);
    await finish(tester);
  });

  testWidgets('turning a default off in Loadout removes it from new sessions, and the session already started keeps it', (tester) async {
    final agent = OptionsAgent();
    final store = _store();
    final written = await pumpApp(tester, store, linked: agent.openLink, config: CanoaConfig.parse(overlaysText));
    await _startOnClaude(tester, agent, 1);
    expect(_overlaysAsked(agent, 0), ['a', 'b']);
    final first = store.selected!;
    expect(first.overlays, {'a', 'b'});

    await tester.tap(find.text('Loadout').first);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('overlay-default-b')));
    await tester.pumpAndSettle();
    expect(written.last.loadoutOff, {'b'});
    expect(written.last.overlays.keys, ['a', 'b'], reason: 'it stays installed');
    await tester.pageBack();
    await tester.pumpAndSettle();

    expect(first.overlays, {'a', 'b'}, reason: 'the session already started is not changed');
    expect(_pill(tester, 'overlays'), 'Overlays (2)', reason: 'its composer shows what it was given');
    await _startOnClaude(tester, agent, 2);
    expect(_overlaysAsked(agent, 1), ['a'], reason: 'a new session starts without it');
    expect(store.selected!.overlays, {'a'});
    expect(first.overlays, {'a', 'b'});
    await finish(tester);
  });

  testWidgets('a session reopened asks for the overlays it started with, not the defaults of the day', (tester) async {
    final agent = OptionsAgent();
    final ledger = SessionLedger(MemoryLedgerBackend());
    final store = SessionStore(ledger: ledger);
    await pumpApp(tester, store, linked: agent.openLink, config: CanoaConfig.parse(overlaysText));
    await _startOnClaude(tester, agent, 1);
    final entry = ledger.entries.single;
    expect(entry.overlays, ['a', 'b'], reason: 'the ledger keeps them with the session');
    expect(LedgerEntry.fromJson(entry.toJson())!.overlays, ['a', 'b']);
    await finish(tester);
  });
}
