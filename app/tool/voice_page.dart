// The web player of a voice's audio, as a bare page for a browser test: PCM that a real voice made is fetched from the page's own
// site and played by the app's own Web Audio player (`lib/voice/pcm_player_web.dart`), and the page reports how much it queued and
// how long playing it took. Built beside the app with
//   dart compile js -O1 -o <dir>/page.js tool/voice_page.dart
// and loaded by a page that runs it; `clients/browser/chromium/e2e/voice_pcm.mjs` does both.
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:canoa/voice/pcm_player.dart';
import 'package:canoa/voice/speech_output.dart' show PcmFormat;
import 'package:web/web.dart' as web;

/// Plays the 16-bit mono PCM at [url], made at [rate] samples a second, in chunks as a voice provider's audio arrives.
Future<String> _play(String url, int rate) async {
  final player = platformPcmPlayer();
  if (player == null) return jsonEncode({'error': 'this build has no player'});
  final answer = await web.window.fetch(url.toJS).toDart;
  final bytes = (await answer.arrayBuffer().toDart).toDart.asUint8List();
  // A page may make a sound only after a press: the test's own click stands for it, and the context starts here.
  player.unlock();
  final started = DateTime.now();
  await player.open(PcmFormat(rate: rate));
  const piece = 8192;
  for (var at = 0; at < bytes.length; at += piece) {
    await player.write(Uint8List.sublistView(bytes, at, math.min(at + piece, bytes.length)));
  }
  await player.drain();
  return jsonEncode({
    'bytes': bytes.length,
    'seconds': bytes.length / (rate * 2),
    'tookMs': DateTime.now().difference(started).inMilliseconds,
  });
}

Future<String> _guard(Future<String> step) => step.catchError((Object error) => jsonEncode({'error': '$error'}));

void main() {
  globalContext
    ..setProperty('canoaPlay'.toJS, ((JSString url, JSNumber rate) => _guard(_play(url.toDart, rate.toDartInt)).then((answer) => answer.toJS).toJS).toJS)
    ..setProperty('canoaReady'.toJS, true.toJS);
}
