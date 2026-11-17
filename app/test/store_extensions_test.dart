// The store types beyond the built-in four, over a scripted host (test/fixtures/store_link.dart): Runtimes is always there, and
// each type an extension adds is asked of the host and listed from the stores the host says it is made of.
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/store/store_extensions.dart';
import 'package:canoa/store/store_runtimes.dart';
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
}
