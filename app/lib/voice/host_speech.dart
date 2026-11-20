import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../config/canoa_config.dart' show defaultVoiceBackend;
import '../desktop/control_client.dart';
import '../desktop/host_control.dart';
import '../desktop/host_grants.dart';
import '../store/programs.dart' show ProgramStores;
import 'speech_backend.dart';

/// Where the audio of a dictation comes from. The microphone opens in [open] and closes when the stream is cancelled, so nothing
/// listens before a dictation starts and nothing is kept after it.
abstract interface class AudioCapture {
  /// 16-bit little-endian mono PCM at [rate] samples a second, in chunks, until cancelled.
  Stream<Uint8List> open({required int rate});
}

/// The most audio sent to the host in one call, in bytes before base64.
const _chunkBytes = 64 * 1024;

/// One kind of speech provider the host has, with the schema and the guatiao-intake form its settings are drawn with.
final class SpeechKind {
  const SpeechKind({required this.id, required this.title, required this.description, required this.schema, required this.form});

  final String id, title, description;
  final Map<String, Object?> schema, form;

  static SpeechKind? parse(Object? json) {
    if (json is! Map || json['id'] is! String) return null;
    return SpeechKind(
      id: json['id']! as String,
      title: json['title'] is String ? json['title']! as String : json['id']! as String,
      description: json['description'] is String ? json['description']! as String : '',
      schema: _map(json['schema']),
      form: _map(json['form']),
    );
  }
}

/// A provider in the host's canoa.cfg, as the host lists it. A setting that is a secret is never in [settings]; [secretsSet] names
/// the ones that have a value.
final class HostSpeechProvider {
  const HostSpeechProvider({
    required this.name,
    required this.kind,
    required this.title,
    required this.description,
    required this.settings,
    required this.secretsSet,
    required this.partials,
    this.problem,
    this.models = const [],
    this.offer,
    this.account,
  });

  final String name, kind, title, description;
  final Map<String, Object?> settings;
  final Set<String> secretsSet;
  final bool partials;

  /// The models its service has, when the host knows them; a recording may ask for one.
  final List<SpeechChoice> models;

  /// The offer it was made from, when it was one of Canoa's.
  final String? offer;

  /// The account whose API key it uses, when it was made of one: the key is the account's and never leaves the host.
  final String? account;

  /// Why it cannot be used, in the host's words; null when it can.
  final String? problem;

  static HostSpeechProvider? parse(Object? json) {
    if (json is! Map || json['name'] is! String) return null;
    final secrets = json['secrets_set'];
    return HostSpeechProvider(
      name: json['name']! as String,
      kind: json['provider'] is String ? json['provider']! as String : '',
      title: json['title'] is String ? json['title']! as String : json['name']! as String,
      description: json['description'] is String ? json['description']! as String : '',
      settings: _map(json['settings']),
      secretsSet: {if (secrets is List) ...secrets.whereType<String>()},
      partials: json['partials'] == true,
      problem: json['problem'] is String ? json['problem']! as String : null,
      models: _choices(json['models']),
      offer: json['offer'] is String ? json['offer']! as String : null,
      account: json['account'] is String ? json['account']! as String : null,
    );
  }
}

/// A list of `{id, label}` rows as choices; what is not one is left out.
List<SpeechChoice> _choices(Object? rows) => [
  if (rows is List)
    for (final row in rows)
      if (row is Map && row['id'] is String) SpeechChoice(row['id']! as String, row['label'] is String ? row['label']! as String : row['id']! as String),
];

/// One speech-to-text provider Canoa offers out of the box: a local engine the host installs through its Store, or a named service
/// added with the person's own key.
final class SpeechOffer {
  const SpeechOffer({
    required this.id,
    required this.title,
    required this.summary,
    required this.where,
    required this.local,
    required this.models,
    this.defaultModel,
    this.license,
    this.store,
    this.kind,
    this.settings = const {},
    this.installed = false,
    this.version,
    this.provider,
    this.account,
    this.service,
    this.unavailable,
  });

  final String id, title, summary, where;

  /// Whether the audio is turned into text on the host itself, with nothing sent to another service.
  final bool local;
  final String? license;
  final List<SpeechChoice> models;
  final String? defaultModel;

  /// The Store program a local engine is installed as; null for a service.
  final String? store;

  /// A service's provider kind and the settings it starts from.
  final String? kind;
  final Map<String, Object?> settings;

  /// Whether the engine's program is installed on the host, and its version.
  final bool installed;
  final String? version;

  /// The name of the provider canoa.cfg has made from it, if any.
  final String? provider;

  /// The account whose API key an offer made of an account uses; the account's name is what tells the person which key pays.
  final String? account;

  /// The Store type whose service on the host an offer calls, set up with a model the person has loaded there.
  final String? service;

  /// Why it cannot be set up now, in the host's words; null when it can.
  final String? unavailable;

  /// Whether the host makes the provider itself from a model the person picks, with no program to install and no form to fill.
  bool get direct => account != null || service != null;

  static SpeechOffer? parse(Object? json) {
    if (json is! Map || json['id'] is! String) return null;
    String? text(String key) => json[key] is String ? json[key]! as String : null;
    return SpeechOffer(
      id: json['id']! as String,
      title: text('title') ?? json['id']! as String,
      summary: text('summary') ?? '',
      where: text('where') ?? '',
      local: json['local'] == true,
      models: _choices(json['models']),
      defaultModel: text('default_model'),
      license: text('license'),
      store: text('store'),
      kind: text('kind'),
      settings: _map(json['settings']),
      installed: json['installed'] == true,
      version: text('version'),
      provider: text('provider'),
      account: text('account'),
      service: text('service'),
      unavailable: text('unavailable'),
    );
  }
}

Map<String, Object?> _map(Object? value) => value is Map ? value.cast<String, Object?>() : const {};

/// The speech providers of the host this app reaches: what its canoa.cfg defines, the kinds it offers, and the settings of each
/// written back through the host's control channel (`speech/*`). The providers run on the host, so a page and a platform with no
/// speech service of its own dictate through them.
final class HostSpeechCatalog extends ChangeNotifier {
  HostSpeechCatalog(this._control, {this.capture, this.grants, this.programs, this.wait = const Duration(seconds: 30), this.finishWait = const Duration(minutes: 3)});

  /// How long the host may take to answer a dictation's call before the dictation says it did not; opening a recording, a chunk of
  /// audio and a listing are answered at once, so a host that is silent for this long is not coming back.
  final Duration wait;

  /// How long `speech/finish` may take: the provider turns the whole recording into text there, and its own limit (a command's
  /// `timeout_seconds`) ends it with a reason of its own before this does.
  final Duration finishWait;

  final HostControl _control;

  /// What the host lets this key do; the offers say what is missing. Null when the app knows no grants (everything is tried).
  final HostGrants? grants;

  /// The host's Runtimes stores, which a local engine is installed through.
  final ProgramStores? programs;

  /// Where audio comes from; null while this build has no way to record, which a dictation says in words.
  final AudioCapture? capture;

  List<SpeechKind> _types = const [];
  List<HostSpeechProvider> _providers = const [];
  List<SpeechOffer> _offers = const [];
  String? _problem;
  bool _loaded = false;

  List<SpeechKind> get types => _types;
  List<HostSpeechProvider> get providers => _providers;

  /// What Canoa offers to add or install, read with the providers.
  List<SpeechOffer> get offers => _offers;

  /// The offer with the id [id].
  SpeechOffer? offerOf(String? id) => _offers.where((offer) => offer.id == id).firstOrNull;

  /// Whether the host's grants let this key [grant]; true while they are not known, so the host's own refusal says it.
  bool may(String grant) => grants == null || !grants!.known || grants!.may(grant);

  /// Why the host's providers are not listed (no grant, no link); null when they are or nothing was asked yet.
  String? get problem => _problem;
  bool get loaded => _loaded;

  /// The providers as speech services Settings and the composer choose between.
  List<SpeechBackend> get backends => _backends;
  List<SpeechBackend> _backends = const [];

  /// The host's own answer to [method], or a [SpeechFailure] saying why not.
  Future<Map<String, Object?>> call(String method, Map<String, Object?> params, {Duration? within}) async {
    try {
      final client = await _control.ready(wait: false);
      final result = await client.call(method, params, null, within ?? const Duration(minutes: 3));
      return _map(result);
    } on ControlFailed catch (error) {
      throw SpeechFailure(error.message);
    } on StateError catch (error) {
      throw SpeechFailure(error.message);
    } on TimeoutException {
      throw const SpeechFailure('The host did not answer in time.');
    }
  }

  /// Reads the providers again.
  Future<void> refresh() async {
    try {
      final answer = await call('speech/providers', const {}, within: wait);
      _types = [for (final kind in (answer['types'] as List? ?? const [])) ?SpeechKind.parse(kind)];
      _providers = [for (final provider in (answer['providers'] as List? ?? const [])) ?HostSpeechProvider.parse(provider)];
      _problem = null;
      try {
        final offered = await call('speech/offers', const {}, within: wait);
        _offers = [for (final offer in (offered['offers'] as List? ?? const [])) ?SpeechOffer.parse(offer)];
      } on SpeechFailure {
        // A host that makes no offers (an older one) still has its providers.
        _offers = const [];
      }
    } on SpeechFailure catch (failure) {
      _types = const [];
      _providers = const [];
      _offers = const [];
      _problem = failure.message;
    }
    // One backend per provider, kept until the next read, so a screen holding one is not handed another on each build.
    _backends = [for (final provider in _providers.where((p) => p.problem == null)) HostSpeech(this, provider)];
    _loaded = true;
    notifyListeners();
  }

  /// Writes the provider [name] as a [kind] with [settings]. A secret left out or empty keeps what the host holds; [clear] names
  /// the ones to take out. Throws a [SpeechFailure] with the host's words when it refuses.
  Future<void> save(String name, String kind, Map<String, Object?> settings, {List<String> clear = const []}) async {
    await call('speech/configure', {'name': name, 'provider': kind, 'settings': settings, if (clear.isNotEmpty) 'clear': clear});
    await refresh();
  }

  Future<void> remove(String name) async {
    await call('speech/configure', {'name': name, 'remove': true});
    await refresh();
  }

  /// Makes the provider of [offer] from the copy the host's Store has installed, or from the account or the service it names with
  /// [model], and lists it. Throws a [SpeechFailure] with the host's words when it cannot.
  Future<String> enable(String offer, {String? name, String? model}) async {
    final made = await call('speech/enable', {'offer': offer, 'name': ?name, 'model': ?model});
    await refresh();
    return made['name'] as String? ?? offer;
  }
}

/// The service `[voice] backend` names among [platform]'s and the host's. A person who never chose one (the default, the operating
/// system's service) and whose system has none gets the host's first local provider, so a page dictates as soon as its host has
/// one; a provider that sends the audio to a named service is only ever the person's own choice.
SpeechBackend? resolveSpeech(String id, List<SpeechBackend> platform, List<SpeechBackend> host) {
  for (final each in [...platform, ...host]) {
    if (each.id == id) return each;
  }
  if (id == defaultVoiceBackend) {
    for (final each in host) {
      if (each is HostSpeech && each.local) return each;
    }
  }
  return null;
}

/// The parts of `speech/open` that carry what the person wrote of their own voice in Settings; nothing for what they left empty.
Map<String, Object?> _profileParams(VoiceProfile? profile) => profile == null || profile.isEmpty
    ? const {}
    : {
        if (profile.vocabulary.isNotEmpty) 'vocabulary': profile.vocabulary,
        if (profile.accent != null || profile.voice != null) 'profile': {'accent': ?profile.accent, 'voice': ?profile.voice},
      };

/// The id a host provider has in `[voice] backend`.
String hostSpeechId(String name) => 'host:$name';

/// One provider of the host as a speech service: the audio is streamed to the host in chunks and its partial and final text come
/// back. The recording belongs to this link; nothing is kept on this side.
final class HostSpeech implements SpeechBackend {
  HostSpeech(this._catalog, this.provider);

  final HostSpeechCatalog _catalog;
  final HostSpeechProvider provider;

  final _levels = StreamController<double>.broadcast();
  StreamController<Transcript>? _out;
  StreamSubscription<Uint8List>? _audio;
  String? _session;
  Future<void> _sending = Future.value();
  bool _ending = false;

  @override
  String get id => hostSpeechId(provider.name);

  @override
  String get name {
    final account = provider.account;
    if (account != null) return '${provider.name} (OpenAI, account $account)';
    return provider.title == provider.name ? provider.name : '${provider.name} (${provider.title})';
  }

  SpeechOffer? get _offer => _catalog.offerOf(provider.offer);

  /// Whether the host turns the audio into text itself, with a program it installed and nothing sent to another service.
  bool get local => _offer?.local ?? false;

  @override
  SpeechWhere get where => SpeechWhere(
    place: SpeechPlace.host,
    leaves: AudioLeaves.maybe,
    words: _offer?.where ?? 'The audio goes to your host, which hands it to the provider you set up there: a program it runs or an address you named.',
    endpoint: provider.settings['url'] is String ? provider.settings['url']! as String : null,
  );

  @override
  Future<List<SpeechChoice>> models() async => provider.models;

  @override
  Future<List<SpeechChoice>> languages() async => const [];

  @override
  Future<List<SpeechChoice>> devices() async => const [];

  @override
  Stream<double> get levels => _levels.stream;

  @override
  Stream<Transcript> start({String? device, String? language, String? model, VoiceProfile? profile}) {
    final out = StreamController<Transcript>();
    _out = out;
    _ending = false;
    unawaited(_run(out, language, model, profile));
    return out.stream;
  }

  Future<void> _run(StreamController<Transcript> out, String? language, String? model, VoiceProfile? profile) async {
    final capture = _catalog.capture;
    if (capture == null) {
      out.addError(const SpeechFailure('This build cannot record audio yet.'));
      await _close(out);
      return;
    }
    try {
      const rate = 16000;
      // The model is asked for only when the host lists it, so a choice made for another service is never sent to this one.
      final asked = model != null && provider.models.any((each) => each.id == model) ? model : null;
      final opened = await _catalog.call('speech/open', {'provider': provider.name, 'rate': rate, 'language': ?language, 'model': ?asked, ..._profileParams(profile)}, within: _catalog.wait);
      final session = opened['session'] as String;
      _session = session;
      if (_ending) {
        await _cancelSession(session);
        await _close(out);
        return;
      }
      final audio = capture.open(rate: rate);
      _audio = audio.listen(
        (chunk) {
          for (var at = 0; at < chunk.length; at += _chunkBytes) {
            final part = Uint8List.sublistView(chunk, at, at + _chunkBytes > chunk.length ? chunk.length : at + _chunkBytes);
            _sending = _sending.then((_) => _push(session, part, out));
          }
        },
        onError: (Object error) => out.addError(const SpeechFailure('The microphone failed.')),
        onDone: () {
          if (!_ended.isCompleted) _ended.complete();
        },
        cancelOnError: false,
      );
      // `stop` and `cancel` end the audio; the recording is finished or cancelled after the last chunk is taken.
      await _ended.future;
      // A microphone that does not let go within a moment is not waited for: the recording is finished with what it sent.
      await _audio?.cancel().timeout(const Duration(seconds: 5), onTimeout: () {});
      _audio = null;
      await _sending;
      if (_cancelled) {
        await _cancelSession(session);
      } else {
        final finished = await _catalog.call('speech/finish', {'session': session}, within: _catalog.finishWait);
        out.add(Transcript(finished['text'] as String? ?? '', isFinal: true, frontMatter: _map(finished['front_matter'])));
      }
    } on SpeechFailure catch (failure) {
      out.addError(failure);
      final session = _session;
      if (session != null) await _cancelSession(session);
    } on Object {
      out.addError(const SpeechFailure('The host could not recognize the speech.'));
    }
    await _close(out);
  }

  Completer<void> _ended = Completer<void>();
  bool _cancelled = false;

  Future<void> _push(String session, Uint8List part, StreamController<Transcript> out) async {
    if (_cancelled || out.isClosed) return;
    try {
      final heard = await _catalog.call('speech/push', {'session': session, 'audio': base64Encode(part)}, within: _catalog.wait);
      final partial = heard['partial'];
      if (partial is String && !out.isClosed) out.add(Transcript(partial, isFinal: false));
    } on SpeechFailure catch (failure) {
      _cancelled = true;
      if (!out.isClosed) out.addError(failure);
      if (!_ended.isCompleted) _ended.complete();
    }
  }

  Future<void> _cancelSession(String session) async {
    try {
      await _catalog.call('speech/cancel', {'session': session}, within: _catalog.wait);
    } on SpeechFailure {
      // The host has already ended it.
    }
  }

  Future<void> _close(StreamController<Transcript> out) async {
    _session = null;
    _ended = Completer<void>();
    _cancelled = false;
    _sending = Future.value();
    if (!out.isClosed) await out.close();
    if (identical(_out, out)) _out = null;
  }

  @override
  Future<void> stop() async {
    _ending = true;
    if (!_ended.isCompleted) _ended.complete();
  }

  @override
  Future<void> cancel() async {
    _ending = true;
    _cancelled = true;
    if (!_ended.isCompleted) _ended.complete();
  }
}
