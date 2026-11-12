// A canoa the tests cannot tell from the real one: what was started and run, and children they end by hand.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/desktop/host_tool.dart';

final class FakeChild implements HostChild {
  FakeChild({this.leavesOnInput = true});

  /// Whether closing its input ends it, as `--until-stdin-closes` does.
  final bool leavesOnInput;
  final out = StreamController<String>.broadcast(sync: true);
  final err = StreamController<String>.broadcast(sync: true);
  final _exit = Completer<int>();
  bool inputClosed = false, killed = false;

  /// Ends it with [code]. Its standard error closes with it unless [closeErrors] is false, for a test to say more first.
  void end(int code, {bool closeErrors = true}) {
    if (!_exit.isCompleted) _exit.complete(code);
    if (closeErrors && !err.isClosed) unawaited(err.close());
  }

  @override
  int get pid => 0;
  @override
  Stream<String> get lines => out.stream;
  @override
  Stream<String> get errors => err.stream;
  @override
  Future<int> get exitCode => _exit.future;
  @override
  Future<void> closeInput() async {
    inputClosed = true;
    if (leavesOnInput) end(0);
  }

  @override
  void kill() {
    killed = true;
    end(-9);
  }
}

final class FakeTool implements HostTool {
  final started = <List<String>>[];
  final children = <FakeChild>[];
  final ran = <List<String>>[];
  bool leavesOnInput = true;
  HostOutput Function(List<String>) answer = (_) => const HostOutput(0, '', '');

  @override
  String get path => 'canoa';

  @override
  Future<HostChild> start(List<String> args) async {
    started.add(args);
    final child = FakeChild(leavesOnInput: leavesOnInput);
    children.add(child);
    return child;
  }

  @override
  Future<HostOutput> run(List<String> args) async {
    ran.add(args);
    return answer(args);
  }
}

/// The host's own sources a test's app is shown, as `canoa serve --local-sources` lists them.
const defaultLocalSources = [('Claude', 'claude'), ('Codex', 'codex'), ('Antigravity', null), ('Grok', null), ('Pi', null)];

List<(String, String?)> _defaultSources() => defaultLocalSources;

/// A host that serves [sources] to this process only, and whose links are the test's: [open] gives the agent behind a
/// source, or throws.
final class LinkedTool implements HostTool {
  LinkedTool({this.sources = _defaultSources, this.missing = const {}, required this.open});

  /// The sources the host serves each time it starts, so a restart can serve more or fewer.
  final List<(String, String?)> Function() sources;

  /// The programs each source is missing, by name.
  final Map<String, List<String>> missing;
  final HostLink Function(String name) open;

  @override
  String get path => 'canoa';

  @override
  Future<HostChild> start(List<String> args) async => LinkedChild([
    for (final (name, harness) in sources())
      jsonEncode({
        'name': name, 'kind': 'agent', 'local': true, 'missing': missing[name] ?? const <String>[],
        'harness': ?harness, 'peer': 'sha256:${'0' * 64}', 'method': 'noise',
      }),
    jsonEncode({'ready': true, 'peer': 'sha256:${'0' * 64}', 'method': 'noise'}),
  ], open);

  @override
  Future<HostOutput> run(List<String> args) async => const HostOutput(0, '', '');
}

final class LinkedChild extends FakeChild implements HostLinks {
  LinkedChild(this._said, this._open);

  final List<String> _said;
  final HostLink Function(String name) _open;

  /// What the host said as it came up, for whoever listens first.
  @override
  Stream<String> get lines => Stream<String>.multi((controller) {
    _said.forEach(controller.add);
    out.stream.listen(controller.add);
  });

  @override
  HostLink openLink(String entry) => _open(entry);
}

/// A supervisor over a [LinkedTool], already told to run: its sources are listed once the app has drawn a frame.
HostSupervisor linkedHost(
  HostLink Function(String name) open, {
  Map<String, List<String>> missing = const {},
  List<(String, String?)> Function()? sources,
}) {
  final host = HostSupervisor(LinkedTool(open: open, missing: missing, sources: sources ?? _defaultSources));
  unawaited(host.apply(configPath: '/c/canoa.cfg'));
  return host;
}
