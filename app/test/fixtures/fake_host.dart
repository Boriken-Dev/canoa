// A canoa-host the tests cannot tell from the real one: what was started and run, and children they end by hand.
import 'dart:async';

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
  String get path => 'canoa-host';

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
