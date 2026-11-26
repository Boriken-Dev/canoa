// What a person types as a catalog's source, written as the full URI canoa.cfg holds.
import 'package:canoa/config/catalog_specs.dart';
import 'package:canoa/store/source_uri.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('a path becomes a file URI, with its backslashes and spaces written', () {
    expect(sourceUri(r'D:\models\whisper list.json', uris: true), 'file:///D:/models/whisper%20list.json');
    expect(sourceUri('C:/x/y.json', uris: true), 'file:///C:/x/y.json');
    expect(sourceUri('/home/me/models.json', uris: true), 'file:///home/me/models.json');
    expect(sourceUri(r'\\host\share\m.json', uris: true), 'file://host/share/m.json');
  });

  test('owner/repo and a GitHub address become the git address of the repository, keeping the folder and the ref', () {
    expect(sourceUri('anthropics/skills', uris: true), 'git+https://github.com/anthropics/skills.git');
    expect(sourceUri('https://github.com/owner/repo', uris: true), 'git+https://github.com/owner/repo.git');
    expect(sourceUri('https://github.com/owner/repo.git', uris: true), 'git+https://github.com/owner/repo.git');
    expect(sourceUri('https://github.com/owner/repo/tree/dev/models/base', uris: true), 'git+https://github.com/owner/repo.git!/models/base?ref=dev');
    expect(sourceUri('https://github.com/owner/repo/blob/main/catalog.json', uris: true), 'git+https://github.com/owner/repo.git!/catalog.json?ref=main');
  });

  test('anything with a scheme is left as typed, and so is everything for a type that keeps its own grammar', () {
    for (final typed in ['https://example.org/c.json', 'hf://owner/name?ref=main', 'git+https://h/o/r.git!/c.toml', 'data:,x', 'file:///C:/x']) {
      expect(sourceUri(typed, uris: true), typed);
    }
    for (final typed in ['owner/repo', r'D:\x\y', 'https://github.com/o/r/tree/main/s']) {
      expect(sourceUri(typed, uris: false), typed);
    }
    expect(sourceUri('  ', uris: true), '');
    expect(sourceUri('relative/name.json/more', uris: true), 'relative/name.json/more');
  });

  test('an entry that names a generator is read back as one, and a generator and a provider are never both', () {
    final spec = CatalogSpec.parse('store.whisper.catalogs.mine', {'generator': 'whisper_hf', 'source': 'hf://a/b?ref=main', 'include': '*.bin'});
    expect(spec.generator, 'whisper_hf');
    expect(spec.isDocument, isFalse);
    expect(spec.source, 'hf://a/b?ref=main');
    expect(spec.toRaw(), {'generator': 'whisper_hf', 'source': 'hf://a/b?ref=main', 'include': '*.bin'});
    expect(spec, CatalogSpec.parse('x', spec.toRaw()));
    expect(spec, isNot(const CatalogSpec('whisper_hf', {'source': 'hf://a/b?ref=main', 'include': '*.bin'})));
    expect(CatalogSpec.parse('x', 'D:/m.json').isDocument, isTrue);
    expect(() => CatalogSpec.parse('x', {'generator': 'g', 'provider': 'p'}), throwsFormatException);
    expect(() => CatalogSpec.parse('x', {'generator': 3}), throwsFormatException);
  });
}
