import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart';

import 'package:coqui_acp/page.dart';

import 'acp/browser_connect.dart';
import 'auth/browser_peer.dart';
import 'config/connection.dart';
import 'brand.dart';
import 'selectable.dart';
import 'shell/hosts/hosts_controller.dart' show NearbyCandidate, NearbyResult, NearbySearch;

/// What a session on a pinned peer needs to start: where, which key, and the workspace and title it starts with.
/// No trust-on-first-use is performed: the id is the one the person saved or verified out of band.
class ConnectTarget {
  const ConnectTarget({
    required this.webSocket,
    required this.webTransport,
    required this.peerId,
    required this.cwd,
    required this.title,
    required this.method,
    this.worktree = false,
  });
  final String webSocket;
  final String webTransport;
  final String peerId;
  final String cwd;
  final String title;

  /// Whether the host makes a new git worktree of [cwd], on a branch of the session's own, for the session to work in.
  final bool worktree;

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

/// Uses the agents a person grants through the browser's provider, asking them in the provider's own window.
typedef UseBrowserAgents = Future<void> Function(AcpPage acp, {required String cwd, required String title});

/// Adds a saved connection: the Hosts section's Add. Nothing is ticked and nothing is saved until the person names it, checks the id
/// against what the other end shows, and adds it; the proof at connect then decides whether that end holds the id. A candidate found
/// nearby only fills the fields: it is a claim.
Future<void> showAddConnectionDialog(
  BuildContext context, {
  required Future<void> Function(Connection) onAdd,
  NearbySearch? nearby,
  MethodCheck methodUnavailable = browserMethodUnavailable,
  LocalPeerId localPeerId = localBrowserPeerId,
  PairByPin pair = pairBrowserAcp,
  bool inBrowser = kIsWeb,
}) => showSelectableDialog<void>(
  context: context,
  builder: (_) => _AddConnectionDialog(
    onAdd: onAdd,
    nearby: nearby,
    methodUnavailable: methodUnavailable,
    localPeerId: localPeerId,
    pair: pair,
    inBrowser: inBrowser,
  ),
);

class _AddConnectionDialog extends StatefulWidget {
  const _AddConnectionDialog({
    required this.onAdd,
    required this.methodUnavailable,
    required this.localPeerId,
    required this.pair,
    required this.inBrowser,
    this.nearby,
  });

  final Future<void> Function(Connection) onAdd;
  final NearbySearch? nearby;
  final MethodCheck methodUnavailable;
  final LocalPeerId localPeerId;
  final PairByPin pair;

  /// Whether this app is a web page, whose key and identity are the browser's; otherwise they are this app's own.
  final bool inBrowser;

  @override
  State<_AddConnectionDialog> createState() => _AddConnectionDialogState();
}

class _AddConnectionDialogState extends State<_AddConnectionDialog> {
  final name = TextEditingController();
  final address = TextEditingController();
  final pinned = TextEditingController();
  final pairUrl = TextEditingController();
  final pin = TextEditingController();
  ConnectionKind kind = ConnectionKind.canoa;
  ConnectionTransport transport = ConnectionTransport.websocket;
  PeerMethod? method;
  Future<(String?, String?)>? own;
  late final Future<String?> noiseUnavailable = widget.methodUnavailable(PeerMethod.noise);
  bool busy = false;
  String? error;
  String? paired;

  /// What a search nearby found, once one has run: null before.
  NearbyResult? found;
  bool looking = false;
  NearbyCandidate? chosen;

  /// This app's id under [method], or why it has none: (id, null) or (null, why).
  Future<(String?, String?)> _own(PeerMethod method) async {
    final why = await widget.methodUnavailable(method);
    if (why != null) return (null, why);
    return (await widget.localPeerId(method), null);
  }

  void _choose(PeerMethod? chosen) {
    method = chosen;
    own = chosen == null ? null : _own(chosen);
  }

  /// This page's origin, which `canoa pair` lets in by `--origin`; null where the app is no web page.
  static String? get _pageOrigin {
    final base = Uri.base;
    return base.scheme == 'http' || base.scheme == 'https' || base.scheme == 'chrome-extension'
        ? '${base.scheme}://${base.authority}'
        : null;
  }

  @override
  void dispose() {
    name.dispose();
    address.dispose();
    pinned.dispose();
    pairUrl.dispose();
    pin.dispose();
    super.dispose();
  }

  Future<void> _look() async {
    final search = widget.nearby;
    if (search == null || looking) return;
    setState(() {
      looking = true;
      chosen = null;
    });
    try {
      final result = await search();
      if (mounted) setState(() => found = result);
    } on Object catch (problem) {
      if (mounted) setState(() => found = NearbyResult(const [], problem: '$problem'));
    } finally {
      if (mounted) setState(() => looking = false);
    }
  }

  /// Fills the fields from a candidate. What it says is a claim: the person still names the connection and adds it, and the
  /// first connection is what proves the key.
  void _take(NearbyCandidate candidate, bool on) => setState(() {
    chosen = on ? candidate : null;
    if (!on) return;
    kind = ConnectionKind.acp;
    transport = ConnectionTransport.websocket;
    address.text = candidate.url;
    pinned.text = candidate.peer;
    if (name.text.isEmpty) name.text = candidate.label;
    final named = PeerMethod.values.where((each) => each.id == candidate.method).firstOrNull;
    if (named != null) _choose(named);
  });

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
        paired = 'Paired under Noise. Enter its address, then add it.';
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
    final label = name.text.trim();
    if (!Connection.isName(label)) {
      setState(() => error = 'Name it: 1 to 64 characters.');
      return;
    }
    final problem = Connection.addressProblem(kind, transport, address.text);
    if (problem != null) {
      setState(() => error = problem);
      return;
    }
    final peer = pinned.text.trim().toLowerCase();
    if (!RegExp(r'^sha256:[0-9a-f]{64}$').hasMatch(peer)) {
      setState(() => error = 'Enter the verified server peer ID (sha256:64 hex).');
      return;
    }
    final method = this.method;
    if (method == null) {
      setState(() => error = 'Choose the method the host proves by: ${Brand.program} serve prints it.');
      return;
    }
    setState(() {busy = true; error = null;});
    final why = await widget.methodUnavailable(method);
    if (why != null) {
      if (mounted) setState(() {busy = false; error = '${method.label} is unavailable in $_here: $why';});
      return;
    }
    try {
      await widget.onAdd(Connection(
        name: label,
        kind: kind,
        address: address.text.trim(),
        peer: peer,
        method: method.id,
        transport: kind == ConnectionKind.acp ? transport : ConnectionTransport.websocket,
      ));
      if (mounted) Navigator.of(context).pop();
    } on FormatException catch (problem) {
      if (mounted) setState(() => error = problem.message);
    } catch (e) {
      if (mounted) setState(() => error = 'That was not saved: $e');
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  /// What holds the key a peer pins: the browser for a web page, this app otherwise.
  String get _here => widget.inBrowser ? 'this browser' : 'this app';

  Widget _nearby(BuildContext context) {
    final text = Theme.of(context).textTheme.bodySmall;
    final result = found;
    return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      const SizedBox(height: 12),
      Row(children: [
        Expanded(child: Text('Nearby', style: Theme.of(context).textTheme.titleSmall)),
        if (widget.nearby != null)
          TextButton.icon(
            key: const ValueKey('nearby-look'),
            onPressed: looking || busy ? null : _look,
            icon: looking
                ? const SizedBox(width: 16, height: 16, child: CircularProgressIndicator(strokeWidth: 2))
                : const Icon(Icons.radar, size: 18),
            label: Text(result == null ? 'Look nearby' : 'Look again'),
          ),
      ]),
      if (widget.nearby == null)
        Text(
          'Looking nearby needs the ${Brand.name} on this computer, which browses for agents; none is reachable from here.',
          key: const ValueKey('nearby-needs-host'),
          style: text,
        )
      else if (result == null)
        Text('Agents on this computer and on the link announce themselves. What they say is a claim: nothing is added until you add it.', style: text)
      else ...[
        if (result.problem != null) Text(result.problem!, key: const ValueKey('nearby-problem'), style: text),
        if (result.candidates.isEmpty && result.problem == null) Text('Nothing announced itself.', key: const ValueKey('nearby-none'), style: text),
        for (final candidate in result.candidates)
          CheckboxListTile(
            key: ValueKey('nearby ${candidate.url}'),
            dense: true,
            contentPadding: EdgeInsets.zero,
            controlAffinity: ListTileControlAffinity.leading,
            value: identical(chosen, candidate),
            onChanged: busy ? null : (on) => _take(candidate, on ?? false),
            title: Text(candidate.label),
            subtitle: Text(
              '${candidate.url}${candidate.offers.isEmpty ? '' : '\nOffers ${candidate.offers.join(', ')}'}\nIts claim: ${candidate.peer}',
              style: text,
            ),
          ),
      ],
    ]);
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme.bodySmall;
    return AlertDialog(
      title: const Text('Add connection'),
      content: SizedBox(
        width: 460,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'Remote sessions require verified peer authentication. The other end must be explicitly paired with $_here. Only enter a fingerprint verified out of band. '
                '${Brand.name} never trusts the first key it sees, and keeps no key or password here: only where to connect and the id to expect.',
              ),
              const SizedBox(height: 12),
              TextField(key: const ValueKey('connection-name'), controller: name, decoration: const InputDecoration(labelText: 'Name')),
              const SizedBox(height: 12),
              Text('What it is', style: text),
              const SizedBox(height: 4),
              SegmentedButton<ConnectionKind>(
                key: const ValueKey('connection-kind'),
                segments: [for (final each in ConnectionKind.values) ButtonSegment(value: each, label: Text(each.label))],
                selected: {kind},
                showSelectedIcon: false,
                onSelectionChanged: busy ? null : (chosen) => setState(() => kind = chosen.first),
              ),
              const SizedBox(height: 4),
              Text(
                kind == ConnectionKind.canoa
                    ? 'It lists its agents and what it can do; its address is the one ${Brand.program} serve prints, such as https://host:port/.'
                    : 'It offers a session and nothing else: no folder browsing, files or management.',
                style: text,
              ),
              if (kind == ConnectionKind.acp) ...[
                const SizedBox(height: 8),
                SegmentedButton<ConnectionTransport>(
                  key: const ValueKey('connection-transport'),
                  segments: const [
                    ButtonSegment(value: ConnectionTransport.websocket, label: Text('WebSocket')),
                    ButtonSegment(value: ConnectionTransport.webtransport, label: Text('WebTransport')),
                  ],
                  selected: {transport},
                  showSelectedIcon: false,
                  onSelectionChanged: busy ? null : (chosen) => setState(() => transport = chosen.first),
                ),
              ],
              TextField(
                key: const ValueKey('connection-address'),
                controller: address,
                decoration: InputDecoration(
                  labelText: 'Address',
                  hintText: switch ((kind, transport)) {
                    (ConnectionKind.canoa, _) => 'https://host:port/',
                    (_, ConnectionTransport.webtransport) => 'https://agent.example:4433/acp',
                    _ => 'wss://agent.example/acp',
                  },
                ),
              ),
              TextField(
                key: const ValueKey('connection-peer'),
                controller: pinned,
                decoration: const InputDecoration(labelText: 'Pinned peer ID', hintText: 'sha256:…'),
              ),
              const SizedBox(height: 12),
              Text('Method the other end proves by', style: text),
              const SizedBox(height: 4),
              SegmentedButton<PeerMethod>(
                key: const ValueKey('connection-method'),
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
                      style: text)),
                    IconButton(
                      tooltip: 'Copy your peer ID for pairing',
                      icon: const Icon(Icons.copy),
                      onPressed: () => Clipboard.setData(ClipboardData(text: peer)),
                    ),
                  ]);
                },
              ),
              // Pairing by PIN is the browser's: a desktop app has no such method, so it offers none.
              if (widget.inBrowser) FutureBuilder<String?>(
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
                        SelectableText('There, run: ${Brand.program} pair --method noise --origin $origin', style: text),
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
              _nearby(context),
              if (error != null)
                Padding(
                  padding: const EdgeInsets.only(top: 12),
                  child: Text(error!, key: const ValueKey('connection-error'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
                ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(onPressed: busy ? null : () => Navigator.of(context).pop(), child: const Text('Cancel')),
        FilledButton(
          key: const ValueKey('connection-add'),
          onPressed: busy ? null : _submit,
          child: busy
              ? const SizedBox(width: 18, height: 18, child: CircularProgressIndicator(strokeWidth: 2))
              : const Text('Add'),
        ),
      ],
    );
  }
}
