// Every family but Canoa's and the terminal's comes from the host's `looks/list`: a document with an unknown and a missing token still
// builds a theme, a family the host does not list is kept as the config wrote it and draws in Canoa's look with the reason said in
// Appearance, and a family some other agent brings is offered like the rest.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/settings/app_sections.dart';
import 'package:canoa/settings/settings_controller.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/theme/vendor_looks.dart';
import 'package:material_ui/material_ui.dart';
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
    final look = lookFor(claudeLook, Brightness.dark);
    expect(look.id, claudeLook);
    expect(look.accent, const Color(0xFF102030));
    expect(look.radius, 21);
    expect(look.page, canoa.page, reason: 'a token the document lacks');
    expect(look.text, canoa.text, reason: 'a token it holds in a form this app does not read');
    final theme = themeFor(claudeLook, Brightness.dark);
    expect(theme.colorScheme.primary, const Color(0xFF102030));
    expect(theme.extension<CanoaLook>()!.page, canoa.page);
    expect(theme.scaffoldBackgroundColor, canoa.page);
  });

  test('a brightness the document lacks draws in Canoa\'s look', () {
    vendorLooks.adopt(_document(dark: {'accent': '#102030'}));
    expect(lookFor(claudeLook, Brightness.light).accent, lookFor(LookId.canoa, Brightness.light).accent);
    expect(lookFor(claudeLook, Brightness.light).id, LookId.canoa);
  });

  test('before a host answers, and for a host that lists no such family, Claude and ChatGPT draw as Canoa and say why', () {
    vendorLooks.clear();
    for (final id in [claudeLook, chatgptLook]) {
      expect(lookFor(id, Brightness.dark).accent, lookFor(LookId.canoa, Brightness.dark).accent);
      expect(vendorLooks.fallbackReason(id), contains('has not answered'));
    }
    expect(vendorLooks.fallbackReason(LookId.canoa), isNull);
    expect(vendorLooks.fallbackReason(LookId.terminal), isNull);
    vendorLooks.adopt(_document());
    expect(vendorLooks.fallbackReason(chatgptLook), contains('lists no ChatGPT'));
    expect(vendorLooks.fallbackReason(claudeLook), isNull);
    vendorLooks.clear();
    vendorLooks.failed('no link');
    expect(vendorLooks.fallbackReason(claudeLook), contains('no link'));
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
    final accents = {for (final id in allLooks) lookFor(id, Brightness.dark).accent};
    expect(accents.length, allLooks.length);
    expect(lookFor(chatgptLook, Brightness.dark).controlRadius, 20);
    expect(vendorLooks.familyOfVendor('codex'), chatgptLook);
  });

  testWidgets('Appearance says why a family it cannot draw yet is shown as Canoa', (tester) async {
    vendorLooks.clear();
    final controller = SettingsController(config: const CanoaConfig(look: claudeLook), apply: (c) async => c);
    await tester.pumpWidget(MaterialApp(theme: themeFor(LookId.canoa, Brightness.dark), home: Scaffold(body: SingleChildScrollView(child: AppearanceSection(controller: controller)))));
    expect(find.byKey(const Key('settings-look-fallback')), findsOneWidget);
    listCrateLooks();
    await tester.pump();
    expect(find.byKey(const Key('settings-look-fallback')), findsNothing);
  });

  test('a family the host lists that this app has never heard of is offered, drawn and read from the config', () {
    vendorLooks.adopt({
      'looks': [
        {'id': 'gizmo', 'label': 'Gizmo', 'vendor': 'gizmo', 'dark': {'accent': '#0A0B0C'}},
      ],
    });
    expect(vendorLooks.families.map((id) => id.label), ['Gizmo']);
    final gizmo = LookId.named('gizmo')!;
    expect(lookFor(gizmo, Brightness.dark).accent, const Color(0xFF0A0B0C));
    expect(vendorLooks.familyOfVendor('gizmo'), gizmo);
    expect(CanoaConfig.parse('[appearance]\nlook = "gizmo"\n').look, gizmo);
  });

  test('a config naming a family the host does not list keeps it as written and draws Canoa\'s look', () {
    vendorLooks.adopt(_document());
    final config = CanoaConfig.parse('[appearance]\nlook = "elsewhere"\n');
    expect(config.look.name, 'elsewhere');
    expect(CanoaConfig.parse(config.encode()).look.name, 'elsewhere');
    expect(lookFor(config.look, Brightness.dark).accent, lookFor(LookId.canoa, Brightness.dark).accent);
    expect(vendorLooks.fallbackReason(config.look), contains('lists no elsewhere'));
    expect(() => CanoaConfig.parse('[appearance]\nlook = "Not An Id"\n'), throwsFormatException);
  });

  testWidgets('the Family choice lists Canoa, the terminal and each family the host listed, and the one the config names', (tester) async {
    vendorLooks.adopt({
      'looks': [
        {'id': 'gizmo', 'label': 'Gizmo', 'vendor': 'gizmo'},
      ],
    });
    final controller = SettingsController(config: const CanoaConfig(look: LookId('elsewhere')), apply: (c) async => c);
    await tester.pumpWidget(MaterialApp(theme: themeFor(LookId.canoa, Brightness.dark), home: Scaffold(body: SingleChildScrollView(child: AppearanceSection(controller: controller)))));
    await tester.tap(find.byKey(const Key('settings-look')));
    await tester.pumpAndSettle();
    for (final label in ['Canoa', 'Terminal', 'Gizmo', 'elsewhere']) {
      expect(find.text(label), findsWidgets, reason: label);
    }
    expect(find.text('Claude'), findsNothing, reason: 'a host that lists no Claude family offers none');
  });
}
