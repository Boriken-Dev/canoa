// canoa.cfg's `[store.extensions.catalogs]`: the extensions catalogs the Store lists, none until the table names one. The host
// reads the table; the app reads it so a save of another table keeps it and a bad one is refused when canoa.cfg is read.
import 'package:canoa/config/canoa_config.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('none is listed until the table names one, and the table is kept when another part of [store] is written', () {
    expect(CanoaConfig.parse('').extensionCatalogs, isEmpty);
    final config = CanoaConfig.parse('[store.extensions.catalogs]\nteam = "https://example.org/team.git@main#extensions.toml"\n');
    expect(config.extensionCatalogs, {'team': 'https://example.org/team.git@main#extensions.toml'});
    final changed = config.withStore(config.store.withLimit('catalog_max_entries', 5000));
    expect(changed.extensionCatalogs, config.extensionCatalogs, reason: 'a limit saved beside it leaves it as it was');
    expect(changed.store.catalogMaxEntries, 5000);
  });

  test('a label that is no name, a source that is no text and a table that is no table are refused', () {
    for (final bad in [
      '[store.extensions.catalogs]\n"two words" = "https://example.org/x.git@main#e.toml"\n',
      '[store.extensions.catalogs]\nteam = 3\n',
      '[store.extensions.catalogs]\nteam = ""\n',
      '[store]\nextension_catalogs = ["https://example.org/x.git@main#e.toml"]\n',
    ]) {
      expect(() => CanoaConfig.parse(bad), throwsFormatException, reason: bad);
    }
  });

  test('the overlay catalogs are a table of their own: an extensions catalog never becomes one', () {
    final config = CanoaConfig.parse('[store.extensions.catalogs]\nteam = "https://example.org/team.git@main#extensions.toml"\n');
    expect(config.catalogs.keys, ['boriken'], reason: 'the default overlay catalog alone');
  });
}
