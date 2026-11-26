// A build's names come from its brand: no string in `lib/` spells the product's name, its configuration file or its command line.
// What stays written out is a protocol's own identifier (the namespace SDKs, the browser extension and the host share) or a
// code name that is not shown (a look's id, a key), listed below with why.
import 'dart:io';

import 'package:canoa/brand.dart';
import 'package:canoa/config/config_layers.dart';
import 'package:canoa/worktree/worktree.dart';
import 'package:flutter_test/flutter_test.dart';

/// Substrings of a literal that name the protocol or a code identifier, not the product to a person.
const _wire = [
  'dev.boriken.canoa', // reverse-DNS identifiers: metadata keys, native messaging host, storage names
  '_canoa/', // the control methods the host and the app share
  'canoaBridge', // the browser extension's page bridge
  'canoa_noise', // the Noise module's file
  'canoa.json', // the discovery file a host serves
  'canoa.popout', // the channel between a window and its popouts
  'canoa.read', // the messages of reading aloud between windows
  'x-canoa-office-key', // the header the Office pane's key travels in
  'canoaId', // a field of the ledger's records
  'shells.origin.canoa', // a text key
  'canoa_config_io.dart', // a conditional import
  '{"canoa"', // the Office pane's messages
  'canoa.components', // a variable of the A2UI catalog
];

/// Whole literals that are an id, not a name: the default look, a rule's marker, the value a connection's kind takes.
const _ids = {'canoa'};

/// Files that hold a name on purpose: the brand itself, the look named for the product's design, and the fallback listing of
/// Settings pages the host generates for the default brand.
const _files = {'lib/brand.dart', 'lib/theme/look.dart', 'lib/settings/built_in_sections.dart'};

final _literal = RegExp(r'''r?(?:'((?:[^'\\\n]|\\.)*)'|"((?:[^"\\\n]|\\.)*)")''');

/// The literals of [source] that spell the product's names, outside comments and directives.
List<String> namedLiterals(String source) {
  final found = <String>[];
  for (final line in source.split('\n')) {
    final code = line.trimLeft();
    if (code.startsWith('//') || code.startsWith('import ') || code.startsWith('export ') || code.startsWith('part ')) continue;
    for (final match in _literal.allMatches(line)) {
      var text = (match.group(1) ?? match.group(2) ?? '').replaceAll(RegExp(r'\$\{[^}]*\}|\$\w+'), '');
      if (_ids.contains(text)) continue;
      for (final allowed in _wire) {
        text = text.replaceAll(allowed, '');
      }
      if (RegExp('canoa', caseSensitive: false).hasMatch(text)) found.add(match.group(0)!);
    }
  }
  return found;
}

void main() {
  test('what counts as a literal that names the product', () {
    expect(namedLiterals("throw FormatException('canoa.cfg: not valid');"), hasLength(1));
    expect(namedLiterals("Text('Canoa is ready')"), hasLength(1));
    expect(namedLiterals("run('canoa serve')"), hasLength(1));
    expect(namedLiterals("Text('\${Brand.name} is ready')"), isEmpty);
    expect(namedLiterals("Text('\${canoa.name} \$canoaFlag')"), isEmpty);
    expect(namedLiterals("const key = 'dev.boriken.canoa/account';"), isEmpty);
    expect(namedLiterals("// the canoa.cfg file\nimport 'package:canoa/brand.dart';"), isEmpty);
  });

  test('no string in lib spells a name the brand gives', () {
    final problems = <String>[];
    for (final entity in Directory('lib').listSync(recursive: true)) {
      if (entity is! File || !entity.path.endsWith('.dart')) continue;
      final path = entity.path.replaceAll(r'\', '/');
      if (_files.contains(path)) continue;
      for (final literal in namedLiterals(entity.readAsStringSync())) {
        problems.add('$path: $literal');
      }
    }
    expect(problems, isEmpty, reason: 'take the name from Brand:\n${problems.join('\n')}');
  });

  test('the names the app derives follow the brand', () {
    expect(worktreeBranchPrefix, '${Brand.program}/');
    expect(safeWorktreeBranch('${Brand.program}/fix-it'), isTrue);
    expect(safeWorktreeBranch('other/fix-it'), isFalse);
    final linux = ConfigFiles.resolve(
      env: {'HOME': '/home/p', 'XDG_CONFIG_HOME': '/home/p/.config'},
      windows: false,
      programDir: '/opt/app',
      exists: (_) => true,
    );
    expect(linux.own, '/home/p/.config/${Brand.folder}/${Brand.configFile}');
    expect(linux.files.first.path, '/opt/app/${Brand.configFile}');
    expect(linux.files[1].path, '/etc/${Brand.folder}/${Brand.configFile}');
    expect(
      () => decodeConfig('{', name: 'settings.json'),
      throwsA(isA<FormatException>().having((error) => error.message, 'message', startsWith('${Brand.configFile}: '))),
    );
  });
}
