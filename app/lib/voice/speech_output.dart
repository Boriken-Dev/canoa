import 'dart:async';
import 'dart:typed_data';

import 'speech_backend.dart' show SpeechFailure;

/// Whether the text leaves the computer the voice runs on while it is turned into speech.
enum TextLeaves { no, maybe, yes }

/// Where a voice provider turns text into audio, in words the page can show before anything is spoken.
final class OutputWhere {
  const OutputWhere({required this.leaves, required this.words, this.endpoint});

  final TextLeaves leaves;

  /// One or two sentences saying what happens to the text.
  final String words;

  /// The address the text goes to, when it goes to one; null otherwise.
  final String? endpoint;
}

/// One thing a voice provider offers to choose from: a model or a style.
final class OutputChoice {
  const OutputChoice(this.id, this.label, {this.description});
  final String id, label;
  final String? description;
}

/// One voice a provider can speak with.
final class OutputVoice {
  const OutputVoice({required this.id, required this.name, this.language, this.gender, this.description, this.models = const []});

  /// What a request names.
  final String id, name;

  /// The language it speaks, as a BCP 47 tag; null when it speaks several or the provider does not say.
  final String? language;

  /// `female`, `male` or `neutral`, when the provider says.
  final String? gender;
  final String? description;

  /// The models it works with; empty says every model the provider lists.
  final List<String> models;

  bool worksWith(String? model) => models.isEmpty || model == null || models.contains(model);

  static OutputVoice? parse(Object? json) {
    if (json is! Map || json['id'] is! String) return null;
    return OutputVoice(
      id: json['id']! as String,
      name: json['name'] is String ? json['name']! as String : json['id']! as String,
      language: json['language'] is String ? json['language']! as String : null,
      gender: json['gender'] is String ? json['gender']! as String : null,
      description: json['description'] is String ? json['description']! as String : null,
      models: [if (json['models'] is List) ...(json['models']! as List).whereType<String>()],
    );
  }
}

/// The speeds a provider takes, as multiples of normal speech.
final class SpeedRange {
  const SpeedRange(this.min, this.max);
  final double min, max;
  double clamp(double speed) => speed < min ? min : (speed > max ? max : speed);
}

/// The format of the audio an utterance delivers: 16-bit little-endian PCM at [rate] samples a second.
final class PcmFormat {
  const PcmFormat({required this.rate, this.channels = 1});
  final int rate, channels;

  @override
  bool operator ==(Object other) => other is PcmFormat && other.rate == rate && other.channels == channels;

  @override
  int get hashCode => Object.hash(rate, channels);
}

/// What a person chose for one thing to be read: every part is optional and a provider uses what it has.
final class SpeakChoices {
  const SpeakChoices({this.voice, this.model, this.style, this.speed, this.language});
  final String? voice, model, style, language;

  /// A multiple of normal speech, 1 being normal.
  final double? speed;
}

/// One text being spoken. A provider that hands audio back delivers it in [audio], in order, for the app to play; one that plays by
/// itself (a browser's voices) has a null [format] and an empty [audio]. [done] completes when everything was delivered or played,
/// or ends with a [SpeechFailure] saying why it could not.
abstract interface class SpeechUtterance {
  /// The audio's format once the provider has begun, or null when it plays the audio itself. Ends with a [SpeechFailure] when the
  /// provider refused to begin.
  Future<PcmFormat?> get format;

  /// The audio, in order; closes when the text has been spoken.
  Stream<Uint8List> get audio;

  /// Completes when the utterance is over, with a [SpeechFailure] when it ended in one. Stopping completes it without one.
  Future<void> get done;

  /// Ends it: what was not yet spoken is dropped.
  Future<void> stop();
}

/// A way of turning text into speech. Nothing is spoken before [speak] is called, and a provider that fails reports its failure: the
/// app never hands the same text to another one.
abstract interface class SpeechOutput {
  /// What a setting names: `host:NAME` for a provider of the host, `browser` for the browser's own voices.
  String get id;

  /// What Settings calls it.
  String get name;

  OutputWhere get where;

  /// The models it offers when it has several, else empty.
  List<OutputChoice> get models;

  /// The model it uses when none is chosen.
  String? get defaultModel;

  /// The styles it offers, else empty.
  List<OutputChoice> get styles;

  /// The speeds it takes; null when it has one speed.
  SpeedRange? get speed;

  /// Whether a request's `voice`, `model`, `style`, `speed` and `language` have an effect, so a form offers only those.
  Set<String> get uses;

  /// The voices it has now, for [language] (a BCP 47 tag or its first part) when given.
  Future<List<OutputVoice>> voices({String? language});

  /// Starts speaking [text]. The choices a provider does not list are refused with a [SpeechFailure] on [SpeechUtterance.done].
  SpeechUtterance speak(String text, {SpeakChoices choices = const SpeakChoices()});
}
