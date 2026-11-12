// Moving a session to another source: what the host reports is read with every field checked, the loss report reads as
// sentences, a history shows as a conversation, and, against the real host library with Claude's and Codex's command lines
// stood in for, a session of Claude moves to Codex and shows what was said and what did not carry.
//
//   CANOA_TEST_LIBRARY=<canoa_ffi> CANOA_TEST_HOST=<canoa> CLAUDE_CONFIG_DIR=<empty dir> CODEX_HOME=<empty dir> \
//     flutter test test/session_move_test.dart
//
// The two directories keep the stand-ins' session files away from the person's own.
import 'dart:io';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/history/session_move.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp/coqui_acp.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/embedded_host.dart';
import 'fixtures/examples.dart';

const _history = '{"format":"coqui-acp-history","version":1,"id":"0f8fad5b-d9cb-469f-a165-70867728950e","origin":{"harness":"claude"}}\n'
    '{"method":"session/prompt","params":{"prompt":[{"type":"text","text":"write it"}]}}\n'
    '{"method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"hmm"}}}}\n'
    '{"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Done: "}}}}\n'
    '{"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"write it"}}}}\n'
    '{"method":"history/turn_end","params":{"stopReason":"end_turn"}}\n';

void main() {
  test('a report from the host is read with every field checked', () {
    final report = MoveReport.parse({
      'session': 'abc',
      'entry': 'Codex',
      'cwd': '/work',
      'replayed': false,
      'compacted': true,
      'losses': [
        {'kind': 'thinking', 'count': 2},
        {'kind': 'tool_as_note', 'count': 1, 'detail': 'Write'},
        {'kind': 7, 'count': 1},
        {'kind': 'item_cut', 'count': 'many'},
        'nonsense',
      ],
    })!;
    expect(report.session, 'abc');
    expect(report.compacted, isTrue);
    expect(report.losses.map((loss) => loss.kind), ['thinking', 'tool_as_note']);
    expect(report.loss('tool_as_note')!.detail, 'Write');
    expect(MoveReport.parse({'session': '', 'entry': 'x', 'cwd': '/'}), isNull);
    expect(MoveReport.parse({'entry': 'x', 'cwd': '/'}), isNull);
    expect(MoveReport.parse('nope'), isNull);
  });

  test('what did not carry is said in a sentence, and an unknown kind in the host\'s own word', () {
    expect(lossText(const MoveLoss('thinking', 3, null)), contains('3 thoughts left out'));
    expect(lossText(const MoveLoss('tool_as_note', 1, null)), contains('1 tool call carried as a note'));
    expect(lossText(const MoveLoss('replayed_as_prompt', 1, null)), contains('first message'));
    expect(lossText(const MoveLoss('something_new', 4, 'why')), 'something_new (4). why');
    const clean = MoveReport(session: 's', entry: 'Codex', cwd: '/w', replayed: false, compacted: false, losses: []);
    expect(moveSummary(clean, from: 'Claude'), allOf(contains('Moved from Claude to Codex'), contains('Everything carried.')));
    const lossy = MoveReport(
        session: 's', entry: 'Grok', cwd: '/w', replayed: true, compacted: false, losses: [MoveLoss('thinking', 1, null)]);
    expect(moveSummary(lossy, from: 'Claude'), allOf(contains('Did not carry:'), contains('1 thought left out'), contains('first message')));
  });

  test('a history shows as the conversation: prompts and the agent\'s messages joined, nothing else', () {
    final messages = historyMessages(_history);
    expect([for (final each in messages) (each.role, each.text)], [(MessageRole.user, 'write it'), (MessageRole.agent, 'Done: write it')]);
    expect(historyMessages('{}\nnot json\n[]\n'), isEmpty);
    expect(historyMessages(_history, limit: 1), hasLength(1), reason: 'bounded');
  });

  final missing = embeddedHostMissing() ?? exampleMissing('harness_standin');
  test('a session of Claude moves to Codex and shows what was said and what did not carry', () async {
    final dir = Directory.systemTemp.createTempSync('canoa-move-');
    addTearDown(() => dir.deleteSync(recursive: true));
    final bin = Directory('${dir.path}/bin')..createSync();
    final work = Directory('${dir.path}/work')..createSync();
    String standin(String name) {
      final path = '${bin.path}/$name${Platform.isWindows ? '.exe' : ''}';
      File(exampleBinary('harness_standin')!).copySync(path);
      return path.replaceAll(r'\', '/');
    }

    final host = await EmbeddedHost.start(
      dir: Directory('${dir.path}/host')..createSync(),
      config: '[host.agents.Claude]\nharness = "claude"\nexecutable = "${standin('claude')}"\nlisten = "127.0.0.1:0"\n'
          '[host.agents.Codex]\nharness = "codex"\nexecutable = "${standin('codex')}"\nlisten = "127.0.0.1:0"\n',
    );
    addTearDown(host.supervisor.stop);
    final control = await HostControl(host.supervisor).ready();
    final store = SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));
    final claude = await host.agent('Claude', decider: (_, _) async => const AcpAllow());
    final opened = await claude.openSession(cwd: work.path);
    final first = store.addProviderSession(
        title: 'Fix it', agent: 'Claude', peerId: claude.peerId, remoteSessionId: opened.id, cwd: work.path);
    store.applyOpened(first, opened);
    store.attachVerifiedClient(first.id, claude);
    expect(store.canMove(first), isTrue);
    await store.sendText('write it');
    final deadline = DateTime.now().add(const Duration(seconds: 30));
    while (!first.messages.any((message) => message.text.contains('Done: write it'))) {
      if (DateTime.now().isAfter(deadline)) fail('Claude did not answer');
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    // The host keeps the turn once the answer has gone by.
    while (true) {
      try {
        await control.call('history/export', {'entry': 'Claude', 'session': opened.id});
        break;
      } on Object {
        if (DateTime.now().isAfter(deadline)) fail('the host kept no history');
        await Future<void>.delayed(const Duration(milliseconds: 50));
      }
    }

    final moved = await moveSession(
      store: store,
      control: control,
      session: first,
      to: 'Codex',
      connect: (entry) => host.agent(entry, decider: (_, _) async => const AcpAllow()),
      keep: (client) => addTearDown(client.close),
    );
    expect(moved.agent, 'Codex');
    expect(moved.remoteSessionId, isNot(opened.id));
    expect(store.sessions, containsAll([first, moved]), reason: 'the old session stays');
    final said = moved.messages.where((message) => message.role != MessageRole.notice).map((message) => message.text).join('\n');
    expect(said, allOf(contains('write it'), contains('Done: write it')), reason: 'the history it carries');
    final notice = moved.messages.where((message) => message.role == MessageRole.notice).map((message) => message.text).join('\n');
    expect(notice, contains('Moved from Claude to Codex'));
    expect(notice, anyOf(contains('Everything carried.'), contains('Did not carry:')), reason: 'the loss report');
    expect(first.messages.where((message) => message.text.contains('Moved from')), isEmpty);
  }, skip: missing, timeout: const Timeout(Duration(minutes: 3)));
}
