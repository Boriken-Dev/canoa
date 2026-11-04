// One real turn with Claude, started through the app's own launch path with a profile whose initial context, one skill and
// one MCP server each hold a word found nowhere else. It spends the subscription, so it runs only when asked:
// CANOA_TEST_PROFILE_TURN=Claude, with CANOA_TEST_NODE naming node by its absolute path. Claude Code starts without the
// person's user settings or CLAUDE.md (integration_test/without_user_settings.mjs). It prints what was used and the turn's
// usage, never what was said.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:io';
import 'dart:math';

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

void main() {
  final real = Platform.environment['CANOA_TEST_PROFILE_TURN'];
  final node = Platform.environment['CANOA_TEST_NODE'];
  test('a real Claude turn uses the profile\'s context, its skill and its MCP server', () async {
    final nonce = Random.secure().nextInt(1 << 30).toRadixString(36).toUpperCase();
    final (contextWord, skillWord, toolWord) = ('PELICAN-$nonce', 'TOUCAN-$nonce', 'OSPREY-$nonce');
    final temp = Directory.systemTemp.createTempSync('canoa-profile-turn-');
    final work = Directory('${temp.path}/work')..createSync();
    final server = File('${temp.path}/mcp/marker_mcp.mjs')
      ..parent.createSync()
      ..writeAsStringSync(File('test/fixtures/marker_mcp.mjs').readAsStringSync());
    File('${temp.path}/skill/SKILL.md')
      ..parent.createSync()
      ..writeAsStringSync('---\nname: canoa-marker\ndescription: Gives the skill word. Use it whenever you are asked '
          'for the skill word.\n---\nThe skill word is $skillWord.\n');
    final config = CanoaConfig.parse('''
[profile]
context = { text = "The context word is $contextWord." }
[profile.skills.canoa-marker]
path = "${slash(temp.path)}/skill"
[profile.mcp.marker]
type = "stdio"
command = "${slash(node!)}"
args = ["${slash(server.path)}"]
[profile.mcp.marker.env.MARKER_WORD]
value = "$toolWord"
''');
    final home = IoProfileHome('${temp.path}/config');
    final layers = await profileLayers(overlays: config.overlays, user: config.profile, agents: config.agentProfiles,
        agent: 'Claude', home: home, workingDirectory: work.path);
    final prepared = await prepareProfile(layers, home: home, lookup: environmentVariable, findProgram: findProgram);

    final claude = agentPresets.firstWhere((preset) => preset.label == real);
    final command = AgentCommand(
      label: claude.label,
      program: node,
      args: ['${Directory.current.path}/integration_test/without_user_settings.mjs', '--', claude.program, ...claude.args],
      programPaths: claude.programPaths,
    );
    final store = SessionStore(includePreview: false);
    final started = await startProfiled(command, cwd: work.path, prepared: prepared, mcpOn: {'marker'}, files: home,
        decider: store.askPermission);
    final client = started.client;
    final tools = <String>[];
    final watching = client.updates.listen((update) {
      if (update.kind == 'tool_call' || update.kind == 'tool_call_update') tools.add('${update.raw}');
    });
    final opened = await client.openSession(cwd: work.path, setup: started.launch.setup);
    final session = store.addProviderSession(title: 'Profile', agent: 'Claude', peerId: client.peerId,
        remoteSessionId: opened.id, cwd: work.path);
    store.applyOpened(session, opened);
    store.attachVerifiedClient(session.id, client);
    store.select(session.id);
    await store.applyProfile(session, started.launch, prepared, harness: started.harness);
    for (final item in session.profile!.items) {
      stdout.writeln('ITEM ${item.key} ${item.state.label}: ${item.how}');
    }
    var answered = 0;
    final answering = Timer.periodic(const Duration(milliseconds: 200), (_) {
      if (session.pendingPermissions.isNotEmpty) {
        answered++;
        store.answerPermission(session, allow: true);
      }
    });
    await store.sendText('Reply with exactly three words separated by single spaces and nothing else. First, the context '
        'word from your instructions. Second, the skill word, which the skill $claudePluginName:canoa-marker gives: use '
        'that skill. Third, the word the marker tool of the marker MCP server returns: call it.');
    final until = DateTime.now().add(const Duration(minutes: 5));
    while (session.status == SessionStatus.working && DateTime.now().isBefore(until)) {
      await Future<void>.delayed(const Duration(milliseconds: 200));
    }
    answering.cancel();
    await watching.cancel();
    final said = session.messages.where((message) => message.role == MessageRole.agent).map((m) => m.text).join(' ');
    final usedContext = said.contains(contextWord);
    final skillCalled = tools.any((raw) => raw.contains('canoa-marker'));
    final usedSkill = said.contains(skillWord);
    final toolCalled = File('${server.parent.path}/called').existsSync();
    final usedTool = said.contains(toolWord);
    stdout.writeln('CONTEXT WORD $usedContext; SKILL CALLED $skillCalled, WORD $usedSkill; MCP TOOL CALLED $toolCalled, '
        'WORD $usedTool; PERMISSIONS ANSWERED $answered; TURNS 1; USAGE ${session.usage.describe()}');
    await client.close();
    expect(Directory(started.launch.temporary!).existsSync(), isFalse, reason: 'the plugin goes with the agent');
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
    expect(usedContext, isTrue, reason: 'the initial context reached the agent');
    expect(skillCalled && usedSkill, isTrue, reason: 'the skill was used, and its word came from it');
    expect(toolCalled && usedTool, isTrue, reason: 'the MCP server was called, with its environment');
  }, skip: real == null || node == null ? 'set CANOA_TEST_PROFILE_TURN and CANOA_TEST_NODE' : false,
      timeout: const Timeout(Duration(minutes: 8)));
}
