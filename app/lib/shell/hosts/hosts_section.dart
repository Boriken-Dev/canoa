import 'dart:async';

import 'package:coqui_acp/page.dart' show AcpPage;
import 'package:flutter/material.dart';

import '../../config/connection.dart';
import '../../connect_dialog.dart' show showAddConnectionDialog;
import '../../selectable.dart';
import '../../theme/look.dart';
import 'host_status.dart';
import 'hosts_controller.dart';
import 'start_session_dialog.dart';

Color _stateColor(BuildContext context, HostState state) => switch (state) {
  HostState.online => context.look.success,
  HostState.offline || HostState.changed || HostState.refused => Theme.of(context).colorScheme.error,
  HostState.checking || HostState.unchecked => Theme.of(context).colorScheme.outline,
};

IconData _iconOf(HostItem item) => switch (item.kind) {
  HostItemKind.local => Icons.computer_outlined,
  HostItemKind.served => Icons.dns_outlined,
  HostItemKind.saved => item.connection!.kind == ConnectionKind.canoa ? Icons.lan_outlined : Icons.link_rounded,
};

Future<void> _openAdd(BuildContext context, HostsController controller) =>
    controller.addDialog?.call(context) ??
    showAddConnectionDialog(context, onAdd: controller.add, nearby: controller.nearby);

/// The Hosts section's index: every execution target the person can use. Local first, the host that served this page, then the
/// connections saved, each with its kind and where it stands, and Add.
class HostsIndex extends StatefulWidget {
  const HostsIndex({super.key, required this.controller});

  final HostsController controller;

  @override
  State<HostsIndex> createState() => _HostsIndexState();
}

class _HostsIndexState extends State<HostsIndex> {
  HostsController get controller => widget.controller;

  @override
  void initState() {
    super.initState();
    // A Canoa host is read from the list it serves the first time the section is shown; nothing is connected to.
    scheduleMicrotask(controller.lookAtHosts);
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) {
      final items = controller.items;
      final selected = controller.selected;
      return Material(
        color: context.look.rail,
        child: Column(children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(18, 14, 8, 6),
            child: Row(children: [
              Expanded(child: Text('Hosts', style: Theme.of(context).textTheme.titleLarge)),
              IconButton(
                key: const ValueKey('hosts-add'),
                tooltip: 'Add connection',
                icon: const Icon(Icons.add),
                onPressed: controller.canSave ? () => unawaited(_openAdd(context, controller)) : null,
              ),
            ]),
          ),
          const Divider(height: 1),
          Expanded(
            child: items.isEmpty
                ? const Padding(padding: EdgeInsets.all(18), child: Text('No host yet. Add a connection to a Canoa host or an agent elsewhere.'))
                : ListView(children: [
                    for (final item in items) _Row(controller: controller, item: item, selected: item.id == selected?.id),
                  ]),
          ),
          if (controller.browserProvider != null && controller.useBrowserAgents != null) _BrowserAgents(controller: controller),
        ]),
      );
    },
  );
}

class _Row extends StatelessWidget {
  const _Row({required this.controller, required this.item, required this.selected});

  final HostsController controller;
  final HostItem item;
  final bool selected;

  @override
  Widget build(BuildContext context) {
    final status = item.kind == HostItemKind.saved ? controller.statusOf(item) : null;
    final scheme = Theme.of(context).colorScheme;
    return Semantics(
      selected: selected,
      child: Material(
        color: selected ? context.look.selected : Colors.transparent,
        child: InkWell(
          key: ValueKey('host-row ${item.id}'),
          onTap: () => controller.select(item.id),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 10),
            child: Row(children: [
              Icon(_iconOf(item), size: 20, color: scheme.onSurfaceVariant),
              const SizedBox(width: 12),
              Expanded(
                child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                  Text(item.name, maxLines: 1, overflow: TextOverflow.ellipsis),
                  Text(item.kindLabel, maxLines: 1, overflow: TextOverflow.ellipsis, style: Theme.of(context).textTheme.bodySmall),
                ]),
              ),
              if (status != null) Tooltip(
                message: status.state.label,
                child: Icon(Icons.circle, size: 10, color: _stateColor(context, status.state)),
              ),
            ]),
          ),
        ),
      ),
    );
  }
}

/// Opens the agents the browser's provider holds, as the extension's panel did from the Connect dialog.
class _BrowserAgents extends StatelessWidget {
  const _BrowserAgents({required this.controller});

  final HostsController controller;

  Future<void> _use(BuildContext context, AcpPage acp) async {
    final asked = await showSelectableDialog<String>(context: context, builder: (_) => const _WorkspaceDialog());
    if (asked == null || asked.isEmpty || !context.mounted) return;
    try {
      await controller.useBrowserAgents!(acp, cwd: asked, title: '');
    } on Object catch (error) {
      if (context.mounted) ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text('Connection refused: $error')));
    }
  }

  @override
  Widget build(BuildContext context) => FutureBuilder(
    future: controller.browserProvider,
    builder: (context, snapshot) {
      final acp = snapshot.data;
      if (acp == null) return const SizedBox.shrink();
      return Padding(
        padding: const EdgeInsets.all(12),
        child: OutlinedButton.icon(
          key: const ValueKey('hosts-browser-agents'),
          onPressed: () => unawaited(_use(context, acp)),
          icon: const Icon(Icons.extension_rounded),
          label: const Text('Use an agent from this browser'),
        ),
      );
    },
  );
}

/// The chosen host's overview: where it stands, its identity and the trust in it, the agents it serves and what it says it can do.
/// Local also holds Hosting, which is what the host does for browsers.
class HostsOverview extends StatelessWidget {
  const HostsOverview({super.key, required this.controller, this.localDetail, this.servedDetail});

  final HostsController controller;

  /// What Local's overview adds under its facts: Hosting.
  final WidgetBuilder? localDetail;

  /// What the overview of the host that served this page adds, when this key may host: the same Hosting, on that host.
  final WidgetBuilder? servedDetail;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) {
      final item = controller.selected;
      return Align(
        alignment: Alignment.topCenter,
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 760),
          child: item == null
              ? Center(
                  child: Column(mainAxisSize: MainAxisSize.min, children: [
                    const Text('Nothing to show: no host is listed.', key: ValueKey('host-empty')),
                    const SizedBox(height: 12),
                    if (controller.canSave)
                      FilledButton(onPressed: () => unawaited(_openAdd(context, controller)), child: const Text('Add connection')),
                  ]),
                )
              : ListView(
                  key: ValueKey('host-overview ${item.id}'),
                  padding: const EdgeInsets.fromLTRB(16, 16, 16, 32),
                  children: [
                    ...switch (item.kind) {
                      HostItemKind.saved => _saved(context, item.connection!),
                      HostItemKind.local => _own(context, item, controller.local!(), 'This computer: the machine Canoa UI runs on, and the host inside it.'),
                      HostItemKind.served => _own(
                        context,
                        item,
                        controller.servedSummary?.call() ?? const HostSummary(state: 'Serving this page'),
                        'The host that served this page: this page reaches everything through it, and it is not Local.',
                      ),
                    },
                    if (switch (item.kind) {
                      HostItemKind.local => localDetail,
                      HostItemKind.served => servedDetail,
                      HostItemKind.saved => null,
                    }
                        case final detail?) ...[
                      const SizedBox(height: 24),
                      Text('Hosting', style: Theme.of(context).textTheme.titleMedium),
                      const SizedBox(height: 8),
                      detail(context),
                    ],
                  ],
                ),
        ),
      );
    },
  );

  Widget _title(BuildContext context, HostItem item, {Widget? trailing}) => Row(children: [
    Icon(_iconOf(item), size: 28),
    const SizedBox(width: 12),
    Expanded(child: Text(item.name, style: Theme.of(context).textTheme.headlineSmall)),
    ?trailing,
  ]);

  Widget _block(BuildContext context, String title, List<Widget> children, {Key? key}) => Padding(
    key: key,
    padding: const EdgeInsets.only(top: 20),
    child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      Text(title, style: Theme.of(context).textTheme.titleSmall),
      const SizedBox(height: 6),
      ...children,
    ]),
  );

  Widget _list(BuildContext context, List<String> names, String none, {Key? key}) => names.isEmpty
      ? Text(none, key: key)
      : Column(key: key, crossAxisAlignment: CrossAxisAlignment.start, children: [
          for (final name in names) Padding(padding: const EdgeInsets.symmetric(vertical: 2), child: Text(name)),
        ]);

  Widget _id(BuildContext context, String? peer, String? method) => peer == null
      ? const Text('Not known yet.')
      : SelectableText('$peer${method == null || method.isEmpty ? '' : '\nProves by $method'}', style: const TextStyle(fontFamily: 'monospace'));

  List<Widget> _own(BuildContext context, HostItem item, HostSummary summary, String blurb) => [
    _title(context, item, trailing: Chip(label: Text(summary.state), key: const ValueKey('host-state'))),
    const SizedBox(height: 4),
    Text(blurb),
    if (summary.problem != null) Padding(padding: const EdgeInsets.only(top: 8), child: Text(summary.problem!)),
    _block(context, 'Identity and trust', [
      _id(context, summary.peer, summary.method),
      const SizedBox(height: 4),
      const Text('This is the id browsers and other machines pin. Keys are paired and removed under Hosting.'),
    ], key: const ValueKey('host-identity')),
    _block(context, 'Served agents', [_list(context, summary.agents, 'It serves none.', key: const ValueKey('host-agents'))]),
    _block(context, 'Capabilities', [
      _list(context, summary.capabilities, 'None reported: the host has not said what this app may use.', key: const ValueKey('host-capabilities')),
    ]),
  ];

  List<Widget> _saved(BuildContext context, Connection connection) {
    final item = HostItem.saved(connection);
    final status = controller.statusOf(item);
    final host = connection.kind == ConnectionKind.canoa;
    final canStart = controller.start != null;
    return [
      _title(context, item, trailing: Chip(
        key: const ValueKey('host-state'),
        avatar: Icon(Icons.circle, size: 10, color: _stateColor(context, status.state)),
        label: Text(status.state.label),
      )),
      const SizedBox(height: 4),
      Text('${connection.kind.label} at ${connection.address}'),
      if (status.detail != null) Padding(padding: const EdgeInsets.only(top: 6), child: Text(status.detail!, key: const ValueKey('host-detail'))),
      const SizedBox(height: 12),
      Wrap(spacing: 8, runSpacing: 8, children: [
        FilledButton.icon(
          key: const ValueKey('host-new-session'),
          onPressed: canStart ? () => unawaited(_start(context, connection, status)) : null,
          icon: const Icon(Icons.add_comment_outlined),
          label: const Text('New session'),
        ),
        OutlinedButton.icon(
          key: const ValueKey('host-reconnect'),
          onPressed: status.state == HostState.checking ? null : () => unawaited(controller.check(connection)),
          icon: const Icon(Icons.refresh),
          label: Text(status.state == HostState.unchecked && !host ? 'Check' : 'Reconnect'),
        ),
        OutlinedButton(
          key: const ValueKey('host-rename'),
          onPressed: controller.canSave ? () => unawaited(_rename(context, connection)) : null,
          child: const Text('Rename'),
        ),
        OutlinedButton(
          key: const ValueKey('host-remove'),
          onPressed: controller.canSave ? () => unawaited(_remove(context, connection)) : null,
          child: const Text('Remove'),
        ),
      ]),
      _block(context, 'Identity and trust', [
        _id(context, connection.peer, connection.method),
        const SizedBox(height: 4),
        Text(
          status.state == HostState.changed
              ? 'It says it holds ${status.peer}: not the id pinned here, so this app will not connect.'
              : 'Pinned. This app connects only when the other end proves it holds this id; what a host lists of itself is its claim.',
        ),
      ], key: const ValueKey('host-identity')),
      _block(context, 'Served agents', [
        host
            ? _list(
                context,
                [for (final agent in status.agents) agent.name],
                status.state == HostState.online ? 'It lists no agent.' : 'Read when the host answers.',
                key: const ValueKey('host-agents'),
              )
            : const Text('A direct endpoint is the agent: it lists none.', key: ValueKey('host-agents')),
      ]),
      _block(context, 'Capabilities', [
        host
            ? _list(
                context,
                status.capabilities,
                status.state == HostState.online ? 'None reported.' : 'Read from the host when it answers; nothing is assumed.',
                key: const ValueKey('host-capabilities'),
              )
            : const Text(
                'A session and nothing else: no folder browsing, files or management. Type the workspace as a path on that machine.',
                key: ValueKey('host-capabilities'),
              ),
      ]),
    ];
  }

  Future<void> _start(BuildContext context, Connection connection, HostStatus status) async {
    final target = await showStartSessionDialog(context, connection: connection, status: status);
    if (target == null || !context.mounted) return;
    try {
      await controller.start!(target);
    } on Object catch (error) {
      if (context.mounted) ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text('Connection refused: $error')));
    }
  }

  Future<void> _rename(BuildContext context, Connection connection) =>
      showSelectableDialog<void>(context: context, builder: (_) => _RenameDialog(controller: controller, connection: connection));

  Future<void> _remove(BuildContext context, Connection connection) async {
    final affected = controller.sessionsOn(connection);
    final sure = await showSelectableDialog<bool>(
      context: context,
      builder: (dialog) => AlertDialog(
        title: Text('Remove ${connection.name}?'),
        content: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.start, children: [
          const Text('Only this app\'s record of it goes. Nothing is stopped on the other end, and its key stays paired there.'),
          const SizedBox(height: 12),
          if (affected.isEmpty)
            const Text('No session of yours is on it.', key: ValueKey('remove-sessions'))
          else
            Column(key: const ValueKey('remove-sessions'), crossAxisAlignment: CrossAxisAlignment.start, children: [
              Text('${affected.length == 1 ? 'This session is' : 'These ${affected.length} sessions are'} on it and cannot continue once it is gone:'),
              for (final session in affected) Padding(padding: const EdgeInsets.only(top: 4), child: Text(session.title)),
            ]),
        ]),
        actions: [
          TextButton(onPressed: () => Navigator.of(dialog).pop(false), child: const Text('Cancel')),
          FilledButton(key: const ValueKey('remove-confirm'), onPressed: () => Navigator.of(dialog).pop(true), child: const Text('Remove')),
        ],
      ),
    );
    if (sure == true) await controller.remove(connection);
  }
}

/// Asks for the workspace the browser's agents work in.
class _WorkspaceDialog extends StatefulWidget {
  const _WorkspaceDialog();

  @override
  State<_WorkspaceDialog> createState() => _WorkspaceDialogState();
}

class _WorkspaceDialogState extends State<_WorkspaceDialog> {
  final cwd = TextEditingController(text: '/');

  @override
  void dispose() {
    cwd.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Use an agent from this browser'),
    content: TextField(controller: cwd, decoration: const InputDecoration(labelText: 'Workspace (absolute path)')),
    actions: [
      TextButton(onPressed: () => Navigator.of(context).pop(), child: const Text('Cancel')),
      FilledButton(onPressed: () => Navigator.of(context).pop(cwd.text.trim()), child: const Text('Use')),
    ],
  );
}

/// Renames a saved connection, saying why a name cannot be taken.
class _RenameDialog extends StatefulWidget {
  const _RenameDialog({required this.controller, required this.connection});

  final HostsController controller;
  final Connection connection;

  @override
  State<_RenameDialog> createState() => _RenameDialogState();
}

class _RenameDialogState extends State<_RenameDialog> {
  late final field = TextEditingController(text: widget.connection.name);
  String? problem;

  @override
  void dispose() {
    field.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    try {
      await widget.controller.rename(widget.connection, field.text.trim());
      if (mounted) Navigator.of(context).pop();
    } on FormatException catch (error) {
      if (mounted) setState(() => problem = error.message);
    } on Object catch (error) {
      if (mounted) setState(() => problem = '$error');
    }
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Rename connection'),
    content: Column(mainAxisSize: MainAxisSize.min, children: [
      TextField(key: const ValueKey('rename-field'), controller: field, autofocus: true, decoration: const InputDecoration(labelText: 'Name')),
      if (problem != null) Padding(padding: const EdgeInsets.only(top: 8), child: Text(problem!, key: const ValueKey('rename-problem'))),
    ]),
    actions: [
      TextButton(onPressed: () => Navigator.of(context).pop(), child: const Text('Cancel')),
      FilledButton(key: const ValueKey('rename-save'), onPressed: _save, child: const Text('Rename')),
    ],
  );
}
