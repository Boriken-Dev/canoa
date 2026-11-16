// The app's speech code against the real host's `@control`: `store_control`, an example of canoa-host, serves the control channel
// over standard streams with the canoa.cfg in its folder, and `speech_standin` is the program a provider of that file runs. A
// recorded WAV is streamed through the host and its transcript comes back; a provider's settings are written to canoa.cfg and read
// back without the key. Build it first: `cargo build -p canoa-host --bin canoa --examples`. Skipped without it.
@TestOn('vm')
@Timeout(Duration(minutes: 2))
library;

import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/voice/host_speech.dart';
import 'package:canoa/voice/speech_backend.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/control_process.dart';
import 'fixtures/examples.dart';

/// A second of 16 kHz mono silence as a WAV file holds it: a 44-byte header and then the samples.
Uint8List _recordedWav() {
  final samples = 32000;
  final wav = ByteData(44 + samples);
  void text(int at, String value) {
    for (var i = 0; i < value.length; i++) {
      wav.setUint8(at + i, value.codeUnitAt(i));
    }
  }

  text(0, 'RIFF');
  wav.setUint32(4, 36 + samples, Endian.little);
  text(8, 'WAVEfmt ');
  wav.setUint32(16, 16, Endian.little);
  wav.setUint16(20, 1, Endian.little);
  wav.setUint16(22, 1, Endian.little);
  wav.setUint32(24, 16000, Endian.little);
  wav.setUint32(28, 32000, Endian.little);
  wav.setUint16(32, 2, Endian.little);
  wav.setUint16(34, 16, Endian.little);
  text(36, 'data');
  wav.setUint32(40, samples, Endian.little);
  return wav.buffer.asUint8List();
}

/// A capture that plays a recorded WAV's samples in chunks, as a microphone would deliver them, and then ends.
final class _RecordedCapture implements AudioCapture {
  _RecordedCapture(this.wav);

  final Uint8List wav;
  int opened = 0;

  @override
  Stream<Uint8List> open({required int rate}) async* {
    opened++;
    final samples = Uint8List.sublistView(wav, 44);
    for (var at = 0; at < samples.length; at += 8192) {
      yield Uint8List.sublistView(samples, at, at + 8192 > samples.length ? samples.length : at + 8192);
    }
  }
}

void main() {
  final skip = exampleMissing('store_control') ?? exampleMissing('speech_standin') ?? exampleMissing('stdio_agent') ?? exampleMissing('version_standin');

  group('the real host', skip: skip, () {
    late Directory dir;
    late ControlProcess example;
    late HostControl control;
    late _RecordedCapture capture;
    late HostSpeechCatalog catalog;

    setUp(() async {
      dir = await Directory.systemTemp.createTemp('canoa-speech-host');
      final program = exampleBinary('speech_standin')!.replaceAll(r'\', '/');
      await File('${dir.path}/canoa.cfg').writeAsString(
        '[speech.says]\nprovider = "speech_command"\nprogram = "$program"\nargs = ["fixed"]\n',
      );
      example = ControlProcess(await Process.start(exampleBinary('store_control')!, [dir.path]));
      control = HostControl.dialing(() async => example.open());
      capture = _RecordedCapture(_recordedWav());
      catalog = HostSpeechCatalog(control, capture: capture);
    });

    tearDown(() async {
      await control.close();
      await example.process.stdin.close();
      await example.process.exitCode.timeout(const Duration(seconds: 20), onTimeout: () {
        example.process.kill();
        return -1;
      });
      try {
        await dir.delete(recursive: true);
      } on FileSystemException {
        // Windows may hold a file a moment longer.
      }
    });

    test('a recorded WAV streamed through the chosen provider comes back as its transcript', () async {
      await catalog.refresh();
      expect(catalog.problem, isNull);
      expect([for (final kind in catalog.types) kind.id], ['speech_command', 'speech_endpoint']);
      final backend = catalog.backends.single;
      expect(backend.id, 'host:says');
      expect(backend.where.place, SpeechPlace.host);

      final heard = await backend.start(language: 'en').toList();
      expect(heard, isNotEmpty);
      expect(heard.last.isFinal, isTrue);
      expect(heard.last.text, 'the quick brown fox');
      expect(heard.last.frontMatter, isA<Map<String, Object?>>());
      expect(capture.opened, 1);
    });

    test('a provider written from the form is in canoa.cfg, listed without its key, and keeps the key when saved again', () async {
      await catalog.refresh();
      await catalog.save('cloud', 'speech_endpoint', {
        'url': 'http://127.0.0.1:9/v1/audio/transcriptions',
        'model': 'whisper-1',
        'api_key': 'sk-test-0123456789',
      });
      var text = await File('${dir.path}/canoa.cfg').readAsString();
      expect(text, contains('[speech.cloud]'));
      expect(text, contains('sk-test-0123456789'));
      final cloud = catalog.providers.firstWhere((p) => p.name == 'cloud');
      expect(cloud.settings['model'], 'whisper-1');
      expect(cloud.settings.containsKey('api_key'), isFalse);
      expect(cloud.secretsSet, {'api_key'});

      // The form never shows the key, so it sends none, and the host keeps the one it has.
      await catalog.save('cloud', 'speech_endpoint', {'url': 'http://127.0.0.1:9/v1/audio/transcriptions', 'model': 'whisper-2'});
      text = await File('${dir.path}/canoa.cfg').readAsString();
      expect(text, contains('whisper-2'));
      expect(text, contains('sk-test-0123456789'));
      expect(catalog.providers.firstWhere((p) => p.name == 'cloud').secretsSet, {'api_key'});

      await catalog.remove('cloud');
      expect(catalog.providers.map((p) => p.name), ['says']);
      expect(await File('${dir.path}/canoa.cfg').readAsString(), isNot(contains('sk-test')));
      await expectLater(catalog.save('x', 'speech_command', {'nonsense': 1}), throwsA(isA<SpeechFailure>()));
    });
  });
}
