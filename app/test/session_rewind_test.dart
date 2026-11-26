// Going back to before one of the person's messages against the real host library with Claude's command line stood in for: a
// session of two turns goes back to before the second, the new session holds the first and not the second, and the one it came
// from is as it was.
//
//   CANOA_TEST_LIBRARY=<canoa_ffi> CANOA_TEST_HOST=<canoa> CLAUDE_CONFIG_DIR=<empty dir> \
//     flutter test test/session_rewind_test.dart
//
// The directory keeps the stand-in's session files away from the person's own.
import 'dart:io';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/history/session_move.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp/coqui_acp.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/embedded_host.dart';
import 'fixtures/examples.dart';

void main() {
  // The stand-in is given a login of its own in the folder the test is pointed at; with none named the person's own would be read.
  final home = Platform.environment['CLAUDE_CONFIG_DIR'];
  final missing =
      embeddedHostMissing() ??
      exampleMissing('harness_standin') ??
      (home == null || home.isEmpty
          ? 'CLAUDE_CONFIG_DIR names a folder of its own'
          : null);
  test(
    'a session goes back to before its second message, in a new session of the same agent',
    () async {
      File('$home/.credentials.json').writeAsStringSync('{"fixture":true}');
      final dir = Directory.systemTemp.createTempSync('canoa-rewind-');
      addTearDown(() => dir.deleteSync(recursive: true));
      final bin = Directory('${dir.path}/bin')..createSync();
      final work = Directory('${dir.path}/work')..createSync();
      final program = '${bin.path}/claude${Platform.isWindows ? '.exe' : ''}';
      File(exampleBinary('harness_standin')!).copySync(program);

      final host = await EmbeddedHost.start(
        dir: Directory('${dir.path}/host')..createSync(),
        config:
            '[host.agents.Claude]\nharness = "claude"\nexecutable = "${program.replaceAll(r'\', '/')}"\nlisten = "127.0.0.1:0"\n',
      );
      addTearDown(host.supervisor.stop);
      final control = await HostControl(host.supervisor).ready();
      final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
      final claude = await host.agent(
        'Claude',
        decider: (_, _) async => const AcpAllow(),
      );
      final opened = await claude.openSession(cwd: work.path);
      final first = store.addProviderSession(
        title: 'Fix it',
        agent: 'Claude',
        peerId: claude.peerId,
        remoteSessionId: opened.id,
        cwd: work.path,
      );
      store.applyOpened(first, opened);
      store.attachVerifiedClient(first.id, claude);

      final deadline = DateTime.now().add(const Duration(seconds: 30));
      Future<void> turn(String text) async {
        await store.sendText(text);
        while (!first.messages.any(
          (message) => message.text.contains('Done: $text'),
        )) {
          if (DateTime.now().isAfter(deadline))
            fail('Claude did not answer $text');
          await Future<void>.delayed(const Duration(milliseconds: 50));
        }
        // The host keeps the turn once the answer has gone by.
        while (true) {
          try {
            final kept = await control.call('history/export', {
              'entry': 'Claude',
              'session': opened.id,
            }) as Map;
            if ((kept['text'] as String).contains('Done: $text')) break;
          } on Object {
            // Not kept yet.
          }
          if (DateTime.now().isAfter(deadline))
            fail('the host kept no history of $text');
          await Future<void>.delayed(const Duration(milliseconds: 50));
        }
      }

      await turn('first question');
      await turn('second question');

      final back = await moveSession(
        store: store,
        control: control,
        session: first,
        to: 'Claude',
        before: 1,
        expect: 'second question',
        connect: (entry) =>
            host.agent(entry, decider: (_, _) async => const AcpAllow()),
        keep: (client) => addTearDown(client.close),
        notes: (_) => ['went back'],
      );
      expect(back.agent, 'Claude');
      expect(back.remoteSessionId, isNot(opened.id));
      expect(
        store.sessions,
        containsAll([first, back]),
        reason: 'the session it came from stays',
      );
      final said = back.messages
          .where((message) => message.role != MessageRole.notice)
          .map((message) => message.text)
          .join('\n');
      expect(
        said,
        allOf(contains('first question'), contains('Done: first question')),
      );
      expect(
        said,
        isNot(contains('second question')),
        reason: 'what came after is not carried',
      );
      expect(
        back.messages
            .where((message) => message.role == MessageRole.notice)
            .map((message) => message.text),
        ['went back'],
      );
      expect(
        first.messages.where(
          (message) => message.text.contains('second question'),
        ),
        isNotEmpty,
      );

      // A message that is not the one at that turn carries nothing.
      await expectLater(
        moveSession(
          store: store,
          control: control,
          session: first,
          to: 'Claude',
          before: 1,
          expect: 'first question',
          connect: (entry) =>
              host.agent(entry, decider: (_, _) async => const AcpAllow()),
          keep: (client) => addTearDown(client.close),
        ),
        throwsA(anything),
      );
      expect(store.sessions, hasLength(2));
    },
    skip: missing,
    timeout: const Timeout(Duration(minutes: 3)),
  );
}
