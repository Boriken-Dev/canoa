// The app with every outside service a test controls: what sources need, folders, files, the clipboard, starting an agent
// and making worktrees.
import 'dart:io' show Directory;
import 'dart:typed_data';

import 'package:canoa/acp/agent_commands.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatDensity, ClipboardReader;
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/desktop_shell.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/main.dart';
import 'package:canoa/popout/popout_hub.dart' show PopoutWindows;
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/source_choices.dart';
import 'package:canoa/worktree/worktree.dart';
import 'package:file_selector/file_selector.dart' show XFile;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_host.dart' show defaultLocalSources, linkedHost;
import 'source_choices.dart';

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
  final opened = <String>[];

  HostLink open(String name) {
    if (name != '@control') opened.add(name);
    throw StateError('nothing starts in this test');
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

/// The programs each of the host's own sources needs, by name.
const _needs = {'Claude': ['claude'], 'Codex': ['codex'], 'Antigravity': ['agy'], 'Grok': ['grok'], 'Pi': ['pi']};

/// The app on [store] at [size], each outside service the test's. Answers what canoa.cfg was written as.
Future<List<CanoaConfig>> pumpApp(
  WidgetTester tester,
  SessionStore store, {
  CanoaConfig config = const CanoaConfig(agents: [scripted], density: ChatDensity.verbose),
  Set<String> missing = const {},
  Size size = const Size(1280, 800),
  Future<String?> Function()? pickFolder,
  bool Function(String folder)? folderExists,
  Future<List<XFile>> Function()? pickFiles,
  Map<String, int> files = const {},
  ClipboardReader? clipboard,
  HostLink Function(String name)? linked,
  Worktrees? worktrees,
  DesktopShell? shell,
  HostSupervisor? host,
  String? documents,
  ChoicesLoader? askChoices,
  PopoutWindows? popoutWindows,
}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final dir = Directory.systemTemp.createTempSync('canoa-app');
  addTearDown(() => dir.deleteSync(recursive: true));
  // A session with no folder chosen makes its own under Documents: never the person's.
  final home = Directory.systemTemp.createTempSync('canoa-documents');
  addTearDown(() => home.deleteSync(recursive: true));
  final written = <CanoaConfig>[];
  await tester.pumpWidget(CanoaApp(
    documents: documents ?? home.path,
    store: store,
    config: config,
    configPath: '${dir.path}/canoa.cfg',
    writeConfig: (changed) async => written.add(changed),
    isGitRepository: (folder) => folder.endsWith('repo'),
    folderExists: folderExists ?? (_) => true,
    pickFolder: pickFolder ?? () async => '/work/repo',
    pickFiles: pickFiles ?? () async => const [],
    statFile: (path) async => files[path],
    clipboard: clipboard ?? FakeClipboard(),
    worktrees: () => worktrees ?? CountedWorktrees(),
    shell: shell,
    // The host's own sources are listed, and a session on one runs over a link that is the test's.
    host: host ??
        linkedHost(linked ?? (name) => throw StateError('nothing starts in this test: $name'),
            // The commands of canoa.cfg are sources the host serves, and a Settings save that changes them restarts it.
            sources: () => [
              ...defaultLocalSources,
              for (final agent in (written.isEmpty ? config : written.last).agents) (agent.label, null),
            ],
            missing: {
          for (final MapEntry(:key, :value) in _needs.entries)
            if (value.any(missing.contains)) key: [for (final program in value) if (missing.contains(program)) program],
        }),
    askChoices: askChoices ?? FakeHost().call,
    popoutWindows: popoutWindows,
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

/// The items of the model pill's menu ('model-choice') or of its effort submenu ('effort-choice').
Finder choiceItems(String kind) =>
    find.byWidgetPredicate((widget) => widget.key is ValueKey<String> && (widget.key as ValueKey<String>).value.startsWith('$kind='));

/// Opens the model pill, and for the effort its submenu.
Future<void> _openModelMenu(WidgetTester tester, {required bool effort}) async {
  await tester.tap(find.byKey(const Key('option-model')));
  await frames(tester);
  if (effort) {
    await tester.tap(find.byKey(const Key('option-effort')));
    await frames(tester);
  }
}

/// Picks [label] from the composer's control [key]. The model and effort are one pill: `option-effort` is its submenu.
Future<void> pick(WidgetTester tester, String key, String label) async {
  if (key == 'option-effort') {
    await _openModelMenu(tester, effort: true);
  } else {
    await tester.tap(find.byKey(Key(key)));
    await frames(tester);
  }
  await tester.tap(find.text(label).last);
  await settle(tester);
}

/// The labels a control's menu offers, closing it again. For `option-model` the models, for `option-effort` the efforts in its
/// submenu; a mode's menu without its header or the descriptions under each mode.
Future<List<String>> menuOf(WidgetTester tester, String key) async {
  List<String> firstTexts(Finder items) {
    final out = <String>[];
    for (final item in items.evaluate()) {
      final texts = find.descendant(of: find.byWidget(item.widget), matching: find.byType(Text)).evaluate();
      if (texts.isNotEmpty) out.add((texts.first.widget as Text).data ?? '');
    }
    return out;
  }
  final List<String> labels;
  if (key == 'option-model' || key == 'option-effort') {
    await _openModelMenu(tester, effort: key == 'option-effort');
    final kind = key == 'option-model' ? 'model-choice' : 'effort-choice';
    labels = firstTexts(choiceItems(kind));
  } else {
    await tester.tap(find.byKey(Key(key)));
    await frames(tester);
    final items = find.byWidgetPredicate(
      (widget) => (widget is PopupMenuItem || widget is CheckedPopupMenuItem) && widget.key != const Key('mode-header'),
    );
    labels = firstTexts(items);
  }
  await tester.tapAt(const Offset(2, 2));
  await frames(tester);
  return labels;
}

/// Lets what the app left timed, a snackbar or a ledger write, run out before the test ends.
Future<void> finish(WidgetTester tester) async {
  await tester.pump(const Duration(seconds: 10));
  await tester.pump(const Duration(seconds: 10));
}
