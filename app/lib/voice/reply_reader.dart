import 'dart:async';
import 'dart:collection';

import 'package:flutter/foundation.dart';

import 'pcm_player.dart';
import 'speakable.dart';
import 'speech_backend.dart' show SpeechFailure;
import 'speech_output.dart';

/// The voice and choices a reply is read with at the moment it starts.
typedef ReadingVoice = ({SpeechOutput output, SpeakChoices choices});

/// One reply of a conversation as the reader sees it while it follows a turn: the key it keeps, its text so far, and whether it is
/// whole (its turn is over, or something came after it).
typedef ReplyText = ({String key, String text, bool complete});

/// Reads replies aloud: one the person asked for (Play), and, in voice mode, each reply of the conversation a sentence at a time as
/// it streams in. A reply is read as its prose in order, one sentence after another, through the voice and choices Settings holds;
/// audio the provider hands back is played here as it arrives, one the provider plays itself (a browser's) is waited for.
///
/// Reading stops at once when [stop] is called, and nothing that was not yet spoken is spoken later. A provider that fails stops the
/// reading and [error] says why: the text is never handed to another provider.
final class ReplyReader extends ChangeNotifier {
  ReplyReader({required this.voice, this.player, this.whyNone, this.offered});

  /// The voice to read with, at the moment a reading starts; null when there is none, which [whyNone] says in words.
  final Future<ReadingVoice?> Function() voice;

  /// Plays what a provider hands back as audio; null where this build has no way to.
  final PcmPlayer? player;

  /// Why there is no [voice], when there is a better reason than "none is set up".
  final String? Function()? whyNone;

  /// Whether this app or page can read aloud at all; false leaves the play buttons out. Always, when null.
  final bool Function()? offered;

  bool get available => offered?.call() ?? true;

  // The sentences waiting, in the order they are read, and the one being read.
  final _queue = Queue<_Sentence>();
  _Sentence? _current;
  int _generation = 0;
  int _workerGeneration = -1;
  bool _playerOpen = false;
  bool _disposed = false;
  bool _working = false;
  Completer<void>? _wake;
  String? _error;
  String? _errorKey;

  // Voice mode: what is followed, and what is not read because the person stopped it.
  bool _voiceMode = false;
  final _followed = <String, SentenceCutter>{};
  final _heard = <String>{};
  final _muted = <String>{};
  String? _session;

  /// The reply being read now, by the key it was given; null when nothing is read.
  String? get reading => _current?.key ?? (_queue.isEmpty ? null : _queue.first.key);

  /// Whether anything is being read or waits to be.
  bool get busy => _working || _queue.isNotEmpty;

  /// Whether the reply [key] is being read or waits to be.
  bool isReading(String key) => _current?.key == key || _queue.any((each) => each.key == key);

  /// Why the last reading failed, in words, and the key of the reply it was for; null for a reply that voice mode was reading. Kept
  /// until [clearError].
  String? get error => _error;
  String? get errorKey => _errorKey;

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  /// Ends any reading, and nothing is told after it.
  @override
  void dispose() {
    stop();
    _disposed = true;
    super.dispose();
  }

  void clearError() {
    if (_error == null) return;
    _error = null;
    _errorKey = null;
  }

  /// Whether voice mode is on: each reply is read as it arrives.
  bool get voiceMode => _voiceMode;

  /// Turns voice mode on or off. Turning it on reads no earlier reply; turning it off stops what is being read.
  void setVoiceMode(bool on) {
    if (_voiceMode == on) return;
    _voiceMode = on;
    _followed.clear();
    _heard.clear();
    _muted.clear();
    _session = null;
    if (!on) stop();
    notifyListeners();
  }

  /// Gets the audio output ready from inside the press that will lead to speech, which a browser needs.
  void unlock() => player?.unlock();

  /// Reads the reply [key], whose text is [markdown], from its start; reading the one that is being read stops it. Whatever else was
  /// being read stops first.
  Future<void> toggle(String key, String markdown) async {
    if (isReading(key)) {
      stop();
      return;
    }
    unlock();
    stop();
    clearError();
    _muted.add(key);
    for (final sentence in sentencesOf(markdown)) {
      _queue.add(_Sentence(key, sentence));
    }
    _begin();
  }

  /// Ends every reading at once and drops what had not been said; in voice mode the reply being followed is not read any more.
  void stop() {
    _generation++;
    final dropping = [?_current, ..._queue];
    _queue.clear();
    _current = null;
    _muted.addAll(_followed.keys);
    _followed.clear();
    for (final sentence in dropping) {
      unawaited(sentence.utterance?.stop());
    }
    if (_playerOpen) {
      _playerOpen = false;
      unawaited(player?.stop());
    }
    _poke();
    notifyListeners();
  }

  /// Voice mode is told what the conversation [session] holds now: its replies in order, each by a key that does not change. Each
  /// reply that is new since voice mode was turned on is read, a sentence at a time, as its text grows; the replies already there
  /// are not.
  void observe(String session, List<ReplyText> replies) {
    if (!_voiceMode) return;
    if (_session != session) {
      // A conversation that was not followed: what it already says is not read.
      _session = session;
      _heard
        ..clear()
        ..addAll([for (final reply in replies) reply.key]);
      _followed.clear();
      _muted.clear();
      return;
    }
    var added = false;
    for (final reply in replies) {
      if (_heard.contains(reply.key) || _muted.contains(reply.key)) continue;
      final cutter = _followed.putIfAbsent(reply.key, SentenceCutter.new);
      for (final sentence in cutter.feed(reply.text, last: reply.complete)) {
        _queue.add(_Sentence(reply.key, sentence, followed: true));
        added = true;
      }
      if (cutter.finished) {
        _followed.remove(reply.key);
        _heard.add(reply.key);
      }
    }
    if (added) {
      clearError();
      _begin();
    }
  }

  /// Whether a reply of voice mode is still being written, so reading waits for more of it rather than ending.
  bool get _waitingForMore => _voiceMode && _followed.isNotEmpty;

  void _begin() {
    notifyListeners();
    if (_working && _workerGeneration == _generation) {
      _poke();
      return;
    }
    _working = true;
    _workerGeneration = _generation;
    unawaited(_work(_generation));
  }

  void _poke() {
    final wake = _wake;
    _wake = null;
    if (wake != null && !wake.isCompleted) wake.complete();
  }

  Future<_Sentence?> _next(int generation) async {
    while (generation == _generation) {
      if (_queue.isNotEmpty) return _queue.removeFirst();
      if (!_waitingForMore) return null;
      await (_wake ??= Completer<void>()).future;
    }
    return null;
  }

  /// The two sentences ahead are being turned into speech while the one before is played, so there is no gap between them.
  void _prepare(ReadingVoice chosen) {
    var ahead = 0;
    for (final sentence in _queue) {
      sentence.utterance ??= chosen.output.speak(sentence.text, choices: chosen.choices);
      if (++ahead == 1) break;
    }
  }

  Future<void> _work(int generation) async {
    try {
      final chosen = await voice();
      if (chosen == null) {
        throw SpeechFailure(whyNone?.call() ?? 'No voice is set up to read replies aloud. Choose one in Settings, under Voice.');
      }
      PcmFormat? open;
      while (generation == _generation) {
        final sentence = await _next(generation);
        if (sentence == null) break;
        _current = sentence;
        notifyListeners();
        final utterance = sentence.utterance ??= chosen.output.speak(sentence.text, choices: chosen.choices);
        _prepare(chosen);
        final format = await utterance.format;
        if (format != null) {
          final out = player;
          if (out == null) throw const SpeechFailure('This build has no way to play the audio a voice provider sends back.');
          if (open != format) {
            if (open != null) {
              _playerOpen = false;
              await out.drain();
            }
            await out.open(format);
            _playerOpen = true;
            open = format;
          }
          await for (final piece in utterance.audio) {
            if (generation != _generation) break;
            await out.write(piece);
          }
        }
        await utterance.done;
        if (generation == _generation) {
          _current = null;
          notifyListeners();
        }
      }
      if (generation == _generation && open != null) {
        _playerOpen = false;
        await player?.drain();
      }
    } on SpeechFailure catch (failure) {
      _fail(generation, failure.message);
    } on Object {
      _fail(generation, 'The reply could not be read aloud.');
    } finally {
      // Only the newest reading owns the flag: one that was stopped leaves it to the one that took over.
      if (_workerGeneration == generation) {
        _current = null;
        _working = false;
      }
      notifyListeners();
    }
  }

  void _fail(int generation, String message) {
    if (generation != _generation) return;
    _error = message;
    final failed = _current ?? _queue.firstOrNull;
    _errorKey = failed == null || failed.followed ? null : failed.key;
    stop();
  }
}

final class _Sentence {
  _Sentence(this.key, this.text, {this.followed = false});
  final String key, text;

  /// Whether voice mode queued it, rather than a press of Play.
  final bool followed;
  SpeechUtterance? utterance;
}
