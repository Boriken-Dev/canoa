// The timing sink writes only when CANOA_TIMING_LOG names a file, and CANOA_TEST_TIMING_AGENT measures a start through the
// app's own store and client: the host's source to run (Scripted is the stand-in `stdio_agent`; Claude, Codex or Antigravity
// are real, and spend a little, so they are asked for by name). CANOA_TEST_TIMING_RUNS is how many starts (default 5);
// CANOA_TEST_TIMING_PROMPT=0 opens a session and sends nothing (Antigravity).
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/chat/timing_log.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/embedded_host.dart';
import 'fixtures/examples.dart';

void main() {
  final log = Platform.environment['CANOA_TIMING_LOG'] ?? '';
  final agent = Platform.environment['CANOA_TEST_TIMING_AGENT'];

  test('without CANOA_TIMING_LOG a timed step runs and writes nothing', () async {
    final opened = <String>[];
    await IOOverrides.runZoned(() async {
      expect(await timed('step', () async => 7), 7);
      timingMark('step', 1);
    }, createFile: (path) {
      opened.add(path);
      return File(path);
    });
    expect(opened, isEmpty);
    await expectLater(timed('step', () async => throw StateError('kept')), throwsStateError);
  }, skip: log.isEmpty ? false : 'CANOA_TIMING_LOG is set');

  test('with CANOA_TIMING_LOG a timed step appends its name and milliseconds, also when it fails', () async {
    final file = File(log);
    final before = file.existsSync() ? file.readAsLinesSync().length : 0;
    expect(await timed('probe_ok', () async => 7), 7);
    await expectLater(timed('probe_failed', () async => throw StateError('kept')), throwsStateError);
    final added = file.readAsLinesSync().skip(before).toList();
    expect(added.length, 2);
    expect(added[0], matches(RegExp(r'^probe_ok, \d+$')));
    expect(added[1], matches(RegExp(r'^probe_failed, \d+$')));
  }, skip: log.isEmpty ? 'set CANOA_TIMING_LOG' : false);

  test('a start through the store, timed step by step', () async {
    final runs = int.tryParse(Platform.environment['CANOA_TEST_TIMING_RUNS'] ?? '') ?? 5;
    final prompt = Platform.environment['CANOA_TEST_TIMING_PROMPT'] != '0';
    final scripted = agent == 'Scripted';
    final config = scripted
        ? '[host.agents.Scripted]\nprogram = ${jsonEncode(exampleBinary('stdio_agent'))}\n'
            'args = ["--scripted", "--modes=default,bypassPermissions"]\nlisten = "127.0.0.1:0"\n'
        : agent == 'Antigravity'
            ? '[host.agents.Antigravity]\nprogram = "agy"\nargs = ["acp"]\nlisten = "127.0.0.1:0"\n'
            : '[host.agents.$agent]\nharness = "${agent!.toLowerCase()}"\nlisten = "127.0.0.1:0"\n';
    final host = await EmbeddedHost.start(config: config);
    addTearDown(host.stop);
    for (var run = 0; run < runs; run++) {
      timingMark('run', run);
      final dir = Directory.systemTemp.createTempSync('canoa-timing-');
      final store = SessionStore();
      store.starter = (draft) async {
        final client = await timed('link_and_initialize', () => host.agent(agent!, decider: store.askPermission));
        final opened = await client.openSession(cwd: dir.path);
        store.bindDraft(draft, client, opened, cwd: dir.path);
        await store.applyHostProfile(draft);
      };
      final session = store.createDraft(source: AgentSource.hosted(agent!, local: true), cwd: dir.path);
      if (prompt) {
        final refusing = Timer.periodic(const Duration(milliseconds: 200), (_) {
          if (session.pendingPermissions.isNotEmpty) store.answerPermission(session, allow: false);
        });
        await store.sendText('Reply with only the word ok.');
        final until = DateTime.now().add(const Duration(minutes: 3));
        while (session.status == SessionStatus.working && DateTime.now().isBefore(until)) {
          await Future<void>.delayed(const Duration(milliseconds: 100));
        }
        refusing.cancel();
      } else {
        final starter = store.starter!;
        await timed('start', () => starter(session));
      }
      await store.close(session);
      final closed = DateTime.now();
      while (true) {
        try {
          dir.deleteSync(recursive: true);
          break;
        } on FileSystemException {
          if (DateTime.now().difference(closed) > const Duration(seconds: 30)) break;
          await Future<void>.delayed(const Duration(milliseconds: 200));
        }
      }
    }
  }, skip: agent == null ? 'set CANOA_TEST_TIMING_AGENT' : (log.isEmpty ? 'set CANOA_TIMING_LOG' : embeddedHostMissing()),
      timeout: const Timeout(Duration(minutes: 30)));
}
