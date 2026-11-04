// A profile through the real launch path of a harness installed here: the preset's adapter is started, its session
// opened with what the profile gives it, and the report printed. No prompt is sent, so nothing is spent. Runs only when
// asked: CANOA_TEST_PROFILE_HARNESS=Codex (or Pi), with CANOA_TEST_NODE naming node by its absolute path.
@TestOn('vm')
library;

import 'dart:io';

import 'package:canoa/acp/agent_commands.dart';
import 'package:canoa/acp/local_agent.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/apply.dart';
import 'package:canoa/profile/launch.dart';
import 'package:canoa/profile/profile_home_io.dart';
import 'package:canoa/profile/profile_layers.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';

String slash(String path) => path.replaceAll(r'\', '/');

/// What each harness is expected to take, by key: Codex all three, Pi none of them.
const expected = {
  'Codex': {'context': ItemState.applied, 'skills.canoa-marker': ItemState.applied, 'mcp.marker': ItemState.applied},
  'Pi': {
    'context': ItemState.unsupported,
    'skills.canoa-marker': ItemState.unsupported,
    'mcp.marker': ItemState.unsupported,
  },
};

void main() {
  final harness = Platform.environment['CANOA_TEST_PROFILE_HARNESS'];
  final node = Platform.environment['CANOA_TEST_NODE'];
  test('a harness installed here opens a session with the profile, and shows what applied', () async {
    final temp = Directory.systemTemp.createTempSync('canoa-profile-harness-');
    final work = Directory('${temp.path}/work')..createSync();
    final server = File('${temp.path}/mcp/marker_mcp.mjs')
      ..parent.createSync()
      ..writeAsStringSync(File('test/fixtures/marker_mcp.mjs').readAsStringSync());
    File('${temp.path}/skill/SKILL.md')
      ..parent.createSync()
      ..writeAsStringSync('---\nname: canoa-marker\ndescription: Gives the skill word.\n---\nThe skill word is TOUCAN.\n');
    final config = CanoaConfig.parse('''
[profile]
context = { text = "The context word is PELICAN." }
[profile.env.CANOA_PROFILE_PROBE]
value = "set"
[profile.skills.canoa-marker]
path = "${slash(temp.path)}/skill"
[profile.mcp.marker]
type = "stdio"
command = "${slash(node!)}"
args = ["${slash(server.path)}"]
''');
    final home = IoProfileHome('${temp.path}/config');
    final layers = await profileLayers(overlays: config.overlays, user: config.profile, agents: config.agentProfiles,
        agent: harness!, home: home, workingDirectory: work.path);
    final prepared = await prepareProfile(layers, home: home, lookup: environmentVariable, findProgram: findProgram);
    final command = agentPresets.firstWhere((preset) => preset.label == harness);
    final store = SessionStore(includePreview: false);
    final started = await startProfiled(command, cwd: work.path, prepared: prepared, mcpOn: {'marker'}, files: home,
        decider: store.askPermission);
    final client = started.client;
    stdout.writeln('AGENT ${client.identity.name} ${client.identity.version} PROTOCOL ${client.protocolVersion} '
        'HARNESS ${started.harness}');
    final opened = await client.openSession(cwd: work.path, setup: started.launch.setup);
    final session = store.addProviderSession(title: harness, agent: harness, peerId: client.peerId,
        remoteSessionId: opened.id, cwd: work.path);
    store.applyOpened(session, opened);
    store.attachVerifiedClient(session.id, client);
    await store.applyProfile(session, started.launch, prepared, harness: started.harness);
    for (final item in session.profile!.items) {
      stdout.writeln('ITEM ${item.key} ${item.state.label}: ${item.how}');
    }
    stdout.writeln('SUMMARY ${session.messages.last.text}');
    await client.close();
    final closed = DateTime.now();
    while (true) {
      try {
        temp.deleteSync(recursive: true);
        break;
      } on FileSystemException {
        if (DateTime.now().difference(closed) > const Duration(seconds: 30)) rethrow;
        await Future<void>.delayed(const Duration(milliseconds: 200));
      }
    }
    final items = {for (final item in session.profile!.items) item.key: item.state};
    expect(items['env.CANOA_PROFILE_PROBE'], ItemState.applied);
    expected[harness]!.forEach((key, state) => expect(items[key], state, reason: key));
  }, skip: harness == null || node == null ? 'set CANOA_TEST_PROFILE_HARNESS and CANOA_TEST_NODE' : false,
      timeout: const Timeout(Duration(minutes: 6)));
}
