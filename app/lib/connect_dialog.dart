import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'package:coqui_acp/page.dart';

import 'acp/agent_commands.dart';
import 'acp/local_agent.dart';
import 'auth/browser_peer.dart';

/// Explicit, out-of-band connection configuration for a pinned peer.
/// No trust-on-first-use is performed from the connection dialog.
class ConnectTarget {
  const ConnectTarget({
    required this.webSocket,
    required this.webTransport,
    required this.peerId,
    required this.method,
    required this.cwd,
    required this.title,
  });
  final String webSocket;
  final String webTransport;
  final String peerId;

  /// The proof the host runs, one of [proofMethods], as `canoa-host serve` prints it.
  final String method;
  final String cwd;
  final String title;
}

/// The proofs a host can run. None is assumed: the person names the one the host prints.
const List<String> proofMethods = ['noise', 'webcrypto'];

/// Runs an agent on this computer, as a child of the app.
typedef RunLocalAgent = Future<void> Function(AgentCommand command, {required String cwd, required String title});

/// Uses the agents a person grants through the browser's provider, asking them in the provider's own window.
typedef UseBrowserAgents = Future<void> Function(AcpPage acp, {required String cwd, required String title});

Future<void> showCanoaConnectDialog(
  BuildContext context,
  Future<void> Function(ConnectTarget) onConnect, {
  Future<AcpPage?>? browserProvider,
  UseBrowserAgents? onUseBrowserAgents,
  RunLocalAgent? onRunLocalAgent,
  List<AgentCommand> localAgents = agentPresets,
  Future<bool> Function(AgentCommand) checkInstalled = isInstalled,
}) => showDialog<void>(
  context: context,
  builder: (_) => _ConnectDialog(
    onConnect: onConnect,
    browserProvider: browserProvider,
    onUseBrowserAgents: onUseBrowserAgents,
    onRunLocalAgent: onRunLocalAgent,
    localAgents: localAgents,
    checkInstalled: checkInstalled,
  ),
);

class _ConnectDialog extends StatefulWidget {
  const _ConnectDialog({
    required this.onConnect,
    this.browserProvider,
    this.onUseBrowserAgents,
    this.onRunLocalAgent,
    this.localAgents = agentPresets,
    this.checkInstalled = isInstalled,
  });
  final Future<bool> Function(AgentCommand) checkInstalled;
  final RunLocalAgent? onRunLocalAgent;
  final List<AgentCommand> localAgents;
  final Future<void> Function(ConnectTarget) onConnect;
  final Future<AcpPage?>? browserProvider;
  final UseBrowserAgents? onUseBrowserAgents;

  @override
  State<_ConnectDialog> createState() => _ConnectDialogState();
}

class _ConnectDialogState extends State<_ConnectDialog> {
  final ws = TextEditingController();
  final wt = TextEditingController();
  final pinned = TextEditingController();
  final cwd = TextEditingController(text: '/');
  final title = TextEditingController();
  final command = TextEditingController();
  late final Future<String> localPeerId = localBrowserPeerId();
  String? method;
  bool busy = false;
  String? error;

  @override
  void dispose() {
    ws.dispose();
    wt.dispose();
    pinned.dispose();
    cwd.dispose();
    title.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    if (busy) return;
    final peer = pinned.text.trim().toLowerCase();
    if (!RegExp(r'^sha256:[0-9a-f]{64}$').hasMatch(peer)) {
      setState(() => error = 'Enter the verified server peer ID (sha256:64 hex).');
      return;
    }
    final method = this.method;
    if (method == null) {
      setState(() => error = 'Choose the method the host proves by: canoa-host serve prints it.');
      return;
    }
    final request = ConnectTarget(
      webSocket: ws.text.trim(),
      webTransport: wt.text.trim(),
      peerId: peer,
      method: method,
      cwd: cwd.text.trim(),
      title: title.text.trim(),
    );
    setState(() {busy = true; error = null;});
    try {
      await widget.onConnect(request);
      if (mounted) Navigator.of(context).pop();
    } catch (e) {
      if (mounted) setState(() => error = 'Connection refused: $e');
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  late final Future<List<AgentCommand>> _installed = () async {
    final found = <AgentCommand>[];
    for (final command in widget.localAgents) {
      if (await widget.checkInstalled(command)) found.add(command);
    }
    return found;
  }();

  void _runTyped() {
    final parsed = parseCommand(command.text);
    if (parsed == null) {
      setState(() => error = 'That is not a command: check its quotes.');
      return;
    }
    _runLocal(parsed);
  }

  Future<void> _runLocal(AgentCommand command) async {
    if (busy) return;
    setState(() {busy = true; error = null;});
    try {
      await widget.onRunLocalAgent!(command, cwd: cwd.text.trim(), title: title.text.trim());
      if (mounted) Navigator.of(context).pop();
    } catch (e) {
      if (mounted) setState(() => error = '$e');
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  /// No await before the provider's request: it needs the click's user activation.
  Future<void> _useBrowser(AcpPage acp) async {
    if (busy) return;
    final asked = widget.onUseBrowserAgents!(acp, cwd: cwd.text.trim(), title: title.text.trim());
    setState(() {busy = true; error = null;});
    try {
      await asked;
      if (mounted) Navigator.of(context).pop();
    } catch (e) {
      if (mounted) setState(() => error = 'Connection refused: $e');
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('Connect to verified agent'),
      content: SizedBox(
        width: 440,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text(
                'The server must be explicitly paired with this browser. '
                'Only enter a fingerprint verified out of band. '
                'Canoa never trusts the first key it sees.',
              ),
              const SizedBox(height: 12),
              if (widget.onRunLocalAgent != null)
                FutureBuilder<List<AgentCommand>>(
                  future: _installed,
                  builder: (context, snapshot) {
                    final found = snapshot.data ?? const <AgentCommand>[];
                    if (found.isEmpty) return const SizedBox.shrink();
                    return Padding(
                      padding: const EdgeInsets.only(bottom: 12),
                      child: Wrap(
                        spacing: 8,
                        runSpacing: 8,
                        children: [
                          for (final command in found)
                            FilledButton.icon(
                              onPressed: busy ? null : () => _runLocal(command),
                              icon: const Icon(Icons.play_arrow_rounded),
                              label: Text('Run ${command.label}'),
                            ),
                        ],
                      ),
                    );
                  },
                ),
              if (widget.onRunLocalAgent != null)
                Padding(
                  padding: const EdgeInsets.only(bottom: 12),
                  child: TextField(
                    controller: command,
                    decoration: InputDecoration(
                      labelText: 'Or run any ACP agent, by its command',
                      suffixIcon: IconButton(
                        tooltip: 'Run this command',
                        icon: const Icon(Icons.play_arrow_rounded),
                        onPressed: busy ? null : _runTyped,
                      ),
                    ),
                    onSubmitted: (_) => _runTyped(),
                  ),
                ),
              if (widget.onUseBrowserAgents != null)
                FutureBuilder<AcpPage?>(
                  future: widget.browserProvider,
                  builder: (context, snapshot) {
                    final acp = snapshot.data;
                    if (acp == null) return const SizedBox.shrink();
                    return Padding(
                      padding: const EdgeInsets.only(bottom: 12),
                      child: FilledButton.icon(
                        onPressed: busy ? null : () => _useBrowser(acp),
                        icon: const Icon(Icons.extension_rounded),
                        label: const Text('Use an agent from this browser'),
                      ),
                    );
                  },
                ),
              FutureBuilder<String>(
                future: localPeerId,
                builder: (context, snapshot) {
                  if (!snapshot.hasData) {
                    return Text(snapshot.hasError
                        ? 'Browser identity unavailable: ${snapshot.error}'
                        : 'Loading your browser identity…');
                  }
                  final peer = snapshot.data!;
                  return Row(children: [
                    Expanded(child: SelectableText('Your browser peer ID:\n$peer',
                      style: Theme.of(context).textTheme.bodySmall)),
                    IconButton(
                      tooltip: 'Copy your peer ID for pairing',
                      icon: const Icon(Icons.copy),
                      onPressed: () => Clipboard.setData(ClipboardData(text: peer)),
                    ),
                  ]);
                },
              ),
              TextField(controller: title,
                decoration: const InputDecoration(labelText: 'Session title (optional)')),
              TextField(controller: wt,
                decoration: const InputDecoration(
                  labelText: 'WebTransport HTTPS endpoint (optional)',
                  hintText: 'https://agent.example:4433/acp',
                )),
              TextField(controller: ws,
                decoration: const InputDecoration(
                  labelText: 'WebSocket WSS endpoint',
                  hintText: 'wss://agent.example/acp',
                )),
              TextField(controller: pinned,
                decoration: const InputDecoration(
                  labelText: 'Pinned server peer ID',
                  hintText: 'sha256:…',
                )),
              const SizedBox(height: 12),
              Text('Method the host proves by', style: Theme.of(context).textTheme.bodySmall),
              const SizedBox(height: 4),
              SegmentedButton<String>(
                segments: [for (final name in proofMethods) ButtonSegment(value: name, label: Text(name))],
                selected: {?method},
                emptySelectionAllowed: true,
                showSelectedIcon: false,
                onSelectionChanged: (chosen) => setState(() => method = chosen.firstOrNull),
              ),
              TextField(controller: cwd,
                decoration: const InputDecoration(
                  labelText: 'Agent workspace (absolute path)',
                )),
              if (error != null)
                Padding(
                  padding: const EdgeInsets.only(top: 12),
                  child: Text(error!, style: TextStyle(
                    color: Theme.of(context).colorScheme.error,
                  )),
                ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: busy ? null : () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: busy ? null : _submit,
          child: busy
              ? const SizedBox(width: 18, height: 18,
                  child: CircularProgressIndicator(strokeWidth: 2))
              : const Text('Connect'),
        ),
      ],
    );
  }
}
