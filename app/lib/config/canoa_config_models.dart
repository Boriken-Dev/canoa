part of 'canoa_config.dart';

/// One entry of `[host]`, as canoa.cfg has it.
final class HostEntry {
  const HostEntry({required this.name, required this.kind, required this.runs, this.args = const [], this.listen, this.harness});

  factory HostEntry.read(String name, String kind, Map<dynamic, dynamic> table) => HostEntry(
    name: name,
    kind: kind,
    runs: '${table[kind == 'agent' ? 'program' : 'library'] ?? ''}',
    args: [if (table['args'] case final List<dynamic> args) for (final arg in args) '$arg'],
    listen: table['listen'] is String ? table['listen'] as String : null,
    harness: table['harness'] is String ? table['harness'] as String : null,
  );

  final String name;

  /// `agent` or `provider`.
  final String kind;

  /// The program an agent runs, or a provider's library.
  final String runs;
  final List<String> args;

  /// The address it listens on, when the file names one.
  final String? listen;

  /// The harness provider an agent entry runs, when it runs one instead of a program.
  final String? harness;
}

/// How a dictation is held: one press starts it and the next ends it, or it lasts while a key is down.
enum DictationMode { toggle, hold }

/// `[voice]`'s text-to-speech keys: who reads a reply aloud and with what. A null is the provider's own choice, and a null
/// [output] is the first voice this app or page has.
final class SpeakSettings {
  const SpeakSettings({this.output, this.voice, this.model, this.style, this.speed, this.readReplies = false});

  /// The voice provider by the id it has in the app (`host:NAME`, `browser`, `browser:online`).
  final String? output;
  final String? voice, model, style;

  /// A multiple of normal speech, 1 being normal.
  final double? speed;

  /// Whether each reply is read aloud, a sentence at a time, as it arrives.
  final bool readReplies;

  bool get isDefault => output == null && voice == null && model == null && style == null && speed == null && !readReplies;

  /// The same with the named parts changed; a part named in [clear] goes back to the provider's own choice.
  SpeakSettings copyWith({
    String? output, String? voice, String? model, String? style, double? speed, bool? readReplies,
    bool clearOutput = false, bool clearVoice = false, bool clearModel = false, bool clearStyle = false, bool clearSpeed = false,
  }) => SpeakSettings(
    output: clearOutput ? null : output ?? this.output,
    voice: clearVoice ? null : voice ?? this.voice,
    model: clearModel ? null : model ?? this.model,
    style: clearStyle ? null : style ?? this.style,
    speed: clearSpeed ? null : speed ?? this.speed,
    readReplies: readReplies ?? this.readReplies,
  );
}

/// `[voice]`: where speech is turned into text and how, and who reads replies aloud. Nothing is recorded until the person starts a
/// dictation.
final class VoiceSettings {
  const VoiceSettings({this.backend = defaultVoiceBackend, this.model, this.language, this.device, this.mode = DictationMode.toggle, this.profile = const VoiceProfile(), this.acknowledged = const {}, this.speak = const SpeakSettings()});

  /// Who reads replies aloud; dictation's choices are the rest.
  final SpeakSettings speak;

  /// The speech service, by the id it has in the app; [defaultVoiceBackend] is the operating system's.
  final String backend;

  /// The service's model where it has several; null is the service's own choice.
  final String? model;

  /// A language tag such as `es-PR`; null is automatic where the service can tell, else the system's.
  final String? language;

  /// The input device by the id the service gives it; null is the system's default.
  final String? device;
  final DictationMode mode;

  /// What the person says of their own voice, sent to a provider as hints; they write it, nothing infers it.
  final VoiceProfile profile;

  /// The services whose "where the audio goes" notice the person has accepted, by id; the notice is not shown again for them.
  final Set<String> acknowledged;

  VoiceSettings copyWith({String? backend, String? model, bool clearModel = false, String? language, bool clearLanguage = false,
      String? device, bool clearDevice = false, DictationMode? mode, VoiceProfile? profile, Set<String>? acknowledged, SpeakSettings? speak}) => VoiceSettings(
    speak: speak ?? this.speak,
    backend: backend ?? this.backend,
    model: clearModel ? null : model ?? this.model,
    language: clearLanguage ? null : language ?? this.language,
    device: clearDevice ? null : device ?? this.device,
    mode: mode ?? this.mode,
    profile: profile ?? this.profile,
    acknowledged: acknowledged ?? this.acknowledged,
  );
}

/// The id of the operating system's speech service, which `[voice] backend` names unless the person chose another.
const defaultVoiceBackend = 'system';

/// The label and source of the catalog Canoa offers first, until the person removes it (`[store.overlays.catalogs] boriken = false`) or gives
/// that label a catalog of their own. The host reads the same pair. The app keeps its own because it shows a profile's effective view
/// while its host is stopped; `host_constants_test.dart` holds the pair equal to the host's.
const defaultCatalogLabel = 'boriken';
const defaultCatalogSpec = 'https://github.com/Boriken-Dev/agent-overlays.git@main#catalog.toml';

/// Where a [CanoaConfig] was read from: the format and first-line marker of the person's own file, the layers below it (the
/// installed copy and the system's file, lowest first), and the person's file as it was read. A save writes the person's
/// file only, in its own format.
final class ConfigSource {
  const ConfigSource({this.format = ConfigFormat.toml, this.marked = false, this.layers = const [], this.own = const {}, this.unreadable});

  /// Why canoa.cfg could not be read, for a configuration that is the defaults the app runs on instead: nothing is written over the
  /// file while it is set. Null for one that was read.
  final String? unreadable;

  final ConfigFormat format;

  /// Whether the file's first line names its format, so that it is written back with it.
  final bool marked;

  /// The layers below the person's file, lowest first.
  final List<ConfigLayer> layers;

  /// The person's file as read.
  final Map<String, dynamic> own;

  /// This source with the person's file holding [value] at the top-level [key], or none when [value] is null: what a save of one
  /// table by another part of the app leaves in it.
  ConfigSource withOwn(String key, Object? value) {
    final next = {...own};
    if (value == null) {
      next.remove(key);
    } else {
      next[key] = value;
    }
    return ConfigSource(format: format, marked: marked, layers: layers, own: next, unreadable: unreadable);
  }

  /// The layers below the person's file, merged.
  Map<String, dynamic> get lower {
    var merged = <String, dynamic>{};
    for (final layer in layers) {
      merged = mergeConfig(merged, layer.data);
    }
    return merged;
  }
}

