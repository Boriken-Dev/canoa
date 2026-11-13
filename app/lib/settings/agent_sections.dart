import 'dart:async';

import 'package:flutter/material.dart';

import '../acp/agent_commands.dart';
import '../chat/modes.dart';
import '../permissions/permission_rules.dart';
import '../permissions/protected_paths.dart';
import '../profile/profile.dart';
import '../sources/source_icons.dart';
import '../sources/sources.dart';
import '../store/programs.dart';
import '../store/programs_view.dart';
import '../theme/look.dart';
import 'setting_layers.dart';
import 'settings_controller.dart';
import 'settings_row.dart';

/// Where a new session's agent comes from, listed as T3 Code lists its providers: the name, the version when it is known,
/// whether it is ready and a switch. A source's detail holds the programs and the command behind it.
class SourcesSection extends StatefulWidget {
  const SourcesSection({super.key, required this.controller, this.presets = const [], this.hosted = const [], this.checkSource, this.programs});

  final SettingsController controller;

  /// The built-in sources, which can be hidden from a new session but not changed.
  final List<AgentSource> presets;

  /// The agent entries of the canoa this app runs, which Hosting changes.
  final List<String> hosted;
  final Future<SourceCheck> Function(AgentSource source)? checkSource;
  final ProgramStores? programs;

  @override
  State<SourcesSection> createState() => _SourcesSectionState();
}

class _SourcesSectionState extends State<SourcesSection> {
  final _name = TextEditingController();
  final _command = TextEditingController();
  String? _problem;
  final Map<AgentSource, Future<SourceCheck>> _checks = {};
  final Set<String> _open = {};
  String? _seenPrograms;

  @override
  void initState() {
    super.initState();
    // Opening the sources is when the publishers are asked for the newest versions (kept for an hour); nothing is installed.
    if (widget.programs case final programs?) {
      programs.addListener(_programsChanged);
      unawaited(programs.refresh(latest: true));
    }
  }

  @override
  void dispose() {
    widget.programs?.removeListener(_programsChanged);
    _name.dispose();
    _command.dispose();
    super.dispose();
  }

  /// A program found, installed, updated or removed changes what its sources need, so they are read again.
  void _programsChanged() {
    final now = widget.programs?.signature;
    if (!mounted || now == _seenPrograms) return;
    _seenPrograms = now;
    setState(_checks.clear);
  }

  Future<SourceCheck> _check(AgentSource source) =>
      _checks[source] ??= widget.checkSource?.call(source) ?? Future.value(const SourceCheck());

  void _add() {
    final config = widget.controller.config;
    final name = _name.text.trim();
    final command = parseCommand(_command.text);
    final taken = {for (final preset in widget.presets) preset.label, for (final agent in config.agents) agent.label};
    final problem = name.isEmpty
        ? 'Name it.'
        : taken.contains(name)
        ? 'Another source is called $name.'
        : command == null
        ? 'That is not a command: check its quotes.'
        : null;
    setState(() => _problem = problem);
    if (problem != null) return;
    _name.clear();
    _command.clear();
    unawaited(widget.controller.change((c) => c.copyWith(agents: [
      ...c.agents,
      AgentCommand(label: name, program: command!.program, args: command.args),
    ])));
  }

  /// What the person is told of a source in one line: ready, or what it needs.
  String _readiness(SourceCheck check) {
    if (check.problem case final problem?) return problem;
    if (check.missing.isNotEmpty) return 'Needs ${check.missing.join(' and ')}';
    if (check.updateAvailable && check.latest != null) return 'Ready, ${check.latest} is available';
    return 'Ready';
  }

  /// A source's detail: where its programs are, or what it needs and how to put that right, with the Store's buttons.
  Widget _state(AgentSource source, SourceCheck check) {
    final programs = widget.programs;
    final status = check.store == null ? null : programs?.status(check.store!);
    if (programs != null && status != null) {
      return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text(statusLines(status).join('\n'), key: Key('source-state ${source.label}')),
        if (status.problem case final problem?) Text(problem, style: TextStyle(color: Theme.of(context).colorScheme.error)),
        const SizedBox(height: 4),
        ProgramButtons(programs: programs, status: status),
      ]);
    }
    final problem = check.describe(source.label);
    if (problem != null) return Text(problem, style: TextStyle(color: Theme.of(context).colorScheme.error));
    return Text(check.found.isEmpty ? 'Ready' : check.found.entries.map((found) => '${found.key}: ${found.value}').join('\n'));
  }

  Widget _row({
    required AgentSource source,
    required String key,
    Widget? toggle,
    Widget? detail,
    String? command,
    List<String>? configKey,
  }) => FutureBuilder<SourceCheck>(
    future: _check(source),
    builder: (context, snapshot) {
      final check = snapshot.data;
      final scheme = Theme.of(context).colorScheme;
      final open = _open.contains(key);
      final ready = check?.ready ?? false;
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        InkWell(
          key: Key('source-row $key'),
          onTap: () => setState(() => open ? _open.remove(key) : _open.add(key)),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
            child: Row(children: [
              SourceMark.of(source, size: 28),
              const SizedBox(width: 12),
              Expanded(
                child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                  Wrap(crossAxisAlignment: WrapCrossAlignment.center, spacing: 8, children: [
                    Text(source.displayName, style: Theme.of(context).textTheme.bodyLarge),
                    if (check?.version case final version?)
                      Text('v${version.replaceFirst(RegExp('^v'), '')}',
                          key: Key('source-version $key'),
                          style: TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12, color: scheme.onSurfaceVariant)),
                    if (check?.updateAvailable ?? false) Icon(Icons.arrow_circle_up, size: 16, color: scheme.primary),
                  ]),
                  if (configKey != null) SettingNote(configKey: configKey),
                  Text(
                    check == null ? 'Looking for what it needs…' : _readiness(check),
                    key: Key('source-ready $key'),
                    style: Theme.of(context).textTheme.bodySmall?.copyWith(color: check != null && !ready ? scheme.error : scheme.onSurfaceVariant),
                  ),
                ]),
              ),
              ?toggle,
              Icon(open ? Icons.expand_less : Icons.expand_more),
            ]),
          ),
        ),
        if (open)
          Padding(
            key: Key('source-detail $key'),
            padding: const EdgeInsets.fromLTRB(56, 0, 16, 12),
            child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
              if (command != null)
                SelectableText(command, style: const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12)),
              if (check != null) _state(source, check) else const LinearProgressIndicator(),
              ?detail,
            ]),
          ),
      ]);
    },
  );

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.controller,
    builder: (context, _) {
      final c = widget.controller;
      final config = c.config;
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        const Padding(
          padding: EdgeInsets.only(bottom: 12),
          child: Text('Where a new session\'s agent comes from. Adding or removing one restarts Canoa\'s host, which ends the sessions open now.'),
        ),
        SettingsGroup(title: 'Sources', children: [
          for (final preset in widget.presets)
            _row(
              source: preset,
              key: preset.label,
              configKey: const ['sources', 'hidden'],
              toggle: Switch(
                key: Key('source-offered ${preset.label}'),
                value: !config.hiddenSources.contains(preset.label),
                onChanged: (on) => c.change((x) {
                  final hidden = {...x.hiddenSources};
                  on ? hidden.remove(preset.label) : hidden.add(preset.label);
                  return x.copyWith(hiddenSources: hidden);
                }),
              ),
            ),
          for (final agent in config.agents)
            _row(
              source: AgentSource.hosted(agent.label, local: true),
              key: agent.label,
              configKey: ['agents', agent.label],
              command: [agent.program, ...agent.args].join(' '),
              detail: Align(
                alignment: Alignment.centerRight,
                child: TextButton.icon(
                  key: Key('source-remove ${agent.label}'),
                  icon: const Icon(Icons.delete_outline),
                  label: Text('Remove ${agent.label}'),
                  onPressed: () => c.change((x) => x.copyWith(agents: [for (final each in x.agents) if (each != agent) each])),
                ),
              ),
            ),
          for (final name in widget.hosted)
            SettingsRow(title: name, description: 'Served by canoa: Hosting changes it'),
          if (widget.presets.isEmpty && config.agents.isEmpty && widget.hosted.isEmpty)
            const SettingsRow(title: 'No sources yet', description: 'Add a command below'),
        ]),
        SettingsGroup(title: 'Add a source', children: [
          SettingsRow(
            title: 'Name',
            below: TextField(key: const Key('source-name'), controller: _name, decoration: const InputDecoration(isDense: true, border: OutlineInputBorder())),
          ),
          SettingsRow(
            title: 'Command',
            description: 'An ACP agent on standard streams',
            below: TextField(
              key: const Key('source-command'),
              controller: _command,
              decoration: InputDecoration(isDense: true, border: const OutlineInputBorder(), errorText: _problem, errorMaxLines: 3),
              onSubmitted: (_) => _add(),
            ),
          ),
          SettingsRow(title: 'Add it', control: FilledButton(key: const Key('source-add'), onPressed: _add, child: const Text('Add'))),
        ]),
      ]);
    },
  );
}

/// The MCP servers of the person's own profile, each switched on or off for the agent chosen. Servers an overlay brings, and
/// those to find or remove, are in Loadout.
class McpSection extends StatefulWidget {
  const McpSection({super.key, required this.controller, this.agents = const [], this.onLoadout});

  final SettingsController controller;
  final List<String> agents;
  final VoidCallback? onLoadout;

  @override
  State<McpSection> createState() => _McpSectionState();
}

class _McpSectionState extends State<McpSection> {
  String? _agent;

  static String _describe(McpServer server) => switch (server) {
    McpStdio(:final command, :final args) => 'Program: ${[command, ...args].join(' ')}',
    McpHttp(:final url) => 'Address: ${Uri.tryParse(url)?.replace(query: '', fragment: '') ?? url}',
  };

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.controller,
    builder: (context, _) {
      final c = widget.controller;
      final agent = widget.agents.contains(_agent) ? _agent : widget.agents.firstOrNull;
      final servers = [for (final MapEntry(:key, :value) in c.config.profile.mcp.entries) if (value != null) (key, value)];
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        ScopeBar(label: 'agent', value: agent, choices: widget.agents, onChanged: (picked) => setState(() => _agent = picked)),
        SettingsGroup(title: 'Your servers', children: [
          if (servers.isEmpty)
            const SettingsRow(title: 'None of your own', description: 'Find and install some in Loadout'),
          for (final (name, server) in servers)
            SwitchRow(
              key: Key('mcp-$name'),
              title: name,
              configKey: ['profile', 'mcp', name],
              description: _describe(server),
              enabled: agent != null,
              value: agent == null || c.config.agentMcp[agent]?[name] != false,
              onChanged: (on) => c.change((x) => x.withAgentMcp(agent!, {...?x.agentMcp[agent], name: on})),
            ),
        ]),
        if (widget.onLoadout != null)
          Align(
            alignment: Alignment.centerLeft,
            child: TextButton(key: const Key('mcp-loadout'), onPressed: widget.onLoadout, child: const Text('Loadout…')),
          ),
      ]);
    },
  );
}

/// The protected list's text, with the lines the recommended list also has shown in the accent colour.
class _ProtectedController extends TextEditingController {
  _ProtectedController(String text) : super(text: text);

  @override
  TextSpan buildTextSpan({required BuildContext context, TextStyle? style, required bool withComposing}) {
    final mark = style?.copyWith(color: Theme.of(context).colorScheme.primary);
    final recommended = recommendedProtected.map((line) => line.trim()).where((line) => line.isNotEmpty).toSet();
    final lines = text.split('\n');
    return TextSpan(style: style, children: [
      for (var i = 0; i < lines.length; i++)
        TextSpan(text: i == lines.length - 1 ? lines[i] : '${lines[i]}\n', style: recommended.contains(lines[i].trim()) ? mark : null),
    ]);
  }
}

/// What a new session starts in, Bypass and the log of what it answered, the rules "Always allow here" made, and the paths no
/// rule covers.
class PermissionsSection extends StatefulWidget {
  const PermissionsSection({super.key, required this.controller, this.rules, this.sources = const []});

  final SettingsController controller;
  final PermissionRules? rules;

  /// The sources a rule can be for, besides those the rules name.
  final List<String> sources;

  @override
  State<PermissionsSection> createState() => _PermissionsSectionState();
}

class _PermissionsSectionState extends State<PermissionsSection> {
  static const _all = 'All sources';
  late final _protected = _ProtectedController((widget.controller.config.protectedPaths ?? recommendedProtected).join('\n'));
  String _scope = _all;

  @override
  void dispose() {
    _protected.dispose();
    super.dispose();
  }

  void _setProtected(List<String>? lines) {
    widget.controller.change((c) => lines == null ? c.copyWith(clearProtectedPaths: true) : c.copyWith(protectedPaths: lines));
    widget.rules?.useProtected(lines == null ? null : ProtectedPaths(lines));
  }

  Widget _protectedPaths(BuildContext context) {
    final recommended = recommendedProtected.map((line) => line.trim()).where((line) => line.isNotEmpty && !line.startsWith('#')).toList();
    final mine = _protected.text.split('\n').map((line) => line.trim()).toSet();
    final removed = recommended.where((line) => !mine.contains(line)).length;
    return SettingsGroup(title: 'Protected paths', children: [
      SettingsRow(
        title: 'Paths a remembered rule never covers',
        configKey: const ['permissions', 'protected'],
        onLayerReset: () {
          widget.controller.change((c) => c.resetKey(const ['permissions', 'protected']));
          _protected.text = (widget.controller.config.protectedPaths ?? recommendedProtected).join('\n');
          widget.rules?.useProtected(widget.controller.config.protectedPaths == null ? null : ProtectedPaths(widget.controller.config.protectedPaths!));
        },
        description: 'One pattern a line, as in a gitignore file: a "!" line allows one back, the last line that matches wins, "#" '
            'starts a comment, "~/" is your home folder, and a pattern without a start matches anywhere. Lines in the accent '
            'colour are Canoa\'s recommendation.',
        below: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          TextField(
            key: const Key('settings-protected-paths'),
            controller: _protected,
            minLines: 6,
            maxLines: 14,
            style: const TextStyle(fontFamily: 'monospace'),
            decoration: const InputDecoration(border: OutlineInputBorder(), isDense: true),
            onChanged: (text) {
              setState(() {});
              final lines = text.split('\n');
              final same = lines.length == recommendedProtected.length && [for (var i = 0; i < lines.length; i++) lines[i] == recommendedProtected[i]].every((v) => v);
              _setProtected(same ? null : lines);
            },
          ),
          Row(children: [
            Expanded(
              child: Text(removed == 0 ? 'Every recommended line is kept.' : '$removed recommended ${removed == 1 ? 'line is' : 'lines are'} removed.',
                  key: const Key('protected-paths-state')),
            ),
            TextButton(
              key: const Key('restore-recommended-paths'),
              onPressed: () {
                setState(() => _protected.text = recommendedProtected.join('\n'));
                _setProtected(null);
              },
              child: const Text('Restore recommended'),
            ),
          ]),
        ]),
      ),
    ]);
  }

  /// The rules made by "Always allow here", each with when it was made and how often it answered, and a way to take it back.
  Widget _remembered(BuildContext context, PermissionRules rules) => ListenableBuilder(
    listenable: rules,
    builder: (context, _) {
      final all = rules.rules;
      final sources = {...widget.sources, for (final rule in all) rule.source}.toList()..sort();
      final scope = sources.contains(_scope) ? _scope : _all;
      final shown = [for (final rule in all) if (scope == _all || rule.source == scope) rule];
      String day(DateTime at) => at.toLocal().toIso8601String().substring(0, 10);
      final error = Theme.of(context).colorScheme.error;
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        ScopeBar(label: 'source', value: scope, choices: [_all, ...sources], onChanged: (picked) => setState(() => _scope = picked)),
        SettingsGroup(title: 'Remembered rules', children: [
          SettingsRow(
            title: all.isEmpty ? 'Nothing is allowed without asking' : '${all.length} remembered',
            description: all.isEmpty
                ? '"Always allow here" on a permission card remembers a rule for a source and a folder; it is listed here'
                : 'Each is answered with allow once',
          ),
          if (rules.problem case final problem?) SettingsRow(title: problem),
          if (rules.unsaved)
            const SettingsRow(
              key: Key('rules-unsaved'),
              title: 'Not yet saved',
              description: 'A change could not be written. It is in effect here, and written when the file allows. If Canoa quits first, a revoke is lost.',
            ),
          if (rules.writeProblem case final problem?)
            SettingsRow(key: const Key('rules-write-problem'), title: problem),
          for (final rule in shown)
            SettingsRow(
              key: Key('rule-${rule.id}'),
              title: rule.describe,
              description: '${rule.source} in ${rule.folder}. Made ${day(rule.made)}; used ${rule.fired == 1 ? 'once' : '${rule.fired} times'}'
                  '${rule.lastUsed == null ? '' : ', last ${day(rule.lastUsed!)}'}'
                  '${rule.external ? '. Not made in this app: the file was edited, or another program wrote it.' : ''}',
              control: TextButton(key: Key('revoke-${rule.id}'), onPressed: () => rules.revoke(rule.id), child: Text('Revoke', style: TextStyle(color: error))),
            ),
        ]),
      ]);
    },
  );

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.controller,
    builder: (context, _) {
      final c = widget.controller;
      final config = c.config;
      final rules = widget.rules;
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        SettingsGroup(title: 'New sessions', children: [
          ChoiceRow<String?>(
            title: 'Mode of a new session',
            configKey: const ['permissions', 'default_mode'],
            description: 'Each session can change it from the composer, and keeps its own',
            fieldKey: const Key('settings-default-mode'),
            value: config.defaultMode,
            choices: [(null, "The agent's own"), for (final mode in modeOrder) (mode, modeLabels[mode] ?? mode)],
            onChanged: (mode) => c.change((x) => mode == null ? x.copyWith(clearDefaultMode: true) : x.copyWith(defaultMode: mode)),
          ),
        ]),
        SettingsGroup(title: 'Bypass', children: [
          SwitchRow(
            key: const Key('settings-allow-once-only'),
            title: 'Answer with allow once only',
            configKey: const ['permissions', 'allow_once_only'],
            description: 'A request that offers only an "always" answer is then asked, not answered. Off, Bypass takes the "always" the agent offers',
            value: config.allowOnceOnly,
            onChanged: (v) => c.change((x) => x.copyWith(allowOnceOnly: v)),
          ),
          if (rules != null)
            SettingsRow(
              title: 'Log',
              description: 'Every rule made, used or revoked is appended here, and never rewritten',
              machine: true,
              below: SelectableText(rules.log?.location ?? 'No log is kept here', key: const Key('rules-log-location'),
                  style: const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12)),
            ),
        ]),
        if (rules != null) _remembered(context, rules),
        _protectedPaths(context),
      ]);
    },
  );
}

/// What the app has of usage today: when its control turns to a warning.
class UsageSection extends StatelessWidget {
  const UsageSection({super.key, required this.controller});

  final SettingsController controller;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) {
      final warn = controller.config.warnPermille;
      return SettingsGroup(children: [
        ChoiceRow<int>(
          title: 'Warn from',
          configKey: const ['usage', 'warn_permille'],
          description: 'The usage control shows a warning once this share of a limit is used',
          fieldKey: const Key('settings-warn'),
          value: warn,
          choices: [for (final p in {500, 600, 700, 800, 900, 950, warn}.toList()..sort()) (p, p % 10 == 0 ? '${p ~/ 10}%' : '${p / 10}%')],
          onChanged: (v) => controller.change((c) => c.copyWith(warnPermille: v)),
        ),
      ]);
    },
  );
}
