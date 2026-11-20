import 'dart:async';
import 'dart:convert';
import 'package:flutter/foundation.dart';

import 'speech_backend.dart' show SpeechFailure;
import 'speech_output.dart';

/// The host's own answer to a method of `speech/*`, or a [SpeechFailure] saying why not. It is the call [HostSpeechCatalog.call]
/// makes, which the voice side shares so one link and one set of messages serve both directions.
typedef HostCall = Future<Map<String, Object?>> Function(String method, Map<String, Object?> params);

Map<String, Object?> _map(Object? value) => value is Map ? value.cast<String, Object?>() : const {};
List<Object?> _list(Object? value) => value is List ? value.cast<Object?>() : const [];
String? _text(Object? value) => value is String ? value : null;

/// One kind of voice provider the host has, with the schema and the guatiao-intake form its settings are drawn with.
final class VoiceKind {
  const VoiceKind({required this.id, required this.title, required this.description, required this.schema, required this.form});

  final String id, title, description;
  final Map<String, Object?> schema, form;

  static VoiceKind? parse(Object? json) {
    if (json is! Map || json['id'] is! String) return null;
    return VoiceKind(
      id: json['id']! as String,
      title: _text(json['title']) ?? json['id']! as String,
      description: _text(json['description']) ?? '',
      schema: _map(json['schema']),
      form: _map(json['form']),
    );
  }
}

OutputChoice? _choice(Object? json) {
  if (json is! Map || json['id'] is! String) return null;
  final id = json['id']! as String;
  return OutputChoice(id, _text(json['label']) ?? id, description: _text(json['description']));
}

/// A voice provider of the host, as `speech/voices` lists it: what it says of itself, where the text goes, and the voices, models,
/// styles and speeds a person may choose. A setting that is a secret is never in [settings]; [secretsSet] names the ones with a value.
final class HostVoiceProvider {
  const HostVoiceProvider({
    required this.name,
    required this.kind,
    required this.builtin,
    required this.title,
    required this.description,
    required this.settings,
    required this.secretsSet,
    required this.where,
    required this.models,
    required this.defaultModel,
    required this.styles,
    required this.speed,
    required this.uses,
    required this.voices,
    this.maxChars,
    this.problem,
    this.voicesProblem,
  });

  final String name, kind, title, description;

  /// Whether the host offers it with no table in canoa.cfg (the operating system's voices, the voices of a Store program that is installed).
  final bool builtin;
  final Map<String, Object?> settings;
  final Set<String> secretsSet;
  final OutputWhere where;
  final List<OutputChoice> models, styles;
  final String? defaultModel;
  final SpeedRange? speed;
  final Set<String> uses;
  final List<OutputVoice> voices;
  final int? maxChars;

  /// Why it cannot be used, in the host's words; null when it can.
  final String? problem;

  /// Why its voices could not be listed, when it can speak but not say with what.
  final String? voicesProblem;

  static HostVoiceProvider? parse(Object? json) {
    if (json is! Map || json['name'] is! String) return null;
    final name = json['name']! as String;
    final leaves = switch (json['leaves']) {
      'yes' => TextLeaves.yes,
      'maybe' => TextLeaves.maybe,
      _ => TextLeaves.no,
    };
    final speed = _map(json['speed']);
    final min = speed['min'], max = speed['max'];
    return HostVoiceProvider(
      name: name,
      kind: _text(json['provider']) ?? '',
      builtin: json['builtin'] == true,
      title: _text(json['title']) ?? name,
      description: _text(json['description']) ?? '',
      settings: _map(json['settings']),
      secretsSet: {..._list(json['secrets_set']).whereType<String>()},
      where: OutputWhere(
        leaves: leaves,
        words: _text(json['words']) ?? 'The text goes to a provider on your host.',
        endpoint: _text(json['endpoint']),
      ),
      models: [for (final choice in _list(json['models'])) ?_choice(choice)],
      defaultModel: _text(json['default_model']),
      styles: [for (final choice in _list(json['styles'])) ?_choice(choice)],
      speed: min is num && max is num ? SpeedRange(min.toDouble(), max.toDouble()) : null,
      uses: {..._list(json['uses']).whereType<String>()},
      voices: [for (final voice in _list(json['voices'])) ?OutputVoice.parse(voice)],
      maxChars: json['max_chars'] is int ? json['max_chars']! as int : null,
      problem: _text(json['problem']),
      voicesProblem: _text(json['voices_problem']),
    );
  }
}

/// The text-to-speech providers of the host this app reaches: what its canoa.cfg defines, the ones the host offers by itself, the kinds
/// it has, and the settings of each written back through the host's control channel (`speech/*`). The providers run on the host, so a
/// page and a platform with no voices of its own read replies aloud through them.
final class HostVoiceCatalog extends ChangeNotifier {
  HostVoiceCatalog(this._call);

  final HostCall _call;

  List<VoiceKind> _types = const [];
  List<HostVoiceProvider> _providers = const [];
  List<SpeechOutput> _outputs = const [];
  String? _problem;
  bool _loaded = false;

  List<VoiceKind> get types => _types;
  List<HostVoiceProvider> get providers => _providers;

  /// The providers that can speak, as outputs Settings and the chat choose between; the same objects until the next read.
  List<SpeechOutput> get outputs => _outputs;

  /// Why the host's providers are not listed (no grant, no link); null when they are or nothing was asked yet.
  String? get problem => _problem;
  bool get loaded => _loaded;

  /// Reads the providers and their voices again.
  Future<void> refresh() async {
    try {
      final answer = await _call('speech/voices', const {});
      _types = [for (final kind in _list(answer['types'])) ?VoiceKind.parse(kind)];
      _providers = [for (final provider in _list(answer['providers'])) ?HostVoiceProvider.parse(provider)];
      _problem = null;
    } on SpeechFailure catch (failure) {
      _types = const [];
      _providers = const [];
      _problem = failure.message;
    }
    _outputs = [for (final provider in _providers.where((p) => p.problem == null)) HostSpeechOutput(_call, provider)];
    _loaded = true;
    notifyListeners();
  }

  /// Writes the provider [name] as a [kind] with [settings]. A secret left out or empty keeps what the host holds; [clear] names the
  /// ones to take out. Throws a [SpeechFailure] with the host's words when it refuses.
  Future<void> save(String name, String kind, Map<String, Object?> settings, {List<String> clear = const []}) async {
    await _call('speech/say_configure', {'name': name, 'provider': kind, 'settings': settings, if (clear.isNotEmpty) 'clear': clear});
    await refresh();
  }

  Future<void> remove(String name) async {
    await _call('speech/say_configure', {'name': name, 'remove': true});
    await refresh();
  }
}

/// The id a host provider has in a setting that names an output.
String hostOutputId(String name) => 'host:$name';

/// One provider of the host as a way of speaking: the text is sent to the host, which hands it to the provider, and the audio comes
/// back in pieces to be played as it arrives. Nothing is kept on this side.
final class HostSpeechOutput implements SpeechOutput {
  HostSpeechOutput(this._call, this.provider);

  final HostCall _call;
  final HostVoiceProvider provider;

  @override
  String get id => hostOutputId(provider.name);

  @override
  String get name => provider.title == provider.name ? provider.name : '${provider.name} (${provider.title})';

  @override
  OutputWhere get where => provider.where;

  @override
  List<OutputChoice> get models => provider.models;

  @override
  String? get defaultModel => provider.defaultModel;

  @override
  List<OutputChoice> get styles => provider.styles;

  @override
  SpeedRange? get speed => provider.speed;

  @override
  Set<String> get uses => provider.uses;

  @override
  Future<List<OutputVoice>> voices({String? language}) async {
    if (language == null || language.isEmpty) return provider.voices;
    String primary(String tag) => tag.split(RegExp('[-_]')).first.toLowerCase();
    final wanted = primary(language);
    return [
      for (final voice in provider.voices)
        if (voice.language == null || primary(voice.language!) == wanted) voice,
    ];
  }

  @override
  SpeechUtterance speak(String text, {SpeakChoices choices = const SpeakChoices()}) => _HostUtterance(_call, provider.name, text, choices);
}

/// The most audio one call asks the host for, in bytes.
const _pieceBytes = 64 * 1024;

/// How long a call waits for audio at the host, in milliseconds.
const _waitMs = 1000;

final class _HostUtterance implements SpeechUtterance {
  _HostUtterance(this._call, this._provider, this._said, this._choices) {
    _format.future.ignore();
    _done.future.ignore();
    unawaited(_run());
  }

  final HostCall _call;
  final String _provider, _said;
  final SpeakChoices _choices;

  final _out = StreamController<Uint8List>();
  final _format = Completer<PcmFormat?>();
  final _done = Completer<void>();
  String? _id;
  bool _stopped = false;

  @override
  Future<PcmFormat?> get format => _format.future;

  @override
  Stream<Uint8List> get audio => _out.stream;

  @override
  Future<void> get done => _done.future;

  Future<void> _run() async {
    try {
      final started = await _call('speech/say', {
        'provider': _provider,
        'text': _said,
        if (_choices.voice != null) 'voice': _choices.voice,
        if (_choices.model != null) 'model': _choices.model,
        if (_choices.style != null) 'style': _choices.style,
        if (_choices.speed != null) 'speed': _choices.speed,
        if (_choices.language != null) 'language': _choices.language,
      });
      final id = started['utterance'];
      if (id is! String) throw const SpeechFailure('The host did not say how to read the audio.');
      _id = id;
      final rate = started['rate'];
      final channels = started['channels'];
      if (!_format.isCompleted) {
        _format.complete(PcmFormat(rate: rate is int ? rate : 22050, channels: channels is int ? channels : 1));
      }
      if (_stopped) {
        await _stopAtHost();
        return;
      }
      while (!_stopped) {
        // A player that is behind holds this back, which holds the provider back at its own buffer.
        while (_out.isPaused && !_stopped) {
          await Future<void>.delayed(const Duration(milliseconds: 40));
        }
        if (_stopped) break;
        final piece = await _call('speech/audio', {'utterance': id, 'wait_ms': _waitMs, 'max_bytes': _pieceBytes});
        final bytes = base64Decode(_text(piece['audio']) ?? '');
        if (bytes.isNotEmpty && !_out.isClosed && !_stopped) _out.add(bytes);
        // The host ends an utterance that is over, whether it finished or failed: there is nothing to stop.
        final over = piece['done'] == true;
        if (over) _id = null;
        final why = _text(piece['error']);
        if (why != null) throw SpeechFailure(why);
        if (over) break;
      }
      await _finish();
    } on SpeechFailure catch (failure) {
      await _fail(failure);
    } on FormatException {
      await _fail(const SpeechFailure('The host answered audio that could not be read.'));
    } on Object {
      await _fail(const SpeechFailure('The host could not read the text aloud.'));
    }
  }

  Future<void> _finish() async {
    if (!_format.isCompleted) _format.complete(null);
    if (!_out.isClosed) unawaited(_out.close());
    if (!_done.isCompleted) _done.complete();
  }

  Future<void> _fail(SpeechFailure failure) async {
    if (!_format.isCompleted) _format.completeError(failure);
    if (!_out.isClosed) {
      _out.addError(failure);
      // Closing completes once a listener has taken the end, which may be never: nothing here waits for it.
      unawaited(_out.close());
    }
    if (!_done.isCompleted) _done.completeError(failure);
    await _stopAtHost();
  }

  Future<void> _stopAtHost() async {
    final id = _id;
    _id = null;
    if (id == null) return;
    try {
      await _call('speech/stop', {'utterance': id});
    } on SpeechFailure {
      // The host has already ended it.
    }
  }

  @override
  Future<void> stop() async {
    if (_stopped) return;
    _stopped = true;
    await _stopAtHost();
    await _finish();
  }
}
