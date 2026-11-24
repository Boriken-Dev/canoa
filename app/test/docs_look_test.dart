import 'dart:convert';
import 'dart:io';

import 'package:canoa/theme/look.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

/// The default look's colours the documentation site is drawn in, `docs/assets/look.json`; `tools/docs_brand.py` reads it.
const _file = '../docs/assets/look.json';

String _hex(Color color) => '#${(color.toARGB32() & 0xFFFFFF).toRadixString(16).padLeft(6, '0').toUpperCase()}';

Map<String, String> _tokens(Brightness brightness) {
  final look = lookFor(LookId.canoa, brightness);
  return {
    'page': _hex(look.page),
    'rail': _hex(look.rail),
    'bar': _hex(look.bar),
    'surface': _hex(look.surface),
    'border': _hex(look.border),
    'text': _hex(look.text),
    'muted': _hex(look.muted),
    'accent': _hex(look.accent),
    'on_accent': _hex(look.onAccent),
  };
}

void main() {
  test('docs/assets/look.json holds the default look\'s colours', () {
    final text = '${const JsonEncoder.withIndent('  ').convert({'light': _tokens(Brightness.light), 'dark': _tokens(Brightness.dark)})}\n';
    if (Platform.environment['CANOA_WRITE_LOOK'] != null) File(_file).writeAsStringSync(text);
    expect(
      File(_file).readAsStringSync().replaceAll('\r\n', '\n'),
      text,
      reason: 'docs/assets/look.json is stale; regenerate it: (cd app && CANOA_WRITE_LOOK=1 flutter test test/docs_look_test.dart)',
    );
  });
}
