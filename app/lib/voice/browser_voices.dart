import 'dart:async';
import 'dart:typed_data';

import 'browser_synthesis.dart';
import 'speech_backend.dart' show SpeechFailure;
import 'speech_output.dart';

/// One voice a browser lists.
final class BrowserVoice {
  const BrowserVoice({required this.uri, required this.name, required this.lang, required this.localService, this.isDefault = false});

  /// What names it: the browser's own `voiceURI`.
  final String uri, name;

  /// Its language, as a BCP 47 tag.
  final String lang;

  /// Whether it runs on this device. A voice that is not local sends the text to a server of the browser's maker.
  final bool localService;
  final bool isDefault;
}

/// A text being spoken by the browser, which plays it itself.
abstract interface class BrowserSpeaking {
  /// Completes when the browser has finished, or fails with a [SpeechFailure].
  Future<void> get done;

  void cancel();
}

/// The browser's speech synthesis, behind what the app needs of it, so the choice of voices does not depend on a browser.
abstract interface class BrowserSynthesis {
  /// The voices the browser lists once it has them; some browsers list none until they have loaded, which [voices] waits for.
  Future<List<BrowserVoice>> voices();

  /// Speaks [text] with the voice [voiceUri] (the browser's default when null) at [rate] times normal, in [lang] when given.
  BrowserSpeaking speak(String text, {String? voiceUri, String? lang, double rate = 1});
}

/// The browser's own voices, as two ways of speaking: the ones that run on this device, whose text stays on it, and the ones that do
/// not, which the browser's maker speaks from a server and which are listed with where the text goes. A page lists the second only
/// to say what they are; choosing one is the person's.
final class BrowserSpeech implements SpeechOutput {
  BrowserSpeech(this._synthesis, {required this.online});

  final BrowserSynthesis _synthesis;

  /// Whether this is the voices that are not on the device.
  final bool online;

  @override
  String get id => online ? 'browser:online' : 'browser';

  @override
  String? get voicesProblem => null;

  @override
  String get name => online ? "The browser's online voices" : "The browser's own voices";

  @override
  OutputWhere get where => online
      ? const OutputWhere(
          leaves: TextLeaves.yes,
          words: "These voices run on a server of the browser's maker: the browser sends the text there to have it spoken, and what that "
              'server keeps is its own.',
        )
      : const OutputWhere(
          leaves: TextLeaves.no,
          words: "The voices installed on this device speak it, through the browser; the text is sent nowhere.",
        );

  @override
  List<OutputChoice> get models => const [];

  @override
  String? get defaultModel => null;

  @override
  List<OutputChoice> get styles => const [];

  @override
  SpeedRange? get speed => const SpeedRange(0.5, 2);

  @override
  Set<String> get uses => const {'voice', 'speed', 'language'};

  Future<List<BrowserVoice>> _mine() async => [
    for (final voice in await _synthesis.voices())
      if (voice.localService != online) voice,
  ];

  @override
  Future<List<OutputVoice>> voices({String? language}) async {
    String primary(String tag) => tag.split(RegExp('[-_]')).first.toLowerCase();
    final wanted = language == null || language.isEmpty ? null : primary(language);
    return [
      for (final voice in await _mine())
        if (wanted == null || primary(voice.lang) == wanted)
          OutputVoice(id: voice.uri, name: voice.name, language: voice.lang.replaceAll('_', '-')),
    ];
  }

  @override
  SpeechUtterance speak(String text, {SpeakChoices choices = const SpeakChoices()}) => _BrowserUtterance(this, text, choices);
}

final class _BrowserUtterance implements SpeechUtterance {
  _BrowserUtterance(this._speech, this._text, this._choices) {
    _done.future.ignore();
    unawaited(_run());
  }

  final BrowserSpeech _speech;
  final String _text;
  final SpeakChoices _choices;
  final _done = Completer<void>();
  BrowserSpeaking? _speaking;
  bool _stopped = false;

  @override
  Future<PcmFormat?> get format async => null;

  @override
  Stream<Uint8List> get audio => const Stream.empty();

  @override
  Future<void> get done => _done.future;

  Future<void> _run() async {
    try {
      final mine = await _speech._mine();
      if (_stopped) return _end();
      String? uri;
      final wanted = _choices.voice;
      if (wanted != null) {
        final found = mine.where((voice) => voice.uri == wanted);
        if (found.isEmpty) {
          throw SpeechFailure(
            _speech.online
                ? 'This browser has no online voice $wanted.'
                : 'This browser has no voice $wanted on this device. A voice that is not on the device is one of the online voices, '
                      'which send the text to the browser\'s maker.',
          );
        }
        uri = found.first.uri;
      } else if (_choices.language != null) {
        String primary(String tag) => tag.split(RegExp('[-_]')).first.toLowerCase();
        final language = primary(_choices.language!);
        final same = mine.where((voice) => primary(voice.lang) == language);
        if (same.isNotEmpty) uri = same.first.uri;
      }
      if (uri == null && mine.isEmpty) {
        throw SpeechFailure(
          _speech.online ? 'This browser lists no online voices.' : 'This browser lists no voices on this device.',
        );
      }
      final speed = _choices.speed == null ? 1.0 : _speech.speed!.clamp(_choices.speed!);
      final speaking = _speech._synthesis.speak(_text, voiceUri: uri ?? mine.first.uri, lang: _choices.language, rate: speed);
      _speaking = speaking;
      if (_stopped) {
        speaking.cancel();
        return _end();
      }
      await speaking.done;
      _end();
    } on SpeechFailure catch (failure) {
      if (!_done.isCompleted) _done.completeError(failure);
    } on Object {
      if (!_done.isCompleted) _done.completeError(const SpeechFailure("The browser could not read the text aloud."));
    }
  }

  void _end() {
    if (!_done.isCompleted) _done.complete();
  }

  @override
  Future<void> stop() async {
    _stopped = true;
    _speaking?.cancel();
    _end();
  }
}

/// The browser's voices as ways of speaking, when this build runs in a browser that has speech synthesis; empty elsewhere.
List<SpeechOutput> browserSpeechOutputs({BrowserSynthesis? synthesis}) {
  final browser = synthesis ?? platformBrowserSynthesis();
  if (browser == null) return const [];
  return [BrowserSpeech(browser, online: false), BrowserSpeech(browser, online: true)];
}
