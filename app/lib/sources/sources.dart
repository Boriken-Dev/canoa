import '../acp/agent_commands.dart';

/// Where a source's agent comes from: a preset this app starts, a command from Canoa.cfg's `[agents.NAME]`, or an agent
/// entry of the `canoa-host` this app runs.
enum SourceKind { preset, command, hosted }

/// One way to run an agent, as Settings lists it and the new-session picker offers it.
final class AgentSource {
  const AgentSource.preset(AgentCommand this.command) : kind = SourceKind.preset, hostEntry = null;
  const AgentSource.command(AgentCommand this.command) : kind = SourceKind.command, hostEntry = null;
  const AgentSource.hosted(String this.hostEntry) : kind = SourceKind.hosted, command = null;

  final SourceKind kind;

  /// The program and its arguments, and what else it needs, for a source this app starts.
  final AgentCommand? command;

  /// The name of the `[host]` agent entry, for a source `canoa-host` serves.
  final String? hostEntry;

  String get label => command?.label ?? hostEntry!;

  /// Whether this app starts the program itself, on this computer.
  bool get runsHere => command != null;

  @override
  bool operator ==(Object other) => other is AgentSource && other.kind == kind && other.label == label;

  @override
  int get hashCode => Object.hash(kind, label);
}

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

/// Finds a program the way a command is found, answering its absolute path or null.
typedef ProgramLookup = Future<String?> Function(String program);

/// Whether [command]'s program and everything it requires can be found.
Future<SourceCheck> checkCommand(AgentCommand command, ProgramLookup find) async {
  final found = <String, String>{};
  final missing = <String>[];
  for (final program in {command.program, ...command.requires}) {
    final path = await find(program);
    if (path == null) {
      missing.add(program);
    } else {
      found[program] = path;
    }
  }
  return SourceCheck(found: found, missing: missing);
}

/// The programs a command runs and requires, in order, once each.
List<String> commandNeeds(AgentCommand command) => {command.program, ...command.requires}.toList();

/// What the session list keeps a source's memory under: its label, and for a hosted entry the host's name before it, since a
/// preset and a hosted entry can share a label.
String sourceKey(AgentSource source) => source.kind == SourceKind.hosted ? 'canoa-host/${source.label}' : source.label;
