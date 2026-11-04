// A profile applied to each harness, from layers that live on disk: what each one is given, through what, and what it
// is not; an overlay's MCP servers substituted against its own environment and held to what the person shares; skills and
// context read once, inside their layer; and MCP servers from elsewhere off until allowed at their definition.
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/apply.dart';
import 'package:canoa/profile/profile.dart';
import 'package:canoa/profile/profile_home_io.dart';
import 'package:canoa/profile/profile_layers.dart';
import 'package:coqui_acp/coqui_acp.dart';
import 'package:flutter_test/flutter_test.dart';

late Directory temp;

String slash(String path) => path.replaceAll(r'\', '/');

void write(String path, String text) => File(path)
  ..parent.createSync(recursive: true)
  ..writeAsStringSync(text);

/// What Claude's adapter, Codex's, pi-acp and Gemini CLI say they take, as `initialize` says it.
const claudeTakes = AgentTakes(mcp: _all, systemPrompt: true, additionalDirectories: true);
const codexTakes = AgentTakes(mcp: _noSse, systemPrompt: true, additionalDirectories: true);
const piTakes = AgentTakes(mcp: _stdio);
const geminiTakes = AgentTakes(mcp: _all);
bool _all(AcpMcpTransport transport) => true;
bool _noSse(AcpMcpTransport transport) => transport != AcpMcpTransport.sse;
bool _stdio(AcpMcpTransport transport) => transport == AcpMcpTransport.stdio;

/// Every variable this test's process "has".
const hostEnv = {'SHARED_TOKEN': 'shared-secret', 'UNSHARED': 'not-yours', 'HOST_ONLY': 'host'};

Future<String?> findProgram(String program) async => program == 'node' ? '/usr/bin/node' : null;

/// The layers of [config] for [agent], with an overlay directory, the user's skill directory and a project in [temp].
Future<(ProfileLayers, IoProfileHome)> layersOf(String config, {String agent = 'Claude', String? project}) async {
  final home = IoProfileHome('${temp.path}/config');
  final parsed = CanoaConfig.parse(config);
  final layers = await profileLayers(
    overlays: parsed.overlays,
    user: parsed.profile,
    agents: parsed.agentProfiles,
    agent: agent,
    home: home,
    workingDirectory: project,
  );
  return (layers, home);
}

Future<PreparedProfile> prepared(ProfileLayers layers, IoProfileHome home) =>
    prepareProfile(layers, home: home, lookup: (name) => hostEnv[name], findProgram: findProgram);

ProfileItem item(List<ProfileItem> items, String key) => items.firstWhere((item) => item.key == key);

void main() {
  setUp(() {
    temp = Directory.systemTemp.createTempSync('canoa-apply-');
    write('${temp.path}/team/overlay.cfg', '''
[env]
TEAM = "crew"

[[mcp.servers]]
name = "team"
transport = "stdio"
command = "node"
args = ["serve.mjs"]
env = { WHERE = "\${AOS_OVERLAY_ROOT}", WHO = "\${TEAM}-bot" }
env_passthrough = ["SHARED_TOKEN", "UNSHARED"]

[[mcp.servers]]
name = "docs"
transport = "streamable-http"
url = "https://docs.example/mcp"
headers_env = { Authorization = "SHARED_TOKEN" }
''');
    write('${temp.path}/team/CONTEXT.md', 'From the team overlay.\n');
    write('${temp.path}/team/skills/review/SKILL.md', '---\nname: review\n---\nReview it.\n');
    write('${temp.path}/team/skills/review/notes/extra.md', 'More.\n');
    write('${temp.path}/mine/SKILL.md', '---\nname: mine\n---\nMine.\n');
    Directory('${temp.path}/config').createSync();
  });
  tearDown(() => temp.deleteSync(recursive: true));

  String config({String extra = ''}) => '''
[overlays]
team = { dir = "${slash(temp.path)}/team" }
[profile]
share_env = ["SHARED_TOKEN"]
context = { text = "From the person." }
[profile.env.FROM_HOST]
from_env = "HOST_ONLY"
[profile.skills.mine]
path = "${slash(temp.path)}/mine"
[profile.mcp.marker]
type = "stdio"
command = "node"
args = ["marker.mjs"]
[profile.mcp.marker.env.MARKER_TOKEN]
from_env = "HOST_ONLY"
$extra''';

  test('Claude takes context, skills as a plugin and MCP servers, each through its own channel', () async {
    final (layers, home) = await layersOf(config());
    final ready = await prepared(layers, home);
    final launch = await launchFor(ready, harness: 'claude-code', takes: claudeTakes,
        mcpOn: {'marker', 'team', 'docs'}, files: home);
    addTearDown(() => home.remove(launch.temporary!));
    final setup = launch.setup;
    expect(setup.systemPrompt, 'From the team overlay.\n\n\nFrom the person.');
    final plugin = (((setup.meta!['claudeCode'] as Map)['options'] as Map)['plugins'] as List).single as Map;
    expect(plugin['type'], 'local');
    final root = plugin['path'] as String;
    expect(File('$root/.claude-plugin/plugin.json').existsSync(), isTrue);
    expect(File('$root/skills/review/SKILL.md').readAsStringSync(), contains('Review it.'));
    expect(File('$root/skills/review/notes/extra.md').existsSync(), isTrue);
    expect(File('$root/skills/mine/SKILL.md').readAsStringSync(), contains('Mine.'));
    expect([for (final server in setup.mcpServers) server.name], unorderedEquals(['marker', 'team', 'docs']));
    expect(setup.additionalDirectories, isEmpty);
    expect(ready.environment['FROM_HOST']!.expose(), 'host');
    final items = launch.items;
    for (final key in ['env.FROM_HOST', 'skills.review', 'skills.mine', 'mcp.marker', 'mcp.team', 'mcp.docs']) {
      expect(item(items, key).state, ItemState.applied, reason: key);
    }
    expect(items.where((item) => item.key == 'context').map((item) => item.state).toSet(), {ItemState.applied});
    expect(item(items, 'skills.review').how, contains('canoa-profile:review'));
  });

  test('Codex reads skills from a directory it is given, and is told of no SSE server', () async {
    final (layers, home) = await layersOf(config(extra: '''
[profile.mcp.events]
type = "sse"
url = "https://events.example/sse"
'''), agent: 'Codex');
    final ready = await prepared(layers, home);
    final launch = await launchFor(ready, harness: 'codex', takes: codexTakes, mcpOn: {'marker', 'events'}, files: home);
    addTearDown(() => home.remove(launch.temporary!));
    expect(launch.setup.meta, isNull);
    final dir = launch.setup.additionalDirectories.single;
    expect(File('$dir/$codexSkillsPath/review/SKILL.md').existsSync(), isTrue);
    expect(File('$dir/$codexSkillsPath/mine/SKILL.md').existsSync(), isTrue);
    expect(launch.setup.systemPrompt, isNotNull);
    expect([for (final server in launch.setup.mcpServers) server.name], ['marker']);
    expect(item(launch.items, 'mcp.events').state, ItemState.unsupported);
    expect(item(launch.items, 'mcp.marker').how, contains('Codex skips it'));
  });

  test('Pi is given only the environment, and says why not the rest', () async {
    final (layers, home) = await layersOf(config(), agent: 'Pi');
    final ready = await prepared(layers, home);
    final launch = await launchFor(ready, harness: 'pi', takes: piTakes, mcpOn: {'marker'}, files: home);
    expect(launch.temporary, isNull, reason: 'nothing is written for a harness that reads nothing of it');
    expect(launch.setup.mcpServers, isEmpty);
    expect(launch.setup.systemPrompt, isNull);
    expect(item(launch.items, 'env.FROM_HOST').state, ItemState.applied);
    expect(item(launch.items, 'context').state, ItemState.unsupported);
    expect(item(launch.items, 'skills.mine').state, ItemState.unsupported);
    expect(item(launch.items, 'mcp.marker').state, ItemState.unsupported);
    expect(item(launch.items, 'mcp.marker').how, contains('pi-acp'));
  });

  test('Gemini CLI takes MCP servers but no context or skills', () async {
    final (layers, home) = await layersOf(config(), agent: 'Gemini');
    final launch = await launchFor(await prepared(layers, home), harness: 'gemini-cli', takes: geminiTakes,
        mcpOn: {'marker'}, files: home);
    expect([for (final server in launch.setup.mcpServers) server.name], ['marker']);
    expect(item(launch.items, 'context').state, ItemState.unsupported);
    expect(item(launch.items, 'skills.review').state, ItemState.unsupported);
  });

  test("an overlay's server is substituted against its own environment and gets only what the person shares", () async {
    final (layers, home) = await layersOf(config());
    final ready = await prepared(layers, home);
    final team = ready.mcp.firstWhere((server) => server.name == 'team');
    final stdio = team.server! as AcpMcpStdio;
    expect(stdio.command, '/usr/bin/node');
    expect(stdio.env['WHO'], 'crew-bot');
    expect(slash(stdio.env['WHERE']!).toLowerCase(), endsWith('/team'));
    expect(stdio.env['SHARED_TOKEN'], 'shared-secret');
    expect(stdio.env.containsKey('UNSHARED'), isFalse);
    expect(team.withheld.single, contains('UNSHARED'));
    expect(team.receives, containsAll(['WHO', 'WHERE', 'SHARED_TOKEN', 'UNSHARED']));
    final docs = ready.mcp.firstWhere((server) => server.name == 'docs').server! as AcpMcpHttp;
    expect(docs.headers['Authorization'], 'shared-secret');
    expect(team.notes.join(), contains("not the overlay's"));
  });

  test('a server from an overlay is off until allowed at its definition; the person\'s own is on unless turned off', () async {
    final (layers, home) = await layersOf(config());
    final ready = await prepared(layers, home);
    final team = ready.mcp.firstWhere((server) => server.name == 'team');
    final marker = ready.mcp.firstWhere((server) => server.name == 'marker');
    expect(mcpOnByDefault(team, const {}), isFalse);
    expect(mcpOnByDefault(team, const {'team': true}), isFalse, reason: 'only its digest allows it');
    expect(mcpOnByDefault(team, {'team': team.digest}), isTrue);
    expect(mcpOnByDefault(marker, const {}), isTrue);
    expect(mcpOnByDefault(marker, const {'marker': false}), isFalse);

    // The overlay changes its server: what was allowed was another definition.
    final manifest = File('${temp.path}/team/overlay.cfg');
    manifest.writeAsStringSync(manifest.readAsStringSync().replaceFirst('serve.mjs', 'other.mjs'));
    final (again, home2) = await layersOf(config());
    final changed = (await prepared(again, home2)).mcp.firstWhere((server) => server.name == 'team');
    expect(changed.digest, isNot(team.digest));
    expect(mcpOnByDefault(changed, {'team': team.digest}), isFalse);

    final launch = await launchFor(ready, harness: 'claude-code', takes: claudeTakes, mcpOn: {'marker'}, files: home);
    addTearDown(() => home.remove(launch.temporary!));
    expect(item(launch.items, 'mcp.team').state, ItemState.off);
    expect(item(launch.items, 'mcp.team').how, 'not allowed for this session');
  });

  test("an overlay's skills and context are the bytes its digest covered, not the files read again", () async {
    final (layers, home) = await layersOf(config());
    File('${temp.path}/team/skills/review/SKILL.md').writeAsStringSync('Swapped after the check.');
    File('${temp.path}/team/CONTEXT.md').writeAsStringSync('Swapped after the check.');
    final ready = await prepared(layers, home);
    final review = ready.skills.firstWhere((skill) => skill.name == 'review');
    expect(utf8.decode(review.files['SKILL.md']!), contains('Review it.'));
    expect(ready.contexts.first.text, 'From the team overlay.\n');
  });

  test("a project's skill or context that leads out of its directory is refused", () async {
    final project = Directory('${temp.path}/project')..createSync();
    final outside = Directory('${temp.path}/outside')..createSync();
    write('${outside.path}/SKILL.md', 'Outside.');
    write('${outside.path}/secret.md', 'Outside.');
    Link('${project.path}/skills').createSync(outside.path);
    write('${project.path}/ctx', '');
    Link('${project.path}/context.md').createSync('${outside.path}/secret.md');
    write('${project.path}/.canoa/profile.toml',
        '[skills.out]\npath = "skills"\n[context]\nfile = "context.md"\n');
    var (layers, home) = await layersOf(config(), project: project.path);
    await home.trust.load();
    await home.trust.trust(layers.project!.directory, layers.project!.profile!);
    (layers, home) = await layersOf(config(), project: project.path);
    final ready = await prepared(layers, home);
    expect(ready.skills.firstWhere((skill) => skill.name == 'out').refused, contains('outside its layer'));
    expect(ready.problems.single.key, 'context');
    expect(ready.problems.single.state, ItemState.refused);
  });

  test('a skill directory holding a link is refused, and one with no SKILL.md', () async {
    write('${temp.path}/linked/SKILL.md', 'Linked.');
    Link('${temp.path}/linked/out').createSync(temp.path);
    Directory('${temp.path}/empty').createSync();
    final (layers, home) = await layersOf(config(extra: '''
[profile.skills.linked]
path = "${slash(temp.path)}/linked"
[profile.skills.empty]
path = "${slash(temp.path)}/empty"
'''));
    final ready = await prepared(layers, home);
    expect(ready.skills.firstWhere((skill) => skill.name == 'linked').refused, contains('link'));
    expect(ready.skills.firstWhere((skill) => skill.name == 'empty').refused, contains('SKILL.md'));
  });

  test('a server whose program is not found is refused, and an unshared required reference stops the launch', () async {
    final (layers, home) = await layersOf(config(extra: '''
[profile.mcp.missing]
type = "stdio"
command = "no-such-program"
'''));
    final ready = await prepared(layers, home);
    expect(ready.mcp.firstWhere((server) => server.name == 'missing').refused, contains('not found'));
    final launch = await launchFor(ready, harness: 'claude-code', takes: claudeTakes, mcpOn: {'missing'}, files: home);
    addTearDown(() => home.remove(launch.temporary!));
    expect(item(launch.items, 'mcp.missing').state, ItemState.refused);

    final (broken, home2) = await layersOf(config(extra: '[profile.env.GONE]\nfrom_env = "NOT_SET_ANYWHERE"\n'));
    expect(() => prepared(broken, home2), throwsA(isA<ProfileException>()));
  });

  test('the model, effort and mode go to the options that offer them', () {
    final ready = PreparedProfile(
      model: const Sourced('opus', LayerId(LayerKind.user), null),
      effort: const Sourced('high', LayerId(LayerKind.agent, 'Claude'), null),
      mode: const Sourced('plan', LayerId(LayerKind.user), null),
    );
    expect(choicesFor(ready, [
      ('model', 'model', ['sonnet', 'opus'], false),
      ('reasoning', 'thought_level', ['low', 'high'], false),
      ('mode', 'mode', ['default'], false),
    ]), [('model', 'opus', 'model'), ('reasoning', 'high', 'effort')]);
  });

  test("[agents.NAME.mcp] reads back as written, and keeping it keeps the rest of Canoa.cfg", () {
    final parsed = CanoaConfig.parse('''
[permissions]
full_access = ["Claude"]
[notifications]
finished = false
[agents.Claude.mcp]
marker = true
team = "sha256:${'a' * 64}"
off = false
''');
    expect(parsed.agentMcp['Claude'], {'marker': true, 'team': 'sha256:${'a' * 64}', 'off': false});
    final changed = parsed.withAgentMcp('Codex', {'marker': false});
    expect(changed.fullAccess, {'Claude'});
    expect(changed.notifications.finished, isFalse);
    // What saving Settings does with the profile tables the app holds.
    final saved = parsed.copyWith(look: parsed.look).withProfilesOf(changed);
    expect(saved.fullAccess, {'Claude'});
    expect(saved.notifications.finished, isFalse);
    final reread = CanoaConfig.parse(changed.encode());
    expect(reread.agentMcp['Codex'], {'marker': false});
    expect(reread.agentMcp['Claude']!['marker'], true);
    expect(() => CanoaConfig.parse('[agents.Claude.mcp]\nmarker = "yes"\n'), throwsFormatException);
  });
}
