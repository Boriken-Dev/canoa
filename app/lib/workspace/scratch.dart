import '../sources/sources.dart';
import 'documents.dart';
import 'site.dart';
import 'workspace_template.dart';

/// The `acp_harness` provider ids Canoa knows, which a source of the same name stands for.
const _providerIds = {'claude', 'codex', 'grok', 'antigravity', 'pi'};

/// The harness a [source] stands for, for `${handler}`: its provider id (`claude`, `codex`, `grok`, `antigravity`, `pi`, or a
/// hosted entry's own `harness`), else its label.
String handlerOf(AgentSource source) {
  final id = source.harness ?? source.label.toLowerCase();
  return source.harness != null || _providerIds.contains(id) ? id : source.label;
}

/// The folder [template] makes for the session [sessionId] started from [source], or null where this build has no folders
/// or no Documents folder to put it in. The arguments stand in for the system's, for a test.
String? scratchFolder(String template, String sessionId, {
  AgentSource? source,
  DateTime? now,
  String? documents,
  String? home,
  String? host,
  String? separator,
  String? fqdn,
}) {
  // A site name is taken only as [siteName] writes it: one folder name, so it cannot reach outside the scratch root.
  if (fqdn != null && siteName(fqdn) != fqdn) {
    throw ArgumentError.value(fqdn, 'fqdn', 'is not a host name that is safe as a folder name');
  }
  // A page has no folders of its own; the folder it is given is on the machine its agent runs on.
  if (!hasWorkspaceFolders && documents == null) return null;
  final root = documents ?? documentsFolder();
  if (root == null && template.contains(r'${documents}')) return null;
  return expandWorkspace(
    template,
    WorkspaceValues(
      documents: root ?? '',
      home: home ?? homeFolder() ?? '',
      session: sessionId,
      now: (now ?? DateTime.now()).toLocal(),
      handler: source == null ? '' : handlerOf(source),
      source: source?.label ?? '',
      host: host ?? localHostName() ?? '',
      fqdn: fqdn ?? '',
    ),
    separator: separator ?? (hasWorkspaceFolders ? pathSeparator : separatorOf(root ?? '')),
  );
}

/// The separator of a path written for another machine, from the folder it starts with: a drive letter or a share is Windows.
String separatorOf(String folder) =>
    RegExp(r'^(?:[A-Za-z]:|\\\\)').hasMatch(folder) || (folder.contains(r'\') && !folder.contains('/')) ? r'\' : '/';
