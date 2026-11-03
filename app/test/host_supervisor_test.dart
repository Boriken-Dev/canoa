// The supervisor's decisions against a host it cannot tell from the real one: when it starts, how long it waits after an
// end, what it does with a wrong Canoa.cfg, and how it ends the host. The real binary's side is in host_test.dart.
import 'dart:async';

import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_host.dart';

const _line = '{"name":"claude","kind":"agent","url":"ws://127.0.0.1:7801","peer":"sha256:aa"}';

void main() {
  test('with no canoa-host there is nothing to run', () async {
    final supervisor = HostSupervisor(null);
    await supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: true);
    expect(supervisor.phase, HostPhase.absent);
  });

  test('nothing starts until Canoa.cfg names something to host', () async {
    final tool = FakeTool();
    final supervisor = HostSupervisor(tool);
    await supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: false);
    expect(supervisor.phase, HostPhase.idle);
    expect(tool.started, isEmpty);
  });

  test('it serves Canoa.cfg tied to its input, and says where each entry listens', () async {
    final tool = FakeTool();
    final supervisor = HostSupervisor(tool);
    await supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: true);
    expect(tool.started.single, ['serve', '--config', '/c/Canoa.cfg', '--until-stdin-closes']);
    expect(supervisor.phase, HostPhase.starting);
    tool.children.single.out.add(_line);
    expect(supervisor.phase, HostPhase.running);
    expect(supervisor.listening.single.url, 'ws://127.0.0.1:7801');
    expect(supervisor.peerId, 'sha256:aa');
    expect(supervisor.peerMethod, isNull, reason: 'a host that does not say its method');
  });

  test('an end of its own starts it again after a pause that doubles, up to a ceiling', () {
    fakeAsync((async) {
      final tool = FakeTool();
      final supervisor = HostSupervisor(
        tool,
        longestPause: const Duration(seconds: 4),
        clock: () => async.getClock(DateTime(2026)).now(),
      );
      unawaited(supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: true));
      async.flushMicrotasks();
      final pauses = <Duration>[];
      for (var end = 0; end < 4; end++) {
        tool.children.last.err.add('canoa-host: the agent could not start');
        tool.children.last.end(1);
        async.flushMicrotasks();
        expect(supervisor.phase, HostPhase.waiting);
        expect(supervisor.problem, 'the agent could not start');
        pauses.add(supervisor.retryAt!.difference(async.getClock(DateTime(2026)).now()));
        final before = tool.started.length;
        async.elapse(pauses.last - const Duration(milliseconds: 1));
        expect(tool.started, hasLength(before), reason: 'not before the pause is over');
        async.elapse(const Duration(milliseconds: 1));
        expect(tool.started, hasLength(before + 1));
      }
      expect(pauses, const [Duration(seconds: 1), Duration(seconds: 2), Duration(seconds: 4), Duration(seconds: 4)]);
    });
  });

  test('after a steady run, the next end waits only the first pause', () {
    fakeAsync((async) {
      final tool = FakeTool();
      final supervisor = HostSupervisor(tool, clock: () => async.getClock(DateTime(2026)).now());
      unawaited(supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: true));
      async.flushMicrotasks();
      for (var end = 0; end < 3; end++) {
        tool.children.last.end(1);
        async.flushMicrotasks();
        async.elapse(supervisor.retryAt!.difference(async.getClock(DateTime(2026)).now()));
      }
      expect(supervisor.restarts, 3);
      async.elapse(const Duration(minutes: 1));
      tool.children.last.end(1);
      async.flushMicrotasks();
      expect(supervisor.restarts, 1);
      expect(supervisor.retryAt!.difference(async.getClock(DateTime(2026)).now()), const Duration(seconds: 1));
    });
  });

  test('a wrong Canoa.cfg is said and not retried until asked', () {
    fakeAsync((async) {
      final tool = FakeTool();
      final supervisor = HostSupervisor(tool);
      unawaited(supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: true));
      async.flushMicrotasks();
      tool.children.single.err.add('canoa-host: Canoa.cfg line 3: unknown key');
      tool.children.single.end(2);
      async.flushMicrotasks();
      expect(supervisor.phase, HostPhase.failed);
      expect(supervisor.problem, 'Canoa.cfg line 3: unknown key');
      async.elapse(const Duration(hours: 1));
      expect(tool.started, hasLength(1));
      unawaited(supervisor.restart());
      async.flushMicrotasks();
      expect(tool.started, hasLength(2));
    });
  });

  test('stopping closes its input, kills it if it stays, and nothing starts it again', () {
    fakeAsync((async) {
      final tool = FakeTool()..leavesOnInput = false;
      final supervisor = HostSupervisor(tool);
      unawaited(supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: true));
      async.flushMicrotasks();
      final child = tool.children.single;
      var stopped = false;
      unawaited(supervisor.stop().then((_) => stopped = true));
      async.flushMicrotasks();
      expect(child.inputClosed, isTrue);
      expect(child.killed, isFalse, reason: 'it gets the time to end its agents');
      async.elapse(hostLeaving);
      expect(child.killed, isTrue);
      expect(stopped, isTrue);
      expect(supervisor.phase, HostPhase.stopped);
      async.elapse(const Duration(minutes: 5));
      expect(tool.started, hasLength(1));
    });
  });

  test('a host that ends when its input closes is not killed', () async {
    final tool = FakeTool();
    final supervisor = HostSupervisor(tool);
    await supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: true);
    await supervisor.stop();
    expect(tool.children.single.inputClosed, isTrue);
    expect(tool.children.single.killed, isFalse);
  });

  test('a changed Canoa.cfg ends the running host before the new one starts', () async {
    final tool = FakeTool();
    final supervisor = HostSupervisor(tool);
    await supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: true);
    await supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: true);
    expect(tool.children.first.inputClosed, isTrue);
    expect(tool.started, hasLength(2));
    await supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: false);
    expect(tool.children.last.inputClosed, isTrue);
    expect(supervisor.phase, HostPhase.idle);
  });

  test('keys, accept and revoke run the host, and its refusal is said in its words', () async {
    final tool = FakeTool()
      ..answer = (args) => switch (args.first) {
        'peers' => const HostOutput(
          0,
          '{"peer":"sha256:w","state":"waiting","source":"127.0.0.1:5"}\n'
              '{"peer":"sha256:a","state":"accepted","source":null}\nnot json\n',
          '',
        ),
        'accept' => const HostOutput(2, '', 'canoa-host: that key is not waiting\n'),
        _ => const HostOutput(0, 'removed\n', ''),
      };
    final supervisor = HostSupervisor(tool);
    final keys = await supervisor.keys();
    expect(keys.map((key) => (key.id, key.accepted, key.from)), [
      ('sha256:a', true, null),
      ('sha256:w', false, '127.0.0.1:5'),
    ]);
    await expectLater(
      supervisor.accept('sha256:w'),
      throwsA(isA<HostCommandFailed>().having((error) => error.message, 'message', 'that key is not waiting')),
    );
    await supervisor.revoke('sha256:a');
    expect(tool.ran, [
      ['peers', '--format', 'json'],
      ['accept', '--', 'sha256:w'],
      ['revoke', '--', 'sha256:a'],
    ]);
  });

  test('pairing shows the PIN, then who paired, and cancelling ends it', () async {
    final tool = FakeTool();
    final supervisor = HostSupervisor(tool);
    final starting = supervisor.pair();
    await Future<void>.delayed(Duration.zero);
    final child = tool.children.single;
    expect(tool.started.single, ['pair', '--until-stdin-closes']);
    child.out.add('{"pin":"123456","peer":"sha256:host","url":"ws://127.0.0.1:9"}');
    final pairing = await starting;
    expect((pairing.pin, pairing.hostId, pairing.url), ('123456', 'sha256:host', 'ws://127.0.0.1:9'));
    child.out.add('{"paired":"sha256:them"}');
    expect(await pairing.paired, 'sha256:them');

    final second = supervisor.pair();
    await Future<void>.delayed(Duration.zero);
    tool.children.last.out.add('{"pin":"654321","peer":"sha256:host","url":"ws://127.0.0.1:9"}');
    final cancelled = await second;
    cancelled.cancel();
    expect(tool.children.last.killed, isTrue);
    await expectLater(cancelled.paired, throwsA(isA<HostCommandFailed>()));
  });

  test("a host whose exit comes before its last words is reported in those words", () {
    fakeAsync((async) {
      final tool = FakeTool();
      final supervisor = HostSupervisor(tool);
      unawaited(supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: true));
      async.flushMicrotasks();
      // A real process's status can arrive before the line it wrote last on standard error.
      tool.children.last.end(2, closeErrors: false);
      async.elapse(const Duration(milliseconds: 100));
      tool.children.last.err.add('canoa-host: the configuration is malformed at line 2');
      unawaited(tool.children.last.err.close());
      async.flushMicrotasks();
      expect(supervisor.phase, HostPhase.failed);
      expect(supervisor.problem, 'the configuration is malformed at line 2');
    });
  });

  test("a host whose standard error never closes is still reported, after a bound", () {
    fakeAsync((async) {
      final tool = FakeTool();
      final supervisor = HostSupervisor(tool, errorDrain: const Duration(seconds: 2));
      unawaited(supervisor.apply(configPath: '/c/Canoa.cfg', hasEntries: true));
      async.flushMicrotasks();
      tool.children.last.end(2, closeErrors: false);
      async.elapse(const Duration(milliseconds: 1999));
      expect(supervisor.phase, isNot(HostPhase.failed));
      async.elapse(const Duration(milliseconds: 1));
      expect(supervisor.phase, HostPhase.failed);
      expect(supervisor.problem, 'canoa-host ended with status 2');
    });
  });
}
