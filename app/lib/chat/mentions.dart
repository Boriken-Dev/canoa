import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart' show Icons;

import '../acp/local_agent.dart' show InsideFile;
import '../session_store.dart';

/// How long a folder's listing is kept, so typing after an `@` does not read the disk again for each letter.
const Duration _keepListing = Duration(seconds: 5);
final Map<String, (DateTime, Future<List<String>>)> _listings = {};

Future<List<String>> _listingOf(String scope, String folder, Future<List<String>> Function(String folder) list) {
  final key = '$scope|$folder';
  final kept = _listings[key];
  final now = DateTime.now();
  if (kept != null && now.difference(kept.$1) < _keepListing) return kept.$2;
  if (_listings.length > 16) _listings.clear();
  final read = list(folder);
  _listings[key] = (now, read);
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

/// The two things an `@` asks of the computer a session's folder is on: the names under a folder, and what one name is.
typedef FolderServices = ({Future<List<String>> Function(String folder) list, Future<InsideFile?> Function(String folder, String relative) resolve});

/// What an `@` names in [session]: files and folders under its folder on this computer. Nothing outside the folder is offered or
/// taken, and a pick goes to the agent as a link to the path, or as the file's text where the agent takes files and it is small. [list] and [resolve] are the platform's file services.
final class SessionMentions implements ComposerMentions {
  SessionMentions({required this.store, required this.session, required this.list, required this.resolve, this.scope = ''});

  /// Whose folders [list] reads, so that two hosts' folders of one path are not taken for each other in the kept listings.
  final String scope;

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
    final entries = await _listingOf(scope, folder, list);
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
    return store.addMention(session, relative, found.path, found.size, folder: found.folder, onAgentHost: scope.isNotEmpty);
  }
}

/// What an `@` names in [session]: the files and folders [files] offers (when the session has a folder), the session's MCP
/// resources, its subagent tasks, and the other conversations this app holds.
final class SessionAtMentions implements ComposerMentions {
  SessionAtMentions({required this.store, required this.session, this.files});

  final SessionStore store;
  final ChatSession session;
  final ComposerMentions? files;

  /// How many of each group the list holds, so that neither crowds out the others.
  static const int filesShown = 6, resourcesShown = 4, subagentsShown = 4, conversationsShown = 3;

  /// The conversations other than [session] that have something said, the latest first, whose title holds [query] (any, with none).
  List<ChatSession> conversationsFor(String query) {
    final wanted = query.toLowerCase();
    final found = [
      for (final other in store.listed)
        if (other.id != session.id && other.messages.isNotEmpty && (wanted.isEmpty || other.title.toLowerCase().contains(wanted))) other,
    ]..sort((a, b) => b.lastActivity.compareTo(a.lastActivity));
    return found.take(conversationsShown).toList();
  }

  /// MCP server resources offered to [session] whose name, title, server or uri matches [query].
  List<McpResource> resourcesFor(String query) {
    final all = session.mcpInventory?.resources ?? const <McpResource>[];
    if (all.isEmpty) return const [];
    final wanted = query.toLowerCase();
    if (wanted.isEmpty) return all.take(resourcesShown).toList();
    final starts = <McpResource>[], holds = <McpResource>[];
    for (final res in all) {
      final name = res.name.toLowerCase();
      final title = (res.title ?? '').toLowerCase();
      final server = res.server.toLowerCase();
      final uri = res.uri.toLowerCase();
      if (name.startsWith(wanted) || title.startsWith(wanted)) {
        starts.add(res);
      } else if (name.contains(wanted) || title.contains(wanted) || server.contains(wanted) || uri.contains(wanted)) {
        holds.add(res);
      }
    }
    return [...starts, ...holds].take(resourcesShown).toList();
  }

  /// Subagents (A2A tasks) of [session] matching [query], active tasks first.
  List<DelegateTask> subagentsFor(String query) {
    final all = session.tasks.all;
    if (all.isEmpty) return const [];
    final wanted = query.toLowerCase();
    final matching = [
      for (final task in all)
        if (wanted.isEmpty ||
            task.title.toLowerCase().contains(wanted) ||
            task.source.toLowerCase().contains(wanted) ||
            task.id.toLowerCase().contains(wanted))
          task,
    ]..sort((a, b) {
      final aLive = a.phase.ended ? 1 : 0;
      final bLive = b.phase.ended ? 1 : 0;
      return aLive != bLive ? aLive.compareTo(bLive) : b.order.compareTo(a.order);
    });
    return matching.take(subagentsShown).toList();
  }

  static String _phaseLabel(TaskPhase phase) => switch (phase) {
    TaskPhase.submitted => 'Queued',
    TaskPhase.working => 'Working',
    TaskPhase.inputRequired || TaskPhase.authRequired => 'Needs you',
    TaskPhase.completed => 'Done',
    TaskPhase.failed || TaskPhase.rejected => 'Failed',
    TaskPhase.canceled => 'Stopped',
  };

  @override
  Future<List<MentionCandidate>> search(String query) async {
    final named = files == null ? const <MentionCandidate>[] : (await files!.search(query)).take(filesShown).toList();
    final resources = resourcesFor(query);
    final subagents = subagentsFor(query);
    final conversations = conversationsFor(query);
    return [
      ...named,
      for (final res in resources)
        MentionCandidate(
          label: res.name,
          detail: '${res.server}${res.mimeType == null ? '' : ' · ${res.mimeType}'}',
          group: 'Resources',
          icon: Icons.description_outlined,
          key: res,
        ),
      for (final task in subagents)
        MentionCandidate(
          label: task.title,
          detail: '${task.source} · ${_phaseLabel(task.phase)}',
          group: 'Subagents',
          icon: Icons.account_tree_outlined,
          key: task,
        ),
      for (final other in conversations)
        MentionCandidate(
          label: other.title,
          detail: other.agent,
          group: 'Conversations',
          icon: Icons.forum_outlined,
          key: other.id,
        ),
    ];
  }

  @override
  String? pick(MentionCandidate candidate) {
    final key = candidate.key;
    if (candidate.group == 'Resources' && key is McpResource) {
      return store.addResource(session, candidate.label, key.uri, mimeType: key.mimeType);
    }
    if (candidate.group == 'Subagents' && key is DelegateTask) {
      // A subagent pick names the task in the message; no separate attachment is added.
      return null;
    }
    if (candidate.group == 'Conversations' && key is String) {
      final other = store.sessions.where((each) => each.id == key).firstOrNull;
      return other == null ? 'That conversation is gone.' : store.addConversation(session, other, candidate.label);
    }
    final source = files;
    return source == null ? 'This session has no workspace.' : source.pick(candidate);
  }
}
