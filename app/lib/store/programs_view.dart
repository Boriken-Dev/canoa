import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show SecretRequestCard;
import 'package:flutter/material.dart';

import '../desktop/control_client.dart';
import '../forms/schema_fields.dart';
import '../selectable.dart';
import '../sources/source_icons.dart';
import 'programs.dart';
import 'store_cards.dart';
import 'store_view.dart';

// The Store's programs on screen: each extension's program with the version found and whether a newer one exists, and the
// buttons that plan, confirm, run and stream an install, update, roll back or removal. Everything shown is what the host
// answered; a refusal is shown in the host's words.

/// [bytes] as people read a size.
String sizeText(int bytes) {
  if (bytes < 1024) return '$bytes bytes';
  const units = ['KB', 'MB', 'GB'];
  var value = bytes / 1024;
  var unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return '${value.toStringAsFixed(value < 10 ? 1 : 0)} ${units[unit]}';
}

/// What the host said when it refused, or what went wrong reaching it.
String hostWords(Object error) => error is ControlFailed ? error.message : error is StateError ? error.message : '$error';

/// A line saying where a status stands: the version, whether the copy is Canoa's, and what the publisher says is newest.
List<String> statusLines(ProgramStatus status) {
  final copy = status.copy;
  final latest = status.latest;
  return [
    if (!status.reported && status.problem == null)
      'Looking…'
    else if (copy == null)
      'Not installed'
    else
      'Version ${copy.version ?? 'not read'} · ${copy.managed ? 'installed by Canoa' : 'found at ${copy.path}'}',
    if (copy != null && !copy.managed && copy.manager.isNotEmpty && copy.manager != 'UNKNOWN') 'Put there by ${copy.manager.toLowerCase()}',
    if (latest != null)
      if (!latest.known)
        'Newest version unknown${latest.unknownBecause == null ? '' : ': ${latest.unknownBecause}'}'
      else if (status.updateAvailable)
        'Update available: ${latest.version}'
      else if (copy != null && copy.version != null)
        'Up to date (${latest.version})'
      else
        'Newest version: ${latest.version}',
    if (copy?.update case UpdatedElsewhere(:final how)) 'Updated by the program itself, or by: $how',
    if (copy?.update case UpdatedNever(:final why)) 'Canoa cannot update it: $why',
  ];
}

/// The Store's programs, one tile each. Opening it asks the host for the copies and, at most once an hour, the publishers for
/// the newest versions; nothing is installed until a button is pressed and its plan confirmed.
class ProgramsSection extends StatefulWidget {
  const ProgramsSection({super.key, required this.programs});

  final ProgramStores programs;

  @override
  State<ProgramsSection> createState() => _ProgramsSectionState();
}

class _ProgramsSectionState extends State<ProgramsSection> {
  @override
  void initState() {
    super.initState();
    unawaited(widget.programs.refresh(latest: true));
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.programs,
    builder: (context, _) {
      final programs = widget.programs;
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Row(children: [
          Expanded(child: Text('Programs', style: Theme.of(context).textTheme.titleMedium)),
          IconButton(
            key: const Key('programs-refresh'),
            tooltip: 'Check for updates',
            onPressed: programs.loading ? null : () => unawaited(programs.refresh(latest: true, force: true)),
            icon: const Icon(Icons.refresh),
          ),
        ]),
        const Text('The agent programs Canoa can run, found by the Store. Nothing is installed or updated until you ask.'),
        if (programs.loading) const Padding(padding: EdgeInsets.only(top: 8), child: LinearProgressIndicator()),
        if (programs.problem case final problem?)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(problem, key: const Key('programs-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
          ),
        for (final status in programs.statuses) ProgramTile(programs: programs, status: status),
      ]);
    },
  );
}

/// One program: what is found, and the buttons that apply.
class ProgramTile extends StatelessWidget {
  const ProgramTile({super.key, required this.programs, required this.status});

  final ProgramStores programs;
  final ProgramStatus status;

  @override
  Widget build(BuildContext context) {
    final id = status.info.provider;
    final error = Theme.of(context).colorScheme.error;
    return Card(
      key: Key('program-$id'),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Row(children: [
            SourceMark.provider(
              firstMarkId([for (final each in status.info.serves) each.provider, id, status.info.program, status.info.title]),
              size: 24,
              key: Key('program-mark-$id'),
            ),
            const SizedBox(width: 8),
            Flexible(child: Text(status.info.title, style: Theme.of(context).textTheme.titleSmall)),
          ]),
          if (status.info.publisher.isNotEmpty) Text(status.info.publisher, style: Theme.of(context).textTheme.bodySmall),
          for (final line in statusLines(status)) Text(line, key: line.startsWith('Update available') ? Key('program-update-note-$id') : null),
          if (status.problem case final problem?) Text(problem, key: Key('program-problem-$id'), style: TextStyle(color: error)),
          const SizedBox(height: 8),
          ProgramButtons(programs: programs, status: status),
        ]),
      ),
    );
  }
}

/// The buttons that apply to a program as it stands: Install when nothing is found, Update when the publisher has a newer
/// version than Canoa's copy, Roll back and Remove for Canoa's own copy.
class ProgramButtons extends StatelessWidget {
  const ProgramButtons({super.key, required this.programs, required this.status});

  final ProgramStores programs;
  final ProgramStatus status;

  @override
  Widget build(BuildContext context) {
    final id = status.info.provider;
    final copy = status.copy;
    return Wrap(spacing: 8, runSpacing: 8, children: [
      if (copy == null && status.reported && status.problem == null)
        FilledButton(
          key: Key('program-install-$id'),
          onPressed: () => runProgramAction(context, programs, status, 'install'),
          child: const Text('Install'),
        ),
      if (copy != null && copy.managed && status.updateAvailable)
        FilledButton(
          key: Key('program-update-$id'),
          onPressed: () => runProgramAction(context, programs, status, 'update'),
          child: const Text('Update'),
        ),
      if (status.hasManaged)
        OutlinedButton(
          key: Key('program-rollback-$id'),
          onPressed: () => rollBack(context, programs, status),
          child: const Text('Roll back'),
        ),
      if (status.hasManaged)
        OutlinedButton(
          key: Key('program-remove-$id'),
          onPressed: () => runProgramAction(context, programs, status, 'uninstall'),
          child: const Text('Remove'),
        ),
    ]);
  }
}

/// Plans [action] (`install`, `update` or `uninstall`), shows the plan for the person to confirm, runs it with its progress
/// shown, and shows how it ended. A refusal is shown as the host words it.
///
/// [channel] names the release channel the plan is made for, and the result says whether the person confirmed and the program was
/// applied without failing, so a caller that goes on to use the program knows it is there.
Future<bool> runProgramAction(BuildContext context, ProgramStores programs, ProgramStatus status, String action, {String? channel}) async {
  final title = status.info.title;
  final ProgramPlan plan;
  try {
    plan = await programs.plan(status.info.provider, action, channel: channel);
  } on Object catch (error) {
    if (context.mounted) await _showError(context, _verb(action, title), hostWords(error));
    return false;
  }
  if (!context.mounted) return false;
  final confirmed = await showSelectableDialog<bool>(
    context: context,
    builder: (_) => PlanDialog(plan: plan, title: title),
  );
  if (confirmed != true || !context.mounted) return false;
  final run = programs.apply(plan);
  await showSelectableDialog<void>(
    context: context,
    barrierDismissible: false,
    builder: (_) => ProgressDialog(run: run, title: _verb(action, title)),
  );
  try {
    await run.done;
    return true;
  } on Object {
    return false;
  }
}

/// Installs the runtime [provider] that the host's Store lists for a program an overlay needs: the host's plan is shown for the
/// person to confirm, then run with its progress. Says so when the host lists no such runtime now. The result is whether it was
/// installed, so a caller that asks the host what the overlay needs again knows to.
Future<bool> installRuntime(BuildContext context, ProgramStores programs, String provider) async {
  try {
    await programs.ensure();
  } on Object catch (error) {
    if (context.mounted) await _showError(context, 'Install $provider', hostWords(error));
    return false;
  }
  final status = programs.status(provider);
  if (status == null) {
    if (context.mounted) await _showError(context, 'Install $provider', 'The host does not list this runtime now.');
    return false;
  }
  if (!context.mounted) return false;
  return runProgramAction(context, programs, status, status.hasManaged ? 'update' : 'install');
}

/// Puts the host's copy of [status]'s program back to the version before, after the person confirms.
Future<void> rollBack(BuildContext context, ProgramStores programs, ProgramStatus status) async {
  final title = status.info.title;
  final confirmed = await showSelectableDialog<bool>(
    context: context,
    builder: (dialog) => AlertDialog(
      key: const Key('rollback-dialog'),
      title: Text('Roll back $title'),
      content: Text('Canoa goes back to the version of $title it kept before this one. Nothing is downloaded.'),
      actions: [
        TextButton(onPressed: () => Navigator.pop(dialog, false), child: const Text('Cancel')),
        FilledButton(key: const Key('rollback-confirm'), onPressed: () => Navigator.pop(dialog, true), child: const Text('Roll back')),
      ],
    ),
  );
  if (confirmed != true || !context.mounted) return;
  try {
    final version = await programs.rollback(status.info.provider);
    if (context.mounted) {
      ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text('$title is at ${version ?? 'the version before'} again.')));
    }
  } on Object catch (error) {
    if (context.mounted) await _showError(context, 'Roll back $title', hostWords(error));
  }
}

String _verb(String action, String title) => switch (action) {
  'install' => 'Install $title',
  'update' => 'Update $title',
  'uninstall' => 'Remove $title',
  _ => '$action $title',
};

Future<void> _showError(BuildContext context, String title, String message) => showSelectableDialog<void>(
  context: context,
  builder: (dialog) => AlertDialog(
    key: const Key('program-error-dialog'),
    title: Text(title),
    content: Text(message, key: const Key('program-error')),
    actions: [TextButton(onPressed: () => Navigator.pop(dialog), child: const Text('Close'))],
  ),
);

/// What a plan will do, before it is confirmed: the version, the size, each file it fetches (one the publisher gives nothing to
/// check marked), the programs it runs, and everything it changes outside Canoa's folder. Pops true when the person confirms.
class PlanDialog extends StatelessWidget {
  const PlanDialog({super.key, required this.plan, required this.title});

  final ProgramPlan plan;
  final String title;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final warning = Theme.of(context).colorScheme.error;
    final remove = plan.action == 'uninstall';
    return AlertDialog(
      key: const Key('plan-dialog'),
      title: Text(_verb(plan.action, title)),
      content: SizedBox(
        width: 480,
        child: SingleChildScrollView(
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
            if (plan.version != null)
              Text(
                plan.fromVersion != null && plan.fromVersion != plan.version
                    ? 'Version ${plan.fromVersion} to ${plan.version}'
                    : 'Version ${plan.version}',
                key: const Key('plan-version'),
              ),
            if (plan.bytes != null) Text('Download: ${sizeText(plan.bytes!)}', key: const Key('plan-size')),
            if (plan.downloads.isNotEmpty) ...[
              const SizedBox(height: 12),
              Text('It downloads', style: text.titleSmall),
              for (final (at, each) in plan.downloads.indexed)
                Padding(
                  padding: const EdgeInsets.only(top: 4),
                  child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                    Text(each.url, style: text.bodySmall),
                    if (each.verified)
                      Text('Checked against the publisher\'s ${each.how}${each.bytes == null ? '' : ' · ${sizeText(each.bytes!)}'}',
                          style: text.bodySmall)
                    else
                      Text(
                        'Unverified: ${each.how.isEmpty ? 'the publisher gives nothing to check it against' : each.how}',
                        key: Key('plan-unverified-$at'),
                        style: text.bodySmall?.copyWith(color: warning, fontWeight: FontWeight.w600),
                      ),
                  ]),
                ),
            ],
            if (plan.steps.isNotEmpty) ...[
              const SizedBox(height: 12),
              Text('It runs', style: text.titleSmall),
              for (final (at, step) in plan.steps.indexed) Text('• $step', key: Key('plan-step-$at')),
            ],
            const SizedBox(height: 12),
            Text(remove ? 'It undoes' : 'It changes outside Canoa\'s folder', style: text.titleSmall),
            if (plan.changes.isEmpty)
              const Text('Nothing: no PATH entry, registry key or profile file.', key: Key('plan-no-changes'))
            else
              for (final (at, change) in plan.changes.indexed)
                Text('• ${change.kind}: ${change.target}\n  ${change.detail}${change.undone ? ' (undone by removing it)' : ''}',
                    key: Key('plan-change-$at')),
            for (final (at, notice) in plan.notices.indexed) ...[
              const SizedBox(height: 8),
              Text(notice, key: Key('plan-notice-$at')),
            ],
          ]),
        ),
      ),
      actions: [
        TextButton(key: const Key('plan-cancel'), onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
        FilledButton(
          key: const Key('plan-confirm'),
          onPressed: () => Navigator.pop(context, true),
          child: Text(switch (plan.action) { 'install' => 'Install', 'update' => 'Update', _ => 'Remove' }),
        ),
      ],
    );
  }
}

/// A running install: its lines, a bar when the host gives a fraction, a question when it asks one, and Cancel, which closes
/// the link and so stops it on the host. When it ends, how it ended, in the host's words.
class ProgressDialog extends StatefulWidget {
  const ProgressDialog({super.key, required this.run, required this.title});

  final ProgramRun run;
  final String title;

  @override
  State<ProgressDialog> createState() => _ProgressDialogState();
}

class _ProgressDialogState extends State<ProgressDialog> {
  Map<String, Object?>? _result;
  Object? _failure;
  final _scroll = ScrollController();

  @override
  void initState() {
    super.initState();
    widget.run.addListener(_changed);
    widget.run.done.then((result) {
      if (mounted) setState(() => _result = result);
    }, onError: (Object error) {
      if (mounted) setState(() => _failure = error);
    });
    WidgetsBinding.instance.addPostFrameCallback((_) => _changed());
  }

  @override
  void dispose() {
    widget.run.removeListener(_changed);
    _scroll.dispose();
    super.dispose();
  }

  void _changed() {
    if (!mounted) return;
    setState(() {});
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_scroll.hasClients) _scroll.jumpTo(_scroll.position.maxScrollExtent);
    });
  }

  @override
  Widget build(BuildContext context) {
    final run = widget.run;
    final ended = _result != null || _failure != null;
    final cancelled = _failure is ProgramCancelled;
    final text = Theme.of(context).textTheme;
    return AlertDialog(
      key: const Key('program-progress'),
      title: Text(widget.title),
      content: SizedBox(
        width: 480,
        child: SingleChildScrollView(child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
          if (!ended) LinearProgressIndicator(key: const Key('program-bar'), value: run.fraction),
          const SizedBox(height: 8),
          ConstrainedBox(
            constraints: const BoxConstraints(maxHeight: 220),
            child: ListView(
              key: const Key('program-lines'),
              controller: _scroll,
              shrinkWrap: true,
              children: [for (final line in run.lines) Text(line, style: text.bodySmall?.copyWith(fontFamily: 'monospace'))],
            ),
          ),
          // A question the install asks is a form the host sealed to this app's key: what is typed goes to the host and nowhere else.
          if (run.asking case final asking?) ...[
            Padding(
              padding: const EdgeInsets.only(top: 8, bottom: 4),
              child: Text(
                'What you send is sealed so only Canoa\'s host can read it, and the host gives it to the install of ${run.askedBy}.',
                key: const Key('program-ask-note'),
                style: text.bodySmall,
              ),
            ),
            SecretRequestCard(
              key: ValueKey('program-ask ${asking.request.context.requestId}'),
              request: asking.request,
              environment: asking.environment,
              agent: run.askedBy,
              onReply: run.answer,
            ),
          ],
          if (_result case final result?) ...[
            const SizedBox(height: 8),
            Text(
              result['applied'] == false
                  ? 'Nothing to do: it is already at the newest version.'
                  : result['version'] is String
                      ? 'Done: version ${result['version']}.'
                      : 'Done.',
              key: const Key('program-done'),
            ),
          ],
          if (_failure != null) ...[
            const SizedBox(height: 8),
            Text(
              cancelled ? 'Cancelled. Nothing was kept of it.' : hostWords(_failure!),
              key: Key(cancelled ? 'program-cancelled' : 'program-error'),
              style: cancelled ? null : TextStyle(color: Theme.of(context).colorScheme.error),
            ),
          ],
        ])),
      ),
      actions: [
        if (!ended)
          TextButton(key: const Key('program-cancel'), onPressed: () => unawaited(run.cancel()), child: const Text('Cancel'))
        else
          FilledButton(key: const Key('program-close'), onPressed: () => Navigator.pop(context), child: const Text('Close')),
      ],
    );
  }
}

/// A question put to the person in a dialog, as a form from the schema it gave, for the plain inputs a sign-in takes before it runs.
/// Drawn by [SchemaFields], so it has what every form has (sections, conditions, units, suggestions) and is checked by the same
/// rules. Pops the values and the secrets, or null.
class AskDialog extends StatefulWidget {
  const AskDialog({super.key, required this.question});

  final ProgramAsk question;

  @override
  State<AskDialog> createState() => _AskDialogState();
}

class _AskDialogState extends State<AskDialog> {
  late Map<String, Object?> _values = {
    for (final MapEntry(:key, :value) in widget.question.fields.entries)
      if (value['default'] != null) key: value['default'],
  };
  String? _problem;

  void _submit() {
    final schema = widget.question.schema;
    final why = schemaProblem(schema, const {}, _values);
    if (why != null) {
      setState(() => _problem = why);
      return;
    }
    final values = <String, Object?>{}, secrets = <String, Object?>{};
    final fields = widget.question.fields;
    for (final MapEntry(:key, :value) in collectSchemaValues(schema, const {}, _values).entries) {
      (ProgramAsk.sensitive(fields[key] ?? const {}) ? secrets : values)[key] = value;
    }
    Navigator.pop(context, (values: values, secrets: secrets));
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    key: const Key('program-ask'),
    title: Text(widget.question.title.isEmpty ? 'The install asks' : widget.question.title),
    content: SizedBox(
      width: 420,
      child: SingleChildScrollView(
        child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          SchemaFields(
            schema: widget.question.schema,
            form: const {},
            values: _values,
            idPrefix: 'ask',
            onSubmitted: _submit,
            onValues: (held) => setState(() {
              _values = held;
              _problem = null;
            }),
          ),
          if (_problem != null)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(_problem!, key: const Key('ask-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
            ),
        ]),
      ),
    ),
    actions: [
      TextButton(key: const Key('ask-cancel'), onPressed: () => Navigator.pop(context), child: const Text('Cancel the install')),
      FilledButton(key: const Key('ask-submit'), onPressed: _submit, child: const Text('Answer')),
    ],
  );
}

/// The Agents type of the Store: the programs the host can run, those found on it under Installed and the others under
/// Discover, with the same tiles and buttons as the programs list. Opening it asks the host for the copies and, at most once an
/// hour, the publishers for the newest versions.
class AgentsBody extends StatefulWidget {
  const AgentsBody({super.key, required this.programs, required this.view});

  final ProgramStores programs;
  final StoreView view;

  @override
  State<AgentsBody> createState() => _AgentsBodyState();
}

class _AgentsBodyState extends State<AgentsBody> with AutomaticKeepAliveClientMixin {
  @override
  bool get wantKeepAlive => true;

  @override
  void initState() {
    super.initState();
    unawaited(widget.programs.refresh(latest: true));
  }

  @override
  Widget build(BuildContext context) {
    super.build(context);
    return ListenableBuilder(
      listenable: widget.programs,
      builder: (context, _) {
        final programs = widget.programs;
        final view = widget.view;
        final shown = [
          for (final status in programs.statuses)
            if (status.installed != view.discover &&
                (view.discover || !view.updatesOnly || status.updateAvailable) &&
                view.matches('${status.info.title}\n${status.info.publisher}\n${status.info.provider}'))
              status,
        ];
        return CardList(
          header: [
            Text(
              view.discover
                  ? 'The agent programs Canoa can run and that are not on this computer yet. Nothing is installed until you ask.'
                  : 'The agent programs found on this computer. Nothing is updated until you ask.',
            ),
            if (programs.loading) const Padding(padding: EdgeInsets.only(top: 8), child: LinearProgressIndicator()),
            if (programs.problem case final problem?)
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Text(problem, key: const Key('programs-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
              ),
            const SizedBox(height: 8),
            if (!programs.loading && shown.isEmpty)
              Text(view.discover ? 'Every program is installed.' : 'No program is installed.', key: const Key('agents-nothing')),
          ],
          cards: [for (final status in shown) ProgramTile(programs: programs, status: status)],
        );
      },
    );
  }
}
