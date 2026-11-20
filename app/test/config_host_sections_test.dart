// The tables a page and the app the host runs in read and write on the host are the ones the host names
// (crates/canoa-host/src/control/files.rs): a table one has and the other lacks is never read or never saved.
import 'dart:io';

import 'package:canoa/served/host_config.dart';
import 'package:flutter_test/flutter_test.dart';

Set<String> _namesIn(String constant) {
  final source = File('../crates/canoa-host/src/control/files.rs').readAsStringSync();
  final body = RegExp('const $constant: &\\[&str\\] = &\\[(.*?)\\];', dotAll: true).firstMatch(source);
  expect(body, isNotNull, reason: '$constant is not found in the host');
  return {for (final name in RegExp(r'"(\w+)"').allMatches(body!.group(1)!)) name.group(1)!};
}

void main() {
  test('a page reads and saves the host\'s SECTIONS, and the app the host runs in the OWNER_SECTIONS besides', () {
    expect(hostConfigSections.toSet(), _namesIn('SECTIONS'));
    expect(hostConfigSections.toSet().length, hostConfigSections.length, reason: 'a table is listed once');
    expect(ownerSections.toSet(), _namesIn('OWNER_SECTIONS'));
    expect(ownerSections.toSet().intersection(hostConfigSections.toSet()), isEmpty);
  });
}
