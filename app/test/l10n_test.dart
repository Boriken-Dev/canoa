import 'package:canoa/brand.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/l10n/app_text.dart';
import 'package:canoa/l10n/base_text.dart';
import 'package:canoa/l10n/text_pack.dart';
import 'package:canoa/shell/section_rail.dart';
import 'package:canoa/shell/sections.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/theme/vendor_looks.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

/// What a host lists for a look that brings its own words: two strings of the base replaced, one language of its own, one symbol
/// drawn as a glyph and another as a base symbol.
Map<String, Object?> _document() => {
  'id': 'stage',
  'label': 'Stage',
  'vendor': 'stage',
  'dark': {'accent': '#33AA55'},
  'light': {'accent': '#116633'},
  'text': {
    'default_language': 'en',
    'strings': {
      'en': {'nav.conversations': 'Scenes', 'nav.settings': 'Backstage', 'language.name': 'English'},
      'fr': {'nav.conversations': 'Scènes', 'language.name': 'Français'},
    },
    'symbols': {
      'conversations': {'glyph': '>'},
      'store': {'like': 'terminal'},
    },
  },
};

void main() {
  tearDown(vendorLooks.clear);

  group('the lookup', () {
    final pack = TextPack.parse(_document()['text'])!;

    test('without a theme every key is the base English', () {
      const text = AppText();
      expect(text('nav.conversations'), 'Conversations');
      expect(text('nav.settings_tooltip'), '${Brand.name} settings');
      expect(text('tray.open_waiting', {'count': 3}), 'Open ${Brand.name} (3 waiting for you)');
    });

    test('a theme replaces some keys and the rest fall back to the base', () {
      final text = AppText(pack: pack);
      expect(text('nav.conversations'), 'Scenes');
      expect(text('nav.settings'), 'Backstage');
      expect(text('nav.store'), 'Store');
    });

    test('a language the base has replaces a word, and the theme still wins where it has one', () {
      final text = AppText(language: 'es', pack: pack);
      // The theme has no Spanish: its own default language (English) comes before the base's Spanish.
      expect(text('nav.conversations'), 'Scenes');
      // A key only the base has in Spanish reads in Spanish.
      expect(text('nav.store'), 'Tienda');
      // A key only English has anywhere falls back to English.
      expect(text('composer.cache_tooltip'), 'How long the prompt cache lives');
    });

    test('a language only the theme adds is offered and read', () {
      final text = AppText(language: 'fr', pack: pack);
      expect(text('nav.conversations'), 'Scènes');
      expect(text('nav.store'), 'Store');
      expect(text.languages, containsAll(['en', 'es', 'fr']));
      expect(text.languageName('fr'), 'Français');
      expect(AppText.resolveLanguage('fr', pack), 'fr');
      expect(AppText.resolveLanguage('fr', null), isNot('fr'));
    });

    test('a key nobody has reads as itself, and maybe says so', () {
      const text = AppText();
      expect(text('no.such.key'), 'no.such.key');
      expect(text.maybe('no.such.key'), isNull);
      expect(text.maybe('settings.section.general'), 'General');
    });

    test('a symbol is the theme\'s glyph, another base symbol, or the base one', () {
      final text = AppText(pack: pack);
      expect(text.symbol('conversations'), isA<GlyphSymbol>().having((symbol) => symbol.glyph, 'glyph', '>'));
      expect(text.symbol('store'), isA<IconSymbol>().having((symbol) => symbol.icon, 'icon', baseSymbols['terminal']));
      expect(text.symbol('settings'), isA<IconSymbol>().having((symbol) => symbol.icon, 'icon', baseSymbols['settings']));
    });

    test('every key the base has in another language is a key of the English', () {
      final english = baseStrings[baseLanguage]!.keys.toSet();
      for (final MapEntry(key: language, value: table) in baseStrings.entries) {
        expect(english.containsAll(table.keys), isTrue, reason: '$language has keys English lacks: ${table.keys.toSet().difference(english)}');
      }
    });
  });

  group('a look from the host', () {
    test('carries its text, which the look hands the widgets', () {
      vendorLooks.adopt({'looks': [_document()]});
      final look = lookFor(LookId.named('stage')!, Brightness.dark);
      expect(look.words, isNotNull);
      expect(look.words!.symbols.keys, containsAll(['conversations', 'store']));
      expect(lookFor(LookId.canoa, Brightness.dark).words, isNull);
    });

    testWidgets('the selector draws the look\'s symbol and its names, in the language the config chose', (tester) async {
      vendorLooks.adopt({'looks': [_document()]});
      final id = LookId.named('stage')!;
      Widget app(String? language, LookId look) => MaterialApp(
        theme: themeFor(look, Brightness.dark),
        builder: (context, child) => AppTextHost(language: language, child: child!),
        home: Scaffold(
          body: Builder(builder: (context) => SectionRail(
            selected: 'a',
            onSelect: (_) {},
            sections: [
              AppSection(id: 'a', label: context.t('nav.conversations'), symbol: 'conversations', main: const SizedBox()),
              AppSection(id: 'b', label: context.t('nav.store'), symbol: 'store', main: const SizedBox()),
            ],
          )),
        ),
      );

      await tester.pumpWidget(app(null, id));
      await tester.pumpAndSettle();
      expect(find.text('>'), findsOneWidget);
      expect(find.bySemanticsLabel('Scenes'), findsOneWidget);
      expect(find.bySemanticsLabel('Store'), findsOneWidget);
      expect(find.byIcon(baseSymbols['terminal']!), findsOneWidget);

      await tester.pumpWidget(app('fr', id));
      await tester.pumpAndSettle();
      expect(find.bySemanticsLabel('Scènes'), findsOneWidget);

      // The same sections under a look that sets no words draw the base: an icon, and the English names.
      await tester.pumpWidget(app('es', LookId.canoa));
      await tester.pumpAndSettle();
      expect(find.text('>'), findsNothing);
      expect(find.byIcon(baseSymbols['conversations']!), findsOneWidget);
      expect(find.bySemanticsLabel('Conversaciones'), findsOneWidget);
      expect(find.bySemanticsLabel('Tienda'), findsOneWidget);
    });
  });

  group('the language setting', () {
    test('is written only when chosen, and read back', () {
      const chosen = CanoaConfig(language: 'es');
      expect(chosen.encode(), contains('language = "es"'));
      expect(const CanoaConfig().encode(), isNot(contains('language')));
      expect(CanoaConfig.parse(chosen.encode()).language, 'es');
      expect(CanoaConfig.parse(const CanoaConfig().encode()).language, '');
    });
  });
}
