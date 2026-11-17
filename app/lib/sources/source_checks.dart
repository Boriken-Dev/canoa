import 'dart:async';

import 'sources.dart';

/// What each source's lookup last answered. A refresh asks every source side by side and records each answer as it arrives, so
/// a source that is ready is usable while another is still being looked for; a lookup already running for a source is joined,
/// not repeated; and an answer to a refresh that a later one has replaced is dropped.
final class SourceChecks {
  SourceChecks(this.lookup, {this.patience = const Duration(seconds: 30), this.onChange});

  /// Looks one source up: what it needs and whether it is here.
  final Future<SourceCheck> Function(AgentSource source) lookup;

  /// How long one lookup may take before its source shows as not answering.
  final Duration patience;

  /// Told each time an answer is recorded.
  final void Function()? onChange;

  Map<AgentSource, SourceCheck> _results = const {};
  final Map<AgentSource, Future<SourceCheck>> _running = {};
  int _run = 0;

  /// The answers so far; a source not looked at yet has none.
  Map<AgentSource, SourceCheck> get results => _results;

  /// Looks at [sources], ending when every lookup has. A source of an earlier refresh that is gone has no answer afterwards.
  Future<void> refresh(Iterable<AgentSource> sources) async {
    final run = ++_run;
    final wanted = sources.toList();
    _running.clear();
    _results = {for (final entry in _results.entries) if (wanted.contains(entry.key)) entry.key: entry.value};
    await Future.wait([
      for (final source in wanted)
        check(source).then((answer) {
          if (run != _run) return;
          _results = {..._results, source: answer};
          onChange?.call();
        }),
    ]);
  }

  /// [source]'s lookup, joining the one running for it, and ending after [patience] with a problem when it does not answer.
  Future<SourceCheck> check(AgentSource source) {
    final running = _running[source];
    if (running != null) return running;
    late final Future<SourceCheck> started;
    started = Future.sync(() => lookup(source))
        .timeout(patience, onTimeout: () => SourceCheck(problem: '${source.label} did not answer in ${patience.inSeconds} seconds.'))
        .whenComplete(() {
          if (identical(_running[source], started)) _running.remove(source);
        });
    return _running[source] = started;
  }
}
