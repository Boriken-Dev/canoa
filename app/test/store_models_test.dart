// The models of a Store type over a scripted host (test/fixtures/store_link.dart): the list says where each stands, + shows what it
// takes before anything runs, a refused action is shown in the host's words, a model's page draws the sections and the settings form
// the library described, and a model that may be the person's speech-to-text has a button for it. The type here is `runner`: the app
// names no vendor's, so nothing in it can depend on one.
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/store/store_extension.dart';
import 'package:canoa/store/store_models.dart';
import 'package:flutter/material.dart';
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

ExtensionStoreType _typeOf(TypeModels models) => modelsStoreType(id: _type, models: models, icon: Icons.memory);

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
    expect(type.id, modelsTypeId(_type));
    expect(type.label, 'Runner models');
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
    final only = nothing.items().single;
    expect(only.install, isNull, reason: 'there is nothing to download or load');
    expect(only.summary, contains('Runner list'));
    none.dispose();
  });

  test('a host that cannot list them is shown in its own words, and asked again the next time', () async {
    final host = _host([_row('m')])..failures['store/models'] = ControlFailed(-32002, 'the model catalog was not listed: no network');
    final shown = _models(host);
    final type = _typeOf(shown);
    await type.load!();
    expect(type.items().single.summary, 'the model catalog was not listed: no network');
    host.failures.clear();
    await type.load!();
    expect(type.items().single.name, 'm');
    expect(host.params['store/models']!.first, {'type': _type});
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
