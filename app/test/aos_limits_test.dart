// What a hostile overlay's text cannot do to the app, as canoa's tests/aos_limits.rs holds the host to it.
import 'dart:convert';

import 'package:canoa/profile/aos.dart';
import 'package:canoa/profile/overlay_layer.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('a value that doubles line after line is refused before it is large', () {
    final text = StringBuffer('A0=${'x' * 32}\n');
    for (var line = 1; line < 48; line++) {
      text.write('A$line=\${A${line - 1}}\${A${line - 1}}\n');
    }
    final clock = Stopwatch()..start();
    expect(() => readDotenv(utf8.encode('$text'), AosEnvironment()),
        throwsA(isA<AosException>().having((e) => e.code, 'code', 'too-large')));
    expect(clock.elapsed, lessThan(const Duration(seconds: 5)));
  });

  test('expressions nested past the limit are refused, not followed down the stack', () {
    const depth = 200000;
    final text = '${r'${U:-' * depth}x${'}' * depth}';
    expect(() => substitute(text, AosEnvironment().lookup),
        throwsA(isA<AosException>().having((e) => e.code, 'code', 'too-deep')));
  });

  test('a pattern that would take forever is refused quickly', () {
    final env = AosEnvironment()..set('A', 'a' * 20000);
    final clock = Stopwatch()..start();
    expect(() => substitute(r'${A//*z/}', env.lookup),
        throwsA(isA<AosException>().having((e) => e.code, 'code', 'too-complex')));
    expect(clock.elapsed, lessThan(const Duration(seconds: 5)));
    env.set('P', '/usr/bin:/bin');
    expect(substitute(r'${P//:/ }', env.lookup), '/usr/bin /bin');
  });

  test('only a loopback address or localhost is this computer for plain http', () {
    for (final remote in [
      'http://127.0.0.1.attacker.example/mcp', 'http://127.evil.example:80/x', 'http://localhost.attacker.example/',
      'http://127.0.0.256/x',
    ]) {
      expect(isInsecureUrl(remote), isTrue, reason: remote);
    }
    for (final local in [
      'http://127.0.0.1:9000/x', 'http://127.1.2.3/x', 'http://localhost/x', 'http://LOCALHOST:8080/',
      'http://[::1]:7000/x',
    ]) {
      expect(isInsecureUrl(local), isFalse, reason: local);
    }
  });

  test('a harness hint that is not one plain line is not applied', () {
    final layer = overlayLayer(identity: 'o', root: '/ov/o', os: 'linux', harness: 'claude-code', tree: [
      AosTreeEntry.file('overlay.cfg', utf8.encode('[harness.claude-code]\nmodel = "claude\\nfake: row"\neffort = "high"\n')),
    ]);
    expect(layer.profile.model, isNull);
    expect(layer.profile.effort, 'high');
    expect(layer.aos!.notApplied, contains(startsWith('harness.claude-code.model')));
  });

  test("a valid [about] icon is kept as the bytes the digest covered, and an invalid one is not", () {
    final png = base64Decode('iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAGUlEQVR42mNQc6v4TwlmGDVg1IBRA4aLAQD27eMQzhBbwQAAAABJRU5ErkJggg==');
    Map<String, List<int>> iconOf(String path, List<int> bytes) => {
      for (final entry in overlayLayer(identity: 'o', root: '/ov/o', os: 'linux', tree: [
        AosTreeEntry.file('overlay.cfg', utf8.encode('[about]\nicon = "$path"\n')),
        AosTreeEntry.file(path, bytes),
      ]).aos!.icon.entries)
        entry.key: entry.value,
    };
    expect(iconOf('icon.png', png), {'icon.png': png});
    expect(iconOf('icon.png', utf8.encode('<svg/>')), isEmpty, reason: 'not a PNG');
  });
}
