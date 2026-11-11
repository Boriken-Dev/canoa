// The host library running in the test's own process, as in the app, with its own sources served: what a test uses to reach a
// real agent (or a stand-in on PATH) the way the app does, over a link. Needs CANOA_TEST_LIBRARY, a built canoa_ffi library,
// and CANOA_TEST_HOST, a built canoa, which stands for what runs the host's own commands since a test process is not the app.
import 'dart:io';

import 'package:canoa/acp/client.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool_io.dart';
import 'package:canoa_native/canoa_native.dart';
import 'package:flutter_test/flutter_test.dart';

/// Why a test using [EmbeddedHost] cannot run here, or null.
String? embeddedHostMissing() {
  if ((Platform.environment['CANOA_TEST_LIBRARY'] ?? '').isEmpty) return 'CANOA_TEST_LIBRARY names a built canoa_ffi library';
  if ((Platform.environment['CANOA_TEST_HOST'] ?? '').isEmpty) return 'CANOA_TEST_HOST names a built canoa';
  return null;
}

final class EmbeddedHost {
  EmbeddedHost._(this.supervisor, this.dir);

  final HostSupervisor supervisor;
  final Directory dir;

  /// Starts the host on a new home in [dir] (a temporary one by default) with [config] as canoa.cfg, which can name entries
  /// that stand in for one of the host's own sources.
  static Future<EmbeddedHost> start({String config = '', Directory? dir}) async {
    final problem = embeddedHostMissing();
    if (problem != null) fail(problem);
    final home = dir ?? await Directory.systemTemp.createTemp('canoa-embedded-host');
    final cfg = '${home.path}${Platform.pathSeparator}canoa.cfg';
    await File(cfg).writeAsString(config);
    final tool = LibraryHostTool(
      CanoaNative.open(path: File(Platform.environment['CANOA_TEST_LIBRARY']!).absolute.path),
      home: '${home.path}${Platform.pathSeparator}home',
      program: File(Platform.environment['CANOA_TEST_HOST']!).absolute.path,
    );
    final supervisor = HostSupervisor(tool);
    await supervisor.apply(configPath: cfg);
    final deadline = DateTime.now().add(const Duration(seconds: 30));
    while (supervisor.phase != HostPhase.running) {
      if (DateTime.now().isAfter(deadline)) fail('the host did not run: ${supervisor.problem} ${supervisor.log}');
      await Future<void>.delayed(const Duration(milliseconds: 20));
    }
    return EmbeddedHost._(supervisor, home);
  }

  /// A connected agent of the host's source [name], ready to open a session.
  Future<VerifiedAgent> agent(String name, {AgentDecider? decider, AgentElicitor? elicitor}) async {
    final link = supervisor.openLink(name);
    final client = VerifiedAgent.fromChild(
      label: name, messages: link.messages, send: link.send, close: link.close, decider: decider, elicitor: elicitor,
    );
    try {
      await client.initialize();
      return client;
    } on Object {
      await client.close();
      rethrow;
    }
  }

  Future<void> stop() async {
    await supervisor.stop();
    try {
      await dir.delete(recursive: true);
    } on FileSystemException {
      // Windows may hold a file a moment longer.
    }
  }
}
