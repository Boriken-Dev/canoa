import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show SecretRequestCard;
import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../selectable.dart';
import 'store_update_all.dart';

/// Plans [candidates] together and shows every plan at once; on the person's yes the items are applied one after another, with
/// the count and each item's progress, and a summary that names each failure and offers to run only those again. Closes when the
/// person does; nothing is applied before they confirm.
Future<void> showUpdateAll(BuildContext context, List<UpdateCandidate> candidates) async {
  final run = UpdateAllRun(candidates);
  unawaited(run.plan());
  try {
    await showSelectableDialog<void>(context: context, barrierDismissible: false, builder: (_) => UpdateAllDialog(run: run));
  } finally {
    run.dispose();
  }
}

/// The dialog of one [UpdateAllRun], through its four phases: planning, the combined plan, running and the summary.
class UpdateAllDialog extends StatelessWidget {
  const UpdateAllDialog({super.key, required this.run});

  final UpdateAllRun run;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: run,
    builder: (context, _) {
      final text = AppText.of(context);
      final phase = run.phase;
      return AlertDialog(
        key: const Key('update-all'),
        title: Text(text('store.update_all')),
        content: SizedBox(
          width: 520,
          child: SingleChildScrollView(
            child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
              ..._head(context, text),
              for (final (at, entry) in run.entries.indexed) ...[
                if (_groupStarts(at)) Padding(
                  padding: const EdgeInsets.only(top: 12, bottom: 4),
                  child: Text(entry.candidate.group!, style: Theme.of(context).textTheme.titleSmall),
                ),
                _EntryRow(run: run, entry: entry, phase: phase),
              ],
              if (phase == UpdatePhase.running) _CurrentProgress(run: run),
            ]),
          ),
        ),
        actions: _actions(context, text),
      );
    },
  );

  bool _groupStarts(int at) {
    final group = run.entries[at].candidate.group;
    if (group == null) return false;
    if (run.entries.every((each) => each.candidate.group == group)) return false;
    return at == 0 || run.entries[at - 1].candidate.group != group;
  }

  List<Widget> _head(BuildContext context, AppText text) {
    final style = Theme.of(context).textTheme.bodyMedium;
    switch (run.phase) {
      case UpdatePhase.planning:
        return [
          const LinearProgressIndicator(key: Key('update-all-planning')),
          const SizedBox(height: 8),
          Text(text('store.update_all_planning'), style: style),
        ];
      case UpdatePhase.review:
        return [Text(text('store.update_all_review', {'count': run.ready.length}), key: const Key('update-all-review'), style: style)];
      case UpdatePhase.running:
        final value = run.total == 0 ? null : run.finished / run.total;
        return [
          LinearProgressIndicator(key: const Key('update-all-bar'), value: value),
          const SizedBox(height: 8),
          Text(
            run.stopping
                ? text('store.update_all_stopping')
                : text('store.update_all_running', {'done': (run.finished + 1).clamp(1, run.total), 'total': run.total}),
            key: const Key('update-all-count'),
            style: style,
          ),
        ];
      case UpdatePhase.ended:
        final failed = run.failed.length;
        final done = run.updated;
        return [
          Text(
            failed == 0 && run.notRun == 0
                ? text('store.update_all_summary_ok', {'done': done})
                : text('store.update_all_summary', {'done': done, 'failed': failed, 'skipped': run.notRun}),
            key: const Key('update-all-summary'),
            style: style,
          ),
        ];
    }
  }

  List<Widget> _actions(BuildContext context, AppText text) {
    switch (run.phase) {
      case UpdatePhase.planning:
        return [TextButton(key: const Key('update-all-cancel'), onPressed: () => Navigator.pop(context), child: Text(text('store.update_all_cancel')))];
      case UpdatePhase.review:
        return [
          TextButton(key: const Key('update-all-cancel'), onPressed: () => Navigator.pop(context), child: Text(text('store.update_all_cancel'))),
          FilledButton(
            key: const Key('update-all-confirm'),
            onPressed: run.ready.isEmpty ? null : () => unawaited(run.start()),
            child: Text(text('store.update_all_count', {'count': run.ready.length})),
          ),
        ];
      case UpdatePhase.running:
        return [
          TextButton(
            key: const Key('update-all-stop'),
            onPressed: run.stopping ? null : run.stop,
            child: Text(text('store.update_all_stop')),
          ),
        ];
      case UpdatePhase.ended:
        return [
          if (run.failed.isNotEmpty)
            OutlinedButton(
              key: const Key('update-all-retry'),
              onPressed: () => unawaited(run.retry()),
              child: Text(text('store.update_all_retry', {'count': run.failed.length})),
            ),
          FilledButton(key: const Key('update-all-close'), onPressed: () => Navigator.pop(context), child: Text(text('store.update_all_close'))),
        ];
    }
  }
}

/// One item: its name and versions, what its plan changes, what needs the person's eye, and where it stands once it runs.
class _EntryRow extends StatelessWidget {
  const _EntryRow({required this.run, required this.entry, required this.phase});

  final UpdateAllRun run;
  final UpdateEntry entry;
  final UpdatePhase phase;

  @override
  Widget build(BuildContext context) {
    final text = AppText.of(context);
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final id = entry.candidate.id;
    final item = entry.item;
    final from = entry.from, to = entry.to;
    final versions = to == null ? null : from == null ? '→ $to' : text('store.update_all_versions', {'from': from, 'to': to});
    final small = theme.textTheme.bodySmall;
    final planned = phase == UpdatePhase.review || phase == UpdatePhase.planning;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
        SizedBox(width: 24, child: _Mark(key: Key('update-all-status-$id'), entry: entry)),
        const SizedBox(width: 8),
        Expanded(
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Text(entry.candidate.title, key: Key('update-all-title-$id'), style: theme.textTheme.titleSmall),
            if (versions != null) Text(versions, key: Key('update-all-versions-$id'), style: small),
            if (item != null && planned) ...[
              for (final (at, line) in item.changes.indexed) Text(line, key: Key('update-all-change-$id-$at'), style: small),
              for (final (at, line) in item.attention.indexed)
                Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
                  Icon(Icons.warning_amber_rounded, size: 14, color: scheme.error),
                  const SizedBox(width: 4),
                  Expanded(child: Text(line, key: Key('update-all-attention-$id-$at'), style: small?.copyWith(color: scheme.error))),
                ]),
              if (item.details.isNotEmpty)
                Theme(
                  data: theme.copyWith(dividerColor: Colors.transparent),
                  child: ExpansionTile(
                    key: Key('update-all-details-$id'),
                    tilePadding: EdgeInsets.zero,
                    dense: true,
                    title: Text(text('store.update_all_details', {'count': item.details.length}), style: small),
                    expandedCrossAxisAlignment: CrossAxisAlignment.start,
                    children: [for (final line in item.details) Align(alignment: Alignment.centerLeft, child: Text(line, style: small?.copyWith(fontFamily: 'monospace')))],
                  ),
                ),
            ],
            if (entry.status == UpdateStatus.failed && entry.reason != null)
              Text(
                phase == UpdatePhase.review ? text('store.update_all_cannot_plan', {'why': entry.reason}) : entry.reason!,
                key: Key('update-all-reason-$id'),
                style: small?.copyWith(color: scheme.error),
              ),
            if (entry.status == UpdateStatus.notRun) Text(text('store.update_all_not_run'), key: Key('update-all-not-run-$id'), style: small),
          ]),
        ),
      ]),
    );
  }
}

class _Mark extends StatelessWidget {
  const _Mark({super.key, required this.entry});

  final UpdateEntry entry;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    if (entry.planning || entry.status == UpdateStatus.running) {
      return const Padding(padding: EdgeInsets.all(3), child: SizedBox(width: 16, height: 16, child: CircularProgressIndicator(strokeWidth: 2)));
    }
    return switch (entry.status) {
      UpdateStatus.done => Icon(Icons.check_circle, size: 20, color: scheme.primary),
      UpdateStatus.failed => Icon(Icons.error, size: 20, color: scheme.error),
      UpdateStatus.notRun => Icon(Icons.remove_circle_outline, size: 20, color: scheme.onSurfaceVariant),
      _ => Icon(Icons.radio_button_unchecked, size: 20, color: scheme.onSurfaceVariant),
    };
  }
}

/// What the item being applied says, its bar when a program gives a fraction, and the question it asks, sealed to this app's key.
class _CurrentProgress extends StatelessWidget {
  const _CurrentProgress({required this.run});

  final UpdateAllRun run;

  @override
  Widget build(BuildContext context) {
    final entry = run.current;
    if (entry == null) return const SizedBox.shrink();
    final text = AppText.of(context);
    final small = Theme.of(context).textTheme.bodySmall;
    final program = entry.program;
    final asking = program?.asking;
    return Padding(
      padding: const EdgeInsets.only(top: 8),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        if (program?.fraction case final fraction?) LinearProgressIndicator(key: const Key('update-all-item-bar'), value: fraction),
        ConstrainedBox(
          constraints: const BoxConstraints(maxHeight: 120),
          child: ListView(
            key: const Key('update-all-lines'),
            shrinkWrap: true,
            reverse: true,
            children: [for (final line in entry.lines.reversed) Text(line, style: small?.copyWith(fontFamily: 'monospace'))],
          ),
        ),
        if (asking != null && program != null) ...[
          Padding(
            padding: const EdgeInsets.only(top: 8, bottom: 4),
            child: Text(text('store.update_all_sealed', {'program': program.askedBy}), key: const Key('update-all-ask-note'), style: small),
          ),
          SecretRequestCard(
            key: ValueKey('update-all-ask ${asking.request.context.requestId}'),
            request: asking.request,
            environment: asking.environment,
            agent: program.askedBy,
            onReply: program.answer,
          ),
        ],
      ]),
    );
  }
}
