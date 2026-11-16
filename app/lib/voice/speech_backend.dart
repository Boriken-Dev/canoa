import 'dart:async';

/// Where a speech service runs.
enum SpeechPlace {
  /// On this device, with no network involved.
  thisDevice('This device'),

  /// On a host the person added.
  host('A host'),

  /// A named service on the network, under an account or endpoint of its own.
  cloud('A named cloud service'),

  /// The operating system's service, which decides for itself whether it asks a server.
  system('The operating system');

  const SpeechPlace(this.label);
  final String label;
}

/// Whether the audio leaves this device while it is being turned into text.
enum AudioLeaves { no, maybe, yes }

/// Where a backend turns speech into text, in words the page can show before anything is recorded.
final class SpeechWhere {
  const SpeechWhere({required this.place, required this.leaves, required this.words, this.endpoint});

  final SpeechPlace place;
  final AudioLeaves leaves;

  /// One or two sentences saying what happens to the audio.
  final String words;

  /// The account or address the audio goes to, when the backend has one of its own; null otherwise.
  final String? endpoint;
}

/// One thing a backend offers to choose from: a model, a language or an input device.
final class SpeechChoice {
  const SpeechChoice(this.id, this.label);
  final String id, label;
}

/// What a backend has heard so far. A provisional transcript is replaced by the next one; a final one ends a phrase.
final class Transcript {
  const Transcript(this.text, {required this.isFinal});
  final String text;
  final bool isFinal;
}

/// Why a capture ended without a result the person asked for. [message] says it in words and never holds the audio or a key.
final class SpeechFailure implements Exception {
  const SpeechFailure(this.message);
  final String message;

  @override
  String toString() => message;
}

/// A way of turning the person's speech into text. Nothing here listens until [start] is called, and a failed backend
/// reports its failure: the app never hands the same audio to another one.
abstract interface class SpeechBackend {
  /// The id `[voice] backend` names.
  String get id;

  /// What Settings calls it.
  String get name;

  SpeechWhere get where;

  /// The models the backend has when it has several, else empty. Does not open the microphone.
  Future<List<SpeechChoice>> models();

  /// The languages it can tell apart, as far as it knows before a first capture; empty when it cannot say.
  Future<List<SpeechChoice>> languages();

  /// The input devices it can use besides the system's default; empty when it records from the default only.
  Future<List<SpeechChoice>> devices();

  /// Opens the microphone and turns what it hears into text until [stop] or [cancel], or the service ends the capture. The
  /// stream closes when the capture is over and carries a [SpeechFailure] when it failed.
  Stream<Transcript> start({String? device, String? language, String? model});

  /// How loud the input is, from 0 to 1, while a capture runs.
  Stream<double> get levels;

  /// Ends the capture and lets the service finish the phrase it was hearing.
  Future<void> stop();

  /// Ends the capture and drops what the service had not finished.
  Future<void> cancel();
}
