// The players of 16-bit PCM: the WAV a file player is given, the command lines Linux and macOS play with, and, on Windows, the real
// waveOut output, which is given silence so nothing is heard.
@TestOn('vm')
library;

import 'dart:io';
import 'dart:typed_data';

import 'package:canoa/voice/pcm_player.dart';
import 'package:canoa/voice/pcm_player_io.dart';
import 'package:canoa/voice/pcm_player_windows.dart';
import 'package:canoa/voice/speech_backend.dart' show SpeechFailure;
import 'package:canoa/voice/speech_output.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('a WAV holds the samples after a header that names their format', () {
    final pcm = Uint8List.fromList([1, 0, 2, 0, 3, 0]);
    final wav = wavOf(pcm, const PcmFormat(rate: 22050));
    expect(String.fromCharCodes(wav.sublist(0, 4)), 'RIFF');
    expect(String.fromCharCodes(wav.sublist(8, 16)), 'WAVEfmt ');
    final header = ByteData.sublistView(wav);
    expect(header.getUint32(4, Endian.little), 36 + 6);
    expect((header.getUint16(22, Endian.little), header.getUint32(24, Endian.little), header.getUint32(28, Endian.little)), (1, 22050, 44100));
    expect((header.getUint16(32, Endian.little), header.getUint16(34, Endian.little)), (2, 16));
    expect(String.fromCharCodes(wav.sublist(36, 40)), 'data');
    expect(header.getUint32(40, Endian.little), 6);
    expect(wav.sublist(44), pcm);
    expect(bytesPerSecond(const PcmFormat(rate: 24000, channels: 2)), 96000);
  });

  group('Linux', () {
    test('plays through the first of paplay, pw-cat and aplay that is installed, with the audio on its standard input', () async {
      final started = <(String, List<String>)>[];
      final player = StreamPlayer(
        linuxPlayers(),
        (program) => program == 'pw-cat' || program == 'aplay' ? '/usr/bin/$program' : null,
        start: (program, arguments) {
          started.add((program, arguments));
          throw const SpeechFailure('stop here');
        },
      );
      await expectLater(player.open(const PcmFormat(rate: 22050)), throwsA(isA<SpeechFailure>()));
      expect(started.single.$1, '/usr/bin/pw-cat');
      expect(started.single.$2, ['--playback', '--format', 's16', '--rate', '22050', '--channels', '1', '-']);
    });

    test('the command lines of each player carry the rate and channels', () {
      final byName = {for (final each in linuxPlayers()) each.program: each};
      const format = PcmFormat(rate: 24000, channels: 2);
      expect(byName['paplay']!.arguments(format), ['--raw', '--format=s16le', '--rate=24000', '--channels=2']);
      expect(byName['aplay']!.arguments(format), ['-q', '-t', 'raw', '-f', 'S16_LE', '-r', '24000', '-c', '2', '-']);
    });

    test('with none installed, it says so in words, and plays nothing', () async {
      final player = StreamPlayer(linuxPlayers(), (_) => null);
      await expectLater(
        player.open(const PcmFormat(rate: 22050)),
        throwsA(isA<SpeechFailure>().having((failure) => failure.message, 'message', contains('No program to play audio'))),
      );
    });
  });

  group('macOS', () {
    test('without afplay it says so', () async {
      final player = AfplayPlayer(find: (_) => null);
      await expectLater(player.open(const PcmFormat(rate: 22050)), throwsA(isA<SpeechFailure>()));
    });
  });

  group('Windows', skip: Platform.isWindows ? null : 'waveOut is Windows\'s', () {
    test('plays silence through the system output, finishes when it has played it, and can be stopped', () async {
      final player = WaveOutPlayer();
      const format = PcmFormat(rate: 22050);
      try {
        await player.open(format);
      } on SpeechFailure catch (failure) {
        markTestSkipped('This computer has no audio output: ${failure.message}');
        return;
      }
      final silence = Uint8List(bytesPerSecond(format) ~/ 5);
      final watch = Stopwatch()..start();
      await player.write(silence);
      await player.write(silence.sublist(0, silence.length - 1));
      await player.drain();
      // Two fifths of a second were played before it finished, not queued and dropped.
      expect(watch.elapsedMilliseconds, greaterThan(250));
      // A new session after the first, then stopped well before it would end.
      await player.open(format);
      await player.write(Uint8List(bytesPerSecond(format) * 2));
      watch
        ..reset()
        ..start();
      await player.stop();
      expect(watch.elapsedMilliseconds, lessThan(1000));
      await player.stop();
      await player.drain();
    });
  });
}
