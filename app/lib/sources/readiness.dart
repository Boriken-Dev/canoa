import '../store/programs.dart';
import 'sources.dart';

/// What the Store's extensions report of [source]'s program, as the source's readiness; null when no extension serves it, and
/// then the host's own listing of what the source is missing stands. Asks the host for the copies the first time, nothing
/// more: the newest version is asked only by a page that shows it.
Future<SourceCheck?> storeReadiness(ProgramStores programs, AgentSource source) async {
  await programs.ensure();
  final status = programs.forSource(harness: source.harness, label: source.label);
  if (status == null) return null;
  final copy = status.copy;
  final note = switch (copy?.update) {
    UpdatedElsewhere(:final how) => how,
    UpdatedNever(:final why) => why,
    _ => null,
  };
  return SourceCheck.fromStore(
    store: status.info.provider,
    title: status.info.title,
    installed: status.installed,
    path: copy?.path,
    version: status.version,
    latest: status.latest?.known == true ? status.latest?.version : null,
    updateAvailable: status.updateAvailable,
    updateNote: note,
    problem: status.problem,
  );
}
