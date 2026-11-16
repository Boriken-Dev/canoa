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
  final Future<InsideFile?> Function(String folder, String relative) resolve;

  /// Whether [session] has a folder an `@` could name files of: one on a computer this app reads folders of ([readable], which
  /// is this computer's for a session started here when null).
  static bool offeredFor(ChatSession session, {bool Function(ChatSession session)? readable}) =>
      (readable?.call(session) ?? session.place == SessionPlace.local) && (session.cwd?.isNotEmpty ?? false);

  /// Each candidate is resolved as it is offered, so what a pick attaches is what was checked, and a name that is not inside the
  /// folder is never offered.
  @override
  Future<List<MentionCandidate>> search(String query) async {
    final folder = session.cwd;
    if (folder == null || folder.isEmpty) return const [];
    final entries = await _listingOf(folder, list);
    final ranked = rankMentions(entries, query);
    final found = await Future.wait([for (final entry in ranked) resolve(folder, entry)]);
    return [
      for (var i = 0; i < ranked.length; i++)
        if (found[i] case final inside?)
          MentionCandidate(label: ranked[i], folder: ranked[i].endsWith('/'), key: (ranked[i], inside)),
    ];
  }

  @override
  String? pick(MentionCandidate candidate) {
    final folder = session.cwd;
    if (folder == null || folder.isEmpty) return 'This session has no workspace.';
    final key = candidate.key;
    final (String relative, InsideFile found) = key is (String, InsideFile) ? key : (candidate.label, (path: '', folder: false, size: -1));
    if (found.size < 0) return '$relative is not in this session\'s workspace, so it is not attached.';
    return store.addMention(session, relative, found.path, found.size, folder: found.folder);
  }
}
