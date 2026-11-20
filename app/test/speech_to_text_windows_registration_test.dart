// The Windows speech plugin registers itself the way the generated registrant does: before the app's `main` has created Flutter's
// bindings, so nothing in registration may touch a channel. This file never initializes the binding, which the plugin's other tests do.

import 'package:flutter_test/flutter_test.dart';
// ignore: depend_on_referenced_packages
import 'package:speech_to_text_platform_interface/speech_to_text_platform_interface.dart' show SpeechToTextPlatform;
// ignore: depend_on_referenced_packages
import 'package:speech_to_text_windows/speech_to_text_windows.dart';

void main() {
  test('registering before the bindings exist makes the plugin the platform instance', () {
    expect(SpeechToTextPlatform.instance, isNot(isA<SpeechToTextWindows>()));
    SpeechToTextWindows.registerWith();
    expect(SpeechToTextPlatform.instance, isA<SpeechToTextWindows>());
  });
}
