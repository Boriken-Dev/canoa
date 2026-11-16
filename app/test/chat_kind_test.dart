// A session is a chat, which works in a folder Canoa made and never shows, or a workspace session in a folder the person chose.
// The ledger keeps which, a chat reopens in its own folder, and the conversation's name is the agent's until the person names it.
import 'dart:io';

import 'package:canoa/ledger/ledger_list.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/rail/session_groups.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

const _source = AgentSource.hosted('Scripted', local: true);
const _folder = '/chats/one';

Directory _documents() {
  final documents = Directory.systemTemp.createTempSync('canoa-documents');
  addTearDown(() => documents.deleteSync(recursive: true));
  return documents;
}

/// A chat started by its first message against [agent], without the screen: the draft is bound where the app binds it.
Future<ChatSession> _startedChat(SessionStore store, OptionsAgent agent) async {
  store.starter = (draft) async {
    final client = await agent.start(scripted);
    store.bindDraft(draft, client, await client.openSession(cwd: _folder), cwd: _folder);
  };
  final draft = store.createDraft(source: _source);
  expect(draft.kind, SessionKind.chat);
  await store.sendText('Fix the parser');
  return draft;
}

/// Lets what the agent sent reach the session.
Future<void> _heard() => Future<void>.delayed(const Duration(milliseconds: 60));

void main() {
  group('the title', () {
    test('the agent names it, the person\'s name wins over any later one, and wins again once it is reopened', () async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      final store = SessionStore(ledger: ledger);
      final agent = OptionsAgent();
      final session = await _startedChat(store, agent);
      expect(session.title, 'Fix the parser', reason: 'until the agent says, the first line of the message');

      agent.titled('Parser fix');
      await _heard();
      expect(session.title, 'Parser fix');
      expect(ledger.entries.single.title, 'Parser fix');
      expect(ledger.entries.single.named, isFalse);

      store.rename(session, 'My parser');
      agent.titled('Something else');
      await _heard();
      expect(session.title, 'My parser', reason: 'the agent no longer names it');
      expect(ledger.entries.single.title, 'My parser');
      expect(ledger.entries.single.named, isTrue);
      expect(LedgerEntry.fromJson(ledger.entries.single.toJson())!.named, isTrue, reason: 'kept across a restart');

      final entry = ledger.entries.single;
      await store.close(session);
      agent.replay.add({'sessionUpdate': 'session_info_update', 'title': 'Replayed by the agent'});
      final reopened = await store.reopen(entry, await agent.start(scripted));
      expect(reopened.title, 'My parser', reason: 'neither the agent\'s list nor its replay replaces the person\'s name');
      agent.titled('And again');
      await _heard();
      expect(reopened.title, 'My parser');
      expect(ledger.entries.single.title, 'My parser');
    });

    test('a chat nobody renamed takes the agent\'s later title after it is reopened', () async {
      final ledger = SessionLedger(MemoryLedgerBackend());
      final store = SessionStore(ledger: ledger);
      final agent = OptionsAgent();
      final session = await _startedChat(store, agent);
      agent.titled('Parser fix');
      await _heard();
      final entry = ledger.entries.single;
      await store.close(session);
      final reopened = await store.reopen(entry, await agent.start(scripted));
      agent.titled('A better name');
      await _heard();
      expect(reopened.title, 'A better name');
      expect(ledger.entries.single.title, 'A better name');
    });
  });

  test('a reopened chat keeps its kind and its folder', () async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    final store = SessionStore(ledger: ledger);
    final agent = OptionsAgent();
    final session = await _startedChat(store, agent);
    expect(session.kind, SessionKind.chat);
    expect(session.cwd, _folder);
    expect(locationOf(session), isNull, reason: 'a chat\'s row names no folder');
    expect([for (final group in groupByWorkspace(store.sessions)) group.label], ['Chats']);

    final entry = ledger.entries.single;
    expect(entry.kind, SessionKind.chat);
    expect(LedgerEntry.fromJson(entry.toJson())!.kind, SessionKind.chat);
    await store.close(session);
    final reopened = await store.reopen(entry, await agent.start(scripted));
    expect(reopened.kind, SessionKind.chat);
    expect(reopened.cwd, _folder, reason: 'resumed in its own folder');
    expect(reopened.workspaceId, session.workspaceId);
    expect((agent.last('session/load')['params'] as Map)['cwd'], _folder);
    expect([for (final group in groupByWorkspace(store.sessions)) group.label], ['Chats']);
  });

  testWidgets('Earlier conversations lists chats under Chats and shows no folder for them', (tester) async {
    final ledger = SessionLedger(MemoryLedgerBackend());
    ledger.record(
      peerId: 'local:Scripted', sessionId: 'c1', entry: 'Scripted', place: SessionPlace.local, reopenBy: 'session/load',
      cwd: '/chats/one', title: 'A chat', kind: SessionKind.chat,
    );
    ledger.record(
      peerId: 'local:Scripted', sessionId: 'w1', entry: 'Scripted', place: SessionPlace.local, reopenBy: 'session/load',
      cwd: '/work/repo', title: 'A project',
    );
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: LedgerList(ledger: ledger, store: SessionStore()))));
    await tester.pump();
    expect(find.byKey(const ValueKey('earlier chats')), findsOneWidget);
    expect(find.text('A chat'), findsOneWidget);
    expect(find.textContaining('/chats/one'), findsNothing, reason: 'the folder of a chat is not listed');
    expect(find.textContaining('/work/repo'), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(seconds: 1));
  });

  test('a workspace session is grouped by its folder, and an entry without a kind is one', () {
    final ledger = SessionLedger(MemoryLedgerBackend());
    ledger.record(peerId: 'local:Scripted', sessionId: 's1', entry: 'Scripted', place: SessionPlace.local, cwd: '/w/repo');
    expect(ledger.entries.single.kind, SessionKind.workspace);
    final json = ledger.entries.single.toJson()..remove('kind');
    expect(LedgerEntry.fromJson(json)!.kind, SessionKind.workspace);
    final store = SessionStore();
    final workspace = store.createDraft(cwd: '/w/repo');
    final chat = store.createDraft();
    expect((workspace.kind, chat.kind), (SessionKind.workspace, SessionKind.chat));
    expect([for (final group in groupByWorkspace(store.sessions)) group.label], ['Chats', 'repo']);
    expect(locationOf(workspace), 'repo');
  });

  testWidgets('the inspector says Chat and keeps the folder under its technical details', (tester) async {
    final store = SessionStore();
    final agent = OptionsAgent();
    await pumpApp(tester, store, documents: _documents().path, linked: agent.openLink);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    final session = store.selected!;
    await send(tester, 'Fix the parser');
    await until(tester, () => session.remoteSessionId != null, 'the session to start');
    final folder = session.cwd!;
    await tester.tap(find.byTooltip('Session details'));
    await frames(tester);
    expect(find.byKey(const ValueKey('fact Kind')), findsOneWidget);
    expect(find.byKey(const ValueKey('fact Workspace')), findsNothing);
    expect(find.text(folder), findsNothing, reason: 'folded away');
    await tester.tap(find.byKey(const Key('inspector-technical')));
    await frames(tester);
    expect(find.text(folder), findsOneWidget);
    await finish(tester);
  });
}
