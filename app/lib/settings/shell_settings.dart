import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../l10n/app_text.dart';
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

  /// The key of the words that say where it comes from.
  String get originKey => 'shells.origin.$origin';
}

/// One agent entry of the host and how it is told its shell, as `shell/list` words it.
final class AgentShell {
  const AgentShell({
    this.entry,
    required this.harness,
    this.title,
    this.hosted = true,
    this.setting,
    this.own = false,
    this.id,
    this.env = const [],
    this.notice,
    this.problem,
  });

  /// The entry that runs it; null for an agent no entry runs yet.
  final String? entry;

  /// The agent's id, and its name for people when the host gives one.
  final String harness;
  final String? title;

  /// Whether an entry of canoa.cfg runs it, which is what a shell setting is written to.
  final bool hosted;

  /// What the agent is called.
  String get name => entry ?? title ?? harness;

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
    if (json is! Map || json['harness'] is! String) return null;
    return AgentShell(
      entry: json['entry'] is String ? json['entry'] as String : null,
      harness: json['harness']! as String,
      title: json['title'] is String ? json['title'] as String : null,
      hosted: json['hosted'] != false,
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
  ShellSettings({required this._files, this.restart, this.saved});

  final HostFiles _files;

  /// Has the host start again, which reads what was changed; null where this page cannot.
  final Future<bool> Function()? restart;

  /// Runs after canoa.cfg was written, so what the app holds of it is read again.
  final Future<void> Function()? saved;

  /// Says a word of this page in the person's language; the page sets it, and the base English is used before it does.
  String Function(String key, [Map<String, Object?> args]) say =
      AppText.base.call;

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
          ? say('shells.denied_list')
          : error.message;
    } on Object catch (error) {
      _problem = say('shells.unreachable', {'why': '$error'});
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
          return say('shells.no_entry', {'name': entry});
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
      return error.code == -32001 ? say('shells.denied_write') : error.message;
    } on Object catch (error) {
      return say('shells.save_unreachable', {'why': '$error'});
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
  void didChangeDependencies() {
    super.didChangeDependencies();
    settings.say = context.t;
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
            context.t('shells.intro'),
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
                    Expanded(child: Text(context.t('shells.restart_note'))),
                    if (settings.restart != null)
                      FilledButton(
                        key: const Key('shells-restart'),
                        onPressed: () => unawaited(settings.restartHost()),
                        child: Text(context.t('shells.restart')),
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
              Padding(
                padding: const EdgeInsets.symmetric(vertical: 8),
                child: Text(
                  context.t('shells.none'),
                  key: const Key('shells-none'),
                ),
              ),
            SettingsGroup(
              children: [
                SettingsRow(
                  key: const Key('shell-host'),
                  title: context.t('shells.host'),
                  description: _hostDescription(context, report),
                  control: _menu(
                    context,
                    key: const Key('shell-host-menu'),
                    report: report,
                    value: report.setting,
                    inherit: context.t('shells.host_inherit'),
                    onChanged: (value) => unawaited(
                      _save(context, settings.chooseForHost(value)),
                    ),
                  ),
                ),
              ],
            ),
            if (report.agents.isNotEmpty)
              SettingsGroup(
                title: context.t('shells.agents'),
                children: [
                  for (final agent in report.agents)
                    SettingsRow(
                      key: Key(
                        agent.entry == null
                            ? 'shell-agent-unhosted-${agent.harness}'
                            : 'shell-agent-${agent.entry}',
                      ),
                      title: agent.name,
                      description: _agentDescription(context, agent),
                      control: agent.entry == null
                          ? null
                          : _menu(
                              context,
                              key: Key('shell-agent-menu-${agent.entry}'),
                              report: report,
                              value: agent.own ? agent.setting : null,
                              inherit: context.t('shells.agent_inherit'),
                              onChanged: (value) => unawaited(
                                _save(
                                  context,
                                  settings.chooseForAgent(agent.entry!, value),
                                ),
                              ),
                            ),
                    ),
                ],
              ),
            if (report.shells.isNotEmpty)
              SettingsGroup(
                title: context.t('shells.found'),
                children: [
                  for (final shell in report.shells)
                    SettingsRow(
                      key: Key('shell-found-${shell.id}'),
                      title: shell.label,
                      description: _where(context, shell),
                    ),
                ],
              ),
          ],
        ],
      );
    },
  );

  /// Where a shell comes from and its program, in the person's words.
  static String _where(BuildContext context, HostShell shell) => context.t(
    'shells.where',
    {'origin': context.t(shell.originKey), 'path': shell.path},
  );

  Widget _menu(
    BuildContext context, {
    required Key key,
    required ShellReport report,
    required String? value,
    required String inherit,
    required void Function(String?) onChanged,
  }) {
    // A setting that names a program, or a shell that is gone, is still what is chosen.
    final known = report.shells.any((shell) => shell.id == value);
    return SizedBox(
      width: 260,
      child: DropdownButton<String>(
        key: key,
        isExpanded: true,
        value: value ?? _inherit,
        items: [
          DropdownMenuItem(value: _inherit, child: Text(inherit)),
          for (final shell in report.shells)
            DropdownMenuItem(
              value: shell.id,
              child: Text(
                context.t('shells.choice', {
                  'label': shell.label,
                  'origin': context.t(shell.originKey),
                }),
                overflow: TextOverflow.ellipsis,
              ),
            ),
          if (value != null && !known)
            DropdownMenuItem(
              value: value,
              child: Text(value, overflow: TextOverflow.ellipsis),
            ),
        ],
        onChanged: (chosen) =>
            onChanged(chosen == null || chosen == _inherit ? null : chosen),
      ),
    );
  }

  static String _hostDescription(BuildContext context, ShellReport report) {
    if (report.problem case final problem?) return problem;
    final chosen = report.chosen;
    if (chosen != null) return '${chosen.label}. ${_where(context, chosen)}';
    return report.setting == null
        ? context.t('shells.host_none')
        : report.setting!;
  }

  static String _agentDescription(BuildContext context, AgentShell agent) {
    if (agent.problem case final problem?) return problem;
    if (!agent.hosted) {
      return context.t('shells.unhosted', {'agent': agent.name});
    }
    if (agent.setting == null) {
      return context.t('shells.unset', {'agent': agent.name});
    }
    final from = context.t(agent.own ? 'shells.own' : 'shells.from_host');
    if (agent.env.isNotEmpty) {
      return context.t('shells.told', {
        'from': from,
        'agent': agent.name,
        'env': agent.env.join(', '),
      });
    }
    return '$from. ${agent.notice ?? ''}'.trim();
  }

  Future<void> _save(BuildContext context, Future<String?> change) async {
    final problem = await change;
    if (problem != null && context.mounted) {
      ScaffoldMessenger.maybeOf(context)
          ?.showSnackBar(SnackBar(content: Text(problem)));
    }
  }
}
