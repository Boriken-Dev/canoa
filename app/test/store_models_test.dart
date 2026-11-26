// The models of a Store type over a scripted host (test/fixtures/store_link.dart): the list says where each stands, + shows what it
// takes before anything runs, a refused action is shown in the host's words, a model's page draws the sections and the settings form
// the library described, and a model that may be the person's speech-to-text has a button for it. The type here is `runner`: the app
// names no vendor's, so nothing in it can depend on one.
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/l10n/app_text.dart';
import 'package:canoa/store/store_extension.dart';
import 'package:canoa/store/store_models.dart';
import 'package:canoa/store/store_view.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_link.dart';

const _type = 'runner';

/// What a library describes for a number a model is run with, as the host answers it.
Map<String, Object?> get _settings => {
  'help': 'The window a model is run with.',
  'schema': {
    'type': 'object',
    'properties': {
      'window': {'type': ['integer', 'null'], 'title': 'Window', 'description': 'Tokens a prompt and its answer take together.'},
    },
  },
  'form': {
    'fields': {
      'window': {'unit': 'tokens', 'placeholder': 'the model\'s own'},
    },
  },
};

/// A row as a library words it, with where the model stands.
Map<String, Object?> _row(String id, {bool cached = false, bool loaded = false, String? by, int size = 10137, bool speech = false, Map<String, Object?> values = const {}}) {
  final gb = (size / 1024).toStringAsFixed(1);
  final stand = loaded ? 'Loaded' : cached ? 'Downloaded' : 'Not downloaded';
  final (title, confirm, label, steps) = loaded
      ? ('Unload $id', 'Takes the model out of memory.', 'Unload', ['unload'])
      : cached
          ? ('Load $id', 'Loads $id. A loaded model stays in memory.', 'Load', ['load'])
          : ('Download and load $id', 'Downloads $id ($gb GB) into its cache, which can take long, and then loads it. A loaded model stays in memory.', 'Download and load', ['download', 'load']);
  return {
    'id': id,
    'by': by,
    'summary': '$stand · chat, gpu, $gb GB',
    'tags': ['chat', 'gpu'],
    'on': loaded,
    'state': loaded ? 'Loaded' : cached ? 'Downloaded' : null,
    'facts': [
      if (by != null) {'label': 'Alias', 'value': by},
    ],
    'actions': [
      {'title': title, 'confirm': confirm, 'label': label, 'steps': steps},
    ],
    'sections': [
      {
        'title': 'Window',
        'facts': cached
            ? [
                {'label': 'Most the model takes (context length)', 'value': '40960 tokens'},
                {'label': 'Prompt and answer together, as cached (max length)', 'value': '8192 tokens'},
              ]
            : <Object?>[],
        'paragraphs': ['A longer window lets a longer prompt through.', 'It applies when an account runs this model with the sdk engine.'],
        'settings': true,
      },
    ],
    'speech': speech
        ? {'text': 'Dictation can send its audio to this model: nothing is sent to another service. Load it with + first.', 'caution': loaded ? null : 'It is not loaded now.'}
        : null,
    'values': values,
  };
}

/// A host whose models are [models], moved by `store/model` the way a library's program would, and whose settings it keeps.
FakeStoreHost _host(List<Map<String, Object?>> models, {bool settings = true, String? unavailable}) {
  final host = FakeStoreHost([]);
  host.modelsAnswer = (given) => {
    'type': given['type'],
    'catalog': 'Runner',
    'publisher': 'Acme',
    'missing': 'Not installed: install it from the Runner list.',
    'unavailable': unavailable,
    'settings': settings ? _settings : null,
    'speech': models.any((row) => row['speech'] != null),
    'models': models,
  };
  host.onModel = (given) {
    final at = models.indexWhere((row) => row['id'] == given['model']);
    var row = models[at];
    switch (given['action']) {
      case 'download':
        row = _row(row['id']! as String, cached: true, by: row['by'] as String?, speech: row['speech'] != null, values: row['values']! as Map<String, Object?>);
      case 'load':
        if (row['state'] == null) throw ControlFailed(-32002, '${row['id']} is not downloaded');
        row = _row(row['id']! as String, cached: true, loaded: true, by: row['by'] as String?, speech: row['speech'] != null, values: row['values']! as Map<String, Object?>);
      case 'unload':
        row = _row(row['id']! as String, cached: true, by: row['by'] as String?, speech: row['speech'] != null, values: row['values']! as Map<String, Object?>);
    }
    models[at] = row;
  };
  host.onSetting = (given) {
    final at = models.indexWhere((row) => row['id'] == given['model']);
    final values = Map<String, Object?>.from(given['values']! as Map);
    String? note;
    // The library holds a number to what the model allows, and says so.
    if (values['window'] case final int window when window > 40960) {
      note = '$window tokens is above the model\'s own context length: the window is 40960 tokens';
      values['window'] = 40960;
    }
    final row = models[at];
    models[at] = _row(row['id']! as String, cached: row['state'] != null, loaded: row['on'] == true, by: row['by'] as String?, speech: row['speech'] != null, values: values);
    return {'ok': true, 'model': given['model'], 'values': values, 'note': note};
  };
  return host;
}

TypeModels _models(FakeStoreHost host) {
  final shared = host.client();
  return TypeModels(type: _type, title: 'Runner models', control: () async => shared, open: () async => host.client());
}

ExtensionStoreType _typeOf(TypeModels models) => modelsStoreType(models: [models], icon: Icons.memory);

Future<void> _open(WidgetTester tester, ExtensionStoreItem item) async {
  await tester.pumpWidget(
    MaterialApp(
      home: Builder(
        builder: (context) => TextButton(key: const Key('go'), onPressed: () => item.install!(context), child: const Text('go')),
      ),
    ),
  );
  await tester.tap(find.byKey(const Key('go')));
  await tester.pumpAndSettle();
}

/// A window tall enough for the model page's whole list, so its buttons can be tapped.
void _tall(WidgetTester tester) {
  tester.view.physicalSize = const Size(900, 2000);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

void main() {
  test('the models are listed with where each stands, and a library that says it is unavailable is shown as it worded it', () async {
    final models = [
      _row('big:2', by: 'big', loaded: true, cached: true),
      _row('small:1', cached: true, size: 2400),
      _row('tiny', size: 151),
    ];
    final host = _host(models);
    final shown = _models(host);
    final type = _typeOf(shown);
    expect(type.id, modelsTypeId);
    expect(type.label, 'Models');
    expect(type.items(), isEmpty, reason: 'nothing is shown before the host has answered');
    await type.load!();
    final items = type.items();
    expect(items.map((each) => each.name), ['big:2', 'small:1', 'tiny']);
    expect(items.map((each) => each.installed), [true, false, false], reason: 'a loaded model is the installed one');
    expect(items[0].summary, startsWith('Loaded · chat, gpu'));
    expect(items[1].summary, startsWith('Downloaded'));
    expect(items[2].summary, startsWith('Not downloaded'));
    expect(items[0].by, 'big');
    expect(items[2].by, 'Acme', reason: 'a row that names no one is by the library\'s publisher');
    expect(items[0].tags, ['chat', 'gpu']);
    expect(items.every((each) => each.catalog == 'Runner'), isTrue);
    shown.dispose();

    final missing = _host([], unavailable: 'Not installed: install it from the Runner list, and its models are listed here.');
    final none = _models(missing);
    final nothing = _typeOf(none);
    await nothing.load!();
    expect(nothing.items(), isEmpty, reason: 'a library that lists none is a note, never a row');
    final note = nothing.notices!().single;
    expect(note.problem, isFalse);
    expect(note.words(AppText.base), contains('Runner list'));
    none.dispose();
  });

  test('the models of every type are one page, each grouped under the catalog its library names', () async {
    final host = FakeStoreHost([])
      ..modelsAnswer = (given) => {
        'type': given['type'],
        'catalog': given['type'] == 'runner' ? 'Runner' : 'Speaker',
        'publisher': 'Acme',
        'models': [
          {'id': '${given['type']}-one', 'summary': 'one', 'tags': <String>[], 'actions': <Object?>[]},
          {'id': '${given['type']}-two', 'summary': 'two', 'tags': <String>[], 'actions': <Object?>[]},
        ],
      };
    final shared = host.client();
    final runner = TypeModels(type: 'runner', title: 'Runner models', control: () async => shared, open: () async => host.client());
    final speaker = TypeModels(type: 'speaker', title: 'Speaker voices', control: () async => shared, open: () async => host.client());
    final page = modelsStoreType(models: [runner, speaker]);
    expect(page.id, modelsTypeId);
    await page.load!();
    expect(page.items().map((each) => '${each.catalog}/${each.name}'), ['Runner/runner-one', 'Runner/runner-two', 'Speaker/speaker-one', 'Speaker/speaker-two']);
    runner.dispose();
    speaker.dispose();
  });

  test('a host that cannot list them is shown in its own words, and asked again the next time', () async {
    final host = _host([_row('m')])..failures['store/models'] = ControlFailed(-32002, 'the model catalog was not listed: no network');
    final shown = _models(host);
    final type = _typeOf(shown);
    await type.load!();
    expect(type.items(), isEmpty, reason: 'an error is not a model');
    final notice = type.notices!().single;
    expect(notice.problem, isTrue);
    expect(notice.words(AppText.base), 'Runner models: the model catalog was not listed: no network');
    host.failures.clear();
    await notice.retry!();
    expect(type.notices!(), isEmpty);
    expect(type.items().single.name, 'm');
    expect(host.params['store/models']!.first, {'type': _type});
    shown.dispose();
  });

  testWidgets('a model whose files are checked shows a check mark that names the algorithm, and one with none shows no mark', (tester) async {
    _tall(tester);
    final host = _host([
      {..._row('checked', by: 'big'), 'verified': 'SHA-256'},
      {..._row('mixed'), 'verified': 'SHA-256, MD5'},
      _row('bare'),
    ]);
    final shown = _models(host);
    final page = _typeOf(shown);
    await tester.runAsync(page.load!);
    expect(page.items().map((each) => each.verified), ['SHA-256', 'SHA-256, MD5', null]);
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: ExtensionBody(type: page, view: StoreView(modelsTypeId)))));
    await tester.pumpAndSettle();
    expect(find.descendant(of: find.byKey(const Key('store-entry-Runner-checked')), matching: find.byKey(const Key('verified-mark'))), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('store-entry-Runner-bare')), matching: find.byKey(const Key('verified-mark'))), findsNothing);
    final mark = find.descendant(of: find.byKey(const Key('store-entry-Runner-mixed')), matching: find.byType(Tooltip));
    expect(tester.widget<Tooltip>(mark.first).message, 'Verified with SHA-256, MD5');
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/store_models_verified_desktop.png'));

    for (final (id, field) in [('checked', 'SHA-256'), ('bare', 'No')]) {
      final item = page.items().firstWhere((each) => each.name == id);
      await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => item.details!(context, null))));
      await tester.pumpAndSettle();
      expect(find.textContaining('Verified: $field', findRichText: true), findsOneWidget, reason: '$id: the page says how its files are checked, or No');
    }
    shown.dispose();
  });

  testWidgets('a type that failed shows a notice with Retry next to the models of one that worked, and no row of its own', (tester) async {
    var down = true;
    final host = FakeStoreHost([])
      ..modelsAnswer = (given) {
        if (given['type'] == 'runner' && down) {
          throw ControlFailed(-32002, 'the model catalog was not listed: runner exited 1: no network');
        }
        return {
          'type': given['type'],
          'catalog': given['type'] == 'runner' ? 'Runner' : 'Speaker',
          'publisher': 'Acme',
          'models': [
            {'id': '${given['type']}-one', 'summary': 'one', 'tags': <String>[], 'actions': <Object?>[]},
          ],
        };
      };
    final shared = host.client();
    final broken = TypeModels(type: 'runner', title: 'Runner models', control: () async => shared, open: () async => host.client());
    final working = TypeModels(type: 'speaker', title: 'Speaker voices', control: () async => shared, open: () async => host.client());
    final page = modelsStoreType(models: [broken, working]);
    await tester.binding.setSurfaceSize(const Size(900, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: ExtensionBody(type: page, view: StoreView(modelsTypeId)))));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('store-type-notice-runner')), findsOneWidget);
    expect(find.text('Runner models: the model catalog was not listed: runner exited 1: no network'), findsOneWidget);
    expect(find.byKey(const Key('store-entry-Runner models-Runner models')), findsNothing, reason: 'the error is not an item');
    expect(find.byKey(const Key('store-entry-Runner-Runner')), findsNothing);
    expect(find.byKey(const Key('store-entry-Speaker-speaker-one')), findsOneWidget, reason: 'the type that worked lists its model');
    expect(find.byKey(const Key('store-count')), findsOneWidget);
    expect(tester.widget<Chip>(find.byKey(const Key('store-count'))).label, isA<Text>().having((text) => text.data, 'count', '1'));
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/store_type_problem_desktop.png'));

    down = false;
    await tester.tap(find.byKey(const Key('store-type-notice-retry-runner')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('store-type-notice-runner')), findsNothing, reason: 'Retry listed it again');
    expect(find.byKey(const Key('store-entry-Runner-runner-one')), findsOneWidget);
    expect(host.params['store/models']!.last, {'type': 'runner', 'refresh': true});
    broken.dispose();
    working.dispose();
  });

  testWidgets('a type whose program is not there shows a quiet note, not a row', (tester) async {
    final host = _host([], unavailable: 'Not installed: install it from the Runner list, and its models are listed here.');
    final shown = _models(host);
    final page = _typeOf(shown);
    await tester.binding.setSurfaceSize(const Size(900, 500));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: ExtensionBody(type: page, view: StoreView(modelsTypeId)))));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('store-type-notice-runner')), findsOneWidget);
    expect(find.textContaining('Runner list'), findsOneWidget);
    expect(find.byKey(const Key('store-type-notice-retry-runner')), findsNothing, reason: 'a note has nothing to ask again');
    expect(find.byKey(const Key('store-entry-Runner-Runner')), findsNothing);
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/store_type_unavailable_desktop.png'));
    shown.dispose();
  });

  testWidgets('+ on a model that is not downloaded says what it takes, then runs its steps only once confirmed', (tester) async {
    final host = _host([_row('big:2')]);
    final shown = _models(host);
    final type = _typeOf(shown);
    await tester.runAsync(type.load!);
    await _open(tester, type.items().single);
    expect(find.byKey(const Key('model-confirm')), findsOneWidget);
    expect(find.textContaining('Downloads big:2 (9.9 GB)'), findsOneWidget);
    expect(find.textContaining('stays in memory'), findsOneWidget);
    expect(host.params['store/model'], isNull, reason: 'nothing runs before the person says yes');

    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(host.params['store/model'], isNull, reason: 'Cancel runs nothing');

    await _open(tester, type.items().single);
    await tester.tap(find.byKey(const Key('model-confirm-yes')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(host.params['store/model']!.map((given) => given['action']), ['download', 'load']);
    expect(host.params['store/model']!.every((given) => given['model'] == 'big:2' && given['type'] == _type), isTrue);
    expect(type.items().single.installed, isTrue, reason: 'the list was read again after, and the model is loaded');
    shown.dispose();
  });

  testWidgets('an action the host refuses is shown as the host worded it', (tester) async {
    final host = _host([_row('m', cached: true)])..failures['store/model'] = ControlFailed(-32002, 'load m: not enough memory');
    final shown = _models(host);
    final type = _typeOf(shown);
    await tester.runAsync(type.load!);
    await _open(tester, type.items().single);
    await tester.tap(find.byKey(const Key('model-confirm-yes')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('model-failed')), findsOneWidget);
    expect(find.text('load m: not enough memory'), findsOneWidget);
    expect(type.items().single.installed, isFalse);
    shown.dispose();
  });

  testWidgets('a model that may be the speech-to-text has Use as speech provider, which makes the provider of the type and loads nothing', (tester) async {
    _tall(tester);
    final host = _host([_row('talk', cached: true, speech: true), _row('chat:2')]);
    final shown = _models(host);
    final type = _typeOf(shown);
    await tester.runAsync(type.load!);
    final items = type.items();
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => items[1].details!(context, null))));
    expect(find.byKey(const Key('model-use-speech')), findsNothing, reason: 'a chat model is not a speech model');
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => items[0].details!(context, null))));
    expect(find.byKey(const Key('model-speech-caution')), findsOneWidget);
    expect(find.textContaining('nothing is sent to another service'), findsOneWidget);
    await tester.tap(find.byKey(const Key('model-use-speech')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 100)));
    await tester.pumpAndSettle();
    expect(host.params['speech/enable']!.single, {'offer': _type, 'model': 'talk'});
    expect(host.params['store/model'], isNull, reason: 'the model was not loaded');
    expect(find.byKey(const Key('model-speech-done')), findsOneWidget);
    shown.dispose();
  });

  testWidgets('a refusal to make the provider is shown in the host\'s words', (tester) async {
    final host = _host([_row('talk', speech: true)])..failures['speech/enable'] = ControlFailed(-32001, 'this key may not set up a speech provider');
    final shown = _models(host);
    final type = _typeOf(shown);
    await tester.runAsync(type.load!);
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => type.items().single.details!(context, null))));
    await tester.tap(find.byKey(const Key('model-use-speech')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 100)));
    await tester.pumpAndSettle();
    expect(find.text('this key may not set up a speech provider'), findsOneWidget);
    expect(find.byKey(const Key('model-speech-done')), findsNothing);
    shown.dispose();
  });

  testWidgets('+ on a loaded model offers to unload it', (tester) async {
    final host = _host([_row('m', cached: true, loaded: true)]);
    final shown = _models(host);
    final type = _typeOf(shown);
    await tester.runAsync(type.load!);
    await _open(tester, type.items().single);
    expect(find.text('Unload m'), findsOneWidget);
    await tester.tap(find.byKey(const Key('model-confirm-yes')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(host.params['store/model']!.single['action'], 'unload');
    expect(type.items().single.installed, isFalse);
    shown.dispose();
  });

  testWidgets('a model page draws the library\'s sections and the form of its settings, and keeps what the person sets as the host held it', (tester) async {
    _tall(tester);
    final host = _host([_row('big:2', cached: true, by: 'big')]);
    final shown = _models(host);
    final type = _typeOf(shown);
    await tester.runAsync(type.load!);
    final item = type.items().single;
    expect(item.details, isNotNull, reason: 'the tile opens a page');
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => item.details!(context, null))));
    await tester.pumpAndSettle();
    expect(find.textContaining('40960 tokens'), findsOneWidget);
    expect(find.textContaining('8192 tokens'), findsOneWidget);
    expect(find.textContaining('with the sdk engine'), findsOneWidget);
    expect(find.byKey(const Key('model-setting-help')), findsOneWidget);
    expect(find.text('Window'), findsWidgets);

    await tester.enterText(find.byKey(const Key('model-setting-field-window')), '16384');
    await tester.tap(find.byKey(const Key('model-setting-save')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(host.params['store/model_setting']!.single, {'type': _type, 'model': 'big:2', 'values': {'window': 16384}});
    expect(find.text('Saved.'), findsOneWidget);
    expect(shown.models.single.values, {'window': 16384}, reason: 'the list was read again');

    await tester.enterText(find.byKey(const Key('model-setting-field-window')), '100000');
    await tester.tap(find.byKey(const Key('model-setting-save')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(find.textContaining('the window is 40960 tokens'), findsOneWidget, reason: 'the library lowered it and said so');
    expect(shown.models.single.values, {'window': 40960});

    await tester.enterText(find.byKey(const Key('model-setting-field-window')), '');
    await tester.tap(find.byKey(const Key('model-setting-save')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(host.params['store/model_setting']!.last, {'type': _type, 'model': 'big:2', 'values': <String, Object?>{}});
    expect(shown.models.single.values, isEmpty);
    expect(find.text('The settings are removed.'), findsOneWidget);
    shown.dispose();
  });

  testWidgets('a type that keeps no setting has no form, and a refusal to keep one is shown as the host worded it', (tester) async {
    _tall(tester);
    final bare = _host([_row('m')], settings: false);
    final shownBare = _models(bare);
    final typeBare = _typeOf(shownBare);
    await tester.runAsync(typeBare.load!);
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => typeBare.items().single.details!(context, null))));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('model-setting-save')), findsNothing);
    shownBare.dispose();

    final host = _host([_row('m')])..failures['store/model_setting'] = ControlFailed(-32002, 'this host was not started with a canoa.cfg a page may edit');
    final shown = _models(host);
    final type = _typeOf(shown);
    await tester.runAsync(type.load!);
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => type.items().single.details!(context, null))));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('model-setting-field-window')), '4096');
    await tester.tap(find.byKey(const Key('model-setting-save')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(find.text('this host was not started with a canoa.cfg a page may edit'), findsOneWidget);
    shown.dispose();
  });
}
