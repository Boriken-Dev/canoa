import 'package:flutter/foundation.dart';

import '../sources/agent_marks.dart';

/// One mode an agent is known to offer before it has run.
final class ProviderMode {
  const ProviderMode(this.id, this.title);

  final String id, title;
}

/// What an agent's account provider offers: the kinds of account and the ways to sign in, by id and name.
final class ProviderAccounts {
  const ProviderAccounts({
    required this.provider,
    required this.title,
    this.description = '',
    this.kinds = const [],
    this.logins = const [],
    this.usage = const [],
  });

  /// The account provider's own id.
  final String provider;
  final String title, description;

  /// `(id, title)` of each kind of account.
  final List<(String, String)> kinds;

  /// `(id, title, produces)` of each way to sign in.
  final List<(String, String, String)> logins;

  /// The ids of the kinds whose accounts report their plan's usage.
  final List<String> usage;
}

/// One agent the host has loaded, as the providers it loaded say of it: nothing in the app names an agent, so what is here is all
/// the app knows of one.
final class AgentProvider {
  const AgentProvider({
    required this.id,
    required this.title,
    required this.program,
    this.vendor,
    this.aos,
    this.modelVariable,
    this.standards = const {},
    this.names = const [],
    this.markers = const [],
    this.modes = const [],
    this.mark,
    this.look,
    this.harness = false,
    this.imports = false,
    this.accounts,
    this.source,
    this.launch,
    this.origin = '',
  });

  /// The id the agent goes by: its harness provider's when it has one, else its source's name in lower case.
  final String id;

  /// Its name for people, and its program's.
  final String title, program;

  /// The vendor a session on it is stamped with, and its id in the standard registry of harnesses.
  final String? vendor, aos;

  /// The variable its running model is read from.
  final String? modelVariable;

  /// The credential standards that are the vendor's own, with their names.
  final Map<String, String> standards;

  /// The names it reports in `initialize`.
  final List<String> names;

  /// The `_meta` keys that are `true` for this agent alone.
  final List<String> markers;
  final List<ProviderMode> modes;

  /// Its mark, or null for the generic one.
  final AgentMark? mark;

  /// The id of the look document it carries, when it has one.
  final String? look;

  /// Whether the host runs a harness provider for it, which signs sessions in and reads them.
  final bool harness;

  /// Whether the host can bring in the sessions the agent's own program kept on this computer.
  final bool imports;

  /// What its account provider offers; null when it has none.
  final ProviderAccounts? accounts;

  /// The name of the source the host serves for it of its own, when it does.
  final String? source;

  /// How the host starts the program of an agent that reports its own quota, and the reader that reads it: `(program, args, usage)`.
  final (String, List<String>, String)? launch;

  /// Where its providers come from: `bundled`, or the entry or extension that loads them.
  final String origin;

  static AgentProvider? parse(Object? json) {
    if (json is! Map) return null;
    String? text(String key) => json[key] is String && (json[key] as String).isNotEmpty ? json[key] as String : null;
    final id = text('id');
    if (id == null) return null;
    List<Map<Object?, Object?>> maps(Object? value) => [if (value is List) for (final each in value) if (each is Map) each];
    final standards = {for (final each in maps(json['standards'])) if (each['id'] is String) each['id'] as String: '${each['title'] ?? each['id']}'};
    final accounts = json['accounts'];
    return AgentProvider(
      id: id,
      title: text('title') ?? id,
      program: text('program') ?? text('title') ?? id,
      vendor: text('vendor'),
      aos: text('aos'),
      modelVariable: text('model_variable'),
      standards: standards,
      names: [if (json['names'] is List) for (final each in json['names'] as List) if (each is String) each],
      markers: [if (json['markers'] is List) for (final each in json['markers'] as List) if (each is String) each],
      modes: [for (final each in maps(json['modes'])) if (each['id'] is String) ProviderMode(each['id'] as String, '${each['title'] ?? each['id']}')],
      mark: text('mark') == null ? null : parseMarkSvg(text('mark')!),
      look: text('look'),
      harness: json['harness'] == true,
      imports: json['imports'] == true,
      accounts: accounts is Map
          ? ProviderAccounts(
              provider: '${accounts['provider'] ?? ''}',
              title: '${accounts['title'] ?? id}',
              description: '${accounts['description'] ?? ''}',
              kinds: [for (final each in maps(accounts['kinds'])) ('${each['id']}', '${each['title'] ?? each['id']}')],
              logins: [for (final each in maps(accounts['logins'])) ('${each['id']}', '${each['title'] ?? each['id']}', '${each['produces'] ?? ''}')],
              usage: [if (accounts['usage'] is List) for (final each in accounts['usage'] as List) '$each'],
            )
          : null,
      source: text('source'),
      launch: json['launch'] is Map && (json['launch'] as Map)['program'] is String && (json['launch'] as Map)['usage'] is String
          ? (
              (json['launch'] as Map)['program'] as String,
              [if ((json['launch'] as Map)['args'] is List) for (final each in (json['launch'] as Map)['args'] as List) '$each'],
              (json['launch'] as Map)['usage'] as String,
            )
          : null,
      origin: text('origin') ?? '',
    );
  }
}

/// A reader of an agent's own quota the host offers: the id a host entry's `usage` names, and where it comes from (`bundled`, or the
/// entry or extension whose library or source offers it).
final class UsageReaderOffer {
  const UsageReaderOffer(this.id, this.origin);

  final String id;
  final String origin;

  static UsageReaderOffer? parse(Object? json) {
    if (json is! Map || json['id'] is! String || (json['id'] as String).isEmpty) return null;
    return UsageReaderOffer(json['id'] as String, json['origin'] is String ? json['origin'] as String : '');
  }
}

/// The agents the host has loaded (`providers/list`): where the app learns which agents there are, what each is called, which
/// vendor and credential standards are its own, and what its mark looks like. Until a host answers, none is known, so nothing is
/// listed, named or drawn for an agent.
final class ProviderCatalog extends ChangeNotifier {
  List<AgentProvider> _providers = const [];
  List<UsageReaderOffer> _usageReaders = const [];
  bool _answered = false;
  String? _problem;
  int _revision = 0;

  /// The agents in the order the host listed them.
  List<AgentProvider> get providers => _providers;

  /// The readers of an agent's own quota the host offers, bundled first: the values an agent entry's `usage` may take there.
  List<UsageReaderOffer> get usageReaders => _usageReaders;

  /// The agents that run a harness provider in the host, which are the ones an account can be added to.
  List<AgentProvider> get withAccounts => [for (final each in _providers) if (each.accounts != null) each];

  /// Whether the host has answered, whatever it listed.
  bool get answered => _answered;

  /// Why the host could not be asked, or null.
  String? get problem => _problem;

  /// How many times the answer changed, for a cache keyed on it.
  int get revision => _revision;

  /// Takes the host's answer to `providers/list`. A row that is not an object with an `id` is skipped.
  void adopt(Object? answer) {
    final list = answer is Map ? answer['providers'] : null;
    _providers = [if (list is List) for (final each in list) ?AgentProvider.parse(each)];
    final readers = answer is Map ? answer['usage_readers'] : null;
    _usageReaders = [if (readers is List) for (final each in readers) ?UsageReaderOffer.parse(each)];
    _answered = true;
    _problem = null;
    _changed();
  }

  /// Takes the agents as they were parsed, as [adopt] does from the host's answer.
  void replace(List<AgentProvider> providers, {List<UsageReaderOffer> usageReaders = const []}) {
    _providers = List.of(providers);
    _usageReaders = List.of(usageReaders);
    _answered = true;
    _problem = null;
    _changed();
  }

  /// Notes that the host could not be asked, with [reason].
  void failed(String reason) {
    _problem = reason;
    _changed();
  }

  /// Forgets what the host said, as before it has answered.
  void clear() {
    _providers = const [];
    _usageReaders = const [];
    _answered = false;
    _problem = null;
    _changed();
  }

  void _changed() {
    _revision++;
    notifyListeners();
  }

  /// The agent that goes by [id], or null.
  AgentProvider? byId(String? id) => id == null ? null : _providers.where((each) => each.id == id).firstOrNull;

  /// The agent a name means: an id, a title, a source's name, a standard of its vendor, an AOS id, a name it reports, the vendor's
  /// own name (`anthropic`), or the first word of a compound such as `claude-code`, `claude_store` or `anthropic-api-key`. Null when
  /// no loaded agent is meant.
  AgentProvider? resolve(String? name) {
    if (name == null) return null;
    final whole = name.trim().toLowerCase();
    if (whole.isEmpty) return null;
    AgentProvider? first(bool Function(AgentProvider each) test) => _providers.where(test).firstOrNull;
    bool exact(AgentProvider each, String word) =>
        each.id == word ||
        each.title.toLowerCase() == word ||
        each.source?.toLowerCase() == word ||
        each.aos == word ||
        each.vendor == word ||
        each.standards.containsKey(word) ||
        each.names.any((reported) => reported.toLowerCase() == word);
    final whole0 = first((each) => exact(each, whole));
    if (whole0 != null) return whole0;
    final word = whole.split(RegExp(r'[-_\s.]')).first;
    return word == whole ? null : first((each) => exact(each, word));
  }

  /// The agent that reports itself as [name] in `initialize`, or null.
  AgentProvider? byReportedName(String? name) => name == null ? null : first((each) => each.names.contains(name));

  /// The agent whose description lists one of the `_meta` [markers] an agent set to `true`, or null.
  AgentProvider? byMarkers(List<String> markers) =>
      markers.isEmpty ? null : first((each) => each.markers.any(markers.contains));

  /// The agent whose AOS harness id is [aos], or null.
  AgentProvider? byAos(String? aos) => aos == null ? null : first((each) => each.aos == aos);

  AgentProvider? first(bool Function(AgentProvider each) test) => _providers.where(test).firstOrNull;

  /// What a person reads for the credential standard [id]: the title its vendor gives it, else the id as written.
  String standardTitle(String id) {
    for (final each in _providers) {
      final title = each.standards[id];
      if (title != null) return title;
    }
    return id;
  }

  /// The mark of the agent [name] means, or null for the generic icon.
  AgentMark? markOf(String? name) => resolve(name)?.mark;

  /// The name of the agent [id] means for people: its title, else [id] as written with a capital.
  String titleOf(String id) {
    final known = resolve(id);
    if (known != null) return known.title;
    return id.isEmpty ? id : '${id[0].toUpperCase()}${id.substring(1)}';
  }
}

/// The agents of the host this app reaches, which marks, names and the accounts' screens consult.
final ProviderCatalog providerCatalog = ProviderCatalog();
