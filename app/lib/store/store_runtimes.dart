import '../config/canoa_config.dart';
import '../profile/profile.dart';
import 'programs.dart';

// The Runtimes type of the Store: the programs other programs need (Node, Python), which the host installs and keeps like an
// agent's. This is its data: what the page lists and what its buttons do, over a [ProgramStores] made with `kind: 'runtimes'`.
// The app holds no runtime name or address: every row is a store the host lists.

/// The catalog name the Runtimes rows come from: the host itself, which is the only place a runtime is installed from.
const String runtimesCatalog = 'This host';

/// The id and plural title of the Runtimes type in the Store's index.
const String runtimesTypeId = 'runtimes';
const String runtimesTypeLabel = 'Runtimes';

/// Something in canoa.cfg that starts a runtime's command from the `PATH`: an agent the host serves or the person added, or an MCP
/// server's command.
final class RuntimeUser {
  const RuntimeUser(this.kind, this.name, this.command);

  /// `Agent` or `MCP server`.
  final String kind;
  final String name;

  /// The command it starts, as written.
  final String command;
}

/// [command] as a name the `PATH` finds: with no directory in front of it (a path names one copy, not a runtime that is found), and
/// with no `.exe`, `.cmd` or `.bat` after it, in lower case; null for a command that names a path.
String? bareCommand(String command) {
  final text = command.trim();
  if (text.isEmpty || text.contains('/') || text.contains('\\')) return null;
  return text.toLowerCase().replaceFirst(RegExp(r'\.(exe|cmd|bat)$'), '');
}

/// One runtime as a row of the Runtimes page.
final class RuntimeItem {
  const RuntimeItem._(this._programs, this.status);

  /// The runtime the host lists as [provider], or null when it lists none now.
  static RuntimeItem? find(ProgramStores programs, String provider) {
    final status = programs.status(provider);
    return status == null ? null : RuntimeItem._(programs, status);
  }

  final ProgramStores _programs;
  final ProgramStatus status;

  String get provider => status.info.provider;
  String get name => status.info.title;
  String get by => status.info.publisher;

  /// What it is, as its store says, and the name of the glyph its store picked.
  String? get about => status.info.summary;
  String? get icon => status.info.icon;
  String? get homepage => status.info.homepage;

  /// The places it downloads from.
  List<String> get endpoints => status.info.endpoints;

  /// The newest version its publisher says, when it was asked and said.
  String? get newest => status.latest?.known == true ? status.latest!.version : null;

  /// What the agents, MCP servers and tools of canoa.cfg that start one of this runtime's commands from the `PATH` are: the host puts
  /// an installed runtime's folder on the `PATH` of everything it starts.
  List<RuntimeUser> users(CanoaConfig config) {
    final provided = {for (final each in status.info.commands) each.toLowerCase()};
    if (provided.isEmpty) return const [];
    bool names(String command) => provided.contains(bareCommand(command));
    return [
      for (final entry in config.hostEntries)
        if (entry.kind == 'agent' && names(entry.runs)) RuntimeUser('Agent', entry.name, entry.runs),
      for (final agent in config.agents)
        if (names(agent.program)) RuntimeUser('Agent', agent.label, agent.program),
      for (final MapEntry(:key, :value) in config.profile.mcp.entries)
        if (value case McpStdio(:final command) when names(command)) RuntimeUser('MCP server', key, command),
    ];
  }

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
