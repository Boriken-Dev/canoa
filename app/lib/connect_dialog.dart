import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'package:coqui_acp/page.dart';

import 'acp/agent_commands.dart';
import 'acp/browser_connect.dart';
import 'acp/local_agent.dart';
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
    required this.method,
  });
  final String webSocket;
  final String webTransport;
  final String peerId;
  final String cwd;
  final String title;

  /// The proof the agent's key makes: named by the person, or by the record or host the agent came from; never assumed,
  /// and never guessed from a connection.
  final PeerMethod method;
}

/// Why this browser cannot prove by a method, or null.
typedef MethodCheck = Future<String?> Function(PeerMethod method);

/// This browser's own peer id under a method.
typedef LocalPeerId = Future<String> Function(PeerMethod method);

/// Pairs this browser's Noise key with the machine waiting at an address, by the PIN it shows, and answers that machine's id.
typedef PairByPin = Future<String> Function(Uri url, String pin);

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
  MethodCheck methodUnavailable = browserMethodUnavailable,
  LocalPeerId localPeerId = localBrowserPeerId,
  PairByPin pair = pairBrowserAcp,
  PeerMethod? method,
  bool inBrowser = kIsWeb,
}) => showDialog<void>(
  context: context,
  builder: (_) => _ConnectDialog(
    onConnect: onConnect,
    browserProvider: browserProvider,
    onUseBrowserAgents: onUseBrowserAgents,
    onRunLocalAgent: onRunLocalAgent,
    localAgents: localAgents,
    checkInstalled: checkInstalled,
    methodUnavailable: methodUnavailable,
    localPeerId: localPeerId,
    pair: pair,
    method: method,
    inBrowser: inBrowser,
  ),
);

class _ConnectDialog extends StatefulWidget {
  const _ConnectDialog({
    required this.onConnect,
    required this.methodUnavailable,
    required this.localPeerId,
    required this.pair,
    this.browserProvider,
    this.onUseBrowserAgents,
    this.onRunLocalAgent,
    this.localAgents = agentPresets,
    this.checkInstalled = isInstalled,
    this.method,
    this.inBrowser = kIsWeb,
  });

  /// Whether this app is a web page, whose key and identity are the browser's; otherwise they are this app's own.
  final bool inBrowser;

  /// The method an agent's record or its host named, prefilled; null for an agent typed in by hand.
  final PeerMethod? method;
  final MethodCheck methodUnavailable;
  final LocalPeerId localPeerId;
  final PairByPin pair;
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
  final pairUrl = TextEditingController();
  final pin = TextEditingController();
  late PeerMethod? method = widget.method;
  late Future<(String?, String?)>? own = switch (method) {
    final PeerMethod method => _own(method),
    null => null,
  };
  late final Future<String?> noiseUnavailable = widget.methodUnavailable(PeerMethod.noise);
  bool busy = false;
  String? error;
  String? paired;

  /// This browser's id under [method], or why it has none: (id, null) or (null, why).
  Future<(String?, String?)> _own(PeerMethod method) async {
    final why = await widget.methodUnavailable(method);
    if (why != null) return (null, why);
    return (await widget.localPeerId(method), null);
  }

  /// This page's origin, which `pair-accept` lets in by `--origin`; null where the app is no web page.
  static String? get _pageOrigin {
    final base = Uri.base;
    return base.scheme == 'http' || base.scheme == 'https' || base.scheme == 'chrome-extension'
        ? '${base.scheme}://${base.authority}'
        : null;
  }

  void _choose(PeerMethod? chosen) {
    method = chosen;
    own = chosen == null ? null : _own(chosen);
  }

  @override
  void dispose() {
    ws.dispose();
    wt.dispose();
    pinned.dispose();
    cwd.dispose();
    title.dispose();
    pairUrl.dispose();
    pin.dispose();
    super.dispose();
  }

  Future<void> _pair() async {
    if (busy) return;
    final url = Uri.tryParse(pairUrl.text.trim());
    final code = pin.text.trim();
    if (url == null || !url.hasAuthority || code.isEmpty) {
      setState(() => error = 'Enter the address the other machine pairs on, and the PIN it shows.');
      return;
    }
    setState(() {busy = true; error = null; paired = null;});
    try {
      final peer = await widget.pair(url, code);
      if (!mounted) return;
      setState(() {
        pinned.text = peer;
        _choose(PeerMethod.noise);
        paired = 'Paired under Noise. Enter its agent\'s endpoint, then connect.';
      });
    } catch (e) {
      if (mounted) setState(() => error = 'Pairing failed: $e. A PIN is good for one try: ask for a new one.');
    } finally {
      // Tried once, right or wrong, the PIN is spent.
      if (mounted) {
        pin.clear();
        setState(() => busy = false);
      }
    }
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
      cwd: cwd.text.trim(),
      title: title.text.trim(),
      method: method,
    );
    setState(() {busy = true; error = null;});
    final why = await widget.methodUnavailable(request.method);
    if (why != null) {
      if (mounted) setState(() {busy = false; error = '${request.method.label} is unavailable in $_here: $why';});
      return;
    }
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

  /// What holds the key a peer pins: the browser for a web page, this app otherwise.
  String get _here => widget.inBrowser ? 'this browser' : 'this app';

  @override
  Widget build(BuildContext context) {
    // An agent on this computer needs only where it works and a title, so those come first, beside the Run buttons; pairing and
    // endpoints are about agents elsewhere and follow under their own heading.
    final local = widget.onRunLocalAgent != null;
    final intro = Text(
      'The server must be explicitly paired with $_here. '
      'Only enter a fingerprint verified out of band. '
      'Canoa never trusts the first key it sees.',
    );
    final titleField = TextField(controller: title,
      decoration: const InputDecoration(labelText: 'Session title (optional)'));
    final cwdField = TextField(controller: cwd,
      decoration: const InputDecoration(
        labelText: 'Agent workspace (absolute path)',
      ));
    return AlertDialog(
      title: Text(local ? 'Run or connect to an agent' : 'Connect to verified agent'),
      content: SizedBox(
        width: 440,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (local) ...[cwdField, titleField] else intro,
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
              if (local) ...[
                Text('Or connect to an agent on another machine', style: Theme.of(context).textTheme.titleSmall),
                const SizedBox(height: 4),
                intro,
                const SizedBox(height: 12),
              ],
              Text('Method the host proves by', style: Theme.of(context).textTheme.bodySmall),
              const SizedBox(height: 4),
              SegmentedButton<PeerMethod>(
                segments: [for (final each in PeerMethod.values) ButtonSegment(value: each, label: Text(each.label))],
                selected: {?method},
                emptySelectionAllowed: true,
                showSelectedIcon: false,
                onSelectionChanged: busy ? null : (chosen) => setState(() => _choose(chosen.firstOrNull)),
              ),
              const SizedBox(height: 8),
              if (own == null)
                Text('Choose the method to see $_here\'s peer ID under it.')
              else FutureBuilder<(String?, String?)>(
                key: ValueKey(own),
                future: own!,
                builder: (context, snapshot) {
                  if (!snapshot.hasData) {
                    return Text(switch ((snapshot.hasError, widget.inBrowser)) {
                      (true, true) => 'Browser identity unavailable: ${snapshot.error}',
                      (true, false) => 'This app has no peer ID: ${snapshot.error}',
                      (false, true) => 'Loading your browser identity…',
                      (false, false) => 'Loading this app\'s peer ID…',
                    });
                  }
                  final (peer, why) = snapshot.data!;
                  if (peer == null) {
                    return Text('${method!.label} is unavailable in $_here: $why',
                      style: TextStyle(color: Theme.of(context).colorScheme.error));
                  }
                  return Row(children: [
                    Expanded(child: SelectableText('${widget.inBrowser ? 'Your browser' : 'This app\'s'} peer ID under ${method!.label}:\n$peer',
                      style: Theme.of(context).textTheme.bodySmall)),
                    IconButton(
                      tooltip: 'Copy your peer ID for pairing',
                      icon: const Icon(Icons.copy),
                      onPressed: () => Clipboard.setData(ClipboardData(text: peer)),
                    ),
                  ]);
                },
              ),
              FutureBuilder<String?>(
                future: noiseUnavailable,
                builder: (context, snapshot) {
                  if (snapshot.connectionState != ConnectionState.done) return const SizedBox.shrink();
                  final why = snapshot.hasError ? '${snapshot.error}' : snapshot.data;
                  if (why != null) {
                    return Padding(
                      padding: const EdgeInsets.only(top: 12),
                      child: Text('Pairing by PIN needs Noise, which is unavailable in $_here: $why'),
                    );
                  }
                  return Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      const SizedBox(height: 12),
                      const Text('Or pair by the PIN the other machine shows, under Noise:'),
                      if (_pageOrigin case final origin?)
                        SelectableText('There, run: coqui-acp-bridge pair-accept --method noise --origin $origin',
                          style: Theme.of(context).textTheme.bodySmall),
                      TextField(controller: pairUrl,
                        decoration: const InputDecoration(
                          labelText: 'Where it waits to pair',
                          hintText: 'ws://127.0.0.1:PORT or wss://agent.example/pair',
                        )),
                      Row(children: [
                        Expanded(child: TextField(controller: pin,
                          decoration: const InputDecoration(labelText: 'PIN'),
                          onSubmitted: (_) => _pair())),
                        TextButton(onPressed: busy ? null : _pair, child: const Text('Pair')),
                      ]),
                      if (paired != null) Text(paired!),
                    ],
                  );
                },
              ),
              if (!local) titleField,
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
              if (!local) cwdField,
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
