// The Agent Overlay Specification's conformance vectors, read from crates/canoa-host/tests/vectors/aos/ as canoa-host
// reads them (tests/aos_vectors.rs). A vector Canoa does not apply is skipped by name, and the skips are printed.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/profile/aos.dart';
import 'package:flutter_test/flutter_test.dart';

List<Map<String, dynamic>> _vectors(String name) =>
    ((jsonDecode(File('../crates/canoa-host/tests/vectors/aos/$name.json').readAsStringSync()) as Map)['cases'] as List)
        .cast<Map<String, dynamic>>();

/// Equal with types: `30` and `30.0` differ, and so do `1` and `true`.
bool _same(Object? a, Object? b) {
  if (a is Map && b is Map) return a.length == b.length && a.keys.every((key) => b.containsKey(key) && _same(a[key], b[key]));
  if (a is List && b is List) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (!_same(a[i], b[i])) return false;
    }
    return true;
  }
  return a.runtimeType == b.runtimeType && a == b;
}

bool _classMatches(Object? expected, String code) => expected is List ? expected.contains(code) : expected == code;

List<int> _bytes(Map<dynamic, dynamic> item) =>
    item['text'] is String ? utf8.encode(item['text'] as String) : base64Decode(item['base64'] as String);

List<String> _strings(Object? value) => [for (final item in (value as List?) ?? const []) '$item'];

AosEnvironment _environment(Object? values, {bool windows = false, List<String> secret = const []}) {
  final env = AosEnvironment(windows: windows);
  for (final MapEntry(:key, :value) in ((values as Map?) ?? const {}).entries) {
    env.set('$key', '$value', secret: secret.contains(key));
  }
  return env;
}

final _skipped = <String, List<(String, String)>>{};

void _skip(String file, String id, String why) => (_skipped[file] ??= []).add((id, why));

(Set<String>, Map<String, String>) _tree(Object? files) => switch (files) {
  final Map<dynamic, dynamic> texts => ({for (final key in texts.keys) '$key'}, {
    for (final MapEntry(:key, :value) in texts.entries) '$key': '$value',
  }),
  final List<dynamic> paths => ({for (final path in paths) '$path'}, const {}),
  _ => (const {}, const {}),
};

AosFacts _facts(Map<dynamic, dynamic> value) {
  Map<String, String> map(String key) => {
    for (final MapEntry(:key, :value) in ((value[key] as Map?) ?? const {}).entries) '$key': '$value',
  };
  return AosFacts(
    value['os'] as String,
    grants: {for (final MapEntry(:key, :value) in ((value['grants'] as Map?) ?? const {}).entries) '$key': _strings(value)},
    hostEnv: map('host_env'),
    secrets: map('secrets'),
    secretFiles: value['secret_files'] is Map ? map('secret_files') : null,
    providers: value.containsKey('providers') ? _strings(value['providers']) : const ['env', 'file'],
  );
}

const _appliedWeakenings = {'unpinned', 'env-sensitive', 'env-passthrough', 'secret-read', 'insecure-transport'};
const _envErrors = {
  'unresolved-reference', 'invalid-reference', 'missing-file', 'invalid-env-file', 'env-path-conflict',
  'reserved-variable', 'undefined-variable', 'secret-placeholder', 'invalid-pattern',
};

List<List<String>> _weakeningList(Iterable<AosWeakening> list) {
  final set = {for (final w in list) if (_appliedWeakenings.contains(w.id)) '${w.id}\u0000${w.subject}'}.toList()..sort();
  return [for (final pair in set) pair.split('\u0000')];
}

void main() {
  tearDownAll(() {
    for (final MapEntry(key: file, value: skips) in _skipped.entries) {
      // ignore: avoid_print
      print('$file: ${skips.length} skipped');
      for (final (id, why) in skips) {
        // ignore: avoid_print
        print('  skipped $id: $why');
      }
    }
  });

  group('reading', () {
    for (final vector in _vectors('reading')) {
      final id = vector['id'] as String;
      final format = vector['format'] as String;
      final bytes = _bytes(vector);
      final expect_ = vector['expect'] as Map;
      if (format == 'env-script') {
        _skip('reading', id, 'Canoa runs no env script, so reads no env script output');
        continue;
      }
      if (format == 'catalog') {
        _skip('reading', id, 'a catalog: the store reads catalogs');
        continue;
      }
      final firstLine = utf8.decode(bytes, allowMalformed: true).replaceFirst('\u{feff}', '').split('\n').first;
      final words = firstLine.startsWith('#!') ? firstLine.substring(2).trim().split(RegExp(r'\s+')) : const <String>[];
      final word = words.isEmpty ? '' : words.last.split(RegExp(r'[/\\]')).last.replaceFirst(RegExp(r'^\.+'), '');
      final yaml = const {'yaml', 'yml'}.contains(word.toLowerCase());
      test(id, () {
        Object? data;
        String? error;
        try {
          if (format == 'aos') {
            final limits = vector['limits'] as Map?;
            data = readDocument(bytes,
                limits: AosLimits(maxBytes: (limits?['max_bytes'] as int?) ?? 1024 * 1024,
                    maxDepth: (limits?['max_depth'] as int?) ?? 64));
          } else {
            final env = _environment(vector['env'],
                windows: vector['os'] == 'windows', secret: _strings(vector['secret_variables']));
            data = readDotenv(bytes, env);
          }
        } on AosException catch (refusal) {
          error = refusal.code;
        }
        if (yaml) {
          expect(error, 'unsupported-format', reason: 'YAML is refused');
          return;
        }
        if (error != null) {
          expect(_classMatches(expect_['error'], error), isTrue, reason: 'refused as $error; expected $expect_');
        } else {
          expect(expect_.containsKey('data'), isTrue, reason: 'read, and the vector refuses it');
          expect(_same(data, expect_['data']), isTrue, reason: 'read $data; expected ${expect_['data']}');
        }
      });
      if (yaml) _skip('reading', id, 'YAML, which Canoa refuses as an unsupported format');
    }
  });

  group('validation', () {
    for (final vector in _vectors('validation')) {
      test(vector['id'], () {
        final files = vector['files'] == null ? null : _strings(vector['files']).toSet();
        final issues = validate((vector['document'] as Map).cast<String, Object?>(), files: files);
        final expected = vector['expect'] as Map;
        if (expected['applies'] case final bool value) expect(applies(issues), value, reason: '$issues');
        if (expected['valid'] == true) {
          expect(issues, isEmpty);
        } else {
          final at = expected['at'] as String;
          expect(issues.any((issue) => issue.at == at || issue.at.startsWith('$at/')), isTrue,
              reason: 'a problem at $at; found $issues');
        }
      });
    }
  });

  group('names', () {
    for (final vector in _vectors('names')) {
      test(vector['id'], () {
        final name = vector['name'] as String;
        final expected = vector['expect'] as Map;
        expect(validOverlayName(name), expected['valid']);
        if (expected['valid'] == true) {
          expect(normalizeName(name), expected['normalized']);
          expect(rootVariable(name), expected['variable']);
        }
      });
    }
  });

  group('sources', () {
    for (final vector in _vectors('source')) {
      test(vector['id'], () {
        final expected = vector['expect'] as Map;
        try {
          final source = parseSource(vector['source'] as String);
          expect({'kind': source.kind.name, 'location': source.location, 'ref': source.reference, 'path': source.path},
              expected);
        } on AosException catch (refusal) {
          expect(_classMatches(expected['error'], refusal.code), isTrue, reason: '${refusal.code}; expected $expected');
        }
      });
    }
  });

  group('digest', () {
    for (final vector in _vectors('digest')) {
      test(vector['id'], () {
        final entries = [
          for (final item in (vector['tree'] as List).cast<Map<dynamic, dynamic>>())
            AosTreeEntry(
              [for (final name in _split(item['path'] is String ? utf8.encode(item['path'] as String) : base64Decode(item['path_base64'] as String))) name],
              switch (item['type']) {
                'dir' => AosEntryKind.dir,
                'symlink' => AosEntryKind.symlink,
                'special' => AosEntryKind.special,
                _ => AosEntryKind.file,
              },
              item.containsKey('text') || item.containsKey('base64') ? _bytes(item) : const [],
            ),
        ];
        final expected = vector['expect'] as Map;
        try {
          final tree = digestEntries(entries);
          expect(tree.digest, expected['digest']);
          expect([for (final line in tree.lines) utf8.decode(line)], expected['lines']);
        } on AosException catch (refusal) {
          expect(_classMatches(expected['error'], refusal.code), isTrue, reason: '${refusal.code}; expected $expected');
        }
      });
    }
  });

  group('compose', () {
    for (final vector in _vectors('compose')) {
      final id = vector['id'] as String;
      final expected = vector['expect'] as Map;
      if (expected.containsKey('environments')) {
        _skip('compose', id, 'dependency environments: Canoa installs no dependencies');
        continue;
      }
      test(id, () {
        final manifests = {
          for (final MapEntry(:key, :value) in (vector['overlays'] as Map).entries)
            '$key': (value as Map).cast<String, Object?>(),
        };
        final List<String> visited;
        final List<(String, String)> missing;
        try {
          (visited, missing) = resolveRequires(manifests, _strings(vector['requested']));
        } on AosCycle catch (cycle) {
          expect(expected['error'], 'cycle');
          expect(cycle.chain, expected['chain']);
          return;
        }
        final active = compositionOrder(manifests, visited);
        expect(active, expected['active']);
        final files = vector['files'] as Map?;
        Set<String> filesOf(String identity) => _tree(files?[identity]).$1;
        if (expected.containsKey('install_order')) {
          expect(visited, expected['install_order']);
          expect([for (final (by, name) in missing) [by, name]], expected['missing']);
          expect({for (final identity in active) identity: rootVariable(identity)}, expected['root_variables']);
          expect([for (final identity in active) ..._strings(manifests[identity]!['routing'])], expected['routing']);
          List<List<String>> each(List<String> Function(Map<String, Object?>, Set<String>) of) => [
            for (final identity in active)
              for (final path in of(manifests[identity]!, filesOf(identity))) [identity, path],
          ];
          expect(each(rulesOf), expected['rules']);
          expect(each(contextOf), expected['context']);
          final winners = <String, (String, String)>{};
          final collisions = <String>{};
          for (final identity in active) {
            for (final name in skillsOf(manifests[identity]!, filesOf(identity))) {
              final folded = name.toLowerCase();
              if (winners.containsKey(folded)) collisions.add(folded);
              winners[folded] = (name, identity);
            }
          }
          expect({for (final (name, identity) in winners.values) name: identity}, expected['skills']);
          expect(collisions.toList()..sort(), expected['skill_collisions']);
        }
        if (expected['env'] case final Object want) {
          final facts = vector['facts'] as Map;
          final env = _environment(facts['base_env'], windows: facts['os'] == 'windows');
          for (final identity in active) {
            env.set(rootVariable(identity), (facts['roots'] as Map)[identity] as String);
          }
          try {
            for (final identity in active) {
              final (tree, texts) = _tree(files?[identity]);
              applyOverlay(
                env,
                AosOverlay(identity: identity, manifest: manifests[identity]!, files: tree,
                    root: (facts['roots'] as Map)[identity] as String, read: (path) => texts[path] == null ? null : utf8.encode(texts[path]!)),
                _facts(facts),
                [],
              );
            }
            expect(env.values, want);
          } on AosException catch (refusal) {
            expect({'error': refusal.code, 'subject': refusal.subject}, want);
          }
        }
      });
    }
  });

  group('effective', () {
    for (final vector in _vectors('effective')) {
      final id = vector['id'] as String;
      final expected = vector['expect'] as Map;
      final facts = vector['facts'] as Map;
      if (((facts['packages'] as Map?)?.isNotEmpty ?? false) && ((expected['dependencies'] as List?)?.isNotEmpty ?? false)) {
        _skip('effective', id, 'installed packages: Canoa installs no dependencies, so adds no package directory');
        continue;
      }
      if (expected['error'] case final String code when !_envErrors.contains(code)) {
        _skip('effective', id, 'a refusal of setup or dependencies, which Canoa does not run');
        continue;
      }
      test(id, () {
        final manifest = (vector['manifest'] as Map).cast<String, Object?>();
        final aosFacts = _facts(facts);
        final (tree, texts) = _tree(facts['files']);
        final identity = facts['identity'] as String;
        final root = facts['root'] as String;
        final env = _environment(facts['base_env'], windows: aosFacts.os == 'windows');
        env.set(rootVariable(identity), root);
        if (facts['state'] case final String state) env.set(stateVariable(identity), state);
        final found = <AosWeakening>[if (facts['pinned'] != true) const AosWeakening('unpinned', '')];
        try {
          applyOverlay(
            env,
            AosOverlay(identity: identity, manifest: manifest, files: tree, root: root, state: facts['state'] as String?,
                read: (path) => texts[path] == null ? null : utf8.encode(texts[path]!)),
            aosFacts,
            found,
          );
        } on AosException catch (refusal) {
          expect({'error': refusal.code, 'subject': refusal.subject}, {'error': expected['error'], 'subject': expected['subject']});
          return;
        }
        expect(expected.containsKey('error'), isFalse, reason: 'applied, and the vector refuses it');
        final ungranted = grantsReport(manifest, aosFacts, found);
        expect(env.values, expected['env']);
        expect(ungranted, expected['ungranted']);
        expect(_weakeningList(found), _weakeningList([
          for (final pair in (expected['weakenings'] as List).cast<List<dynamic>>()) AosWeakening('${pair[0]}', '${pair[1]}'),
        ]));
      });
    }
  });

  group('catalog', () {
    for (final vector in _vectors('catalog')) {
      final id = vector['id'] as String;
      final icon = vector['icon'] as Map?;
      if (icon == null) {
        _skip('catalog', id, 'a catalog: the store reads catalogs');
        continue;
      }
      test(id, () {
        final limits = icon['limits'] as Map?;
        final expected = vector['expect'] as Map;
        try {
          final header = checkIcon(icon['name'] as String, base64Decode(icon['base64'] as String),
              limits: IconLimits(
                maxBytes: (limits?['max_bytes'] as int?) ?? 512 * 1024,
                maxWidth: (limits?['max_width'] as int?) ?? 1024,
                maxHeight: (limits?['max_height'] as int?) ?? 1024,
              ));
          expect({'format': header.format, 'width': header.width, 'height': header.height}, expected);
        } on AosException catch (refusal) {
          expect(refusal.code, expected['error']);
        }
      });
    }
  });
}

List<List<int>> _split(List<int> path) {
  final out = <List<int>>[[]];
  for (final byte in path) {
    if (byte == 0x2f) {
      out.add([]);
    } else {
      out.last.add(byte);
    }
  }
  return out;
}
