// Claude and Codex with no Node anywhere on PATH: the host this app carries serves both as its own sources, and a session of
// each runs over the in-process link, its permission asked in the conversation and answered there. The tools are
// crates/canoa-host's harness stand-in, copied as `claude` and `codex` into a directory first on PATH, so no model is reached
// and nothing is spent.
//
//   CANOA_TEST_STANDIN_BIN=<dir holding claude(.exe) and codex(.exe)> STANDIN_RECORD=<file> PATH=<that dir>:$PATH \
//     flutter test integration_test/native_agents_test.dart -d windows   (or -d linux)
//
// The test refuses to run when node or npx is on PATH, or when claude or codex is found anywhere but that directory.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/local_agent.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:canoa_native/canoa_native.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';

final String? _bin = Platform.environment['CANOA_TEST_STANDIN_BIN'];
final String? _record = Platform.environment['STANDIN_RECORD'];

List<Map<String, dynamic>> _records() {
  final file = File(_record!);
  if (!file.existsSync()) return const [];
  return [for (final line in file.readAsLinesSync()) if (line.isNotEmpty) jsonDecode(line) as Map<String, dynamic>];
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('with no Node, Claude and Codex are offered and each runs a turn through the host the app carries', (tester) async {
    for (final program in ['node', 'npx']) {
      expect(await findProgram(program), isNull, reason: '$program is on PATH: this test is for a computer without Node');
    }
    final bin = Directory(_bin!).resolveSymbolicLinksSync();
    for (final program in ['claude', 'codex']) {
      final found = await findProgram(program);
      expect(found != null && File(found).parent.resolveSymbolicLinksSync() == bin, isTrue,
          reason: '$program must be the stand-in, never the real one: found $found');
    }
    final dir = Directory.systemTemp.createTempSync('canoa-native-');
    final work = Directory('${dir.path}${Platform.pathSeparator}work')..createSync();
    File('${dir.path}/skill/SKILL.md')
      ..parent.createSync()
      ..writeAsStringSync('---\nname: canoa-marker\ndescription: A stand-in skill.\n---\nNothing to do.\n');
    final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
    final cfg = File('${dir.path}${Platform.pathSeparator}canoa.cfg')..writeAsStringSync('');
    final host = HostSupervisor(LibraryHostTool(CanoaNative.open(), home: '${dir.path}${Platform.pathSeparator}home'));
    await host.apply(configPath: cfg.path);
    addTearDown(host.stop);
    await tester.pumpWidget(CanoaApp(store: store, config: const CanoaConfig(), host: host, pickFolder: () async => work.path));
    await tester.pumpAndSettle();

    bool showing(Finder finder) => finder.evaluate().isNotEmpty;
    Future<void> until(bool Function() done, String what, {Duration within = const Duration(minutes: 2)}) async {
      final deadline = DateTime.now().add(within);
      while (!done()) {
        if (DateTime.now().isAfter(deadline)) fail('timed out waiting for $what');
        await tester.pump(const Duration(milliseconds: 200));
      }
    }

    final offered = <String, bool>{};
    Future<void> turn(String label, String text) async {
      await tester.tap(find.byTooltip('New session'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('new-session-source')));
      await until(() => showing(find.byKey(Key('source $label'))), 'the $label source', within: const Duration(seconds: 30));
      await tester.pumpAndSettle();
      final source = find.byKey(Key('source $label')).last;
      offered[label] = !showing(find.descendant(of: source, matching: find.textContaining('Needs')));
      await tester.tap(source);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('new-session-folder')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('new-session-browse')));
      await tester.pumpAndSettle();
      final composer = find.descendant(of: find.byKey(const Key('composer')), matching: find.byType(TextField));
      await tester.tap(composer);
      await tester.enterText(composer, text);
      await tester.tap(find.byTooltip('Send'));
      await tester.pump();
      final allow = find.descendant(
          of: find.byType(PermissionCard),
          matching: find.byWidgetPredicate((widget) => widget is PermissionChoice && widget.option.kind == 'allow_once'));
      await until(() => showing(allow), 'the $label question');
      await tester.tap(allow.first);
      await until(() => store.selected?.messages.any((message) => message.text.contains('Done: $text')) ?? false,
          'the $label answer');
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Close conversation'));
      await tester.pumpAndSettle();
    }

    await turn('Claude', 'write it');
    await turn('Codex', 'run it');

    debugPrint('OFFERED $offered');
    expect(offered, {'Claude': true, 'Codex': true}, reason: 'both ready with no Node');
    expect(File('${work.path}/standin.txt').existsSync(), isTrue, reason: 'what was allowed was done');
    expect(_records().where((entry) => entry['harness'] == 'claude'), isNotEmpty, reason: 'the stand-in claude ran');

    await host.stop();
    final closedAt = DateTime.now();
    while (true) {
      try {
        dir.deleteSync(recursive: true);
        break;
      } on FileSystemException {
        if (DateTime.now().difference(closedAt) > const Duration(seconds: 30)) fail('something the agent started is still running');
        await tester.pump(const Duration(milliseconds: 200));
      }
    }
  }, skip: _bin == null || _record == null || (!Platform.isWindows && !Platform.isLinux),
      timeout: const Timeout(Duration(minutes: 10)));
}
