// A catalog entry's homepage and repository are links a person may open, and only when they are plain web addresses.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/loadout/loadout_page.dart';
import 'package:canoa/store/links.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart';

void main() {
  test('only a plain web address is a link', () {
    expect(webLink('https://example.org/docs?a=b#c')?.host, 'example.org');
    expect(webLink('http://example.org')?.scheme, 'http');
    for (final text in [
      '', ' https://example.org', 'https://example.org/a b', 'javascript:alert(1)', 'file:///etc/passwd', 'ftp://example.org/x',
      'https://user@example.org/', 'https:///nohost', 'example.org', 'https://exаmple.org/', 'https://example.org/\u202E',
      'https://example.org/\n',
    ]) {
      expect(webLink(text), isNull, reason: text);
    }
  });

  testWidgets('the page shows homepage and repository as links that open only when tapped', (tester) async {
    const repo = 'https://example.invalid/team.git';
    final overlays = MemoryOverlays()
      ..put(repo, {
        'catalog.toml': '''
[overlays.x]
source = "https://example.invalid/x.git@main"
about = { summary = "Docs.", homepage = "https://example.org/docs", repository = "https://example.org/src.git" }
''',
      });
    final opened = <Uri>[];
    final before = openLink;
    openLink = (uri) async {
      opened.add(uri);
      return true;
    };
    addTearDown(() => openLink = before);
    await tester.binding.setSurfaceSize(const Size(1280, 1600));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(
      home: LoadoutPage(
        config: CanoaConfig.parse('[catalogs]\nteam = "$repo@main#catalog.toml"\n'),
        home: MemoryHome(overlays),
        onSave: (changed) async => changed,
      ),
    ));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('overlays-discover')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('store-entry-team-x')));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('store-link-homepage')), findsOneWidget);
    expect(find.byKey(const Key('store-link-repository')), findsOneWidget);
    expect(opened, isEmpty, reason: 'Canoa opens nothing until asked');
    final link = find.byKey(const Key('store-link-homepage'));
    final text = tester.widget<Text>(find.descendant(of: link, matching: find.byType(Text)).first);
    final span = (text.textSpan! as TextSpan).children!.last as TextSpan;
    expect(span.text, 'https://example.org/docs');
    (span.recognizer! as dynamic).onTap();
    expect(opened, [Uri.parse('https://example.org/docs')]);
  });
}
