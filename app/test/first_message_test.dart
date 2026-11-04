// A session starts on its first message: nothing runs and no worktree exists before Send, the first Send starts the agent
// once and delivers the message with the chosen model and effort, a start that fails keeps the message, and a session closed
// before Send leaves nothing in the ledger or on disk. What is written in one, and what goes with it, outlives the app.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/chat/session_options.dart';
import 'package:canoa/drafts/draft_backend_io.dart';
import 'package:canoa/drafts/draft_store.dart';
import 'package:canoa/ledger/ledger_backend_io.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

const _source = AgentSource.command(scripted);

void main() {
  testWidgets('the first Send makes the worktree and starts the agent, once, with model and effort set before the message', (tester) async {
    final agent = OptionsAgent();
    final worktrees = CountedWorktrees();
    final ledger = SessionLedger(MemoryLedgerBackend())
      ..rememberSource('Scripted', options: OptionsAgentOptions.parsed, images: true, files: true);
    final store = SessionStore(includePreview: false, ledger: ledger);
    await pumpApp(tester, store, startAgent: agent.start, worktrees: worktrees);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await openFolder(tester);
    await tester.tap(find.text('Worktree'));
    await pick(tester, 'option-model', 'Opus');
    await pick(tester, 'option-effort', 'High');
    expect(agent.starts, isEmpty, reason: 'nothing before Send');
    expect(worktrees.made, isEmpty);
    expect(ledger.entries, isEmpty);

    await send(tester, 'Fix the failing test');
    final session = store.selected!;
    expect(agent.starts, hasLength(1));
    expect(worktrees.made.single.$1, '/work/repo');
    expect(worktrees.made.single.$2, startsWith('canoa/scripted-'), reason: 'named after its source, never from what was said');
    expect(session.cwd, session.worktree!.path);
    expect(agent.methods, ['initialize', 'session/new', 'session/set_config_option', 'session/set_config_option', 'session/prompt']);
    expect(agent.last('session/new')['params']['cwd'], session.worktree!.path);
    expect(agent.last('session/prompt')['params']['prompt'], [{'type': 'text', 'text': 'Fix the failing test'}]);
    expect((agent.model, agent.effort), ('opus', 'high'));
    expect(session.title, 'Fix the failing test');
    expect(ledger.entries.single.sessionId, 'o1', reason: 'kept by the ledger once started');
    expect(find.widgetWithText(TextField, 'Fix the failing test'), findsNothing, reason: 'sent, the composer is empty');
    expect(find.text('Fix the failing test'), findsWidgets);

    await send(tester, 'And again');
    expect(agent.starts, hasLength(1), reason: 'started once');
    expect(worktrees.made, hasLength(1));
    expect(agent.methods.last, 'session/prompt');
    await finish(tester);
  });

  testWidgets('a start that fails keeps the message, says why, and takes back the worktree it made', (tester) async {
    final agent = OptionsAgent(failStart: 'scripted-agent could not start: no such file');
    final worktrees = CountedWorktrees();
    final ledger = SessionLedger(MemoryLedgerBackend());
    final store = SessionStore(includePreview: false, ledger: ledger);
    await pumpApp(tester, store, startAgent: agent.start, worktrees: worktrees);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await openFolder(tester);
    await tester.tap(find.text('Worktree'));
    await tester.pumpAndSettle();
    await send(tester, 'Fix the failing test');

    final session = store.selected!;
    expect(agent.starts, hasLength(1));
    expect(session.status, SessionStatus.draft);
    expect(session.title, SessionStore.draftTitle);
    expect(find.widgetWithText(TextField, 'Fix the failing test'), findsOneWidget);
    expect(find.textContaining('Not started: scripted-agent could not start: no such file'), findsWidgets);
    expect(worktrees.removed.single.branch, worktrees.made.single.$2);
    expect(session.worktree, isNull);
    expect(ledger.entries, isEmpty);
    await finish(tester);
  });

  testWidgets('what goes with the first message, added before the agent starts, goes with it', (tester) async {
    final agent = OptionsAgent();
    final store = SessionStore(includePreview: false);
    await pumpApp(tester, store, startAgent: agent.start);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    final session = store.selected!;
    expect(store.addAttachment(session, 'shot.png', [137, 80, 78, 71]), isNull);
    expect(store.addFolder(session, 'repo', '/work/repo'), isNull);
    await tester.pumpAndSettle();
    expect(agent.starts, isEmpty);
    await send(tester, 'Look');
    final prompt = agent.last('session/prompt')['params']['prompt'] as List;
    expect(prompt.map((block) => block['type']), ['text', 'image', 'resource_link']);
    expect(session.attachments, isEmpty);
    await finish(tester);
  });

  test('a session closed before Send leaves nothing in the ledger or on disk', () async {
    final dir = Directory.systemTemp.createTempSync('canoa-drafts');
    addTearDown(() => dir.deleteSync(recursive: true));
    final ledgerFile = File('${dir.path}/sessions.json');
    final ledger = SessionLedger(FileLedgerBackend(ledgerFile.path));
    final keeper = DraftKeeper(FileDraftBackend('${dir.path}/drafts'));
    final store = SessionStore(includePreview: false, ledger: ledger, drafts: keeper);
    final draft = store.createDraft(source: _source, cwd: '/work/repo');
    store.keepDraftText(draft, 'Half a thought');
    expect(store.addAttachment(draft, 'shot.png', [137, 80, 78, 71]), isNull);
    await keeper.flush();
    expect(Directory('${dir.path}/drafts').listSync(), hasLength(1), reason: 'kept while it is open');

    await store.close(draft);
    await keeper.flush();
    await ledger.flush();
    expect(Directory('${dir.path}/drafts').listSync(), isEmpty);
    expect(jsonDecode(ledgerFile.readAsStringSync())['sessions'], isEmpty);
  });

  test('a session not yet started comes back after a restart, with its text, choices and attachments', () async {
    final dir = Directory.systemTemp.createTempSync('canoa-drafts');
    addTearDown(() => dir.deleteSync(recursive: true));
    final before = SessionStore(includePreview: false, drafts: DraftKeeper(FileDraftBackend('${dir.path}/drafts')));
    final draft = before.createDraft(source: _source, cwd: '/work/repo', worktree: true);
    before.keepDraftText(draft, 'Half a thought');
    before.addAttachment(draft, 'shot.png', [137, 80, 78, 71]);
    before.addText(draft, 'line\n' * 3);
    before.addFolder(draft, 'repo', '/work/repo');
    draft.pending['model'] = const PendingChoice('model', OptionRole.model, 'opus');
    before.updateDraft(draft, overlays: {'team'});
    await before.drafts!.flush();

    final after = SessionStore(includePreview: false, drafts: DraftKeeper(FileDraftBackend('${dir.path}/drafts')));
    await after.restoreDrafts((kind, label) => kind == 'command' && label == 'Scripted' ? _source : null);
    final back = after.sessions.single;
    expect((back.isDraft, back.source, back.cwd, back.wantsWorktree, back.draftText), (true, _source, '/work/repo', true, 'Half a thought'));
    expect(back.overlays, {'team'});
    expect(back.pending['model']?.value, 'opus');
    expect(back.attachments.map((item) => (item.kind, item.name)), [
      (AttachmentKind.image, 'shot.png'),
      (AttachmentKind.text, 'Pasted text, 4 lines'),
      (AttachmentKind.folder, 'repo'),
    ]);
    expect(back.attachments.first.bytes, [137, 80, 78, 71]);
  });

  test('a draft closed while its agent starts is not bound, and the starter takes back what it made', () async {
    final store = SessionStore(includePreview: false);
    final agent = OptionsAgent();
    var cleaned = false;
    store.starter = (draft) async {
      final client = await agent.start(scripted);
      await store.close(draft);
      try {
        store.bindDraft(draft, client, await client.openSession(cwd: '/w'), cwd: '/w');
      } catch (_) {
        await client.close();
        cleaned = true;
        rethrow;
      }
    };
    final draft = store.createDraft(source: _source, cwd: '/w');
    await expectLater(store.sendText('hello'), throwsStateError);
    expect(cleaned, isTrue);
    expect(store.sessions, isEmpty);
    expect(agent.methods, isNot(contains('session/prompt')));
    expect(draft.remoteSessionId, isNull);
  });

  test('a choice before the agent starts that it does not offer is said, not sent', () async {
    final store = SessionStore(includePreview: false);
    final agent = OptionsAgent();
    store.starter = (draft) async {
      final client = await agent.start(scripted);
      store.bindDraft(draft, client, await client.openSession(cwd: '/w'), cwd: '/w');
    };
    final draft = store.createDraft(source: _source, cwd: '/w');
    draft.pending['model'] = const PendingChoice('model', OptionRole.model, 'gone');
    await store.sendText('hello');
    expect(agent.methods, isNot(contains('session/set_config_option')));
    expect(draft.messages.any((message) => message.text == 'The agent offers no model of gone now, so it was not set.'), isTrue);
  });
}
