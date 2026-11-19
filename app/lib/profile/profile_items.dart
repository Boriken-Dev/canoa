/// The items a profile may set, as the host lists them: each is a provider the host loaded (the ones Canoa carries, the ones that ship
/// with it and the ones an extension adds), and the app knows an item only by what its provider says of it: its name, the harnesses
/// it applies to and the form it asks for, a guatiao schema and the guatiao-intake form that shows it.
library;

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../forms/schema_fields.dart' show SchemaChoice, formHasWidget, schemaProperties;

/// One thing a profile may set.
final class ProfileItemInfo {
  const ProfileItemInfo({
    required this.id,
    required this.title,
    required this.origin,
    this.help,
    this.stored = 'item',
    this.harnesses = const [],
    this.schema = const {},
    this.form = const {},
  });

  /// The id the profile keeps its values under.
  final String id;
  final String title;
  final String? help;

  /// Where its provider comes from: `built-in`, `bundled`, or the name of the extension or entry that loads it.
  final String origin;

  /// `setting` for an item that is one of the session's own settings (kept in the profile under its own name), else `item`.
  final String stored;

  /// The ids of the harnesses it applies to; none means every harness.
  final List<String> harnesses;

  /// The values it takes, as a JSON Schema of an object; empty for an item that takes none.
  final Map<String, Object?> schema;

  /// How the schema is drawn (sections, widgets, `visibleWhen`).
  final Map<String, Object?> form;

  /// Whether it asks for anything.
  bool get hasForm => schemaProperties(schema).isNotEmpty;

  bool get isSetting => stored == 'setting';

  /// Whether a member of its form is one of the host's client certificates, so the list of them is wanted to draw it.
  bool get asksForClientCertificate => formHasWidget(form, clientCertificateWidget);

  static ProfileItemInfo? read(Object? raw) {
    if (raw is! Map || raw['id'] is! String || raw['title'] is! String) return null;
    return ProfileItemInfo(
      id: raw['id'] as String,
      title: raw['title'] as String,
      origin: raw['origin'] as String? ?? '',
      help: raw['help'] as String?,
      stored: raw['stored'] as String? ?? 'item',
      harnesses: [if (raw['harnesses'] is List) for (final each in raw['harnesses'] as List) if (each is String) each],
      schema: raw['schema'] is Map ? (raw['schema'] as Map).cast<String, Object?>() : const {},
      form: raw['form'] is Map ? (raw['form'] as Map).cast<String, Object?>() : const {},
    );
  }
}

/// The items the host lists.
final class ProfileItemCatalog extends ChangeNotifier {
  List<ProfileItemInfo> _items = const [];
  bool _answered = false;
  String? _problem;

  /// The items in the order the host listed them.
  List<ProfileItemInfo> get items => _items;

  /// Whether the host has answered, whatever it listed.
  bool get answered => _answered;

  /// Why the host could not be asked, or null.
  String? get problem => _problem;

  /// Whether any item's form asks for a client certificate.
  bool get asksForClientCertificate => _items.any((each) => each.asksForClientCertificate);

  ProfileItemInfo? byId(String id) {
    for (final each in _items) {
      if (each.id == id) return each;
    }
    return null;
  }

  /// Takes the host's answer to `profile_items/list`.
  void adopt(Object? answer) {
    final raw = answer is Map ? answer['items'] : null;
    _items = [if (raw is List) for (final each in raw) ?ProfileItemInfo.read(each)];
    _answered = true;
    _problem = null;
    notifyListeners();
  }

  /// The host could not be asked.
  void failed(String why) {
    _problem = why;
    _answered = true;
    notifyListeners();
  }
}

/// Asks the host for its items again when the extensions it has loaded are not the ones the last answer was made under, so an item
/// an extension adds or takes away shows without a reload of the app.
final class ProfileItemsRefresh {
  ProfileItemsRefresh(this._ask);

  final Future<void> Function() _ask;
  String? _under;

  /// [signature] is what the extensions the host reports amount to. The first one only notes what the answer was made under; each
  /// later one that differs asks again.
  void extensionsAre(String signature) {
    final before = _under;
    _under = signature;
    if (before != null && before != signature) unawaited(_ask());
  }
}

/// The items this app's host lists.
final ProfileItemCatalog profileItems = ProfileItemCatalog();

/// The widget hint a form gives a member that is one of the client certificates the host can present.
const clientCertificateWidget = 'client_certificate';

/// What a profile's form offers where it asks for a client certificate: the keystores' own choice, then the certificates the host
/// lists with `network/client_certificates`, as a browser's picker does.
final class ClientCertificateChoices extends ChangeNotifier {
  /// The value that leaves the choice to the keystores.
  static const system = 'system';

  List<SchemaChoice> _certificates = const [];

  static const _first = SchemaChoice(
    value: system,
    label: 'System (default)',
    detail: "The keystores' own choice, by the authorities the server names",
  );

  /// The choices: the keystores' own, then one for each certificate the host listed, which a form's value names by thumbprint.
  List<SchemaChoice> get choices => [_first, ..._certificates];

  /// Takes the host's answer to `network/client_certificates`; one that is not an answer (a refusal) leaves only the keystores' own.
  void adopt(Object? answer, {DateTime? now}) {
    final raw = answer is Map ? answer['certificates'] : null;
    final today = now ?? DateTime.now();
    _certificates = [
      if (raw is List)
        for (final each in raw)
          if (each is Map && each['thumbprint'] is String) _choice(each, today),
    ];
    notifyListeners();
  }

  static SchemaChoice _choice(Map<Object?, Object?> each, DateTime now) {
    final subject = each['subject'] is String && (each['subject']! as String).isNotEmpty ? each['subject']! as String : 'No subject';
    final thumbprint = each['thumbprint']! as String;
    final notAfter = each['not_after'];
    final ends = notAfter is num ? DateTime.fromMillisecondsSinceEpoch(notAfter.toInt() * 1000, isUtc: true) : null;
    final date = ends == null ? null : '${ends.year}-${ends.month.toString().padLeft(2, '0')}-${ends.day.toString().padLeft(2, '0')}';
    final expiry = date == null ? null : (ends!.isBefore(now.toUtc()) ? 'expired $date' : 'expires $date');
    final issuer = each['issuer'] is String && (each['issuer']! as String).isNotEmpty ? 'issued by ${each['issuer']}' : null;
    final source = each['source'] is String && (each['source']! as String).isNotEmpty ? each['source']! as String : null;
    return SchemaChoice(
      value: thumbprint,
      label: subject,
      detail: [?issuer, ?expiry, ?source, 'SHA-256 ${thumbprint.length > 16 ? '${thumbprint.substring(0, 16)}…' : thumbprint}'].join(' · '),
    );
  }

  /// The host could not be asked or would not answer: only the keystores' own choice is offered.
  void failed() => adopt(null);
}

/// The certificates this app's host lists.
final ClientCertificateChoices profileCertificates = ClientCertificateChoices();
