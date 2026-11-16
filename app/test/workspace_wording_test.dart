// "Workspace" is the word for the directory a session works in. The files that word appears in are read for their string
// literals: a literal that says "folder" must mean something else (an attachment, a path inside a workspace, the agent's own
// storage), and each such literal is listed here with the reason.
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

const _files = [
  'lib/chat/new_session_view.dart',
  'lib/chat/composer.dart',
  'lib/chat/composer_controls.dart',
  'lib/chat/inspector.dart',
  'lib/chat/mentions.dart',
  'lib/chat/session_views.dart',
  'lib/chat/remember_dialog.dart',
  'lib/permissions/permission_rules.dart',
  'lib/served/folder_dialog.dart',
  'lib/settings/agent_sections.dart',
  'lib/settings/app_sections.dart',
  'lib/desktop/hosting_panel.dart',
  'lib/main.dart',
  'lib/session_store.dart',
  'lib/store/programs_view.dart',
  'lib/workspace/workspace_template.dart',
  'lib/rail/session_rail.dart',
  'lib/rail/session_groups.dart',
  'lib/ledger/ledger_list.dart',
];

/// Literals that say "folder" and mean a folder: file name part, then the text.
const _folders = <(String, String)>[
  ('composer.dart', "'Folder'"), // an attachment of the folder kind
  ('remember_dialog.dart', 'This file, or this folder and what is under it'), // a path inside the workspace
  ('remember_dialog.dart', 'Never a hidden file or folder'),
  ('remember_dialog.dart', 'the folders around it'),
  ('folder_dialog.dart', 'Up one folder'), // the listing's own entries
  ('folder_dialog.dart', 'No folders here.'),
  ('folder_dialog.dart', 'More folders are here than the host lists.'),
  ('agent_sections.dart', 'is your home folder'),
  ('permission_rules.dart', 'The workspace holds your home folder.'),
  ('permission_rules.dart', 'Canoa\\\'s own configuration folder'),
  ('session_store.dart', "own folder"), // where the agent keeps its sessions
  ('programs_view.dart', "outside Canoa\\'s folder"), // Canoa's own folder
  ('new_session_view.dart', 'recent-folder'), // keys
  ('new_session_view.dart', 'new-session-folder'),
];

List<String> _literals(String line) => [
  for (final match in RegExp(r'''('(?:[^'\\]|\\.)*')|("(?:[^"\\]|\\.)*")''').allMatches(line)) match.group(0)!,
];

void main() {
  test('no string literal of the files that name a session\'s directory calls it a folder', () {
    final found = <String>[];
    for (final path in _files) {
      final lines = File(path).readAsLinesSync();
      for (var i = 0; i < lines.length; i++) {
        final line = lines[i];
        final trimmed = line.trimLeft();
        if (trimmed.startsWith('//') || trimmed.startsWith('import ') || trimmed.startsWith('export ')) continue;
        if (line.contains('Key(')) continue;
        for (final literal in _literals(line)) {
          // An interpolated variable is code, not wording.
          final words = literal.replaceAll(RegExp(r'\$\{[^}]*\}|\$\w+'), '');
          if (!RegExp(r'folder', caseSensitive: false).hasMatch(words)) continue;
          // A JSON field or a map key, not text a person reads.
          if (RegExp(r"^'[a-z_]*folder[a-z_]*'$").hasMatch(literal)) continue;
          final name = path.split('/').last;
          if (_folders.any((each) => each.$1 == name && literal.contains(each.$2))) continue;
          found.add('$path:${i + 1}: $literal');
        }
      }
    }
    expect(found, isEmpty, reason: found.join('\n'));
  });
}
