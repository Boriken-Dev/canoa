// The Programs type's data over a scripted host (test/fixtures/store_link.dart): the host lists the type apart from the agents'
// programs, a row says what is installed, installing shows the plan first and a refused plan installs nothing, and removing asks
// the host to take its own copy away.
import 'package:canoa/store/store_runtimes.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_link.dart';

FakeStore _node({List<Map<String, Object?>> copies = const [], String? latest}) =>
    FakeStore('node_store', title: 'Node.js', copies: copies, latest: latest, runtime: true);

void main() {
  test('the programs are listed apart from the agents and asked as their own kind', () async {
    final host = FakeStoreHost([claudeLike(), _node(latest: '24.2.0')]);
    final runtimes = host.programs(kind: programsTypeId);
    await runtimes.refresh();
    expect(runtimeItems(runtimes).map((each) => each.provider), ['node_store']);
    expect(host.params['store/list']!.single, {'kind': 'programs'});
    final agents = host.programs();
    await agents.refresh();
    expect(agents.statuses.map((each) => each.info.provider), ['one_store']);
    expect(host.params['store/list']!.last, isEmpty, reason: 'an agents page asks as it always did');
  });

  test('a row says what is installed, and what a plan fetches is read before anything runs', () async {
    final host = FakeStoreHost([
      _node(copies: [copyOf('24.1.0', managed: true)], latest: '24.2.0'),
    ]);
    final runtimes = host.programs(kind: programsTypeId);
    await runtimes.refresh(latest: true);
    final item = runtimeItems(runtimes).single;
    expect(item.name, 'Node.js');
    expect(item.installed, isTrue);
    expect(item.managed, isTrue);
    expect(item.updateAvailable, isTrue);
    expect(item.summary, 'Installed 24.1.0');
    var shown = 0;
    final ran = await item.install((plan) async {
      shown++;
      expect(plan.action, 'update');
      return false;
    });
    expect(ran, isFalse);
    expect(shown, 1);
    expect(host.calls.where((call) => call.startsWith('store/apply')), isEmpty, reason: 'a refused plan installs nothing');
    expect(host.calls, contains('store/plan node_store'));
  });

  test('an install that is confirmed is applied, and a remove asks the host to take its copy away', () async {
    final host = FakeStoreHost([_node(latest: '24.2.0')]);
    host.afterApply = (store, plan) => store.copies = [copyOf('24.2.0', managed: true)];
    final runtimes = host.programs(kind: programsTypeId);
    await runtimes.refresh();
    var item = runtimeItems(runtimes).single;
    expect(item.installed, isFalse);
    expect(item.summary, 'Not installed');
    expect(await item.install((plan) async => plan.action == 'install'), isTrue);
    item = runtimeItems(runtimes).single;
    expect(item.installed, isTrue);
    expect(item.version, '24.2.0');
    await item.remove();
    expect(host.params['store/remove']!.single, {'provider': 'node_store'});
    expect(host.calls.where((call) => call.startsWith('store/report')).length, greaterThan(1), reason: 'read again after the change');
  });
}
