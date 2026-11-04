// A profile through the app's own launch path, to a child that answers `initialize` as each real adapter does: what reaches
// its environment and its session request, what the session's report says, and the files written for it gone with it.
@TestOn('vm')
library;

import 'dart:convert';
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

late Directory temp;
final running = <ProfiledAgent>[];

String slash(String path) => path.replaceAll(r'\', '/');

/// A child named [label], answering as [kind], recording into the returned file.
(AgentCommand, File) agent(String label, String kind) {
  final record = File('${temp.path}/$kind.jsonl');
  return (
    AgentCommand(label: label, program: 'dart', args: [
      '${Directory.current.path}/test/fixtures/profile_agent.dart', '--as=$kind', '--record=${record.path}',
      '--env=PROFILE_TOKEN,NOT_IN_PROFILE',
    ]),
    record,
  );
}

Future<(PreparedProfile, IoProfileHome)> prepare(String label) async {
  File('${temp.path}/skill/SKILL.md')
    ..parent.createSync(recursive: true)
    ..writeAsStringSync('---\nname: marker\n---\nSay the skill word.\n');
  final config = CanoaConfig.parse('''
[profile]
model = "opus"
effort = "high"
context = { text = "Say the context word." }
[profile.env.PROFILE_TOKEN]
value = "t"
[profile.skills.marker]
path = "${slash(temp.path)}/skill"
[profile.mcp.marker]
type = "stdio"
command = "dart"
args = ["marker.dart"]
''');
  final home = IoProfileHome('${temp.path}/config');
  final layers = await profileLayers(overlays: config.overlays, user: config.profile, agents: config.agentProfiles,
      agent: label, home: home);
  return (await prepareProfile(layers, home: home, lookup: environmentVariable, findProgram: findProgram), home);
}

Future<(ChatSession, Map<String, dynamic>, ProfiledAgent)> launch(String label, String kind) async {
  final (command, record) = agent(label, kind);
  final (prepared, home) = await prepare(label);
  final store = SessionStore(includePreview: false);
  final started = await startProfiled(command, cwd: temp.path, prepared: prepared, mcpOn: {'marker'}, files: home,
      decider: store.askPermission);
  running.add(started);
  final opened = await started.client.openSession(cwd: temp.path, setup: started.launch.setup);
  final session = store.addProviderSession(title: label, agent: label, peerId: started.client.peerId,
      remoteSessionId: opened.id, cwd: temp.path);
  store.applyOpened(session, opened);
  store.attachVerifiedClient(session.id, started.client);
  await store.applyProfile(session, started.launch, prepared, harness: started.harness);
  final sent = jsonDecode(record.readAsLinesSync().first) as Map<String, dynamic>;
  return (session, sent, started);
}

ItemState state(ChatSession session, String key) => session.profile!.items.firstWhere((item) => item.key == key).state;

void main() {
  setUp(() => temp = Directory.systemTemp.createTempSync('canoa-launch-'));
  tearDown(() async {
    for (final started in running) {
      await started.client.close();
    }
    running.clear();
    // Windows holds a directory while a process that worked in it is still leaving.
    final until = DateTime.now().add(const Duration(seconds: 30));
    while (true) {
      try {
        temp.deleteSync(recursive: true);
        return;
      } on FileSystemException {
        if (DateTime.now().isAfter(until)) rethrow;
        await Future<void>.delayed(const Duration(milliseconds: 200));
      }
    }
  });

  test("Claude's adapter is given all of it, each its own way, and its files go when it does", () async {
    final (session, sent, started) = await launch('Claude', 'claude');
    final params = sent['params'] as Map;
    expect(sent['env'], {'PROFILE_TOKEN': true, 'NOT_IN_PROFILE': false});
    expect(params['systemPrompt'], {'mode': 'append', 'content': [{'type': 'text', 'text': 'Say the context word.'}]});
    final server = (params['mcpServers'] as List).single as Map;
    expect(server.containsKey('type'), isFalse, reason: "v1's stdio entry has no type");
    expect(server['name'], 'marker');
    expect(File(server['command'] as String).existsSync(), isTrue, reason: 'an absolute path to the program');
    final plugin = ((((params['_meta'] as Map)['claudeCode'] as Map)['options'] as Map)['plugins'] as List).single as Map;
    final pluginDir = plugin['path'] as String;
    expect(File('$pluginDir/skills/marker/SKILL.md').existsSync(), isTrue);
    expect(started.harness, 'claude-code');
    for (final key in ['env.PROFILE_TOKEN', 'context', 'skills.marker', 'mcp.marker', 'model', 'effort']) {
      expect(state(session, key), ItemState.applied, reason: key);
    }
    expect({for (final option in session.options) option.id: option.current}, {'model': 'opus', 'effort': 'high'});
    expect(session.messages.last.text, startsWith('Profile for Claude Code: 6 applied'));
    await started.client.close();
    expect(Directory(pluginDir).parent.existsSync(), isFalse, reason: 'written for the session, gone with its agent');
  });

  test('Codex gets its skills as a directory, through additionalDirectories', () async {
    final (session, sent, started) = await launch('Codex', 'codex');
    final params = sent['params'] as Map;
    final dir = (params['additionalDirectories'] as List).single as String;
    expect(File('$dir/$codexSkillsPath/marker/SKILL.md').existsSync(), isTrue);
    expect(params.containsKey('_meta'), isFalse);
    expect(state(session, 'skills.marker'), ItemState.applied);
    expect(state(session, 'mcp.marker'), ItemState.applied);
    await started.client.close();
  });

  test('pi-acp is given the environment, model and effort, and every other part says why not', () async {
    final (session, sent, started) = await launch('Pi', 'pi');
    final params = sent['params'] as Map;
    expect(sent['env'], {'PROFILE_TOKEN': true, 'NOT_IN_PROFILE': false});
    expect(params['mcpServers'], isEmpty);
    expect(params.containsKey('systemPrompt'), isFalse);
    expect(started.launch.temporary, isNull);
    expect(state(session, 'env.PROFILE_TOKEN'), ItemState.applied);
    expect(state(session, 'model'), ItemState.applied);
    for (final key in ['context', 'skills.marker', 'mcp.marker']) {
      expect(state(session, key), ItemState.unsupported, reason: key);
    }
    expect(session.messages.last.text, contains('not applied: context (not supported)'));
    await started.client.close();
  });
}
