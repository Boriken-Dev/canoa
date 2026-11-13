import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLookOf;
import 'package:flutter/material.dart';

import '../ledger/session_ledger.dart';
import '../session_store.dart';
import '../workspace/scratch.dart';
import '../workspace/site.dart';
import '../workspace/workspace_template.dart';
import 'tab_shares.dart';

/// The folder a session of [site] gets: [template] with the site and a new id put in, under [documents], the Documents folder
/// of the machine its agent runs on (which a page cannot read, so the person says it). One folder per site and one per session.
({String id, String folder}) siteScratch(String site, {required String template, required String documents, String? id}) {
  final session = id ?? newSessionUuid();
  final text = template.contains(r'${fqdn}') ? template : defaultBrowserWorkspace;
  final folder = scratchFolder(text, session, documents: documents, fqdn: site, separator: separatorOf(documents));
  if (folder == null) throw StateError('No folder can be made for $site');
  return (id: session, folder: folder);
}

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
    required this.documents,
    required this.template,
    required this.onNew,
    required this.onResume,
  });

  final TabShares tabs;
  final SessionLedger ledger;
  final SessionStore store;

  /// The Documents folder of the agent's machine, or null until the person gives it.
  final ValueNotifier<String?> documents;

  /// canoa.cfg's `[sessions] workspace`: the browser's own default replaces it when it has no `${fqdn}`.
  final String template;

  /// Starts a session for the site. Called by the person's click, with nothing awaited before it.
  final Future<void> Function(String site) onNew;
  final Future<void> Function(LedgerEntry entry) onResume;

  @override
  State<SiteSessions> createState() => _SiteSessionsState();
}

class _SiteSessionsState extends State<SiteSessions> {
  final _root = TextEditingController();
  String? _failed;
  bool _busy = false;

  @override
  void dispose() {
    _root.dispose();
    super.dispose();
  }

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
      listenable: Listenable.merge([widget.tabs, widget.ledger, widget.store, widget.documents]),
      builder: (context, _) {
        final site = frontSite(widget.tabs);
        if (site == null) return const SizedBox.shrink();
        final look = context.chatLook;
        final theme = Theme.of(context);
        final documents = widget.documents.value;
        final open = {
          for (final session in widget.store.sessions)
            if (session.peerId != null && session.remoteSessionId != null)
              LedgerEntry.keyOf(session.peerId!, session.remoteSessionId!),
        };
        final earlier = [for (final entry in widget.ledger.forSite(site)) if (!open.contains(entry.key)) entry];
        final home = documents == null
            ? null
            // The folder a session would get, without its own last part.
            : siteScratch(site, template: widget.template, documents: documents, id: '-').folder.replaceFirst(RegExp(r'[\\/]-$'), '');
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
                  onPressed: _busy || documents == null ? null : () => unawaited(_run(() => widget.onNew(site))),
                  child: const Text('New session'),
                ),
              ]),
              if (home != null)
                Padding(
                  padding: const EdgeInsets.only(top: 4),
                  child: Text('Each works in a folder of its own under $home', key: const Key('site-folder'), style: TextStyle(color: look.muted, fontSize: 11)),
                ),
              if (documents == null) ...[
                const SizedBox(height: 8),
                Text(
                  'Where is the Documents folder of the machine your agent runs on? A page cannot read it, and every session of a site gets a folder in it.',
                  style: TextStyle(color: look.muted, fontSize: 12),
                ),
                const SizedBox(height: 6),
                Row(children: [
                  Expanded(
                    child: TextField(
                      key: const Key('site-documents'),
                      controller: _root,
                      decoration: const InputDecoration(isDense: true, hintText: '/home/ana/Documents', border: OutlineInputBorder()),
                      onSubmitted: (_) => _saveRoot(),
                    ),
                  ),
                  const SizedBox(width: 8),
                  TextButton(key: const Key('site-documents-save'), onPressed: _saveRoot, child: const Text('Save')),
                ]),
              ],
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

  /// A Documents folder must be a path of the machine: absolute, as a drive or a share on Windows, else from `/`.
  void _saveRoot() {
    final text = _root.text.trim();
    if (!RegExp(r'^(?:/|[A-Za-z]:[\\/]|\\\\)').hasMatch(text) || text.contains('\u0000')) {
      setState(() => _failed = 'Give an absolute folder, like /home/ana/Documents or C:\\Work\\Documents');
      return;
    }
    setState(() => _failed = null);
    widget.documents.value = text.length > 1 ? text.replaceFirst(RegExp(r'[\\/]+$'), '') : text;
  }
}
