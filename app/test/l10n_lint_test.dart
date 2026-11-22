// Words the app shows come from its text provider (`context.t`), which a look can replace and a language can translate. This finds
// the ones still written into a widget: a `Text('…')`, or a label, hint, tooltip, title or message given as a string literal.
//
// `l10n_baseline.json` counts them per file for the screens not converted yet. A file may not gain one, and a file that loses
// some must lower its count, so the list only shrinks. After converting a screen, or to see what is left:
//
//     L10N_WRITE_BASELINE=1 flutter test test/l10n_lint_test.dart
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

const _quoted = r'''(?:'((?:[^'\\\n]|\\.)*)'|"((?:[^"\\\n]|\\.)*)")''';
final _literal = RegExp(
  r'(?:\bText\(\s*(?:const\s+)?|\b(?:label|labelText|hintText|tooltip|title|helperText|semanticLabel|message|subtitle|description)\s*:\s*(?:const\s+)?)r?' + _quoted,
);

/// Whether [text] says something to a person: a letter once the interpolations are taken out.
bool _says(String text) => RegExp(r'[A-Za-z]').hasMatch(text.replaceAll(RegExp(r'\$\{[^}]*\}|\$\w+'), ''));

/// The user-visible literals of [source] that are words.
List<String> literalsIn(String source) => [
  for (final match in _literal.allMatches(source))
    if (_says(match.group(1) ?? match.group(2) ?? '')) match.group(1) ?? match.group(2)!,
];

void main() {
  test('what counts as a literal word', () {
    expect(literalsIn("Text('Save')"), ['Save']);
    expect(literalsIn("const Text(\n  'Save as'\n)"), ['Save as']);
    expect(literalsIn("ListTile(title: Text(\"Open\"), tooltip: 'Close it')"), ['Open', 'Close it']);
    expect(literalsIn("Text(context.t('x'))"), isEmpty);
    expect(literalsIn("Text('\$count')"), isEmpty);
    expect(literalsIn("Text('\${a} \${b}')"), isEmpty);
    expect(literalsIn("Text('→')"), isEmpty);
    expect(literalsIn("Text('\$name is gone')"), ['\$name is gone']);
    expect(literalsIn("label: context.t('composer.more')"), isEmpty);
  });

  test('no widget gains a literal word, and none keeps one that was converted', () {
    final counts = <String, int>{};
    for (final entity in Directory('lib').listSync(recursive: true)) {
      if (entity is! File || !entity.path.endsWith('.dart')) continue;
      final count = literalsIn(entity.readAsStringSync()).length;
      if (count > 0) counts[entity.path.replaceAll(r'\', '/')] = count;
    }
    final baselineFile = File('test/l10n_baseline.json');
    if (Platform.environment['L10N_WRITE_BASELINE'] == '1') {
      final sorted = {for (final key in counts.keys.toList()..sort()) key: counts[key]};
      baselineFile.writeAsStringSync('${const JsonEncoder.withIndent('  ').convert(sorted)}\n');
      return;
    }
    final baseline = (jsonDecode(baselineFile.readAsStringSync()) as Map<String, dynamic>).cast<String, int>();
    final problems = <String>[];
    for (final path in {...counts.keys, ...baseline.keys}) {
      final have = counts[path] ?? 0, allowed = baseline[path] ?? 0;
      if (have > allowed) problems.add('$path has $have literal words, $allowed allowed: use context.t');
      if (have < allowed) problems.add('$path has $have literal words, the baseline says $allowed: lower it (L10N_WRITE_BASELINE=1)');
    }
    expect(problems, isEmpty, reason: problems.join('\n'));
  });
}
