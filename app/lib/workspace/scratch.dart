import '../providers/provider_catalog.dart';
import '../sources/sources.dart';

/// The harness a [source] stands for, for `${handler}`: its provider id (a loaded agent's, or a hosted entry's own `harness`), else
/// its label.
String handlerOf(AgentSource source) {
  final id = source.harness ?? source.label.toLowerCase();
  return source.harness != null || providerCatalog.byId(id) != null ? id : source.label;
}

/// The separator of a path written for another machine, from the folder it starts with: a drive letter or a share is Windows.
String separatorOf(String folder) =>
    RegExp(r'^(?:[A-Za-z]:|\\\\)').hasMatch(folder) || (folder.contains(r'\') && !folder.contains('/')) ? r'\' : '/';
