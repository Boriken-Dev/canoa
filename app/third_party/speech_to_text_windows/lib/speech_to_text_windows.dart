import 'dart:async';
import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:speech_to_text_platform_interface/speech_to_text_platform_interface.dart';

export 'package:speech_to_text_platform_interface/speech_to_text_platform_interface.dart' show SpeechListenOptions;

/// Windows implementation of the speech_to_text plugin over the desktop speech recognizer (SAPI 5).
///
/// The native side reports every session as text events the platform interface already understands: the words heard so far as a
/// partial result, then one final result when the session ends, and the statuses `listening`, `notListening` and `done`.
class SpeechToTextWindows extends SpeechToTextPlatform {
  static const MethodChannel _channel = MethodChannel('speech_to_text_windows');

  /// Registers this class as the default instance of [SpeechToTextPlatform].
  static void registerWith() {
    SpeechToTextPlatform.instance = SpeechToTextWindows();
  }

  /// Touches no channel: the generated registrant calls [registerWith] before the app's `main` has created Flutter's bindings, and
  /// a channel's handler cannot be set until they exist. [initialize] sets it.
  SpeechToTextWindows();

  /// A failure of a call to the native side reaches the listener as an error of the session, not as a silent `false`.
  void _fail(String code) {
    onError?.call(jsonEncode({'errorMsg': code, 'permanent': true}));
  }

  @override
  Future<bool> hasPermission() async {
    try {
      return await _channel.invokeMethod<bool>('hasPermission') ?? false;
    } on Object {
      return false;
    }
  }

  @override
  Future<bool> initialize({debugLogging = false, List<SpeechConfigOption>? options}) async {
    _channel.setMethodCallHandler(_handleMethodCall);
    final params = <String, dynamic>{'debugLogging': debugLogging};
    for (final option in options ?? const <SpeechConfigOption>[]) {
      if (option.platform == 'windows') params[option.name] = option.value;
    }
    try {
      return await _channel.invokeMethod<bool>('initialize', params) ?? false;
    } on Object {
      _fail('error_initialize');
      return false;
    }
  }

  @override
  Future<bool> listen({
    String? localeId,
    @Deprecated('Use SpeechListenOptions.partialResults instead') partialResults = true,
    @Deprecated('Use SpeechListenOptions.onDevice instead') onDevice = false,
    @Deprecated('Use SpeechListenOptions.listenMode instead') int listenMode = 0,
    @Deprecated('Use SpeechListenOptions.sampleRate instead') sampleRate = 0,
    SpeechListenOptions? options,
  }) async {
    final params = <String, dynamic>{
      'localeId': localeId,
      'partialResults': options?.partialResults ?? partialResults,
      'onDevice': options?.onDevice ?? onDevice,
      'listenMode': options?.listenMode.index ?? listenMode,
      'sampleRate': options?.sampleRate ?? sampleRate,
      'autoPunctuation': options?.autoPunctuation ?? false,
      'enableHapticFeedback': options?.enableHapticFeedback ?? false,
      'cancelOnError': options?.cancelOnError ?? false,
    };
    try {
      final started = await _channel.invokeMethod<bool>('listen', params) ?? false;
      if (!started) _fail('error_listen');
      return started;
    } on Object {
      _fail('error_listen');
      return false;
    }
  }

  @override
  Future<void> stop() async {
    try {
      await _channel.invokeMethod<void>('stop');
    } on Object {
      _fail('error_stop');
    }
  }

  @override
  Future<void> cancel() async {
    try {
      await _channel.invokeMethod<void>('cancel');
    } on Object {
      _fail('error_cancel');
    }
  }

  @override
  Future<List<dynamic>> locales() async {
    try {
      return await _channel.invokeMethod<List<dynamic>>('locales') ?? const [];
    } on Object {
      return const [];
    }
  }

  Future<void> _handleMethodCall(MethodCall call) async {
    final argument = call.arguments;
    if (argument is! String) return;
    switch (call.method) {
      case 'textRecognition':
        onTextRecognition?.call(argument);
      case 'notifyError':
        onError?.call(argument);
      case 'notifyStatus':
        onStatus?.call(argument);
    }
  }
}
