import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLookOf;
import 'package:flutter/material.dart';

import '../ledger/session_ledger.dart';
import '../session_store.dart';
import '../workspace/site.dart';
import 'tab_shares.dart';

/// The site the tab in front shows, or null when it is not a web page with a host name that is safe as a folder.
String? frontSite(TabShares tabs) {
  final front = tabs.frontTab;
  return front == null ? null : siteFqdn(front.url);
}

/// The sessions of the site in front that are not open, newest first, to resume or to start another beside. Nothing of another
/// site is read into it.
class SiteSessions extends StatefulWidget {
  const SiteSessions({
    super.key,
    required this.tabs,
    required this.ledger,
    required this.store,
    required this.onNew,
    required this.onResume,
  });

  final TabShares tabs;
  final SessionLedger ledger;
  final SessionStore store;

  /// Starts a session for the site. Called by the person's click, with nothing awaited before it.
  final Future<void> Function(String site) onNew;
  final Future<void> Function(LedgerEntry entry) onResume;

  @override
  State<SiteSessions> createState() => _SiteSessionsState();
}

class _SiteSessionsState extends State<SiteSessions> {
  String? _failed;
  bool _busy = false;

  static String _when(DateTime at) {
    final local = at.toLocal();
    String two(int value) => value.toString().padLeft(2, '0');
    return '${local.year}-${two(local.month)}-${two(local.day)} ${two(local.hour)}:${two(local.minute)}';
  }

  Future<void> _run(Future<void> Function() action) async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _failed = null;
    });
    try {
      await action();
    } on Object catch (error) {
      if (mounted) setState(() => _failed = '$error'.replaceFirst(RegExp(r'^(Bad state|Exception): '), ''));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: Listenable.merge([widget.tabs, widget.ledger, widget.store]),
      builder: (context, _) {
        final site = frontSite(widget.tabs);
        if (site == null) return const SizedBox.shrink();
        final look = context.chatLook;
        final theme = Theme.of(context);
        final open = {
          for (final session in widget.store.sessions)
            if (session.peerId != null && session.remoteSessionId != null)
              LedgerEntry.keyOf(session.peerId!, session.remoteSessionId!),
        };
        final earlier = [for (final entry in widget.ledger.forSite(site)) if (!open.contains(entry.key)) entry];
        return Padding(
          padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Row(children: [
                Icon(Icons.public, size: 18, color: look.muted),
                const SizedBox(width: 8),
                Expanded(child: Text(site, key: const Key('site-name'), style: theme.textTheme.titleSmall, overflow: TextOverflow.ellipsis)),
                FilledButton.tonal(
                  key: const Key('site-new'),
                  onPressed: _busy ? null : () => unawaited(_run(() => widget.onNew(site))),
                  child: const Text('New session'),
                ),
              ]),
              if (_failed != null)
                Padding(
                  padding: const EdgeInsets.only(top: 6),
                  child: Text(_failed!, key: const Key('site-failed'), style: TextStyle(color: theme.colorScheme.error, fontSize: 12)),
                ),
              if (earlier.isNotEmpty) ...[
                const SizedBox(height: 12),
                Text('Earlier on $site', style: theme.textTheme.labelLarge?.copyWith(fontWeight: FontWeight.w700)),
                for (final entry in earlier)
                  ListTile(
                    key: ValueKey('site-session ${entry.key}'),
                    dense: true,
                    contentPadding: EdgeInsets.zero,
                    title: Text(entry.title ?? 'Untitled session', maxLines: 1, overflow: TextOverflow.ellipsis),
                    subtitle: Text([_when(entry.lastUsed), entry.cwd].nonNulls.join(' · '), maxLines: 2, overflow: TextOverflow.ellipsis),
                    trailing: TextButton(
                      key: ValueKey('site-resume ${entry.key}'),
                      onPressed: _busy ? null : () => unawaited(_run(() => widget.onResume(entry))),
                      child: const Text('Resume'),
                    ),
                  ),
              ],
            ],
          ),
        );
      },
    );
  }
}
