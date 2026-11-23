import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../shell/index_pane.dart';
import '../theme/look.dart';
import '../settings/shell_settings.dart' show HostShell;
import 'terminal_controller.dart';
import 'terminal_surface.dart';

/// A host a shell can be opened on from the Terminal section: this computer, the host that served the page, or a saved Canoa host.
final class TerminalTarget {
  const TerminalTarget({
    required this.id,
    required this.name,
    required this.kind,
    required this.access,
  });

  final String id;
  final String name;

  /// What the host is, as the Hosts section words it.
  final String kind;

  /// Asks the host what it offers: shells on it, or the reason it offers none. Asked once and kept until the person asks again.
  final Future<TerminalAccess> Function() access;
}

/// The shells opened in one folder of one host, as the Terminal surface keeps a conversation's: tabs, and a split in any of them.
final class TerminalGroup {
  TerminalGroup(this.target, this.cwd, this.workspace, {this.shell});

  final TerminalTarget target;

  /// The folder the shells start in; null for the host's default.
  final String? cwd;

  /// The id of the shell they run, one the host listed; null for the host's own choice.
  final String? shell;
  final TerminalWorkspace workspace;

  String get key => '${target.id}\u0000${cwd ?? ''}\u0000${shell ?? ''}';
}

/// The Terminal section's state: the shells the person opened with no conversation, grouped by host and folder, and what each host
/// said when asked for one. Shells belong to their host's link and end with the group or the app.
class TerminalSectionController extends ChangeNotifier {
  TerminalSectionController({
    required this.targets,
    this.workspaces = _none,
    this.selected = _nothing,
  });

  /// The hosts offered now, in the order the Hosts section lists them.
  final List<TerminalTarget> Function() targets;

  /// The folders the conversations on a host use, offered when a shell is opened.
  final List<String> Function(String targetId) workspaces;

  /// The host and folder of the conversation now selected, which a new shell starts in unless the person says otherwise; null when
  /// none is selected or its host offers no shell here.
  final ({String target, String folder})? Function() selected;

  static ({String target, String folder})? _nothing() => null;

  static List<String> _none(String _) => const [];

  final List<TerminalGroup> groups = [];
  final Map<String, TerminalAccess> _access = {};
  final Map<String, Future<TerminalAccess>> _asking = {};
  TerminalGroup? _shown;
  bool _disposed = false;

  /// Why the last New shell was refused, in the host's words; cleared by the next try that works.
  String? refusal;

  /// What each host said when asked, by target id: absent until it has been.
  TerminalAccess? accessOf(String targetId) => _access[targetId];

  bool isAsking(String targetId) => _asking.containsKey(targetId);

  TerminalGroup? get shown =>
      groups.contains(_shown) ? _shown : groups.firstOrNull;

  void show(TerminalGroup group, [TerminalTab? tab]) {
    if (!groups.contains(group)) return;
    _shown = group;
    refusal = null;
    if (tab != null) group.workspace.activate(tab);
    notifyListeners();
  }

  /// Asks every host now listed what it offers, once each, so the index says which will open a shell before one is tried.
  void askHosts() {
    for (final target in targets()) {
      if (!_access.containsKey(target.id)) unawaited(_ask(target));
    }
  }

  /// Forgets what [targetId] said and asks it again: its grants may have changed.
  Future<void> askAgain(String targetId) {
    final target = targets().where((each) => each.id == targetId).firstOrNull;
    _access.remove(targetId);
    return target == null ? Future.value() : _ask(target);
  }

  Future<TerminalAccess> _ask(TerminalTarget target) {
    final known = _access[target.id];
    if (known != null) return Future.value(known);
    final asking = _asking[target.id] ??= () async {
      await Future<void>.value();
      TerminalAccess answer;
      try {
        answer = await target.access();
      } on Object catch (error) {
        answer = TerminalRefused(
          '${target.name} could not be asked for a terminal: $error',
        );
      }
      _asking.remove(target.id);
      if (_disposed) return answer;
      _access[target.id] = answer;
      notifyListeners();
      return answer;
    }();
    // The question is under way once it is recorded: say so to what draws it.
    scheduleMicrotask(() => _disposed ? null : notifyListeners());
    return asking;
  }

  /// The shells [targetId]'s host lists for a new terminal to name; none when it offers no terminal or lists none.
  Future<List<HostShell>> shellsOf(String targetId) async {
    final target = targets().where((each) => each.id == targetId).firstOrNull;
    if (target == null) return const [];
    final access = await _ask(target);
    return access is TerminalOnHost ? access.host.shells() : const [];
  }

  /// A new shell on [targetId] in [cwd] (the host's default when null or empty) running [shell] (an id the host listed; its own
  /// choice when null): in the group for that triple, which is made when it is the first. A host that offers none leaves [refusal]
  /// and returns null.
  Future<TerminalGroup?> open(
    String targetId,
    String? cwd, {
    String? shell,
  }) async {
    final target = targets().where((each) => each.id == targetId).firstOrNull;
    if (target == null) {
      refusal = 'That host is not listed any more.';
      notifyListeners();
      return null;
    }
    final access = await _ask(target);
    if (_disposed) return null;
    if (access is! TerminalOnHost) {
      refusal = access is TerminalRefused
          ? access.reason
          : '${target.name} offers no terminal.';
      notifyListeners();
      return null;
    }
    final folder = cwd == null || cwd.trim().isEmpty ? null : cwd.trim();
    var group = groups
        .where(
          (each) =>
              each.target.id == targetId &&
              each.cwd == folder &&
              each.shell == shell,
        )
        .firstOrNull;
    if (group == null) {
      group = TerminalGroup(
        target,
        folder,
        TerminalWorkspace(host: access.host, cwd: folder, shell: shell),
        shell: shell,
      );
      group.workspace.addListener(() => _changed(group!));
      groups.add(group);
    }
    if (group.workspace.newTab() == null) {
      refusal =
          '${target.name} holds ${TerminalWorkspace.limit} shells in one folder at most here: close one first.';
      notifyListeners();
      return group;
    }
    refusal = null;
    _shown = group;
    notifyListeners();
    return group;
  }

  void _changed(TerminalGroup group) {
    if (group.workspace.tabs.isEmpty && groups.remove(group)) {
      // Not now: the workspace is still telling its listeners.
      scheduleMicrotask(group.workspace.dispose);
    }
    if (!_disposed) notifyListeners();
  }

  /// Ends every shell on [targetId]'s host and forgets what it said: the host was removed from the list, or is listed under
  /// another name now.
  Future<void> closeHost(String targetId) async {
    _access.remove(targetId);
    final gone = [
      for (final group in groups)
        if (group.target.id == targetId) group,
    ];
    groups.removeWhere(gone.contains);
    if (!_disposed) notifyListeners();
    await Future.wait([for (final group in gone) group.workspace.release()]);
  }

  /// Ends every shell: the app is closing.
  Future<void> release() => Future.wait([
    for (final group in [...groups]) group.workspace.release(),
  ]);

  /// Every notifier the index and the main view draw from.
  Listenable get changes => Listenable.merge([
    this,
    for (final group in groups) ...[
      group.workspace,
      for (final tab in group.workspace.tabs)
        for (final pane in tab.panes) pane,
    ],
  ]);

  @override
  void dispose() {
    _disposed = true;
    unawaited(release());
    super.dispose();
  }
}

String _stateWord(PaneState state) => switch (state) {
  PaneState.starting => 'starting',
  PaneState.running => 'running',
  PaneState.exited => 'exited',
  PaneState.failed => 'lost',
};

/// What a tab stands for in the index: running while any of its shells is, else the state its focused shell is in.
PaneState _tabState(TerminalTab tab) {
  for (final pane in tab.panes) {
    if (pane.state == PaneState.running) return PaneState.running;
  }
  return tab.focused.state;
}

/// The Terminal section's index: the hosts, and under each the folders and shells opened on it, running or exited.
class TerminalIndex extends StatefulWidget {
  const TerminalIndex({super.key, required this.controller, this.askFolder});

  final TerminalSectionController controller;

  /// Asks which host and folder a new shell is for; the section's own dialog when null.
  final Future<({String target, String? cwd, String? shell})?> Function(
    BuildContext context,
  )?
  askFolder;

  @override
  State<TerminalIndex> createState() => _TerminalIndexState();
}

class _TerminalIndexState extends State<TerminalIndex> {
  TerminalSectionController get controller => widget.controller;

  @override
  void initState() {
    super.initState();
    scheduleMicrotask(controller.askHosts);
  }

  Future<void> _new(BuildContext context) async {
    final choice =
        await (widget.askFolder?.call(context) ??
            showNewShellDialog(context, controller));
    if (choice != null) {
      await controller.open(choice.target, choice.cwd, shell: choice.shell);
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    // The shells come and go with the controller; each one's own state is listened to through what it holds now.
    listenable: controller,
    builder: (context, _) =>
        ListenableBuilder(listenable: controller.changes, builder: _list),
  );

  Widget _list(BuildContext context, Widget? _) {
    final targets = controller.targets();
    return IndexPane(
      title: 'Terminal',
      trailing: IconButton(
        key: const ValueKey('terminal-section new'),
        tooltip: 'New shell',
        icon: const Icon(Icons.add),
        onPressed: targets.isEmpty ? null : () => unawaited(_new(context)),
      ),
      body: targets.isEmpty
          ? const Padding(
              padding: EdgeInsets.all(18),
              child: Text('No host to open a shell on.'),
            )
          : ListView(
              children: [
                for (final target in targets)
                  _HostBlock(controller: controller, target: target),
              ],
            ),
    );
  }
}

class _HostBlock extends StatelessWidget {
  const _HostBlock({required this.controller, required this.target});

  final TerminalSectionController controller;
  final TerminalTarget target;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final access = controller.accessOf(target.id);
    final groups = [
      for (final group in controller.groups)
        if (group.target.id == target.id) group,
    ];
    final String? note = switch (access) {
      TerminalRefused(:final reason) => reason,
      null => controller.isAsking(target.id) ? 'Asking the host…' : null,
      TerminalOnHost() => null,
    };
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 12, 12, 2),
          child: Row(
            children: [
              Icon(
                access is TerminalRefused ? Icons.block : Icons.dns_outlined,
                size: 18,
                color: scheme.onSurfaceVariant,
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      target.name,
                      key: ValueKey('terminal host ${target.id}'),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(fontWeight: FontWeight.w600),
                    ),
                    Text(
                      target.kind,
                      style: Theme.of(context).textTheme.bodySmall,
                    ),
                  ],
                ),
              ),
              if (access is TerminalRefused)
                IconButton(
                  key: ValueKey('terminal ask again ${target.id}'),
                  tooltip: 'Ask ${target.name} again',
                  visualDensity: VisualDensity.compact,
                  iconSize: 16,
                  icon: const Icon(Icons.refresh),
                  onPressed: () => unawaited(controller.askAgain(target.id)),
                ),
            ],
          ),
        ),
        if (note != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(46, 0, 12, 6),
            child: Text(
              note,
              key: ValueKey('terminal host note ${target.id}'),
              style: Theme.of(context).textTheme.bodySmall?.copyWith(
                color: access is TerminalRefused
                    ? scheme.error
                    : scheme.onSurfaceVariant,
              ),
            ),
          ),
        for (final group in groups) ...[
          Padding(
            padding: const EdgeInsets.fromLTRB(46, 6, 12, 0),
            child: Text(
              group.cwd ?? 'The host\'s default folder',
              key: ValueKey('terminal folder ${group.key}'),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: Theme.of(
                context,
              ).textTheme.labelMedium?.copyWith(color: scheme.onSurfaceVariant),
            ),
          ),
          for (final tab in group.workspace.tabs)
            _TabRow(controller: controller, group: group, tab: tab),
        ],
      ],
    );
  }
}

class _TabRow extends StatelessWidget {
  const _TabRow({
    required this.controller,
    required this.group,
    required this.tab,
  });

  final TerminalSectionController controller;
  final TerminalGroup group;
  final TerminalTab tab;

  @override
  Widget build(BuildContext context) {
    final state = _tabState(tab);
    final scheme = Theme.of(context).colorScheme;
    final shown =
        identical(controller.shown, group) &&
        identical(group.workspace.active, tab);
    final color = switch (state) {
      PaneState.running => context.look.success,
      PaneState.starting => scheme.outline,
      PaneState.exited => scheme.outline,
      PaneState.failed => scheme.error,
    };
    return Semantics(
      selected: shown,
      child: Material(
        color: shown ? context.look.selected : Colors.transparent,
        child: InkWell(
          key: ValueKey('terminal shell ${group.key} ${tab.id}'),
          onTap: () => controller.show(group, tab),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(46, 6, 4, 6),
            child: Row(
              children: [
                Icon(Icons.circle, size: 9, color: color),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    tab.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                Text(
                  _stateWord(state),
                  key: ValueKey('terminal state ${group.key} ${tab.id}'),
                  style: Theme.of(
                    context,
                  ).textTheme.bodySmall?.copyWith(color: color),
                ),
                IconButton(
                  key: ValueKey(
                    'terminal section close ${group.key} ${tab.id}',
                  ),
                  tooltip: 'Close ${tab.name}',
                  visualDensity: VisualDensity.compact,
                  iconSize: 14,
                  icon: const Icon(Icons.close),
                  onPressed: () => group.workspace.closeTab(tab),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// The Terminal section's main view: the shells of the group chosen in the index, in tabs, and what to do when there are none.
class TerminalMain extends StatelessWidget {
  const TerminalMain({super.key, required this.controller, this.onNewShell});

  final TerminalSectionController controller;

  /// Opens the New shell dialog; the section's own when null.
  final Future<void> Function(BuildContext context)? onNewShell;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) {
      final group = controller.shown;
      if (group == null) {
        return Center(
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                const Icon(Icons.terminal, size: 36),
                const SizedBox(height: 12),
                const Text(
                  'A shell on any of your hosts, with no conversation open.',
                ),
                const SizedBox(height: 12),
                FilledButton.icon(
                  key: const ValueKey('terminal-section start'),
                  onPressed: () =>
                      unawaited((onNewShell ?? _askAndOpen)(context)),
                  icon: const Icon(Icons.add),
                  label: const Text('New shell'),
                ),
                if (controller.refusal case final why?)
                  Padding(
                    padding: const EdgeInsets.only(top: 12),
                    child: Text(
                      why,
                      key: const ValueKey('terminal-section refusal'),
                      textAlign: TextAlign.center,
                      style: TextStyle(
                        color: Theme.of(context).colorScheme.error,
                      ),
                    ),
                  ),
              ],
            ),
          ),
        );
      }
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Material(
            color: context.look.bar,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 6),
              child: Text(
                '${group.target.name}  ·  ${group.cwd ?? 'the host\'s default folder'}',
                key: const ValueKey('terminal-section title'),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: Theme.of(context).textTheme.bodySmall,
              ),
            ),
          ),
          if (controller.refusal case final why?)
            Container(
              key: const ValueKey('terminal-section refusal'),
              color: Theme.of(context).colorScheme.errorContainer,
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 6),
              child: Text(
                why,
                style: TextStyle(
                  color: Theme.of(context).colorScheme.onErrorContainer,
                ),
              ),
            ),
          Expanded(
            child: TerminalPanel(
              key: ObjectKey(group),
              workspace: group.workspace,
              focusOnMount: true,
            ),
          ),
        ],
      );
    },
  );

  Future<void> _askAndOpen(BuildContext context) async {
    final choice = await showNewShellDialog(context, controller);
    if (choice != null) {
      await controller.open(choice.target, choice.cwd, shell: choice.shell);
    }
  }
}

/// Asks which host the shell is on, then which folder (one the host's conversations use, or one typed, or the host's default) and
/// which of the shells the host found it runs (its own choice by default).
Future<({String target, String? cwd, String? shell})?> showNewShellDialog(
  BuildContext context,
  TerminalSectionController controller,
) => showDialog<({String target, String? cwd, String? shell})>(
  context: context,
  builder: (context) => _NewShellDialog(controller: controller),
);

class _NewShellDialog extends StatefulWidget {
  const _NewShellDialog({required this.controller});

  final TerminalSectionController controller;

  @override
  State<_NewShellDialog> createState() => _NewShellDialogState();
}

class _NewShellDialogState extends State<_NewShellDialog> {
  late final ({String target, String folder})? _selected = () {
    final chosen = widget.controller.selected();
    return chosen != null &&
            widget.controller.targets().any((each) => each.id == chosen.target)
        ? chosen
        : null;
  }();
  late String _target =
      _selected?.target ?? widget.controller.targets().first.id;
  late final TextEditingController _folder = TextEditingController(
    text: _selected?.folder ?? '',
  );

  /// The id of the shell chosen, null for the host's own, and what the host listed for the host now chosen.
  String? _shell;
  late Future<List<HostShell>> _shells = widget.controller.shellsOf(_target);

  @override
  void dispose() {
    _folder.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final targets = widget.controller.targets();
    final known = widget.controller.workspaces(_target);
    return AlertDialog(
      title: const Text('New shell'),
      content: SizedBox(
        width: 420,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            DropdownButtonFormField<String>(
              key: const ValueKey('terminal-section host'),
              initialValue: _target,
              decoration: const InputDecoration(labelText: 'Host'),
              items: [
                for (final target in targets)
                  DropdownMenuItem(value: target.id, child: Text(target.name)),
              ],
              onChanged: (value) => setState(() {
                _target = value ?? _target;
                _shell = null;
                _shells = widget.controller.shellsOf(_target);
                // The conversation's folder belongs to its own host only.
                if (_selected case final chosen?) {
                  if (_target == chosen.target) {
                    if (_folder.text.isEmpty) _folder.text = chosen.folder;
                  } else if (_folder.text == chosen.folder) {
                    _folder.clear();
                  }
                }
              }),
            ),
            const SizedBox(height: 12),
            FutureBuilder<List<HostShell>>(
              future: _shells,
              builder: (context, snapshot) {
                final shells = snapshot.data ?? const <HostShell>[];
                if (shells.isEmpty) return const SizedBox.shrink();
                return Padding(
                  padding: const EdgeInsets.only(bottom: 12),
                  child: DropdownButtonFormField<String?>(
                    key: const ValueKey('terminal-section shell'),
                    initialValue: _shell,
                    isExpanded: true,
                    decoration: const InputDecoration(labelText: 'Shell'),
                    items: [
                      const DropdownMenuItem<String?>(
                        value: null,
                        child: Text('The host\'s own choice'),
                      ),
                      for (final shell in shells)
                        DropdownMenuItem<String?>(
                          value: shell.id,
                          child: Text(
                            shell.label,
                            overflow: TextOverflow.ellipsis,
                          ),
                        ),
                    ],
                    onChanged: (value) => setState(() => _shell = value),
                  ),
                );
              },
            ),
            TextField(
              key: const ValueKey('terminal-section folder'),
              controller: _folder,
              decoration: const InputDecoration(
                labelText: 'Folder',
                hintText: 'Empty for the host\'s default (its user\'s home)',
              ),
            ),
            if (known.isNotEmpty) ...[
              const SizedBox(height: 8),
              Wrap(
                spacing: 6,
                runSpacing: 4,
                children: [
                  for (final folder in known)
                    ActionChip(
                      label: Text(folder),
                      onPressed: () => setState(() => _folder.text = folder),
                    ),
                ],
              ),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const ValueKey('terminal-section open'),
          onPressed: () => Navigator.pop(context, (
            target: _target,
            cwd: _folder.text.trim().isEmpty ? null : _folder.text.trim(),
            shell: _shell,
          )),
          child: const Text('Open'),
        ),
      ],
    );
  }
}
