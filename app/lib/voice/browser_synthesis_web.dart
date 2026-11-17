import 'dart:async';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'package:web/web.dart' as web;

import 'browser_voices.dart';
import 'speech_backend.dart' show SpeechFailure;

/// The page's speech synthesis, or null in a browser that has none.
BrowserSynthesis? create() => web.window.has('speechSynthesis') ? _PageSynthesis() : null;

/// How long a browser that fills its voice list late is waited for, in milliseconds.
const _loadMs = 2000;

final class _PageSynthesis implements BrowserSynthesis {
  web.SpeechSynthesis get _engine => web.window.speechSynthesis;

  @override
  Future<List<BrowserVoice>> voices() async {
    List<BrowserVoice> now() => [
      for (final voice in _engine.getVoices().toDart)
        BrowserVoice(uri: voice.voiceURI, name: voice.name, lang: voice.lang, localService: voice.localService, isDefault: voice.default_),
    ];
    final listed = now();
    if (listed.isNotEmpty) return listed;
    // Chrome lists no voice until it has loaded them and says so with `voiceschanged`.
    final loaded = Completer<void>();
    void onChanged(web.Event _) {
      if (!loaded.isCompleted) loaded.complete();
    }

    final listener = onChanged.toJS;
    _engine.addEventListener('voiceschanged', listener);
    try {
      await loaded.future.timeout(const Duration(milliseconds: _loadMs));
    } on TimeoutException {
      // None came: this browser has none.
    } finally {
      _engine.removeEventListener('voiceschanged', listener);
    }
    return now();
  }

  @override
  BrowserSpeaking speak(String text, {String? voiceUri, String? lang, double rate = 1}) {
    final utterance = web.SpeechSynthesisUtterance(text)..rate = rate;
    if (lang != null && lang.isNotEmpty) utterance.lang = lang;
    if (voiceUri != null) {
      for (final voice in _engine.getVoices().toDart) {
        if (voice.voiceURI == voiceUri) {
          utterance.voice = voice;
          utterance.lang = voice.lang;
          break;
        }
      }
    }
    return _PageSpeaking(_engine, utterance);
  }
}

final class _PageSpeaking implements BrowserSpeaking {
  _PageSpeaking(this._engine, this._utterance) {
    _done.future.ignore();
    _utterance.onend = ((web.Event _) => _finish()).toJS;
    _utterance.onerror = ((web.SpeechSynthesisErrorEvent event) {
      // Cancelling a text is how a page stops one, and the browser reports it as an error that is not one.
      final why = event.error;
      if (_cancelled || why == 'canceled' || why == 'interrupted') {
        _finish();
      } else if (!_done.isCompleted) {
        _done.completeError(SpeechFailure('The browser could not read the text aloud ($why).'));
      }
    }).toJS;
    _engine.speak(_utterance);
  }

  final web.SpeechSynthesis _engine;
  final web.SpeechSynthesisUtterance _utterance;
  final _done = Completer<void>();
  bool _cancelled = false;

  void _finish() {
    if (!_done.isCompleted) _done.complete();
  }

  @override
  Future<void> get done => _done.future;

  @override
  void cancel() {
    _cancelled = true;
    _engine.cancel();
    _finish();
  }
}
