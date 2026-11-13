import 'dart:async';

import 'package:flutter/material.dart';

import 'tab_header.dart';
import 'tab_shares.dart';

/// What a popped-out window says of the tab its session works with, when another client holds it, and the way to take it: the
/// same `Move here` the panel offers, as a line over the conversation. The window shares nothing and attaches nothing.
class TabOwnerBanner extends StatefulWidget {
  const TabOwnerBanner({super.key, required this.tabs, required this.sessionKey});

  final TabShares tabs;

  /// The key the session is known by in the worker.
  final String sessionKey;

  @override
  State<TabOwnerBanner> createState() => _TabOwnerBannerState();
}

class _TabOwnerBannerState extends State<TabOwnerBanner> {
  @override
  void dispose() {
    widget.tabs.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.tabs,
    builder: (context, _) {
      final offer = widget.tabs.moveOffer(widget.sessionKey);
      if (offer == null) return const SizedBox.shrink();
      return Padding(
        padding: const EdgeInsets.fromLTRB(12, 8, 12, 0),
        child: TabBanner(
          key: const Key('move-here'),
          text: offer,
          action: 'Move here',
          onAction: () => unawaited(widget.tabs.moveHere(widget.sessionKey)),
        ),
      );
    },
  );
}
