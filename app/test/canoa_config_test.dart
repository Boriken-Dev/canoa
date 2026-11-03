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
  test('rejects unknown or mistyped settings', () {
    expect(() => CanoaConfig.parse('[desktop]\nstart_hidden = "yes"'),
        throwsFormatException);
    expect(() => CanoaConfig.parse('[other]\na = 1'), throwsFormatException);
    expect(() => CanoaConfig.parse('[network]\nconnect_timeout = 0'),
        throwsFormatException);
  });
}
