import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/popout/open_in.dart';
import 'package:canoa/profile/profile.dart' show GitOverlay;
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('config roundtrip', () {
    const original = CanoaConfig();
    expect(CanoaConfig.parse(original.encode()).encode(), original.encode());
    expect(original.closeToTray, true);
  });
  test('reads AI6-style .cfg in TOML', () {
    final parsed = CanoaConfig.parse('''
#!toml
[desktop]
start_hidden = true
minimize_to_tray = true
[network]
prefer_webtransport = false
connect_timeout = 12
''');
    expect(parsed.startHidden, true);
    expect(parsed.preferWebTransport, false);
    expect(CanoaConfig.parse(parsed.encode()).connectTimeout, 12);
  });
  test('saving Settings or pinning an overlay keeps the default mode and the notifications', () {
    final config = CanoaConfig.parse('[permissions]\ndefault_mode = "plan"\n[notifications]\nfinished = false\n'
        '[overlays]\nmine = { dir = "mine" }\n');
    for (final saved in [config.withProfilesOf(config), config.withOverlayPin('mine', 'sha256:${'0' * 64}')]) {
      expect(saved.defaultMode, 'plan');
      expect(saved.notifications.finished, isFalse);
      final reread = CanoaConfig.parse(saved.encode());
      expect(reread.defaultMode, 'plan');
      expect(reread.notifications.finished, isFalse);
    }
  });

  test('an overlay entry carries the values of its declared options, kept when it is pinned and saved', () {
    final config = CanoaConfig.parse('[overlays]\nmine = { dir = "mine", options = { depth = "deep", rounds = 3 } }\n'
        'plain = "https://example.org/a.git@main"\n');
    expect(config.overlayOptions('mine'), {'depth': 'deep', 'rounds': 3});
    expect(config.overlayOptions('plain'), isEmpty);
    for (final saved in [config.withProfilesOf(config), config.withOverlayPin('mine', 'sha256:${'0' * 64}')]) {
      expect(CanoaConfig.parse(saved.encode()).overlayOptions('mine'), {'depth': 'deep', 'rounds': 3});
    }
    final changed = config.withOverlayOptions('plain', {'depth': 'quick'});
    expect(CanoaConfig.parse(changed.encode()).overlayOptions('plain'), {'depth': 'quick'});
    expect((CanoaConfig.parse(changed.encode()).overlays['plain'] as GitOverlay).spec, (config.overlays['plain'] as GitOverlay).spec);
    expect(CanoaConfig.parse(config.withOverlayOptions('mine', {}).encode()).overlayOptions('mine'), isEmpty);
    expect(() => CanoaConfig.parse('[overlays]\nmine = { dir = "mine", options = "deep" }\n'), throwsFormatException);
  });

  test('an overlay entry carries how its knowledge reaches the agent, and every rewrite of the entry keeps it', () {
    final config = CanoaConfig.parse('[overlays]\nmine = { dir = "mine", kb = "index", options = { depth = "deep" } }\n'
        'plain = { source = "https://example.org/a.git@main", kb = "files" }\nbare = "https://example.org/b.git@main"\n');
    expect(config.overlayKb('mine'), 'index');
    expect(config.overlayKb('plain'), 'files');
    expect(config.overlayKb('bare'), isNull);
    // Pinning, saving Settings and setting options all write the entry again, and none loses `kb`.
    for (final saved in [
      config.withProfilesOf(config),
      config.withOverlayPin('mine', 'sha256:${'0' * 64}'),
      config.withOverlayPin('plain', 'sha256:${'1' * 64}'),
      config.withOverlayOptions('mine', {'depth': 'quick'}),
    ]) {
      final reread = CanoaConfig.parse(saved.encode());
      expect(reread.overlayKb('mine'), 'index');
      expect(reread.overlayKb('plain'), 'files');
    }
    expect(CanoaConfig.parse(config.withOverlayPin('mine', 'sha256:${'0' * 64}').encode()).overlayOptions('mine'), {'depth': 'deep'});

    final chosen = config.withOverlayKb('bare', 'index');
    expect(CanoaConfig.parse(chosen.encode()).overlayKb('bare'), 'index');
    expect((CanoaConfig.parse(chosen.encode()).overlays['bare'] as GitOverlay).spec, (config.overlays['bare'] as GitOverlay).spec);
    final cleared = chosen.withOverlayKb('mine', null);
    expect(CanoaConfig.parse(cleared.encode()).overlayKb('mine'), isNull);
    expect(CanoaConfig.parse(cleared.encode()).overlayOptions('mine'), {'depth': 'deep'}, reason: 'its other keys stay');
    expect(() => config.withOverlayKb('mine', 'sometimes'), throwsArgumentError);
    expect(() => config.withOverlayKb('nope', 'index'), throwsArgumentError);
    expect(() => CanoaConfig.parse('[overlays]\nmine = { dir = "mine", kb = "sometimes" }\n'), throwsFormatException);
  });

  test('[host.kb] is read and set by its embedder, its ranker and the server the panel keeps, and the rest stays', () {
    final config = CanoaConfig.parse(
      '[host.kb]\nchunk_bytes = 512\n[host.kb.embedder]\nuse = "openai:work"\nmodel = "m"\ndimensions = 256\n'
      '[sdk.other]\nprovider = "server"\naddress = "http://h"\n'
      '[host.agents.a]\nprogram = "agent"\nlisten = "127.0.0.1:7000"\n',
    );
    expect(config.hostKb, {
      'embedder_use': 'openai:work',
      'embedder_model': 'm',
    });
    expect(const CanoaConfig().hostKb, isEmpty);

    // A server and a ranker with no `use` of its own use the server; the embedder keeps its backend.
    final changed = config.withHostKb({
      'embedder_use': 'openai:work',
      'embedder_model': 'm',
      'ranker_model': 'q',
      'server': 'http://127.0.0.1:60570',
    });
    final reread = CanoaConfig.parse(changed.encode());
    expect(reread.hostKb, {
      'embedder_use': 'openai:work',
      'embedder_model': 'm',
      'ranker_use': 'knowledge',
      'ranker_model': 'q',
      'server': 'http://127.0.0.1:60570',
    });
    expect(
      ((reread.host!['kb']) as Map)['chunk_bytes'],
      512,
      reason: 'a key the app does not edit stays',
    );
    expect(
      (((reread.host!['kb']) as Map)['embedder'] as Map)['dimensions'],
      256,
      reason: 'so does one of a role it edits',
    );
    expect(reread.hostEntries.single.name, 'a');
    final sdk = reread.tables['sdk'] as Map;
    expect(sdk['other'], isNotNull, reason: 'another offer stays');
    expect((sdk['knowledge'] as Map)['capabilities'], ['embed', 'chat']);
    expect((sdk['knowledge'] as Map)['provider'], 'server');

    // A role with no model goes, with the server when none is named; a table with nothing left goes; so does a `[host]`.
    final less = reread.withHostKb({
      'embedder_use': 'openai:work',
      'embedder_model': ' ',
    });
    expect(CanoaConfig.parse(less.encode()).hostKb, isEmpty);
    expect(
      (CanoaConfig.parse(less.encode()).tables['sdk'] as Map).containsKey(
        'knowledge',
      ),
      isFalse,
    );
    final bare = CanoaConfig.parse('[host.kb.ranker]\nuse = "x"\nmodel = "q"\n')
        .withHostKb(const {});
    expect(bare.host, isNull);
    expect(CanoaConfig.parse(bare.encode()).hostKb, isEmpty);
    // With no `use` and no server, a role uses the operation's own choice.
    expect(const CanoaConfig().withHostKb({'embedder_model': 'm'}).hostKb, {
      'embedder_use': 'default',
      'embedder_model': 'm',
    });
  });

  test('allow_once_only is read, written and kept by every copy, and off by default', () {
    expect(const CanoaConfig().allowOnceOnly, isFalse);
    expect(CanoaConfig.parse(const CanoaConfig().encode()).allowOnceOnly, isFalse);
    final config = CanoaConfig.parse('[permissions]\nallow_once_only = true\n');
    expect(config.allowOnceOnly, isTrue);
    expect(config.defaultMode, isNull);
    for (final saved in [config, config.withProfilesOf(config), config.withHost(null), config.copyWith(brightness: 'light')]) {
      expect(CanoaConfig.parse(saved.encode()).allowOnceOnly, isTrue);
    }
    expect(config.copyWith(allowOnceOnly: false).allowOnceOnly, isFalse);
    expect(() => CanoaConfig.parse('[permissions]\nallow_once_only = "yes"\n'), throwsFormatException);
  });

  test('keeps the host table canoa reads when settings are saved', () {
    final parsed = CanoaConfig.parse('''
[desktop]
start_hidden = true
[host]
origins = ["https://canoa.example"]
[host.agents.claude]
program = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@0.99.1"]
listen = "127.0.0.1:7801"
[host.providers.echo]
library = "echo_provider.dll"
provider = "echo_provider"
''');
    final saved = CanoaConfig.parse(parsed.copyWith(startHidden: false).encode());
    expect(saved.startHidden, false);
    expect(saved.host, parsed.host);
    expect((saved.host!['agents'] as Map)['claude']['listen'], '127.0.0.1:7801');
    expect(CanoaConfig.parse(saved.encode()).encode(), saved.encode());
    expect(() => CanoaConfig.parse('host = 1'), throwsFormatException);
  });
  test('an agent added to [host] survives saving, beside what was there, and can be taken out again', () {
    final before = CanoaConfig.parse('''
[desktop]
start_hidden = true
[host]
origins = ["https://canoa.example"]
[host.providers.echo]
library = "echo_provider.dll"
provider = "echo_provider"
''');
    final added = before.withHostAgent(
      name: 'mine',
      program: r'C:\Program Files\agent.exe',
      args: ['--acp', 'a "quoted" word'],
      listen: '127.0.0.1:7811',
      workingRoots: ['/home/me/src'],
    );
    final saved = CanoaConfig.parse(added.encode());
    expect(saved.startHidden, isTrue);
    expect(saved.host!['origins'], ['https://canoa.example']);
    expect(saved.hostEntries.map((entry) => (entry.name, entry.kind, entry.runs, entry.args.join('|'), entry.listen)), [
      ('mine', 'agent', r'C:\Program Files\agent.exe', '--acp|a "quoted" word', '127.0.0.1:7811'),
      ('echo', 'provider', 'echo_provider.dll', '', null),
    ]);
    expect(before.hostEntries, hasLength(1), reason: 'the original is not changed');
    final reading = CanoaConfig.parse(
      before.withHostAgent(name: 'agy', program: 'agy', args: ['acp'], listen: '127.0.0.1:7812', usage: 'antigravity').encode(),
    );
    expect(((reading.host!['agents'] as Map)['agy'] as Map)['usage'], 'antigravity');
    expect(((saved.host!['agents'] as Map)['mine'] as Map).containsKey('usage'), isFalse);
    expect(() => saved.withHostAgent(name: 'mine', program: 'x', listen: '127.0.0.1:1'), throwsArgumentError);
    expect(() => saved.withHostAgent(name: 'echo', program: 'x', listen: '127.0.0.1:1'), throwsArgumentError);
    // What an entry's name may hold is the host's to say when it checks the file, so the app writes any name, quoted.
    final spaced = saved.withHostAgent(name: 'no spaces', program: 'x', listen: '127.0.0.1:1');
    expect(CanoaConfig.parse(spaced.encode()).hostEntries.map((entry) => entry.name), contains('no spaces'));
    final removed = CanoaConfig.parse(saved.withoutHostEntry('mine').encode());
    expect(removed.hostEntries.map((entry) => entry.name), ['echo']);
  });

  test('keeps the usage limit phrases canoa reads, unset, a list or empty, when settings are saved', () {
    expect(CanoaConfig.parse('').limitPhrases, isNull);
    expect(CanoaConfig.parse('[usage]\nwarn_permille = 500').encode(), isNot(contains('limit_phrases')));
    final listed = CanoaConfig.parse('[usage]\nwarn_permille = 500\nlimit_phrases = ["out of credits", "429"]');
    expect(listed.limitPhrases, ['out of credits', '429']);
    final saved = CanoaConfig.parse(listed.copyWith(startHidden: true).encode());
    expect(saved.limitPhrases, ['out of credits', '429']);
    expect(saved.warnPermille, 500);
    // An empty list is a setting of its own: it infers nothing, and must not turn into unset.
    expect(CanoaConfig.parse(CanoaConfig.parse('[usage]\nlimit_phrases = []').encode()).limitPhrases, isEmpty);
    expect(() => CanoaConfig.parse('[usage]\nlimit_phrases = "quota"'), throwsFormatException);
    expect(() => CanoaConfig.parse('[usage]\nlimit_phrases = [1]'), throwsFormatException);
  });

  test('rejects unknown or mistyped settings', () {
    expect(() => CanoaConfig.parse('[desktop]\nstart_hidden = "yes"'),
        throwsFormatException);
    expect(() => CanoaConfig.parse('[network]\nconnect_timeout = 0'),
        throwsFormatException);
  });

  test('the app runs its own host unless canoa.cfg leaves it off, and saving keeps the choice', () {
    expect(const CanoaConfig().localHost, isTrue);
    expect(CanoaConfig.parse('').localHost, isTrue);
    final off = CanoaConfig.parse('[desktop]\nlocal_host = false\n');
    expect(off.localHost, isFalse);
    expect(CanoaConfig.parse(off.encode()).localHost, isFalse);
    expect(off.copyWith(startHidden: true).localHost, isFalse);
    expect(off.withHost(const {}).localHost, isFalse);
    expect(off.withProfileTables(overlays: const {}, tables: const {}).localHost, isFalse);
  });
  test('an overlay\'s hooks are allowed per agent at a digest, kept across a save, and refused for anything else', () {
    final digest = 'sha256:${'a' * 64}';
    final config = CanoaConfig.parse('[agents.Claude.hooks]\nteam = "$digest"\n');
    expect(config.agentHooks, {'Claude': {'team': digest}});
    final other = 'sha256:${'b' * 64}';
    final changed = config.withAgentHooks('Claude', {'team': other, 'mine': digest});
    expect(CanoaConfig.parse(changed.encode()).agentHooks, {'Claude': {'team': other, 'mine': digest}});
    expect(changed.withAgentHooks('Claude', const {}).agentHooks, isEmpty);
    expect(config.withAgentMcp('Claude', {'files': false}).agentHooks['Claude'], {'team': digest}, reason: 'mcp is separate');
    for (final bad in ['team = true', 'team = "sha256:12"', 'team = "yes"']) {
      expect(() => CanoaConfig.parse('[agents.Claude.hooks]\n$bad\n'), throwsFormatException, reason: bad);
    }
    expect(() => CanoaConfig.parse('[agents.Claude]\nhooks = 3\n'), throwsFormatException);
  });
  test('[appearance] open_in is this window unless it says new windows, and only those two are accepted', () {
    expect(const CanoaConfig().openIn, OpenIn.thisWindow);
    expect(const CanoaConfig().encode().contains('open_in'), isFalse);
    final chosen = CanoaConfig.parse('[appearance]\nopen_in = "new_windows"\n');
    expect(chosen.openIn, OpenIn.newWindows);
    expect(CanoaConfig.parse(chosen.encode()).openIn, OpenIn.newWindows);
    expect(chosen.copyWith(openIn: OpenIn.thisWindow).openIn, OpenIn.thisWindow);
    expect(chosen.withHost(const {}).openIn, OpenIn.newWindows, reason: 'another table written leaves it');
    expect(() => CanoaConfig.parse('[appearance]\nopen_in = "tabs"\n'), throwsFormatException);
    expect(() => CanoaConfig.parse('[appearance]\nopen_in = true\n'), throwsFormatException);
  });
}
