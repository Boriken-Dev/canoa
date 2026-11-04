import 'package:canoa/config/canoa_config.dart';
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
[assets]
cdn = "local"
''');
    expect(parsed.startHidden, true);
    expect(parsed.preferWebTransport, false);
    expect(CanoaConfig.parse(parsed.encode()).cdn, 'local');
  });
  test('saving Settings or pinning an overlay keeps full access and the notifications', () {
    final config = CanoaConfig.parse('[permissions]\nfull_access = ["Claude"]\n[notifications]\nfinished = false\n'
        '[overlays]\nmine = { dir = "mine" }\n');
    for (final saved in [config.withProfilesOf(config), config.withOverlayPin('mine', 'sha256:${'0' * 64}')]) {
      expect(saved.fullAccess, {'Claude'});
      expect(saved.notifications.finished, isFalse);
      final reread = CanoaConfig.parse(saved.encode());
      expect(reread.fullAccess, {'Claude'});
      expect(reread.notifications.finished, isFalse);
    }
  });

  test('keeps the host table canoa-host reads when settings are saved', () {
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
    expect(() => saved.withHostAgent(name: 'mine', program: 'x', listen: '127.0.0.1:1'), throwsArgumentError);
    expect(() => saved.withHostAgent(name: 'echo', program: 'x', listen: '127.0.0.1:1'), throwsArgumentError);
    expect(() => saved.withHostAgent(name: 'no spaces', program: 'x', listen: '127.0.0.1:1'), throwsArgumentError);
    final removed = CanoaConfig.parse(saved.withoutHostEntry('mine').encode());
    expect(removed.hostEntries.map((entry) => entry.name), ['echo']);
  });

  test('rejects unknown or mistyped settings', () {
    expect(() => CanoaConfig.parse('[desktop]\nstart_hidden = "yes"'),
        throwsFormatException);
    expect(() => CanoaConfig.parse('[other]\na = 1'), throwsFormatException);
    expect(() => CanoaConfig.parse('[network]\nconnect_timeout = 0'),
        throwsFormatException);
  });
}
