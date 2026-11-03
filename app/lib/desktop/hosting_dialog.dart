import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../acp/agent_commands.dart';
import '../config/canoa_config.dart';
import '../theme/look.dart';
import 'host_supervisor.dart';
import 'host_tool.dart';

/// Checks and saves a changed Canoa.cfg, then serves it: what is wrong with it, or null once it is saved.
typedef SaveHosting = Future<String?> Function(CanoaConfig changed);

/// What this computer hosts for browsers: the host's state, its agents, adding one, and the keys that may connect, each
/// paired or removed only by the person's own click on a dialog that shows the whole id.
class HostingDialog extends StatefulWidget {
  const HostingDialog({
    super.key,
    required this.supervisor,
    required this.config,
    required this.onSave,
    this.freePort = freeLoopbackPort,
  });

  final HostSupervisor supervisor;
  final CanoaConfig config;
  final SaveHosting onSave;
  final Future<int> Function() freePort;

  @override
  State<HostingDialog> createState() => _HostingDialogState();
}

class _HostingDialogState extends State<HostingDialog> {
  late CanoaConfig config = widget.config;
  late Future<List<HostKey>> keys = _keys();
  HostPairing? pairing;
  String? paired;
  String? message;
  bool busy = false;

  Future<List<HostKey>> _keys() => widget.supervisor.tool == null ? Future.value(const []) : widget.supervisor.keys();

  void _refresh() => setState(() {
    keys = _keys();
  });

  @override
  void dispose() {
    pairing?.cancel();
    super.dispose();
  }

  Future<void> _run(Future<void> Function() step) async {
    if (busy) return;
    setState(() {
      busy = true;
      message = null;
    });
    try {
      await step();
    } on Object catch (error) {
      if (mounted) setState(() => message = '$error');
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Future<void> _save(CanoaConfig changed) async {
    final problem = await widget.onSave(changed);
    if (problem != null) throw HostCommandFailed(problem);
    if (mounted) setState(() => config = changed);
  }

  Future<void> _add() async {
    final added = await showDialog<_NewAgent>(context: context, builder: (_) => const _AddAgentDialog());
    if (added == null) return;
    await _run(() async {
      final port = await widget.freePort();
      await _save(
        config.withHostAgent(
          name: added.name,
          program: added.command.program,
          args: added.command.args,
          listen: '127.0.0.1:$port',
          workingRoots: [if (added.root.isNotEmpty) added.root],
        ),
      );
    });
  }

  Future<void> _remove(HostEntry entry) async {
    final sure = await _confirm(
      title: 'Stop hosting ${entry.name}?',
      body: [Text('Browsers can no longer reach ${entry.name} through this computer, and a link to it now ends.')],
      action: 'Stop hosting',
    );
    if (sure) await _run(() => _save(config.withoutHostEntry(entry.name)));
  }

  Future<void> _accept(HostKey key) async {
    final sure = await _confirm(
      title: 'Pair this key?',
      body: [
        const Text(
          'Pair it only if it is exactly the id Canoa shows where you connected from, character for character. '
          'Anyone on this computer can make a key wait here.',
        ),
        const SizedBox(height: 12),
        _Id(key.id),
        if (key.from != null)
          Padding(padding: const EdgeInsets.only(top: 8), child: Text('It connected from ${key.from}.')),
      ],
      action: 'Pair',
    );
    if (sure) {
      await _run(() => widget.supervisor.accept(key.id));
      _refresh();
    }
  }

  Future<void> _revoke(HostKey key) async {
    final sure = await _confirm(
      title: key.accepted ? 'Remove this key?' : 'Turn this key away?',
      body: [
        _Id(key.id),
        const SizedBox(height: 12),
        Text(key.accepted ? 'Its links end within a second, and it has to be paired again.' : 'It stops waiting here.'),
      ],
      action: 'Remove',
    );
    if (sure) {
      await _run(() => widget.supervisor.revoke(key.id));
      _refresh();
    }
  }

  Future<void> _showPin() async {
    HostPairing? started;
    await _run(() async {
      pairing?.cancel();
      started = await widget.supervisor.pair();
    });
    final running = started;
    if (running == null) return;
    if (!mounted) return running.cancel();
    setState(() {
      pairing = running;
      paired = null;
    });
    try {
      final who = await running.paired;
      if (!mounted) return;
      setState(() {
        paired = who;
        pairing = null;
      });
      _refresh();
    } on HostCommandFailed catch (error) {
      if (mounted && pairing == running) {
        setState(() {
          pairing = null;
          message = 'Pairing ended: $error';
        });
      }
    }
  }

  void _cancelPin() {
    final running = pairing;
    setState(() => pairing = null);
    running?.cancel();
  }

  Future<bool> _confirm({required String title, required List<Widget> body, required String action}) async =>
      await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: Text(title),
          content: SizedBox(
            width: 460,
            child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.start, children: body),
          ),
          actions: [
            TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
            FilledButton(onPressed: () => Navigator.pop(context, true), child: Text(action)),
          ],
        ),
      ) ??
      false;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return AlertDialog(
      title: const Text('Hosting'),
      content: SizedBox(
        width: 560,
        child: SingleChildScrollView(
          child: ListenableBuilder(
            listenable: widget.supervisor,
            builder: (context, _) => Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                _Status(supervisor: widget.supervisor),
                const SizedBox(height: 16),
                Row(
                  children: [
                    Expanded(child: Text('Agents this computer hosts', style: text.titleSmall)),
                    TextButton.icon(
                      onPressed: busy || widget.supervisor.tool == null ? null : _add,
                      icon: const Icon(Icons.add),
                      label: const Text('Add an agent'),
                    ),
                  ],
                ),
                if (config.hostEntries.isEmpty)
                  const Padding(padding: EdgeInsets.symmetric(vertical: 8), child: Text('None yet.')),
                for (final entry in config.hostEntries) _entryTile(entry),
                const Divider(height: 32),
                Row(
                  children: [
                    Expanded(child: Text('Keys that may connect', style: text.titleSmall)),
                    IconButton(
                      tooltip: 'Look again',
                      onPressed: busy ? null : _refresh,
                      icon: const Icon(Icons.refresh),
                    ),
                  ],
                ),
                const Text('A browser pairs by connecting once: its key then waits here until you pair it.'),
                FutureBuilder<List<HostKey>>(
                  future: keys,
                  builder: (context, snapshot) {
                    if (snapshot.hasError) return Text('The keys could not be read: ${snapshot.error}');
                    final found = snapshot.data;
                    if (found == null) return const LinearProgressIndicator();
                    if (found.isEmpty) {
                      return const Padding(padding: EdgeInsets.symmetric(vertical: 8), child: Text('No keys yet.'));
                    }
                    return Column(children: [for (final key in found) _keyTile(key)]);
                  },
                ),
                const Divider(height: 32),
                Text('Pair a program by PIN', style: text.titleSmall),
                const Text(
                  'For a program with a key of its own, such as coqui-acp-bridge pair-dial. A browser cannot pair by PIN.',
                ),
                const SizedBox(height: 8),
                _pinPanel(text),
                if (message != null)
                  Padding(
                    padding: const EdgeInsets.only(top: 12),
                    child: Text(message!, style: TextStyle(color: Theme.of(context).colorScheme.error)),
                  ),
              ],
            ),
          ),
        ),
      ),
      actions: [TextButton(onPressed: () => Navigator.pop(context), child: const Text('Close'))],
    );
  }

  Widget _entryTile(HostEntry entry) {
    String? url;
    for (final listening in widget.supervisor.listening) {
      if (listening.name == entry.name) url = listening.url;
    }
    return ListTile(
      contentPadding: EdgeInsets.zero,
      leading: Icon(entry.kind == 'agent' ? Icons.smart_toy_outlined : Icons.extension_outlined),
      title: Text(entry.name),
      subtitle: Text(
        [entry.runs, ...entry.args].join(' ') + (url == null ? '' : '\n$url'),
        maxLines: 3,
        overflow: TextOverflow.ellipsis,
      ),
      trailing: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (url != null)
            IconButton(
              tooltip: 'Copy its address',
              icon: const Icon(Icons.copy),
              onPressed: () => Clipboard.setData(ClipboardData(text: url!)),
            ),
          IconButton(
            tooltip: 'Stop hosting ${entry.name}',
            icon: const Icon(Icons.delete_outline),
            onPressed: busy ? null : () => _remove(entry),
          ),
        ],
      ),
    );
  }

  Widget _keyTile(HostKey key) => ListTile(
    contentPadding: EdgeInsets.zero,
    leading: Icon(key.accepted ? Icons.verified_user_outlined : Icons.hourglass_top),
    title: Text(key.accepted ? 'Paired' : 'Waiting${key.from == null ? '' : ' (from ${key.from})'}'),
    subtitle: _Id(key.id),
    trailing: Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (!key.accepted) TextButton(onPressed: busy ? null : () => _accept(key), child: const Text('Pair…')),
        IconButton(
          tooltip: key.accepted ? 'Remove this key' : 'Turn this key away',
          icon: const Icon(Icons.remove_circle_outline),
          onPressed: busy ? null : () => _revoke(key),
        ),
      ],
    ),
  );

  Widget _pinPanel(TextTheme text) {
    final running = pairing;
    if (running != null) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SelectableText(
            running.pin,
            style: text.displaySmall?.copyWith(fontFamily: CanoaLook.monospaceFont, letterSpacing: 6),
          ),
          SelectableText('Dial ${running.url}', style: text.bodySmall),
          const SizedBox(height: 4),
          const Text('The other side shows this host as:'),
          Row(
            children: [
              Expanded(child: _Id(running.hostId)),
              ?_method(running.method),
            ],
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              const SizedBox(width: 16, height: 16, child: CircularProgressIndicator(strokeWidth: 2)),
              const SizedBox(width: 8),
              const Expanded(child: Text('Waiting for the PIN. One wrong try ends it.')),
              TextButton(onPressed: _cancelPin, child: const Text('Cancel')),
            ],
          ),
        ],
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (paired != null) ...[const Text('Paired:'), _Id(paired!), const SizedBox(height: 8)],
        OutlinedButton.icon(
          onPressed: busy || widget.supervisor.tool == null ? null : _showPin,
          icon: const Icon(Icons.pin_outlined),
          label: const Text('Show a PIN'),
        ),
      ],
    );
  }
}

/// The proof a host runs, shown beside its id, or nothing from a host that does not say.
Widget? _method(String? method) => method == null ? null : Text(' ($method)');

/// A peer id whole, as text that can be selected and compared.
class _Id extends StatelessWidget {
  const _Id(this.id);
  final String id;
  @override
  Widget build(BuildContext context) =>
      SelectableText(id, style: const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12));
}

class _Status extends StatelessWidget {
  const _Status({required this.supervisor});
  final HostSupervisor supervisor;

  @override
  Widget build(BuildContext context) {
    final (icon, said) = switch (supervisor.phase) {
      HostPhase.absent => (Icons.block, 'canoa-host was not found beside Canoa, so nothing is hosted.'),
      HostPhase.idle => (Icons.pause_circle_outline, 'Nothing to host: add an agent.'),
      HostPhase.starting => (Icons.hourglass_top, 'Starting…'),
      HostPhase.running => (Icons.check_circle_outline, 'Hosting, on this computer only.'),
      HostPhase.waiting => (Icons.restart_alt, 'It stopped: starting again soon. ${supervisor.problem ?? ''}'),
      HostPhase.failed => (Icons.error_outline, 'Canoa.cfg is not right for it: ${supervisor.problem ?? ''}'),
      HostPhase.stopped => (Icons.stop_circle_outlined, 'Stopped.'),
    };
    final peer = supervisor.peerId;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            Icon(icon),
            const SizedBox(width: 8),
            Expanded(child: Text(said)),
            if (supervisor.phase == HostPhase.failed || supervisor.phase == HostPhase.waiting)
              TextButton(onPressed: () => unawaited(supervisor.restart()), child: const Text('Try again')),
          ],
        ),
        if (peer != null) ...[
          const SizedBox(height: 8),
          const Text('A browser pins this host as:'),
          Row(
            children: [
              Expanded(child: _Id(peer)),
              ?_method(supervisor.peerMethod),
              IconButton(
                tooltip: 'Copy the host id',
                icon: const Icon(Icons.copy),
                onPressed: () => Clipboard.setData(ClipboardData(text: peer)),
              ),
            ],
          ),
        ],
      ],
    );
  }
}

final class _NewAgent {
  const _NewAgent(this.name, this.command, this.root);
  final String name;
  final AgentCommand command;
  final String root;
}

class _AddAgentDialog extends StatefulWidget {
  const _AddAgentDialog();
  @override
  State<_AddAgentDialog> createState() => _AddAgentDialogState();
}

class _AddAgentDialogState extends State<_AddAgentDialog> {
  final name = TextEditingController();
  final command = TextEditingController();
  final root = TextEditingController();
  String? error;

  @override
  void dispose() {
    name.dispose();
    command.dispose();
    root.dispose();
    super.dispose();
  }

  void _submit() {
    final parsed = parseCommand(command.text);
    final folder = root.text.trim();
    final problem = !HostEntry.isName(name.text.trim())
        ? 'A name is 1 to 64 letters, digits, - or _.'
        : parsed == null
        ? 'That is not a command: check its quotes.'
        : folder.isNotEmpty && !(folder.startsWith('/') || RegExp(r'^[A-Za-z]:[\\/]').hasMatch(folder))
        ? 'The folder must be a whole path.'
        : null;
    if (problem != null) return setState(() => error = problem);
    Navigator.pop(context, _NewAgent(name.text.trim(), parsed!, folder));
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Add an agent'),
    content: SizedBox(
      width: 460,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text(
            'Any program that speaks ACP on its standard streams. It runs as you, for each browser that connects.',
          ),
          TextField(
            controller: name,
            decoration: const InputDecoration(labelText: 'Name'),
          ),
          TextField(
            controller: command,
            decoration: const InputDecoration(
              labelText: 'Command',
              hintText: 'npx -y @agentclientprotocol/claude-agent-acp@0.99.1',
            ),
          ),
          TextField(
            controller: root,
            decoration: const InputDecoration(labelText: 'Only in this folder (optional)', hintText: 'a whole path'),
          ),
          if (error != null)
            Padding(
              padding: const EdgeInsets.only(top: 12),
              child: Text(error!, style: TextStyle(color: Theme.of(context).colorScheme.error)),
            ),
        ],
      ),
    ),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
      FilledButton(onPressed: _submit, child: const Text('Add')),
    ],
  );
}
