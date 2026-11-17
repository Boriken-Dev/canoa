// The Claude and ChatGPT families come from the host's `looks/list`: a document with an unknown and a missing token still builds a
// theme, and before a host answers those families draw in Canoa's look with the reason said in Appearance.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/settings/app_sections.dart';
import 'package:canoa/settings/settings_controller.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/theme/vendor_looks.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/vendor_looks_fixture.dart';

Map<String, Object?> _document({Map<String, Object?> dark = const {}, Map<String, Object?>? light}) => {
  'looks': [
    {'id': 'claude', 'label': 'Claude', 'vendor': 'claude', 'dark': dark, 'light': ?light},
  ],
};

void main() {
  tearDown(listCrateLooks);

  test('a document with an unknown and a missing token builds a theme that takes Canoa\'s value for the missing one', () {
    vendorLooks.adopt(_document(dark: {'accent': '#102030', 'nonsense': '#FFFFFF', 'text': 'not a colour', 'radius': 21}));
    final canoa = lookFor(LookId.canoa, Brightness.dark);
    final look = lookFor(LookId.claude, Brightness.dark);
    expect(look.id, LookId.claude);
    expect(look.accent, const Color(0xFF102030));
    expect(look.radius, 21);
    expect(look.page, canoa.page, reason: 'a token the document lacks');
    expect(look.text, canoa.text, reason: 'a token it holds in a form this app does not read');
    final theme = themeFor(LookId.claude, Brightness.dark);
    expect(theme.colorScheme.primary, const Color(0xFF102030));
    expect(theme.extension<CanoaLook>()!.page, canoa.page);
    expect(theme.scaffoldBackgroundColor, canoa.page);
  });

  test('a brightness the document lacks draws in Canoa\'s look', () {
    vendorLooks.adopt(_document(dark: {'accent': '#102030'}));
    expect(lookFor(LookId.claude, Brightness.light).accent, lookFor(LookId.canoa, Brightness.light).accent);
    expect(lookFor(LookId.claude, Brightness.light).id, LookId.canoa);
  });

  test('before a host answers, and for a host that lists no such family, Claude and ChatGPT draw as Canoa and say why', () {
    vendorLooks.clear();
    for (final id in [LookId.claude, LookId.chatgpt]) {
      expect(lookFor(id, Brightness.dark).accent, lookFor(LookId.canoa, Brightness.dark).accent);
      expect(vendorLooks.fallbackReason(id), contains('has not answered'));
    }
    expect(vendorLooks.fallbackReason(LookId.canoa), isNull);
    expect(vendorLooks.fallbackReason(LookId.terminal), isNull);
    vendorLooks.adopt(_document());
    expect(vendorLooks.fallbackReason(LookId.chatgpt), contains('lists no ChatGPT'));
    expect(vendorLooks.fallbackReason(LookId.claude), isNull);
    vendorLooks.clear();
    vendorLooks.failed('no link');
    expect(vendorLooks.fallbackReason(LookId.claude), contains('no link'));
  });

  test('a host cannot replace the built-in looks', () {
    vendorLooks.adopt({
      'looks': [
        {'id': 'canoa', 'vendor': 'canoa', 'dark': {'accent': '#102030'}},
        {'id': 'terminal', 'vendor': 'terminal', 'dark': {'accent': '#102030'}},
      ],
    });
    expect(lookFor(LookId.canoa, Brightness.dark).accent, isNot(const Color(0xFF102030)));
    expect(lookFor(LookId.terminal, Brightness.dark).accent, isNot(const Color(0xFF102030)));
  });

  test('the crates\' documents draw the families with their own accents and shapes', () {
    final accents = {for (final id in LookId.values) lookFor(id, Brightness.dark).accent};
    expect(accents.length, LookId.values.length);
    expect(lookFor(LookId.chatgpt, Brightness.dark).controlRadius, 20);
    expect(vendorLooks.familyOfVendor('codex'), LookId.chatgpt);
  });

  testWidgets('Appearance says why a family it cannot draw yet is shown as Canoa', (tester) async {
    vendorLooks.clear();
    final controller = SettingsController(config: const CanoaConfig(look: LookId.claude), apply: (c) async => c);
    await tester.pumpWidget(MaterialApp(theme: themeFor(LookId.canoa, Brightness.dark), home: Scaffold(body: SingleChildScrollView(child: AppearanceSection(controller: controller)))));
    expect(find.byKey(const Key('settings-look-fallback')), findsOneWidget);
    listCrateLooks();
    await tester.pump();
    expect(find.byKey(const Key('settings-look-fallback')), findsNothing);
  });
}
