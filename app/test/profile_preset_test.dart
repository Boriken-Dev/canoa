// A new session's profile, chosen in its footer: it sets the draft's source, model, effort and mode; changing one of them reads
// "Modified" with Save changes and Save as new profile writing canoa.cfg; the effective setup names the layer of every value; and
// the editor in Settings, Profiles writes the file the same way.
import 'dart:io' show Directory;

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/profile/profile_home_io.dart' show IoProfileHome;
import 'package:canoa/profile/profile.dart' show ProfileAccess;
import 'package:canoa/profile/profiles_panel.dart';
import 'package:canoa/session_store.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

/// A home of the test's own, so the profile layers read without a host.
IoProfileHome _home() {
  final dir = Directory.systemTemp.createTempSync('canoa-preset');
  addTearDown(() => dir.deleteSync(recursive: true));
  return IoProfileHome(dir.path);
}

SessionStore _store() => SessionStore(ledger: SessionLedger(MemoryLedgerBackend()));

const _deep = Preset(
  source: 'Claude',
  model: 'opus',
  effort: 'high',
  context: 'Read the tests before the code.',
  cacheLifetime: 'short',
);

CanoaConfig _config([Map<String, Preset> presets = const {'Deep review': _deep}]) =>
    const CanoaConfig(agents: [scripted]).withPresets(presets);

/// The text a control's pill shows.
String _pill(WidgetTester tester, String key) {
  final texts = find.descendant(of: find.byKey(Key(key)), matching: find.byType(Text));
  return texts.evaluate().map((element) => (element.widget as Text).data).join();
}

Future<void> _draft(WidgetTester tester, String source) async {
  await newSession(tester);
  await chooseSource(tester, source);
  await openFolder(tester);
}

Future<void> _menuItem(WidgetTester tester, String key) async {
  // A choice leaves the menu open, so it is opened only when it is not.
  if (find.byKey(Key(key)).evaluate().isEmpty) {
    await tester.tap(find.byKey(const Key('profile-pill')));
    await frames(tester);
  }
  await tester.tap(find.byKey(Key(key)));
  await settle(tester);
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/profile_preset_test.dart')));

  testWidgets('choosing a profile sets the draft\'s source, model and effort, and its name shows in the footer', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    await pumpApp(tester, _store(), profileHome: _home(), config: _config(), linked: agent.openLink);
    await _draft(tester, 'Codex');
    expect(_pill(tester, 'profile-pill'), 'Profile', reason: 'none chosen yet');
    await _menuItem(tester, 'profile-choice Deep review');
    expect(_pill(tester, 'profile-pill'), 'Deep review', reason: 'as the profile makes it, nothing is changed');
    expect(_pill(tester, 'new-session-source'), contains('Claude'));
    expect(_pill(tester, 'option-model'), 'Opus 5.5 · High');
    expect(agent.starts, isEmpty, reason: 'choosing a profile starts nothing');
    await finish(tester);
  });

  testWidgets('a new draft that names no profile starts with the default profile, and does not read as modified', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    final config = _config({'default': const Preset(model: 'opus', effort: 'high'), 'Deep review': _deep});
    await pumpApp(tester, _store(), profileHome: _home(), config: config, linked: agent.openLink);
    await _draft(tester, 'Claude');
    expect(_pill(tester, 'option-model'), 'Opus 5.5 · High', reason: 'the default profile\'s model and effort, with no profile chosen');
    expect(_pill(tester, 'profile-pill'), 'Profile');
    await pick(tester, 'option-effort', 'Low');
    expect(_pill(tester, 'profile-pill'), 'default · Modified', reason: 'changed from the default profile');
    // The default is the first choice, chosen by none, and is listed once.
    await tester.tap(find.byKey(const Key('profile-pill')));
    await frames(tester);
    expect(find.byKey(const Key('profile-choice-none')), findsOneWidget);
    expect(find.byKey(const Key('profile-choice default')), findsNothing);
    expect(find.descendant(of: find.byKey(const Key('profile-choice-none')), matching: find.text('Default')), findsOneWidget);
    await finish(tester);
  });

  testWidgets('changing a value marks the profile Modified; Save changes writes canoa.cfg and Back to the profile undoes it', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    final written = await pumpApp(tester, _store(), profileHome: _home(), config: _config(), linked: agent.openLink);
    await _draft(tester, 'Claude');
    await _menuItem(tester, 'profile-choice Deep review');
    await pick(tester, 'option-effort', 'Low');
    expect(_pill(tester, 'profile-pill'), 'Deep review · Modified');

    await _menuItem(tester, 'profile-reset');
    expect(_pill(tester, 'profile-pill'), 'Deep review');
    expect(_pill(tester, 'option-model'), 'Opus 5.5 · High');

    await pick(tester, 'option-effort', 'Low');
    expect(written, isEmpty);
    await _menuItem(tester, 'profile-save-changes');
    final saved = written.last.presets['Deep review']!;
    expect(saved.effort, 'low');
    expect(saved.model, 'opus');
    expect(saved.context, _deep.context, reason: 'the starting instructions are not the footer\'s to change');
    expect(saved.source, 'Claude');
    expect(_pill(tester, 'profile-pill'), 'Deep review', reason: 'now it is as the saved profile makes it');
    await finish(tester);
  });

  testWidgets('Save changes keeps what the footer cannot show: the profile\'s access and the items a provider adds', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    const held = Preset(
      source: 'Claude',
      model: 'opus',
      effort: 'high',
      access: ProfileAccess(sandbox: 'read-only', network: false),
      items: {'region': {'name': 'north'}},
    );
    final written = await pumpApp(tester, _store(), profileHome: _home(), config: _config({'Deep review': held}), linked: agent.openLink);
    await _draft(tester, 'Claude');
    await _menuItem(tester, 'profile-choice Deep review');
    await pick(tester, 'option-effort', 'Low');
    await _menuItem(tester, 'profile-save-changes');
    final saved = written.last.presets['Deep review']!;
    expect(saved.effort, 'low');
    expect(saved.access.sandbox, 'read-only');
    expect(saved.access.network, false);
    expect(saved.items, {'region': {'name': 'north'}});
    await finish(tester);
  });

  testWidgets('Save as new profile writes the draft\'s values under a name, with the starting instructions of the profile it began from',
      (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    final written = await pumpApp(tester, _store(), profileHome: _home(), config: _config(), linked: agent.openLink);
    await _draft(tester, 'Claude');
    await _menuItem(tester, 'profile-choice Deep review');
    await pick(tester, 'option-model', 'Haiku 5.5');
    expect(_pill(tester, 'profile-pill'), 'Deep review · Modified');

    await _menuItem(tester, 'profile-save-new');
    await tester.enterText(find.byKey(const Key('profile-name')), 'Deep review');
    await tester.pump();
    expect(find.text('Another profile has that name'), findsOneWidget);
    expect(tester.widget<FilledButton>(find.byKey(const Key('profile-name-save'))).onPressed, isNull);
    await tester.enterText(find.byKey(const Key('profile-name')), 'Quick look');
    await tester.pump();
    await tester.tap(find.byKey(const Key('profile-name-save')));
    await settle(tester);

    expect(written.last.presets.keys, ['default', 'Deep review', 'Quick look']);
    final made = written.last.presets['Quick look']!;
    expect(made.model, 'haiku');
    expect(made.effort, 'high');
    expect(made.context, _deep.context);
    expect(written.last.presets['Deep review'], _deep, reason: 'the profile it began from is as it was');
    expect(_pill(tester, 'profile-pill'), 'Quick look');
    await finish(tester);
  });

  for (final (label, size) in [('phone', const Size(390, 844)), ('desktop', const Size(1280, 800))]) {
    testWidgets('the profile menu in the footer with a profile changed, at $label width', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      await pumpApp(tester, _store(), profileHome: _home(), config: _config(), linked: agent.openLink, size: size);
      await _draft(tester, 'Claude');
      await _menuItem(tester, 'profile-choice Deep review');
      // The profile menu stays open after a choice, and at a phone's width it covers the model's control.
      await tester.tapAt(const Offset(2, 2));
      await frames(tester);
      await pick(tester, 'option-effort', 'Low');
      await tester.tap(find.byKey(const Key('profile-pill')));
      await frames(tester);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/profile_menu_$label.png'));
      await finish(tester);
    });

    testWidgets('the effective setup, at $label width', (tester) async {
      final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
      final config = CanoaConfig.parse('[profile]\nmodel = "gpt-9"\neffort = "low"\n[profiles."Deep review"]\nsource = "Claude"\n'
          'effort = "high"\ncontext = "Read the tests before the code."\n');
      await pumpApp(tester, _store(), profileHome: _home(), config: config, linked: agent.openLink, size: size);
      await _draft(tester, 'Claude');
      await _menuItem(tester, 'profile-choice Deep review');
      await _menuItem(tester, 'profile-effective');
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/profile_setup_$label.png'));
      await finish(tester);
    });
  }

  testWidgets('a value the agent does not offer is not taken, is said in the pill, and does not read as a change', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    final config = _config({'Odd': const Preset(source: 'Claude', model: 'no-such-model', effort: 'low')});
    await pumpApp(tester, _store(), profileHome: _home(), config: config, linked: agent.openLink);
    await _draft(tester, 'Claude');
    await _menuItem(tester, 'profile-choice Odd');
    expect(_pill(tester, 'profile-pill'), 'Odd');
    expect(_pill(tester, 'option-model'), 'Opus 5.5 · Low', reason: 'the model stays the agent\'s default, the effort is taken');
    final tip = tester.widget<Tooltip>(find.descendant(of: find.byKey(const Key('profile-pill')), matching: find.byType(Tooltip)));
    expect(tip.message, contains('model'));
    expect(tip.message, contains('no-such-model'));
    await finish(tester);
  });

  testWidgets('the effective setup names the layer of every value, the session\'s own among them, and says what the agent lacks',
      (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    final config = CanoaConfig.parse('[profile]\nmodel = "sonnet"\neffort = "low"\n[profiles.Deep]\nsource = "Claude"\n'
        'effort = "high"\ncontext = "Read the tests first."\n');
    await pumpApp(tester, _store(), profileHome: _home(), config: config, linked: agent.openLink);
    await _draft(tester, 'Claude');
    await _menuItem(tester, 'profile-choice Deep');
    await _menuItem(tester, 'profile-effective');
    expect(find.byKey(const Key('effective-setup')), findsOneWidget);
    expect(layerOf(tester, 'effort'), 'session', reason: 'the profile wins over the person\'s own low');
    expect(layerOf(tester, 'model'), 'user', reason: 'what the profile leaves unset stays the lower layer\'s');
    expect(find.textContaining('Read the tests first.'), findsOneWidget);
    expect(find.byKey(const Key('effective-refused')), findsNothing);
    await finish(tester);
  });

  testWidgets('a model in the person\'s own layer that the agent does not offer is named in the effective setup', (tester) async {
    final agent = OptionsAgent(modes: OptionsAgent.claudeModes);
    final config = CanoaConfig.parse('[profile]\nmodel = "gpt-9"\n[profiles.Plain]\nsource = "Claude"\n');
    await pumpApp(tester, _store(), profileHome: _home(), config: config, linked: agent.openLink);
    await _draft(tester, 'Claude');
    await _menuItem(tester, 'profile-choice Plain');
    await _menuItem(tester, 'profile-effective');
    expect(layerOf(tester, 'model'), 'user');
    expect(find.byKey(const Key('effective-refused')), findsOneWidget);
    expect(find.textContaining('the agent does not offer it'), findsOneWidget);
    await finish(tester);
  });

  group('Settings, Profiles', () {
    Future<List<CanoaConfig>> pumpPanel(WidgetTester tester, CanoaConfig config) async {
      final written = <CanoaConfig>[];
      await tester.pumpWidget(MaterialApp(
        home: Scaffold(
          body: SingleChildScrollView(
            child: StatefulBuilder(
              builder: (context, setState) => ProfilesPanel(
                config: config,
                agents: const ['Claude', 'Codex'],
                onChange: (edit) async {
                  config = edit(config);
                  written.add(config);
                  setState(() {});
                },
              ),
            ),
          ),
        ),
      ));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));
      return written;
    }

    testWidgets('the default profile is edited and copied but never removed, and keeps its name', (tester) async {
      await tester.binding.setSurfaceSize(const Size(900, 1600));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final written = await pumpPanel(tester, CanoaConfig.parse('[profiles.default]\nmodel = "opus"\n'));
      expect(find.byKey(const Key('profile-edit-default')), findsOneWidget);
      expect(find.byKey(const Key('profile-copy-default')), findsOneWidget);
      expect(find.byKey(const Key('profile-delete-default')), findsNothing);

      await tester.tap(find.byKey(const Key('profile-edit-default')));
      await tester.pump();
      expect(tester.widget<TextField>(find.byKey(const Key('profile-edit-name'))).enabled, isFalse);
      await tester.enterText(find.byKey(const Key('profile-edit-effort')), 'high');
      await tester.tap(find.byKey(const Key('profile-edit-save')));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));
      expect(written.last.presets['default'], const Preset(model: 'opus', effort: 'high'));
      // Whatever is written keeps it, even a write that names none.
      expect(written.last.withPresets(const {}).tables['profiles'], {'default': <String, dynamic>{}});
    });

    testWidgets('a new profile is written with its starting instructions, and an edit, a copy and a delete follow', (tester) async {
      await tester.binding.setSurfaceSize(const Size(900, 1600));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final written = await pumpPanel(tester, CanoaConfig.parse('[loadouts.lean]\n'));
      // The default profile is there before any is made.
      expect(find.byKey(const Key('profile-row-default')), findsOneWidget);
      expect(find.byKey(const Key('profile-delete-default')), findsNothing, reason: 'it cannot be removed');

      await tester.tap(find.byKey(const Key('profile-new')));
      await tester.pump();
      await tester.enterText(find.byKey(const Key('profile-edit-name')), 'Deep review');
      await tester.enterText(find.byKey(const Key('profile-edit-model')), 'opus');
      await tester.enterText(find.byKey(const Key('profile-edit-effort')), 'high');
      await tester.enterText(find.byKey(const Key('profile-edit-context')), 'Read the tests first.');
      await tester.pump();
      await tester.tap(find.byKey(const Key('profile-edit-save')));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));
      expect(written.last.presets['Deep review'],
          const Preset(model: 'opus', effort: 'high', context: 'Read the tests first.'));
      expect(find.byKey(const Key('profile-row-Deep review')), findsOneWidget);
      expect(find.byKey(const Key('profile-editor')), findsNothing);

      await tester.tap(find.byKey(const Key('profile-edit-Deep review')));
      await tester.pump();
      await tester.enterText(find.byKey(const Key('profile-edit-name')), 'Deeper');
      await tester.enterText(find.byKey(const Key('profile-edit-effort')), '');
      await tester.pump();
      await tester.tap(find.byKey(const Key('profile-edit-save')));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));
      expect(written.last.presets.keys, ['default', 'Deeper'], reason: 'renamed in place');
      expect(written.last.presets['Deeper'], const Preset(model: 'opus', context: 'Read the tests first.'));

      await tester.tap(find.byKey(const Key('profile-copy-Deeper')));
      await tester.pump();
      await tester.enterText(find.byKey(const Key('profile-edit-name')), 'Deeper');
      await tester.pump();
      expect(find.text('Another profile has that name'), findsOneWidget);
      expect(tester.widget<FilledButton>(find.byKey(const Key('profile-edit-save'))).onPressed, isNull);
      await tester.enterText(find.byKey(const Key('profile-edit-name')), 'Third');
      await tester.pump();
      await tester.tap(find.byKey(const Key('profile-edit-save')));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));
      expect(written.last.presets.keys, ['default', 'Deeper', 'Third']);

      await tester.tap(find.byKey(const Key('profile-delete-Deeper')));
      await tester.pump();
      await tester.tap(find.byKey(const Key('profile-delete-confirm')));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));
      expect(written.last.presets.keys, ['default', 'Third']);
    });
  });
}

String layerOf(WidgetTester tester, String key) => (tester.widget(find.byKey(Key('profile-layer-$key'))) as Text).data!;
