import '../desktop/host_tool.dart';

/// One way to run an agent, as Settings lists it and the new-session picker offers it: an agent entry of the `canoa` this app
/// runs, which is one of the host's own sources (a built-in one, or a command of canoa.cfg's `[agents.NAME]`), or an entry
/// canoa.cfg's `[host]` names.
final class AgentSource {
  const AgentSource.hosted(String this.hostEntry, {this.harness, this.title, this.local = false});

  /// The name of the agent entry the host serves.
  final String? hostEntry;

  String get label => hostEntry!;

  /// The provider id of a hosted `harness` entry; null for any other entry.
  final String? harness;

  /// A hosted entry's `title` in canoa.cfg.
  final String? title;

  /// Whether it is one of the host's own sources (Claude, Codex, ...), reached through the in-process link and not an address.
  final bool local;

  /// What a person is shown: a hosted entry's title, else its name. [label] stays the identity, which sessions remember.
  String get displayName => title ?? label;

  @override
  bool operator ==(Object other) => other is AgentSource && other.label == label;

  @override
  int get hashCode => label.hashCode;
}

/// The sources the host lists: each agent entry it is serving, in its order. An entry added to canoa.cfg, a harness entry
/// naming any provider included, is a source here as soon as the host serves it, with nothing for the app to know of it.
List<AgentSource> hostedSources(Iterable<HostListening> serving) => [
  for (final entry in serving)
    if (entry.kind == 'agent') AgentSource.hosted(entry.name, harness: entry.harness, title: entry.title, local: entry.local),
];

/// What a source needs and whether it is here: each program it runs or requires, found or not.
final class SourceCheck {
  const SourceCheck({
    this.found = const {},
    this.missing = const [],
    this.problem,
    this.store,
    this.title,
    this.version,
    this.latest,
    this.updateAvailable = false,
    this.updateNote,
  });

  /// A source whose program the Store's extension reports: [store] is that extension's id, [title] its program's name for
  /// people. A program the extension did not find is [missing] by that name; one it found is [found] at its path, with the
  /// [version] it read and the [latest] the publisher says, when the person's page has asked.
  factory SourceCheck.fromStore({
    required String store,
    required String title,
    required bool installed,
    String? path,
    String? version,
    String? latest,
    bool updateAvailable = false,
    String? updateNote,
    String? problem,
  }) => SourceCheck(
    found: {if (installed) title: ?path},
    missing: [if (!installed) title],
    problem: problem,
    store: store,
    title: title,
    version: version,
    latest: latest,
    updateAvailable: updateAvailable,
    updateNote: updateNote,
  );

  /// The Store extension that reports this source's program; null where none does.
  final String? store;

  /// That program's name, as its extension gives it.
  final String? title;

  /// The version the extension found.
  final String? version;

  /// The newest version the publisher says, when it was asked.
  final String? latest;

  /// Whether [latest] is newer than [version].
  final bool updateAvailable;

  /// Who updates the copy found, when the Store cannot (the program's own command, or why not).
  final String? updateNote;

  /// Each program found, with the path it was found at.
  final Map<String, String> found;

  /// The programs not found.
  final List<String> missing;

  /// Why a source that runs no program here cannot be used now, such as a hosted entry the host is not serving.
  final String? problem;

  bool get ready => missing.isEmpty && problem == null;

  /// What is missing and how to put it right, in a person's words; null when nothing is.
  String? describe(String label, {List<String> needs = const []}) {
    if (problem != null) return problem;
    if (missing.isEmpty) return null;
    if (store != null) return '${title ?? label} is not installed. Install it in Settings, under Sources, or in the Store.';
    final found = [for (final MapEntry(:key, :value) in this.found.entries) 'found $key at $value'];
    return '$label needs ${_and(needs.isEmpty ? [...this.found.keys, ...missing] : needs)} on PATH: '
        '${[...found, '${_and(missing)} ${missing.length == 1 ? 'was' : 'were'} not found'].join('; ')}. '
        'Install what is missing, or start Canoa where PATH finds it.';
  }

  static String _and(List<String> words) =>
      words.length < 2 ? words.join() : '${words.sublist(0, words.length - 1).join(', ')} and ${words.last}';
}

/// What the session list keeps a source's memory under: its label, which the host keeps unique among its entries.
String sourceKey(AgentSource source) => source.label;
