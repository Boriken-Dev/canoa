import 'dart:typed_data';

import 'pcm_player_stub.dart'
    if (dart.library.js_interop) 'pcm_player_web.dart'
    if (dart.library.io) 'pcm_player_io.dart'
    as platform;
import 'speech_output.dart' show PcmFormat;

/// Plays 16-bit little-endian PCM as it arrives, on the speakers of the computer this app or page runs on. Nothing is kept: what
/// was played is gone, and a failure to play is a [SpeechFailure] saying why.
abstract interface class PcmPlayer {
  /// Gets the output ready from inside a press or a click, which a browser needs before it lets a page make a sound. Does nothing
  /// where that is not so; the app calls it from the handler of the press that will lead to speech.
  void unlock();

  /// Begins playing audio in [format]; the audio comes through [write]. A player that is already open in the same format goes on.
  Future<void> open(PcmFormat format);

  /// Queues [pcm], which follows what was written before. Completes when the player has room for more, so a caller that waits for it
  /// keeps no more than a few seconds ahead of the speakers.
  Future<void> write(Uint8List pcm);

  /// Completes when everything written has been played, and closes the output.
  Future<void> drain();

  /// Drops what was written and has not been played, and closes the output.
  Future<void> stop();
}

/// The player of this platform: the page's Web Audio in a browser, the system's own audio output on Windows (waveOut), macOS
/// (`afplay`) and Linux (`paplay`, `pw-cat` or `aplay`, whichever is installed); null where there is none.
PcmPlayer? platformPcmPlayer() => platform.create();

/// How many seconds of audio a player holds before [PcmPlayer.write] makes its caller wait.
const pcmAheadSeconds = 3.0;

/// Bytes in one second of [format]'s audio.
int bytesPerSecond(PcmFormat format) => format.rate * format.channels * 2;

/// The WAV file holding [pcm] in [format], for a player that is given a file.
Uint8List wavOf(Uint8List pcm, PcmFormat format) {
  final out = ByteData(44 + pcm.length);
  void tag(int at, String text) {
    for (var i = 0; i < 4; i++) {
      out.setUint8(at + i, text.codeUnitAt(i));
    }
  }

  tag(0, 'RIFF');
  out.setUint32(4, 36 + pcm.length, Endian.little);
  tag(8, 'WAVE');
  tag(12, 'fmt ');
  out.setUint32(16, 16, Endian.little);
  out.setUint16(20, 1, Endian.little);
  out.setUint16(22, format.channels, Endian.little);
  out.setUint32(24, format.rate, Endian.little);
  out.setUint32(28, bytesPerSecond(format), Endian.little);
  out.setUint16(32, format.channels * 2, Endian.little);
  out.setUint16(34, 16, Endian.little);
  tag(36, 'data');
  out.setUint32(40, pcm.length, Endian.little);
  final bytes = out.buffer.asUint8List();
  bytes.setRange(44, 44 + pcm.length, pcm);
  return bytes;
}
