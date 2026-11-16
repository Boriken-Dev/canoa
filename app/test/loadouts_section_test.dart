// Loadouts: the list with what each holds and whether it can start, and the editor: create, duplicate, edit, use and delete a
// loadout, the way to the Store that keeps the draft, a missing overlay called out, and no control that reaches an item inside an
// overlay. Goldens are drawn in the test font; with CANOA_SAMPLES_DIR set the same scenes are written with real fonts.
@Timeout(Duration(minutes: 3))
library;

import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/loadout/loadout_editor.dart';
import 'package:canoa/loadout/loadouts_section.dart';
import 'package:canoa/profile/skill_copies.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_fixtures.dart';

const text = '''
[overlays]
review = "https://example.invalid/review.git@main"
docs = "https://example.invalid/docs.git@main"

[profile.mcp.files]
type = "stdio"
command = "files-mcp"

[profile.mcp.tracker]
type = "stdio"
command = "tracker-mcp"

[loadouts.review]
description = "Reading and reviewing code"
overlays = ["review", "gone"]
skills = ["audit"]
mcp = ["files"]

[loadouts.writing]
overlays = ["docs"]

[defaults]
loadout = "writing"
''';

SkillCopy _skill(String name) =>
    SkillCopy(name: name, source: 'https://example.invalid/$name.git', digest: 'sha256:${'a' * 64}', overlays: const [], chosen: true);

/// The section over a configuration it keeps in step with what it saves.
class _World {
  _World(String source) : config = CanoaConfig.parse(source) {
    overlays.skills['audit'] = _skill('audit');
    overlays.skills['lint'] = _skill('lint');
  }

  CanoaConfig config;
  final overlays = MemoryOverlays();
  final saved = <CanoaConfig>[];
  final draft = ValueNotifier<LoadoutDraft?>(null);
  final used = <String>[];
  var storeOpened = 0;
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/loadouts_section_test.dart')));

  for (final (label, size) in [('desktop', const Size(1280, 800)), ('phone', const Size(390, 844))]) {
    group('at $label width', () {
      late _World world;

      Future<void> pump(WidgetTester tester, {bool shown = true}) async {
        await tester.pumpWidget(MaterialApp(
          debugShowCheckedModeBanner: false,
          theme: themeFor(LookId.canoa, Brightness.dark),
          home: Scaffold(
            body: StatefulBuilder(
              builder: (context, setState) => shown
                  ? LoadoutsSection(
                      config: world.config,
                      home: MemoryHome(world.overlays),
                      agents: const ['Claude', 'Codex'],
                      draft: world.draft,
                      onStore: () => world.storeOpened++,
                      onUse: world.used.add,
                      onSave: (changed) async {
                        world.saved.add(changed);
                        setState(() => world.config = changed);
                        return changed;
                      },
                    )
                  : const Text('The Store'),
            ),
          ),
        ));
        await tester.pumpAndSettle();
      }

      Future<void> open(WidgetTester tester, {String source = text}) async {
        world = _World(source);
        await tester.binding.setSurfaceSize(size);
        addTearDown(() => tester.binding.setSurfaceSize(null));
        await pump(tester);
      }

      Future<void> tap(WidgetTester tester, String key) async {
        await tester.ensureVisible(find.byKey(Key(key)));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(Key(key)));
        await tester.pumpAndSettle();
      }

      Future<void> golden(WidgetTester tester, String name) async {
        expect(tester.takeException(), isNull);
        await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/loadouts_${name}_$label.png'));
      }

      testWidgets('the list counts what each loadout holds, marks the default and flags what is missing', (tester) async {
        await open(tester);
        expect(find.text('2 overlays · 1 skill · 1 MCP server'), findsOneWidget);
        expect(find.text('1 overlay · 0 skills · 0 MCP servers'), findsOneWidget);
        expect(find.byKey(const Key('loadout-default-writing')), findsOneWidget);
        expect(find.byKey(const Key('loadout-default-review')), findsNothing);
        expect(tester.widget<Text>(find.byKey(const Key('loadout-ready-writing'))).data, 'Ready');
        expect(tester.widget<Text>(find.byKey(const Key('loadout-ready-review'))).data, 'Missing: gone (overlay)');
        await golden(tester, 'list');
      });

      testWidgets('no loadout yet says what new sessions start with', (tester) async {
        await open(tester, source: '[overlays]\na = "https://example.invalid/a.git@main"\n');
        expect(find.byKey(const Key('loadouts-empty')), findsOneWidget);
        await golden(tester, 'empty');
      });

      testWidgets('creating a loadout: a name, an installed overlay and skill from the picker, saved to canoa.cfg', (tester) async {
        await open(tester);
        await tap(tester, 'loadout-new');
        expect(tester.widget<FilledButton>(find.byKey(const Key('loadout-save'))).onPressed, isNull, reason: 'no name yet');
        await tester.enterText(find.byKey(const Key('loadout-name')), 'deep');
        await tester.pumpAndSettle();
        await tap(tester, 'loadout-add-overlays');
        expect(find.byKey(const Key('loadout-pick-review')), findsOneWidget);
        await tester.enterText(find.byKey(const Key('loadout-picker-search')), 'doc');
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('loadout-pick-review')), findsNothing);
        await tester.tap(find.byKey(const Key('loadout-pick-docs')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('loadout-picker-add')));
        await tester.pumpAndSettle();
        await tap(tester, 'loadout-add-skills');
        await tester.tap(find.byKey(const Key('loadout-pick-lint')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('loadout-picker-add')));
        await tester.pumpAndSettle();
        await golden(tester, 'editor');
        await tap(tester, 'loadout-save');
        final made = world.saved.single.loadouts['deep']!;
        expect(made.overlays, ['docs']);
        expect(made.skills, ['lint']);
        expect(world.saved.single.defaultLoadout, 'writing', reason: 'creating one leaves the default alone');
        expect(find.byKey(const Key('loadout-card-deep')), findsOneWidget, reason: 'back to the list, which shows it');
      });

      testWidgets('duplicating starts a new loadout from the old one; it cannot take a name that is held', (tester) async {
        await open(tester);
        await tap(tester, 'loadout-duplicate-writing');
        expect(tester.widget<TextFormField>(find.byKey(const Key('loadout-name'))).initialValue, 'writing copy');
        await tester.enterText(find.byKey(const Key('loadout-name')), 'Review');
        await tester.pumpAndSettle();
        expect(find.text('Another loadout has that name'), findsOneWidget);
        expect(tester.widget<FilledButton>(find.byKey(const Key('loadout-save'))).onPressed, isNull);
        await tester.enterText(find.byKey(const Key('loadout-name')), 'writing2');
        await tester.pumpAndSettle();
        await tap(tester, 'loadout-save');
        final saved = world.saved.single;
        expect(saved.loadouts.keys, ['review', 'writing', 'writing2']);
        expect(saved.loadouts['writing2']!.overlays, ['docs']);
        expect(saved.loadouts['writing']!.overlays, ['docs'], reason: 'the original is as it was');
      });

      testWidgets('editing: an overlay taken out and a server added; renaming the default keeps it the default', (tester) async {
        await open(tester);
        await tap(tester, 'loadout-edit-writing');
        await tap(tester, 'loadout-remove-overlay-docs');
        await tap(tester, 'loadout-add-mcp');
        await tester.tap(find.byKey(const Key('loadout-pick-tracker')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('loadout-picker-add')));
        await tester.pumpAndSettle();
        expect(find.text('Allowed for 2 of 2 agents'), findsOneWidget);
        await tester.enterText(find.byKey(const Key('loadout-name')), 'prose');
        await tester.pumpAndSettle();
        await tap(tester, 'loadout-save');
        final saved = world.saved.single;
        expect(saved.loadouts.keys, ['review', 'prose'], reason: 'renamed where it was');
        expect(saved.loadouts['prose']!.overlays, isEmpty);
        expect(saved.loadouts['prose']!.mcp, ['tracker']);
        expect(saved.defaultLoadout, 'prose');
      });

      testWidgets('an overlay that is not installed is flagged in the editor, and nothing there switches an item inside an overlay', (tester) async {
        await open(tester);
        await tap(tester, 'loadout-edit-review');
        final row = find.byKey(const Key('loadout-row-overlay-gone'));
        expect(find.descendant(of: row, matching: find.text('Not installed')), findsOneWidget);
        expect(find.descendant(of: find.byKey(const Key('loadout-row-overlay-review')), matching: find.text('Installed')), findsOneWidget);
        expect(find.byType(Switch), findsNothing);
        expect(find.byType(Checkbox), findsNothing);
        expect(find.byType(CheckboxListTile), findsNothing);
        final order = [for (final name in ['review', 'gone']) tester.getTopLeft(find.byKey(Key('loadout-row-overlay-$name'))).dy];
        expect(order[0] < order[1], isTrue, reason: 'overlays show in the order the profile composes them, those it lost last');
      });

      testWidgets('the way to the Store keeps the draft: the editor is as it was when the section is shown again', (tester) async {
        await open(tester);
        await tap(tester, 'loadout-new');
        await tester.enterText(find.byKey(const Key('loadout-name')), 'later');
        await tester.pumpAndSettle();
        await tap(tester, 'loadout-add-skills');
        await tester.tap(find.byKey(const Key('loadout-pick-audit')));
        await tester.tap(find.byKey(const Key('loadout-picker-store')));
        await tester.pumpAndSettle();
        expect(world.storeOpened, 1);
        await pump(tester, shown: false);
        expect(find.byKey(const Key('loadout-name')), findsNothing);
        await pump(tester);
        expect(tester.widget<TextFormField>(find.byKey(const Key('loadout-name'))).initialValue, 'later');
        expect(find.byKey(const Key('loadout-row-skill-audit')), findsOneWidget, reason: 'what was picked on the way is in it');
        expect(world.saved, isEmpty, reason: 'nothing was written');
      });

      testWidgets('use in a new session names the loadout; delete asks first and writes', (tester) async {
        await open(tester);
        await tap(tester, 'loadout-use-review');
        expect(world.used, ['review']);
        await tap(tester, 'loadout-delete-writing');
        await tester.tap(find.text('Keep it'));
        await tester.pumpAndSettle();
        expect(world.saved, isEmpty);
        await tap(tester, 'loadout-delete-writing');
        await tester.tap(find.byKey(const Key('loadout-delete-confirm')));
        await tester.pumpAndSettle();
        expect(world.saved.single.loadouts.keys, ['review']);
        expect(world.saved.single.defaultLoadout, isNull, reason: 'the default goes with its loadout');
        await tap(tester, 'loadout-default-set-review');
        expect(world.saved.last.defaultLoadout, 'review');
      });
    });
  }
}
