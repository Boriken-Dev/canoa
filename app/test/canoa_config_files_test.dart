// Where canoa.cfg is read from: the same cases as the host's `Layers` (crates/canoa-host/src/config/layers.rs).
import 'package:canoa/config/config_layers.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final windows in [false, true]) {
    final sep = windows ? ';' : ':';
    final dir = windows ? r'D:\work' : '/work';
    String at(String name) => windows ? '$dir\\$name' : '$dir/$name';
    final a = at('a.cfg'), b = at('b.cfg');
    final present = {a, b};
    ConfigFiles resolve(String? spec, {Map<String, String> env = const {}}) => ConfigFiles.resolve(
      env: {
        if (windows) ...{'APPDATA': r'C:\Users\me\AppData\Roaming', 'USERPROFILE': r'C:\Users\me'} else ...{'HOME': '/home/me'},
        ...env,
      },
      windows: windows,
      programDir: windows ? r'C:\Program Files\Canoa' : '/opt/canoa',
      exists: present.contains,
      explicit: spec,
    );
    final label = windows ? 'Windows' : 'POSIX';

    test('$label: none names the installed, system and user files', () {
      final files = resolve(null);
      expect(files.files.map((f) => f.origin), [ConfigOrigin.installed, ConfigOrigin.system, ConfigOrigin.own]);
      expect(files.files.first.path, windows ? r'C:\Program Files\Canoa\canoa.cfg' : '/opt/canoa/canoa.cfg');
      expect(files.files[1].path, windows ? r'C:\ProgramData\Canoa\canoa.cfg' : '/etc/Canoa/canoa.cfg');
      expect(files.own, windows ? r'C:\Users\me\AppData\Roaming\Canoa\canoa.cfg' : '/home/me/.config/Canoa/canoa.cfg');
      expect(files.hostArgs, isEmpty);
    });

    test('$label: CANOA_CONFIG is read when nothing is given', () {
      final files = resolve(null, env: {'CANOA_CONFIG': a});
      expect(files.own, a);
      expect(files.files.map((f) => f.path), [a]);
    });

    test('$label: one file, or several, are the only ones read, the last highest', () {
      final one = resolve(a);
      expect(one.files.map((f) => f.path), [a]);
      expect(one.own, a);
      expect(one.hostArgs, ['--config', a]);
      final two = resolve('$a$sep$b');
      expect(two.files.map((f) => f.path), [a, b]);
      expect(two.own, b);
      expect(two.checkSpec('X'), '$a${sep}X');
    });

    test('$label: a leading + puts them over the installed, system and user files', () {
      final plus = resolve('+$a$sep$b');
      expect(plus.files.map((f) => f.origin),
          [ConfigOrigin.installed, ConfigOrigin.system, ConfigOrigin.user, ConfigOrigin.listed, ConfigOrigin.own]);
      expect(plus.files.map((f) => f.path).skip(3), [a, b]);
      expect(plus.own, b);
      expect(plus.checkSpec('X'), '+$a${sep}X');
      expect(resolve(null).checkSpec('X'), '+X');
    });

    test('$label: an empty entry and a missing file are refused naming the entry', () {
      expect(() => resolve('$a$sep'), throwsA(isA<FormatException>().having((e) => e.message, 'message', contains('entry 2 is empty'))));
      expect(() => resolve('+'), throwsFormatException);
      expect(
        () => resolve('$a$sep${at('missing.cfg')}'),
        throwsA(isA<FormatException>().having((e) => e.message, 'message', allOf(contains('entry 2'), contains('missing.cfg')))),
      );
    });
  }
}
