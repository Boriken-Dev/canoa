import '../desktop/control_client.dart';
import '../desktop/host_control.dart';

/// One model an offer lists, as `kb/offers` answers it.
final class KbModel {
  const KbModel(this.id, this.name, this.capabilities);

  final String id;
  final String name;

  /// What the model can do, as ids; empty means the host's listing says nothing, which is not the same as none.
  final List<String> capabilities;

  String get label => name == id ? id : '$name ($id)';

  static KbModel? parse(Object? json) {
    if (json is! Map || json['id'] is! String) return null;
    final id = json['id'] as String;
    return KbModel(id, json['name'] is String ? json['name'] as String : id, _strings(json['capabilities']));
  }
}

/// The models of one offer for one operation: the ones that report what the operation needs, and the ones that report nothing, kept
/// apart. A typed id is taken whatever these hold.
final class KbModels {
  const KbModels({this.listed = false, this.why, this.offered = const [], this.unreported = const []});

  /// Whether the offer's service listed its models.
  final bool listed;
  final String? why;
  final List<KbModel> offered;
  final List<KbModel> unreported;

  static KbModels? parse(Object? json) {
    if (json is! Map) return null;
    List<KbModel> models(Object? rows) => [if (rows is List) for (final row in rows) ?KbModel.parse(row)];
    return KbModels(
      listed: json['listed'] == true,
      why: json['why'] is String ? json['why'] as String : null,
      offered: models(json['offered']),
      unreported: models(json['unreported']),
    );
  }
}

/// One thing that can do an operation: an account's API, a server of the person's, or a harness on a login.
final class KbOffer {
  const KbOffer({required this.id, required this.title, required this.kind, required this.location, this.address, this.models});

  final String id;
  final String title;

  /// `account`, `server` or `harness`.
  final String kind;

  /// Where the text goes, in the offer's own words.
  final String location;
  final String? address;

  /// Its models for the operation, when they were asked for.
  final KbModels? models;

  static KbOffer? parse(Object? json) {
    if (json is! Map || json['id'] is! String) return null;
    String? text(String key) => json[key] is String ? json[key] as String : null;
    return KbOffer(
      id: json['id'] as String,
      title: text('title') ?? json['id'] as String,
      kind: text('kind') ?? 'backend',
      location: text('location') ?? '',
      address: text('address'),
      models: KbModels.parse(json['models']),
    );
  }
}

/// What can do one operation (`complete`, `embed`, `rerank`), and how the person's canoa.cfg orders it.
final class KbOperation {
  const KbOperation({
    required this.id,
    required this.title,
    required this.capability,
    this.use,
    this.fallback = const [],
    this.asked = const [],
    this.problem,
    this.offers = const [],
  });

  final String id;
  final String title;
  final String capability;

  /// `[assist.OPERATION]` `use` and `fallback`, as written.
  final String? use;
  final List<String> fallback;

  /// The backends that would be asked, in order; empty with a [problem] when none can.
  final List<String> asked;
  final String? problem;
  final List<KbOffer> offers;

  KbOffer? offer(String id) => offers.where((each) => each.id == id).firstOrNull;

  static KbOperation? parse(Object? json) {
    if (json is! Map || json['id'] is! String) return null;
    String? text(String key) => json[key] is String ? json[key] as String : null;
    return KbOperation(
      id: json['id'] as String,
      title: text('title') ?? json['id'] as String,
      capability: text('capability') ?? '',
      use: text('use'),
      fallback: _strings(json['fallback']),
      asked: _strings(json['asked']),
      problem: text('problem'),
      offers: [if (json['offers'] is List) for (final each in json['offers'] as List) ?KbOffer.parse(each)],
    );
  }
}

/// An agent the person could offer as a harness backend, with the profiles and accounts it has.
final class KbHarnessSource {
  const KbHarnessSource(this.source, this.accounts, this.profiles);

  final String source;
  final List<String> accounts;
  final List<String> profiles;
}

/// `kb/offers`: everything that can do the host's own model work.
final class KbOffers {
  const KbOffers({required this.operations, this.harnesses = const [], this.idleSeconds, this.maxSessions});

  final List<KbOperation> operations;
  final List<KbHarnessSource> harnesses;

  /// What `[assist_pool]` means now, defaults included.
  final int? idleSeconds;
  final int? maxSessions;

  KbOperation? operation(String id) => operations.where((each) => each.id == id).firstOrNull;

  static KbOffers? parse(Object? json) {
    if (json is! Map) return null;
    final pool = json['pool'];
    return KbOffers(
      operations: [if (json['operations'] is List) for (final each in json['operations'] as List) ?KbOperation.parse(each)],
      harnesses: [
        if (json['harnesses'] is List)
          for (final each in json['harnesses'] as List)
            if (each is Map && each['source'] is String) KbHarnessSource(each['source'] as String, _strings(each['accounts']), _strings(each['profiles'])),
      ],
      idleSeconds: pool is Map && pool['idle_seconds'] is int ? pool['idle_seconds'] as int : null,
      maxSessions: pool is Map && pool['max_sessions'] is int ? pool['max_sessions'] as int : null,
    );
  }
}

List<String> _strings(Object? json) => [if (json is List) for (final each in json) if (each is String) each];

/// What the page asks the host: [KbOffers] (with the models of [offer] when one is named) and a try of a choice.
final class KbHost {
  const KbHost({required this.offers, required this.test});

  /// `kb/offers`: null when the host cannot answer (an older host, or a key without the accounts grant).
  final Future<KbOffers?> Function([String? offer]) offers;

  /// `kb/test`: `{ok, ms, ...}` or `{ok: false, why}`; throws only when the host cannot be asked at all.
  final Future<Map<String, Object?>> Function(Map<String, Object?> params) test;

  /// The host's own channel.
  factory KbHost.over(HostControl control) => KbHost(
    offers: ([offer]) async {
      try {
        final client = await control.ready(wait: false);
        // Listing an offer's models is a request to its service, which may be slow.
        final result = await client.call('kb/offers', {'offer': ?offer}, null, const Duration(minutes: 1));
        return KbOffers.parse(result);
      } on ControlFailed {
        return null;
      } on Object {
        return null;
      }
    },
    test: (params) async {
      final client = await control.ready(wait: false);
      final seconds = params['timeout_seconds'] is int ? params['timeout_seconds'] as int : 30;
      try {
        final result = await client.call('kb/test', params, null, Duration(seconds: seconds + 90));
        return result is Map ? result.cast<String, Object?>() : {'ok': false, 'why': '$result'};
      } on ControlFailed catch (error) {
        return {'ok': false, 'why': error.message};
      }
    },
  );
}
