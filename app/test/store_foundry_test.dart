// Foundry Local's models in the Store over a scripted host (test/fixtures/store_link.dart): the list says what is downloaded and
// loaded, + shows what it takes before anything runs, a refused download or load is shown in the host's words, and a host without
// Foundry Local says to install it from Runtimes.
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/store/store_extension.dart';
import 'package:canoa/store/store_foundry.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_link.dart';

Map<String, Object?> _model(String id, {bool cached = false, bool loaded = false, String? alias, int size = 10137}) => {
  'id': id,
  'alias': alias,
  'task': 'chat',
  'device': 'gpu',
  'size_mb': size,
  'description': 'chat, gpu, ${(size / 1024).toStringAsFixed(1)} GB',
  'cached': cached,
  'loaded': loaded,
};

FoundryModels _models(FakeStoreHost host) {
  final shared = host.client();
  return FoundryModels(control: () async => shared, open: () async => host.client());
}

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
  test('the models are listed with where each stands, and a host without Foundry Local says to install it', () async {
    final host = FakeStoreHost([])
      ..foundryInstalled = true
      ..foundryModels = [
        _model('qwen3-14b-generic-gpu:2', alias: 'qwen3-14b', loaded: true, cached: true),
        _model('phi-4-mini-generic-cpu:1', cached: true, size: 2400),
        _model('whisper-tiny', size: 151),
      ];
    final models = _models(host);
    final type = foundryModelsType(models);
    expect(type.id, foundryTypeId);
    await type.load!();
    final items = type.items();
    expect(items.map((each) => each.name), ['qwen3-14b-generic-gpu:2', 'phi-4-mini-generic-cpu:1', 'whisper-tiny']);
    expect(items.map((each) => each.installed), [true, false, false], reason: 'a loaded model is the installed one');
    expect(items[0].summary, startsWith('Loaded · chat, gpu'));
    expect(items[1].summary, startsWith('Downloaded'));
    expect(items[2].summary, startsWith('Not downloaded'));
    expect(items[0].by, 'qwen3-14b');
    expect(items[0].tags, ['chat', 'gpu']);

    host.foundryInstalled = false;
    host.foundryModels = [];
    await models.refresh();
    final nothing = type.items().single;
    expect(nothing.install, isNull, reason: 'there is nothing to download or load');
    expect(nothing.summary, contains('Runtimes'));
    models.dispose();
  });

  test('a host that cannot list them is shown in its own words, and asked again the next time', () async {
    final host = FakeStoreHost([])..failures['foundry/models'] = ControlFailed(-32002, 'the model catalog was not listed: no network');
    final models = _models(host);
    final type = foundryModelsType(models);
    await type.load!();
    expect(type.items().single.summary, 'the model catalog was not listed: no network');
    host.failures.clear();
    host.foundryInstalled = true;
    host.foundryModels = [_model('m')];
    await type.load!();
    expect(type.items().single.name, 'm');
    models.dispose();
  });

  testWidgets('+ on a model that is not downloaded says what it takes, then downloads and loads it only once confirmed', (tester) async {
    final host = FakeStoreHost([])
      ..foundryInstalled = true
      ..foundryModels = [_model('qwen3-14b-generic-gpu:2')];
    final models = _models(host);
    final type = foundryModelsType(models);
    await tester.runAsync(type.load!);
    await _open(tester, type.items().single);
    expect(find.byKey(const Key('foundry-confirm')), findsOneWidget);
    expect(find.textContaining('downloads qwen3-14b-generic-gpu:2 (9.9 GB)'), findsOneWidget);
    expect(find.textContaining('foundry server stop'), findsOneWidget);
    expect(host.params['foundry/model'], isNull, reason: 'nothing runs before the person says yes');

    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(host.params['foundry/model'], isNull, reason: 'Cancel runs nothing');

    await _open(tester, type.items().single);
    await tester.tap(find.byKey(const Key('foundry-confirm-yes')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(host.params['foundry/model']!.map((given) => given['action']), ['download', 'load']);
    expect(host.params['foundry/model']!.every((given) => given['model'] == 'qwen3-14b-generic-gpu:2'), isTrue);
    final now = type.items().single;
    expect(now.installed, isTrue, reason: 'the list was read again after, and the model is loaded');
    models.dispose();
  });

  testWidgets('a load the host refuses is shown as the host worded it', (tester) async {
    final host = FakeStoreHost([])
      ..foundryInstalled = true
      ..foundryModels = [_model('m', cached: true)]
      ..failures['foundry/model'] = ControlFailed(-32002, 'load m: not enough memory');
    final models = _models(host);
    final type = foundryModelsType(models);
    await tester.runAsync(type.load!);
    await _open(tester, type.items().single);
    await tester.tap(find.byKey(const Key('foundry-confirm-yes')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('foundry-failed')), findsOneWidget);
    expect(find.text('load m: not enough memory'), findsOneWidget);
    expect(type.items().single.installed, isFalse);
    models.dispose();
  });

  testWidgets('a Whisper model has a page with Use as speech provider, which makes the provider and loads nothing', (tester) async {
    _tall(tester);
    final host = FakeStoreHost([])
      ..foundryInstalled = true
      ..foundryModels = [_model('openai-whisper-tiny-generic-cpu', cached: true), _model('qwen3-14b-generic-gpu:2')];
    final models = _models(host);
    final type = foundryModelsType(models);
    await tester.runAsync(type.load!);
    final items = type.items();
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => items[1].details!(context, null))));
    expect(find.byKey(const Key('foundry-use-speech')), findsNothing, reason: 'a chat model is not a speech model, and its page has the window only');
    expect(find.byKey(const Key('foundry-window')), findsOneWidget);
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => items[0].details!(context, null))));
    expect(find.byKey(const Key('foundry-speech-not-loaded')), findsOneWidget);
    expect(find.textContaining('never loads a model'), findsOneWidget);
    await tester.tap(find.byKey(const Key('foundry-use-speech')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 100)));
    await tester.pumpAndSettle();
    expect(host.params['speech/enable']!.single, {'offer': 'foundry', 'model': 'openai-whisper-tiny-generic-cpu'});
    expect(host.params['foundry/model'], isNull, reason: 'the model was not loaded');
    expect(find.byKey(const Key('foundry-speech-done')), findsOneWidget);
    expect(find.textContaining('"foundry"'), findsOneWidget);
    models.dispose();
  });

  testWidgets('a refusal to make the provider is shown in the host\'s words', (tester) async {
    final host = FakeStoreHost([])
      ..foundryInstalled = true
      ..foundryModels = [_model('openai-whisper-tiny-generic-cpu')]
      ..failures['speech/enable'] = ControlFailed(-32001, 'this key may not set up a speech provider');
    final models = _models(host);
    final type = foundryModelsType(models);
    await tester.runAsync(type.load!);
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => type.items().single.details!(context, null))));
    await tester.tap(find.byKey(const Key('foundry-use-speech')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 100)));
    await tester.pumpAndSettle();
    expect(find.text('this key may not set up a speech provider'), findsOneWidget);
    expect(find.byKey(const Key('foundry-speech-done')), findsNothing);
    models.dispose();
  });

  testWidgets('+ on a loaded model offers to unload it', (tester) async {
    final host = FakeStoreHost([])
      ..foundryInstalled = true
      ..foundryModels = [_model('m', cached: true, loaded: true)];
    final models = _models(host);
    final type = foundryModelsType(models);
    await tester.runAsync(type.load!);
    await _open(tester, type.items().single);
    expect(find.text('Unload m'), findsOneWidget);
    await tester.tap(find.byKey(const Key('foundry-confirm-yes')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(host.params['foundry/model']!.single['action'], 'unload');
    expect(type.items().single.installed, isFalse);
    models.dispose();
  });

  testWidgets('a model page shows the model\'s own window and keeps the one the person sets, lowered to its context length', (tester) async {
    _tall(tester);
    final downloaded = _model('qwen3-14b-generic-gpu:2', cached: true)..['window'] = {'context_length': 40960, 'max_length': 8192, 'chosen': null};
    final host = FakeStoreHost([])
      ..foundryInstalled = true
      ..foundryModels = [downloaded];
    final models = _models(host);
    final type = foundryModelsType(models);
    await tester.runAsync(type.load!);
    final item = type.items().single;
    expect(item.details, isNotNull, reason: 'the tile opens a page');
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => item.details!(context, null))));
    await tester.pumpAndSettle();
    expect(find.textContaining('40960 tokens'), findsOneWidget);
    expect(find.textContaining('8192 tokens'), findsOneWidget);
    expect(find.textContaining('none: the model runs with its own'), findsOneWidget);
    expect(find.byKey(const Key('foundry-window-engine')), findsOneWidget);
    expect(find.textContaining('sdk engine'), findsWidgets);
    expect(find.textContaining('does not use this setting'), findsOneWidget);
    expect(tester.widget<OutlinedButton>(find.byKey(const Key('foundry-window-clear'))).onPressed, isNull, reason: 'there is nothing to remove');

    await tester.enterText(find.byKey(const Key('foundry-window-field')), '16384');
    await tester.tap(find.byKey(const Key('foundry-window-set')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(host.params['foundry/window']!.single, {'model': 'qwen3-14b-generic-gpu:2', 'window': 16384});
    expect(find.text('Kept: 16384 tokens.'), findsOneWidget);
    expect(find.textContaining('Your setting: 16384 tokens'), findsOneWidget);
    expect(models.models.single.chosenWindow, 16384, reason: 'the list was read again');

    await tester.enterText(find.byKey(const Key('foundry-window-field')), '100000');
    await tester.tap(find.byKey(const Key('foundry-window-set')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(find.textContaining('above the model\'s own context length: the window is 40960 tokens'), findsOneWidget);
    expect(models.models.single.chosenWindow, 40960);

    await tester.tap(find.byKey(const Key('foundry-window-clear')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(host.params['foundry/window']!.last, {'model': 'qwen3-14b-generic-gpu:2', 'window': null});
    expect(models.models.single.chosenWindow, isNull);
    models.dispose();
  });

  testWidgets('a window that is not a number of tokens is not sent, and a refusal is shown as the host worded it', (tester) async {
    _tall(tester);
    final host = FakeStoreHost([])
      ..foundryInstalled = true
      ..foundryModels = [_model('m')];
    final models = _models(host);
    final type = foundryModelsType(models);
    await tester.runAsync(type.load!);
    final item = type.items().single;
    await tester.pumpWidget(MaterialApp(home: Builder(builder: (context) => item.details!(context, null))));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('foundry-window-unknown')), findsOneWidget, reason: 'a model not downloaded has no numbers of its own');
    for (final bad in ['', '0', 'many', '-3']) {
      await tester.enterText(find.byKey(const Key('foundry-window-field')), bad);
      await tester.tap(find.byKey(const Key('foundry-window-set')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('foundry-window-failed')), findsOneWidget, reason: 'for "$bad"');
    }
    expect(host.params['foundry/window'], isNull);

    host.failures['foundry/window'] = ControlFailed(-32002, 'this host was not started with a canoa.cfg a page may edit');
    await tester.enterText(find.byKey(const Key('foundry-window-field')), '4096');
    await tester.tap(find.byKey(const Key('foundry-window-set')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
    await tester.pumpAndSettle();
    expect(find.text('this host was not started with a canoa.cfg a page may edit'), findsOneWidget);
    models.dispose();
  });
}
