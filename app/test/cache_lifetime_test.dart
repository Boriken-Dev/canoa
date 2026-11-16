// How long a new session's prompt cache lives: the chat asks long unless the person or the agent's entry in canoa.cfg says
// otherwise, the new-session view lets the person choose for one session and shows what the provider says it does with the
// choice (so "ignored" is visible), and the choice is kept with the draft and the session and reaches the agent that starts.
import 'dart:convert';

import 'package:canoa/chat/cache_lifetime.dart';
import 'package:canoa/drafts/draft_store.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/source_choices.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/source_choices.dart';

const _claudeSays =
    'sets CLAUDE_CODE_PROMPT_CACHE_TTL and CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL to 5m for short and 1h for long, and leaves both as the environment has them for default';
const _claudeChoices = [
  {'id': 'off', 'label': 'Off'},
  {'id': 'short', 'label': '5 minutes'},
  {'id': 'long', 'label': '1 hour'},
];
const _codexSays = "ignored: Codex's app server has no setting for how long its prompt cache lives (Codex 0.166.0)";

SourceChoices? _answer(String label, {String? setting}) {
  final base = switch (label) {
    'Claude' => {...claudeAnswer, 'cache_lifetime': _claudeSays, 'cache_choices': _claudeChoices},
    'Codex' => {...codexAnswer, 'cache_lifetime': _codexSays},
    _ => null,
  };
  if (base == null) return null;
  return SourceChoices.parse(jsonEncode({...base, 'cache_lifetime_setting': ?setting}));
}

Future<void> _draft(WidgetTester tester, String source) async {
  await newSession(tester);
  await chooseSource(tester, source);
  await openFolder(tester);
}

String _pill(WidgetTester tester, String key) =>
    find.descendant(of: find.byKey(Key(key)), matching: find.byType(Text)).evaluate().map((element) => (element.widget as Text).data).join();

/// The page prints no provider sentence, no variable name and no "Prompt cache:" line.
void _quiet(WidgetTester tester) {
  expect(find.byKey(const Key('new-session-cache-note')), findsNothing);
  for (final text in ['Provider:', 'CLAUDE_CODE', 'PROMPT_CACHE', 'Prompt cache:', 'ignored:']) {
    expect(find.textContaining(text), findsNothing, reason: text);
  }
}

SessionStore _store() => SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));

/// The cache lifetime the agent was asked for on `initialize`, as the host reads it.
Object? _asked(OptionsAgent agent) {
  final initialize = agent.requests.firstWhere((request) => request['method'] == 'initialize');
  return ((initialize['params'] as Map)['_meta'] as Map)[cacheLifetimeMeta];
}

void main() {
  testWidgets('a chat session asks long, and the view shows the provider\'s sentence', (tester) async {
    await pumpApp(tester, _store(), askChoices: (source) async => _answer(source.label));
    await _draft(tester, 'Claude');
    expect(_pill(tester, 'option-cache'), 'Cache: 1 hour');
    _quiet(tester);
    await finish(tester);
  });

  testWidgets('a provider that has no control shows no control and no sentence', (tester) async {
    await pumpApp(tester, _store(), askChoices: (source) async => _answer(source.label));
    await _draft(tester, 'Codex');
    _quiet(tester);
    expect(find.byKey(const Key('option-cache')), findsNothing, reason: 'nothing to choose where the choice is ignored');
    await finish(tester);
  });

  testWidgets('a source whose provider cannot say has no cache control to set', (tester) async {
    await pumpApp(tester, _store(), askChoices: (source) async => _answer(source.label));
    await _draft(tester, 'Scripted');
    expect(find.byKey(const Key('option-cache')), findsNothing);
    _quiet(tester);
    await finish(tester);
  });

  testWidgets('the person overrides it for one session, and the agent that starts is asked for that', (tester) async {
    final agent = OptionsAgent();
    final store = _store();
    await pumpApp(tester, store, linked: agent.openLink, askChoices: (source) async => _answer(source.label));
    await _draft(tester, 'Claude');
    final menu = await menuOf(tester, 'option-cache');
    expect(menu, ['Not chosen (1 hour)', 'Agent default', 'Off', '5 minutes', '1 hour'], reason: 'the provider\'s own names');
    await pick(tester, 'option-cache', '5 minutes');
    expect(_pill(tester, 'option-cache'), 'Cache: 5 minutes');
    _quiet(tester);
    expect(store.selected!.cacheLifetime, CacheLifetime.short);
    expect(agent.starts, isEmpty, reason: 'choosing starts nothing');
    await send(tester, 'Fix the failing test');
    expect(_asked(agent), {'lifetime': 'short', 'chosen': true});
    expect(store.ledger!.entries.single.cacheLifetime, 'short', reason: 'kept with the session, asked again when it is reopened');
    await finish(tester);
  });

  testWidgets('Off reaches the agent as the control for no prompt cache', (tester) async {
    final agent = OptionsAgent();
    final store = _store();
    await pumpApp(tester, store, linked: agent.openLink, askChoices: (source) async => _answer(source.label));
    await _draft(tester, 'Claude');
    await pick(tester, 'option-cache', 'Off');
    expect(_pill(tester, 'option-cache'), 'Cache: Off');
    await send(tester, 'Fix the failing test');
    expect(_asked(agent), {'lifetime': 'off', 'chosen': true});
    await finish(tester);
  });

  testWidgets('a session nobody chose for is asked long', (tester) async {
    final agent = OptionsAgent();
    await pumpApp(tester, _store(), linked: agent.openLink, askChoices: (source) async => _answer(source.label));
    await _draft(tester, 'Claude');
    await send(tester, 'Fix the failing test');
    expect(_asked(agent), {'lifetime': 'long', 'chosen': false});
    await finish(tester);
  });

  test('a lifetime the provider does not honour shows as unavailable and the agent\'s default is asked', () async {
    const codex = AgentSource.hosted('Codex', harness: 'codex', local: true);
    final store = SessionStore()..choicesLoader = (_) async => _answer('Codex');
    final draft = store.createDraft(source: codex);
    await Future<void>.delayed(const Duration(milliseconds: 20));
    store.setCacheLifetime(draft, CacheLifetime.off);
    expect(store.cacheUnavailable(draft), CacheLifetime.off);
    expect(store.cacheLifetimeOf(draft), (lifetime: CacheLifetime.harness, chosen: false));
    expect(store.cacheLifetimeOf(store.createDraft(source: codex)), (lifetime: CacheLifetime.harness, chosen: false),
        reason: 'not even the chat\'s long, where the provider has said it ignores the choice');
    const claude = AgentSource.hosted('Claude', harness: 'claude', local: true);
    final honoured = SessionStore()..choicesLoader = (_) async => _answer('Claude');
    final asked = honoured.createDraft(source: claude);
    await Future<void>.delayed(const Duration(milliseconds: 20));
    honoured.setCacheLifetime(asked, CacheLifetime.off);
    expect(honoured.cacheUnavailable(asked), isNull);
    expect(honoured.cacheLifetimeOf(asked), (lifetime: CacheLifetime.off, chosen: true));
  });

  test('an entry sets the default until the person chooses', () async {
    const source = AgentSource.hosted('Claude', harness: 'claude', local: true);
    final bare = SessionStore();
    expect(bare.cacheLifetimeOf(bare.createDraft(source: source)), (lifetime: CacheLifetime.long, chosen: false),
        reason: 'no entry says: the chat default');
    final store = SessionStore()..choicesLoader = (_) async => _answer('Claude', setting: 'short');
    final draft = store.createDraft(source: source);
    await Future<void>.delayed(const Duration(milliseconds: 20));
    expect(store.cacheLifetimeOf(draft), (lifetime: CacheLifetime.short, chosen: false), reason: 'the entry replaces the chat default');
    store.setCacheLifetime(draft, CacheLifetime.long);
    expect(store.cacheLifetimeOf(draft), (lifetime: CacheLifetime.long, chosen: true), reason: 'the person wins');
    store.setCacheLifetime(draft, null);
    expect(store.cacheLifetimeOf(draft), (lifetime: CacheLifetime.short, chosen: false));
  });

  test('a draft keeps the choice across runs', () async {
    final backend = MemoryDraftBackend();
    final keeper = DraftKeeper(backend);
    final store = SessionStore(drafts: keeper);
    final draft = store.createDraft();
    store.setCacheLifetime(draft, CacheLifetime.short);
    await keeper.flush();
    expect((await keeper.load()).single.record['cacheLifetime'], 'short');
    final again = SessionStore(drafts: keeper);
    await again.restoreDrafts((label) => null);
    expect(again.sessions.single.cacheLifetime, CacheLifetime.short);
    again.setCacheLifetime(again.sessions.single, null);
    await keeper.flush();
    expect((await keeper.load()).single.record.containsKey('cacheLifetime'), isFalse);
  });

  test('the wire carries the lifetime and whether the person chose it', () {
    expect(cacheLifetimeRequest(CacheLifetime.long, chosen: false), {'lifetime': 'long', 'chosen': false});
    expect(cacheLifetimeRequest(CacheLifetime.short, chosen: true), {'lifetime': 'short', 'chosen': true});
    expect(CacheLifetime.named('forever'), isNull);
  });
}
