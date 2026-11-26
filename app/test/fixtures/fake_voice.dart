import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/voice/pcm_player.dart';
import 'package:canoa/voice/speech_output.dart';

/// A text being spoken by a stand-in: its audio is the text's own bytes, in one piece, and it ends at once, unless the test holds it.
final class FakeUtterance implements SpeechUtterance {
  FakeUtterance(this.text, this.choices, {this.hold = false, this.failWith, this.pcm = const PcmFormat(rate: 22050)}) {
    _done.future.ignore();
    _formatDone.future.ignore();
    scheduleMicrotask(() {
      final failure = failWith;
      if (failure != null) {
        _formatDone.completeError(failure);
        _audio.addError(failure);
        unawaited(_audio.close());
        _done.completeError(failure);
        return;
      }
      _formatDone.complete(pcm);
      if (!hold) finish();
    });
  }

  final String text;
  final SpeakChoices choices;
  final bool hold;
  final Object? failWith;

  /// The audio's format, or null for a provider that plays by itself.
  final PcmFormat? pcm;
  bool stopped = false;

  final _audio = StreamController<Uint8List>();
  final _formatDone = Completer<PcmFormat?>();
  final _done = Completer<void>();

  /// Delivers the audio and ends, for an utterance the test held.
  void finish() {
    if (_done.isCompleted) return;
    if (pcm != null) _audio.add(Uint8List.fromList(utf8.encode(text)));
    unawaited(_audio.close());
    _done.complete();
  }

  @override
  Future<PcmFormat?> get format => _formatDone.future;

  @override
  Stream<Uint8List> get audio => _audio.stream;

  @override
  Future<void> get done => _done.future;

  @override
  Future<void> stop() async {
    stopped = true;
    if (!_audio.isClosed) unawaited(_audio.close());
    if (!_done.isCompleted) _done.complete();
  }
}

/// A voice provider that speaks nothing and keeps what it was asked: each text, with the choices that came with it.
final class FakeOutput implements SpeechOutput {
  FakeOutput(
    this.id,
    this.name, {
    this.where = const OutputWhere(leaves: TextLeaves.no, words: 'The text stays on this device.'),
    this.models = const [],
    this.defaultModel,
    this.styles = const [],
    this.speed,
    this.uses = const {'voice'},
    this.voiceList = const [],
    this.voicesProblem,
    this.hold = false,
    this.failWith,
    this.pcm = const PcmFormat(rate: 22050),
  });

  @override
  final String id, name;
  @override
  final OutputWhere where;
  @override
  final List<OutputChoice> models, styles;
  @override
  final String? defaultModel;
  @override
  final SpeedRange? speed;
  @override
  final Set<String> uses;
  final List<OutputVoice> voiceList;
  @override
  final String? voicesProblem;

  /// Whether each text is held until the test finishes it.
  bool hold;

  /// What an utterance ends with, when it should fail.
  Object? failWith;
  PcmFormat? pcm;

  final utterances = <FakeUtterance>[];

  /// The texts asked for, in order.
  List<String> get spoken => [for (final each in utterances) each.text];

  @override
  Future<List<OutputVoice>> voices({String? language}) async => voiceList;

  @override
  SpeechUtterance speak(String text, {SpeakChoices choices = const SpeakChoices()}) {
    final utterance = FakeUtterance(text, choices, hold: hold, failWith: failWith, pcm: pcm);
    utterances.add(utterance);
    return utterance;
  }
}

/// A player that plays nothing and keeps what it was given, as text.
final class FakePlayer implements PcmPlayer {
  final events = <String>[];
  final written = <String>[];
  PcmFormat? opened;
  int unlocks = 0;

  @override
  void unlock() => unlocks++;

  @override
  Future<void> open(PcmFormat format) async {
    opened = format;
    events.add('open ${format.rate}');
  }

  @override
  Future<void> write(Uint8List pcm) async {
    written.add(utf8.decode(pcm));
    events.add('write');
  }

  @override
  Future<void> drain() async {
    opened = null;
    events.add('drain');
  }

  @override
  Future<void> stop() async {
    opened = null;
    events.add('stop');
  }
}
