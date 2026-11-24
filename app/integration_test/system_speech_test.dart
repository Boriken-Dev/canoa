// The system speech service on Windows through the real plugin. A recording the system's own voice makes is played in place of the
// microphone (the plugin's inputFile option), so nothing opens the microphone.
//
//   flutter test integration_test/system_speech_test.dart -d windows
import 'dart:async';
import 'dart:io';

import 'package:canoa/voice/speech_backend.dart';
import 'package:canoa/voice/system_speech.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
// ignore: depend_on_referenced_packages
import 'package:speech_to_text_platform_interface/speech_to_text_platform_interface.dart' show SpeechConfigOption, SpeechToTextPlatform;
// ignore: depend_on_referenced_packages
import 'package:speech_to_text_windows/speech_to_text_windows.dart' show SpeechToTextWindows;

const _words = 'hello world this is a test of dictation';

Future<void> main() async {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  late Directory dir;
  late File recorded;
  setUpAll(() async {
    dir = await Directory.systemTemp.createTemp('canoa_speech_');
    recorded = await _record(dir, _words);
  });
  tearDownAll(() => dir.delete(recursive: true));

  testWidgets('the main window dictates a recording through the Windows plugin', (tester) async {
    await tester.runAsync(() async {
      expect(SpeechToTextPlatform.instance, isA<SpeechToTextWindows>(), reason: 'the plugin registers itself before main runs');
      expect((await _dictate(recorded.path)).toLowerCase(), contains('hello world'));
    });
  }, timeout: const Timeout(Duration(minutes: 3)));
}

/// The words the system heard in [wav], which plays in real time; the dictation is stopped once it has played.
Future<String> _dictate(String wav) async {
  final service = SystemSpeech('Windows speech recognition', options: [SpeechConfigOption('windows', 'inputFile', wav)]);
  final heard = <Transcript>[];
  final done = Completer<void>();
  final sub = service.start().listen(
    heard.add,
    onError: (Object error) {
      if (!done.isCompleted) done.completeError(error);
    },
    onDone: () {
      if (!done.isCompleted) done.complete();
    },
  );
  try {
    await Future.any([Future<void>.delayed(const Duration(seconds: 8)), done.future]);
    await service.stop();
    await done.future.timeout(const Duration(seconds: 20));
  } finally {
    await sub.cancel();
  }
  if (heard.isEmpty || !heard.last.isFinal) throw StateError('the system returned no final text: ${heard.map((t) => t.text)}');
  return heard.last.text;
}

/// Writes [words], spoken by the system's own voice, as a 16 kHz mono WAV.
Future<File> _record(Directory into, String words) async {
  final file = File('${into.path}${Platform.pathSeparator}words.wav');
  final script = File('${into.path}${Platform.pathSeparator}record.ps1');
  await script.writeAsString(
    'param([string]\$Out, [string]\$Text)\n'
    'Add-Type -AssemblyName System.Speech\n'
    '\$s = New-Object System.Speech.Synthesis.SpeechSynthesizer\n'
    '\$f = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)\n'
    '\$s.SetOutputToWaveFile(\$Out, \$f)\n'
    '\$s.Speak(\$Text)\n'
    '\$s.Dispose()\n',
  );
  final run = await Process.run('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script.path, file.path, words]);
  if (run.exitCode != 0 || !file.existsSync()) fail('the system voice could not record the words: ${run.stderr}');
  return file;
}
