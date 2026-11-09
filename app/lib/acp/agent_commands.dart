import '../desktop/child_flag.dart';

/// An agent this computer can run: a program speaking ACP on its standard streams, or an adapter that makes one.
final class AgentCommand {
  const AgentCommand({
    required this.label,
    required this.program,
    this.args = const [],
    this.requires = const [],
    this.programPaths = const {},
    this.searchDirs = const [],
    this.hosts = false,
  });

  final String label;
  final String program;
  final List<String> args;

  /// Whether [args] are a command of the host's own, `canoa agent claude` for one, run by the host this app carries rather
  /// than by a program found on PATH: [program] then only names it.
  final bool hosts;

  /// Other programs it needs, such as the tool an adapter drives. It is offered only when these are found.
  final List<String> requires;

  /// Environment variables naming a program by its full path, resolved when it starts.
  final Map<String, String> programPaths;

  /// Where the tool's own installer puts [program], looked in when PATH does not have it: `~` is the home directory, and a
  /// `%NAME%` an environment variable (the directory is skipped where it is not set).
  final List<String> searchDirs;
}

/// The program and arguments that run [command]. A host's own command runs as [hostProgram], a `canoa` the environment
/// names for tests, when there is one, and otherwise as this app's own [executable] with the flag that makes it the host's
/// command; any other runs as it is.
(String, List<String>) invocation(AgentCommand command, {required String executable, String? hostProgram}) {
  if (!command.hosts) return (command.program, command.args);
  if (hostProgram != null && hostProgram.isNotEmpty) return (hostProgram, command.args);
  return (executable, [canoaChildFlag, ...command.args]);
}

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

/// What runs with no setup, for whoever has the tool signed in already, so its subscription applies and no API key is
/// involved. Claude Code and Codex are served by the host this app carries (`canoa agent`), which drives the installed tool
/// over its own protocol, with no Node; the app gives Claude Code a profile's skills as a plugin, so it allows a client's
/// plugins. Pi's adapter is code fetched when it starts, so its version is pinned.
const List<AgentCommand> agentPresets = [
  AgentCommand(
    label: 'Claude',
    program: 'canoa',
    args: ['agent', 'claude', '--allow-client-plugins', '--allow-bypass'],
    requires: ['claude'],
    programPaths: {'COQUI_ACP_CLAUDE_EXECUTABLE': 'claude'},
    hosts: true,
  ),
  AgentCommand(
    label: 'Codex',
    program: 'canoa',
    args: ['agent', 'codex', '--allow-bypass'],
    requires: ['codex'],
    programPaths: {'COQUI_ACP_CODEX_EXECUTABLE': 'codex'},
    hosts: true,
  ),
  // Antigravity's CLI, which replaced Gemini CLI: `agy acp` is its ACP server. Its Windows installer puts it in
  // %LOCALAPPDATA%gyin; where the Linux and macOS installers do is not confirmed, so there it is found on PATH only.
  AgentCommand(label: 'Antigravity', program: 'agy', args: ['acp'], searchDirs: ['%LOCALAPPDATA%/agy/bin']),
  // xAI's Grok CLI in its ACP mode; its installers put it in ~/.grok/bin.
  AgentCommand(label: 'Grok', program: 'grok', args: ['agent', 'stdio'], searchDirs: ['~/.grok/bin']),
  AgentCommand(label: 'Pi', program: 'npx', args: ['-y', 'pi-acp@0.0.34'], requires: ['pi']),
];
