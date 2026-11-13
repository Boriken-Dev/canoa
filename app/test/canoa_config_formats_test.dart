// canoa.cfg in TOML, YAML or JSON, named by its first line (or by its name), and read as layers the host reads the same way.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/config_layers.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:flutter_test/flutter_test.dart';

String _digest() => 'sha256:${'0' * 64}';

Map<String, dynamic> _rich() => {
  'desktop': {'start_hidden': true, 'close_to_tray': true},
  'appearance': {'look': 'canoa', 'brightness': 'dark'},
  'sessions': {'keep': 200},
  'notifications': {'finished': true},
  'permissions': {'default_mode': 'manual'},
  'sources': {'hidden': ['Gemini']},
  'usage': {'warn_permille': 800, 'limit_phrases': ['rate limit']},
  'catalogs': {'team': 'https://example.org/o.git@main#catalog.toml'},
  'store': {'catalog_max_bytes': 4194304},
  'skill_pins': {'review': _digest()},
  'host': {
    'origins': ['https://app.example'],
    'agents': {
      'claude': {'program': 'npx', 'args': ['-y', 'x'], 'listen': '127.0.0.1:7801'},
    },
  },
};

// The tables Settings leaves alone, whichever section changed.
const _untouched = ['catalogs', 'store', 'skill_pins'];

CanoaConfig _sessions(CanoaConfig c, int keep) => c.copyWith(
  sessions: LedgerSettings(
    keep: keep,
    keepText: c.sessions.keepText,
    textMessages: c.sessions.textMessages,
    workspace: c.sessions.workspace,
    harnesses: c.sessions.harnesses,
  ),
);

final _changes = <String, ({CanoaConfig Function(CanoaConfig) change, bool Function(CanoaConfig) shown})>{
  'desktop': (change: (c) => c.copyWith(startHidden: false), shown: (c) => !c.startHidden),
  'appearance': (change: (c) => c.copyWith(brightness: 'light'), shown: (c) => c.brightness == 'light'),
  'sessions': (change: (c) => _sessions(c, 7), shown: (c) => c.sessions.keep == 7),
  'permissions': (change: (c) => c.copyWith(defaultMode: 'plan'), shown: (c) => c.defaultMode == 'plan'),
  'sources': (change: (c) => c.copyWith(hiddenSources: {'Gemini', 'Grok'}), shown: (c) => c.hiddenSources.contains('Grok')),
  'usage': (change: (c) => c.copyWith(warnPermille: 900), shown: (c) => c.warnPermille == 900),
  'host': (
    change: (c) => c.withHostAgent(name: 'mine', program: 'agent', listen: '127.0.0.1:7811'),
    shown: (c) => c.hostEntries.any((entry) => entry.name == 'mine'),
  ),
  'catalogs': (
    change: (c) => c.withCatalog('mine', 'https://example.org/p.git@main#catalog.toml'),
    shown: (c) => (c.tables['catalogs'] as Map).containsKey('mine'),
  ),
};

void main() {
  group('the format a file names', () {
    test('the first line chooses, as anycfg does, and none means TOML', () {
      expect(decodeConfig('[desktop]\nstart_hidden = true\n').format, ConfigFormat.toml);
      expect(decodeConfig('#!toml\n[desktop]\n').marked, isTrue);
      expect(decodeConfig('#!yaml\ndesktop: {start_hidden: true}\n').format, ConfigFormat.yaml);
      expect(decodeConfig('#!/usr/bin/env yaml\ndesktop: {}\n').format, ConfigFormat.yaml);
      expect(decodeConfig('#!.JSON\n{"desktop": {}}\n').format, ConfigFormat.json);
      expect(decodeConfig('#!yml\n{}\n').format, ConfigFormat.yaml);
    });

    test('a name with an extension fixes the format, and its first line is not read', () {
      expect(decodeConfig('{"desktop": {}}', name: 'canoa.json').format, ConfigFormat.json);
      expect(decodeConfig('desktop:\n  start_hidden: true\n', name: r'C:\x\canoa.yml').format, ConfigFormat.yaml);
      expect(decodeConfig('', name: 'canoa.yaml').data, isEmpty);
      expect(() => decodeConfig('{}', name: 'canoa.ini'), throwsFormatException);
    });

    test('an unknown format is refused naming the ones read, and no value', () {
      try {
        decodeConfig('#!ini\n[host]\n');
        fail('refused');
      } on FormatException catch (error) {
        expect(error.message, allOf(contains('toml'), contains('yaml'), contains('json'), isNot(contains('ini'))));
      }
    });

    test('a wrong document is refused without quoting what it holds', () {
      for (final text in ['#!json\n{"a": SECRETSECRET}\n', '#!yaml\na: [SECRETSECRET\n', '#!json\n["SECRETSECRET"]\n', '#!yaml\nhost: {a: [null]}\n']) {
        try {
          decodeConfig(text);
          fail('refused: $text');
        } on FormatException catch (error) {
          expect(error.message, isNot(contains('SECRET')), reason: text);
        }
      }
    });

    test('YAML and JSON have no null: a key holding one is not set', () {
      expect(decodeConfig('#!json\n{"desktop": {"start_hidden": null}}\n').data, {'desktop': <String, dynamic>{}});
      expect(decodeConfig('#!yaml\ndesktop:\n  start_hidden: ~\n').data, {'desktop': <String, dynamic>{}});
    });
  });

  group('each Settings section keeps the other tables and the marker, in YAML and in JSON', () {
    final named = <(String, ConfigFormat, bool, String?)>[
      ('YAML with its marker', ConfigFormat.yaml, true, null),
      ('JSON with its marker', ConfigFormat.json, true, null),
      ('TOML with its marker', ConfigFormat.toml, true, null),
      ('a .yaml file', ConfigFormat.yaml, false, 'canoa.yaml'),
      ('a .json file', ConfigFormat.json, false, 'canoa.json'),
    ];
    for (final (label, format, marked, name) in named) {
      for (final MapEntry(key: section, value: change) in _changes.entries) {
        test('$label: $section', () {
          final text = encodeConfig(_rich(), format, marked: marked);
          if (marked) expect(text, startsWith('#!${format.name}\n'));
          final read = CanoaConfig.parse(text, name: name);
          expect(read.source.format, format);
          final saved = change.change(read).encode();
          if (marked) expect(saved, startsWith('#!${format.name}\n'), reason: saved);
          if (!marked && format == ConfigFormat.json) expect(jsonDecode(saved), isA<Map>());
          final back = decodeConfig(saved, name: name);
          expect(back.format, format);
          expect(back.marked, marked);
          expect(change.shown(CanoaConfig.parse(saved, name: name)), isTrue);
          for (final table in _untouched.where((table) => table != section)) {
            expect(back.data[table], _rich()[table], reason: '$table after $section');
          }
          expect((back.data['host'] as Map)['origins'], ['https://app.example']);
        });
      }
    }

    test('saving settings that changed nothing still reads back the same', () {
      for (final format in ConfigFormat.values) {
        final text = encodeConfig(_rich(), format, marked: true);
        final once = CanoaConfig.parse(text).encode();
        expect(CanoaConfig.parse(once).encode(), once, reason: format.name);
      }
    });
  });

  group('layers', () {
    final layersDir = '${Directory.current.path}/../examples/config/layers';
    Map<String, dynamic> layer(String name) => decodeConfig(File('$layersDir/$name').readAsStringSync(), name: name).data;

    test('three files in three formats merge as the host merges them', () {
      var merged = <String, dynamic>{};
      for (final name in ['installed.cfg', 'system.cfg', 'user.cfg']) {
        merged = mergeConfig(merged, layer(name));
      }
      expect(merged, jsonDecode(File('$layersDir/merged.json').readAsStringSync()));
    });

    test('the app shows the merged values and marks the ones a lower layer gives', () {
      final below = <ConfigLayer>[
        (origin: ConfigOrigin.installed, data: layer('installed.cfg')),
        (origin: ConfigOrigin.system, data: layer('system.cfg')),
      ];
      final read = CanoaConfig.parse(File('$layersDir/user.cfg').readAsStringSync(), name: 'canoa.cfg', below: below);
      expect(read.source.format, ConfigFormat.json);
      expect(read.startHidden, isFalse, reason: 'the system file, over the installed one');
      expect(read.closeToTray, isTrue);
      expect(read.brightness, 'light', reason: 'the person\'s own');
      expect(read.sessions.keep, 50);
      expect(read.lowerLayerOf(['desktop', 'start_hidden']), 'the system file');
      expect(read.lowerLayerOf(['desktop', 'close_to_tray']), 'the installed file');
      expect(read.lowerLayerOf(['appearance', 'brightness']), isNull);
      expect(read.lowerLayerOf(['appearance', 'look']), 'the installed file');
      expect(read.overridesLowerLayer(['appearance', 'brightness']), isTrue);
    });

    test('a save writes the person\'s own file only, never a lower layer\'s value', () {
      final below = <ConfigLayer>[
        (origin: ConfigOrigin.installed, data: layer('installed.cfg')),
        (origin: ConfigOrigin.system, data: layer('system.cfg')),
      ];
      final read = CanoaConfig.parse(File('$layersDir/user.cfg').readAsStringSync(), name: 'canoa.cfg', below: below);
      final saved = read.copyWith(closeToTray: false).encode();
      expect(saved, startsWith('#!json\n'));
      final own = decodeConfig(saved).data;
      expect((own['desktop'] as Map)['close_to_tray'], isFalse);
      expect((own['desktop'] as Map).containsKey('start_hidden'), isFalse, reason: 'a system value is not copied');
      expect((own['appearance'] as Map)['brightness'], 'light');
      expect((own['appearance'] as Map).containsKey('look'), isFalse, reason: 'an installed value is not copied');
      expect((own['sessions'] as Map)['keep'], 50);
      expect((own['sessions'] as Map).containsKey('keep_text'), isFalse);
      expect(own.containsKey('usage'), isFalse);
      expect(own.containsKey('agents'), isFalse);
      expect(((own['host'] as Map)['agents'] as Map).keys, ['beta'], reason: 'entries the person did not touch stay below');
    });

    test('resetting a key in the person\'s file falls back to the lower layer', () {
      final below = <ConfigLayer>[
        (origin: ConfigOrigin.installed, data: layer('installed.cfg')),
        (origin: ConfigOrigin.system, data: layer('system.cfg')),
      ];
      final read = CanoaConfig.parse(File('$layersDir/user.cfg').readAsStringSync(), name: 'canoa.cfg', below: below);
      final reset = read.resetKey(['sessions', 'keep']);
      expect(reset.sessions.keep, 500, reason: 'the system file\'s');
      expect(reset.lowerLayerOf(['sessions', 'keep']), 'the system file');
      expect(decodeConfig(reset.encode()).data['sessions'], isNull);
      expect(reset.resetKey(['appearance', 'brightness']).brightness, 'dark');
    });
  });

  group('the examples', () {
    final examples = '${Directory.current.path}/../examples/config';

    test('the YAML and JSON examples load to the same values the host reads from them', () {
      final yaml = CanoaConfig.parse(File('$examples/canoa-yaml.cfg').readAsStringSync());
      final json = CanoaConfig.parse(File('$examples/canoa-json.cfg').readAsStringSync());
      expect(yaml.source.format, ConfigFormat.yaml);
      expect(json.source.format, ConfigFormat.json);
      for (final read in [yaml, json]) {
        expect(read.startHidden, isFalse);
        expect(read.closeToTray, isTrue);
        expect(read.brightness, 'dark');
        expect(read.sessions.keep, 200);
        expect(read.limitPhrases, ['RESOURCE_EXHAUSTED', 'Error 429']);
        expect(read.hostEntries.map((entry) => entry.name), ['claude']);
        expect(read.host!['origins'], ['https://app.example']);
      }
      expect(yaml.encode().replaceFirst('#!yaml', ''), isNot(contains('#!json')));
      expect(CanoaConfig.parse(File('$examples/canoa.cfg').readAsStringSync()).source.format, ConfigFormat.toml);
    });
  });
}
