/// The items a profile may set, as the host lists them: each is a provider the host loaded (the ones Canoa carries, the ones that ship
/// with it and the ones an extension adds), and the app knows an item only by what its provider says of it: its name, the harnesses
/// it applies to and the form it asks for.
library;

import 'dart:async';

import 'package:flutter/foundation.dart';

/// One field of an item's form.
final class ItemField {
  const ItemField({
    required this.name,
    required this.label,
    required this.shape,
    this.required = false,
    this.defaultValue,
    this.help,
    this.choices = const [],
  });

  /// The key its value is kept under.
  final String name;
  final String label;

  /// `text`, `list`, `number`, `path`, `bool` or `choice`.
  final String shape;
  final bool required;

  /// What it holds when left empty, as the form shows it.
  final String? defaultValue;
  final String? help;

  /// The words a `choice` may be.
  final List<String> choices;

  static ItemField? read(Object? raw) {
    if (raw is! Map || raw['name'] is! String || raw['label'] is! String || raw['shape'] is! String) return null;
    return ItemField(
      name: raw['name'] as String,
      label: raw['label'] as String,
      shape: raw['shape'] as String,
      required: raw['required'] == true,
      defaultValue: raw['default'] as String?,
      help: raw['help'] as String?,
      choices: [if (raw['choices'] is List) for (final each in raw['choices'] as List) if (each is String) each],
    );
  }
}

/// One thing a profile may set.
final class ProfileItemInfo {
  const ProfileItemInfo({
    required this.id,
    required this.title,
    required this.origin,
    this.help,
    this.stored = 'item',
    this.harnesses = const [],
    this.fields = const [],
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
  final List<ItemField> fields;

  bool get isSetting => stored == 'setting';

  static ProfileItemInfo? read(Object? raw) {
    if (raw is! Map || raw['id'] is! String || raw['title'] is! String) return null;
    return ProfileItemInfo(
      id: raw['id'] as String,
      title: raw['title'] as String,
      origin: raw['origin'] as String? ?? '',
      help: raw['help'] as String?,
      stored: raw['stored'] as String? ?? 'item',
      harnesses: [if (raw['harnesses'] is List) for (final each in raw['harnesses'] as List) if (each is String) each],
      fields: [if (raw['fields'] is List) for (final each in raw['fields'] as List) ?ItemField.read(each)],
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
