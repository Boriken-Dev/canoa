import '../sources/sources.dart';

/// The `acp_harness` provider ids Canoa knows, which a source of the same name stands for.
const _providerIds = {'claude', 'codex', 'grok', 'antigravity', 'pi'};

/// The harness a [source] stands for, for `${handler}`: its provider id (`claude`, `codex`, `grok`, `antigravity`, `pi`, or a
/// hosted entry's own `harness`), else its label.
String handlerOf(AgentSource source) {
  final id = source.harness ?? source.label.toLowerCase();
  return source.harness != null || _providerIds.contains(id) ? id : source.label;
}

/// The separator of a path written for another machine, from the folder it starts with: a drive letter or a share is Windows.
String separatorOf(String folder) =>
    RegExp(r'^(?:[A-Za-z]:|\\\\)').hasMatch(folder) || (folder.contains(r'\') && !folder.contains('/')) ? r'\' : '/';
