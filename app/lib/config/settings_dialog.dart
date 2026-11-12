import 'dart:async';

import 'package:flutter/material.dart';
import 'canoa_config.dart';
import '../chat/modes.dart';
import '../acp/agent_commands.dart';
import '../ledger/session_ledger.dart';
import '../permissions/permission_rules.dart';
import '../permissions/protected_paths.dart';
import '../sources/sources.dart';
import '../store/programs.dart';
import '../store/programs_view.dart';
import '../theme/look.dart';
import '../workspace/workspace_template.dart';

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

class SettingsDialog extends StatefulWidget {
  const SettingsDialog({super.key, required this.config, this.onQuit, this.ledger, this.rules, this.onProfiles,
      this.onAllowNotifications, this.onPreview, this.workspacePreview, this.presets = const [], this.hosted = const [], this.checkSource, this.programs, this.openSources = false});
  final CanoaConfig config;

  /// The built-in sources, which can be hidden from a new session but not changed.
  final List<AgentSource> presets;

  /// The agent entries of the canoa this app runs, which Hosting changes.
  final List<String> hosted;

  /// What a source needs and whether it is here.
  final Future<SourceCheck> Function(AgentSource source)? checkSource;

  /// The programs the Store reports, which the sources show the version of and install or update; none where the host has no Store.
  final ProgramStores? programs;

  /// Whether the dialog opens with its sources shown.
  final bool openSources;

  /// Asks the browser for leave to notify, where it has to be asked; true when it was given.
  final Future<bool> Function()? onAllowNotifications;

  /// Told the dialog's look and brightness each time either is picked, so the app can show them before they are saved.
  final void Function(CanoaConfig preview)? onPreview;

  /// The folder a template makes for a session, as the app would make it now; null where this build has no folders for
  /// sessions, and Settings then shows no workspace row.
  final String? Function(String template)? workspacePreview;

  /// The session list, whose kept text Settings can clear.
  final SessionLedger? ledger;

  /// What the person told Canoa to allow without asking, which Settings lists and revokes at once, apart from Save.
  final PermissionRules? rules;

  /// Opens the effective profile of each agent.
  final VoidCallback? onProfiles;

  /// Ends the app and what it started, on a desktop: the way out where there is no tray menu.
  final Future<void> Function()? onQuit;
  @override
  State<SettingsDialog> createState() => SettingsDialogState();
}
class SettingsDialogState extends State<SettingsDialog> {
  late CanoaConfig config = widget.config;
  final _sourceName = TextEditingController();
  late final _protected = _ProtectedController((widget.config.protectedPaths ?? recommendedProtected).join('\n'));
  final _sourceCommand = TextEditingController();
  late final _workspace = TextEditingController(text: widget.config.sessions.workspace);
  String? _workspaceProblem;
  String? _sourceProblem;
  final Map<AgentSource, Future<SourceCheck>> _checks = {};

  @override
  void dispose() {
    widget.programs?.removeListener(_programsChanged);
    _sourceName.dispose();
    _sourceCommand.dispose();
    _workspace.dispose();
    _protected.dispose();
    super.dispose();
  }

  @override
  void initState() {
    super.initState();
    // Opening Settings is when the publishers are asked for the newest versions (kept for an hour); nothing is installed.
    if (widget.programs case final programs?) {
      programs.addListener(_programsChanged);
      unawaited(programs.refresh(latest: true));
    }
  }

  /// A program found, installed, updated or removed changes what its sources need, so they are read again.
  void _programsChanged() {
    if (!mounted) return;
    setState(_checks.clear);
  }

  Future<SourceCheck> _check(AgentSource source) =>
      _checks[source] ??= widget.checkSource?.call(source) ?? Future.value(const SourceCheck());

  /// A source's state in a line: where its programs are, or what it needs and how to put that right.
  Widget _state(AgentSource source) => FutureBuilder<SourceCheck>(
    future: _check(source),
    builder: (context, snapshot) {
      final check = snapshot.data;
      if (check == null) return const Text('Looking for what it needs…');
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
    },
  );

  void _addSource() {
    final name = _sourceName.text.trim();
    final command = parseCommand(_sourceCommand.text);
    final taken = {for (final preset in widget.presets) preset.label, for (final agent in config.agents) agent.label};
    final problem = name.isEmpty
        ? 'Name it.'
        : taken.contains(name)
        ? 'Another source is called $name.'
        : command == null
        ? 'That is not a command: check its quotes.'
        : null;
    if (problem != null) {
      setState(() => _sourceProblem = problem);
      return;
    }
    setState(() {
      _sourceProblem = null;
      config = config.copyWith(agents: [
        ...config.agents,
        AgentCommand(label: name, program: command!.program, args: command.args),
      ]);
      _sourceName.clear();
      _sourceCommand.clear();
    });
  }

  /// The rules made by "Always allow here", each with when it was made and how often it answered, and a way to take it back.
  Widget _permissions(BuildContext context) => ListenableBuilder(
    listenable: widget.rules!,
    builder: (context, _) {
      final rules = widget.rules!.rules;
      String day(DateTime at) => at.toLocal().toIso8601String().substring(0, 10);
      return ExpansionTile(
        key: const Key('settings-permissions'),
        title: const Text('Permissions'),
        subtitle: Text(rules.isEmpty ? 'Nothing is allowed without asking' : '${rules.length} remembered, each answered with allow once'),
        expandedCrossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (widget.rules!.problem case final problem?)
            Padding(padding: const EdgeInsets.symmetric(horizontal: 16), child: Text(problem, style: TextStyle(color: Theme.of(context).colorScheme.error))),
          if (widget.rules!.unsaved)
            const Padding(
              padding: EdgeInsets.symmetric(horizontal: 16),
              child: Text(
                'Not yet saved: a change could not be written. It is in effect here, and written when the file allows. If Canoa quits '
                'first, a revoke is lost.',
                key: Key('rules-unsaved'),
              ),
            ),
          if (widget.rules!.writeProblem case final problem?)
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 16),
              child: Text(problem, key: const Key('rules-write-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
            ),
          if (rules.isEmpty)
            const ListTile(title: Text('"Always allow here" on a permission card remembers a rule for a source and a folder; it is listed here.')),
          for (final rule in rules)
            ListTile(
              key: Key('rule-${rule.id}'),
              title: Text(rule.describe),
              subtitle: Text(
                '${rule.source} in ${rule.folder}\n'
                'Made ${day(rule.made)}; used ${rule.fired == 1 ? 'once' : '${rule.fired} times'}'
                '${rule.lastUsed == null ? '' : ', last ${day(rule.lastUsed!)}'}'
                '${rule.external ? '\nNot made in this app: the file was edited, or another program wrote it.' : ''}',
              ),
              isThreeLine: !rule.external,
              trailing: TextButton(key: Key('revoke-${rule.id}'), onPressed: () => widget.rules!.revoke(rule.id), child: const Text('Revoke')),
            ),
          _protectedPaths(context),
        ],
      );
    },
  );

  /// What a remembered rule never covers: a path on this list asks every time. One pattern a line, like a gitignore file.
  Widget _protectedPaths(BuildContext context) {
    final recommended = recommendedProtected.map((line) => line.trim()).where((line) => line.isNotEmpty && !line.startsWith('#')).toList();
    final mine = _protected.text.split('\n').map((line) => line.trim()).toSet();
    final removed = recommended.where((line) => !mine.contains(line)).length;
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 8, 16, 8),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Text('Protected paths', style: Theme.of(context).textTheme.titleSmall),
        const SizedBox(height: 4),
        const Text(
          'A rule made with "Always allow here" never covers a path on this list: it asks each time. One pattern a line, as in a '
          'gitignore file: a "!" line allows one back, the last line that matches wins, "#" starts a comment, "~/" is your home '
          'folder, and a pattern without a start matches anywhere. Lines in the accent colour are Canoa\'s recommendation; remove '
          'any you do not want.',
        ),
        const SizedBox(height: 8),
        TextField(
          key: const Key('settings-protected-paths'),
          controller: _protected,
          minLines: 6,
          maxLines: 14,
          style: const TextStyle(fontFamily: 'monospace'),
          decoration: const InputDecoration(border: OutlineInputBorder(), isDense: true),
          onChanged: (text) => setState(() {
            final lines = text.split('\n');
            final same = lines.length == recommendedProtected.length && [for (var i = 0; i < lines.length; i++) lines[i] == recommendedProtected[i]].every((v) => v);
            config = same ? config.copyWith(clearProtectedPaths: true) : config.copyWith(protectedPaths: lines);
          }),
        ),
        Row(children: [
          Expanded(
            child: Text(removed == 0 ? 'Every recommended line is kept.' : '$removed recommended ${removed == 1 ? 'line is' : 'lines are'} removed.',
                key: const Key('protected-paths-state')),
          ),
          TextButton(
            key: const Key('restore-recommended-paths'),
            onPressed: () => setState(() {
              _protected.text = recommendedProtected.join('\n');
              config = config.copyWith(clearProtectedPaths: true);
            }),
            child: const Text('Restore recommended'),
          ),
        ]),
      ]),
    );
  }

  Widget _sources(BuildContext context) => ExpansionTile(
    key: const Key('settings-sources'),
    initiallyExpanded: widget.openSources,
    title: const Text('Sources'),
    subtitle: const Text('Where a new session\'s agent comes from. Adding or removing one restarts Canoa\'s host, which ends the sessions open now.'),
    expandedCrossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      for (final preset in widget.presets)
        SwitchListTile(
          key: Key('source-offered ${preset.label}'),
          title: Text(preset.label),
          subtitle: _state(preset),
          value: !config.hiddenSources.contains(preset.label),
          onChanged: (on) => setState(() {
            final hidden = {...config.hiddenSources};
            on ? hidden.remove(preset.label) : hidden.add(preset.label);
            config = config.copyWith(hiddenSources: hidden);
          }),
        ),
      for (final agent in config.agents)
        ListTile(
          key: Key('source ${agent.label}'),
          title: Text(agent.label),
          subtitle: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Text([agent.program, ...agent.args].join(' '), style: const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12)),
            _state(AgentSource.hosted(agent.label, local: true)),
          ]),
          trailing: IconButton(
            tooltip: 'Remove ${agent.label}',
            icon: const Icon(Icons.delete_outline),
            onPressed: () => setState(() =>
                config = config.copyWith(agents: [for (final each in config.agents) if (each != agent) each])),
          ),
        ),
      for (final name in widget.hosted)
        ListTile(title: Text(name), subtitle: const Text('Served by canoa: Hosting changes it')),
      Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
        child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          TextField(key: const Key('source-name'), controller: _sourceName,
              decoration: const InputDecoration(labelText: 'Name of a source to add')),
          TextField(key: const Key('source-command'), controller: _sourceCommand,
              decoration: const InputDecoration(labelText: 'Its command, an ACP agent on standard streams'),
              onSubmitted: (_) => _addSource()),
          if (_sourceProblem != null)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Text(_sourceProblem!, style: TextStyle(color: Theme.of(context).colorScheme.error)),
            ),
          Align(
            alignment: Alignment.centerRight,
            child: TextButton(key: const Key('source-add'), onPressed: _addSource, child: const Text('Add')),
          ),
        ]),
      ),
    ],
  );

  LedgerSettings _sessions({int? keep, bool? keepText, String? workspace}) => LedgerSettings(
    keep: keep ?? config.sessions.keep,
    keepText: keepText ?? config.sessions.keepText,
    textMessages: config.sessions.textMessages,
    workspace: workspace ?? config.sessions.workspace,
    harnesses: config.sessions.harnesses,
  );

  /// The template of the folder a session with none chosen works in, with what it makes now.
  Widget _workspaceRow() {
    final problem = _workspaceProblem;
    final preview = problem == null ? widget.workspacePreview!(_workspace.text) : null;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      child: TextField(
        key: const Key('settings-workspace'),
        controller: _workspace,
        decoration: InputDecoration(
          labelText: 'Folder of a session with none chosen',
          helperText: preview == null ? null : 'Now: $preview',
          helperMaxLines: 3,
          errorText: problem,
          errorMaxLines: 3,
          suffixIcon: _workspace.text == defaultWorkspace
              ? null
              : IconButton(
                  key: const Key('settings-workspace-reset'),
                  tooltip: 'Use the default',
                  icon: const Icon(Icons.restart_alt),
                  onPressed: () => _setWorkspace(defaultWorkspace, text: true),
                ),
        ),
        onChanged: _setWorkspace,
      ),
    );
  }

  void _setWorkspace(String template, {bool text = false}) => setState(() {
    if (text) _workspace.text = template;
    _workspaceProblem = workspaceProblem(template);
    if (_workspaceProblem == null) config = config.copyWith(sessions: _sessions(workspace: template));
  });

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Canoa settings'),
    content: SizedBox(width: 400, child: SingleChildScrollView(child: Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        ListTile(
          title: const Text('Look'),
          trailing: DropdownButton<LookId>(
            key: const Key('settings-look'),
            value: config.look,
            items: [for (final look in LookId.values) DropdownMenuItem(value: look, child: Text(look.label))],
            onChanged: (value) {
              if (value == null) return;
              setState(() => config = config.copyWith(look: value));
              widget.onPreview?.call(config);
            },
          ),
        ),
        ListTile(
          title: const Text('Brightness'),
          trailing: DropdownButton<String>(
            key: const Key('settings-brightness'),
            value: config.brightness,
            items: const [
              DropdownMenuItem(value: 'dark', child: Text('Dark')),
              DropdownMenuItem(value: 'light', child: Text('Light')),
              DropdownMenuItem(value: 'system', child: Text('Follow the system')),
            ],
            onChanged: (value) {
              if (value == null) return;
              setState(() => config = config.copyWith(brightness: value));
              widget.onPreview?.call(config);
            },
          ),
        ),
        SwitchListTile(
          title: const Text('Start hidden in tray'),
          value: config.startHidden,
          onChanged: (v) => setState(() => config = config.copyWith(startHidden: v)),
        ),
        SwitchListTile(
          title: const Text('Close window to tray'),
          value: config.closeToTray,
          onChanged: (v) => setState(() => config = config.copyWith(closeToTray: v)),
        ),
        SwitchListTile(
          title: const Text('Minimize window to tray'),
          value: config.minimizeToTray,
          onChanged: (v) => setState(() => config = config.copyWith(minimizeToTray: v)),
        ),
        SwitchListTile(
          title: const Text('Prefer WebTransport (WSS fallback)'),
          value: config.preferWebTransport,
          onChanged: (v) => setState(() =>
              config = config.copyWith(preferWebTransport: v)),
        ),
        ListTile(
          title: const Text('Network timeout'),
          subtitle: Text('${config.connectTimeout} seconds'),
          trailing: DropdownButton<int>(
            value: config.connectTimeout,
            items: [5,10,15,30,60].map((n) => DropdownMenuItem(
              value: n, child: Text('$n s'))).toList(),
            onChanged: (value) {
              if (value != null) setState(() => config = config.copyWith(connectTimeout: value));
            },
          ),
        ),
        ExpansionTile(
          key: const Key('settings-notifications'),
          title: const Text('Notifications'),
          subtitle: const Text('When a conversation you are not looking at needs you'),
          children: [
            for (final (label, on, set) in <(String, bool, NotificationSettings Function(bool))>[
              ('It asks leave to use a tool', config.notifications.permission,
                  (v) => config.notifications.copyWith(permission: v)),
              ('It has a question for you', config.notifications.question, (v) => config.notifications.copyWith(question: v)),
              ('It finished its turn', config.notifications.finished, (v) => config.notifications.copyWith(finished: v)),
            ])
              SwitchListTile(
                title: Text(label),
                value: on,
                onChanged: (v) => setState(() => config = config.copyWith(notifications: set(v))),
              ),
            if (widget.onAllowNotifications != null)
              ListTile(
                title: const Text('This browser'),
                subtitle: const Text('A browser shows them only once you allow it'),
                trailing: TextButton(
                  onPressed: () async {
                    final allowed = await widget.onAllowNotifications!();
                    if (!context.mounted) return;
                    ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(
                      content: Text(allowed ? 'This browser shows notifications from Canoa.' : 'This browser did not allow them.'),
                    ));
                  },
                  child: const Text('Allow notifications'),
                ),
              ),
          ],
        ),
        if (widget.presets.isNotEmpty || config.agents.isNotEmpty || widget.hosted.isNotEmpty) _sources(context),
        if (widget.rules != null) _permissions(context),
        ListTile(
          key: const Key('settings-default-mode'),
          title: const Text('Mode of a new session'),
          subtitle: const Text('Each session can change it from the composer, and keeps its own.'),
          trailing: DropdownButton<String?>(
            value: config.defaultMode,
            onChanged: (mode) => setState(() => config = mode == null ? config.copyWith(clearDefaultMode: true) : config.copyWith(defaultMode: mode)),
            items: [
              const DropdownMenuItem<String?>(value: null, child: Text("The agent's own")),
              for (final mode in modeOrder) DropdownMenuItem<String?>(value: mode, child: Text(modeLabels[mode] ?? mode)),
            ],
          ),
        ),
        SwitchListTile(
          key: const Key('settings-allow-once-only'),
          title: const Text('Bypass answers with allow once only'),
          subtitle: const Text('A request that offers only an "always" answer is then asked, not answered. Off, Bypass takes the "always" the agent offers.'),
          value: config.allowOnceOnly,
          onChanged: (v) => setState(() => config = config.copyWith(allowOnceOnly: v)),
        ),
        ListTile(
          title: const Text('Sessions remembered'),
          subtitle: const Text('The least recently used go first'),
          trailing: DropdownButton<int>(
            value: config.sessions.keep,
            items: [
              for (final n in {0, 50, 200, 1000, config.sessions.keep}.toList()..sort())
                DropdownMenuItem(value: n, child: Text(n == 0 ? 'None' : '$n')),
            ],
            onChanged: (value) {
              if (value != null) setState(() => config = config.copyWith(sessions: _sessions(keep: value)));
            },
          ),
        ),
        if (widget.workspacePreview != null) _workspaceRow(),
        SwitchListTile(
          title: const Text('Keep what was said with each session'),
          subtitle: const Text('Off: only the session\'s name and place are kept. Turning it off clears what was kept.'),
          value: config.sessions.keepText,
          onChanged: (v) => setState(() => config = config.copyWith(sessions: _sessions(keepText: v))),
        ),
        if (widget.onProfiles != null)
          ListTile(
            title: const Text('Profiles'),
            subtitle: const Text('What each agent starts with, and the layer each value comes from'),
            trailing: TextButton(
              key: const Key('settings-profiles'),
              onPressed: widget.onProfiles,
              child: const Text('Show…'),
            ),
          ),
        if (widget.ledger != null)
          ListenableBuilder(
            listenable: widget.ledger!,
            builder: (context, _) => ListTile(
              title: const Text('What was said, kept with sessions'),
              subtitle: Text(widget.ledger!.holdsText ? 'Some is kept' : 'None is kept'),
              trailing: TextButton(
                onPressed: widget.ledger!.holdsText ? widget.ledger!.clearText : null,
                child: const Text('Clear what was said'),
              ),
            ),
          ),
      ],
    ))),
    actions: [
      if (widget.onQuit != null)
        TextButton(onPressed: () => widget.onQuit!(), child: const Text('Quit Canoa')),
      TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
      FilledButton(
        onPressed: _workspaceProblem == null
            ? () {
                widget.rules?.useProtected(config.protectedPaths == null ? null : ProtectedPaths(config.protectedPaths!));
                Navigator.pop(context, config);
              }
            : null,
        child: const Text('Save'),
      ),
    ],
  );
}
