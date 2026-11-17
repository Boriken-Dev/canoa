import 'dart:async';

import 'package:flutter/material.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../served/host_files.dart';
import 'settings_row.dart';

/// One shell the host found on its computer, whether Canoa installed it or not.
final class HostShell {
  const HostShell({
    required this.id,
    required this.label,
    required this.path,
    required this.family,
    required this.origin,
  });

  /// What `[host] shell`, an agent entry's `shell` and a new terminal name it by.
  final String id;
  final String label, path;

  /// `bash`, `zsh`, `powershell` or `other`.
  final String family;

  /// `canoa`, `git-for-windows`, `msys2`, `wsl`, `system` or `custom`.
  final String origin;

  static HostShell? parse(Object? json) {
    if (json is! Map) return null;
    final id = json['id'], path = json['path'];
    if (id is! String || path is! String) return null;
    return HostShell(
      id: id,
      label: json['label'] is String ? json['label']! as String : id,
      path: path,
      family: json['family'] is String ? json['family']! as String : 'other',
      origin: json['origin'] is String ? json['origin']! as String : 'system',
    );
  }

  /// A person's words for it: the label and where it is.
  String get text => '$label ($path)';
}

/// One agent entry of the host and how it is told its shell, as `shell/list` words it.
final class AgentShell {
  const AgentShell({
    required this.entry,
    required this.harness,
    this.setting,
    this.own = false,
    this.id,
    this.env = const [],
    this.notice,
    this.problem,
  });

  final String entry, harness;

  /// What names its shell: its own `shell`, else the host's; null when neither is set.
  final String? setting;

  /// Whether the entry names it itself.
  final bool own;

  /// The id of the shell the setting found, and the variables its harness is told it with (names only).
  final String? id;
  final List<String> env;

  /// Why the harness cannot be told, and why the setting names no shell, in the host's words.
  final String? notice, problem;

  static AgentShell? parse(Object? json) {
    if (json is! Map || json['entry'] is! String) return null;
    return AgentShell(
      entry: json['entry']! as String,
      harness: json['harness'] is String ? json['harness']! as String : '',
      setting: json['shell'] is String ? json['shell'] as String : null,
      own: json['own'] == true,
      id: json['id'] is String ? json['id'] as String : null,
      env: [
        if (json['env'] is List)
          for (final each in json['env'] as List)
            if (each is String) each,
      ],
      notice: json['notice'] is String ? json['notice'] as String : null,
      problem: json['problem'] is String ? json['problem'] as String : null,
    );
  }
}

/// What `shell/list` answers: the shells found, the host's choice and what each agent entry is told.
final class ShellReport {
  const ShellReport({
    this.shells = const [],
    this.setting,
    this.chosen,
    this.problem,
    this.agents = const [],
  });

  final List<HostShell> shells;

  /// `[host] shell` as written, and the shell it names when it names one this host has.
  final String? setting;
  final HostShell? chosen;
  final String? problem;
  final List<AgentShell> agents;

  static ShellReport parse(Map<Object?, Object?> json) => ShellReport(
    shells: [
      if (json['shells'] is List)
        for (final each in json['shells'] as List) ?HostShell.parse(each),
    ],
    setting: json['setting'] is String ? json['setting'] as String : null,
    chosen: HostShell.parse(json['default']),
    problem: json['problem'] is String ? json['problem'] as String : null,
    agents: [
      if (json['agents'] is List)
        for (final each in json['agents'] as List) ?AgentShell.parse(each),
    ],
  );
}

/// The shell the host runs a terminal with and tells each agent to use, chosen once for the host and again for any agent entry,
/// through the host's own `[host]` table: the host checks what canoa.cfg would then hold before it writes it, and reads the table only
/// when it starts, so [restartPending] is true once something changed. Choosing writes only canoa.cfg: no program's own settings are
/// touched. Needs the grant that guards `[host]`.
class ShellSettings extends ChangeNotifier {
  ShellSettings({required HostFiles files, this.restart, this.saved})
    : _files = files;

  final HostFiles _files;

  /// Has the host start again, which reads what was changed; null where this page cannot.
  final Future<bool> Function()? restart;

  /// Runs after canoa.cfg was written, so what the app holds of it is read again.
  final Future<void> Function()? saved;

  bool _disposed = false;

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  ShellReport _report = const ShellReport();
  String? _problem;
  bool _loading = false, _loaded = false, _restartPending = false;

  ShellReport get report => _report;

  /// Why the list could not be read or the last change was refused, in the host's words.
  String? get problem => _problem;
  bool get loading => _loading;
  bool get loaded => _loaded;
  bool get restartPending => _restartPending;

  /// Reads the shells and the choices from the host.
  Future<void> load() async {
    _loading = true;
    notifyListeners();
    try {
      _report = ShellReport.parse(await _files.shellList());
      _problem = null;
      _loaded = true;
    } on ControlFailed catch (error) {
      _problem = error.code == -32001
          ? 'This key may not list the shells of this host.'
          : error.message;
    } on Object catch (error) {
      _problem = 'The host could not be reached: $error';
    } finally {
      _loading = false;
      notifyListeners();
    }
  }

  /// Makes [shell] (an id the host listed, or a program) the host's shell, or the system's own for null. Returns what is wrong in the
  /// host's words, or null once canoa.cfg holds it.
  Future<String?> chooseForHost(String? shell) => _change((host) {
    if (shell == null) {
      host.remove('shell');
    } else {
      host['shell'] = shell;
    }
    return null;
  });

  /// Makes [shell] the shell of the agent entry [entry], or the host's for null.
  Future<String?> chooseForAgent(String entry, String? shell) =>
      _change((host) {
        final agents = host['agents'];
        final table = agents is Map ? agents[entry] : null;
        if (table is! Map) {
          return 'There is no agent called $entry in the host\'s canoa.cfg.';
        }
        final next = {
          for (final MapEntry(:key, :value) in table.entries) '$key': value,
        };
        if (shell == null) {
          next.remove('shell');
        } else {
          next['shell'] = shell;
        }
        host['agents'] = {
          for (final MapEntry(:key, :value) in (agents as Map).entries)
            '$key': key == entry ? next : value,
        };
        return null;
      });

  /// Reads `[host]` again from the host, lets [edit] change it (or say, as text, why not), has the host check the result and writes
  /// it. Reading it again keeps another page's change to the table.
  Future<String?> _change(
    String? Function(Map<String, Object?> host) edit,
  ) async {
    try {
      final host = {...?await _files.configGet('host')};
      final wrong = edit(host);
      if (wrong != null) return wrong;
      final value = host.isEmpty ? null : host;
      final refused = await _files.configCheck('host', value);
      if (refused != null) return refused;
      await _files.configSet('host', value);
      _restartPending = true;
      _problem = null;
      if (saved != null) await saved!();
      await load();
      return null;
    } on ControlFailed catch (error) {
      return error.code == -32001
          ? "This key may not change the host's own settings."
          : error.message;
    } on Object catch (error) {
      return 'The host could not be reached to save that: $error';
    }
  }

  /// Has the host start again so that it reads the shells now named.
  Future<void> restartHost() async {
    if (restart == null) return;
    if (!await restart!()) return;
    _restartPending = false;
    notifyListeners();
    await load();
  }
}

/// Settings' Shells page: which shell the host's terminals and agents use, chosen from the ones the host found on its computer.
class ShellsSection extends StatefulWidget {
  const ShellsSection({super.key, required this.settings});

  final ShellSettings settings;

  @override
  State<ShellsSection> createState() => _ShellsSectionState();
}

/// The menu value for "none chosen here".
const _inherit = '\u0000inherit';

class _ShellsSectionState extends State<ShellsSection> {
  ShellSettings get settings => widget.settings;

  @override
  void initState() {
    super.initState();
    unawaited(settings.load());
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: settings,
    builder: (context, _) {
      final scheme = Theme.of(context).colorScheme;
      final text = Theme.of(context).textTheme;
      final report = settings.report;
      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            'Agents run their commands with a bash or a PowerShell. Choose which of the shells found on this computer the host starts a '
            'terminal with and tells each agent to use, whether Canoa installed it or not. Nothing here changes a program\'s own settings.',
            style: text.bodyMedium?.copyWith(color: scheme.onSurfaceVariant),
          ),
          const SizedBox(height: 12),
          if (settings.problem case final problem?)
            Padding(
              padding: const EdgeInsets.only(bottom: 12),
              child: Text(
                problem,
                key: const Key('shells-problem'),
                style: TextStyle(color: scheme.error),
              ),
            ),
          if (settings.restartPending)
            Card(
              key: const Key('shells-restart-note'),
              child: Padding(
                padding: const EdgeInsets.all(12),
                child: Row(
                  children: [
                    const Expanded(
                      child: Text(
                        'Saved. The host reads its shells when it starts; the sessions open on it end when it restarts.',
                      ),
                    ),
                    if (settings.restart != null)
                      FilledButton(
                        key: const Key('shells-restart'),
                        onPressed: () => unawaited(settings.restartHost()),
                        child: const Text('Restart the host'),
                      ),
                  ],
                ),
              ),
            ),
          if (settings.loading && !settings.loaded)
            const Padding(
              padding: EdgeInsets.all(12),
              child: LinearProgressIndicator(),
            ),
          if (settings.loaded) ...[
            if (report.shells.isEmpty)
              const Padding(
                padding: EdgeInsets.symmetric(vertical: 8),
                child: Text(
                  'The host found no shell it knows. Install Git Bash or PowerShell from the Store, under Runtimes.',
                  key: Key('shells-none'),
                ),
              ),
            SettingsRow(
              key: const Key('shell-host'),
              title: 'Host shell',
              description: _hostDescription(report),
              control: _menu(
                key: const Key('shell-host-menu'),
                report: report,
                value: report.setting,
                inherit: 'The system\'s own',
                onChanged: (value) =>
                    unawaited(_save(context, settings.chooseForHost(value))),
              ),
            ),
            for (final agent in report.agents)
              SettingsRow(
                key: Key('shell-agent-${agent.entry}'),
                title: agent.entry,
                description: _agentDescription(agent),
                control: _menu(
                  key: Key('shell-agent-menu-${agent.entry}'),
                  report: report,
                  value: agent.own ? agent.setting : null,
                  inherit: 'The host\'s shell',
                  onChanged: (value) => unawaited(
                    _save(context, settings.chooseForAgent(agent.entry, value)),
                  ),
                ),
              ),
          ],
        ],
      );
    },
  );

  Widget _menu({
    required Key key,
    required ShellReport report,
    required String? value,
    required String inherit,
    required void Function(String?) onChanged,
  }) {
    // A setting that names a program, or a shell that is gone, is still what is chosen.
    final known = report.shells.any((shell) => shell.id == value);
    return DropdownButton<String>(
      key: key,
      value: value ?? _inherit,
      items: [
        DropdownMenuItem(value: _inherit, child: Text(inherit)),
        for (final shell in report.shells)
          DropdownMenuItem(
            value: shell.id,
            child: Text(shell.label, overflow: TextOverflow.ellipsis),
          ),
        if (value != null && !known)
          DropdownMenuItem(
            value: value,
            child: Text(value, overflow: TextOverflow.ellipsis),
          ),
      ],
      onChanged: (chosen) =>
          onChanged(chosen == null || chosen == _inherit ? null : chosen),
    );
  }

  static String _hostDescription(ShellReport report) {
    if (report.problem case final problem?) return problem;
    final chosen = report.chosen;
    if (chosen != null) return '${chosen.label}: ${chosen.path}';
    return report.setting == null
        ? 'None chosen: terminals start the person\'s own shell.'
        : report.setting!;
  }

  static String _agentDescription(AgentShell agent) {
    if (agent.problem case final problem?) return problem;
    if (agent.setting == null) {
      return 'No shell is chosen: ${agent.harness} uses its own.';
    }
    final from = agent.own ? 'Its own choice' : 'The host\'s shell';
    if (agent.env.isNotEmpty) {
      return '$from, told to ${agent.harness} as ${agent.env.join(', ')}.';
    }
    return '$from. ${agent.notice ?? ''}'.trim();
  }

  Future<void> _save(BuildContext context, Future<String?> change) async {
    final problem = await change;
    if (problem != null && context.mounted) {
      ScaffoldMessenger.maybeOf(
        context,
      )?.showSnackBar(SnackBar(content: Text(problem)));
    }
  }
}
