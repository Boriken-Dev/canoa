import 'package:coqui_acp_chat/coqui_acp_chat.dart' show ToolStatus;

import '../session_store.dart';

/// A file the session's tools wrote or changed, as the agent reported it.
final class SessionOutput {
  const SessionOutput(this.path, {required this.deleted});

  /// The file's path, absolute where the workspace is known: one file is one output however the agent wrote its path.
  final String path;

  /// Whether the last thing a tool did to it was delete it.
  final bool deleted;

  /// [path] as the workspace names it, or the whole path when it lies elsewhere.
  String shownIn(String? root) {
    final top = root == null ? null : _slashed(root).replaceAll(RegExp(r'/+$'), '');
    final own = _slashed(path);
    if (top != null && top.isNotEmpty && own.length > top.length && _sameStart(own, top) && own[top.length] == '/') return own.substring(top.length + 1);
    return path;
  }

  /// The file's name, for a row's label.
  String get name => path.split(RegExp(r'[\\/]')).where((part) => part.isNotEmpty).lastOrNull ?? path;
}

String _slashed(String path) => path.replaceAll('\\', '/');

// A Windows drive or folder is the same place in any case.
bool _sameStart(String path, String top) => path.substring(0, top.length).toLowerCase() == top.toLowerCase();

bool _absolute(String path) => path.startsWith('/') || path.startsWith('\\') || RegExp(r'^[A-Za-z]:[\\/]').hasMatch(path);

/// [path] joined to [root] when it is relative, so one file written two ways is one output.
String resolvedIn(String? root, String path) {
  if (root == null || root.isEmpty || _absolute(path)) return path;
  final windows = root.contains('\\') && !root.contains('/');
  final base = root.replaceAll(RegExp(r'[\\/]+$'), '');
  final tail = path.replaceAll(RegExp(r'^\.[\\/]+'), '');
  return windows ? '$base\\${tail.replaceAll('/', '\\')}' : '$base/$tail';
}

final _line = RegExp(r':\d+$');

/// The files [session]'s completed tool calls wrote, changed or deleted, newest first: each once, placed by the last call that
/// touched it. A relative path is taken from [root]. A file written again after it was deleted is not deleted.
List<SessionOutput> outputsOf(ChatSession session, String? root) {
  final touched = <String, bool>{};
  for (final entry in session.messages) {
    final tool = entry.tool;
    if (tool == null || tool.status != ToolStatus.done) continue;
    final deleting = tool.kind == 'delete';
    final paths = tool.changedPaths;
    final places = paths.isNotEmpty ? paths : (deleting ? [for (final place in tool.locations) place.replaceFirst(_line, '')] : const <String>[]);
    for (final path in places) {
      final key = resolvedIn(root, path);
      // Moving the key to the end is what makes the order the last touch's.
      touched.remove(key);
      touched[key] = deleting;
    }
  }
  return [for (final key in touched.keys.toList().reversed) SessionOutput(key, deleted: touched[key]!)];
}
