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
  test('rejects unknown or mistyped settings', () {
    expect(() => CanoaConfig.parse('[desktop]\nstart_hidden = "yes"'),
        throwsFormatException);
    expect(() => CanoaConfig.parse('[other]\na = 1'), throwsFormatException);
    expect(() => CanoaConfig.parse('[network]\nconnect_timeout = 0'),
        throwsFormatException);
  });
}
