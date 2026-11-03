// A person's multi-step coding task, done in the app's own window through its widgets only: an agent fixes a failing test, adds
// one, runs them and renames a function; every question is allowed but one, which is refused; one turn is stopped and continued;
// a file is attached; the conversation is closed and opened again from Earlier conversations. Then the repository on disk says
// whether it all happened, and the updates the agent sent say whether each kind has a rendering of its own.
//
//   flutter test integration_test/coding_task_test.dart -d windows   (or -d linux)
//
// `node` has to be on PATH. CANOA_TEST_TASK_AGENT=Claude runs the Claude preset's adapter on the person's own sign-in, through
// without_user_settings.mjs so that their user-level settings and CLAUDE.md stay out, and spends the subscription; without it a
// scripted stand-in (scripted_agent.mjs) plays the agent and nothing is spent. Reaching outside the temporary directory fails the
// test, whether asked (and refused) or done unasked. CANOA_TEST_SHOTS names a
// directory for a screenshot of each step, and CANOA_TEST_UPDATES a file for the kind and shape of each distinct update. Neither
// holds what was said, except the screenshots, which show the screen. CANOA_TEST_APP is this app's directory (default: the
// working directory).
import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/chat/agent_events.dart' show ToolStatus;
import 'package:canoa/chat/transcript.dart';
import 'package:canoa/chat/update_record.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:file_selector_platform_interface/file_selector_platform_interface.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

final String? _real = Platform.environment['CANOA_TEST_TASK_AGENT'];
final String? _shots = Platform.environment['CANOA_TEST_SHOTS'];
final String? _updates = Platform.environment['CANOA_TEST_UPDATES'];
final String _appDir = Platform.environment['CANOA_TEST_APP'] ?? Directory.current.path;

const _fix = 'The test in this repository fails. Find out why and fix the code, not the test. Then add a second test for the same '
    'function and run the tests with `node --test`.';
const _rename = 'Rename the function addNumbers to sum everywhere in this repository, including the README, then run the tests.';
const _goOn = 'Go on with the rename where you stopped, then run the tests.';
const _followNotes = 'Do what the attached notes say.';
const _notes = '1. Add the line `Maintainer: Ana Rivera` directly under the title of README.md.\n'
    '2. Delete the file obsolete.txt with the shell command `rm obsolete.txt`.\n';

/// The file dialog a person would pick from, answered with one file.
final class _Picked extends FileSelectorPlatform {
  _Picked(this.path);
  final String path;

  @override
  Future<List<XFile>> openFiles({List<XTypeGroup>? acceptedTypeGroups, String? initialDirectory, String? confirmButtonText}) async =>
      [XFile(path)];
}

/// A tiny Node project in [dir], committed: one failing test, a function used in three places, and a file to be kept.
Future<void> _project(Directory dir) async {
  final files = {
    'package.json': '{\n  "name": "tiny",\n  "type": "module",\n  "scripts": { "test": "node --test" }\n}\n',
    'README.md': '# tiny\n\n`addNumbers(a, b)` in `src/calc.js` adds two numbers; `total` in `src/report.js` adds a list.\n',
    'src/calc.js': 'export function addNumbers(a, b) {\n  return a - b;\n}\n',
    'src/report.js': "import { addNumbers } from './calc.js';\n\nexport function total(values) {\n"
        '  return values.reduce((sum, value) => addNumbers(sum, value), 0);\n}\n',
    'test/calc.test.js': "import { test } from 'node:test';\nimport assert from 'node:assert/strict';\n"
        "import { addNumbers } from '../src/calc.js';\n\ntest('adds two numbers', () => {\n"
        '  assert.equal(addNumbers(2, 3), 5);\n});\n',
    'obsolete.txt': 'Kept unless the person allows deleting it.\n',
    // The agent asks before each of these, whatever the person's own settings allow. Claude Code on Windows also runs commands
    // through PowerShell, which Bash does not cover.
    '.claude/settings.json': '{"permissions": {"ask": ["Write", "Edit", "Bash", "PowerShell"]}}\n',
  };
  for (final MapEntry(key: path, value: text) in files.entries) {
    final file = File('${dir.path}/$path');
    file.parent.createSync(recursive: true);
    file.writeAsStringSync(text);
  }
  for (final args in [
    ['init', '-q'],
    ['add', '.'],
    ['-c', 'user.name=canoa-test', '-c', 'user.email=canoa-test@localhost', 'commit', '-q', '-m', 'start'],
  ]) {
    final ran = await Process.run('git', args, workingDirectory: dir.path);
    if (ran.exitCode != 0) throw StateError('git ${args.first} failed');
  }
}

/// What `node --test` says in [dir]: its exit code and the passing and failing counts, or -1 for a count it did not give.
Future<(int, int, int)> _nodeTest(Directory dir) async {
  // TAP's summary is ASCII: the default reporter's `ℹ` does not survive Windows decoding a child's output in its code page.
  final ran = await Process.run(
    'node',
    ['--test', '--test-reporter=tap', '--test-timeout=60000'],
    workingDirectory: dir.path,
    runInShell: true,
  ).timeout(const Duration(minutes: 2));
  int count(String what) => int.tryParse(RegExp('^# $what (\\d+)', multiLine: true).firstMatch('${ran.stdout}')?.group(1) ?? '') ?? -1;
  return (ran.exitCode, count('pass'), count('fail'));
}

/// Files under [dir], outside `.git` and `.claude`, that mention [word].
List<String> _mentioning(Directory dir, String word) => [
  for (final entry in dir.listSync(recursive: true))
    if (entry is File &&
        !entry.path.contains('${Platform.pathSeparator}.git${Platform.pathSeparator}') &&
        !entry.path.contains('${Platform.pathSeparator}.claude${Platform.pathSeparator}') &&
        entry.readAsStringSync().contains(word))
      entry.path.substring(dir.path.length + 1),
];

/// An absolute path (Windows, Git Bash's `/c/...` or Unix) or the home directory, where it starts a word.
final _place = RegExp(
  r'''(?<![\w.\-/\\])(?:[A-Za-z]:[\\/]|/[A-Za-z][\w.\-]*/|~[\\/]|\$HOME\b|\$env:USERPROFILE\b|%USERPROFILE%)[^\s"'`;|&<>()]*''',
);

/// Whether [text] names a place outside [root]. `/dev/null` is nowhere.
bool _reachesOut(String text, String root) {
  String plain(String path) =>
      path.replaceAll(r'\', '/').replaceFirstMapped(RegExp(r'^/([A-Za-z])/'), (drive) => '${drive[1]}:/').toLowerCase();
  final inside = plain(root);
  for (final match in _place.allMatches(text)) {
    final path = match[0]!;
    if (path == '/dev/null') continue;
    if (!RegExp(r'^(?:[A-Za-z]:[\\/]|/)').hasMatch(path) || !plain(path).startsWith(inside)) return true;
  }
  return false;
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('a coding task, start to end, through the window', (tester) async {
    final dir = Directory.systemTemp.createTempSync('canoa-task-');
    final repo = Directory('${dir.path}${Platform.pathSeparator}repo')..createSync();
    await _project(repo);
    final notes = File('${dir.path}${Platform.pathSeparator}notes.txt')..writeAsStringSync(_notes);
    FileSelectorPlatform.instance = _Picked(notes.path);
    final updatesFile = _updates == null ? null : File(_updates!);
    updatesFile?.writeAsStringSync('');
    final recorder = UpdateRecorder(write: (line) => updatesFile?.writeAsStringSync('$line\n', mode: FileMode.append));
    final store = SessionStore(includePreview: false, recorder: recorder);
    final preset = _real == null ? null : agentPresets.firstWhere((preset) => preset.label == _real);
    final agent = switch (preset) {
      null => AgentCommand(
        label: 'Scripted',
        program: 'node',
        args: ['$_appDir/integration_test/scripted_agent.mjs', '--state=${dir.path}/scripted.json'],
      ),
      // Claude through its preset's own adapter, but without the person's user-level settings and CLAUDE.md.
      final AgentCommand claude when claude.label == 'Claude' => AgentCommand(
        label: 'Claude without user settings',
        program: 'node',
        args: ['$_appDir/integration_test/without_user_settings.mjs', '--', claude.program, ...claude.args],
        requires: claude.requires,
        programPaths: claude.programPaths,
      ),
      final AgentCommand other => other,
    };
    final label = agent.label;
    final screen = GlobalKey();
    await tester.pumpWidget(RepaintBoundary(
      key: screen,
      child: CanoaApp(store: store, config: CanoaConfig(agents: agent == preset ? const [] : [agent])),
    ));
    await tester.pumpAndSettle();

    var shotNumber = 0;
    Future<void> shot(String name) async {
      await tester.pump();
      if (_shots == null) return;
      final boundary = tester.renderObject<RenderRepaintBoundary>(find.byKey(screen));
      final image = await boundary.toImage();
      final png = await image.toByteData(format: ui.ImageByteFormat.png);
      image.dispose();
      final file = File('$_shots/${(++shotNumber).toString().padLeft(2, '0')}-$name.png');
      file.writeAsBytesSync(png!.buffer.asUint8List());
      debugPrint('SHOT ${file.path}');
    }

    Future<void> until(bool Function() done, String what, {Duration within = const Duration(minutes: 3)}) async {
      final deadline = DateTime.now().add(within);
      while (!done()) {
        if (DateTime.now().isAfter(deadline)) {
          await shot('timed-out');
          fail('timed out waiting for $what');
        }
        await tester.pump(const Duration(milliseconds: 200));
      }
    }

    bool showing(Finder finder) => finder.evaluate().isNotEmpty;
    final composer = find.byWidgetPredicate((widget) => widget is TextField && widget.decoration?.hintText == 'Message agent');
    final question = find.ancestor(of: find.text('Refuse'), matching: find.byType(Card));
    final working = find.byTooltip('Stop');

    Future<void> runAgent() async {
      await tester.tap(find.byTooltip('Connect to verified agent'));
      await tester.pumpAndSettle();
      await tester.enterText(find.widgetWithText(TextField, 'Agent workspace (absolute path)'), repo.path);
      await until(() => showing(find.text('Run $label')), 'the $label preset to be offered', within: const Duration(seconds: 30));
      // The dialog has to have finished opening, or the tap lands where the button was on its way in.
      await tester.pumpAndSettle();
      await shot('connect-dialog');
      await tester.tap(find.text('Run $label'));
      await until(() => !showing(find.byType(AlertDialog)), 'the agent to start', within: const Duration(minutes: 4));
      await tester.pumpAndSettle();
    }

    bool cardSays(Pattern part) => showing(find.descendant(of: question, matching: find.textContaining(part)));
    String cardText() => [
      for (final element in find.descendant(of: question, matching: find.byWidgetPredicate((widget) => widget is Text || widget is SelectableText)).evaluate())
        switch (element.widget) {
          Text(:final data) || SelectableText(:final data) => data ?? '',
          _ => '',
        },
    ].join('\n');

    /// Drags the conversation to its end, as a person has to: it does not follow new entries by itself.
    Future<void> toEnd() async {
      final entries = find.byType(TranscriptEntry);
      if (!showing(entries)) return;
      final list = find.ancestor(of: entries.first, matching: find.byType(Scrollable)).first;
      for (var drags = 0; drags < 20; drags++) {
        final position = tester.state<ScrollableState>(list).position;
        if (position.pixels >= position.maxScrollExtent) break;
        // From the list's right edge, past the cards: an expanded card's own output would take a drag in the middle.
        await tester.dragFrom(tester.getTopRight(list) + const Offset(-12, 200), const Offset(0, -2000));
        await tester.pumpAndSettle();
      }
    }

    /// Sends [text], answers each question, and waits for the turn to end. A question whose card mentions [refuse] is refused, and so
    /// is one reaching outside the temporary directory, which the test then fails on. With [stop], the first
    /// question is not answered: Stop is pressed instead.
    Future<({int asked, int refused, int outside, bool stopped})> turn(String name, String text, {String? refuse, bool stop = false}) async {
      await tester.tap(composer);
      await tester.pump();
      await tester.enterText(composer, text);
      await tester.pump();
      if (tester.widget<TextField>(composer).controller?.text != text) fail('the composer did not take the message');
      await tester.tap(find.byTooltip('Send'));
      await tester.pump();
      await until(() => showing(working), 'the turn to start', within: const Duration(minutes: 2));
      var asked = 0, refused = 0, outside = 0;
      while (showing(working)) {
        if (showing(question)) {
          asked++;
          // The adapter's reason for a path outside the working directory, or any other place outside the temporary one.
          final away = cardSays('outside allowed working director') || _reachesOut(cardText(), dir.path);
          final refusing = away || (refuse != null && cardSays(refuse));
          if (asked == 1 || refusing || stop) await shot('$name-question$asked${away ? '-outside' : refusing ? '-refusing' : ''}');
          if (stop && !away) {
            await tester.tap(working);
            await until(() => !showing(working), 'the turn to stop', within: const Duration(minutes: 2));
            await tester.pumpAndSettle();
            await toEnd();
            await shot('$name-stopped');
            return (asked: asked, refused: refused, outside: outside, stopped: true);
          }
          if (away) {
            outside++;
          } else if (refusing) {
            refused++;
          }
          await tester.tap(find.descendant(of: question, matching: find.text(refusing ? 'Refuse' : 'Allow')));
          await tester.pump(const Duration(milliseconds: 300));
          continue;
        }
        await tester.pump(const Duration(milliseconds: 200));
      }
      await tester.pumpAndSettle();
      await toEnd();
      await shot('$name-done');
      return (asked: asked, refused: refused, outside: outside, stopped: false);
    }

    await shot('start');
    await runAgent();
    await shot('connected');

    final fixed = await turn('fix', _fix);
    debugPrint('STEP fix: asked ${fixed.asked}, outside the directory ${fixed.outside}');

    final stopped = await turn('rename', _rename, stop: true);
    final untouched = File('${repo.path}/src/calc.js').readAsStringSync().contains('addNumbers');
    debugPrint('STEP stop: asked ${stopped.asked}, stopped ${stopped.stopped}, calc.js still has the old name $untouched');
    final stoppedNotice = showing(find.text('Stopped.'));
    final continued = await turn('go-on', _goOn);
    debugPrint('STEP continue: asked ${continued.asked}, outside the directory ${continued.outside}');

    await tester.tap(find.byTooltip('Attach'));
    await until(() => showing(find.widgetWithText(InputChip, 'notes.txt')), 'the file to be attached', within: const Duration(seconds: 10));
    await shot('attached');
    final attached = await turn('notes', _followNotes, refuse: 'obsolete');
    debugPrint('STEP notes: asked ${attached.asked}, refused ${attached.refused}, outside the directory ${attached.outside}');
    final usage = store.selected?.usage.describe();
    // Tools that ran without a question, such as reads, are seen here: none may have reached outside either.
    final ranOutside = [
      for (final entry in store.selected?.toolEntries.values ?? const <ChatMessage>[])
        if (entry.tool!.status == ToolStatus.done && _reachesOut([entry.tool!.title, ...entry.tool!.locations].join('\n'), dir.path))
          entry,
    ].length;

    // Closed, then a new conversation with the same agent, from which the closed one is opened again.
    await tester.tap(find.byTooltip('Close conversation'));
    await tester.pumpAndSettle();
    await shot('closed');
    final closedAll = store.sessions.isEmpty;
    await runAgent();
    await tester.tap(find.byTooltip('Earlier conversations'));
    final listed = find.descendant(of: find.byType(EarlierConversations), matching: find.byType(ListTile));
    await until(() => showing(listed), 'earlier conversations to be listed', within: const Duration(minutes: 1));
    await shot('earlier-list');
    final listedCount = listed.evaluate().length;
    await tester.tap(listed.first);
    await until(
      () => !showing(find.byType(AlertDialog)) && showing(find.byType(TranscriptEntry)) && store.sessions.length == 2,
      'the earlier conversation to be shown',
      within: const Duration(minutes: 2),
    );
    await tester.pump(const Duration(seconds: 2));
    await tester.pumpAndSettle();
    await toEnd();
    await shot('reopened');
    final reopenedEntries = store.selected?.messages.length ?? 0;
    final reopenedUser = store.selected?.messages.where((message) => message.role == MessageRole.user).length ?? 0;
    debugPrint('STEP reopen: listed $listedCount, entries $reopenedEntries, user messages $reopenedUser');

    // Every conversation closed, which ends the agent.
    while (store.sessions.isNotEmpty) {
      await tester.tap(find.byTooltip('Close conversation'));
      await tester.pumpAndSettle();
    }

    for (final seen in recorder.seen) {
      debugPrint('KIND ${seen.kind} ${seen.rendering.name} x${seen.count} ${jsonEncode(seen.shape)}');
    }
    final askedOutside = fixed.outside + stopped.outside + continued.outside + attached.outside;
    debugPrint('USAGE ${usage ?? 'none said'}; outside the directory: $askedOutside asked and refused, $ranOutside ran');

    final (exitCode, passing, failing) = await _nodeTest(repo);
    final oldName = _mentioning(repo, 'addNumbers');
    final newName = _mentioning(repo, 'sum(');
    final maintainer = File('${repo.path}/README.md').readAsStringSync().contains('Maintainer: Ana Rivera');
    final kept = File('${repo.path}/obsolete.txt').existsSync();
    debugPrint('DISK node --test exit $exitCode passing $passing failing $failing; old name in ${oldName.length} files, new in ${newName.length}; '
        'maintainer $maintainer; obsolete.txt kept $kept');

    final closedAt = DateTime.now();
    while (true) {
      try {
        dir.deleteSync(recursive: true);
        debugPrint('DIRECTORY FREE AFTER ${DateTime.now().difference(closedAt).inMilliseconds} ms');
        break;
      } on FileSystemException {
        if (DateTime.now().difference(closedAt) > const Duration(seconds: 30)) fail('something the agent started is still running');
        await tester.pump(const Duration(milliseconds: 200));
      }
    }

    expect(fixed.asked, greaterThan(0), reason: 'the agent asks before it changes anything');
    expect(stopped.stopped, isTrue);
    expect(untouched, isTrue, reason: 'the stopped turn did not rename');
    expect(stoppedNotice, isTrue, reason: 'a stopped turn says so');
    expect(attached.refused, greaterThan(0), reason: 'the agent asked to delete obsolete.txt, and was refused');
    expect(kept, isTrue, reason: 'what was refused did not happen');
    expect(closedAll, isTrue);
    expect(listedCount, greaterThan(0));
    expect(reopenedUser, greaterThanOrEqualTo(3), reason: 'the earlier conversation came back with what was said');
    expect(recorder.unshown, isEmpty, reason: 'kinds no rendering of their own showed');
    expect(askedOutside, 0, reason: 'the agent asked to reach outside the temporary directory');
    expect(ranOutside, 0, reason: 'a tool reached outside the temporary directory');
    if (_real != null) {
      expect(exitCode, 0, reason: 'the tests pass');
      expect(passing, greaterThanOrEqualTo(2), reason: 'a second test was added');
      expect(oldName, isEmpty, reason: 'renamed everywhere');
      expect(newName, isNotEmpty);
      expect(maintainer, isTrue, reason: 'the attached notes were followed');
    }
  }, skip: !Platform.isWindows && !Platform.isLinux, timeout: const Timeout(Duration(minutes: 45)));
}
