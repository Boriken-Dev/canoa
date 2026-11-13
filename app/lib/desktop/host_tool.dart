import 'dart:convert';
import 'dart:typed_data';

import 'host_tool_stub.dart' if (dart.library.io) 'host_tool_io.dart' as platform;

/// The status of `canoa serve` when another host holds its home.
const int statusHeld = 3;

/// What `canoa serve` printed for one entry it serves.
final class HostListening {
  const HostListening({
    required this.name, required this.kind, required this.url, required this.peer, this.method, this.harness, this.title,
    this.local = false, this.missing = const [],
  });

  final String name;

  /// `agent` or `provider`.
  final String kind;
  final String url;

  /// The host's own peer id, which a client pins.
  final String peer;

  /// The proof the host runs, `noise` or `webcrypto`; null from a host that does not say.
  final String? method;

  /// The provider id of a `harness` entry (`claude`, `codex`, an outside provider's); null for any other entry.
  final String? harness;

  /// The entry's `title` in canoa.cfg; null shows its name.
  final String? title;

  /// One of the host's own sources: it has no address ([url] is empty), and is reached only through the host's links.
  final bool local;

  /// The programs a local source needs that the host did not find; empty when it can start.
  final List<String> missing;

  /// One line of `serve`'s output, or null for anything else.
  static HostListening? parse(String line) {
    try {
      final value = jsonDecode(line);
      if (value is! Map) return null;
      final local = value['local'] == true;
      final [name, kind, url, peer] = [value['name'], value['kind'], local ? '' : value['url'], value['peer']];
      if (name is! String || kind is! String || url is! String || peer is! String) return null;
      final method = value['method'];
      final [harness, title] = [value['harness'], value['title']];
      return HostListening(
        name: name, kind: kind, url: url, peer: peer, method: method is String ? method : null,
        harness: harness is String ? harness : null, title: title is String ? title : null,
        local: local,
        missing: [if (value['missing'] is List) for (final each in value['missing'] as List) if (each is String) each],
      );
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

/// What one `canoa` command left when it ended.
final class HostOutput {
  const HostOutput(this.exitCode, this.stdout, this.stderr);
  final int exitCode;
  final String stdout;
  final String stderr;
}

/// A `canoa` command running: in this app's own process through the library, or as a child where `CANOA_HOST` names one.
abstract interface class HostChild {
  /// The process it runs in.
  int get pid;

  /// Its standard output, line by line.
  Stream<String> get lines;

  /// Its standard error, line by line.
  Stream<String> get errors;
  Future<int> get exitCode;

  /// Closes its input, which `serve --until-stdin-closes` ends on, with every agent it started.
  Future<void> closeInput();

  /// Ends it at once.
  void kill();
}

/// A connection to one of a running host's entries from inside this process: whole ACP messages each way, with no proof, since
/// nothing outside the process can hold one. Closing it ends the entry's agent for it.
abstract interface class HostLink {
  /// What the entry's agent sends; ends once the link has.
  Stream<String> get messages;

  /// Sends one message, waiting while the host has taken too many. Throws [StateError] once the link has ended.
  Future<void> send(String message);
  Future<void> close();
}

/// A link that holds the secret its host shares with this process for the secret prompts of the links it opens: minted by the host,
/// kept in memory, and never sent anywhere. Only a link to a host in this process has one.
abstract interface class HostLinkSecret {
  /// The 32 bytes, or null when the host has none to give.
  Uint8List? get linkSecret;
}

/// A [HostChild] that is a `serve` in this process, which can be connected to without a socket.
abstract interface class HostLinks {
  /// Throws when the host does not serve [entry] (yet), or has stopped.
  HostLink openLink(String entry);
}

/// How this app runs `canoa` commands.
abstract interface class HostTool {
  /// What they run from: the library's file, or the program's.
  String get path;
  Future<HostChild> start(List<String> args);
  Future<HostOutput> run(List<String> args);
}

/// How to run `canoa`: the program `CANOA_HOST` names, for tests, else the library this app carries, else not at all. A
/// web build has none.
HostTool? findHostTool() => platform.findHostTool();

/// A port on loopback that nothing listens on at this moment.
Future<int> freeLoopbackPort() => platform.freeLoopbackPort();

/// Runs [use] on a file holding [text] in the directory of [path], so names relative to it read the same, and removes it.
Future<T> withFileBeside<T>(String path, String text, Future<T> Function(String file) use) =>
    platform.withFileBeside(path, text, use);

/// What a line of a host's standard error says after its `canoa: ` prefix (a host built before the rename wrote
/// `canoa-host: `), or null for any other line.
String? hostSaid(String line) {
  for (final prefix in const ['canoa: ', 'canoa-host: ']) {
    if (line.startsWith(prefix)) return line.substring(prefix.length);
  }
  return null;
}
