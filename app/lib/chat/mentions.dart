import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';

import '../acp/local_agent.dart' show InsideFile;
import '../session_store.dart';

/// How long a folder's listing is kept, so typing after an `@` does not read the disk again for each letter.
const Duration _keepListing = Duration(seconds: 5);
final Map<String, (DateTime, Future<List<String>>)> _listings = {};

Future<List<String>> _listingOf(String folder, Future<List<String>> Function(String folder) list) {
  final kept = _listings[folder];
  final now = DateTime.now();
  if (kept != null && now.difference(kept.$1) < _keepListing) return kept.$2;
  if (_listings.length > 16) _listings.clear();
  final read = list(folder);
  _listings[folder] = (now, read);
  return read;
}

/// What [query] names among [entries], paths relative to a folder with `/` and a folder ending in `/`: those whose name starts with
/// it, then whose name holds it, then whose path holds it, then whose path holds its letters in order; shorter paths first within
/// each. Nothing typed lists the top level. A query that climbs out of the folder names nothing.
List<String> rankMentions(List<String> entries, String query, {int limit = 8}) {
  final wanted = query.toLowerCase();
  if (wanted.startsWith('/') || wanted.contains(r'\') || wanted.split('/').contains('..') || wanted.contains(RegExp(r'^[a-z]:'))) {
    return const [];
  }
  if (wanted.isEmpty) {
    return [
      for (final entry in entries)
        if (!(entry.endsWith('/') ? entry.substring(0, entry.length - 1) : entry).contains('/')) entry,
    ].take(limit).toList();
  }
  final scored = <(int, String)>[];
  for (final entry in entries) {
    final path = entry.toLowerCase();
    final bare = path.endsWith('/') ? path.substring(0, path.length - 1) : path;
    final name = bare.substring(bare.lastIndexOf('/') + 1);
    final score = name.startsWith(wanted)
        ? 0
        : name.contains(wanted)
        ? 1
        : path.contains(wanted)
        ? 2
        : _inOrder(path, wanted)
        ? 3
        : -1;
    if (score >= 0) scored.add((score, entry));
  }
  scored.sort((a, b) => a.$1 != b.$1 ? a.$1.compareTo(b.$1) : a.$2.length.compareTo(b.$2.length));
  return [for (final (_, entry) in scored.take(limit)) entry];
}

bool _inOrder(String path, String letters) {
  var at = 0;
  for (final unit in letters.runes) {
    at = path.indexOf(String.fromCharCode(unit), at);
    if (at < 0) return false;
    at++;
  }
  return true;
}

/// What an `@` names in [session]: files and folders under its folder on this computer. Nothing outside the folder is offered or
/// taken, and a pick is a link to the path, never the file's text. [list] and [resolve] are the platform's file services.
final class SessionMentions implements ComposerMentions {
  SessionMentions({required this.store, required this.session, required this.list, required this.resolve});

  final SessionStore store;
  final ChatSession session;
  final Future<List<String>> Function(String folder) list;
  final InsideFile? Function(String folder, String relative) resolve;

  /// Whether [session] has a folder on this computer an `@` could name files of.
  static bool offeredFor(ChatSession session) => session.place == SessionPlace.local && (session.cwd?.isNotEmpty ?? false);

  @override
  Future<List<MentionCandidate>> search(String query) async {
    final folder = session.cwd;
    if (folder == null || folder.isEmpty) return const [];
    final entries = await _listingOf(folder, list);
    return [
      for (final entry in rankMentions(entries, query))
        MentionCandidate(
          label: entry,
          folder: entry.endsWith('/'),
          key: entry,
        ),
    ];
  }

  @override
  String? pick(MentionCandidate candidate) {
    final folder = session.cwd;
    final relative = candidate.key as String? ?? candidate.label;
    if (folder == null || folder.isEmpty) return 'This session has no folder.';
    final found = resolve(folder, relative);
    if (found == null) return '$relative is not in this session\'s folder, so it is not attached.';
    return store.addMention(session, relative, found.path, found.size, folder: found.folder);
  }
}
