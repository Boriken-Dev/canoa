// The store types beyond the built-in four, over a scripted host (test/fixtures/store_link.dart): Runtimes is always there, and
// each type an extension adds is asked of the host and listed from the stores the host says it is made of.
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/store/store_extension.dart' show ExtensionBody, ExtensionStoreType;
import 'package:canoa/store/store_extensions.dart';
import 'package:canoa/store/store_runtimes.dart';
import 'package:canoa/store/store_view.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_link.dart';

void main() {
  test('Runtimes is a type before the host is asked, and an extension\'s type follows it once it is', () async {
    final host = FakeStoreHost([
      FakeStore('node_store', title: 'Node.js', runtime: true, latest: '24.2.0'),
      FakeStore('echo_archive_store', title: 'Echo archive', extensionType: 'echoes', copies: [copyOf('1.0.0')]),
    ])..extensionTypes = [
        {'id': 'echoes', 'title': 'Echoes', 'icon': 'palette', 'stores': ['echo_archive_store'], 'extension': 'sample', 'extension_title': 'Sample'},
        {'id': 'runtimes', 'title': 'Impostor', 'stores': <String>[]},
      ];
    final shared = host.client();
    final extensions = StoreExtensions(control: () async => shared, open: () async => host.client());
    expect(extensions.types.map((type) => type.id), [runtimesTypeId]);
    var heard = 0;
    extensions.addListener(() => heard++);

    await extensions.ensure();
    expect(extensions.types.map((type) => type.id), [runtimesTypeId, 'echoes'], reason: 'a type may not take a built-in one\'s id');
    expect(extensions.types.map((type) => type.label), [runtimesTypeLabel, 'Echoes']);
    expect(heard, 1);

    final echoes = extensions.types.last;
    await echoes.load!();
    final item = echoes.items().single;
    expect(item.name, 'Echo archive');
    expect(item.catalog, 'Sample');
    expect(item.installed, isTrue);
    expect(host.params['store/list']!.any((given) => given['kind'] == 'extension' && given['type'] == 'echoes'), isTrue);

    final runtimes = extensions.types.first;
    await runtimes.load!();
    expect(runtimes.items().map((each) => each.name), ['Node.js']);
    expect(runtimes.items().single.installed, isFalse);
    expect(runtimes.items().single.summary, 'Not installed');
    extensions.dispose();
  });

  test('a host that cannot list extensions leaves Runtimes alone and is asked again next time', () async {
    final host = FakeStoreHost([])..failures['store/extensions'] = ControlFailed(-32601, 'no such method');
    final shared = host.client();
    final extensions = StoreExtensions(control: () async => shared, open: () async => host.client());
    await extensions.ensure();
    expect(extensions.types.map((type) => type.id), [runtimesTypeId]);
    host.failures.clear();
    host.extensionTypes = [
      {'id': 'echoes', 'title': 'Echoes', 'stores': <String>[]},
    ];
    await extensions.ensure();
    expect(extensions.types.map((type) => type.id), [runtimesTypeId, 'echoes']);
    extensions.dispose();
  });

  test('a type\'s icon is one of the app\'s names, or a small picture the extension ships', () async {
    const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGD4DwABBAEAwS2OUAAAAABJRU5ErkJggg==';
    Map<String, Object?> type(String id, String icon, String by) =>
        {'id': id, 'title': id, 'icon': icon, 'stores': <String>[], 'extension': by, 'extension_title': by};
    final host = FakeStoreHost([])
      ..extensionTypes = [
        type('named', 'science', 'a'),
        type('unknown', 'nothing-like-it', 'a'),
        type('pictured', 'data:image/png;base64,$png', 'b'),
        type('webp', 'data:image/webp;base64,UklGRg==', 'b'),
        type('broken', 'data:image/png;base64,@@@', 'b'),
        type('svg', 'data:image/svg+xml;base64,PHN2Zz4=', 'b'),
      ];
    final shared = host.client();
    final extensions = StoreExtensions(control: () async => shared, open: () async => host.client());
    await extensions.ensure();
    ExtensionStoreType added(String id) => extensions.types.firstWhere((each) => each.id == id);
    expect(added('named').icon, Icons.science_outlined);
    expect(added('named').image, isNull);
    expect(added('unknown').icon, Icons.extension_outlined, reason: 'a name the app does not know draws the default');
    expect(added('pictured').image!.path, 'icon.png');
    expect(added('pictured').image!.bytes.take(4), [0x89, 0x50, 0x4e, 0x47]);
    expect(added('webp').image!.path, 'icon.webp');
    expect(added('broken').image, isNull, reason: 'not base64: the default icon');
    expect(added('svg').image, isNull, reason: 'only PNG and WebP are drawn');
    expect(extensions.addedBy, {'a': ['named', 'unknown'], 'b': ['pictured', 'webp', 'broken', 'svg']});
    extensions.dispose();
  });

  testWidgets('an installed item of an extension\'s type has Remove, which shows the host\'s plan and removes once it is confirmed', (tester) async {
    final host = FakeStoreHost([
      FakeStore('echo_archive_store', title: 'Echo archive', extensionType: 'echoes', copies: [copyOf('1.0.0')]),
      FakeStore('plain_store', title: 'Found only', extensionType: 'echoes', copies: [copyOf('2.0.0', managed: false)]),
      FakeStore('absent_store', title: 'Absent', extensionType: 'echoes'),
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
    final view = StoreView('echoes');
    await tester.binding.setSurfaceSize(const Size(900, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: ExtensionBody(type: extensions.types.last, view: view))));
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
}

/// Lets the scripted host answer and the page draw it: a few short frames, since the banner and a running install never settle.
Future<void> _settle(WidgetTester tester) async {
  for (var i = 0; i < 20; i++) {
    await tester.pump(const Duration(milliseconds: 50));
  }
}
