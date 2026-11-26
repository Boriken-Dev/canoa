// Settings' Content page over a scripted host: the table of URI schemes and who brought each, the generators, the providers that were
// refused, the content cache and its Clear, and the Try box that asks the host about one address (stat, list, read, resolve) and
// shows its answer with the badge of the hash its publisher states. Goldens are drawn in the test font; with CANOA_SAMPLES_DIR set the
// same scenes are written with real fonts for `ui/samples/canoa/`.
@Timeout(Duration(minutes: 3))
library;

import 'dart:io';

import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/l10n/app_text.dart';
import 'package:canoa/l10n/base_text.dart' show baseStrings;
import 'package:canoa/store/content_settings.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/settings_fixture.dart';
import 'fixtures/store_fixtures.dart';

/// A host that answers the content methods from what a test sets.
final class ScriptedContent {
  final calls = <(String, Map<String, Object?>)>[];
  var cacheEntries = 2;

  Future<Object?> call(String method, Map<String, Object?> params) async {
    calls.add((method, params));
    switch (method) {
      case 'content/schemes':
        return {
          'schema': 1,
          'schemes': [
            {'scheme': 'file', 'grammar': 'file:///abs/path or D:/folder/file', 'title': 'This computer', 'provider': 'file', 'lists': true, 'resolves': false, 'state': 'on'},
            {'scheme': 'git+https', 'grammar': 'git+https://host/owner/repo.git[!/path][?ref=REV]', 'title': 'Git', 'provider': 'git', 'lists': true, 'resolves': true, 'state': 'on'},
            {'scheme': 'hf', 'grammar': 'hf://owner/name[/path][?ref=REV]', 'title': 'Hugging Face', 'provider': 'huggingface', 'extension': 'canoa-huggingface', 'lists': true, 'resolves': true, 'state': 'off'},
          ],
          'generators': [
            {'id': 'command', 'title': 'Run a program', 'types': [], 'source': 'absent'},
            {'id': 'whisper_hf', 'title': 'Whisper models of a repository', 'extension': 'canoa-whisper', 'types': ['whisper'], 'source': 'required'},
          ],
          'refused': [
            {'kind': 'content_source', 'id': 'rival', 'extension': 'rival_ext', 'why': 'demo is already offered by demo'},
          ],
        };
      case 'content/cache':
        if (params['clear'] == true) cacheEntries = 0;
        return {'entries': cacheEntries, 'bytes': cacheEntries * 2048, 'max_bytes': 512 * 1024 * 1024};
      case 'content/read':
        final uri = params['uri']! as String;
        if (uri.startsWith('nope:')) throw ControlFailed(-32002, 'no provider offers nope:, which nope:x names; the schemes offered are file, https');
        return {
          'text': 'name = "team"\n',
          'bytes': 14,
          'truncated': uri.endsWith('/long'),
          'from': params['refresh'] == true ? 'fetched' : 'cache',
          if (uri.contains('github')) 'badge': 'Git blob',
          'stat': {'kind': 'file'},
        };
      case 'content/stat':
        return {
          'stat': {
            'kind': 'file',
            'size': 14,
            'revision': 'abc123',
            'digests': [
              {'algorithm': 'git_blob', 'value': 'e69de29', 'provenance': 'published'},
            ],
          },
          'badge': 'Git blob',
        };
      case 'content/list':
        return params['cursor'] == null
            ? {
                'entries': [
                  {'path': 'catalog.toml', 'kind': 'file', 'size': 14},
                  {'path': 'sub', 'kind': 'dir'},
                ],
                'next': '2',
              }
            : {
                'entries': [
                  {'path': 'zeta.toml', 'kind': 'file', 'size': 3},
                ],
              };
      case 'content/resolve':
        return {'uri': '${params['uri']}?ref=0123abc', 'revision': '0123abc', 'immutable': true};
    }
    throw ControlFailed(-32601, 'no method $method');
  }
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/content_settings_test.dart')));

  Future<ScriptedContent> open(WidgetTester tester, {Size size = const Size(1100, 2200)}) async {
    final host = ScriptedContent();
    await openSettings(tester, section: 'content', size: size, contentSettings: ContentSettings(host.call));
    await tester.pumpAndSettle();
    return host;
  }

  testWidgets('lists the schemes with who brought each, the generators, what was refused and the cache', (tester) async {
    await open(tester);
    expect(find.byKey(const Key('content-scheme-file')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('content-scheme-file')), matching: find.textContaining('built in')), findsOneWidget);
    final hf = find.byKey(const Key('content-scheme-hf'));
    expect(find.descendant(of: hf, matching: find.textContaining('brought by canoa-huggingface')), findsOneWidget);
    expect(find.descendant(of: hf, matching: find.textContaining('turned off in canoa.cfg')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('content-scheme-git+https')), matching: find.textContaining('resolves a branch or tag')), findsOneWidget);
    expect(find.byKey(const Key('content-generator-command')), findsOneWidget);
    expect(find.byKey(const Key('content-generator-whisper_hf')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('content-refused-rival')), matching: find.textContaining('already offered by demo')), findsOneWidget);
    expect(find.text('2 documents, 4.0 KiB of 512.0 MiB'), findsOneWidget);
  });

  testWidgets('Clear forgets the cache and says so', (tester) async {
    final host = await open(tester);
    await tester.tap(find.byKey(const Key('content-cache-clear')));
    await tester.pumpAndSettle();
    expect(host.calls.where((call) => call.$1 == 'content/cache').last.$2, {'clear': true});
    expect(find.text('0 documents, 0 B of 512.0 MiB'), findsOneWidget);
    expect(tester.widget<TextButton>(find.byKey(const Key('content-cache-clear'))).onPressed, isNull, reason: 'nothing is left to clear');
  });

  testWidgets('Read shows the text, where it came from and the hash its publisher states, or that none is', (tester) async {
    final host = await open(tester);
    await tester.enterText(find.byKey(const Key('content-try-uri')), 'git+https://github.com/o/r.git!/catalog.toml');
    await tester.tap(find.byKey(const Key('content-try-read')));
    await tester.pumpAndSettle();
    expect(host.calls.where((call) => call.$1 == 'content/read').last.$2, {'uri': 'git+https://github.com/o/r.git!/catalog.toml'});
    expect(find.text('name = "team"\n'), findsOneWidget);
    expect(find.text('Git blob verified'), findsOneWidget);
    expect(find.text('Kept copy, 14 B'), findsOneWidget);

    await tester.tap(find.byKey(const Key('content-try-refresh')));
    await tester.pumpAndSettle();
    expect(host.calls.where((call) => call.$1 == 'content/read').last.$2['refresh'], true);
    expect(find.text('Just read, 14 B'), findsOneWidget);

    await tester.enterText(find.byKey(const Key('content-try-uri')), 'https://example.org/long');
    await tester.tap(find.byKey(const Key('content-try-read')));
    await tester.pumpAndSettle();
    expect(find.text('Not verified'), findsOneWidget, reason: 'the publisher states no hash, and no rule ranks one');
    expect(find.text('Only the start is shown.'), findsOneWidget);
  });

  testWidgets('List pages, Stat gives the digests and Resolve the commit', (tester) async {
    final host = await open(tester);
    await tester.enterText(find.byKey(const Key('content-try-uri')), 'git+https://github.com/o/r.git');
    await tester.tap(find.byKey(const Key('content-try-list')));
    await tester.pumpAndSettle();
    expect(find.text('catalog.toml  14 B'), findsOneWidget);
    expect(find.text('sub/'), findsOneWidget);
    await tester.tap(find.byKey(const Key('content-try-more')));
    await tester.pumpAndSettle();
    expect(host.calls.where((call) => call.$1 == 'content/list').last.$2['cursor'], '2');
    expect(find.text('catalog.toml  14 B'), findsOneWidget, reason: 'the first page stays');
    expect(find.text('zeta.toml  3 B'), findsOneWidget);
    expect(find.byKey(const Key('content-try-more')), findsNothing, reason: 'the last page has no next');

    await tester.tap(find.byKey(const Key('content-try-stat')));
    await tester.pumpAndSettle();
    expect(find.text('git_blob: e69de29 (published)'), findsOneWidget);
    expect(find.text('Git blob verified'), findsOneWidget);

    await tester.tap(find.byKey(const Key('content-try-resolve')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('content-try-revision')), findsOneWidget);
    expect(find.text('0123abc'), findsOneWidget);
  });

  testWidgets('what the host refuses is said under the box', (tester) async {
    await open(tester);
    await tester.enterText(find.byKey(const Key('content-try-uri')), 'nope:x');
    await tester.tap(find.byKey(const Key('content-try-read')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('content-try-failure')), findsOneWidget);
    expect(find.textContaining('no provider offers nope:'), findsOneWidget);
  });

  test('every word of the page is in English and in Spanish, with the same placeholders', () {
    final english = baseStrings['en']!, spanish = baseStrings['es']!;
    final keys = english.keys.where((key) => key.startsWith('content.') || key == 'settings.section.content');
    expect(keys, isNotEmpty);
    final placeholder = RegExp(r'\{(\w+)\}');
    for (final key in keys) {
      expect(spanish.containsKey(key), isTrue, reason: '$key has no Spanish');
      expect({for (final m in placeholder.allMatches(spanish[key]!)) m.group(1)}, {for (final m in placeholder.allMatches(english[key]!)) m.group(1)}, reason: key);
    }
    expect(const AppText(language: 'es')('content.cache.state', {'entries': 3, 'size': '1.0 KiB', 'max': '2.0 KiB'}), '3 documentos, 1.0 KiB de 2.0 KiB');
    expect(const AppText(language: 'es')('content.try.verified', {'hash': 'SHA-256'}), 'SHA-256 verificado');
  });

  for (final (label, size) in [('desktop', const Size(1280, 1500)), ('phone', const Size(390, 1900))]) {
    testWidgets('golden at $label width', (tester) async {
      final host = ScriptedContent();
      await openSettings(tester, section: 'content', size: size, contentSettings: ContentSettings(host.call));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('content-try-uri')), 'git+https://github.com/o/r.git!/catalog.toml');
      await tester.tap(find.byKey(const Key('content-try-read')));
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/content_settings_$label.png'));
    });
  }
}
