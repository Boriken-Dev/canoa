// canoa.cfg holds tables this app does not read (the host's `[secrets]`, `[settings.ID]` and `[say.NAME]`, an extension's own): the
// app reads the file whole, keeps them, and writes them back as it found them.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/config_layers.dart' show ConfigOrigin, decodeConfig;
import 'package:flutter_test/flutter_test.dart';

const _foreign = '''
[secrets]
default = "form"

[secrets.providers.work]
kind = "env"
variable = "WORK_TOKEN"

[settings.claude]
effort = "high"

[say.reader]
program = "piper"
speed = 1.5

[assets]
cdn = "local"
''';

void main() {
  group('a table the app does not read', () {
    test('is read, with the tables the app does own, instead of sending the whole file to defaults', () {
      final config = CanoaConfig.parse('[appearance]\nbrightness = "light"\n[desktop]\nstart_hidden = true\n$_foreign');
      expect(config.brightness, 'light');
      expect(config.startHidden, isTrue);
      expect(config.tables['secrets'], {
        'default': 'form',
        'providers': {
          'work': {'kind': 'env', 'variable': 'WORK_TOKEN'},
        },
      });
      expect(config.tables['settings'], {
        'claude': {'effort': 'high'},
      });
      expect(config.tables['say'], {
        'reader': {'program': 'piper', 'speed': 1.5},
      });
      expect(config.tables['assets'], {'cdn': 'local'});
    });

    test('is written back as found when Settings saves another table, in any of the three formats', () {
      final toml = CanoaConfig.parse(_foreign);
      final json = CanoaConfig.parse(
        '{"secrets": {"default": "form", "providers": {"work": {"kind": "env", "variable": "WORK_TOKEN"}}}, '
        '"settings": {"claude": {"effort": "high"}}, "say": {"reader": {"program": "piper", "speed": 1.5}}, "assets": {"cdn": "local"}}',
        name: 'canoa.json',
      );
      final yaml = CanoaConfig.parse(
        'secrets:\n  default: form\n  providers:\n    work: {kind: env, variable: WORK_TOKEN}\n'
        'settings:\n  claude: {effort: high}\nsay:\n  reader: {program: piper, speed: 1.5}\nassets: {cdn: local}\n',
        name: 'canoa.yaml',
      );
      for (final read in [toml, json, yaml]) {
        final saved = read.copyWith(brightness: 'light', connectTimeout: 300);
        final again = CanoaConfig.parse(saved.encode(), name: read.source.marked ? null : 'canoa.${read.source.format.name}');
        for (final table in ['secrets', 'settings', 'say', 'assets']) {
          expect(again.tables[table], toml.tables[table], reason: '${read.source.format.name}: [$table]');
        }
        expect(again.brightness, 'light');
        expect(again.connectTimeout, 300);
      }
    });

    test('is not copied into the person\'s file from a lower layer', () {
      final lower = CanoaConfig.parse(_foreign);
      final own = CanoaConfig.parse(
        '[appearance]\nbrightness = "light"\n',
        below: [(origin: ConfigOrigin.system, data: lower.source.own)],
      );
      expect(own.tables['secrets'], lower.tables['secrets'], reason: 'a lower layer\'s table shows through');
      final saved = CanoaConfig.parse(own.copyWith(brightness: 'dark').encode());
      expect(saved.tables.containsKey('secrets'), isFalse);
    });

    test('keeps a value outside every table, before the first header', () {
      final config = CanoaConfig.parse('title = "mine"\n[desktop]\nstart_hidden = true\n');
      expect(config.tables['title'], 'mine');
      final again = CanoaConfig.parse(config.copyWith(brightness: 'light').encode());
      expect(again.tables['title'], 'mine');
      expect(again.startHidden, isTrue, reason: 'it is not read as a key of the table before it');
      expect(again.brightness, 'light');
    });
  });

  group('a key the app does not know in a table it edits', () {
    const text = '''
[desktop]
start_hidden = true
tray_flavor = "mint"

[voice]
mode = "hold"
new_engine = 3

[sessions]
keep = 7

[sessions.later]
depth = 2
''';

    Map<String, dynamic> written(CanoaConfig config) => decodeConfig(config.encode()).data;

    test('opens, is not read, and is written back with its table as found', () {
      final config = CanoaConfig.parse(text);
      expect(config.startHidden, isTrue);
      expect(config.voice.mode, DictationMode.hold);
      expect(config.sessions.keep, 7);
      final saved = written(config.copyWith(brightness: 'light'));
      expect((saved['desktop'] as Map)['tray_flavor'], 'mint');
      expect((saved['desktop'] as Map)['start_hidden'], true);
      expect((saved['voice'] as Map)['new_engine'], 3);
      expect((saved['sessions'] as Map)['later'], {'depth': 2});
      expect((saved['appearance'] as Map)['brightness'], 'light');
    });

    test('is kept when the app changes a key beside it, and in the other formats', () {
      final json = CanoaConfig.parse('{"desktop": {"start_hidden": true, "tray_flavor": "mint"}}', name: 'canoa.json');
      expect(((decodeConfig(json.copyWith(startHidden: false).encode(), name: 'canoa.json').data)['desktop'] as Map)['tray_flavor'], 'mint');
      final yaml = CanoaConfig.parse('desktop:\n  tray_flavor: mint\n', name: 'canoa.yaml');
      expect(((decodeConfig(yaml.copyWith(startHidden: true).encode(), name: 'canoa.yaml').data)['desktop'] as Map)['tray_flavor'], 'mint');
    });

    test('is not copied into the person\'s file from a lower layer, and is kept where the person wrote it', () {
      final lower = CanoaConfig.parse('[desktop]\ntray_flavor = "mint"\n');
      final below = [(origin: ConfigOrigin.system, data: lower.source.own)];
      final inherited = CanoaConfig.parse('[appearance]\nbrightness = "light"\n', below: below);
      expect(written(inherited.copyWith(brightness: 'dark')).containsKey('desktop'), isFalse);
      final own = CanoaConfig.parse('[desktop]\ntray_flavor = "lime"\n', below: below);
      expect(((written(own.copyWith(brightness: 'dark')))['desktop'] as Map)['tray_flavor'], 'lime');
    });

    test('does not loosen the value of a key the app reads', () {
      expect(() => CanoaConfig.parse('[desktop]\nstart_hidden = "yes"\ntray_flavor = 1\n'), throwsFormatException);
    });
  });

  group('a number the host does not limit', () {
    test('is taken as written: any connect_timeout and output_speed above 0', () {
      final config = CanoaConfig.parse('[network]\nconnect_timeout = 300\n[voice]\noutput_speed = 15\n');
      expect(config.connectTimeout, 300);
      expect(config.connectTimeoutDuration, const Duration(minutes: 5));
      expect(config.voice.speak.speed, 15);
      expect(CanoaConfig.parse('[network]\nconnect_timeout = 0.5\n').connectTimeoutDuration, const Duration(milliseconds: 500));
      final again = CanoaConfig.parse(config.encode());
      expect(again.connectTimeout, 300);
      expect(again.voice.speak.speed, 15);
    });

    test('still refuses what is not a number above 0', () {
      for (final bad in ['0', '-3', '"soon"', 'true']) {
        expect(() => CanoaConfig.parse('[network]\nconnect_timeout = $bad\n'), throwsFormatException, reason: bad);
      }
      expect(() => CanoaConfig.parse('[voice]\noutput_speed = 0\n'), throwsFormatException);
    });
  });
}
