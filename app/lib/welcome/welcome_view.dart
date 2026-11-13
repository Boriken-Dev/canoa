import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLookOf;
import 'package:flutter/material.dart';

import '../desktop/control_client.dart';
import '../history/session_import.dart';
import '../sources/source_icons.dart';
import '../sources/sources.dart';
import '../store/programs.dart';
import '../store/programs_view.dart';
import '../theme/canoa_mark.dart';

/// Earlier sessions of the person's agents that Canoa could bring in, offered on the first screen. [from] names the sources
/// whose history it would read; nothing is read until [list] is called, which the person asks for with Import. [list] gives
/// each session's title and where it ran, never what was said, and [bring] brings in the ones the person chose.
final class HistoryImport {
  const HistoryImport({required this.from, required this.list, required this.bring});

  /// The sources' names, for the offer's text.
  final List<String> from;

  /// Looks in the sources' folders for sessions.
  final Future<List<ImportChoice>> Function() list;

  /// Brings [chosen] in; what came and what did not.
  final Future<ImportOutcome> Function(List<ImportChoice> chosen) bring;
}

/// What a window with no session shows: each agent source with its readiness (what it needs when it is not ready, and the
/// Store's Install where its program is missing), a way to start, and the offer to import earlier sessions.
class WelcomeView extends StatefulWidget {
  const WelcomeView({
    super.key,
    required this.sources,
    required this.checks,
    required this.onNewSession,
    this.programs,
    this.onSettings,
    this.historyImport,
  });

  final List<AgentSource> sources;

  /// What each source needs, as last checked; a source not in it is still being looked at.
  final Map<AgentSource, SourceCheck> checks;
  final VoidCallback onNewSession;

  /// The Store's programs, for a source's Install and its version.
  final ProgramStores? programs;

  /// Opens Settings at its sources, where a source is added or hidden.
  final VoidCallback? onSettings;

  /// The import of earlier sessions, when this app can do one; without it the page makes no offer.
  final HistoryImport? historyImport;

  @override
  State<WelcomeView> createState() => _WelcomeViewState();
}

class _WelcomeViewState extends State<WelcomeView> {
  bool _importing = false;
  String? _imported;
  String? _importFailed;

  /// What the host found after the person said yes, and which of it they chose; null until then.
  List<ImportChoice>? _found;
  final Set<ImportChoice> _chosen = {};
  List<String> _notes = const [];

  /// Looks for sessions, which is the first time anything is read: the person said yes by pressing Import.
  Future<void> _look(HistoryImport offer) async {
    setState(() {
      _importing = true;
      _importFailed = null;
    });
    try {
      final found = await offer.list();
      if (!mounted) return;
      setState(() {
        _found = found;
        _chosen
          ..clear()
          ..addAll(found.where((each) => !each.imported));
      });
    } on Object catch (error) {
      if (mounted) setState(() => _importFailed = 'Could not look: ${_why(error)}');
    } finally {
      if (mounted) setState(() => _importing = false);
    }
  }

  Future<void> _import(HistoryImport offer) async {
    setState(() {
      _importing = true;
      _importFailed = null;
    });
    try {
      final done = await offer.bring(_chosen.toList());
      if (!mounted) return;
      setState(() {
        _notes = done.notes;
        _found = null;
        _imported = done.imported == 1 ? 'Imported 1 session.' : 'Imported ${done.imported} sessions.';
      });
    } on Object catch (error) {
      if (mounted) setState(() => _importFailed = 'Could not import: ${_why(error)}');
    } finally {
      if (mounted) setState(() => _importing = false);
    }
  }

  static String _why(Object error) => error is ControlFailed ? error.message : '$error';

  /// The sessions the host found, by source, each with a box: what Import would bring in is what is ticked.
  Widget _choices(BuildContext context, HistoryImport offer, List<ImportChoice> found) {
    final muted = context.chatLook.muted;
    if (found.isEmpty) {
      return Text('No earlier sessions were found.', key: const Key('welcome-import-none'), style: TextStyle(color: muted));
    }
    final sources = <String>[];
    for (final each in found) {
      if (!sources.contains(each.source)) sources.add(each.source);
    }
    final fresh = found.where((each) => !each.imported).toList();
    return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      ConstrainedBox(
        constraints: const BoxConstraints(maxHeight: 280),
        child: ListView(shrinkWrap: true, children: [
          for (final source in sources) ...[
            Padding(
              padding: const EdgeInsets.only(top: 6, bottom: 2),
              child: Text(
                '$source: ${found.where((each) => each.source == source).length} sessions',
                key: Key('welcome-import-source $source'),
                style: Theme.of(context).textTheme.labelLarge,
              ),
            ),
            for (final each in found.where((each) => each.source == source))
              CheckboxListTile(
                key: Key('welcome-import-pick ${each.id}'),
                dense: true,
                contentPadding: EdgeInsets.zero,
                controlAffinity: ListTileControlAffinity.leading,
                value: each.imported || _chosen.contains(each),
                onChanged: each.imported || _importing
                    ? null
                    : (on) => setState(() => on == true ? _chosen.add(each) : _chosen.remove(each)),
                title: Text(each.title, maxLines: 1, overflow: TextOverflow.ellipsis),
                subtitle: Text(
                  [if (each.imported) 'Already imported', ?each.cwd].join(' · '),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(color: muted),
                ),
              ),
          ],
        ]),
      ),
      if (fresh.isEmpty)
        Text('Everything found is already imported.', style: TextStyle(color: muted))
      else
        Padding(
          padding: const EdgeInsets.only(top: 8),
          child: OutlinedButton(
            key: const Key('welcome-import-run'),
            onPressed: _importing || _chosen.isEmpty ? null : () => unawaited(_import(offer)),
            child: Text(_importing ? 'Importing…' : 'Import ${_chosen.length}'),
          ),
        ),
    ]);
  }

  Widget _readiness(BuildContext context, AgentSource source) {
    final theme = Theme.of(context);
    final muted = context.chatLook.muted;
    final check = widget.checks[source];
    if (check == null) return Text('Looking…', style: TextStyle(color: muted));
    final programs = widget.programs;
    final status = check.store == null ? null : programs?.status(check.store!);
    if (programs != null && status != null && status.reported && status.problem == null && status.copy == null) {
      return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text('Not installed: ${status.info.title} is not on this computer.', style: TextStyle(color: theme.colorScheme.error)),
        const SizedBox(height: 6),
        ProgramButtons(programs: programs, status: status),
      ]);
    }
    final problem = check.describe(source.label);
    if (problem != null) return Text(problem, style: TextStyle(color: theme.colorScheme.error));
    final version = check.version ?? status?.copy?.version;
    return Row(mainAxisSize: MainAxisSize.min, children: [
      Icon(Icons.check_rounded, size: 18, color: context.chatLook.accent),
      const SizedBox(width: 6),
      Text('Ready', style: TextStyle(color: context.chatLook.accent, fontWeight: FontWeight.w600)),
      if (version != null) Text('  ·  $version', style: TextStyle(color: muted)),
    ]);
  }

  Widget _source(BuildContext context, AgentSource source) {
    final theme = Theme.of(context);
    return Padding(
      key: Key('welcome-source ${source.label}'),
      padding: const EdgeInsets.symmetric(vertical: 10),
      child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Padding(padding: const EdgeInsets.only(top: 2), child: SourceMark.of(source, size: 24)),
        const SizedBox(width: 14),
        Expanded(
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Text(source.displayName, style: theme.textTheme.titleMedium),
            const SizedBox(height: 4),
            _readiness(context, source),
          ]),
        ),
      ]),
    );
  }

  Widget _offer(BuildContext context, HistoryImport offer) {
    final theme = Theme.of(context);
    final muted = context.chatLook.muted;
    final names = offer.from.length < 2
        ? offer.from.join()
        : '${offer.from.sublist(0, offer.from.length - 1).join(', ')} and ${offer.from.last}';
    return Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
      Padding(padding: const EdgeInsets.only(top: 2), child: Icon(Icons.history, size: 24, color: muted)),
      const SizedBox(width: 14),
      Expanded(
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Text('Import earlier sessions', style: theme.textTheme.titleMedium),
          const SizedBox(height: 4),
          Text(
            _imported ??
                (_found != null
                    ? 'Choose the sessions to bring in. Their files are only read.'
                    : 'Bring in your $names sessions. Nothing is read until you choose Import.'),
            key: const Key('welcome-import-text'),
            style: TextStyle(color: muted),
          ),
          if (_importFailed case final failed?)
            Text(failed, style: TextStyle(color: theme.colorScheme.error)),
          if (_imported == null && _found == null)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: OutlinedButton(
                key: const Key('welcome-import'),
                onPressed: _importing ? null : () => unawaited(_look(offer)),
                child: Text(_importing ? 'Looking…' : 'Import'),
              ),
            ),
          if (_found case final found?) Padding(padding: const EdgeInsets.only(top: 8), child: _choices(context, offer, found)),
          for (final note in _notes)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Text(note, key: const Key('welcome-import-note'), style: TextStyle(color: muted)),
            ),
        ]),
      ),
    ]);
  }

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final theme = Theme.of(context);
    final offer = widget.historyImport;
    final sources = Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      if (widget.sources.isEmpty)
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 10),
          child: Text('No agent source is running here. Add one in Settings, under Sources, or connect to an agent on another computer.',
              key: const Key('welcome-no-sources'), style: TextStyle(color: look.muted)),
        ),
      for (final source in widget.sources) _source(context, source),
    ]);
    return Center(
      key: const Key('welcome'),
      child: SingleChildScrollView(
        padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 28),
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: look.messageWidth),
          child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            Row(children: [
              CanoaMark(size: 40, color: look.accent),
              const SizedBox(width: 14),
              Expanded(child: Text('Welcome to Canoa', style: theme.textTheme.headlineSmall)),
            ]),
            const SizedBox(height: 8),
            Text('Your agents, in one place. These are the ones Canoa found on this computer.',
                style: TextStyle(color: look.muted)),
            const SizedBox(height: 20),
            const Divider(height: 1),
            sources,
            const Divider(height: 1),
            if (offer != null) ...[
              Padding(padding: const EdgeInsets.symmetric(vertical: 14), child: _offer(context, offer)),
              const Divider(height: 1),
            ],
            const SizedBox(height: 20),
            Wrap(spacing: 12, runSpacing: 8, children: [
              FilledButton.icon(
                key: const Key('welcome-new-session'),
                onPressed: widget.onNewSession,
                icon: const Icon(Icons.add_rounded),
                label: const Text('New session'),
              ),
              if (widget.onSettings != null)
                TextButton(key: const Key('welcome-sources'), onPressed: widget.onSettings, child: const Text('Manage sources')),
            ]),
          ]),
        ),
      ),
    );
  }
}
