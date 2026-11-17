import 'dart:async';
import 'dart:js_interop';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:web/web.dart' as web;

import 'pcm_player.dart';
import 'speech_backend.dart' show SpeechFailure;
import 'speech_output.dart' show PcmFormat;

/// The page's Web Audio. Audio is turned into buffers and scheduled one after another, with no media element, no Blob and nothing
/// fetched, so the page's content security policy has nothing to refuse.
PcmPlayer? create() => _WebAudioPlayer();

final class _WebAudioPlayer implements PcmPlayer {
  web.AudioContext? _context;
  PcmFormat? _format;
  final _playing = <web.AudioBufferSourceNode>{};
  double _endsAt = 0;
  Completer<void>? _idle;
  Completer<void>? _room;
  Uint8List _carry = Uint8List(0);

  web.AudioContext _audio() {
    final made = _context ??= web.AudioContext();
    if (made.state == 'suspended') unawaited(made.resume().toDart.then((_) {}, onError: (_) {}));
    return made;
  }

  @override
  void unlock() {
    try {
      _audio();
    } on Object {
      // The page has no audio; opening says so.
    }
  }

  @override
  Future<void> open(PcmFormat format) async {
    if (_format == format) return;
    if (_format != null) await drain();
    try {
      _audio();
    } on Object {
      throw const SpeechFailure('This browser cannot play audio from a page.');
    }
    _format = format;
    _carry = Uint8List(0);
    _endsAt = 0;
  }

  double get _ahead => math.max(0, _endsAt - (_context?.currentTime ?? 0));

  @override
  Future<void> write(Uint8List pcm) async {
    final format = _format, context = _context;
    if (format == null || context == null) return;
    final block = format.channels * 2;
    var bytes = pcm;
    if (_carry.isNotEmpty) {
      bytes = Uint8List(_carry.length + pcm.length)
        ..setRange(0, _carry.length, _carry)
        ..setRange(_carry.length, _carry.length + pcm.length, pcm);
    }
    final whole = bytes.length - bytes.length % block;
    _carry = Uint8List.fromList(bytes.sublist(whole));
    if (whole == 0) return;
    final frames = whole ~/ block;
    final view = ByteData.sublistView(bytes, 0, whole);
    final buffer = context.createBuffer(format.channels, frames, format.rate.toDouble());
    for (var channel = 0; channel < format.channels; channel++) {
      final samples = Float32List(frames);
      for (var i = 0; i < frames; i++) {
        samples[i] = view.getInt16((i * format.channels + channel) * 2, Endian.little) / 32768.0;
      }
      buffer.copyToChannel(samples.toJS, channel);
    }
    final source = context.createBufferSource()..buffer = buffer;
    source.connect(context.destination);
    final startAt = math.max(_endsAt, context.currentTime + 0.04);
    source.onended = ((web.Event _) {
      _playing.remove(source);
      source.disconnect();
      _wake();
    }).toJS;
    _playing.add(source);
    source.start(startAt);
    _endsAt = startAt + frames / format.rate;
    if (_ahead > pcmAheadSeconds) {
      final room = _room = Completer<void>();
      await room.future;
    }
  }

  void _wake() {
    if (_room != null && _ahead <= pcmAheadSeconds) {
      _room!.complete();
      _room = null;
    }
    if (_playing.isEmpty && _idle != null) {
      _idle!.complete();
      _idle = null;
    }
  }

  @override
  Future<void> drain() async {
    if (_playing.isNotEmpty) {
      final idle = _idle ??= Completer<void>();
      await idle.future;
    }
    _format = null;
  }

  @override
  Future<void> stop() async {
    for (final source in [..._playing]) {
      source.onended = null;
      try {
        source.stop();
      } on Object {
        // It had ended already.
      }
      source.disconnect();
    }
    _playing.clear();
    _format = null;
    _carry = Uint8List(0);
    _endsAt = 0;
    _wake();
  }
}
