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
  const SourceCheck({this.found = const {}, this.missing = const [], this.problem});

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
