// The models that install on their own, over a scripted host (test/fixtures/store_link.dart): + on a model downloads it when the
// program it runs on is there, and offers the program first, in the one confirmation that shows the program's plan with the model
// under it, when it is not. An installed model is removed from its tile. The type here is `runner`: the app names no vendor's.
import 'package:canoa/store/store_extensions.dart';
import 'package:canoa/store/store_models.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_link.dart';

const _type = 'runner';

/// A row as a library words it: installed (and removable) or not (and downloadable).
Map<String, Object?> _row(String id, {required bool installed}) => {
  'id': id,
  'summary': 'a model',
  'tags': <String>['speech-to-text'],
  'on': installed,
  'state': installed ? 'installed' : null,
  'actions': [
    if (installed)
      {
        'title': 'Remove $id',
        'confirm': 'Takes $id off this computer.',
        'label': 'Remove',
        'steps': ['delete_files'],
      }
    else
      {
        'title': 'Install $id',
        'confirm':
            'Downloads $id (78 MB) and checks it against its publisher\'s SHA-256.',
        'label': 'Install',
        'steps': ['fetch_files'],
      },
  ],
};

/// A host with one program store, `runner_store`, and the models `tiny` and `base`, kept as the library would: a download marks one
/// installed and a delete takes it away. The program is found when [there].
({FakeStoreHost host, FakeStore store, Set<String> installed}) _world({
  bool there = false,
}) {
  final store = FakeStore(
    'runner_store',
    title: 'Runner',
    extensionType: _type,
    library: 'canoa-runner',
    libraryTitle: 'Runner library',
    latest: '1.0.0',
    copies: there ? [copyOf('1.0.0')] : const [],
    plan: {
      'install': planOf(
        'install',
        '1.0.0',
        downloads: [
          {
            'url': 'https://example.test/runner-1.0.0.bin',
            'bytes': 1000,
            'check': {'kind': 'sha256', 'value': 'ab' * 32},
          },
        ],
        bytes: 1000,
      ),
    },
  );
  final installed = <String>{};
  final host = FakeStoreHost([store])
    ..extensionTypes = [
      {
        'id': _type,
        'title': 'Runner',
        'stores': ['runner_store'],
        'extension': 'built-in',
        'extension_title': 'Runner library',
        'models': {'provider': 'runner_models', 'title': 'Runner models'},
      },
    ];
  host.modelsAnswer = (given) => {
    'type': given['type'],
    'catalog': 'Runner',
    'publisher': 'Acme',
    'program': {
      'provider': 'runner_store',
      'title': 'Runner',
      'installed': store.copies.isNotEmpty,
    },
    'models': [
      for (final id in ['tiny', 'base'])
        _row(id, installed: installed.contains(id)),
    ],
  };
  host.onModel = (given) {
    switch (given['action']) {
      case 'fetch_files':
        installed.add(given['model']! as String);
      case 'delete_files':
        installed.remove(given['model']! as String);
    }
  };
  host.afterApply = (each, plan) => each.copies = [copyOf('1.0.0')];
  return (host: host, store: store, installed: installed);
}

TypeModels _models(FakeStoreHost host) {
  final shared = host.client();
  return TypeModels(
    type: _type,
    title: 'Runner models',
    control: () async => shared,
    open: () async => host.client(),
    programs: host.programs(kind: 'programs'),
  );
}

Future<void> _open(
  WidgetTester tester,
  void Function(BuildContext context) press,
) async {
  await tester.pumpWidget(
    MaterialApp(
      home: Builder(
        builder: (context) => TextButton(
          key: const Key('go'),
          onPressed: () => press(context),
          child: const Text('go'),
        ),
      ),
    ),
  );
  await tester.tap(find.byKey(const Key('go')));
  await _host(tester);
}

/// Lets the scripted host answer and the page draw what it said.
Future<void> _host(WidgetTester tester) async {
  await tester.runAsync(
    () => Future<void>.delayed(const Duration(milliseconds: 300)),
  );
  await tester.pumpAndSettle();
}

void main() {
  testWidgets(
    'a model whose program is there installs after the one confirmation of its own, and is removed from its tile',
    (tester) async {
      final world = _world(there: true);
      final shown = _models(world.host);
      final type = modelsStoreType(models: [shown]);
      await tester.runAsync(type.load!);
      final tiny = type.items().first;
      expect(tiny.name, 'tiny');
      expect(tiny.installed, isFalse);
      expect(
        tiny.remove,
        isNull,
        reason: 'a model that is not installed has nothing to remove',
      );

      await _open(tester, (context) => tiny.install!(context));
      expect(find.byKey(const Key('model-confirm')), findsOneWidget);
      expect(find.text('Install tiny'), findsOneWidget);
      expect(
        find.byKey(const Key('plan-dialog')),
        findsNothing,
        reason: 'the program is there: nothing of it to confirm',
      );
      await tester.tap(find.byKey(const Key('model-confirm-yes')));
      await _host(tester);
      expect(world.host.params['store/model']!.single, {
        'type': _type,
        'model': 'tiny',
        'action': 'fetch_files',
      });
      expect(
        world.host.params['store/apply'],
        isNull,
        reason: 'the program was not installed again',
      );

      final installed = type.items().first;
      expect(installed.installed, isTrue);
      expect(
        installed.install,
        isNull,
        reason: 'there is nothing left to install',
      );
      expect(installed.remove, isNotNull);
      await _open(tester, (context) => installed.remove!(context));
      expect(find.text('Remove tiny'), findsOneWidget);
      await tester.tap(find.byKey(const Key('model-confirm-yes')));
      await _host(tester);
      expect(world.host.params['store/model']!.last['action'], 'delete_files');
      expect(type.items().first.installed, isFalse);
      shown.dispose();
    },
  );

  testWidgets(
    'a model whose program is missing offers the program first: one confirmation shows its plan with the model, then both are installed',
    (tester) async {
      final world = _world();
      final shown = _models(world.host);
      final type = modelsStoreType(models: [shown]);
      await tester.runAsync(type.load!);
      expect(shown.program?.installed, isFalse);
      expect(shown.program?.title, 'Runner');

      await _open(tester, (context) => type.items().first.install!(context));
      expect(
        find.byKey(const Key('plan-dialog')),
        findsOneWidget,
        reason: 'the program\'s plan is what the person confirms',
      );
      expect(find.text('Install Runner'), findsOneWidget);
      expect(
        tester.widget<Text>(find.byKey(const Key('plan-also'))).data,
        contains('Install tiny'),
        reason: 'the model is named in the same confirmation',
      );
      expect(find.textContaining('Downloads tiny (78 MB)'), findsOneWidget);
      expect(
        find.byKey(const Key('model-confirm')),
        findsNothing,
        reason: 'there is one confirmation, not two',
      );
      expect(world.host.params['store/apply'], isNull);
      expect(
        world.host.params['store/model'],
        isNull,
        reason: 'nothing is asked of the model before the program is there',
      );

      await tester.tap(find.byKey(const Key('plan-confirm')));
      for (
        var i = 0;
        i < 40 && find.byKey(const Key('program-done')).evaluate().isEmpty;
        i++
      ) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 50)),
        );
        await tester.pump(const Duration(milliseconds: 50));
      }
      expect(find.byKey(const Key('program-done')), findsOneWidget);
      await tester.tap(find.byKey(const Key('program-close')));
      await _host(tester);
      expect(
        world.host.params['store/apply']!.single['provider'],
        'runner_store',
      );
      expect(world.host.params['store/model']!.single, {
        'type': _type,
        'model': 'tiny',
        'action': 'fetch_files',
      });
      final calls = world.host.calls;
      expect(
        calls.indexWhere((call) => call.startsWith('store/apply')),
        lessThan(calls.indexOf('store/model')),
        reason: 'the program first',
      );
      expect(
        shown.program?.installed,
        isTrue,
        reason: 'the list was read again',
      );
      expect(type.items().first.installed, isTrue);
      shown.dispose();
    },
  );

  testWidgets(
    'declining the program\'s plan installs neither the program nor the model',
    (tester) async {
      final world = _world();
      final shown = _models(world.host);
      final type = modelsStoreType(models: [shown]);
      await tester.runAsync(type.load!);
      await _open(tester, (context) => type.items().first.install!(context));
      await tester.tap(find.byKey(const Key('plan-cancel')));
      await _host(tester);
      expect(world.host.params['store/apply'], isNull);
      expect(world.host.params['store/model'], isNull);
      expect(type.items().first.installed, isFalse);
      shown.dispose();
    },
  );

  test(
    'a library is badged by its title when the host says one, and by its id when it does not',
    () async {
      final world = _world();
      final bare = FakeStore(
        'bare_store',
        title: 'Bare',
        extensionType: 'bare',
        library: 'canoa-bare',
      );
      world.host.stores.add(bare);
      world.host.extensionTypes = [
        ...world.host.extensionTypes,
        {
          'id': 'bare',
          'title': 'Bare',
          'stores': ['bare_store'],
          'extension': 'built-in',
          'extension_title': 'Bare',
        },
      ];
      final shared = world.host.client();
      final extensions = StoreExtensions(
        control: () async => shared,
        open: () async => world.host.client(),
      );
      await extensions.ensure();
      final programs = extensions.types.first;
      await programs.load!();
      expect(
        {for (final item in programs.items()) item.name: item.catalog},
        {'Runner': 'Runner library', 'Bare': 'canoa-bare'},
      );
      extensions.dispose();
    },
  );
}
