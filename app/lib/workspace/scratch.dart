import '../profile/apply.dart' show harnessIdOf;
import '../sources/sources.dart';
import 'documents.dart';
import 'workspace_template.dart';

/// The harness a [source] stands for, for `${handler}`: a hosted entry's provider id, else its own name; a source this app
/// starts, its AOS harness id (`claude-code`, `codex`, `grok`) where it has one, else its label.
String handlerOf(AgentSource source) => switch (source.kind) {
  SourceKind.hosted => source.harness ?? source.hostEntry!,
  _ => harnessIdOf(entry: source.label) ?? source.label,
};

/// The folder [template] makes for the session [sessionId] started from [source], or null where this build has no folders
/// or no Documents folder to put it in. The arguments stand in for the system's, for a test.
String? scratchFolder(String template, String sessionId, {
  AgentSource? source,
  DateTime? now,
  String? documents,
  String? home,
  String? host,
  String? separator,
}) {
  if (!hasWorkspaceFolders) return null;
  final root = documents ?? documentsFolder();
  if (root == null && template.contains(r'${documents}')) return null;
  return expandWorkspace(
    template,
    WorkspaceValues(
      documents: root ?? '',
      home: home ?? homeFolder() ?? '',
      session: sessionId,
      now: now ?? DateTime.now(),
      handler: source == null ? '' : handlerOf(source),
      source: source?.label ?? '',
      host: host ?? localHostName() ?? '',
    ),
    separator: separator ?? pathSeparator,
  );
}
