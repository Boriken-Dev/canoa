import 'dart:convert';
import 'dart:io';

import 'package:canoa/theme/look.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

/// The default look's colours the portal's pages are drawn in, a copy the `canoa-host` crate carries because it builds packaged
/// on its own.
const _file = '../crates/canoa-host/src/portal/brand/look.json';

String _hex(Color color) => '#${(color.toARGB32() & 0xFFFFFF).toRadixString(16).padLeft(6, '0').toUpperCase()}';

Map<String, String> _tokens(Brightness brightness) {
  final look = lookFor(LookId.canoa, brightness);
  return {
    'page': _hex(look.page),
    'card': _hex(look.bar),
    'field': _hex(look.raised),
    'line': _hex(look.border),
    'text': _hex(look.text),
    'muted': _hex(look.muted),
    'accent': _hex(look.accent),
    'on_accent': _hex(look.onAccent),
    'error': _hex(look.danger),
    'ok': _hex(look.success),
  };
}

void main() {
  test('the portal\'s look.json holds the default look\'s colours', () {
    final text = '${const JsonEncoder.withIndent('  ').convert({'light': _tokens(Brightness.light), 'dark': _tokens(Brightness.dark)})}\n';
    if (Platform.environment['CANOA_WRITE_LOOK'] != null) File(_file).writeAsStringSync(text);
    expect(
      File(_file).readAsStringSync().replaceAll('\r\n', '\n'),
      text,
      reason: 'the portal\'s look.json is stale; regenerate it: (cd app && CANOA_WRITE_LOOK=1 flutter test test/portal_look_test.dart)',
    );
  });
}
