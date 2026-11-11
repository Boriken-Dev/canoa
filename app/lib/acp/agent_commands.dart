/// An agent this computer can run: a program speaking ACP on its standard streams, which canoa.cfg's `[agents.NAME]` names.
final class AgentCommand {
  const AgentCommand({
    required this.label,
    required this.program,
    this.args = const [],
    this.searchDirs = const [],
    this.usage,
  });

  final String label;
  final String program;
  final List<String> args;

  /// What the host reads of this agent's usage when it serves it, canoa.cfg's `usage` key on a `[host.agents]` entry:
  /// `antigravity` for an agent that serves its quota from its own process and reports nothing over ACP. Null reads only
  /// its failed turns.
  final String? usage;

  /// Where the tool's own installer puts [program], looked in when PATH does not have it: `~` is the home directory, and a
  /// `%NAME%` an environment variable (the directory is skipped where it is not set).
  final List<String> searchDirs;
}

/// The `usage` canoa.cfg's host entry for [command] gets: the command's own, else the one of the program the host knows the
/// quota of, found by program name (without a directory or extension) and arguments, so `agy acp` typed by a person is
/// Antigravity's.
String? usageFor(AgentCommand command) {
  if (command.usage != null) return command.usage;
  String name(String program) =>
      program.split(RegExp(r'[\\/]')).last.toLowerCase().replaceFirst(RegExp(r'\.(exe|cmd|bat)$'), '');
  if (name(command.program) == 'agy' && _sameArgs(const ['acp'], command.args)) return 'antigravity';
  return null;
}

bool _sameArgs(List<String> a, List<String> b) =>
    a.length == b.length && [for (var i = 0; i < a.length; i++) a[i] == b[i]].every((same) => same);

/// A command as a person typed it: words split on spaces, with quotes around any that hold one. Null for nothing, or an open quote.
AgentCommand? parseCommand(String text) {
  final words = <String>[];
  final word = StringBuffer();
  String? quote;
  var inWord = false;
  for (final char in text.trim().split('')) {
    if (quote != null) {
      if (char == quote) {
        quote = null;
      } else {
        word.write(char);
      }
    } else if (char == '"' || char == "'") {
      quote = char;
      inWord = true;
    } else if (char.trim().isEmpty) {
      if (inWord) {
        words.add(word.toString());
        word.clear();
        inWord = false;
      }
    } else {
      word.write(char);
      inWord = true;
    }
  }
  if (quote != null) return null;
  if (inWord) words.add(word.toString());
  if (words.isEmpty || words.first.isEmpty) return null;
  return AgentCommand(label: words.first, program: words.first, args: words.sublist(1));
}
