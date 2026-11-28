// The Store's pages are selection areas: the names, descriptions and addresses on a page and on an entry can be selected and copied,
// while the buttons, switches and fields in them keep working.
@Timeout(Duration(minutes: 3))
library;

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/overlays.dart';
import 'package:canoa/store/store_section.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/catalog_standin.dart';
import 'fixtures/store_fixtures.dart';
import 'loadout_test.dart' show withCatalog, world;

const _commit = 'a1b2c3d4e5f60718293a4b5c6d7e8f9012345678';
const _anthropics = 'https://github.com/anthropics/skills.git';

void main() {
  late MemoryOverlays overlays;
  late CanoaConfig config;

  setUp(() {
    overlays = world()
      ..scans[_anthropics] = const SkillScan(commit: _commit, skills: [
        ScannedSkill(name: 'pdf', description: 'Reads and fills PDFs.', path: 'skills/pdf'),
      ]);
  });

  Future<void> open(WidgetTester tester) async {
    await tester.binding.setSurfaceSize(const Size(1280, 2000));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    config = CanoaConfig.parse(withCatalog);
    await tester.pumpWidget(MaterialApp(
      home: StoreSection(
        readAhead: false,
        config: config,
        home: MemoryHome(overlays),
        index: standInIndex(overlays, () => config),
        agents: const ['Claude'],
        onSave: (changed) async => changed,
        readCatalog: standInCatalogRead(overlays, () => config, registry: (uri) async => '{}'),
        findProgram: (program) async => '/usr/bin/$program',
      ),
    ));
    await tester.pumpAndSettle();
  }

  /// Selects everything in the Store's area and copies it, as Select all and Copy do.
  Future<String> copyAll(WidgetTester tester) async {
    var copied = '';
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
      if (call.method == 'Clipboard.setData') copied = (call.arguments as Map)['text'] as String;
      return null;
    });
    addTearDown(() => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, null));
    final region = tester.state<SelectableRegionState>(find.descendant(of: find.byType(StoreSection), matching: find.byType(SelectableRegion)).first);
    region.selectAll();
    await tester.pump();
    // ignore: deprecated_member_use
    region.copySelection(SelectionChangedCause.toolbar);
    await tester.pump();
    return copied;
  }

  testWidgets('a Store page is one selection area: what a skill\'s tile says can be selected and copied, and its buttons still work', (tester) async {
    await open(tester);
    expect(find.descendant(of: find.byType(StoreSection), matching: find.byType(SelectionArea)), findsOneWidget);
    await tester.tap(find.byKey(const Key('store-tab-skills')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('skills-discover')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('store-source-skill-anthropics-pdf')), findsOneWidget);

    final copied = await copyAll(tester);
    expect(copied, contains('pdf'));
    expect(copied, contains('Reads and fills PDFs.'));
    expect(copied, contains('anthropics/skills'), reason: 'the address the skill is read from');

    // A button in the area is still a button.
    await tester.tap(find.byKey(const Key('store-source-skill-install-anthropics-pdf')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('store-skill-review')), findsOneWidget);
  });
}
