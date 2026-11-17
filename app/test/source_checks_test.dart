// Source lookups run side by side: a ready source is usable while another is still being looked for.
import 'dart:async';

import 'package:canoa/sources/source_checks.dart';
import 'package:canoa/sources/sources.dart';
import 'package:flutter_test/flutter_test.dart';

const _ready = AgentSource.hosted('ready');
const _slow = AgentSource.hosted('slow');

void main() {
  test('a refresh that leaves no source says so, though nothing is looked up', () async {
    var changes = 0;
    final checks = SourceChecks((source) => Future.value(const SourceCheck()), onChange: () => changes++);
    await checks.refresh([_ready]);
    final before = changes;
    await checks.refresh(const []);
    expect(checks.results, isEmpty);
    expect(changes, greaterThan(before), reason: 'whoever draws the sources is told the last one is gone');
  });

  test('a ready source is usable while an unrelated source is still being looked for', () async {
    final slow = Completer<SourceCheck>();
    final checks = SourceChecks((source) => source == _slow ? slow.future : Future.value(const SourceCheck()));
    final refreshed = checks.refresh([_ready, _slow]);
    await pumpEventQueue();
    expect(checks.results[_ready]?.ready, isTrue);
    expect(checks.results.containsKey(_slow), isFalse);
    slow.complete(const SourceCheck(missing: ['slow']));
    await refreshed;
    expect(checks.results[_slow]?.ready, isFalse);
  });

  test('a lookup already running for a source is joined, not repeated', () async {
    final slow = Completer<SourceCheck>();
    var asked = 0;
    final checks = SourceChecks((source) {
      asked++;
      return slow.future;
    });
    final refreshed = checks.refresh([_slow]);
    final started = checks.check(_slow);
    slow.complete(const SourceCheck());
    await Future.wait([refreshed, started]);
    expect(asked, 1);
  });

  test('an answer to a refresh that a later one replaced does not overwrite the new configuration', () async {
    final first = Completer<SourceCheck>();
    var calls = 0;
    final checks = SourceChecks((source) => ++calls == 1 ? first.future : Future.value(const SourceCheck(missing: ['now'])));
    final old = checks.refresh([_slow]);
    final latest = checks.refresh([_slow]);
    await latest;
    expect(checks.results[_slow]?.missing, ['now']);
    first.complete(const SourceCheck());
    await old;
    expect(checks.results[_slow]?.missing, ['now']);
  });

  test('a source dropped by a refresh keeps no answer, and one that never answers shows as a problem', () async {
    final checks = SourceChecks((source) => source == _slow ? Completer<SourceCheck>().future : Future.value(const SourceCheck()),
        patience: const Duration(milliseconds: 20));
    await checks.refresh([_ready, _slow]);
    expect(checks.results[_slow]?.problem, contains('did not answer'));
    await checks.refresh([_ready]);
    expect(checks.results.keys, [_ready]);
  });
}
