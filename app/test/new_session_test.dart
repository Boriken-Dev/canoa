// Sources in Settings and a new session's empty view: a source added in Settings is offered, a preset that cannot run says
// what it needs, and a new session is in the list at once with nothing started.
import 'dart:io' show Directory;

import 'package:canoa/chat/new_session_view.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

Future<(SessionStore, Starts, CountedWorktrees, List<CanoaConfig>)> _app(
  WidgetTester tester, {
  CanoaConfig config = const CanoaConfig(agents: [scripted]),
  Set<String> missing = const {},
  Size size = const Size(1280, 800),
  Future<String?> Function()? pickFolder,
}) async {
  final store = SessionStore(includePreview: false);
  final starts = Starts();
  final worktrees = CountedWorktrees();
  final written = await pumpApp(tester, store, config: config, missing: missing, size: size, pickFolder: pickFolder,
      startAgent: starts.call, worktrees: worktrees);
  return (store, starts, worktrees, written);
}

/// The labels the source menu offers.
Future<List<String>> _offered(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('new-session-source')));
  await tester.pumpAndSettle();
  final labels = {
    for (final element in find.byWidgetPredicate((widget) => widget.key is ValueKey<String> &&
        (widget.key! as ValueKey<String>).value.startsWith('source ')).evaluate())
      (element.widget.key! as ValueKey<String>).value.substring('source '.length),
  }.toList();
  await tester.tapAt(const Offset(2, 2));
  await tester.pumpAndSettle();
  return labels;
}

Future<void> _openSources(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Canoa settings'));
  await tester.pumpAndSettle();
  await tester.tap(find.byKey(const Key('settings-sources')));
  await tester.pumpAndSettle();
}

bool _worktreeEnabled(WidgetTester tester) =>
    tester.widget<SegmentedButton<bool>>(find.byKey(const Key('new-session-place'))).segments.last.enabled;

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/new_session_test.dart')));

  test('hidden presets and attachment limits go into Canoa.cfg and come back', () {
    const config = CanoaConfig(hiddenSources: {'Gemini', 'Pi'}, pasteAsFile: 100, embedLimit: 2048);
    final read = CanoaConfig.parse(config.encode());
    expect(read.hiddenSources, {'Gemini', 'Pi'});
    expect((read.pasteAsFile, read.embedLimit), (100, 2048));
    expect((const CanoaConfig().pasteAsFile, const CanoaConfig().embedLimit), (8000, 1024 * 1024));
    expect(() => CanoaConfig.parse('[sources]\nhidden = [1]\n'), throwsFormatException);
    expect(() => CanoaConfig.parse('[sources]\nother = true\n'), throwsFormatException);
    expect(() => CanoaConfig.parse('[attachments]\nembed_limit = -1\n'), throwsFormatException);
  });

  test('a source that cannot run says what it needs, found or not', () {
    const check = SourceCheck(found: {'claude': r'C:\tools\claude.exe'}, missing: ['npx']);
    expect(check.ready, isFalse);
    expect(
      check.describe('Claude', needs: const ['npx', 'claude']),
      r'Claude needs npx and claude on PATH: found claude at C:\tools\claude.exe; npx was not found. '
      'Install what is missing, or start Canoa where PATH finds it.',
    );
    expect(const SourceCheck(found: {'x': '/x'}).describe('X'), isNull);
  });

  testWidgets('a command source added in Settings is saved to Canoa.cfg and offered for a new session', (tester) async {
    final (_, _, _, written) = await _app(tester, config: const CanoaConfig(), size: const Size(1280, 1400));
    await _openSources(tester);
    await tester.enterText(find.byKey(const Key('source-name')), 'Mine');
    await tester.enterText(find.byKey(const Key('source-command')), 'my-agent --acp "a b"');
    await tester.ensureVisible(find.byKey(const Key('source-add')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('source-add')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('source Mine')), findsOneWidget);
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    final mine = written.last.agents.single;
    expect([mine.label, mine.program, ...mine.args], ['Mine', 'my-agent', '--acp', 'a b']);

    await newSession(tester);
    expect(await _offered(tester), containsAll(['Claude', 'Codex', 'Gemini', 'Pi', 'Mine']));
  });

  testWidgets('a preset hidden in Settings is not offered', (tester) async {
    final (_, _, _, written) = await _app(tester, size: const Size(1280, 1400));
    await _openSources(tester);
    await tester.tap(find.byKey(const Key('source-offered Gemini')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(written.last.hiddenSources, {'Gemini'});
    await newSession(tester);
    final offered = await _offered(tester);
    expect(offered, isNot(contains('Gemini')));
    expect(offered, containsAll(['Claude', 'Codex', 'Pi', 'Scripted']));
  });

  testWidgets('a preset that cannot run says what it is missing, in Settings and for a new session, and starts nothing', (tester) async {
    final (store, starts, _, _) = await _app(tester, missing: {'npx'}, size: const Size(1280, 1400));
    await _openSources(tester);
    expect(find.text('Claude needs npx and claude on PATH: found claude at /usr/bin/claude; npx was not found. '
        'Install what is missing, or start Canoa where PATH finds it.'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    await newSession(tester);
    expect(await _offered(tester), contains('Claude'), reason: 'never hidden for what it lacks');
    await tester.tap(find.byKey(const Key('new-session-source')));
    await tester.pumpAndSettle();
    expect(find.descendant(of: find.byKey(const Key('source Claude')), matching: find.text('Needs npx')), findsOneWidget);
    await tester.tap(find.byKey(const Key('source Claude')).last);
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'Fix it');
    await tester.tap(find.byTooltip('Send'));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
    await tester.pumpAndSettle();
    expect(find.textContaining('Not started: Claude needs npx and claude on PATH'), findsWidgets);
    expect(starts.commands, isEmpty);
    expect(store.selected?.status, SessionStatus.draft);
    expect(find.widgetWithText(TextField, 'Fix it'), findsOneWidget, reason: 'the message stays to send again');
  });

  testWidgets('a new session is in the list at once, with no agent started and no worktree made', (tester) async {
    final (store, starts, worktrees, _) = await _app(tester);
    await newSession(tester);
    expect(find.byType(NewSessionView), findsOneWidget);
    await chooseSource(tester, 'Scripted');
    await openFolder(tester);
    expect(_worktreeEnabled(tester), isTrue, reason: '/work/repo is in a git repository');
    await tester.tap(find.text('Worktree'));
    await tester.pumpAndSettle();
    // Real input and output gets its chance, which a start would need.
    await settle(tester, 300);

    final session = store.sessions.single;
    expect(session.status, SessionStatus.draft);
    expect((session.agent, session.cwd, session.wantsWorktree), ('Scripted', '/work/repo', true));
    expect(session.worktree, isNull);
    expect(session.remoteSessionId, isNull);
    expect(starts.commands, isEmpty, reason: 'no process before the first message');
    expect(worktrees.made, isEmpty, reason: 'no worktree before the first message');
    expect(find.descendant(of: find.byType(ListTile), matching: find.text('New')), findsOneWidget);
  });

  testWidgets('Worktree can be chosen only for a folder in a git repository', (tester) async {
    await _app(tester, pickFolder: () async => '/work/plain');
    await newSession(tester);
    await openFolder(tester);
    expect(_worktreeEnabled(tester), isFalse);
  });

  testWidgets('a new session closed before its first message leaves nothing', (tester) async {
    final (store, starts, worktrees, _) = await _app(tester);
    await newSession(tester);
    await tester.tap(find.byTooltip('Close conversation'));
    await tester.pumpAndSettle();
    expect(store.sessions, isEmpty);
    expect(starts.commands, isEmpty);
    expect(worktrees.made, isEmpty);
  });

  testWidgets('Ctrl+N and the palette open a new session with the last source and folder chosen, one at a time', (tester) async {
    final (store, _, _, _) = await _app(tester);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await openFolder(tester);
    await tester.enterText(find.byType(TextField), 'Something to keep');
    await tester.pumpAndSettle();

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyN);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pumpAndSettle();
    expect(store.sessions, hasLength(2));
    final second = store.selected!;
    expect((second.source?.label, second.cwd), ('Scripted', '/work/repo'));
    expect(find.descendant(of: find.byKey(const Key('new-session-source')), matching: find.text('Scripted')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('new-session-folder')), matching: find.text('/work/repo')), findsOneWidget);

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyK);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pumpAndSettle();
    await tester.tap(find.text('New session').last);
    await tester.pumpAndSettle();
    expect(store.sessions, hasLength(2), reason: 'an untouched new session is shown again rather than another made');
    expect(store.selected, second);
  });

  for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
    testWidgets('a new session draws at $label width with nothing overflowing', (tester) async {
      await _app(tester, size: size);
      await newSession(tester);
      await chooseSource(tester, 'Scripted');
      await openFolder(tester);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/new_session_$label.png'));
    });
  }
}
