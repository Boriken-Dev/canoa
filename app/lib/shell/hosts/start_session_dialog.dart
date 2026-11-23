import 'package:material_ui/material_ui.dart';

import '../../auth/peer_proof.dart' show PeerMethod;
import '../../config/connection.dart';
import '../../connect_dialog.dart' show ConnectTarget;
import '../../selectable.dart';
import 'host_status.dart';

/// Asks which agent, workspace and title a new session on [connection] starts with, and answers the target to connect to.
///
/// A Canoa host offers the agents it lists, and may make a worktree for the session; a direct agent is the one agent, with no
/// worktree, since nothing there can make one. The workspace is a path on that machine, typed: browsing it needs the host's own
/// listing, which a direct agent does not offer.
Future<ConnectTarget?> showStartSessionDialog(BuildContext context, {required Connection connection, required HostStatus status}) =>
    showSelectableDialog<ConnectTarget>(
      context: context,
      builder: (_) => _StartSessionDialog(connection: connection, status: status),
    );

class _StartSessionDialog extends StatefulWidget {
  const _StartSessionDialog({required this.connection, required this.status});

  final Connection connection;
  final HostStatus status;

  @override
  State<_StartSessionDialog> createState() => _StartSessionDialogState();
}

class _StartSessionDialogState extends State<_StartSessionDialog> {
  final workspace = TextEditingController(text: '/');
  final title = TextEditingController();
  bool worktree = false;
  HostAgent? agent;
  String? error;

  bool get _host => widget.connection.kind == ConnectionKind.canoa;

  @override
  void initState() {
    super.initState();
    agent = widget.status.agents.firstOrNull;
  }

  @override
  void dispose() {
    workspace.dispose();
    title.dispose();
    super.dispose();
  }

  void _start() {
    final connection = widget.connection;
    final path = workspace.text.trim();
    if (path.isEmpty) {
      setState(() => error = 'Enter the workspace the agent works in, a path on that machine.');
      return;
    }
    final String url;
    if (_host) {
      final chosen = agent;
      if (chosen == null) {
        setState(() => error = 'This host lists no agent yet: check it, or hit Reconnect.');
        return;
      }
      url = chosen.url;
    } else {
      url = connection.address;
    }
    final viaWebTransport = !_host && connection.transport == ConnectionTransport.webtransport;
    Navigator.of(context).pop(ConnectTarget(
      webSocket: viaWebTransport ? '' : url,
      webTransport: viaWebTransport ? url : '',
      peerId: connection.peer,
      cwd: path,
      title: title.text.trim(),
      method: PeerMethod.parse(connection.method),
      worktree: _host && worktree,
    ));
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text('New session on ${widget.connection.name}'),
    content: SizedBox(
      width: 420,
      child: SingleChildScrollView(
        child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.start, children: [
          if (_host) ...[
            if (widget.status.agents.isEmpty)
              const Text('This host has listed no agent.', key: ValueKey('start-no-agents'))
            else
              DropdownButtonFormField<HostAgent>(
                key: const ValueKey('start-agent'),
                initialValue: agent,
                decoration: const InputDecoration(labelText: 'Agent'),
                items: [for (final each in widget.status.agents) DropdownMenuItem(value: each, child: Text(each.name))],
                onChanged: (chosen) => setState(() => agent = chosen),
              ),
          ],
          TextField(
            key: const ValueKey('start-workspace'),
            controller: workspace,
            decoration: const InputDecoration(labelText: 'Workspace (absolute path on that machine)'),
          ),
          TextField(
            key: const ValueKey('start-title'),
            controller: title,
            decoration: const InputDecoration(labelText: 'Session title (optional)'),
          ),
          if (_host)
            CheckboxListTile(
              key: const ValueKey('worktree'),
              contentPadding: EdgeInsets.zero,
              dense: true,
              controlAffinity: ListTileControlAffinity.leading,
              value: worktree,
              onChanged: (on) => setState(() => worktree = on ?? false),
              title: const Text('In a new git worktree, on its own branch'),
              subtitle: const Text('Of the repository at the workspace. Closing the session asks whether to keep it.'),
            ),
          if (error != null)
            Padding(
              padding: const EdgeInsets.only(top: 12),
              child: Text(error!, key: const ValueKey('start-error'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
            ),
        ]),
      ),
    ),
    actions: [
      TextButton(onPressed: () => Navigator.of(context).pop(), child: const Text('Cancel')),
      FilledButton(key: const ValueKey('start-session'), onPressed: _start, child: const Text('Start')),
    ],
  );
}
