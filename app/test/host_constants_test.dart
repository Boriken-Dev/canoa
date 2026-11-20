// The few facts the app keeps beside the host, since it shows them with no host (a page nothing served, a desktop whose host is
// stopped): each is the host's own, read here from the host's source, so a change in one without the other fails. What a host can
// say is read from it instead: the official MCP registry and the default catalog come from `store/catalogs`, and which kinds of
// client the host knows is the host's to refuse.
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:flutter_test/flutter_test.dart';

String _host(String file) => File('../crates/canoa-host/src/$file').readAsStringSync();

/// The text of the Rust constant [name] in [source]: the string it is set to, which may be on the next line.
String _constant(String source, String name) {
  final found = RegExp('const $name: &str =\\s*"([^"]+)";').firstMatch(source);
  expect(found, isNotNull, reason: '$name is not found in the host');
  return found!.group(1)!;
}

void main() {
  test('the catalog offered first is the host\'s', () {
    final index = _host('store/index.rs');
    expect(defaultCatalogLabel, _constant(index, 'DEFAULT_CATALOG_LABEL'));
    expect(defaultCatalogSpec, _constant(index, 'DEFAULT_CATALOG_SPEC'));
  });

  test('the skills directory a search asks is the host\'s', () {
    expect(CanoaConfig.defaultSkillsSearch, _constant(_host('control/skill_search.rs'), 'DEFAULT_BASE'));
  });

  test('every key of [host.kb] the app edits is one the host reads', () {
    final kb = RegExp(r'struct RawKb \{([^}]+)\}').firstMatch(_host('config.rs'));
    expect(kb, isNotNull);
    final read = {for (final field in RegExp(r'^\s*(\w+):', multiLine: true).allMatches(kb!.group(1)!)) field.group(1)!};
    expect(read.containsAll(CanoaConfig.hostKbKeys), isTrue, reason: '${CanoaConfig.hostKbKeys.toSet().difference(read)} is read by the app only');
  });
}
