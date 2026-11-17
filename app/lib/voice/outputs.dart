import 'package:flutter/foundation.dart';

import '../config/canoa_config.dart' show SpeakSettings;
import 'host_speech_output.dart';
import 'speech_output.dart';

/// Every way this app or page can read text aloud: the browser's own voices, where it runs in one, and the host's providers, which
/// speak for a page and a platform with no voices of its own. Settings chooses between them and the chat reads replies through the
/// one chosen.
final class VoiceOutputs extends ChangeNotifier {
  VoiceOutputs({this.host, this.browser = const [], this.load}) {
    host?.addListener(notifyListeners);
  }

  /// The host's providers; null where this app reaches no host.
  final HostVoiceCatalog? host;

  /// The browser's voices; empty outside a browser.
  final List<SpeechOutput> browser;

  /// Reads the host's providers when nothing has, before the first one is wanted (the grant that lets this key use them is asked
  /// there); null where there is nothing to read.
  final Future<void> Function()? load;

  @override
  void dispose() {
    host?.removeListener(notifyListeners);
    super.dispose();
  }

  List<SpeechOutput> get all => [...browser, ...?host?.outputs];

  /// Whether anything is there to read aloud with, or may be once the host's providers are read.
  bool get offered => browser.isNotEmpty || host != null;

  /// Why the host's providers are not there, in the host's words.
  String? get problem => host?.problem;

  /// Reads the host's providers if they have not been read, or the last read failed.
  Future<void> ready() async {
    final catalog = host;
    if (catalog == null || (catalog.loaded && catalog.problem == null)) return;
    await (load?.call() ?? catalog.refresh());
  }

  /// The output the setting [id] names; with none named, this page's own voices if it has them, else the host's operating system,
  /// else the first the host has. Null when the named one is not here (a provider that was removed), which is never replaced by
  /// another.
  SpeechOutput? resolve(String? id) {
    final every = all;
    if (id != null) return every.where((each) => each.id == id).firstOrNull;
    return every.where((each) => each.id == 'browser').firstOrNull ??
        every.where((each) => each.id == hostOutputId('system')).firstOrNull ??
        every.where((each) => each.id != 'browser:online').firstOrNull;
  }
}

/// The choices [settings] holds for [output]: only those it takes, so one a provider ignores is never sent to it.
SpeakChoices choicesFor(SpeechOutput output, SpeakSettings settings) => SpeakChoices(
  voice: output.uses.contains('voice') ? settings.voice : null,
  model: output.uses.contains('model') ? settings.model : null,
  style: output.uses.contains('style') ? settings.style : null,
  speed: output.uses.contains('speed') && settings.speed != null ? output.speed?.clamp(settings.speed!) ?? settings.speed : null,
);
