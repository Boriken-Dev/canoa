import 'dart:async';

import 'package:coqui_acp/secret.dart';
import 'package:flutter/foundation.dart';

import '../acp/link_secret.dart';
import '../accounts/accounts_host.dart' show SignInAsk, openSealedAsk;
import '../desktop/control_client.dart';
import '../desktop/host_control.dart';

/// One secret the host holds or keeps, as the table shows it: where it is kept, what it is called and how long it has left. Never its value.
final class SecretRow {
  const SecretRow({required this.provider, required this.name, required this.kept, this.session, this.expiresIn});

  /// The host's provider entry that holds it (`form` for what the person typed, `system` for the operating system's store).
  final String provider;
  final String name;

  /// `session` (until the conversation ends), `process` (until Canoa closes, or until [expiresIn] runs out) or `stored` (in a store).
  final String kept;

  /// The conversation a `session` row belongs to.
  final String? session;

  /// Seconds the host said this has left when it listed it; null when it lasts as long as it is kept.
  final int? expiresIn;

  String get key => '$provider/$name';

  static SecretRow? parse(Object? json) {
    if (json is! Map) return null;
    final provider = json['provider'], name = json['name'], kept = json['kept'];
    if (provider is! String || name is! String || kept is! String) return null;
    return SecretRow(
      provider: provider,
      name: name,
      kept: kept,
      session: json['session'] as String?,
      expiresIn: json['expires_in'] is int ? json['expires_in'] as int : null,
    );
  }
}

/// A way to keep what is typed, as the host offers it.
final class SecretKeep {
  const SecretKeep(this.id, this.label);

  final String id, label;
}

/// A provider entry of the host's `[secrets.providers]`, or one it has without being told, and what its provider can do.
final class SecretEntry {
  const SecretEntry({required this.name, required this.provider, required this.title, required this.canStore, required this.canList, required this.needsPerson, this.keep});

  final String name, provider, title;
  final bool canStore, canList, needsPerson;
  final String? keep;

  /// Whether a value can be set in it by hand: it keeps what it is given, or it is asked in the app and kept in memory.
  bool get acceptsValues => canStore || needsPerson;
}

final class SecretsListing {
  const SecretsListing({required this.rows, required this.keeps});

  final List<SecretRow> rows;
  final List<SecretKeep> keeps;
}

/// A secret being set: the host asks for the value in a form sealed to this app's key, and says how it ended.
class SecretSet extends ChangeNotifier {
  SecretSet({required this.provider, required this.name, this.onCancel});

  final String provider, name;

  /// Ends what the host is doing, which closing its link does.
  final void Function()? onCancel;

  SignInAsk? asking;
  bool done = false;
  String? problem;
  bool cancelled = false;
  Completer<AcpSecretRequestReply>? _reply;

  bool get running => !done && problem == null && !cancelled;

  /// Puts [request] to the person; completes with their sealed answer, or cancelled when they give up.
  Future<AcpSecretRequestReply> ask(SignInAsk request) {
    if (!running) return Future.value(const AcpSecretCancelled());
    final before = _reply;
    if (before != null && !before.isCompleted) before.complete(const AcpSecretCancelled());
    final reply = _reply = Completer();
    asking = request;
    notifyListeners();
    return reply.future;
  }

  void complete() {
    if (!running) return;
    done = true;
    asking = null;
    notifyListeners();
  }

  void fail(String why) {
    if (!running) return;
    problem = why;
    asking = null;
    if (_reply case final waiting? when !waiting.isCompleted) waiting.complete(const AcpSecretCancelled());
    notifyListeners();
  }

  /// The person's answer to [asking]; the host is told once.
  void answer(AcpSecretRequestReply reply) {
    final waiting = _reply;
    if (waiting == null || waiting.isCompleted) return;
    asking = null;
    waiting.complete(reply);
    notifyListeners();
  }

  /// Gives up: what is asked is cancelled and the host is told to stop.
  void cancel() {
    if (!running) return;
    cancelled = true;
    asking = null;
    if (_reply case final waiting? when !waiting.isCompleted) waiting.complete(const AcpSecretCancelled());
    onCancel?.call();
    notifyListeners();
  }
}

/// What the app asks of the host about its secrets. A list holds no value; [reveal] is the one call that answers one.
abstract interface class SecretsHost {
  Future<SecretsListing> list();

  /// The provider entries and what each can do.
  Future<List<SecretEntry>> entries();

  /// The value of [name] in [provider]'s entry, for the person who asked to see it.
  Future<String> reveal(String provider, String name);

  /// Forgets what the host holds as [name] in [provider]'s entry; whether it held any.
  Future<bool> forget(String provider, String name);

  /// Sets [name] in [provider]'s entry: the host asks for the value in a form only it and this app can read.
  SecretSet set({required String provider, required String name});
}

/// How long a set may wait for the person: the host gives up on a question after four minutes.
const Duration secretSetWait = Duration(minutes: 11);

/// The host's secrets over its control channel: this process's own link asks.
final class ControlSecretsHost implements SecretsHost {
  ControlSecretsHost(this._control, {DateTime Function()? now}) : _now = now ?? DateTime.now;

  final HostControl _control;
  final DateTime Function() _now;

  Future<Object?> _call(String method, Map<String, Object?> params) async => (await _control.ready(wait: false)).call(method, params);

  @override
  Future<SecretsListing> list() async {
    final result = await _call('secrets/list', const {});
    final map = result is Map ? result : const {};
    return SecretsListing(
      rows: [if (map['secrets'] case final List<Object?> rows) for (final each in rows) ?SecretRow.parse(each)],
      keeps: [
        if (map['keeps'] case final List<Object?> keeps)
          for (final each in keeps)
            if (each is Map && each['id'] is String && each['label'] is String) SecretKeep(each['id']! as String, each['label']! as String),
      ],
    );
  }

  @override
  Future<List<SecretEntry>> entries() async {
    final result = await _call('secrets/providers', const {});
    final map = result is Map ? result : const {};
    final titles = <String, Map<Object?, Object?>>{
      if (map['providers'] case final List<Object?> providers)
        for (final each in providers)
          if (each is Map && each['id'] is String) each['id']! as String: each,
    };
    return [
      if (map['entries'] case final List<Object?> entries)
        for (final each in entries)
          if (each is Map && each['name'] is String && each['provider'] is String)
            SecretEntry(
              name: each['name']! as String,
              provider: each['provider']! as String,
              title: (titles[each['provider']]?['title'] as String?) ?? each['provider']! as String,
              canStore: titles[each['provider']]?['can_store'] == true,
              canList: titles[each['provider']]?['can_list'] == true,
              needsPerson: titles[each['provider']]?['needs_person'] == true,
              keep: each['keep'] as String?,
            ),
    ];
  }

  @override
  Future<String> reveal(String provider, String name) async {
    final result = await _call('secrets/reveal', {'provider': provider, 'name': name});
    final value = result is Map ? result['value'] : null;
    if (value is! String) throw const ControlFailed(-32002, 'the host did not answer with a value');
    return value;
  }

  @override
  Future<bool> forget(String provider, String name) async {
    final result = await _call('secrets/forget', {'provider': provider, 'name': name});
    return result is Map && result['forgot'] == true;
  }

  @override
  SecretSet set({required String provider, required String name}) {
    ControlClient? client;
    final run = SecretSet(provider: provider, name: name, onCancel: () => unawaited(client?.close()));
    unawaited(() async {
      try {
        // A link of its own: closing it is how the host is told to stop.
        client = await _control.open(wait: false);
        if (!run.running) return;
        final secrets = await client!.linkSecrets();
        await client!.call(
          'secrets/set',
          {'provider': provider, 'name': name},
          ControlListener(onRequest: (method, params) => _asked(run, secrets, method, params)),
          secretSetWait,
        );
        run.complete();
      } on ControlFailed catch (error) {
        run.fail(error.message);
      } on Object catch (error) {
        // A cancel closes the link, which ends the request with an error that is not news.
        if (run.running) run.fail('$error');
      } finally {
        unawaited(client?.close());
      }
    }());
    return run;
  }

  Future<Object?> _asked(SecretSet run, LinkSecretIdentity? secrets, String method, Map<String, Object?> params) async {
    if (method != acpSecretRequestMethod) throw StateError('the caller answers no $method');
    final asked = await openSealedAsk(secrets, params, () => _now().millisecondsSinceEpoch ~/ 1000);
    if (asked == null) return const AcpSecretCancelled().toJson();
    return (await run.ask(asked)).toJson();
  }
}
