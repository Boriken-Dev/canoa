// `canoa serve` as a process of its own, on a configuration and a home inside a test's folder, and the way a page reaches it: the
// listing of what it serves, which the app reads to find the control channel. The host proves its key by the method it names; the
// page's side of that proof is the web_peer example (test/fixtures/web_peer_bridge.dart), since the Dart VM has no WebCrypto.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/auth/peer_proof.dart';

/// A host started as a supervisor starts one (`--supervised`, which offers `host/restart`), which a test ends and starts again
/// ([stop], [start], or [supervise] after the host asked to restart) on the same configuration and home.
/// Everything it writes goes under [dir]: its identity and the scripts that ship in it in `peers`, and
/// the home folder it reports as the person's in `home`.
final class ServedProcess {
  ServedProcess({
    required this.binary,
    required this.dir,
    required this.config,
    required this.method,
    this.environment = const {},
  });

  /// The built `canoa`.
  final String binary;
  final Directory dir;
  final File config;
  final String method;

  /// More variables for the host's environment, over the ones it is given for its home.
  final Map<String, String> environment;

  /// The status `canoa serve` ends with when a key granted `host.restart` asked it to start again.
  static const restartStatus = 75;

  late Process process;
  final urls = <String, String>{};
  late String peer;
  final _output = <String>[];

  /// The host's state directory (its identity, the keys it paired, the extensions it installed and the scripts it carries).
  String get state => '${dir.path.replaceAll(r'\', '/')}/peers';

  /// The folder it reports as the person's home.
  String get home => '${dir.path.replaceAll(r'\', '/')}/home';

  Map<String, String> get _environment => {
    'HOME': home,
    'USERPROFILE': home,
    'COQUI_AGENTIC_PEER_HOME': state,
    ...environment,
  };

  Future<void> start() async {
    process = await Process.start(binary, [
      'serve',
      '--config',
      config.path,
      '--method',
      method,
      '--supervised',
    ], environment: _environment);
    process.stderr.transform(utf8.decoder).listen(stderr.write);
    final ready = Completer<void>();
    process.stdout
        .transform(utf8.decoder)
        .transform(const LineSplitter())
        .listen((line) {
          _output.add(line);
          final entry = jsonDecode(line) as Map<String, dynamic>;
          if (entry['name'] is String) {
            urls[entry['name'] as String] = entry['url'] as String;
            peer = entry['peer'] as String;
          }
          if (entry['ready'] == true && !ready.isCompleted) ready.complete();
        });
    await ready.future.timeout(const Duration(seconds: 60));
  }

  /// Another `canoa` command on this host's home.
  Future<ProcessResult> command(List<String> args) =>
      Process.run(binary, args, environment: _environment);

  /// What a supervisor does when the host ends because a key asked it to restart: waits for it to end with the status that says so
  /// ([restartStatus]), then starts it again on the same configuration and home.
  Future<void> supervise({
    Duration within = const Duration(seconds: 30),
  }) async {
    final code = await process.exitCode.timeout(within);
    if (code != restartStatus) {
      throw StateError(
        'the host ended with status $code, not the restart status $restartStatus',
      );
    }
    await start();
  }

  Future<void> stop() async {
    process.kill();
    await process.exitCode;
  }

  /// What a page is given to find this host: its key and what it serves, in the form the app reads.
  String listing(PeerMethod method) => jsonEncode({
    'peer': peer,
    'method': method.id,
    'entries': [
      for (final MapEntry(:key, :value) in urls.entries)
        {
          'name': key,
          'kind': key == '@control' ? 'control' : 'agent',
          'url': value,
        },
    ],
  });
}
