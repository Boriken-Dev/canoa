// Every table of canoa.cfg the app reads has a page that edits it: a section the host lists, or one of the panels outside Settings. A
// table added to the reader with no entry in `configPages` fails here, as a key the host's reader accepts with no page fails in
// `config::coverage_tests`.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/settings/config_pages.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/sections_fixture.dart';

void main() {
  final sections = {for (final each in builtInSectionsJson()) (each! as Map)['id']! as String};

  test('every table the app reads has a page', () {
    final missing = [for (final table in CanoaConfig.ownTables) if (!configPages.containsKey(table)) table];
    expect(missing, isEmpty, reason: 'these tables of canoa.cfg have no page: add each to configPages');
  });

  test('every table the app edits through its own fields has a page', () {
    expect([for (final table in CanoaConfig.editedKeys.keys) if (!configPages.containsKey(table)) table], isEmpty);
  });

  test('every page named is a section the host lists or a panel outside Settings', () {
    for (final MapEntry(:key, :value) in configPages.entries) {
      expect(sections.contains(value) || pagesOutsideSettings.contains(value), isTrue, reason: '$key names $value, which is no page');
    }
  });

  test('a table with no page is found', () {
    final tables = {...CanoaConfig.ownTables, 'a_new_table'};
    expect([for (final table in tables) if (!configPages.containsKey(table)) table], ['a_new_table']);
  });
}
