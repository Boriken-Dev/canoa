import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'auth/browser_peer.dart';

/// Explicit, out-of-band connection configuration for a pinned peer.
/// No trust-on-first-use is performed from the connection dialog.
class ConnectTarget {
  const ConnectTarget({
    required this.webSocket,
    required this.webTransport,
    required this.peerId,
    required this.cwd,
    required this.title,
  });
  final String webSocket;
  final String webTransport;
  final String peerId;
  final String cwd;
  final String title;
}

Future<void> showCanoaConnectDialog(
  BuildContext context,
  Future<void> Function(ConnectTarget) onConnect,
) => showDialog<void>(
  context: context,
  builder: (_) => _ConnectDialog(onConnect: onConnect),
);

class _ConnectDialog extends StatefulWidget {
  const _ConnectDialog({required this.onConnect});
  final Future<void> Function(ConnectTarget) onConnect;

  @override
  State<_ConnectDialog> createState() => _ConnectDialogState();
}

class _ConnectDialogState extends State<_ConnectDialog> {
  final ws = TextEditingController();
  final wt = TextEditingController();
  final pinned = TextEditingController();
  final cwd = TextEditingController(text: '/');
  final title = TextEditingController();
  late final Future<String> localPeerId = localBrowserPeerId();
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
    final request = ConnectTarget(
      webSocket: ws.text.trim(),
      webTransport: wt.text.trim(),
      peerId: peer,
      cwd: cwd.text.trim(),
      title: title.text.trim(),
    );
    setState(() {busy = true; error = null;});
    try {
      await widget.onConnect(request);
      if (mounted) Navigator.of(context).pop();
    } catch (e) {
      if (mounted) setState(() => error = 'Connection refused: ${e}');
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
