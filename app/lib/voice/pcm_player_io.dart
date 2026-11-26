import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import '../brand.dart';
import 'pcm_player.dart';
import 'pcm_player_windows.dart';
import 'speech_backend.dart' show SpeechFailure;
import 'speech_output.dart' show PcmFormat;

const _noPlayer = 'No program to play audio with was found on this computer.';

/// The system's own audio output, with no plugin and nothing downloaded: Windows' waveOut, `afplay` on macOS, and on Linux the
/// first of `paplay`, `pw-cat` and `aplay` that is installed.
PcmPlayer? create() {
  if (Platform.isWindows) return WaveOutPlayer();
  if (Platform.isMacOS) return AfplayPlayer();
  if (Platform.isLinux) return StreamPlayer(linuxPlayers(), executableOnPath);
  return null;
}

/// One way of playing raw samples on a command line: [program] and its arguments for [format], reading the samples on its standard
/// input.
typedef PlayerCommand = ({String program, List<String> Function(PcmFormat format) arguments});

/// The programs Linux plays raw samples with, in the order they are tried: PulseAudio's `paplay`, PipeWire's `pw-cat` and ALSA's
/// `aplay`.
List<PlayerCommand> linuxPlayers() => [
  (program: 'paplay', arguments: (f) => ['--raw', '--format=s16le', '--rate=${f.rate}', '--channels=${f.channels}']),
  (program: 'pw-cat', arguments: (f) => ['--playback', '--format', 's16', '--rate', '${f.rate}', '--channels', '${f.channels}', '-']),
  (program: 'aplay', arguments: (f) => ['-q', '-t', 'raw', '-f', 'S16_LE', '-r', '${f.rate}', '-c', '${f.channels}', '-']),
];

/// The path of [program] on this computer's PATH, or null.
String? executableOnPath(String program) {
  final path = Platform.environment['PATH'];
  if (path == null) return null;
  for (final directory in path.split(Platform.isWindows ? ';' : ':')) {
    if (directory.isEmpty) continue;
    final candidate = File('$directory${Platform.pathSeparator}$program');
    if (candidate.existsSync()) return candidate.path;
  }
  return null;
}

/// Plays through a program that reads raw samples on its standard input and plays them as they arrive.
final class StreamPlayer implements PcmPlayer {
  StreamPlayer(this._commands, this._find, {Future<Process> Function(String program, List<String> arguments)? start})
    : _start = start ?? ((program, arguments) => Process.start(program, arguments));

  final List<PlayerCommand> _commands;
  final String? Function(String program) _find;
  final Future<Process> Function(String program, List<String> arguments) _start;

  Process? _process;
  PcmFormat? _format;
  Future<void>? _exited;

  @override
  void unlock() {}

  @override
  Future<void> open(PcmFormat format) async {
    if (_format == format && _process != null) return;
    if (_process != null) await drain();
    for (final command in _commands) {
      final found = _find(command.program);
      if (found == null) continue;
      final process = await _start(found, command.arguments(format));
      // What the player says is not kept; its output is only drained so it never blocks.
      unawaited(process.stdout.drain<void>());
      unawaited(process.stderr.drain<void>());
      _process = process;
      _format = format;
      _exited = process.exitCode.then((_) {
        if (identical(_process, process)) {
          _process = null;
          _format = null;
        }
      });
      return;
    }
    throw const SpeechFailure(_noPlayer);
  }

  @override
  Future<void> write(Uint8List pcm) async {
    final process = _process;
    if (process == null) return;
    try {
      process.stdin.add(pcm);
      // The pipe holds what the player has not read, so this waits at the pace of the speakers.
      await process.stdin.flush();
    } on Object {
      // The player ended: the next open starts another.
      _process = null;
      _format = null;
    }
  }

  @override
  Future<void> drain() async {
    final process = _process, exited = _exited;
    if (process == null) return;
    try {
      await process.stdin.close();
    } on Object {
      // Already closed.
    }
    await exited;
    _process = null;
    _format = null;
  }

  @override
  Future<void> stop() async {
    final process = _process;
    _process = null;
    _format = null;
    process?.kill();
    await _exited;
  }
}

/// macOS has no player that reads raw samples, so what is written is cut into pieces of about a second, each played by `afplay` from a
/// temporary WAV file that is removed as soon as it has played.
final class AfplayPlayer implements PcmPlayer {
  AfplayPlayer({String? Function(String program)? find, Future<Process> Function(String program, List<String> arguments)? start, Directory? temp})
    : _find = find ?? executableOnPath,
      _start = start ?? ((program, arguments) => Process.start(program, arguments)),
      _temp = temp ?? Directory.systemTemp;

  final String? Function(String program) _find;
  final Future<Process> Function(String program, List<String> arguments) _start;
  final Directory _temp;

  PcmFormat? _format;
  final _pending = BytesBuilder(copy: false);
  final _queue = <Uint8List>[];
  Process? _playing;
  bool _running = false;
  Completer<void>? _idle;
  Completer<void>? _room;
  int _queued = 0;
  bool _stopped = false;

  @override
  void unlock() {}

  @override
  Future<void> open(PcmFormat format) async {
    if (_format == format) return;
    if (_format != null) await drain();
    if (_find('afplay') == null) throw const SpeechFailure(_noPlayer);
    _format = format;
    _stopped = false;
  }

  @override
  Future<void> write(Uint8List pcm) async {
    final format = _format;
    if (format == null) return;
    _pending.add(pcm);
    if (_pending.length >= bytesPerSecond(format)) _cut();
    if (_queued > bytesPerSecond(format) * pcmAheadSeconds) {
      final room = _room = Completer<void>();
      await room.future;
    }
  }

  void _cut() {
    final bytes = _pending.takeBytes();
    if (bytes.isEmpty) return;
    _queue.add(bytes);
    _queued += bytes.length;
    if (!_running) unawaited(_run());
  }

  Future<void> _run() async {
    _running = true;
    final format = _format;
    final afplay = _find('afplay');
    try {
      while (_queue.isNotEmpty && format != null && afplay != null && !_stopped) {
        final piece = _queue.removeAt(0);
        final file = File('${_temp.path}${Platform.pathSeparator}${Brand.program}-say-${DateTime.now().microsecondsSinceEpoch}.wav');
        try {
          await file.writeAsBytes(wavOf(piece, format));
          final process = _playing = await _start(afplay, [file.path]);
          unawaited(process.stdout.drain<void>());
          unawaited(process.stderr.drain<void>());
          await process.exitCode;
        } finally {
          _playing = null;
          try {
            await file.delete();
          } on FileSystemException {
            // Gone already.
          }
        }
        _queued -= piece.length;
        _wake();
      }
    } finally {
      _running = false;
      _wake();
    }
  }

  void _wake() {
    final format = _format;
    if (_room != null && (format == null || _queued <= bytesPerSecond(format) * pcmAheadSeconds)) {
      _room!.complete();
      _room = null;
    }
    if (!_running && _queue.isEmpty && _idle != null) {
      _idle!.complete();
      _idle = null;
    }
  }

  @override
  Future<void> drain() async {
    _cut();
    if (_running || _queue.isNotEmpty) {
      final idle = _idle ??= Completer<void>();
      await idle.future;
    }
    _format = null;
  }

  @override
  Future<void> stop() async {
    _stopped = true;
    _queue.clear();
    _pending.clear();
    _queued = 0;
    _playing?.kill();
    _format = null;
    _wake();
  }
}

