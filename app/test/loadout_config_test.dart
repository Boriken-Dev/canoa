// canoa.cfg's `[loadout]`: which overlays new sessions start without, where an installed MCP server came from, and what a server
// or a chosen skill writes into the profile. Every change must read back as written, and nothing else in the file is lost.
import 'package:canoa/config/canoa_config.dart';
import 'package:flutter_test/flutter_test.dart';

const base = '''
[overlays]
team = "https://example.invalid/team.git@main"
docs = "https://example.invalid/docs.git@main"

[profile.mcp.keep]
type = "stdio"
command = "keep-mcp"
''';

final digest = 'sha256:${'a' * 64}';

void main() {
  test('an overlay turned off is left out of the defaults, and turned on again it is back, in the file\'s order', () {
    var config = CanoaConfig.parse(base);
    expect(config.defaultOverlays, ['team', 'docs']);
    config = config.withLoadoutOff({'team'});
    expect(config.loadoutOff, {'team'});
    expect(config.defaultOverlays, ['docs']);
    final again = CanoaConfig.parse(config.encode());
    expect(again.defaultOverlays, ['docs'], reason: 'it reads back as written');
    expect(again.overlays.keys, ['team', 'docs']);
    expect(again.withLoadoutOff({}).defaultOverlays, ['team', 'docs']);
    expect(again.withLoadoutOff({}).encode(), isNot(contains('loadout')), reason: 'nothing off leaves no table');
  });

  test('removing an overlay forgets that it was off, so one put back later starts on', () {
    final config = CanoaConfig.parse(base).withLoadoutOff({'team'}).withoutOverlay('team');
    expect(config.loadoutOff, isEmpty);
    expect(config.overlays.keys, ['docs']);
  });

  test('an MCP server is written to the profile, off for each agent, with where it came from; removed, nothing of it stays', () {
    final origin = McpOrigin(registry: 'https://registry.example.com', server: 'io.github.ana/files', version: '1.2.0', digest: digest);
    final installed = CanoaConfig.parse(base).withInstalledMcp('files', {
      'type': 'stdio',
      'command': 'npx',
      'args': ['-y', 'files-mcp@1.2.0'],
      'env': {'FILES_TOKEN': {'from_env': 'FILES_TOKEN'}},
    }, origin, agents: ['Claude', 'Codex']);
    final read = CanoaConfig.parse(installed.encode());
    expect(read.profile.mcp.keys, containsAll(['keep', 'files']), reason: 'what was there stays');
    expect(read.agentMcp, {'Claude': {'files': false}, 'Codex': {'files': false}});
    expect(read.loadoutMcp['files']!.server, 'io.github.ana/files');
    expect(read.loadoutMcp['files']!.digest, digest);
    expect(installed.encode(), isNot(contains('secret')));

    final allowed = read.withAgentMcp('Claude', {'files': true});
    expect(CanoaConfig.parse(allowed.encode()).agentMcp['Claude'], {'files': true});

    final gone = CanoaConfig.parse(read.withoutMcp('files').encode());
    expect(gone.profile.mcp.keys, ['keep']);
    expect(gone.agentMcp, isEmpty);
    expect(gone.loadoutMcp, isEmpty);
  });

  test('a chosen skill is one profile entry, taken away by its name', () {
    final config = CanoaConfig.parse(base).withProfileSkill('audit', 'overlays/.skills/audit');
    expect(CanoaConfig.parse(config.encode()).profile.skills['audit']!.path, 'overlays/.skills/audit');
    expect(CanoaConfig.parse(config.withProfileSkill('audit', null).encode()).profile.skills, isEmpty);
  });

  test('a loadout table that is not one is refused, naming it', () {
    for (final text in ['[loadout]\noff = "team"\n', '[loadout]\nother = 1\n', '[loadout.mcp]\nx = 3\n', '[loadout]\noff = ["a b"]\n']) {
      expect(() => CanoaConfig.parse(text), throwsA(isA<FormatException>().having((e) => e.message, 'message', contains('loadout'))), reason: text);
    }
  });
}
