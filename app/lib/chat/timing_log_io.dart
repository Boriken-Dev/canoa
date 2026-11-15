import 'dart:io';

/// The file `CANOA_TIMING_LOG` names, read when asked so a test can set it; null or empty means timing is off.
String? _target() {
  final path = Platform.environment['CANOA_TIMING_LOG'];
  return path == null || path.isEmpty ? null : path;
}

/// Appends `step, ms` to the file `CANOA_TIMING_LOG` names. Never a message, a path or a name; nothing without the variable.
void timingMark(String step, int ms) {
  final path = _target();
  if (path == null) return;
  File(path).writeAsStringSync('$step, $ms\n', mode: FileMode.append, flush: true);
}

/// Runs [run] and, when `CANOA_TIMING_LOG` is set, records how long it took under [step], also when it throws.
Future<T> timed<T>(String step, Future<T> Function() run) async {
  if (_target() == null) return run();
  final watch = Stopwatch()..start();
  try {
    return await run();
  } finally {
    timingMark(step, watch.elapsedMilliseconds);
  }
}

/// Whether `CANOA_TIMING_LOG` is set, so callers skip work that only feeds the log.
bool get timingOn => _target() != null;
