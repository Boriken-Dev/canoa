// A session with no folder chosen works in a scratch folder of its own, shown in the new-session view, made on its first
// message and not before, and set in Settings by a template.
import 'dart:io';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/acp/client.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_tool.dart' show HostLink;
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/sources.dart';
import 'package:canoa/workspace/workspace_template.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

const _source = AgentSource.hosted('Scripted', local: true);
final _sep = Platform.pathSeparator;

/// Where [documents] puts the default scratch folder of [session].
String _scratch(Directory documents, ChatSession session) =>
    [documents.path, 'Canoa', 'scratch', session.workspaceId!].join(_sep);

Directory _documents() {
  final documents = Directory.systemTemp.createTempSync('canoa-documents');
  addTearDown(() => documents.deleteSync(recursive: true));
  return documents;
}

/// An agent that notes the folder it was started in.
final class _Agent {
  final agent = OptionsAgent();

  /// The folder each session was opened in, as the agent was asked.
  List<String?> get folders => [
    for (final request in agent.requests)
      if (request['method'] == 'session/new') (request['params'] as Map)['cwd'] as String?,
  ];

  Future<VerifiedAgent> start(AgentCommand command, {AgentDecider? decider, AgentElicitor? elicitor, String? workingDirectory,
      Map<String, String> environment = const {}}) =>
      agent.start(command, decider: decider, elicitor: elicitor, workingDirectory: workingDirectory, environment: environment);

  HostLink openLink(String name) => agent.openLink(name);
}

void main() {
  testWidgets('no folder chosen: the view shows the scratch folder, and the first message makes it and works in it', (tester) async {
    final documents = _documents();
    final ledger = SessionLedger(MemoryLedgerBackend());
    final store = SessionStore(ledger: ledger);
    final started = _Agent();
    await pumpApp(tester, store, documents: documents.path, linked: started.openLink);
    await newSession(tester);
    final draft = store.selected!;
    expect(draft.cwd, isNull, reason: 'no folder was chosen');
    final shown = tester.widget<Text>(find.byKey(const Key('new-session-scratch-path'))).textSpan!.toPlainText();
    expect(draft.workspaceId, matches(RegExp(r'^[0-9a-f-]{36}$')), reason: 'Canoa names the session before any agent runs');
    final folder = _scratch(documents, draft);
    expect(find.byKey(const Key('new-session-scratch-path')), findsOneWidget);
    expect(tester.widget<Text>(find.byKey(const Key('new-session-scratch-path'))).textSpan!.toPlainText(), contains(folder));
    expect(find.text('Choose a folder'), findsNothing);
    expect(Directory(folder).existsSync(), isFalse, reason: 'showing it makes nothing');
    expect(Directory('${documents.path}${_sep}Canoa').existsSync(), isFalse);

    await chooseSource(tester, 'Scripted');
    expect(Directory(folder).existsSync(), isFalse, reason: 'choosing the source makes nothing');
    expect(shown, 'Scratch  $folder', reason: 'the pill shows exactly the folder');
    await send(tester, 'hello');
    await until(tester, () => draft.remoteSessionId != null, 'the session to start');
    expect(Directory(folder).existsSync(), isTrue, reason: 'made by the first message');
    expect(started.folders, [folder]);
    expect(draft.cwd, folder);
    final entry = ledger.entries.single;
    expect(entry.canoaId, draft.workspaceId, reason: 'kept beside the id the agent reports');
    expect(entry.sessionId, draft.remoteSessionId);
    expect(entry.cwd, folder);
    expect(LedgerEntry.fromJson(entry.toJson())!.canoaId, draft.workspaceId);
    await finish(tester);
  });

  testWidgets('two sessions with no folder get two folders', (tester) async {
    final documents = _documents();
    final store = SessionStore();
    await pumpApp(tester, store, documents: documents.path);
    final first = store.createDraft(source: _source);
    final second = store.createDraft(source: _source);
    expect(first.workspaceId, isNot(second.workspaceId));
  });

  testWidgets('picking a folder wins, and the scratch folder is not made', (tester) async {
    final documents = _documents();
    final store = SessionStore();
    final started = _Agent();
    final work = Directory.systemTemp.createTempSync('canoa-work');
    addTearDown(() => work.deleteSync(recursive: true));
    await pumpApp(tester, store, documents: documents.path, linked: started.openLink, pickFolder: () async => work.path);
    await newSession(tester);
    final draft = store.selected!;
    await openFolder(tester);
    expect(draft.cwd, work.path);
    expect(find.byKey(const Key('new-session-scratch-path')), findsNothing);
    await chooseSource(tester, 'Scripted');
    await send(tester, 'hello');
    await until(tester, () => draft.remoteSessionId != null, 'the session to start');
    expect(started.folders, [work.path]);
    expect(Directory('${documents.path}${_sep}Canoa').existsSync(), isFalse);

    // The next new session starts in Scratch again, with the folder just used one click away in the menu.
    await newSession(tester);
    final next = store.selected!;
    expect(next, isNot(draft));
    expect(next.cwd, isNull);
    expect(find.byKey(const Key('new-session-scratch-path')), findsOneWidget);
    expect(await menuOf(tester, 'new-session-folder'), contains(work.path));

    // The menu takes it back to the scratch folder before the session starts.
    final other = store.createDraft(source: _source, cwd: work.path);
    store.select(other.id);
    await frames(tester);
    await tester.tap(find.byKey(const Key('new-session-folder')));
    await frames(tester);
    await tester.tap(find.byKey(const Key('new-session-scratch')));
    await frames(tester);
    expect(other.cwd, isNull);
    expect(find.byKey(const Key('new-session-scratch-path')), findsOneWidget);
    await finish(tester);
  });

  testWidgets('a draft kept between runs keeps its scratch id', (tester) async {
    final store = SessionStore();
    final draft = store.createDraft(source: _source, workspaceId: '11111111-2222-4333-8444-555555555555');
    expect(draft.workspaceId, '11111111-2222-4333-8444-555555555555');
  });

  testWidgets('the folder follows canoa.cfg\'s template', (tester) async {
    final documents = _documents();
    final store = SessionStore();
    const config = CanoaConfig(agents: [scripted], sessions: LedgerSettings(workspace: r'${documents}/work/${source}/${session}'));
    await pumpApp(tester, store, documents: documents.path, config: config);
    await newSession(tester);
    final draft = store.selected!;
    await chooseSource(tester, 'Scripted');
    expect(
      tester.widget<Text>(find.byKey(const Key('new-session-scratch-path'))).textSpan!.toPlainText(),
      contains([documents.path, 'work', 'Scripted', draft.workspaceId!].join(_sep)),
    );
  });

  testWidgets('the time in the folder is fixed when the draft is made, and the pill shows the folder the message makes', (tester) async {
    final documents = _documents();
    final store = SessionStore();
    final started = _Agent();
    const config = CanoaConfig(agents: [scripted], sessions: LedgerSettings(workspace: r'${documents}/${datetime}/${session}'));
    await pumpApp(tester, store, documents: documents.path, config: config, linked: started.openLink);
    await newSession(tester);
    final draft = store.selected!;
    final shown = tester.widget<Text>(find.byKey(const Key('new-session-scratch-path'))).textSpan!.toPlainText();
    final t = draft.created.toLocal();
    String two(int n) => n.toString().padLeft(2, '0');
    expect(shown, contains('${t.year}-${two(t.month)}-${two(t.day)}T${two(t.hour)}${two(t.minute)}${two(t.second)}'));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 2100)));
    await frames(tester);
    await send(tester, 'hello');
    await until(tester, () => draft.remoteSessionId != null, 'the session to start');
    expect(shown, 'Scratch  ${started.folders.single}');
    expect(Directory(started.folders.single!).existsSync(), isTrue);
    await finish(tester);
  });

  testWidgets('Settings shows the template with a preview, refuses a bad one and saves a good one', (tester) async {
    final documents = _documents();
    final store = SessionStore();
    final written = await pumpApp(tester, store, documents: documents.path);
    await tester.tap(find.byTooltip('Canoa settings'));
    await tester.pumpAndSettle();
    final field = find.byKey(const Key('settings-workspace'));
    await tester.ensureVisible(field);
    expect(tester.widget<TextField>(field).controller!.text, defaultWorkspace);
    expect(find.textContaining(documents.path), findsOneWidget, reason: 'the preview');
    expect(find.textContaining(exampleSessionId), findsOneWidget);

    await tester.enterText(field, r'${documents}/${nope}/${session}');
    await tester.pump();
    expect(find.textContaining(r'unknown variable ${nope}'), findsOneWidget);
    expect(tester.widget<FilledButton>(find.widgetWithText(FilledButton, 'Save')).onPressed, isNull);

    await tester.enterText(field, r'${documents}/day/${date}/${session}');
    await tester.pump();
    expect(find.textContaining(r'unknown variable'), findsNothing);
    final today = DateTime.now();
    final date = '${today.year.toString().padLeft(4, '0')}-${today.month.toString().padLeft(2, '0')}-${today.day.toString().padLeft(2, '0')}';
    expect(find.textContaining('day$_sep$date$_sep$exampleSessionId'), findsOneWidget);
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(written.last.sessions.workspace, r'${documents}/day/${date}/${session}');
    expect(CanoaConfig.parse(written.last.encode()).sessions.workspace, written.last.sessions.workspace);
  });
}
