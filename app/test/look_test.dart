// The tokens every surface draws with: readable in each look and brightness, what the component themes take from them, and that the
// management pages name no colour of their own.
import 'dart:io';

import 'package:canoa/theme/look.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/vendor_looks_fixture.dart';

double _contrast(Color a, Color b) {
  final (hi, lo) = (a.computeLuminance(), b.computeLuminance());
  return ((hi > lo ? hi : lo) + 0.05) / ((hi > lo ? lo : hi) + 0.05);
}

void main() {
  final combos = [for (final id in allLooks) for (final brightness in Brightness.values) (id, brightness)];

  group('tokens', () {
    for (final (id, brightness) in combos) {
      final look = lookFor(id, brightness);
      final name = '${id.name} ${brightness.name}';

      test('$name: text, muted text, accent and states are readable on the page and the surfaces over it', () {
        for (final over in [look.page, look.surface, look.raised, look.rail]) {
          expect(_contrast(look.text, over), greaterThanOrEqualTo(7), reason: 'text on $over');
          expect(_contrast(look.muted, over), greaterThanOrEqualTo(4.5), reason: 'muted on $over');
          expect(_contrast(look.accent, over), greaterThanOrEqualTo(3), reason: 'accent on $over');
          for (final state in [look.success, look.warning, look.danger]) {
            expect(_contrast(state, over), greaterThanOrEqualTo(4.5), reason: '$state on $over');
          }
        }
        expect(_contrast(look.onAccent, look.accent), greaterThanOrEqualTo(4.5));
        expect(_contrast(look.text, look.selected), greaterThanOrEqualTo(7), reason: 'the row selected');
      });

      test('$name: the theme carries the tokens in its scheme and its component themes', () {
        final theme = themeFor(id, brightness);
        expect(theme.extension<CanoaLook>(), look);
        final scheme = theme.colorScheme;
        expect((scheme.primary, scheme.onPrimary, scheme.error), (look.accent, look.onAccent, look.danger));
        expect((scheme.surface, scheme.onSurface, scheme.onSurfaceVariant), (look.page, look.text, look.muted));

        final fields = theme.inputDecorationTheme;
        expect(fields.filled, isTrue);
        expect((fields.focusedBorder as OutlineInputBorder).borderSide.color, look.focus);
        expect((fields.enabledBorder as OutlineInputBorder).borderSide.color, look.border);
        expect((fields.border as OutlineInputBorder).borderRadius, BorderRadius.circular(look.controlRadius));

        expect(theme.filledButtonTheme.style!.backgroundColor!.resolve({}), look.accent);
        expect(theme.tabBarTheme.indicatorColor, look.accent);
        expect(theme.dialogTheme.backgroundColor, look.surface);
        expect(theme.cardTheme.color, look.surface);
        expect(theme.popupMenuTheme.color, look.raised);
        final selected = theme.segmentedButtonTheme.style!.backgroundColor!;
        expect(selected.resolve({WidgetState.selected}), look.selected);
        expect(selected.resolve({}), Colors.transparent);
      });
    }

    test('the terminal look squares every corner the canoa look rounds', () {
      expect(lookFor(LookId.terminal, Brightness.dark).radius, lessThan(lookFor(LookId.canoa, Brightness.dark).radius));
      expect(lookFor(LookId.terminal, Brightness.dark).controlRadius, lessThan(lookFor(LookId.canoa, Brightness.dark).controlRadius));
    });

    test('a theme between two looks blends their tokens, and an unnamed one draws as Canoa dark', () {
      final dark = lookFor(LookId.canoa, Brightness.dark);
      final light = lookFor(LookId.canoa, Brightness.light);
      expect(dark.lerp(light, 0).page, dark.page);
      expect(dark.lerp(light, 1).accent, light.accent);
      expect(dark.lerp(light, 0.5).surface, Color.lerp(dark.surface, light.surface, 0.5));
    });
  });

  group('no page names a colour of its own', () {
    // A colour comes from the theme; only a transparent one is spelled out, for a row that is not selected.
    final named = RegExp(r'\bColor\(|\bColors\.(?!transparent\b)\w+|\bCupertinoColors\.');
    for (final directory in ['settings', 'store', 'loadout', 'accounts', 'shell']) {
      test('lib/$directory', () {
        final found = <String>[];
        for (final file in Directory('lib/$directory').listSync(recursive: true).whereType<File>()) {
          if (!file.path.endsWith('.dart')) continue;
          final lines = file.readAsLinesSync();
          for (var i = 0; i < lines.length; i++) {
            if (named.hasMatch(lines[i])) found.add('${file.path}:${i + 1}');
          }
        }
        expect(found, isEmpty);
      });
    }
  });
}
