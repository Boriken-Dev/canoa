// The system speech service on Windows through the real plugin, in each kind of Flutter engine the app runs: the main window's, and a
// second window's (a pop-out window is a second engine of the same program). A recording the system's own voice makes is played in
// place of the microphone (the plugin's inputFile option), so nothing opens the microphone.
//
//   flutter drive --driver=test_driver/integration_test.dart --target=integration_test/system_speech_test.dart -d windows
//
// Not `flutter test`: its generated entry waits for the test server before it calls `main`, and a second engine inherits that entry,
// so the window's `main` never runs. `flutter drive` runs this file's `main` in every engine. The window's engine runs it with the
// arguments the plugin gives it: the recording to dictate and the file to write what it heard to.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/voice/speech_backend.dart';
import 'package:canoa/voice/system_speech.dart';
import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
// ignore: depend_on_referenced_packages
import 'package:speech_to_text_platform_interface/speech_to_text_platform_interface.dart' show SpeechConfigOption, SpeechToTextPlatform;
// ignore: depend_on_referenced_packages
import 'package:speech_to_text_windows/speech_to_text_windows.dart' show SpeechToTextWindows;
import 'package:window_manager/window_manager.dart';

const _words = 'hello world this is a test of dictation';

Future<void> main(List<String> args) async {
  // The registrant has run before `main`, in whichever engine this is.
  if (args.length >= 3 && args.first == 'multi_window') return _window(args[2]);
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

  testWidgets('a second window dictates a recording through the Windows plugin', (tester) async {
    await tester.runAsync(() async {
      final heard = File('${dir.path}${Platform.pathSeparator}window.json');
      final window = await WindowController.create(
        WindowConfiguration(arguments: jsonEncode({'wav': recorded.path, 'out': heard.path}), hiddenAtLaunch: true),
      );
      addTearDown(() => window.hide().then<void>((_) {}, onError: (Object _) {}));
      final end = DateTime.now().add(const Duration(minutes: 2));
      while (!heard.existsSync()) {
        if (DateTime.now().isAfter(end)) fail('the second window never said what it heard');
        await Future<void>.delayed(const Duration(milliseconds: 200));
      }
      final said = jsonDecode(await heard.readAsString()) as Map<String, dynamic>;
      expect(said['platform'], 'SpeechToTextWindows', reason: 'the plugin registers itself in the second window before its main runs');
      expect(said['error'], isNull);
      expect((said['text'] as String).toLowerCase(), contains('hello world'));
    });
  }, timeout: const Timeout(Duration(minutes: 3)));
}

/// What a second window does: dictates the recording and writes what the engine, the words or the failure came to.
Future<void> _window(String launch) async {
  WidgetsFlutterBinding.ensureInitialized();
  final asked = jsonDecode(launch) as Map<String, dynamic>;
  final said = <String, Object?>{'platform': SpeechToTextPlatform.instance.runtimeType.toString()};
  try {
    said['text'] = await _dictate(asked['wav'] as String);
  } on Object catch (error) {
    said['error'] = error is SpeechFailure ? error.message : '$error';
  }
  await File(asked['out'] as String).writeAsString(jsonEncode(said));
  try {
    await windowManager.ensureInitialized();
    await windowManager.close();
  } on Object {
    // The window is hidden and the test hides it too.
  }
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
