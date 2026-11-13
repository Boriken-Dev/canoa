// The start of a session on its first message, at its edges: a second Send while the first is still being delivered, leaving
// and coming back while it starts, a message the agent cannot be given, the folder changed after choosing Worktree, quitting
// or closing while it starts, a draft whose source is gone, and what is remembered for a source.
import 'dart:async';
import 'dart:io' show Directory;

import 'package:canoa/chat/session_options.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/desktop_shell.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/drafts/draft_store.dart';
import 'package:canoa/desktop/host_tool.dart' show HostLink;
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/source_icons.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart' show LogicalKeyboardKey;
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_host.dart';
import 'fixtures/options_agent.dart';

const _source = AgentSource.hosted('Scripted', local: true);

/// The line `canoa serve` prints for an agent entry it serves.
String _serving(String name) =>
    '{"name":"$name","kind":"agent","url":"ws://127.0.0.1:7801","peer":"sha256:aa","method":"noise"}';

int _prompts(OptionsAgent agent) => agent.methods.where((method) => method == 'session/prompt').length;

Finder get _composer => find.descendant(of: find.byKey(const Key('composer')), matching: find.byType(TextField));

String _written(WidgetTester tester) => tester.widget<TextField>(_composer).controller!.text;

/// A store whose source has run before, so its model and effort are offered before it starts.
SessionStore _remembering() => SessionStore(
  ledger: SessionLedger(MemoryLedgerBackend())
    ..rememberSource('Scripted', options: OptionsAgentOptions.parsed, images: true, files: true),
);

final class _Shell implements DesktopShell {
  @override
  Future<void> Function()? beforeQuit;
  @override
  bool get isDesktop => true;
  @override
  Future<void> showWaiting(int count) async {}
  @override
  Future<void> update(CanoaConfig config) async {}
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

/// A start that waits until the test lets it go on, as a slow adapter's does.
final class _HeldStart {
  _HeldStart(this.agent);
  final OptionsAgent agent;
  final _go = Completer<void>();
  bool get waiting => !_go.isCompleted;
  void go() => _go.complete();

  /// The agent's link, whose first message waits until [go].
  HostLink open(String name) => _HeldLink(agent.openLink(name), _go.future);
}

final class _HeldLink implements HostLink {
  _HeldLink(this._inner, this._go);

  final HostLink _inner;
  final Future<void> _go;

  @override
  Stream<String> get messages => _inner.messages;

  @override
  Future<void> send(String message) async {
    await _go;
    return _inner.send(message);
  }

  @override
  Future<void> close() => _inner.close();
}

void main() {
  testWidgets('a second Send while the first message is still being delivered sends nothing', (tester) async {
    final agent = OptionsAgent()..holdChoices = true;
    final store = _remembering();
    await pumpApp(tester, store, linked: agent.openLink);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await pick(tester, 'option-model', 'Opus');
    await send(tester, 'First');
    await until(tester, () => agent.methods.contains('session/set_config_option'), 'the model to be asked for');
    final session = store.selected!;
    expect(session.remoteSessionId, isNotNull, reason: 'started and bound, its first message not yet sent');
    final second = await tester.runAsync(() => store.sendText('Second').then<Object?>((_) => null, onError: (Object error) => error));
    expect(second, isA<StateError>(), reason: 'refused while the first is on its way');
    agent.releaseChoices();
    await until(tester, () => _prompts(agent) == 1, 'the first message');
    await settle(tester);
    expect(_prompts(agent), 1);
    expect(agent.last('session/prompt')['params']['prompt'], [{'type': 'text', 'text': 'First'}]);
    expect(agent.model, 'opus');
    await finish(tester);
  });

  testWidgets('leaving while it starts and coming back finds the composer emptied once the message is sent', (tester) async {
    final held = _HeldStart(OptionsAgent());
    final agent = held.agent;
    final store = _remembering();
    await pumpApp(tester, store, linked: held.open);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    final first = store.selected!;
    await send(tester, 'First');
    expect(first.status, SessionStatus.connecting);
    store.createDraft(source: _source, cwd: '/w');
    await frames(tester);
    store.select(first.id);
    await frames(tester);
    expect(_written(tester), 'First', reason: 'still on its way, so still shown');
    held.go();
    await until(tester, () => _prompts(agent) == 1, 'the first message');
    await settle(tester);
    expect(_written(tester), isEmpty, reason: 'the message went, so it is not there to send again');
    await finish(tester);
  });

  testWidgets('a message the agent cannot be given stays in the composer, with what goes with it', (tester) async {
    final agent = OptionsAgent();
    final store = SessionStore();
    await pumpApp(tester, store, linked: agent.openLink);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await send(tester, 'Say hello');
    final session = store.selected!;
    expect(store.addFile(session, 'gone.txt', '${Directory.systemTemp.path}/canoa-no-such-file.txt', 5), isNull);
    await tester.pump();
    await send(tester, 'Read it');
    await until(tester, () => find.textContaining('Message not accepted').evaluate().isNotEmpty, 'the refusal');
    expect(_written(tester), 'Read it');
    expect(session.attachments.map((item) => item.name), ['gone.txt']);
    expect(_prompts(agent), 1);
    await finish(tester);
  });

  testWidgets('a first message the started agent cannot be given says so, not that nothing started, and stays', (tester) async {
    final agent = OptionsAgent();
    final store = SessionStore();
    await pumpApp(tester, store, linked: agent.openLink);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    final session = store.selected!;
    expect(store.addFile(session, 'gone.txt', '${Directory.systemTemp.path}/canoa-no-such-file.txt', 5), isNull);
    await send(tester, 'Read it');
    await until(tester, () => find.textContaining('Message not accepted').evaluate().isNotEmpty, 'the refusal');
    expect(find.textContaining('Not started'), findsNothing);
    expect(session.remoteSessionId, isNotNull);
    expect(_written(tester), 'Read it');
    await finish(tester);
  });

  test('a queued message that cannot be sent stays in the queue, held', () async {
    final agent = OptionsAgent()..holdTurns = true;
    final store = SessionStore();
    store.starter = (draft) async {
      final client = await agent.start(scripted);
      store.bindDraft(draft, client, await client.openSession(cwd: '/w'), cwd: '/w');
    };
    final session = store.createDraft(source: _source, cwd: '/w');
    // A v1 prompt answers when its turn ends, which the agent holds.
    unawaited(store.sendText('First'));
    for (var i = 0; i < 50 && session.status != SessionStatus.working; i++) {
      await Future<void>.delayed(const Duration(milliseconds: 20));
    }
    expect(session.status, SessionStatus.working);
    store.addFile(session, 'gone.txt', '${Directory.systemTemp.path}/canoa-no-such-file.txt', 5);
    await store.sendText('Second');
    expect(session.queue.single.text, 'Second');
    agent.finishTurn();
    for (var i = 0; i < 50 && session.queueHeld == null; i++) {
      await Future<void>.delayed(const Duration(milliseconds: 20));
    }
    expect(session.queue.map((message) => message.text), ['Second']);
    expect(session.queue.single.attachments.map((item) => item.name), ['gone.txt']);
    expect(session.queueHeld, isNotNull);
  });

  testWidgets('Worktree chosen for a repository, then a folder outside any, makes no worktree', (tester) async {
    final agent = OptionsAgent();
    final worktrees = CountedWorktrees();
    final folders = ['/work/repo', '/work/plain'];
    final store = SessionStore();
    await pumpApp(tester, store, linked: agent.openLink, worktrees: worktrees, pickFolder: () async => folders.removeAt(0));
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await openFolder(tester);
    await tester.tap(find.text('Worktree'));
    await frames(tester);
    await openFolder(tester);
    await send(tester, 'Here');
    await until(tester, () => _prompts(agent) == 1, 'the message');
    expect(worktrees.made, isEmpty, reason: 'it shows Local, and Local is what starts');
    expect(store.selected!.worktree, isNull);
    await finish(tester);
  });

  testWidgets('quitting while a session starts takes back the worktree made for it', (tester) async {
    final held = _HeldStart(OptionsAgent());
    final worktrees = CountedWorktrees();
    final shell = _Shell();
    final store = SessionStore();
    await pumpApp(tester, store, linked: held.open, worktrees: worktrees, shell: shell);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await openFolder(tester);
    await tester.tap(find.text('Worktree'));
    await frames(tester);
    await send(tester, 'Slow');
    await until(tester, () => worktrees.made.isNotEmpty, 'the worktree');
    await tester.runAsync(() => shell.beforeQuit!());
    expect(worktrees.removed.map((worktree) => worktree.branch), [worktrees.made.single.$2]);
    held.go();
    await settle(tester);
    await finish(tester);
  });

  testWidgets('closing a session while it starts ends its agent and takes back its worktree', (tester) async {
    final held = _HeldStart(OptionsAgent());
    final worktrees = CountedWorktrees();
    final keeper = DraftKeeper(MemoryDraftBackend());
    final store = SessionStore(drafts: keeper, ledger: SessionLedger(MemoryLedgerBackend()));
    await pumpApp(tester, store, linked: held.open, worktrees: worktrees);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await openFolder(tester);
    await tester.tap(find.text('Worktree'));
    await frames(tester);
    await send(tester, 'Slow');
    await until(tester, () => worktrees.made.isNotEmpty, 'the worktree');
    await tester.tap(find.byTooltip('Close conversation'));
    await frames(tester);
    held.go();
    await until(tester, () => worktrees.removed.isNotEmpty, 'the worktree to be taken back');
    await tester.runAsync(keeper.flush);
    expect(held.agent.links.single.closed, isTrue);
    expect(held.agent.methods, isNot(contains('session/prompt')));
    expect(await (keeper.backend as MemoryDraftBackend).ids(), isEmpty);
    expect(store.ledger!.entries, isEmpty);
    expect(store.sessions, isEmpty);
    await finish(tester);
  });

  testWidgets('a hosted source is given none of the profile, model included', (tester) async {
    final store = SessionStore();
    final tool = FakeTool();
    final host = HostSupervisor(tool);
    await host.apply(configPath: '/c/canoa.cfg');
    tool.children.single.out.add(_serving('hosted'));
    await pumpApp(tester, store, config: CanoaConfig.parse('[profile]\nmodel = "opus"\n'), host: host);
    await newSession(tester);
    await chooseSource(tester, 'hosted');
    await settle(tester, 200);
    expect(store.selected!.source?.local, isFalse);
    expect(store.selected!.pending, isEmpty);
    await finish(tester);
  });

  testWidgets('a hosted entry is shown by its title and its provider icon, and its name stays its identity', (tester) async {
    final store = SessionStore();
    final tool = FakeTool();
    final host = HostSupervisor(tool);
    await host.apply(configPath: '/c/canoa.cfg');
    await pumpApp(tester, store, host: host);
    await newSession(tester);
    tool.children.single.out
      ..add('{"name":"review","kind":"agent","harness":"claude","title":"Claude for review","url":"ws://127.0.0.1:7801","peer":"sha256:aa","method":"noise"}')
      ..add('{"name":"other","kind":"agent","harness":"outside_provider","url":"ws://127.0.0.1:7802","peer":"sha256:aa","method":"noise"}')
      ..add(_serving('plain'));
    await frames(tester);
    await tester.tap(find.byKey(const Key('new-session-source')));
    await frames(tester);
    IconData iconOf(String name) => tester.widget<Icon>(find.descendant(of: find.byKey(Key('source $name')), matching: find.byType(Icon))).icon!;
    expect(find.descendant(of: find.byKey(const Key('source review')), matching: find.text('Claude for review')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('source other')), matching: find.text('other')), findsOneWidget, reason: 'no title shows the name');
    expect(find.descendant(of: find.byKey(const Key('source review')), matching: find.byType(Icon)), findsNothing,
        reason: 'a Claude entry draws the vendor mark');
    expect(iconOf('other'), providerIcon(null), reason: 'an unknown provider has the generic icon');
    expect(iconOf('plain'), providerIcon(null));
    await tester.tap(find.byKey(const Key('source review')));
    await frames(tester);
    expect(store.selected!.source, const AgentSource.hosted('review'), reason: 'the name is still what is remembered');
    expect(find.descendant(of: find.byKey(const Key('new-session-source')), matching: find.text('Claude for review')), findsOneWidget);
    await finish(tester);
  });

  testWidgets('an entry the host starts serving is in the source picker, and one it stops serving is not', (tester) async {
    final store = SessionStore();
    final tool = FakeTool();
    final host = HostSupervisor(tool);
    await host.apply(configPath: '/c/canoa.cfg');
    await pumpApp(tester, store, host: host);
    await newSession(tester);

    // canoa.cfg is not read by the app: the host's line for a new entry is all it takes.
    tool.children.single.out.add(_serving('outside'));
    await frames(tester);
    await tester.tap(find.byKey(const Key('new-session-source')));
    await frames(tester);
    expect(find.byKey(const Key('source outside')), findsOneWidget);
    await tester.tap(find.byKey(const Key('source outside')));
    await frames(tester);
    expect(store.selected!.source, const AgentSource.hosted('outside'));

    await tester.runAsync(host.stop);
    await settle(tester, 100);
    await tester.tap(find.byKey(const Key('new-session-source')));
    await frames(tester);
    expect(find.byKey(const Key('source outside')), findsNothing, reason: 'the host serves nothing now');
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await frames(tester);
    await finish(tester);
  });

  test('a draft restored before the host served its source gets the source when the host does', () {
    final store = SessionStore();
    final draft = store.createDraft(keptSource: 'late');
    store.resolveKeptSources((label) => null);
    expect(draft.source, isNull);
    store.resolveKeptSources((label) => AgentSource.hosted(label));
    expect(draft.source, const AgentSource.hosted('late'));
    expect(draft.keptSource, isNull);
    expect(draft.agent, 'late');
  });

  test('a choice made before the start that the agent took is the source\'s last choice', () async {
    final ledger = SessionLedger(MemoryLedgerBackend())
      ..rememberSource('Scripted', options: OptionsAgentOptions.parsed, images: true, files: true);
    final store = SessionStore(ledger: ledger);
    final agent = OptionsAgent();
    store.starter = (draft) async {
      final client = await agent.start(scripted);
      store.bindDraft(draft, client, await client.openSession(cwd: '/w'), cwd: '/w');
    };
    final draft = store.createDraft(source: _source, cwd: '/w');
    await store.choose(draft, OptionsAgentOptions.parsed[1], 'opus');
    store.defaultChoices(draft, hints: {OptionRole.effort: 'high'});
    await store.sendText('Go');
    expect((agent.model, agent.effort), ('opus', 'high'));
    expect(ledger.source('Scripted')!.chosen, {'model': 'opus'}, reason: 'the profile\'s effort is not the person\'s choice');
  });

  test('the session list keeps no words of the first message, though the title shown is taken from it', () async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    final store = SessionStore(ledger: ledger);
    final agent = OptionsAgent();
    store.starter = (draft) async {
      final client = await agent.start(scripted);
      store.bindDraft(draft, client, await client.openSession(cwd: '/w'), cwd: '/w');
    };
    final draft = store.createDraft(source: _source, cwd: '/w');
    await store.sendText('The password is hunter2');
    expect(draft.title, 'The password is hunter2');
    expect(ledger.entries.single.title, 'Scripted');
    expect(ledger.encode(), isNot(contains('hunter2')));
  });

  test('a draft whose source is gone keeps it, for when it is back', () async {
    final backend = MemoryDraftBackend();
    final before = SessionStore(drafts: DraftKeeper(backend));
    before.createDraft(source: _source, cwd: '/w');
    await before.drafts!.flush();

    final without = SessionStore(drafts: DraftKeeper(backend));
    await without.restoreDrafts((label) => null);
    final orphan = without.sessions.single;
    expect(orphan.source, isNull);
    without.keepDraftText(orphan, 'still here');
    await without.drafts!.flush();

    final again = SessionStore(drafts: DraftKeeper(backend));
    await again.restoreDrafts((label) => label == 'Scripted' ? _source : null);
    expect((again.sessions.single.source, again.sessions.single.draftText), (_source, 'still here'));
  });
}
