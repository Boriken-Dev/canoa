// canoa.cfg's `[loadouts]` and `[defaults]`: the named collections a session starts with, where an installed MCP server came
// from (`[store.mcp]`), and what a server or a chosen skill writes into the profile. Every change must read back as written, and
// nothing else in the file is lost.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/profile.dart' show OverlaySource;
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
  test('with no loadouts a session starts with every overlay; the first change writes a Default that holds the rest, in the file\'s order', () {
    var config = CanoaConfig.parse(base);
    expect(config.defaultOverlays, ['team', 'docs']);
    expect(config.loadouts, isEmpty);
    expect(config.resolveLoadout(), isNull);
    config = config.withStartingOverlay('team', false);
    expect(config.defaultLoadout, 'Default');
    expect(config.loadouts['Default']!.overlays, ['docs']);
    expect(config.defaultOverlays, ['docs']);
    final again = CanoaConfig.parse(config.encode());
    expect(again.defaultOverlays, ['docs'], reason: 'it reads back as written');
    expect(again.overlays.keys, ['team', 'docs']);
    expect(again.withStartingOverlay('team', true).defaultOverlays, ['team', 'docs'], reason: 'composition order, not the order added');
    expect(again.encode(), isNot(contains('[loadout]')), reason: 'the old table is not written');
  });

  test('removing an overlay takes it out of every loadout, so one put back later starts as a new one does', () {
    final config = CanoaConfig.parse('$base[loadouts.a]\noverlays = ["team", "docs"]\n[loadouts.b]\noverlays = ["team"]\n[defaults]\nloadout = "a"\n')
        .withoutOverlay('team');
    expect(config.loadouts['a']!.overlays, ['docs']);
    expect(config.loadouts['b']!.overlays, isEmpty);
    expect(config.overlays.keys, ['docs']);
    expect(config.defaultLoadout, 'a');
  });

  test('an overlay installed while a loadout is the default starts on in it, and one that was there already keeps its place', () {
    final source = OverlaySource.parseGit('https://example.invalid/new.git@main');
    final config = CanoaConfig.parse('$base[loadouts.main]\noverlays = ["docs"]\n[defaults]\nloadout = "main"\n');
    final added = config.withOverlay('new', source, 'sha256:${'b' * 64}');
    expect(added.loadouts['main']!.overlays, ['docs', 'new']);
    expect(added.withOverlay('new', source, 'sha256:${'c' * 64}').loadouts['main']!.overlays, ['docs', 'new']);
    final off = added.withStartingOverlay('new', false);
    expect(off.withOverlay('new', source, 'sha256:${'d' * 64}').loadouts['main']!.overlays, ['docs'], reason: 'an update does not turn it back on');
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
    expect(read.storeMcp['files']!.server, 'io.github.ana/files');
    expect(read.storeMcp['files']!.digest, digest);
    expect(read.loadouts, isEmpty, reason: 'no loadout is written until one is edited');
    expect(installed.encode(), isNot(contains('secret')));

    final allowed = read.withAgentMcp('Claude', {'files': true});
    expect(CanoaConfig.parse(allowed.encode()).agentMcp['Claude'], {'files': true});

    final gone = CanoaConfig.parse(read.withoutMcp('files').encode());
    expect(gone.profile.mcp.keys, ['keep']);
    expect(gone.agentMcp, isEmpty);
    expect(gone.storeMcp, isEmpty);
  });

  test('a server is in the default loadout when it is installed or allowed, and out of every loadout when it is removed', () {
    final origin = McpOrigin(registry: 'https://registry.example.com', server: 'io.github.ana/files', version: '1.2.0', digest: digest);
    final config = CanoaConfig.parse('$base[loadouts.main]\nmcp = ["keep"]\n[loadouts.other]\nmcp = ["keep"]\n[defaults]\nloadout = "main"\n')
        .withInstalledMcp('files', {'type': 'stdio', 'command': 'npx'}, origin, agents: const []);
    expect(config.loadouts['main']!.mcp, ['keep', 'files']);
    expect(config.loadouts['other']!.mcp, ['keep']);
    expect(config.withStartingMcp('files', false).loadouts['main']!.mcp, ['keep']);
    final removed = config.withoutMcp('keep');
    expect(removed.loadouts['main']!.mcp, ['files']);
    expect(removed.loadouts['other']!.mcp, isEmpty);
  });

  test('a chosen skill that is on is one name in the default loadout, taken away by its name', () {
    final config = CanoaConfig.parse(base).withLoadoutSkills({'audit'});
    final read = CanoaConfig.parse(config.encode());
    expect(read.loadoutSkills, {'audit'});
    expect(read.loadouts['Default']!.skills, ['audit']);
    expect(read.profile.skills, isEmpty, reason: 'no path is written for it');
    expect(read.profileWithChosenSkills.skills['audit']!.path, 'overlays/.skills/audit');
    expect(CanoaConfig.parse(read.withLoadoutSkills({}).encode()).loadoutSkills, isEmpty);
  });

  test('the store keeps its limits, its registries and the origins of its servers in one table', () {
    final config = CanoaConfig.parse('[store]\ncatalog_max_bytes = 100000\nregistries = ["https://registry.team.test"]\n');
    expect(config.store.catalogMaxBytes, 100000);
    final limited = config.withStore(config.store.withLimit('catalog_max_depth', 70));
    final read = CanoaConfig.parse(limited.encode());
    expect(read.store.catalogMaxDepth, 70);
    expect(read.storeRegistries, ['https://registry.team.test'], reason: 'changing a limit leaves the registries');
    expect(read.withStoreRegistries([]).store.catalogMaxBytes, 100000, reason: 'and changing the registries leaves the limits');
  });

  test('a loadout or default that is not one is refused, naming it', () {
    for (final text in [
      '[loadouts.a]\noverlays = "team"\n',
      '[loadouts.a]\norder = 1\n',
      '[loadouts.a]\nskills = ["../x"]\n',
      '[loadouts.a]\nmcp = ["a b"]\n',
      '[loadouts.a]\ndescription = 3\n',
      '[loadouts."a b"]\n',
      '[defaults]\nloadout = "ghost"\n',
      '[loadouts.a]\n[defaults]\nloadout = 1\n',
    ]) {
      expect(() => CanoaConfig.parse(text), throwsA(isA<FormatException>().having((e) => e.message, 'message', contains('loadout'))),
          reason: text);
    }
    expect(() => CanoaConfig.parse('[loadout]\noff = ["team"]\n'), throwsA(isA<FormatException>()), reason: 'the old table has no reader');
  });

  test('a session that names a loadout the file does not hold is refused with the name', () {
    final config = CanoaConfig.parse('$base[loadouts.review]\n');
    expect(() => config.resolveLoadout('ghost'), throwsA(isA<UnknownLoadout>().having((e) => e.toString(), 'message', contains('ghost'))));
    expect(config.resolveLoadout('review')!.overlays, isEmpty);
  });
}
