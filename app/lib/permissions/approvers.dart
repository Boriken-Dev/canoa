import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../brand.dart';

/// The name of the entry that stands for the window a session runs in: it takes a timeout and nothing else.
const String windowApprover = 'window';

/// The words an approver's name is made of: 1 to 32 lower-case letters, digits, `-` and `_`.
final RegExp _name = RegExp(r'^[a-z0-9_-]{1,32}$');

/// Whether [name] can name an approver.
bool approverNameOk(String name) => _name.hasMatch(name) && name != windowApprover;

/// One `[permissions.approvers.NAME]`: somewhere else a permission request is answered from, besides the window.
@immutable
final class ApproverEntry {
  const ApproverEntry({
    required this.name,
    required this.provider,
    this.settings = const {},
    this.notifier,
    this.detail = 'tool',
    this.timeoutSeconds,
    this.onlyWhenAway = false,
  });

  final String name;

  /// The id of the `approver` provider that answers for it.
  final String provider;

  /// The provider's own settings.
  final Map<String, Object?> settings;

  /// The notifier a phone approver sends through.
  final String? notifier;

  /// How much of a request it is shown: `tool` (the kind, name and a one-line title) or `command` (also the command and files).
  final String detail;

  /// How long it is waited for; null waits until the session or turn ends.
  final int? timeoutSeconds;

  /// Whether it is asked only while no window attends the session.
  final bool onlyWhenAway;

  ApproverEntry copyWith({
    String? provider,
    Map<String, Object?>? settings,
    String? detail,
    int? timeoutSeconds,
    bool clearTimeout = false,
    bool? onlyWhenAway,
  }) => ApproverEntry(
    name: name,
    provider: provider ?? this.provider,
    settings: settings ?? this.settings,
    notifier: notifier,
    detail: detail ?? this.detail,
    timeoutSeconds: clearTimeout ? null : timeoutSeconds ?? this.timeoutSeconds,
    onlyWhenAway: onlyWhenAway ?? this.onlyWhenAway,
  );

  @override
  bool operator ==(Object other) =>
      other is ApproverEntry &&
      other.name == name &&
      other.provider == provider &&
      other.notifier == notifier &&
      other.detail == detail &&
      other.timeoutSeconds == timeoutSeconds &&
      other.onlyWhenAway == onlyWhenAway &&
      jsonEncode(other.settings) == jsonEncode(settings);

  @override
  int get hashCode => Object.hash(name, provider, notifier, detail, timeoutSeconds, onlyWhenAway, jsonEncode(settings));
}

/// What `[permissions.approvers]` holds: the entries in the file's order, and the window's own timeout.
@immutable
final class Approvers {
  const Approvers({this.entries = const [], this.windowTimeoutSeconds});

  final List<ApproverEntry> entries;

  /// `[permissions.approvers.window] timeout_seconds`: a request the window leaves unanswered this long is refused.
  final int? windowTimeoutSeconds;

  bool get isEmpty => entries.isEmpty && windowTimeoutSeconds == null;

  /// Reads the table [raw]; throws [FormatException] naming the entry and never a value.
  factory Approvers.parse(Object? raw) {
    if (raw == null) return const Approvers();
    if (raw is! Map) throw const FormatException('${Brand.configFile}: permissions.approvers must be a table');
    int? seconds(String name, Map<dynamic, dynamic> table) {
      final value = table['timeout_seconds'];
      if (value == null) return null;
      if (value is! int || value < 1) {
        throw FormatException('${Brand.configFile}: permissions.approvers.$name.timeout_seconds must be a whole number of at least 1');
      }
      return value;
    }

    final entries = <ApproverEntry>[];
    int? window;
    for (final MapEntry(key: name, value: table) in raw.entries) {
      if (name is! String || table is! Map) {
        throw const FormatException('${Brand.configFile}: each permissions.approvers entry must be a table');
      }
      if (name == windowApprover) {
        window = seconds(name, table);
        continue;
      }
      if (!approverNameOk(name)) {
        throw FormatException('${Brand.configFile}: permissions.approvers.$name: a name is 1 to 32 lower-case letters, digits, - and _');
      }
      final provider = table['provider'];
      if (provider is! String || provider.trim().isEmpty) {
        throw FormatException('${Brand.configFile}: permissions.approvers.$name.provider must name a provider');
      }
      final detail = table['detail'] ?? 'tool';
      if (detail != 'tool' && detail != 'command') {
        throw FormatException('${Brand.configFile}: permissions.approvers.$name.detail must be "tool" or "command"');
      }
      final settings = table['settings'];
      if (settings != null && settings is! Map) {
        throw FormatException('${Brand.configFile}: permissions.approvers.$name.settings must be a table');
      }
      final notifier = table['notifier'];
      entries.add(ApproverEntry(
        name: name,
        provider: provider,
        settings: settings == null ? const {} : {for (final e in (settings as Map).entries) '${e.key}': e.value},
        notifier: notifier is String && notifier.isNotEmpty ? notifier : null,
        detail: detail as String,
        timeoutSeconds: seconds(name, table),
        onlyWhenAway: table['only_when_away'] == true,
      ));
    }
    return Approvers(entries: entries, windowTimeoutSeconds: window);
  }

  /// The tables as TOML, to follow `[permissions]`.
  String toToml() {
    final out = StringBuffer();
    for (final entry in entries) {
      out
        ..write('\n[permissions.approvers.${entry.name}]\n')
        ..write('provider = ${jsonEncode(entry.provider)}\n');
      if (entry.settings.isNotEmpty) out.write('settings = ${inlineToml(entry.settings)}\n');
      if (entry.notifier != null) out.write('notifier = ${jsonEncode(entry.notifier)}\n');
      if (entry.detail != 'tool') out.write('detail = ${jsonEncode(entry.detail)}\n');
      if (entry.timeoutSeconds != null) out.write('timeout_seconds = ${entry.timeoutSeconds}\n');
      if (entry.onlyWhenAway) out.write('only_when_away = true\n');
    }
    if (windowTimeoutSeconds != null) {
      out.write('\n[permissions.approvers.$windowApprover]\ntimeout_seconds = $windowTimeoutSeconds\n');
    }
    return out.toString();
  }

  @override
  bool operator ==(Object other) =>
      other is Approvers && listEquals(other.entries, entries) && other.windowTimeoutSeconds == windowTimeoutSeconds;

  @override
  int get hashCode => Object.hash(Object.hashAll(entries), windowTimeoutSeconds);
}

/// [value] as a TOML inline value: a table, an array, text, a number or a flag.
String inlineToml(Object? value) => switch (value) {
  final Map<dynamic, dynamic> map =>
    map.isEmpty ? '{}' : '{ ${[for (final e in map.entries) '${jsonEncode('${e.key}')} = ${inlineToml(e.value)}'].join(', ')} }',
  final List<dynamic> list => '[${list.map(inlineToml).join(', ')}]',
  final String text => jsonEncode(text),
  final bool flag => '$flag',
  final num number => '$number',
  _ => throw FormatException('a setting of an approver cannot hold ${value.runtimeType}'),
};
