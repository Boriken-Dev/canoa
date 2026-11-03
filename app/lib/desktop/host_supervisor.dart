import 'dart:async';
import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';

import 'host_tool.dart';

enum HostPhase {
  /// No `canoa-host` was found to run.
  absent,

  /// Canoa.cfg names nothing to host.
  idle,
  starting,
  running,

  /// It ended on its own, and starts again after [HostSupervisor.retryIn].
  waiting,

  /// Canoa.cfg is wrong for it; it starts again only when told to.
  failed,
  stopped,
}

/// The most of the host's standard error kept, for saying why it ended.
const int maxHostLog = 16 * 1024;

/// How long the host has to end after its input closes before it is killed: longer than the host gives its agents.
const Duration hostLeaving = Duration(seconds: 8);

/// A `canoa-host` command that did not succeed, in its words.
final class HostCommandFailed implements Exception {
  const HostCommandFailed(this.message);
  final String message;
  @override
  String toString() => message;
}

/// `canoa-host pair` running: the PIN it shows, until a native client uses it, it fails, or it is cancelled.
final class HostPairing {
  HostPairing._(this._child, this.pin, this.hostId, this.method, this.url, this.paired);

  final HostChild _child;
  final String pin;

  /// The host's peer id, which the other side shows once it has paired.
  final String hostId;

  /// The proof the host runs, `noise` or `webcrypto`; null from a host that does not say.
  final String? method;

  /// Where the other side dials.
  final String url;

  /// The id of the key that paired; an error once the PIN is used up, expires or is cancelled.
  final Future<String> paired;

  static Future<HostPairing> _start(HostChild child) async {
    final paired = Completer<String>();
    final shown = Completer<Map<Object?, Object?>>();
    final said = StringBuffer();
    child.errors.listen((line) {
      if (line.startsWith('canoa-host: ')) said.write(line.substring('canoa-host: '.length));
    });
    child.lines.listen((line) {
      final Object? value;
      try {
        value = jsonDecode(line);
      } on FormatException {
        return;
      }
      if (value is! Map) return;
      if (!shown.isCompleted) {
        shown.complete(value);
      } else if (value['paired'] is String && !paired.isCompleted) {
        paired.complete(value['paired'] as String);
      }
    });
    unawaited(
      child.exitCode.then((code) {
        final why = HostCommandFailed(said.isEmpty ? 'pairing ended with status $code' : said.toString());
        if (!shown.isCompleted) shown.completeError(why);
        if (!paired.isCompleted) paired.completeError(why);
      }),
    );
    final first = await shown.future.timeout(
      const Duration(seconds: 20),
      onTimeout: () {
        child.kill();
        throw const HostCommandFailed('canoa-host did not show a PIN');
      },
    );
    final [pin, hostId, url] = [first['pin'], first['peer'], first['url']];
    if (pin is! String || hostId is! String || url is! String) {
      child.kill();
      throw const HostCommandFailed('canoa-host showed no PIN');
    }
    // Nobody may be waiting on it when it is cancelled.
    unawaited(paired.future.then((_) {}, onError: (Object _) {}));
    final method = first['method'];
    return HostPairing._(child, pin, hostId, method is String ? method : null, url, paired.future);
  }

  /// Ends the pairing; the PIN can no longer be used.
  void cancel() => _child.kill();
}

/// Runs `canoa-host serve` as this app's child: started when Canoa.cfg names something to host, started again with a
/// growing pause when it ends on its own, and ended, with every agent it started, when the app quits.
class HostSupervisor extends ChangeNotifier {
  HostSupervisor(
    this.tool, {
    this.firstPause = const Duration(seconds: 1),
    this.longestPause = const Duration(minutes: 1),
    this.steady = const Duration(seconds: 30),
    this.errorDrain = const Duration(seconds: 2),
    DateTime Function()? clock,
  }) : _clock = clock ?? DateTime.now;

  /// Null when none was found: nothing can be hosted.
  final HostTool? tool;

  /// The pause after the first unplanned end, doubled for each one after, up to [longestPause].
  final Duration firstPause, longestPause;

  /// How long a run must last for the next end to count as a first one again.
  final Duration steady;

  /// How long, after the host ends, its standard error may take to close, so its last words reach the reason shown.
  final Duration errorDrain;
  final DateTime Function() _clock;

  HostPhase _phase = HostPhase.stopped;
  HostPhase get phase => tool == null ? HostPhase.absent : _phase;

  /// What the entries are listening on, while running.
  List<HostListening> get listening => List.unmodifiable(_listening);
  final List<HostListening> _listening = [];

  /// The host's peer id, once it has said it.
  String? get peerId => _listening.isEmpty ? _peerId : _listening.first.peer;
  String? _peerId;

  /// The proof the host runs, `noise` or `webcrypto`, once it has said it.
  String? get peerMethod => _listening.isEmpty ? _peerMethod : _listening.first.method;
  String? _peerMethod;

  /// Why it last ended or could not start, in its own words.
  String? get problem => _problem;
  String? _problem;

  /// How many times in a row it has ended on its own.
  int get restarts => _restarts;
  int _restarts = 0;

  /// When it starts again, while [phase] is [HostPhase.waiting].
  DateTime? get retryAt => _retryAt;
  DateTime? _retryAt;

  /// What the running host said on its standard error, its latest [maxHostLog] characters.
  String get log => _log.toString();

  /// The running host's process id.
  int? get pid => _child?.pid;
  final StringBuffer _log = StringBuffer();

  String? _configPath;
  HostChild? _child;
  Timer? _retry;
  DateTime? _startedAt;
  int _generation = 0;

  /// Serves Canoa.cfg at [configPath] when [hasEntries], else stops: called with the file as saved, after every change.
  Future<void> apply({required String? configPath, required bool hasEntries}) async {
    _configPath = configPath;
    await _end();
    _restarts = 0;
    _problem = null;
    if (tool == null) return notifyListeners();
    if (configPath == null || !hasEntries) {
      _phase = HostPhase.idle;
      return notifyListeners();
    }
    await _start();
  }

  /// Starts again at once, as a person asks after fixing what was wrong.
  Future<void> restart() async {
    await _end();
    _restarts = 0;
    if (tool != null && _configPath != null) await _start();
  }

  /// Ends the host and every agent it started. Nothing starts it again until [apply] or [restart].
  Future<void> stop() async {
    await _end();
    _phase = HostPhase.stopped;
    notifyListeners();
  }

  /// The pause before starting again after [ends] unplanned ends in a row.
  Duration pauseAfter(int ends) {
    final doubled = firstPause * math.pow(2, math.max(0, ends - 1)).toInt();
    return doubled > longestPause ? longestPause : doubled;
  }

  Future<void> _start() async {
    final generation = ++_generation;
    _phase = HostPhase.starting;
    _listening.clear();
    _retryAt = null;
    _startedAt = null;
    _log.clear();
    notifyListeners();
    final HostChild child;
    try {
      child = await tool!.start(['serve', '--config', _configPath!, '--until-stdin-closes']);
    } on Object catch (error) {
      _problem = 'canoa-host could not start: $error';
      return _ended(generation, null);
    }
    if (generation != _generation) {
      // Ended while it was starting.
      await _close(child);
      return;
    }
    _child = child;
    _startedAt = _clock();
    final errorsDone = Completer<void>();
    child.errors.listen(
      _keep,
      onDone: () {
        if (!errorsDone.isCompleted) errorsDone.complete();
      },
    );
    child.lines.listen((line) {
      if (generation != _generation) return;
      final entry = HostListening.parse(line);
      if (entry == null) return;
      _listening.add(entry);
      _peerId = entry.peer;
      _peerMethod = entry.method;
      _phase = HostPhase.running;
      notifyListeners();
    });
    // Its status can come before its last line on standard error, which says why: wait for that to close, within a bound.
    unawaited(
      child.exitCode.then((code) async {
        await errorsDone.future.timeout(errorDrain, onTimeout: () {});
        _ended(generation, code);
      }),
    );
  }

  void _keep(String line) {
    _log.writeln(line);
    if (_log.length > maxHostLog) {
      final kept = _log.toString();
      _log
        ..clear()
        ..write(kept.substring(kept.length - maxHostLog));
    }
  }

  void _ended(int generation, int? code) {
    if (generation != _generation) return;
    _child = null;
    _listening.clear();
    if (code != null) {
      final said = const LineSplitter().convert(_log.toString()).where((line) => line.startsWith('canoa-host: '));
      _problem = said.isEmpty ? 'canoa-host ended with status $code' : said.last.substring('canoa-host: '.length);
    }
    // Status 2 is a wrong Canoa.cfg or invocation: the same file fails the same way, so it waits for a person.
    if (code == 2) {
      _phase = HostPhase.failed;
      return notifyListeners();
    }
    final ranFor = _startedAt == null ? Duration.zero : _clock().difference(_startedAt!);
    _restarts = ranFor >= steady ? 1 : _restarts + 1;
    final pause = pauseAfter(_restarts);
    _phase = HostPhase.waiting;
    _retryAt = _clock().add(pause);
    _retry = Timer(pause, () {
      if (generation == _generation) unawaited(_start());
    });
    notifyListeners();
  }

  Future<void> _end() async {
    _generation++;
    _retry?.cancel();
    _retry = null;
    _retryAt = null;
    _listening.clear();
    final child = _child;
    _child = null;
    if (child != null) await _close(child);
  }

  /// Closing its input lets it end its agents itself; killing is for a host that does not.
  Future<void> _close(HostChild child) async {
    await child.closeInput();
    await child.exitCode.timeout(
      hostLeaving,
      onTimeout: () {
        child.kill();
        return child.exitCode;
      },
    );
  }

  /// The keys the host's trust store holds, paired first.
  Future<List<HostKey>> keys() async {
    final listed = await _command(['peers', '--format', 'json']);
    final keys = HostKey.parseAll(listed.stdout);
    return [...keys.where((key) => key.accepted), ...keys.where((key) => !key.accepted)];
  }

  /// Pairs a waiting key. Only a person's choice calls this: nothing is accepted by itself.
  Future<void> accept(String id) => _command(['accept', '--', id]);

  /// Removes a key, paired or waiting. A link it holds ends within about a second.
  Future<void> revoke(String id) => _command(['revoke', '--', id]);

  /// What is wrong with Canoa.cfg at [configPath] for the host, in its words, or null when nothing is.
  Future<String?> check(String configPath) async {
    final tool = this.tool;
    if (tool == null) return 'canoa-host is not installed';
    final checked = await tool.run(['check', '--config', configPath]);
    return checked.exitCode == 0 ? null : _said(checked);
  }

  /// Shows a PIN a native client pairs with. One wrong try ends it.
  Future<HostPairing> pair() async {
    final tool = this.tool;
    if (tool == null) throw const HostCommandFailed('canoa-host is not installed');
    return HostPairing._start(await tool.start(['pair', '--until-stdin-closes']));
  }

  Future<HostOutput> _command(List<String> args) async {
    final tool = this.tool;
    if (tool == null) throw const HostCommandFailed('canoa-host is not installed');
    final ran = await tool.run(args);
    if (ran.exitCode != 0) throw HostCommandFailed(_said(ran));
    return ran;
  }

  static String _said(HostOutput output) {
    final lines = const LineSplitter().convert(output.stderr).where((line) => line.trim().isNotEmpty);
    if (lines.isEmpty) return 'canoa-host ended with status ${output.exitCode}';
    final last = lines.last;
    return last.startsWith('canoa-host: ') ? last.substring('canoa-host: '.length) : last;
  }

  @override
  void dispose() {
    _generation++;
    _retry?.cancel();
    final child = _child;
    _child = null;
    if (child != null) unawaited(_close(child));
    super.dispose();
  }
}
