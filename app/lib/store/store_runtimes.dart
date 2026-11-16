import 'programs.dart';

// The Runtimes type of the Store: the programs other programs need (Node, Python), which the host installs and keeps like an
// agent's. This is its data: what the page lists and what its buttons do, over a [ProgramStores] made with `kind: 'runtimes'`.
// The app holds no runtime name or address: every row is a store the host lists.

/// The catalog name the Runtimes rows come from: the host itself, which is the only place a runtime is installed from.
const String runtimesCatalog = 'This host';

/// The id and plural title of the Runtimes type in the Store's index.
const String runtimesTypeId = 'runtimes';
const String runtimesTypeLabel = 'Runtimes';

/// One runtime as a row of the Runtimes page.
final class RuntimeItem {
  const RuntimeItem._(this._programs, this.status);

  final ProgramStores _programs;
  final ProgramStatus status;

  String get provider => status.info.provider;
  String get name => status.info.title;
  String get by => status.info.publisher;

  /// The version found, with where it comes from: the host's own copy first.
  String? get version => status.version;
  bool get installed => status.installed;

  /// Whether a copy the host installed is there, which is the only kind [remove] takes away.
  bool get managed => status.hasManaged;
  bool get updateAvailable => status.updateAvailable;

  /// What a row says of the runtime besides its name: the version it has, or that it is not installed.
  String get summary =>
      installed ? 'Installed${version == null ? '' : ' ${version!}'}${managed ? '' : ' (found on this computer)'}' : 'Not installed';

  /// What installing (or updating, when a copy of the host's is there) would do: nothing is written.
  Future<ProgramPlan> plan() => _programs.plan(provider, managed ? 'update' : 'install');

  /// Installs or updates once [confirm] has been shown the plan and said yes; a no ends it without asking the host anything more.
  /// Returns whether it ran. Throws what the host refuses with.
  Future<bool> install(Future<bool> Function(ProgramPlan plan) confirm) async {
    final plan = await this.plan();
    if (!await confirm(plan)) return false;
    await _programs.apply(plan).done;
    return true;
  }

  /// Takes the host's copy away.
  Future<void> remove() => _programs.remove(provider);
}

/// The rows of the Runtimes page: every store the host listed for the type, in its order.
List<RuntimeItem> runtimeItems(ProgramStores programs) => [for (final each in programs.statuses) RuntimeItem._(programs, each)];
