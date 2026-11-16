import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/desktop/host_tool.dart';

/// The example's standard streams, shared by the links it serves: a line is a link's number, a space and a message.
final class ControlProcess {
  ControlProcess(this.process) {
    process.stderr.listen((_) {});
    process.stdout.transform(utf8.decoder).transform(const LineSplitter()).listen((line) {
      final at = line.indexOf(' ');
      if (at > 0) inbox[line.substring(0, at)]?.add(line.substring(at + 1));
    });
  }

  final Process process;
  final inbox = <String, StreamController<String>>{};
  int nextLink = 0;
  Future<void> written = Future.value();

  /// One line to the example, after the ones before: a sink takes one flush at a time.
  Future<void> write(String line) => written = written.then((_) async {
    process.stdin.writeln(line);
    await process.stdin.flush();
  });

  /// A new link to `@control`.
  HostLink open() {
    final name = '${++nextLink}';
    inbox[name] = StreamController<String>.broadcast();
    return ControlProcessLink(this, name);
  }
}

final class ControlProcessLink implements HostLink {
  ControlProcessLink(this.shared, this.name);

  final ControlProcess shared;
  final String name;

  @override
  Stream<String> get messages => shared.inbox[name]!.stream;

  @override
  Future<void> send(String message) async {
    await shared.write('$name $message');
  }

  @override
  Future<void> close() async {
    try {
      await shared.write('$name close');
    } on Object {
      // The example is gone already.
    }
    await shared.inbox[name]?.close();
  }
}
