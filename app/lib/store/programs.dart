import 'dart:async';

import 'package:coqui_acp/secret.dart' show AcpSecretCancelled, AcpSecretRequestReply, acpSecretRequestMethod;
import 'package:flutter/foundation.dart';

import '../accounts/accounts_host.dart' show SignInAsk, openSealedAsk;
import '../acp/link_secret.dart' show LinkSecretIdentity;
import '../desktop/control_client.dart';

// The program Store as the host's control channel carries it: what each store's extension reports of its program (found, which
// version, whether a newer one exists) and what it would do to install or update it. The app holds no program name, command
// or address: everything below is read from what the host answers.

String? _text(Object? value) => value is String ? value : null;

Map<String, Object?> _map(Object? value) => value is Map
    ? {for (final MapEntry(:key, :value) in value.entries) '$key': value}
    : const {};

List<Map<String, Object?>> _maps(Object? value) => value is List ? [for (final each in value) _map(each)] : const [];

/// Which harness a store's program serves, and when.
final class ProgramServes {
  const ProgramServes(this.provider, this.when);

  /// The harness provider id.
  final String provider;

  /// The provider settings that select the program, a `harness` key naming the program; empty selects any.
  final Map<String, Object?> when;
}

/// Who a store's program is: `store/list` gives one for each store the host offers.
final class ProgramStoreInfo {
  const ProgramStoreInfo({
    required this.provider,
    required this.program,
    required this.title,
    this.publisher = '',
    this.homepage,
    this.summary,
    this.icon,
    this.commands = const [],
    this.endpoints = const [],
    this.serves = const [],
    this.channels = const [],
    this.library,
    this.libraryTitle,
    this.type,
  });

  /// The store's id, which the other `store/*` methods name.
  final String provider;
  final String program, title, publisher;

  /// The library that offers the store, which groups the Programs list; none when the host does not say.
  final String? library;

  /// The library's own name for itself, when the host says it: what the badge shows in place of [library].
  final String? libraryTitle;

  /// The id of the Store type the library adds that claims this program, when one does: what its models and its setup are filed under.
  final String? type;
  final String? homepage;

  /// What the program is, in a sentence, as the store says it.
  final String? summary;

  /// The name of the glyph the store picked for it; `runtimeGlyphs` draws it, and a name it lacks draws the default.
  final String? icon;

  /// The commands it puts on a `PATH` when the host starts something with it.
  final List<String> commands;

  /// The places it downloads from, and no others.
  final List<String> endpoints;
  final List<ProgramServes> serves;
  final List<String> channels;

  static ProgramStoreInfo? parse(Object? json) {
    final map = _map(json);
    final provider = _text(map['provider']);
    if (provider == null) return null;
    return ProgramStoreInfo(
      provider: provider,
      program: _text(map['program']) ?? provider,
      title: _text(map['title']) ?? _text(map['program']) ?? provider,
      publisher: _text(map['publisher']) ?? '',
      homepage: _text(map['homepage']),
      summary: _text(map['summary']),
      icon: _text(map['icon']),
      commands: [if (map['commands'] is List) for (final each in map['commands'] as List) ?_text(each)],
      endpoints: [if (map['endpoints'] is List) for (final each in map['endpoints'] as List) ?_text(each)],
      serves: [for (final each in _maps(map['serves'])) ProgramServes(_text(each['provider']) ?? '', _map(each['when']))],
      channels: [if (map['channels'] is List) for (final each in map['channels'] as List) ?_text(each)],
      library: _text(map['library']),
      libraryTitle: _text(map['library_title']),
      type: _text(map['type']),
    );
  }

  /// Whether this store's program is what a source runs: the source's harness provider id, else its label, matched to the
  /// provider a store serves and the `harness` setting that selects its program.
  bool servesSource({String? harness, required String label}) {
    for (final each in serves) {
      final wanted = each.when['harness'];
      if (wanted == null) {
        if (harness != null && each.provider == harness) return true;
      } else if (wanted is String) {
        final named = (harness ?? label).toLowerCase();
        if (named == wanted.toLowerCase() || label.toLowerCase() == wanted.toLowerCase()) return true;
      }
    }
    return false;
  }
}

/// Who can update a copy of the program.
sealed class ProgramUpdatable {
  const ProgramUpdatable();

  static ProgramUpdatable parse(Object? json) {
    final map = _map(json);
    return switch (map['kind']) {
      'by_store' => const UpdatedByStore(),
      'elsewhere' => UpdatedElsewhere(_text(map['how']) ?? ''),
      _ => UpdatedNever(_text(map['why']) ?? ''),
    };
  }
}

/// By the store, into the host's folder.
final class UpdatedByStore extends ProgramUpdatable {
  const UpdatedByStore();
}

/// By the program or its package manager; [how] is the command for the person to run.
final class UpdatedElsewhere extends ProgramUpdatable {
  const UpdatedElsewhere(this.how);
  final String how;
}

/// Not at all, for [why].
final class UpdatedNever extends ProgramUpdatable {
  const UpdatedNever(this.why);
  final String why;
}

/// One copy of the program a store found.
final class ProgramCopy {
  const ProgramCopy({
    required this.path,
    this.version,
    this.manager = '',
    this.managed = false,
    this.selfUpdates = false,
    this.update = const UpdatedByStore(),
  });

  final String path;
  final String? version;

  /// Who put it there, as the store names it.
  final String manager;

  /// Whether it is the host's own copy.
  final bool managed;
  final bool selfUpdates;
  final ProgramUpdatable update;

  static ProgramCopy? parse(Object? json) {
    final map = _map(json);
    final path = _text(map['path']);
    if (path == null) return null;
    return ProgramCopy(
      path: path,
      version: _text(map['version']),
      manager: _text(map['manager']) ?? '',
      managed: map['managed'] == true,
      selfUpdates: map['self_updates'] == true,
      update: ProgramUpdatable.parse(map['update']),
    );
  }
}

/// The newest version, or why it is not known.
final class ProgramLatest {
  const ProgramLatest({this.known = false, this.version, this.unknownBecause, this.notesUrl, this.note});

  final bool known;

  /// [note] is a caveat to read next to the version, such as a newer release the store cannot install.
  final String? version, unknownBecause, notesUrl, note;

  static ProgramLatest parse(Object? json) {
    final map = _map(json);
    return ProgramLatest(
      known: map['known'] == true,
      version: _text(map['version']),
      unknownBecause: _text(map['unknown_because']),
      notesUrl: _text(map['notes_url']),
      note: _text(map['note']),
    );
  }
}

/// Whether [latest] is a newer version than [found]: compared as runs of numbers when both are, else just different.
bool isNewer(String latest, String found) {
  final a = RegExp(r'\d+').allMatches(latest).map((each) => int.parse(each.group(0)!)).toList();
  final b = RegExp(r'\d+').allMatches(found).map((each) => int.parse(each.group(0)!)).toList();
  if (a.isEmpty || b.isEmpty) return latest != found;
  for (var at = 0; at < a.length || at < b.length; at++) {
    final left = at < a.length ? a[at] : 0, right = at < b.length ? b[at] : 0;
    if (left != right) return left > right;
  }
  return false;
}

/// What a store's extension reports of its program now.
final class ProgramStatus {
  const ProgramStatus({
    required this.info,
    this.copies = const [],
    this.latest,
    this.latestAt,
    this.problem,
    this.reported = false,
  });

  final ProgramStoreInfo info;

  /// Every copy found, the host's own first.
  final List<ProgramCopy> copies;

  /// What was last asked of the publisher, when it was asked; null until the person's page asked.
  final ProgramLatest? latest;
  final DateTime? latestAt;

  /// Why the report could not be had, in the host's words.
  final String? problem;

  /// Whether the host has answered for this store yet.
  final bool reported;

  /// The copy a source runs: the host's own, else the first found.
  ProgramCopy? get copy => copies.where((each) => each.managed).firstOrNull ?? copies.firstOrNull;
  bool get installed => copy != null;
  String? get version => copy?.version;
  bool get hasManaged => copies.any((each) => each.managed);

  /// Whether a newer version than the copy's exists, as the publisher says.
  bool get updateAvailable {
    final found = version, newest = latest;
    return found != null && newest != null && newest.known && newest.version != null && isNewer(newest.version!, found);
  }

  ProgramStatus copyWith({
    List<ProgramCopy>? copies,
    ProgramLatest? latest,
    DateTime? latestAt,
    String? problem,
    bool clearProblem = false,
    bool? reported,
  }) => ProgramStatus(
    info: info,
    copies: copies ?? this.copies,
    latest: latest ?? this.latest,
    latestAt: latestAt ?? this.latestAt,
    problem: clearProblem ? null : problem ?? this.problem,
    reported: reported ?? this.reported,
  );
}

/// One file a plan fetches.
final class PlanDownload {
  const PlanDownload({required this.url, this.bytes, required this.verified, this.how = '', this.digest});

  final String url;
  final int? bytes;

  /// The checksum it is checked against, in hexadecimal, when the publisher gives one.
  final String? digest;

  /// Whether the publisher gives a checksum to check it against; false is an unverified download, which the person sees.
  final bool verified;

  /// The digest's kind when verified, else why it is not.
  final String how;
}

/// One thing a plan changes outside the host's folder for the program.
final class PlanChange {
  const PlanChange({required this.kind, required this.target, required this.detail, this.undone = false});

  final String kind, target, detail;
  final bool undone;
}

/// What an apply would do, as the host made it. [raw] is handed back unchanged, with its token, to apply it.
final class ProgramPlan {
  const ProgramPlan({
    required this.provider,
    required this.action,
    required this.raw,
    this.version,
    this.fromVersion,
    this.downloads = const [],
    this.steps = const [],
    this.changes = const [],
    this.bytes,
    this.notices = const [],
  });

  final String provider;

  /// `install`, `update` or `uninstall`.
  final String action;
  final Map<String, Object?> raw;
  final String? version, fromVersion;
  final List<PlanDownload> downloads;
  final List<String> steps;
  final List<PlanChange> changes;
  final int? bytes;
  final List<String> notices;

  bool get hasUnverified => downloads.any((each) => !each.verified);

  static ProgramPlan parse(String provider, Object? json) {
    final map = _map(json);
    return ProgramPlan(
      provider: provider,
      action: _text(map['action']) ?? '',
      raw: map,
      version: _text(map['version']),
      fromVersion: _text(map['from_version']),
      downloads: [
        for (final each in _maps(map['downloads']))
          PlanDownload(
            url: _text(each['url']) ?? '',
            bytes: each['bytes'] is int ? each['bytes'] as int : null,
            verified: _map(each['check'])['kind'] != 'unverified',
            how: _text(_map(each['check'])['why']) ?? _text(_map(each['check'])['kind']) ?? '',
            digest: _text(_map(each['check'])['value']),
          ),
      ],
      steps: [if (map['steps'] is List) for (final each in map['steps'] as List) ?_text(each)],
      changes: [
        for (final each in _maps(map['changes']))
          PlanChange(
            kind: _text(each['kind']) ?? '',
            target: _text(each['target']) ?? '',
            detail: _text(each['detail']) ?? '',
            undone: each['undone_by_uninstall'] == true,
          ),
      ],
      bytes: map['bytes'] is int ? map['bytes'] as int : null,
      notices: [if (map['notices'] is List) for (final each in map['notices'] as List) ?_text(each)],
    );
  }
}

/// A form's title and its JSON schema, for the fields of a question that carries no secret.
final class ProgramAsk {
  const ProgramAsk(this.title, this.schema);

  final String title;
  final Map<String, Object?> schema;

  /// The fields, in the schema's order.
  Map<String, Map<String, Object?>> get fields => {
    for (final MapEntry(:key, :value) in _map(schema['properties']).entries) key: _map(value),
  };

  static bool sensitive(Map<String, Object?> field) => field['x-sensitive'] == true || field['writeOnly'] == true;
}

/// The person ended an install.
final class ProgramCancelled implements Exception {
  const ProgramCancelled();

  @override
  String toString() => 'cancelled';
}

/// An install or update running on a link of its own: what it says, what it asks, and how it ended. Closing the link is how
/// the host is told to stop, so [cancel] does only that.
final class ProgramRun extends ChangeNotifier {
  ProgramRun._(this.provider, this.plan, this.askedBy);

  final String provider;
  final ProgramPlan plan;

  /// The store whose install runs here: the one a question it asks is named after, since the value typed goes to its install code.
  final String askedBy;

  /// What the install said, line by line (progress with a fraction replaces the last line that had one).
  final lines = <String>[];

  /// The latest progress, as a fraction of 0 to 1 when the host gave one.
  double? fraction;

  /// The question waiting for the person, if any: a form the host sealed to this link's key, as a sign-in's is.
  SignInAsk? asking;
  Completer<AcpSecretRequestReply>? _answer;
  ControlClient? _client;
  bool _cancelled = false;
  bool _finished = false;

  /// Whether it has ended, however.
  bool get finished => _finished;

  /// How it ended: the version and folder it applied, or a [ControlFailed], a [ProgramCancelled] or another error.
  late final Future<Map<String, Object?>> done;

  /// Answers the question waiting with [reply], which the form sealed: the host is told once.
  void answer(AcpSecretRequestReply reply) {
    final waiting = _answer;
    if (waiting == null || waiting.isCompleted) return;
    _answer = null;
    asking = null;
    waiting.complete(reply);
    notifyListeners();
  }

  /// Ends the install: the link to the host closes, which cancels it there.
  Future<void> cancel() async {
    if (_finished) return;
    _cancelled = true;
    if (_answer case final waiting? when !waiting.isCompleted) waiting.complete(const AcpSecretCancelled());
    await _client?.close();
  }

  /// Takes a question back once the install is over, so the card does not outlive it.
  void _endQuestion() {
    final waiting = _answer;
    _answer = null;
    if (waiting != null && !waiting.isCompleted) waiting.complete(const AcpSecretCancelled());
    if (asking != null) {
      asking = null;
      notifyListeners();
    }
  }

  void _heard(String method, Map<String, Object?> params) {
    if (method != 'store/progress') return;
    final said = _text(params['text']);
    if (said != null && said.isNotEmpty) {
      if (params['fraction'] is num && lines.isNotEmpty && _lastWasProgress) {
        lines.last = said;
      } else {
        lines.add(said);
      }
      _lastWasProgress = params['fraction'] is num;
    }
    if (params['fraction'] is num) fraction = (params['fraction'] as num).toDouble().clamp(0.0, 1.0);
    notifyListeners();
  }

  bool _lastWasProgress = false;

  Future<Object?> _asked(LinkSecretIdentity? secrets, String method, Map<String, Object?> params) async {
    if (method != acpSecretRequestMethod) throw StateError('the caller answers no $method');
    final asked = await openSealedAsk(secrets, params, () => DateTime.now().millisecondsSinceEpoch ~/ 1000);
    if (asked == null || _cancelled) return const AcpSecretCancelled().toJson();
    final waiting = Completer<AcpSecretRequestReply>();
    _answer = waiting;
    asking = asked;
    notifyListeners();
    return (await waiting.future).toJson();
  }
}

/// How long an install may run before the app gives up on its answer: the host ends one that nobody answers after four minutes
/// of a question, and downloads take as long as they take.
const Duration applyWait = Duration(minutes: 30);

/// The programs the host's stores keep, and what the app asks of them. Constructing this asks nothing: the host is asked when a
/// page that shows programs opens ([refresh]), and an install runs only when the person confirms a plan ([apply]).
class ProgramStores extends ChangeNotifier {
  ProgramStores({
    required this._control,
    required this._open,
    DateTime Function()? now,
    this.latestLife = const Duration(hours: 1),
    this.afterChange,
    this.kind,
  }) : _now = now ?? DateTime.now;

  final Future<ControlClient> Function() _control;
  final Future<ControlClient> Function() _open;
  final DateTime Function() _now;

  /// How long what the publisher said of the newest version is kept before a page asks again.
  final Duration latestLife;

  /// Runs once a program was installed, updated or removed: the host reads which program a source runs when it starts, so the app has
  /// it start again here.
  final Future<void> Function()? afterChange;

  /// Which stores the host lists: `programs` for every program a library offers other than an agent's; none lists the agents'.
  final String? kind;

  final _status = <String, ProgramStatus>{};
  List<String> _order = const [];
  bool _loading = false;
  String? _problem;

  /// What the page shows of every store, as one text: it changes only when something a source's readiness rests on does.
  String get signature => [
    for (final each in statuses) '${each.info.provider} ${each.copies.map((copy) => '${copy.path}@${copy.version}').join(',')} ${each.latest?.version} ${each.problem}',
  ].join(';');

  /// Every store the host offers, in its order.
  List<ProgramStatus> get statuses => [for (final id in _order) ?_status[id]];

  /// Why the stores could not be listed.
  String? get problem => _problem;
  bool get loading => _loading;
  ProgramStatus? status(String provider) => _status[provider];

  /// The store whose program a source runs.
  ProgramStatus? forSource({String? harness, required String label}) =>
      statuses.where((each) => each.info.servesSource(harness: harness, label: label)).firstOrNull;

  /// Lists the stores and asks each for its copies (no network, and no program is run). With [latest], also asks the
  /// publishers for the newest versions, those not asked within [latestLife] only unless [force].
  Future<void> refresh({bool latest = false, bool force = false}) {
    final before = _running;
    final next = (() async {
      if (before != null) await before;
      await _refresh(latest: latest, force: force);
    })();
    late final Future<void> chained;
    chained = next.whenComplete(() {
      if (identical(_running, chained)) _running = null;
    });
    return _running = chained;
  }

  Future<void>? _running, _ensuring;
  bool _listed = false;

  /// Reads the copies if the host has not been asked yet: what a readiness check needs, shared by checks made together.
  Future<void> ensure() {
    if (_listed || _running != null) return _running ?? Future<void>.value();
    return _ensuring ??= refresh().whenComplete(() => _ensuring = null);
  }

  Future<void> _refresh({required bool latest, required bool force}) async {
    _loading = true;
    notifyListeners();
    try {
      final client = await _control();
      // One round trip gives every store with its copies and the newest version the host kept; only what the host has no fresh
      // answer for is asked of a publisher, each on its own so that a slow one holds up no other.
      final listed = _map(await client.call('store/overview', {'kind': ?kind}));
      final rows = [for (final each in listed['stores'] is List ? listed['stores'] as List : const []) if (each is Map) _map(each)];
      final infos = <ProgramStoreInfo>[];
      final ask = <String, bool>{};
      for (final row in rows) {
        final info = ProgramStoreInfo.parse(row);
        if (info == null) continue;
        infos.add(info);
        final before = _status[info.provider];
        var next = ProgramStatus(info: info, latest: before?.latest, latestAt: before?.latestAt);
        if (row['report'] is Map) {
          next = next.copyWith(
            copies: [if (_map(row['report'])['copies'] is List) for (final each in _map(row['report'])['copies'] as List) ?ProgramCopy.parse(each)],
            reported: true,
            clearProblem: true,
          );
        } else {
          next = next.copyWith(problem: _text(row['problem']) ?? 'the host could not read this program', reported: true);
        }
        var hostFresh = false;
        if (row['latest'] is Map) {
          final kept = _map(row['latest']);
          hostFresh = kept['stale'] != true;
          next = next.copyWith(
            latest: ProgramLatest.parse(kept),
            latestAt: kept['checked'] is int ? DateTime.fromMillisecondsSinceEpoch(kept['checked'] as int) : _now(),
          );
        }
        final appFresh = before?.latestAt != null && _now().difference(before!.latestAt!) < latestLife;
        if (latest && next.problem == null && (force || !(hostFresh || appFresh))) ask[info.provider] = force || row['latest'] is Map;
        _status[info.provider] = next;
      }
      _order = [for (final each in infos) each.provider];
      _listed = true;
      _problem = null;
      // The rows are there to read now; each fills in as its own answer arrives, and one that never does ends only its own row.
      notifyListeners();
      await Future.wait([for (final MapEntry(:key, :value) in ask.entries) _askLatest(client, key, refresh: value)]);
    } on Object catch (error) {
      _problem = error is ControlFailed ? error.message : '$error';
    } finally {
      _loading = false;

      notifyListeners();
    }
  }

  /// The publisher's newest version of one program, shown as soon as it is answered. [refresh] has the host ask the publisher
  /// now, where it would otherwise answer from what it kept.
  Future<void> _askLatest(ControlClient client, String provider, {required bool refresh}) async {
    try {
      final answer = _map(await client.call('store/latest', {'provider': provider, 'refresh': refresh}));
      _status[provider] = _status[provider]!.copyWith(
        latest: ProgramLatest.parse(answer),
        latestAt: answer['checked'] is int ? DateTime.fromMillisecondsSinceEpoch(answer['checked'] as int) : _now(),
      );
    } on ControlFailed catch (error) {
      _status[provider] = _status[provider]!.copyWith(problem: error.message);
    } on TimeoutException {
      // Not remembered as asked, so the next page asks again.
      _status[provider] = _status[provider]!.copyWith(latest: const ProgramLatest(unknownBecause: 'the publisher did not answer in time'));
    }
    notifyListeners();
  }

  /// What [action] (`install`, `update` or `uninstall`) of [provider] would do, to [version] where the publisher serves more
  /// than the newest and to [channel] where the program has several (a speech model, say): nothing is written. Throws [ControlFailed].
  Future<ProgramPlan> plan(String provider, String action, {String? version, String? channel}) async {
    final client = await _control();
    return ProgramPlan.parse(
      provider,
      await client.call('store/plan', {'provider': provider, 'action': action, 'version': ?version, 'channel': ?channel}),
    );
  }

  /// Carries [plan] out on a link of its own. The run reports as it goes; [ProgramRun.done] ends it. The statuses are read
  /// again once it ends, whatever way.
  ProgramRun apply(ProgramPlan plan) {
    final run = ProgramRun._(plan.provider, plan, _status[plan.provider]?.info.title ?? plan.provider);
    var changed = false;
    run.done = (() async {
      try {
        final client = run._client = await _open();
        if (run._cancelled) {
          await client.close();
          throw const ProgramCancelled();
        }
        final secrets = await client.linkSecrets();
        final result = await client.call(
          'store/apply',
          {'provider': plan.provider, 'plan': plan.raw},
          ControlListener(onNotification: run._heard, onRequest: (method, params) => run._asked(secrets, method, params)),
          applyWait,
        );
        changed = _map(result)['applied'] != false;
        return _map(result);
      } on Object {
        if (run._cancelled) throw const ProgramCancelled();
        rethrow;
      } finally {
        run._finished = true;
        run._endQuestion();
        unawaited(run._client?.close());
        // Readiness is read again before the run is over, so whoever awaits it sees the program as it now is.
        await _afterChange(plan.provider);
        run.notifyListeners();
        if (changed) unawaited(afterChange?.call());
      }
    })();
    // The run's owner reads the failure from done; an unread one is not an error of the app.
    run.done.ignore();
    return run;
  }

  /// Puts the host's copy back to the version before. Throws [ControlFailed].
  Future<String?> rollback(String provider) async {
    var changed = false;
    try {
      final result = _map(await (await _control()).call('store/rollback', {'provider': provider}));
      changed = true;
      return _text(result['version']);
    } finally {
      await _afterChange(provider);
      if (changed) unawaited(afterChange?.call());
    }
  }

  /// Takes the host's copy of [provider] away; a copy found elsewhere on the computer stays. Throws [ControlFailed].
  Future<void> remove(String provider) async {
    try {
      await (await _control()).call('store/remove', {'provider': provider});
    } finally {
      await _afterChange(provider);
    }
  }

  Future<void> _afterChange(String provider) async {
    final before = _status[provider];
    if (before == null) {
      await refresh();
      return;
    }
    try {
      final client = await _control();
      final report = _map(await client.call('store/report', {'provider': provider}));
      _status[provider] = before.copyWith(
        copies: [if (report['copies'] is List) for (final each in report['copies'] as List) ?ProgramCopy.parse(each)],
        clearProblem: true,
        reported: true,
      );
    } on Object catch (error) {
      _status[provider] = before.copyWith(problem: error is ControlFailed ? error.message : '$error');
    }
    notifyListeners();
  }
}
