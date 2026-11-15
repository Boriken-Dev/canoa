// What the panel says about the Canoa on this computer, above the conversation: that it is used, or why it is not and what the
// person can do. The agents the extension has of its own work in every state.
import 'dart:async';

import 'package:flutter/material.dart';

import 'local_host.dart';

class LocalHostBanner extends StatefulWidget {
  const LocalHostBanner({super.key, required this.bridge});

  final LocalHostBridge bridge;

  @override
  State<LocalHostBanner> createState() => _LocalHostBannerState();
}

class _LocalHostBannerState extends State<LocalHostBanner> {
  LocalHostView? view;
  bool busy = false;
  String? problem;
  final pin = TextEditingController();

  @override
  void initState() {
    super.initState();
    unawaited(_run(widget.bridge.status));
  }

  @override
  void dispose() {
    pin.dispose();
    super.dispose();
  }

  Future<void> _run(Future<LocalHostView> Function() call) async {
    setState(() {
      busy = true;
      problem = null;
    });
    try {
      final next = await call();
      if (mounted) setState(() => view = next);
    } on Object catch (error) {
      if (mounted) setState(() => problem = '$error'.replaceFirst('Exception: ', ''));
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final current = view;
    if (current == null) return const SizedBox.shrink();
    final text = Theme.of(context).textTheme;
    final (said, actions) = switch (current.state) {
      LocalHostState.notInstalled => (
        current.forbidden
            ? 'Canoa on this computer does not know this extension. Your own agents work as before.'
            : 'Canoa is not installed on this computer, so this extension uses its own agents. Install Canoa to use the agents and sessions on this computer.',
        const <Widget>[],
      ),
      LocalHostState.stopped => (
        'Canoa on this computer is not running.',
        [
          FilledButton(
            onPressed: busy ? null : () => _run(widget.bridge.start),
            child: Text(busy ? 'Starting…' : 'Start Canoa'),
          ),
        ],
      ),
      LocalHostState.unavailable => ('Canoa on this computer did not answer${current.why == null ? '' : ': ${current.why}'}.', const <Widget>[]),
      LocalHostState.running when current.paired => (
        'Using ${current.name} on this computer${current.entries.isEmpty ? '' : ': ${current.entries.join(', ')}'}.',
        [TextButton(onPressed: busy ? null : () => _run(widget.bridge.unpair), child: const Text('Unpair'))],
      ),
      LocalHostState.running => (
        current.changed
            ? 'This is not the Canoa this browser was paired with. Pair it again to use it.'
            : current.canPin
                ? 'Canoa is running on this computer. Ask it for a PIN (Hosting, then Pair the browser extension) and type it here.'
                : 'Canoa is running on this computer, with a proof a PIN does not pair. Pair this browser with canoa accept.',
        [
          if (current.canPin) ...[
            SizedBox(
              width: 140,
              child: TextField(
                controller: pin,
                enabled: !busy,
                decoration: const InputDecoration(isDense: true, hintText: 'PIN'),
                onSubmitted: (_) => _pair(),
              ),
            ),
            FilledButton(onPressed: busy ? null : _pair, child: const Text('Pair')),
          ],
        ],
      ),
    };
    return Material(
      color: Theme.of(context).colorScheme.surfaceContainerHighest,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        child: Wrap(
          spacing: 8,
          runSpacing: 4,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            Text(said, style: text.bodySmall),
            ...actions,
            if (problem != null) Text(problem!, style: text.bodySmall?.copyWith(color: Theme.of(context).colorScheme.error)),
          ],
        ),
      ),
    );
  }

  void _pair() {
    final typed = pin.text.trim();
    if (typed.isEmpty) return;
    pin.clear();
    unawaited(_run(() => widget.bridge.pair(typed)));
  }
}
