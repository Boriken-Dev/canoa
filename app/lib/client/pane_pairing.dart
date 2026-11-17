// Pairing this browser's key with the host that served the page, by the PIN the host shows, from the chat-only app. A task pane has
// no Hosts section and cannot open a window of its own, so the pairing is a card in the pane itself.
import 'package:flutter/material.dart';

import '../acp/browser_connect.dart' show pairBrowserAcp;
import '../connect_dialog.dart' show PairByPin;
import '../served/navigate.dart' show reloadPage;

/// Asks for the address the host pairs on and the PIN it shows (`canoa pair --method noise --origin ORIGIN` on the host's
/// computer), pairs, checks that what answered is the host that served this page, and then starts the app over, which connects with
/// the key the host now accepts.
class PanePairing extends StatefulWidget {
  const PanePairing({super.key, required this.hostPeer, required this.origin, this.pair = pairBrowserAcp, this.onPaired = reloadPage});

  /// The id of the host that served this page, which the pairing must have been with.
  final String hostPeer;

  /// This page's origin, which the host's `canoa pair` is told to let in (`--origin`).
  final String origin;

  final PairByPin pair;

  /// What to do once the key is paired.
  final VoidCallback onPaired;

  @override
  State<PanePairing> createState() => _PanePairingState();
}

class _PanePairingState extends State<PanePairing> {
  final _address = TextEditingController();
  final _pin = TextEditingController();
  bool _busy = false;
  String? _error;

  @override
  void dispose() {
    _address.dispose();
    _pin.dispose();
    super.dispose();
  }

  Future<void> _pair() async {
    if (_busy) return;
    final url = Uri.tryParse(_address.text.trim());
    final code = _pin.text.trim();
    if (url == null || !url.hasAuthority || !{'ws', 'wss'}.contains(url.scheme) || code.isEmpty) {
      setState(() => _error = 'Enter the address the host pairs on (it starts with ws:// or wss://) and the PIN it shows.');
      return;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final peer = await widget.pair(url, code);
      if (!mounted) return;
      if (peer.toLowerCase() != widget.hostPeer.toLowerCase()) {
        setState(() => _error = 'That is not the host that served this page, so nothing here changes. Ask the host that did for a PIN.');
        return;
      }
      widget.onPaired();
    } on Object catch (error) {
      if (mounted) setState(() => _error = 'Pairing failed: $error. A PIN is good for one try: ask for a new one.');
    } finally {
      // Tried once, right or wrong, the PIN is spent.
      if (mounted) {
        _pin.clear();
        setState(() => _busy = false);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Card(
      key: const Key('pane-pairing'),
      margin: const EdgeInsets.all(12),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Pair this pane with its host', style: text.titleSmall),
            const SizedBox(height: 4),
            Text(
              'The host has not accepted this pane yet. On the host\'s computer run `canoa pair --method noise --origin ${widget.origin}`, '
              'then enter the address and the PIN it shows.',
              style: text.bodySmall,
            ),
            const SizedBox(height: 8),
            TextField(
              key: const Key('pane-pairing-address'),
              controller: _address,
              decoration: const InputDecoration(labelText: 'Address', hintText: 'ws://…'),
              keyboardType: TextInputType.url,
              autocorrect: false,
            ),
            TextField(
              key: const Key('pane-pairing-pin'),
              controller: _pin,
              decoration: const InputDecoration(labelText: 'PIN'),
              keyboardType: TextInputType.number,
              onSubmitted: (_) => _pair(),
            ),
            if (_error != null)
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Text(_error!, key: const Key('pane-pairing-error'), style: text.bodySmall?.copyWith(color: Theme.of(context).colorScheme.error)),
              ),
            Align(
              alignment: Alignment.centerRight,
              child: TextButton(
                key: const Key('pane-pairing-go'),
                onPressed: _busy ? null : _pair,
                child: Text(_busy ? 'Pairing…' : 'Pair'),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
