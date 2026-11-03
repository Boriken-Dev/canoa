// Canoa's chat reaching a command-line tool that `canoa-host` presents as an ACP agent, through the app's own connection
// code: connectVerifiedAcp, the peer proof, its sealed channel and VerifiedAgent, against the real host binary, and the
// session store showing what the host says about the tool.
//
// CANOA_TEST_HOST names a built canoa-host with its examples beside it, as host_test.dart says. The entry is Pi's, from
// examples/config/pi.cfg, with the jsonl_tool fixture as its program. Without CANOA_TEST_HOST the tests are skipped.
//
// A real tool instead: CANOA_TEST_TOOL_CONFIG names a Canoa.cfg and CANOA_TEST_TOOL_ENTRY its [host.tools] entry; the
// host's environment is this test's. CANOA_TEST_TOOL_PROMPT is the one prompt (a short default otherwise), and
// CANOA_TEST_TOOL_ANSWER `allow` or `refuse` answers any question it asks (refuse by default). It prints the shape of
// each update, never its text.
@Timeout(Duration(minutes: 20))
library;

import 'dart:convert';
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/auth/peer_proof.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/node_peer_bridge.dart';

final String? _hostBinary = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path => File(path).absolute.path,
  null => null,
};

final String? _realConfig = Platform.environment['CANOA_TEST_TOOL_CONFIG'];

/// The fixture shaped like Pi's command line, built beside the host.
String _fixture() {
  final examples = File(_hostBinary!).parent.uri.resolve('examples/').toFilePath();
  return '${examples}jsonl_tool${Platform.isWindows ? '.exe' : ''}';
}

/// examples/config/pi.cfg with the fixture as its program.
Future<String> _piConfig() async {
  final shipped = await File('../examples/config/pi.cfg').readAsString();
  return [
    for (final line in const LineSplitter().convert(shipped))
      if (line.startsWith('program = '))
        'program = ${jsonEncode(_fixture())}'
      else if (line.startsWith('args = '))
        'args = ["--mode", "rpc"]'
      else
        line,
  ].join('\n');
}

void main() {
  final skip = _hostBinary == null ? 'CANOA_TEST_HOST names a built canoa-host' : null;
  final entry = _realConfig == null ? 'pi' : Platform.environment['CANOA_TEST_TOOL_ENTRY'] ?? 'pi';
  late Directory dir;
  // Nullable, so a setUpAll that fails partway still has what it started ended.
  Process? host;
  late String url;
  late String peer;
  NodePeerBridge? browser;

  Future<ProcessResult> command(List<String> args) =>
      Process.run(_hostBinary!, args, environment: {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home'});

  setUpAll(() async {
    if (skip != null) return;
    dir = await Directory.systemTemp.createTemp('canoa-host-tool-test');
    await Directory('${dir.path}/work').create();
    final config = File('${dir.path}/Canoa.cfg');
    final real = _realConfig;
    await config.writeAsString(real == null ? await _piConfig() : await File(real).readAsString());
    final started = await Process.start(
      _hostBinary!,
      // The app's browser proof is WebCrypto only, and a new home would prove by Noise.
      ['serve', '--method', 'webcrypto', '--config', config.path],
      environment: {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home'},
    );
    host = started;
    started.stderr.transform(utf8.decoder).listen(stderr.write);
    final lines = started.stdout.transform(utf8.decoder).transform(const LineSplitter());
    await for (final line in lines.timeout(const Duration(seconds: 30))) {
      final listed = jsonDecode(line) as Map<String, dynamic>;
      if (listed['name'] == entry) {
        if (_realConfig == null) expect(listed['kind'], 'tool');
        url = listed['url'] as String;
        peer = listed['peer'] as String;
        break;
      }
    }
    browser = await NodePeerBridge.start();
  });

  tearDownAll(() async {
    if (skip != null) return;
    browser?.kill();
    final started = host;
    if (started == null) return;
    started.kill();
    await started.exitCode;
    // The system ends the host's tools with it, a moment later on Windows: until then one holds its directory.
    for (var tries = 0;; tries++) {
      try {
        await dir.delete(recursive: true);
        break;
      } on FileSystemException {
        if (tries >= 50) rethrow;
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
    }
  });

  Future<VerifiedAgent> connect({AcpDecider? decider}) => connectVerifiedAcp(
    chatEndpoints(webSocket: url),
    expectedPeerId: peer,
    verifyPeer: (channel, expected) => provePeer(browser!, channel, expected),
    decider: decider,
  );

  Future<void> pair() async {
    await expectLater(connect(), throwsA(anything), reason: 'an unpaired key waits');
    final accepted = await command(['accept', await browser!.localId()]);
    expect(accepted.exitCode, 0, reason: accepted.stderr as String);
  }

  test('the chat reaches Pi\'s entry, shows what it does unasked, and a question it asks reaches the person', () async {
    await pair();
    final asked = <AcpPermissionRequest>[];
    var answer = const AcpAllow() as AcpDecision;
    final client = await connect(decider: (request) async {
      asked.add(request);
      return answer;
    });
    final store = SessionStore(includePreview: false);
    try {
      final opened = await client.openSession(cwd: '${dir.path}/work');
      final local = store.addProviderSession(title: 'pi', agent: 'pi', peerId: peer, remoteSessionId: opened.id);
      store.applyOpened(local, opened);
      final notices = local.messages.where((message) => message.role == MessageRole.notice).toList();
      expect(notices, hasLength(1));
      expect(notices.single.text, contains('rm -rf, sudo'));

      final kinds = <String>[];
      final text = StringBuffer();
      final listening = client.updates.listen((update) {
        kinds.add(update.kind);
        if (update.kind == 'agent_message_chunk') text.write(update.text ?? '');
      });
      expect(await client.prompt(opened.id, 'hello'), isNull);
      await Future<void>.delayed(const Duration(milliseconds: 50));
      expect(text.toString(), 'echo: hello');
      expect(kinds, contains('agent_thought_chunk'));

      text.clear();
      await client.prompt(opened.id, 'run rm -rf build');
      await Future<void>.delayed(const Duration(milliseconds: 50));
      expect(asked.single.title, contains('rm -rf build'));
      expect(text.toString(), 'allowed');
      expect(kinds, containsAll(['tool_call', 'tool_call_update']));

      answer = const AcpDeny('not today');
      text.clear();
      await client.prompt(opened.id, 'run sudo reboot');
      await Future<void>.delayed(const Duration(milliseconds: 50));
      expect(asked, hasLength(2));
      expect(text.toString(), 'refused');
      await listening.cancel();
    } finally {
      store.dispose();
      await client.close();
    }
  }, skip: skip ?? (_realConfig != null ? 'CANOA_TEST_TOOL_CONFIG runs the real tool instead' : null));

  test('a real tool runs a turn from its one entry', () async {
    await pair();
    final asked = <String>[];
    final allow = Platform.environment['CANOA_TEST_TOOL_ANSWER'] == 'allow';
    final client = await connect(decider: (request) async {
      asked.add(request.title ?? '?');
      return allow ? const AcpAllow() : const AcpDeny('the test refuses');
    });
    try {
      final opened = await client.openSession(cwd: '${dir.path}/work');
      final meta = opened.raw['_meta'];
      final notices = meta is Map && meta[SessionStore.hostNoticesKey] is List ? meta[SessionStore.hostNoticesKey] as List : const [];
      stdout.writeln('notices: ${notices.length}');
      final shapes = <String, int>{};
      final text = StringBuffer();
      final listening = client.updates.listen((update) {
        shapes[update.kind] = (shapes[update.kind] ?? 0) + 1;
        if (update.kind == 'agent_message_chunk') text.write(update.text ?? '');
      });
      final started = DateTime.now();
      final prompt = Platform.environment['CANOA_TEST_TOOL_PROMPT'] ?? 'Reply with the single word ready.';
      await client.prompt(opened.id, prompt);
      await Future<void>.delayed(const Duration(milliseconds: 200));
      await listening.cancel();
      stdout.writeln('turn: ${DateTime.now().difference(started).inSeconds} s, updates $shapes, '
          'asked ${asked.length} (${allow ? 'allowed' : 'refused'}), text ${text.length} characters');
      expect(text.toString().trim(), isNotEmpty, reason: 'the tool said something');
    } finally {
      await client.close();
    }
  }, skip: skip ?? (_realConfig == null ? 'CANOA_TEST_TOOL_CONFIG names a real tool' : null));
}
