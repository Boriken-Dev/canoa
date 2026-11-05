// The host as a library in this process, through the app's own code. CANOA_TEST_LIBRARY names a built canoa_ffi library;
// CANOA_TEST_HOST a built canoa, which stands for what runs the host's own commands, since this test's process is not the app
// and has no entry for them. Without CANOA_TEST_LIBRARY these tests are skipped. It also holds the C ABI's one table of
// statuses to the Rust one, which needs nothing built.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/desktop/child_flag.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:canoa_native/canoa_native.dart';
import 'package:flutter_test/flutter_test.dart';

final String? _library = switch (Platform.environment['CANOA_TEST_LIBRARY']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};
final String? _host = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};

Future<void> _until(bool Function() done, String what) async {
  final deadline = DateTime.now().add(const Duration(seconds: 30));
  while (!done()) {
    if (DateTime.now().isAfter(deadline)) fail('timed out waiting for $what');
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
}

void main() {
  test('the statuses, the ABI version and the child flag are the ones the library names', () {
    final exports = File('../crates/canoa-ffi/src/exports.rs').readAsStringSync();
    int constant(String name) => int.parse(RegExp('pub const $name: [iu]32 = (\\d+);').firstMatch(exports)!.group(1)!);
    expect(constant('STATUS_OK'), statusOk);
    expect(constant('STATUS_BAD_ARGUMENT'), statusBadArgument);
    expect(constant('STATUS_UNKNOWN_JOB'), statusUnknownJob);
    expect(constant('STATUS_FAILED'), statusFailed);
    expect(constant('STATUS_PANIC'), statusPanic);
    expect(constant('ABI_VERSION'), canoaAbiVersion);
    final jobs = File('../crates/canoa-ffi/src/jobs.rs').readAsStringSync();
    expect(jobs, contains('pub const CHILD_FLAG: &str = "$canoaChildFlag";'));
    for (final entry in ['native/windows/child_entry.cpp', 'native/linux/child_entry.cc']) {
      expect(File(entry).readAsStringSync(), contains('"$canoaChildFlag"'), reason: entry);
    }
  });

  final skip = _library == null ? 'CANOA_TEST_LIBRARY names a built canoa_ffi library' : null;

  group('in this process', skip: skip, () {
    late Directory dir;
    late String cfg;
    late LibraryHostTool tool;

    setUp(() async {
      dir = await Directory.systemTemp.createTemp('canoa-library');
      cfg = '${dir.path}${Platform.pathSeparator}canoa.cfg';
      await File(cfg).writeAsString(const CanoaConfig().encode());
      tool = LibraryHostTool(CanoaNative.open(path: _library), home: '${dir.path}/home', program: _host);
    });
    tearDown(() async {
      try {
        await dir.delete(recursive: true);
      } on FileSystemException {
        // Windows may still hold a file a moment after a stop.
      }
    });

    test('with nothing to host it runs, says who it is, and stops', () async {
      final supervisor = HostSupervisor(tool);
      await supervisor.apply(configPath: cfg);
      await _until(() => supervisor.phase == HostPhase.idle, 'the host to be up');
      expect(supervisor.peerId, isNotNull);
      expect(supervisor.pid, pid, reason: 'it runs in this process');
      await supervisor.stop();
      expect(supervisor.phase, HostPhase.stopped);
    });

    test('a second on the same home says who holds it and waits for a person', () async {
      final first = HostSupervisor(tool);
      await first.apply(configPath: cfg);
      await _until(() => first.phase == HostPhase.idle, 'the first to be up');
      final second = HostSupervisor(tool);
      await second.apply(configPath: cfg);
      await _until(() => second.phase == HostPhase.held, 'the second to refuse');
      expect(second.problem, contains('CanoaUI'));
      expect(second.problem, contains('$pid'));
      expect(first.phase, HostPhase.idle, reason: 'the first goes on');
      await first.stop();
      await second.restart();
      await _until(() => second.phase == HostPhase.idle, 'the second to start once told');
      await second.stop();
    });

    test('a standalone canoa serve and this host never fight over a home, whichever came first', () async {
      final supervisor = HostSupervisor(tool);
      await supervisor.apply(configPath: cfg);
      await _until(() => supervisor.phase == HostPhase.idle, 'this host to be up');
      final environment = {'COQUI_AGENTIC_PEER_HOME': '${dir.path}/home'};
      final refused = await Process.run(_host!, ['serve', '--allow-empty', '--config', cfg], environment: environment);
      expect(refused.exitCode, statusHeld, reason: '${refused.stderr}');
      expect(refused.stderr as String, contains('CanoaUI (process $pid)'));
      expect(refused.stdout as String, isEmpty, reason: 'it serves nothing');
      await supervisor.stop();

      final standalone = await Process.start(_host!, [
        'serve',
        '--allow-empty',
        '--until-stdin-closes',
        '--config',
        cfg,
      ], environment: environment);
      final said = standalone.stdout.transform(utf8.decoder).transform(const LineSplitter()).asBroadcastStream();
      standalone.stderr.listen((_) {});
      await said.firstWhere((line) => line.contains('"ready"')).timeout(const Duration(seconds: 30));
      await supervisor.restart();
      await _until(() => supervisor.phase == HostPhase.held, 'this host to refuse');
      expect(supervisor.problem, contains('canoa (process ${standalone.pid})'));

      // A holder that dies gives the home up, however it died.
      standalone.kill(ProcessSignal.sigkill);
      await standalone.exitCode;
      await supervisor.restart();
      await _until(() => supervisor.phase == HostPhase.idle, 'this host to take the home over');
      await supervisor.stop();
    }, skip: _host == null ? 'CANOA_TEST_HOST names a built canoa' : null);

    test('its other commands run too: keys, check, and pairing given up', () async {
      final supervisor = HostSupervisor(tool);
      expect(await supervisor.keys(), isEmpty);
      expect(await supervisor.check(cfg), isNull);
      await File(cfg).writeAsString('[host.agents.x]\nprogram = 1\n');
      expect(await supervisor.check(cfg), isNotNull);
      await expectLater(supervisor.accept('not an id'), throwsA(isA<HostCommandFailed>()));
      final pairing = await supervisor.pair();
      expect(pairing.pin, hasLength(6));
      pairing.cancel();
      await expectLater(pairing.paired, throwsA(isA<HostCommandFailed>()));
    });

    test('a wrong canoa.cfg is a failed start, in its words', () async {
      await File(cfg).writeAsString('[host.agents.x]\nprogram = 1\n');
      final supervisor = HostSupervisor(tool);
      await supervisor.apply(configPath: cfg);
      await _until(() => supervisor.phase == HostPhase.failed, 'the host to refuse it');
      expect(supervisor.problem, isNotNull);
      unawaited(supervisor.stop());
    });
  });

  test('a library that cannot be loaded is no host, and that is said', () {
    final said = <String>[];
    final tool = findHostToolIn(
      environment: const {},
      open: () => CanoaNative.open(path: 'no-such-library-anywhere'),
      log: said.add,
    );
    expect(tool, isNull);
    expect(said.single, contains('host library'));
  });

  test('the line that says a host is up is no entry, and a held home has the status canoa documents', () {
    expect(HostListening.parse('{"ready":true,"peer":"p","method":"noise"}'), isNull);
    final cli = File('../crates/canoa-host/src/cli.rs').readAsStringSync();
    expect(cli, contains('pub const STATUS_HELD: u8 = $statusHeld;'));
  });
}
