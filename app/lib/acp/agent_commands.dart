/// An agent this computer can run: a program speaking ACP on its standard streams, or an adapter that makes one.
final class AgentCommand {
  const AgentCommand({
    required this.label,
    required this.program,
    this.args = const [],
    this.requires = const [],
    this.programPaths = const {},
  });

  final String label;
  final String program;
  final List<String> args;

  /// Other programs it needs, such as the tool an adapter drives. It is offered only when these are found.
  final List<String> requires;

  /// Environment variables naming a program by its full path, resolved when it starts.
  final Map<String, String> programPaths;
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

/// What runs with no setup, for whoever has the tool signed in already. An adapter drives that tool, so its subscription applies
/// and no API key is involved. The versions are pinned because an adapter is code fetched when it starts.
const List<AgentCommand> agentPresets = [
  AgentCommand(
    label: 'Claude',
    program: 'npx',
    args: ['-y', '@agentclientprotocol/claude-agent-acp@0.99.1'],
    requires: ['claude'],
    programPaths: {'CLAUDE_CODE_EXECUTABLE': 'claude'},
  ),
  AgentCommand(
    label: 'Codex',
    program: 'npx',
    args: ['-y', '@agentclientprotocol/codex-acp@2.5.0'],
    requires: ['codex'],
  ),
  AgentCommand(label: 'Gemini', program: 'gemini', args: ['--acp']),
];
