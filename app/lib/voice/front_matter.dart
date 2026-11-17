import 'dart:convert';

/// The name of the first dictation's notes among a message's attachments; a second one in the same message is `dictation-2.md`.
String dictationNotesName(int number) => number <= 1 ? 'dictation.md' : 'dictation-$number.md';

/// What a speech service said about a dictation besides its words, as the front matter of a small Markdown file the agent reads
/// as one more block of the prompt. It is a block and not `_meta` because an agent passes a prompt's blocks to its model, and
/// ignores a prompt's `_meta`. Values are written as JSON, which YAML reads, so no text can break out of its line.
String dictationFrontMatter(Map<String, Object?> notes) {
  final lines = <String>['dictated: true'];
  for (final entry in notes.entries) {
    if (entry.key == 'dictated') continue;
    final key = RegExp(r'^[A-Za-z_][A-Za-z0-9_-]*$').hasMatch(entry.key) ? entry.key : jsonEncode(entry.key);
    lines.add('$key: ${jsonEncode(entry.value)}');
  }
  return '---\n${lines.join('\n')}\n---\n\nThe message was dictated; the notes above are what the speech service said about it.\n';
}
