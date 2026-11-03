import 'dart:convert';

import 'host_tool_stub.dart' if (dart.library.io) 'host_tool_io.dart' as platform;

/// What `canoa-host serve` printed for one entry it serves.
final class HostListening {
  const HostListening({required this.name, required this.kind, required this.url, required this.peer});

  final String name;

  /// `agent` or `provider`.
  final String kind;
  final String url;

  /// The host's own peer id, which a client pins.
  final String peer;

  /// One line of `serve`'s output, or null for anything else.
  static HostListening? parse(String line) {
    try {
      final value = jsonDecode(line);
      if (value is! Map) return null;
      final [name, kind, url, peer] = [value['name'], value['kind'], value['url'], value['peer']];
      if (name is! String || kind is! String || url is! String || peer is! String) return null;
      return HostListening(name: name, kind: kind, url: url, peer: peer);
    } on FormatException {
      return null;
    }
  }
}

/// A key in the host's trust store: paired, or waiting for a person to accept it.
final class HostKey {
  const HostKey({required this.id, required this.accepted, this.from});

  final String id;
  final bool accepted;

  /// Where a waiting key connected from, when the host recorded it.
  final String? from;

  /// The lines of `peers --format json`. A line that is not one key is left out.
  static List<HostKey> parseAll(String text) => [
    for (final line in const LineSplitter().convert(text))
      if (_parse(line) case final HostKey key) key,
  ];

  static HostKey? _parse(String line) {
    try {
      final value = jsonDecode(line);
      if (value is! Map || value['peer'] is! String) return null;
      final state = value['state'];
      if (state != 'accepted' && state != 'waiting') return null;
      return HostKey(
        id: value['peer'] as String,
        accepted: state == 'accepted',
        from: value['source'] is String ? value['source'] as String : null,
      );
    } on FormatException {
      return null;
    }
  }
}

/// What one run of a `canoa-host` command left.
final class HostOutput {
  const HostOutput(this.exitCode, this.stdout, this.stderr);
  final int exitCode;
  final String stdout;
  final String stderr;
}

/// A `canoa-host` running as this app's child.
abstract interface class HostChild {
  int get pid;

  /// Its standard output, line by line.
  Stream<String> get lines;

  /// Its standard error, line by line.
  Stream<String> get errors;
  Future<int> get exitCode;

  /// Closes its standard input, which `serve --until-stdin-closes` ends on, with every agent it started.
  Future<void> closeInput();

  /// Ends it at once.
  void kill();
}

/// The `canoa-host` program this app runs.
abstract interface class HostTool {
  String get path;
  Future<HostChild> start(List<String> args);
  Future<HostOutput> run(List<String> args);
}

/// The `canoa-host` to run: the one `CANOA_HOST` names, else the one installed beside this app's own executable, else
/// none. A web build has none.
HostTool? findHostTool() => platform.findHostTool();

/// A port on loopback that nothing listens on at this moment.
Future<int> freeLoopbackPort() => platform.freeLoopbackPort();

/// Runs [use] on a file holding [text] in the directory of [path], so names relative to it read the same, and removes it.
Future<T> withFileBeside<T>(String path, String text, Future<T> Function(String file) use) =>
    platform.withFileBeside(path, text, use);
