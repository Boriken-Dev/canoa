// The app with every outside service a test controls: what sources need, folders, files, the clipboard, starting an agent
// and making worktrees.
import 'dart:io' show Directory;
import 'dart:typed_data';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/acp/client.dart';
import 'package:canoa/chat/composer.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/desktop_shell.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/main.dart';
import 'package:canoa/profile/launch.dart' show StartAgent;
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/sources.dart';
import 'package:canoa/worktree/worktree.dart';
import 'package:file_selector/file_selector.dart' show XFile;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// Worktrees that are only counted.
final class CountedWorktrees implements Worktrees {
  final made = <(String, String)>[];
  final removed = <SessionWorktree>[];

  @override
  Future<SessionWorktree> create(String cwd, String branch) async {
    made.add((cwd, branch));
    return SessionWorktree(path: worktreeLocation(cwd, branch), branch: branch, repo: cwd);
  }

  @override
  Future<WorktreeState> status(SessionWorktree worktree) async => const WorktreeState(branch: null, changed: 0, commits: 0);

  @override
  Future<WorktreeRemoval> remove(SessionWorktree worktree, {bool discard = false}) async {
    removed.add(worktree);
    return WorktreeRemoval(branch: worktree.branch, branchKept: false, commits: 0);
  }
}

/// Everything the app would start, counted, and nothing started.
final class Starts {
  final commands = <AgentCommand>[];

  Future<VerifiedAgent> call(AgentCommand command, {AgentDecider? decider, AgentElicitor? elicitor, String? workingDirectory,
      Map<String, String> environment = const {}}) async {
    commands.add(command);
    throw const AcpFailure('nothing starts in this test');
  }
}

/// What a test puts on the clipboard.
final class FakeClipboard implements ClipboardReader {
  Uint8List? copiedImage;
  List<String> copiedFiles = const [];
  String? copiedText;

  @override
  Future<Uint8List?> image() async => copiedImage;
  @override
  Future<List<String>> files() async => copiedFiles;
  @override
  Future<String?> text() async => copiedText;
}

const scripted = AgentCommand(label: 'Scripted', program: 'scripted-agent', args: ['--acp']);

/// Every program is found, except those in [missing].
Future<SourceCheck> Function(AgentSource) checkFinding({Set<String> missing = const {}}) => (source) async {
  final command = source.command;
  if (command == null) return const SourceCheck();
  return SourceCheck(
    found: {for (final program in commandNeeds(command)) if (!missing.contains(program)) program: '/usr/bin/$program'},
    missing: [for (final program in commandNeeds(command)) if (missing.contains(program)) program],
  );
};

/// The app on [store] at [size], each outside service the test's. Answers what canoa.cfg was written as.
Future<List<CanoaConfig>> pumpApp(
  WidgetTester tester,
  SessionStore store, {
  CanoaConfig config = const CanoaConfig(agents: [scripted]),
  Set<String> missing = const {},
  Size size = const Size(1280, 800),
  Future<String?> Function()? pickFolder,
  Future<List<XFile>> Function()? pickFiles,
  Map<String, int> files = const {},
  ClipboardReader? clipboard,
  StartAgent? startAgent,
  Worktrees? worktrees,
  DesktopShell? shell,
  HostSupervisor? host,
}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final dir = Directory.systemTemp.createTempSync('canoa-app');
  addTearDown(() => dir.deleteSync(recursive: true));
  final written = <CanoaConfig>[];
  await tester.pumpWidget(CanoaApp(
    store: store,
    config: config,
    configPath: '${dir.path}/canoa.cfg',
    writeConfig: (changed) async => written.add(changed),
    checkSource: checkFinding(missing: missing),
    isGitRepository: (folder) => folder.endsWith('repo'),
    pickFolder: pickFolder ?? () async => '/work/repo',
    pickFiles: pickFiles ?? () async => const [],
    statFile: (path) async => files[path],
    clipboard: clipboard ?? FakeClipboard(),
    startAgent: startAgent ?? Starts().call,
    worktrees: () => worktrees ?? CountedWorktrees(),
    shell: shell,
    host: host,
  ));
  await tester.pumpAndSettle();
  return written;
}

/// Draws for a while: long enough for a menu or dialog to open or close, with no wait for stillness, which a working
/// session's spinner never reaches.
Future<void> frames(WidgetTester tester) async {
  for (var i = 0; i < 8; i++) {
    await tester.pump(const Duration(milliseconds: 60));
  }
}

/// Opens a new session's view from the side list.
Future<void> newSession(WidgetTester tester) async {
  final button = find.byTooltip('New session');
  if (button.hitTestable().evaluate().isEmpty) {
    await tester.tap(find.byTooltip('Sessions'));
    await frames(tester);
  }
  await tester.tap(button);
  await frames(tester);
}

Future<void> chooseSource(WidgetTester tester, String label) async {
  await tester.tap(find.byKey(const Key('new-session-source')));
  await frames(tester);
  await tester.tap(find.byKey(Key('source $label')).last);
  await frames(tester);
}

Future<void> openFolder(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('new-session-folder')));
  await frames(tester);
  await tester.tap(find.byKey(const Key('new-session-browse')));
  await frames(tester);
}

/// Lets real input and output, and the agent's answers, happen outside the test's clock, then draws.
Future<void> settle(WidgetTester tester, [int ms = 60]) async {
  await tester.runAsync(() => Future<void>.delayed(Duration(milliseconds: ms)));
  await frames(tester);
}

/// Settles until [done], or fails saying [what] after a few seconds.
Future<void> until(WidgetTester tester, bool Function() done, String what) async {
  for (var i = 0; i < 100 && !done(); i++) {
    await settle(tester, 30);
  }
  if (!done()) fail('timed out waiting for $what');
}

/// Writes [text] and presses Send.
Future<void> send(WidgetTester tester, String text) async {
  await tester.enterText(find.byType(TextField).last, text);
  await tester.tap(find.byTooltip('Send'));
  await settle(tester);
}

/// Picks [label] from the composer's control [key].
Future<void> pick(WidgetTester tester, String key, String label) async {
  await tester.tap(find.byKey(Key(key)));
  await frames(tester);
  await tester.tap(find.text(label).last);
  await settle(tester);
}

/// The labels a control's menu offers, closing it again.
Future<List<String>> menuOf(WidgetTester tester, String key) async {
  await tester.tap(find.byKey(Key(key)));
  await frames(tester);
  final items = find.byWidgetPredicate((widget) => widget is PopupMenuItem || widget is CheckedPopupMenuItem);
  final labels = [
    for (final item in items.evaluate())
      for (final text in find.descendant(of: find.byWidget(item.widget), matching: find.byType(Text)).evaluate())
        (text.widget as Text).data ?? '',
  ];
  await tester.tapAt(const Offset(2, 2));
  await frames(tester);
  return labels;
}

/// Lets what the app left timed, a snackbar or a ledger write, run out before the test ends.
Future<void> finish(WidgetTester tester) async {
  await tester.pump(const Duration(seconds: 10));
  await tester.pump(const Duration(seconds: 10));
}
