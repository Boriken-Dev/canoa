// The store types beyond the built-in four, over a scripted host (test/fixtures/store_link.dart): Programs is always there and lists
// every program the host does, Models lists what the libraries that list models do, and Extensions the catalogs'.
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/store/store_extension.dart' show ExtensionBody;
import 'package:canoa/store/store_extension_catalog.dart';
import 'package:canoa/store/store_extensions.dart';
import 'package:canoa/store/store_models.dart';
import 'package:canoa/store/store_runtimes.dart';
import 'package:canoa/store/store_view.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_link.dart';

void main() {
  test('Programs is a type before the host is asked, and lists every program the host does, runtimes and a library\'s alike', () async {
    final host = FakeStoreHost([
      FakeStore('node_store', title: 'Node.js', runtime: true, latest: '24.2.0', library: 'canoa-node'),
      FakeStore('echo_archive_store', title: 'Echo archive', extensionType: 'echoes', library: 'sample', copies: [copyOf('1.0.0')]),
      FakeStore('claude_like_store', title: 'An agent'),
    ])..extensionTypes = [
        {'id': 'echoes', 'title': 'Echoes', 'icon': 'palette', 'stores': ['echo_archive_store'], 'extension': 'sample', 'extension_title': 'Sample'},
        {'id': 'programs', 'title': 'Impostor', 'stores': <String>[]},
      ];
    final shared = host.client();
    final extensions = StoreExtensions(control: () async => shared, open: () async => host.client());
    expect(extensions.types.map((type) => type.id), [programsTypeId, extensionsTypeId], reason: 'no type of a library is listed before the host says so');
    var heard = 0;
    extensions.addListener(() => heard++);

    await extensions.ensure();
    expect(extensions.types.map((type) => type.id), [programsTypeId, extensionsTypeId], reason: 'a type is no page of its own, and may not take a built-in one\'s id');
    expect(extensions.types.map((type) => type.label), [programsTypeLabel, extensionsTypeLabel]);
    expect(heard, 1);

    final programs = extensions.types.first;
    await programs.load!();
    expect(programs.items().map((each) => each.name), ['Node.js', 'Echo archive'], reason: 'one list: the runtimes, then each library\'s programs, no agent');
    expect(programs.items().map((each) => each.catalog), ['canoa-node', 'sample'], reason: 'each row is grouped by the library that offers it');
    expect(programs.items().map((each) => each.installed), [false, true]);
    expect(programs.items().first.summary, 'Not installed');
    expect(host.params['store/list']!.where((given) => given['kind'] == 'programs'), isNotEmpty);
    expect(host.params['store/list']!.any((given) => given['kind'] == 'extension' || given['kind'] == 'runtimes'), isFalse);
    extensions.dispose();
  });

  test('a host that cannot list extensions leaves Programs alone and is asked again next time', () async {
    final host = FakeStoreHost([])..failures['store/extensions'] = ControlFailed(-32601, 'no such method');
    final shared = host.client();
    final extensions = StoreExtensions(control: () async => shared, open: () async => host.client());
    await extensions.ensure();
    expect(extensions.types.map((type) => type.id), [programsTypeId, extensionsTypeId]);
    host.failures.clear();
    host.extensionTypes = [
      {'id': 'echoes', 'title': 'Echoes', 'stores': <String>[], 'models': {'provider': 'echo_models', 'title': 'Echo models'}},
    ];
    await extensions.ensure();
    expect(extensions.types.map((type) => type.id), [programsTypeId, modelsTypeId, extensionsTypeId]);
    extensions.dispose();
  });

  test('the models of every type are one page after Programs, and a type that lists none adds none', () async {
    final host = FakeStoreHost([FakeStore('runner_store', title: 'Runner', extensionType: 'runner', library: 'canoa-runner', copies: [copyOf('0.14.0')])])
      ..extensionTypes = [
        {
          'id': 'runner',
          'title': 'Runner',
          'icon': 'memory',
          'stores': ['runner_store'],
          'extension': 'built-in',
          'extension_title': 'Runner',
          'models': {'provider': 'runner_models', 'title': 'Runner models'},
        },
        {
          'id': 'speaker',
          'title': 'Speaker',
          'stores': <String>[],
          'extension': 'built-in',
          'extension_title': 'Speaker',
          'models': {'provider': 'speaker_voices', 'title': 'Speaker voices'},
        },
        {'id': 'plain', 'title': 'Plain', 'stores': <String>[], 'models': null},
      ];
    final shared = host.client();
    final extensions = StoreExtensions(control: () async => shared, open: () async => host.client());
    await extensions.ensure();
    expect(extensions.types.map((type) => type.id), [programsTypeId, modelsTypeId, extensionsTypeId]);
    expect(extensions.types.map((type) => type.label), ['Programs', 'Models', 'Extensions']);
    final programs = extensions.types.first;
    await programs.load!();
    expect(programs.items().single.name, 'Runner');
    expect(programs.items().single.catalog, 'canoa-runner');
    final models = extensions.types[1];
    await models.load!();
    expect(host.params['store/models']!.map((given) => given['type']).toSet(), {'runner', 'speaker'}, reason: 'each type that lists models is asked, and no other');
    extensions.dispose();

    final without = FakeStoreHost([])..extensionTypes = [];
    final sharedWithout = without.client();
    final bare = StoreExtensions(control: () async => sharedWithout, open: () async => without.client());
    await bare.ensure();
    expect(bare.types.map((type) => type.id), [programsTypeId, extensionsTypeId]);
    bare.dispose();
  });

  test('the titles of the types each extension adds are kept by the extension, though a type is no page of its own', () async {
    Map<String, Object?> type(String id, String by) => {'id': id, 'title': id, 'stores': <String>[], 'extension': by, 'extension_title': by};
    final host = FakeStoreHost([])..extensionTypes = [type('named', 'a'), type('unknown', 'a'), type('pictured', 'b')];
    final shared = host.client();
    final extensions = StoreExtensions(control: () async => shared, open: () async => host.client());
    await extensions.ensure();
    expect(extensions.addedBy, {'a': ['named', 'unknown'], 'b': ['pictured']});
    extensions.dispose();
  });

  testWidgets('an installed item of a library\'s program has Remove, which shows the host\'s plan and removes once it is confirmed', (tester) async {
    final host = FakeStoreHost([
      FakeStore('echo_archive_store', title: 'Echo archive', extensionType: 'echoes', library: 'Sample', copies: [copyOf('1.0.0')]),
      FakeStore('plain_store', title: 'Found only', extensionType: 'echoes', library: 'Sample', copies: [copyOf('2.0.0', managed: false)]),
      FakeStore('absent_store', title: 'Absent', extensionType: 'echoes', library: 'Sample'),
    ])
      ..extensionTypes = [
        {'id': 'echoes', 'title': 'Echoes', 'stores': ['echo_archive_store', 'plain_store', 'absent_store'], 'extension': 'sample', 'extension_title': 'Sample'},
      ];
    host.afterApply = (store, plan) {
      if (plan['action'] == 'uninstall') store.copies = [];
    };
    final shared = host.client();
    final extensions = StoreExtensions(control: () async => shared, open: () async => host.client());
    await extensions.ensure();
    final view = StoreView(programsTypeId);
    await tester.binding.setSurfaceSize(const Size(900, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: ExtensionBody(type: extensions.types.first, view: view))));
    await _settle(tester);

    expect(find.byKey(const Key('store-entry-remove-Sample-Echo archive')), findsOneWidget, reason: 'the host holds its own copy');
    expect(find.byKey(const Key('store-entry-remove-Sample-Found only')), findsNothing, reason: 'a program found on this computer is not Canoa\'s to remove');
    expect(find.byKey(const Key('store-entry-remove-Sample-Absent')), findsNothing, reason: 'nothing installed');

    await tester.tap(find.byKey(const Key('store-entry-remove-Sample-Echo archive')));
    await _settle(tester);
    expect(find.byKey(const Key('plan-dialog')), findsOneWidget);
    expect(host.params['store/plan']!.last, containsPair('action', 'uninstall'));
    expect(host.calls.where((call) => call.startsWith('store/apply')), isEmpty, reason: 'nothing runs before the person confirms');
    await tester.tap(find.byKey(const Key('plan-confirm')));
    for (var i = 0; i < 40 && find.byKey(const Key('program-done')).evaluate().isEmpty; i++) {
      await tester.pump(const Duration(milliseconds: 50));
    }
    expect(find.byKey(const Key('program-done')), findsOneWidget);
    expect(host.calls.where((call) => call == 'store/apply echo_archive_store'), hasLength(1));
    await tester.tap(find.byKey(const Key('program-close')));
    await _settle(tester);
    extensions.dispose();
  });
  testWidgets('a program a type claims is followed by that type\'s setup once it is installed', (tester) async {
    final host = FakeStoreHost([
      FakeStore('bitwarden_store', title: 'Bitwarden', extensionType: 'secrets', library: 'Canoa'),
    ])
      ..extensionTypes = [
        {'id': 'secrets', 'title': 'Secret providers', 'icon': 'lock', 'stores': ['bitwarden_store'], 'extension': 'built-in', 'extension_title': 'Canoa'},
      ];
    final shared = host.client();
    final setUp = <String>[];
    final extensions = StoreExtensions(
      control: () async => shared,
      open: () async => host.client(),
      afterInstall: (context, type) async => setUp.add(type),
    );
    await extensions.ensure();
    final view = StoreView(programsTypeId);
    await tester.binding.setSurfaceSize(const Size(900, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: ExtensionBody(type: extensions.types.first, view: view))));
    await _settle(tester);

    await tester.tap(find.byKey(const Key('store-entry-install-Canoa-Bitwarden')));
    await _settle(tester);
    expect(find.byKey(const Key('plan-dialog')), findsOneWidget);
    expect(setUp, isEmpty, reason: 'nothing is set up before the program is installed');
    await tester.tap(find.byKey(const Key('plan-confirm')));
    for (var i = 0; i < 40 && find.byKey(const Key('program-done')).evaluate().isEmpty; i++) {
      await tester.pump(const Duration(milliseconds: 50));
    }
    expect(find.byKey(const Key('program-done')), findsOneWidget);
    await tester.tap(find.byKey(const Key('program-close')));
    await _settle(tester);
    expect(setUp, ['secrets']);
    extensions.dispose();
  });

}

/// Lets the scripted host answer and the page draw it: a few short frames, since the banner and a running install never settle.
Future<void> _settle(WidgetTester tester) async {
  for (var i = 0; i < 20; i++) {
    await tester.pump(const Duration(milliseconds: 50));
  }
}
