// The profile editor offers what the agent of the chosen source lists for its model, effort and mode, beside a box that still takes
// any text, and asks nothing of a source it cannot ask.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/profile/profile_editor.dart';
import 'package:canoa/sources/source_choices.dart' show ModelChoice;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

const _offered = ProfileChoices(
  models: [
    ModelChoice(id: 'opus', name: 'Opus', efforts: ['low', 'high']),
    ModelChoice(id: 'sonnet', name: 'sonnet', efforts: ['medium']),
  ],
  modes: [('plan', 'Plan'), ('auto', 'Auto')],
);

Future<List<(String, Preset)>> _pump(WidgetTester tester, {Preset preset = const Preset(source: 'Claude'), Future<ProfileChoices?> Function(String)? choicesFor, List<String>? asked}) async {
  await tester.binding.setSurfaceSize(const Size(900, 1600));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final saved = <(String, Preset)>[];
  await tester.pumpWidget(MaterialApp(
    home: Scaffold(
      body: SingleChildScrollView(
        child: ProfileEditor(
          original: null,
          preset: preset,
          taken: const {},
          sources: const ['Claude', 'Codex'],
          loadouts: const [],
          choicesFor: choicesFor == null
              ? null
              : (source) {
                  asked?.add(source);
                  return choicesFor(source);
                },
          onSave: (name, made) async {
            saved.add((name, made));
            return null;
          },
          onCancel: () {},
        ),
      ),
    ),
  ));
  await tester.pumpAndSettle();
  return saved;
}

Future<void> _save(WidgetTester tester) async {
  await tester.enterText(find.byKey(const Key('profile-edit-name')), 'Deep');
  await tester.pump();
  await tester.ensureVisible(find.byKey(const Key('profile-edit-save')));
  await tester.tap(find.byKey(const Key('profile-edit-save')));
  await tester.pumpAndSettle();
}

void main() {
  testWidgets("the models, efforts and modes the agent lists are offered, an effort follows the model, and any other text is kept as typed", (tester) async {
    final asked = <String>[];
    final saved = await _pump(tester, choicesFor: (_) async => _offered, asked: asked);
    expect(asked, ['Claude']);

    await tester.tap(find.byKey(const Key('profile-edit-model-pick')));
    await tester.pumpAndSettle();
    expect(find.text('Opus (opus)'), findsOneWidget);
    expect(find.text('sonnet'), findsOneWidget, reason: 'a model named as its id is shown once');
    await tester.tap(find.text('Opus (opus)'));
    await tester.pumpAndSettle();
    expect(tester.widget<TextField>(find.byKey(const Key('profile-edit-model'))).controller!.text, 'opus');

    await tester.tap(find.byKey(const Key('profile-edit-effort-pick')));
    await tester.pumpAndSettle();
    expect(find.text('low'), findsOneWidget);
    expect(find.text('high'), findsOneWidget);
    expect(find.text('medium'), findsNothing, reason: 'the efforts of the model chosen');
    await tester.tap(find.text('high'));
    await tester.pumpAndSettle();

    await tester.enterText(find.byKey(const Key('profile-edit-mode')), 'some-new-mode');
    await _save(tester);
    expect(saved.single.$2, const Preset(source: 'Claude', model: 'opus', effort: 'high', mode: 'some-new-mode'));
  });

  testWidgets('a source whose agent cannot be asked has the boxes only, and choosing another source asks again and drops the old list', (tester) async {
    final asked = <String>[];
    await _pump(tester, choicesFor: (source) async => source == 'Claude' ? _offered : null, asked: asked, preset: const Preset());
    expect(asked, isEmpty, reason: 'the session\'s own source has no agent to ask');
    expect(find.byKey(const Key('profile-edit-model-pick')), findsNothing);
    await tester.enterText(find.byKey(const Key('profile-edit-model')), 'typed');

    await tester.tap(find.byKey(const Key('profile-edit-source')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Claude').last);
    await tester.pumpAndSettle();
    expect(asked, ['Claude']);
    expect(find.byKey(const Key('profile-edit-model-pick')), findsOneWidget);

    await tester.tap(find.byKey(const Key('profile-edit-source')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Codex').last);
    await tester.pumpAndSettle();
    expect(asked, ['Claude', 'Codex']);
    expect(find.byKey(const Key('profile-edit-model-pick')), findsNothing, reason: 'Codex listed nothing');
    expect(tester.widget<TextField>(find.byKey(const Key('profile-edit-model'))).controller!.text, 'typed');
  });

  testWidgets('an editor given no way to ask is the boxes alone', (tester) async {
    await _pump(tester);
    expect(find.byKey(const Key('profile-edit-model-pick')), findsNothing);
    expect(find.byKey(const Key('profile-edit-effort-pick')), findsNothing);
    expect(find.byKey(const Key('profile-edit-mode-pick')), findsNothing);
  });
}
