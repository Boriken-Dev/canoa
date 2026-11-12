// The app's Store code against the real host's @control: `store_control`, an example of canoa-host, serves the control channel
// with a stand-in program (a made-up one, installed from a fake publisher into a temporary folder) as its only Store, over
// standard streams. Plan, apply, report, update, roll back and remove are the host's own; nothing of this computer's is read or
// written. Build it first: `cargo build -p canoa-host --bin canoa --examples`. Skipped without it.
@Timeout(Duration(minutes: 2))
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/sources/readiness.dart';
import 'package:canoa/sources/sources.dart';
import 'package:canoa/store/programs.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/examples.dart';

/// The example's standard streams, shared by the links it serves: a line is a link's number, a space and a message.
final class _Process {
  _Process(this.process) {
    process.stderr.listen((_) {});
    process.stdout.transform(utf8.decoder).transform(const LineSplitter()).listen((line) {
      final at = line.indexOf(' ');
      if (at > 0) _inbox[line.substring(0, at)]?.add(line.substring(at + 1));
    });
  }

  final Process process;
  final _inbox = <String, StreamController<String>>{};
  int _next = 0;
  Future<void> _written = Future.value();

  /// One line to the example, after the ones before: a sink takes one flush at a time.
  Future<void> write(String line) => _written = _written.then((_) async {
    process.stdin.writeln(line);
    await process.stdin.flush();
  });

  /// A new link to `@control`.
  HostLink open() {
    final name = '${++_next}';
    _inbox[name] = StreamController<String>.broadcast();
    return _ProcessLink(this, name);
  }
}

final class _ProcessLink implements HostLink {
  _ProcessLink(this._shared, this._name);

  final _Process _shared;
  final String _name;

  @override
  Stream<String> get messages => _shared._inbox[_name]!.stream;

  @override
  Future<void> send(String message) async {
    await _shared.write('$_name $message');
  }

  @override
  Future<void> close() async {
    try {
      await _shared.write('$_name close');
    } on Object {
      // The example is gone already.
    }
    await _shared._inbox[_name]?.close();
  }
}

const _echo = AgentSource.hosted('Echo', harness: 'echo', local: true);

void main() {
  final skip = exampleMissing('store_control') ?? exampleMissing('version_standin') ?? exampleMissing('stdio_agent');

  group('the real host', skip: skip, () {
    late Directory dir;
    late _Process example;
    late ControlClient client;
    late ProgramStores programs;
    final links = <ControlClient>[];

    setUp(() async {
      dir = await Directory.systemTemp.createTemp('canoa-store-host');
      final process = await Process.start(exampleBinary('store_control')!, [dir.path]);
      example = _Process(process);
      client = ControlClient(example.open(), timeout: const Duration(seconds: 60));
      links.add(client);
      programs = ProgramStores(
        control: () async => client,
        open: () async => ControlClient(example.open(), timeout: const Duration(seconds: 60)),
      );
    });
    tearDown(() async {
      for (final each in links) {
        await each.close();
      }
      links.clear();
      await example.process.stdin.close();
      await example.process.exitCode.timeout(const Duration(seconds: 20), onTimeout: () {
        example.process.kill();
        return -1;
      });
      try {
        await dir.delete(recursive: true);
      } on FileSystemException {
        // Windows may hold a file a moment longer.
      }
    });

    test('plan, apply with streamed progress, then the report shows the program, then update, roll back and remove', () async {
      await programs.refresh(latest: true);
      var status = programs.status('echo_store')!;
      expect(status.info.title, isNotEmpty);
      expect(status.installed, isFalse);
      expect(status.latest?.known, isTrue);
      expect(status.latest?.version, '2.0.0');

      // Nothing was asked to run, and a readiness check says the same.
      final ready = await storeReadiness(programs, _echo);
      expect(ready!.ready, isFalse);

      final plan = await programs.plan('echo_store', 'install', version: '1.0.0');
      expect(plan.action, 'install');
      expect(plan.version, '1.0.0');
      expect(plan.raw['token'], isA<String>());

      final run = programs.apply(plan);
      final result = await run.done;
      expect(result['applied'], isTrue);
      expect(result['version'], plan.version);
      expect(run.lines, isNotEmpty, reason: 'the host streamed what it did');

      status = programs.status('echo_store')!;
      expect(status.installed, isTrue, reason: 'readiness is read again after an apply');
      expect(status.copy!.managed, isTrue);
      expect(status.version, plan.version);
      expect((await storeReadiness(programs, _echo))!.ready, isTrue);

      // The plan was used up: the host refuses it a second time, in its own words.
      await expectLater(
        programs.apply(plan).done,
        throwsA(isA<ControlFailed>().having((error) => error.code, 'code', -32002)),
      );

      await programs.refresh(latest: true, force: true);
      status = programs.status('echo_store')!;
      expect(status.updateAvailable, isTrue, reason: '${status.version} against ${status.latest?.version}');
      final update = await programs.plan('echo_store', 'update');
      expect(update.version, '2.0.0');
      expect(update.fromVersion, plan.version);
      expect((await programs.apply(update).done)['version'], '2.0.0');
      status = programs.status('echo_store')!;
      expect(status.version, '2.0.0');
      expect(status.updateAvailable, isFalse);

      expect(await programs.rollback('echo_store'), plan.version);
      expect(programs.status('echo_store')!.version, plan.version);

      final remove = await programs.plan('echo_store', 'uninstall');
      await programs.apply(remove).done;
      expect(programs.status('echo_store')!.installed, isFalse);
    });

    test("closing an install's link ends it, and the next install on the same folder is not held", () async {
      final plan = await programs.plan('echo_store', 'install');
      final run = programs.apply(plan);
      await run.cancel();
      await expectLater(run.done, throwsA(isA<ProgramCancelled>()));

      // The host ends the install where it is: the program is whole or absent, and the folder is free for a new plan. A host
      // still stopping answers held for a moment.
      Object? held;
      for (var tries = 0; tries < 50; tries++) {
        await programs.refresh();
        final status = programs.status('echo_store')!;
        if (status.problem == null) break;
        held = status.problem;
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      final status = programs.status('echo_store')!;
      expect(status.problem, isNull, reason: 'still refused: $held');
      if (!status.installed) {
        final again = await programs.plan('echo_store', 'install');
        expect((await programs.apply(again).done)['applied'], isTrue);
      }
      expect(programs.status('echo_store')!.installed, isTrue);
      expect(programs.status('echo_store')!.copy!.managed, isTrue);
    });

    test('a plan the app made up, and a store the host does not have, are refused in the host\'s words', () async {
      final plan = await programs.plan('echo_store', 'install');
      final forged = ProgramPlan(provider: 'echo_store', action: 'install', raw: {...plan.raw, 'token': 'not-a-token'});
      await expectLater(
        programs.apply(forged).done,
        throwsA(isA<ControlFailed>().having((error) => error.code, 'code', -32002)),
      );
      await expectLater(
        programs.plan('no_such_store', 'install'),
        throwsA(isA<ControlFailed>().having((error) => error.code, 'code', -32003)),
      );
      expect(programs.status('echo_store')!.installed, isFalse, reason: 'nothing was installed');
    });
  });
}
