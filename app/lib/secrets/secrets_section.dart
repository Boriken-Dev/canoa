import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show SecretRequestCard;
import 'package:flutter/material.dart';

import 'provider_setup.dart';
import 'secrets_controller.dart';
import 'secrets_host.dart';

/// What the row says of how long it is kept.
String secretKeptText(SecretRow row) {
  final left = row.expiresIn;
  return switch (row.kept) {
    'stored' => 'Stored',
    'session' => 'This conversation',
    _ when left != null => '${_span(left)} left',
    _ => 'Until Canoa closes',
  };
}

String _span(int seconds) {
  if (seconds >= 3600) return '${(seconds / 3600).ceil()} h';
  if (seconds >= 60) return '${(seconds / 60).ceil()} min';
  return '$seconds s';
}

/// The Secrets section's page: the vault the index chose (or all of them), with what the host holds for the agents (where each is kept, what it is called, how long it has left), with the value
/// hidden until the person asks to see it, forget, and a way to set one by hand. A list never holds a value.
class SecretsPage extends StatefulWidget {
  const SecretsPage({super.key, required this.controller, this.refreshEvery = const Duration(seconds: 30)});

  final SecretsController controller;

  /// How often the table is read again so the time left stays true; null reads it only when asked.
  final Duration? refreshEvery;

  @override
  State<SecretsPage> createState() => _SecretsPageState();
}

class _SecretsPageState extends State<SecretsPage> {
  Timer? _timer;

  SecretsController get _secrets => widget.controller;

  @override
  void initState() {
    super.initState();
    _secrets.addListener(_changed);
    WidgetsBinding.instance.addPostFrameCallback((_) => unawaited(_secrets.refresh()));
    final every = widget.refreshEvery;
    if (every != null) _timer = Timer.periodic(every, (_) => unawaited(_secrets.refresh()));
  }

  @override
  void dispose() {
    _timer?.cancel();
    _secrets.removeListener(_changed);
    super.dispose();
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  Future<void> _forget(SecretRow row) async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        key: const Key('secret-forget-confirm'),
        title: Text('Forget ${row.name}?'),
        content: Text(
          row.kept == 'stored'
              ? '${row.name} is taken out of the ${row.provider} vault. An agent that asks for it again will ask you.'
              : 'Canoa stops holding ${row.name}. An agent that asks for it again will ask you.',
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Keep it')),
          FilledButton(key: const Key('secret-forget-yes'), onPressed: () => Navigator.pop(context, true), child: const Text('Forget it')),
        ],
      ),
    );
    if (sure ?? false) await _secrets.forget(row);
  }

  Future<void> _editProvider(SecretEntry entry) async {
    await showSecretProviderSetup(context, _secrets, editing: entry);
    if (mounted) unawaited(_secrets.refresh());
  }

  Future<void> _removeProvider(SecretEntry entry) async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        key: const Key('secret-provider-remove-confirm'),
        title: Text('Remove ${entry.name}?'),
        content: const Text('Canoa stops using this setup. What it kept for you in its own store stays there.'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Keep it')),
          FilledButton(key: const Key('secret-provider-remove-yes'), onPressed: () => Navigator.pop(context, true), child: const Text('Remove it')),
        ],
      ),
    );
    if (sure ?? false) await _secrets.removeEntry(entry.name);
  }

  /// What the chosen vault is: what it can do, its settings and, for one the person set up, a way to change or take it away.
  Widget _vault(SecretEntry entry, ThemeData theme, ColorScheme scheme) {
    final info = _secrets.infoOf(entry);
    final can = [if (entry.canStore) 'keeps what you give it', if (entry.canList) 'lists what it holds', if (entry.needsPerson) 'asks you for a value'];
    final settings = <String>[
      for (final each in entry.settings.entries) '${each.key}: ${each.value}',
      for (final each in entry.from.entries) '${each.key}: the secret ${each.value.name} of ${each.value.entry}',
    ];
    return Padding(
      key: Key('secrets-vault ${entry.name}'),
      padding: const EdgeInsets.only(bottom: 12),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text('${entry.title}${can.isEmpty ? '' : ': ${can.join(', ')}'}', style: theme.textTheme.bodyMedium),
        if (info?.description case final description?) Text(description, style: theme.textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant)),
        const SizedBox(height: 8),
        for (final line in settings) Text(line, key: Key('secrets-vault-setting ${entry.name} $line'), style: theme.textTheme.bodySmall),
        if (entry.setUp)
          Wrap(spacing: 8, children: [
            if (info?.hasSettings ?? true)
              OutlinedButton.icon(key: Key('secret-provider ${entry.name}'), onPressed: () => unawaited(_editProvider(entry)), icon: const Icon(Icons.tune, size: 18), label: const Text('Change its settings')),
            TextButton.icon(key: Key('secret-vault-remove ${entry.name}'), onPressed: () => unawaited(_removeProvider(entry)), icon: Icon(Icons.delete_outline, size: 18, color: scheme.error), label: const Text('Remove this vault')),
          ])
        else
          Text('Built in: nothing to set up.', style: theme.textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant)),
      ]),
    );
  }

  Future<void> _set() async {
    final chosen = _secrets.chosen;
    await showDialog<void>(context: context, builder: (_) => SetSecretDialog(controller: _secrets, initialEntry: chosen != null && chosen.acceptsValues ? chosen.name : null));
    if (mounted) unawaited(_secrets.refresh());
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final chosen = _secrets.chosen;
    final rows = _secrets.visible;
    return Padding(
      key: const Key('secrets-page'),
      padding: const EdgeInsets.all(16),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Row(children: [
          Expanded(child: Text(chosen?.name ?? 'All vaults', key: const Key('secrets-title'), style: theme.textTheme.titleLarge)),
          IconButton(key: const Key('secrets-refresh'), tooltip: 'Read the list again', icon: const Icon(Icons.refresh), onPressed: () => unawaited(_secrets.refresh())),
          if (chosen == null || chosen.acceptsValues) FilledButton.icon(key: const Key('secret-set'), onPressed: _set, icon: const Icon(Icons.add), label: const Text('Set a secret')),
        ]),
        const SizedBox(height: 4),
        Text(
          'What the host holds for your agents. An agent gets a handle to a secret, never the value, and using one still asks you as any command does.',
          style: theme.textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant),
        ),
        const SizedBox(height: 12),
        if (chosen != null) _vault(chosen, theme, scheme),
        if (_secrets.problem case final problem?) Padding(padding: const EdgeInsets.only(bottom: 8), child: SelectableText(problem, key: const Key('secrets-problem'), style: TextStyle(color: scheme.error))),
        if (_secrets.loading && rows.isEmpty) const LinearProgressIndicator(),
        if (!_secrets.loading && rows.isEmpty && _secrets.problem == null)
          Text(
            chosen == null ? 'Nothing is kept. A secret an agent asks you for and you choose to keep appears here.' : 'Nothing is kept in this vault.',
            key: const Key('secrets-empty'),
          ),
        if (rows.isNotEmpty)
          Expanded(
            child: SingleChildScrollView(
              child: SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                child: DataTable(
                  key: const Key('secrets-table'),
                  columnSpacing: 20,
                  columns: [
                    if (chosen == null) const DataColumn(label: Text('Vault')),
                    const DataColumn(label: Text('Name')),
                    const DataColumn(label: Text('Value')),
                    const DataColumn(label: Text('Kept')),
                    const DataColumn(label: Text('')),
                  ],
                  rows: [for (final row in rows) _row(row, scheme, showVault: chosen == null)],
                ),
              ),
            ),
          ),
      ]),
    );
  }

  DataRow _row(SecretRow row, ColorScheme scheme, {required bool showVault}) {
    final value = _secrets.shown(row);
    return DataRow(key: ValueKey('secret ${row.key}'), cells: [
      if (showVault) DataCell(Text(row.provider)),
      DataCell(Text(row.name, key: Key('secret-name ${row.key}'))),
      DataCell(Row(mainAxisSize: MainAxisSize.min, children: [
        ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 280),
          child: value == null
              ? const Text('••••••••', key: Key('secret-hidden'))
              : SelectableText(value, key: Key('secret-value ${row.key}'), maxLines: 1, style: const TextStyle(fontFamily: 'monospace')),
        ),
        IconButton(
          key: Key('secret-show ${row.key}'),
          tooltip: value == null ? 'Show the value' : 'Hide the value',
          icon: Icon(value == null ? Icons.visibility_outlined : Icons.visibility_off_outlined, size: 18),
          onPressed: () => unawaited(_secrets.toggle(row)),
        ),
      ])),
      DataCell(Text(secretKeptText(row), key: Key('secret-kept ${row.key}'))),
      DataCell(IconButton(
        key: Key('secret-forget ${row.key}'),
        tooltip: 'Forget it',
        icon: Icon(Icons.delete_outline, size: 18, color: scheme.error),
        onPressed: () => unawaited(_forget(row)),
      )),
    ]);
  }
}

/// Sets a secret by hand: which vault and what name, then the host's own form, sealed, for the value.
class SetSecretDialog extends StatefulWidget {
  const SetSecretDialog({super.key, required this.controller, this.initialEntry, this.initialName});

  final SecretsController controller;

  /// The entry and the name to start from, for a secret another setup is waiting for.
  final String? initialEntry, initialName;

  @override
  State<SetSecretDialog> createState() => _SetSecretDialogState();
}

class _SetSecretDialogState extends State<SetSecretDialog> {
  late final _name = TextEditingController(text: widget.initialName ?? '');
  late String? _entry = widget.initialEntry;
  SecretSet? _run;

  List<SecretEntry> get _entries => widget.controller.settable;

  @override
  void dispose() {
    _name.dispose();
    final run = _run;
    if (run != null && run.running) run.cancel();
    super.dispose();
  }

  void _start() {
    final entry = _entry ?? (_entries.isEmpty ? null : _entries.first.name);
    final name = _name.text.trim();
    if (entry == null || name.isEmpty) return;
    final run = widget.controller.set(provider: entry, name: name);
    run.addListener(() {
      if (mounted) setState(() {});
    });
    setState(() => _run = run);
  }

  @override
  Widget build(BuildContext context) {
    final run = _run;
    return AlertDialog(
      key: const Key('secret-set-dialog'),
      title: const Text('Set a secret'),
      content: SizedBox(
        width: 420,
        child: run == null ? _choose() : _asking(run),
      ),
      actions: [
        if (run == null || run.running)
          TextButton(
            key: const Key('secret-set-cancel'),
            onPressed: () {
              run?.cancel();
              Navigator.pop(context);
            },
            child: const Text('Cancel'),
          ),
        if (run == null) FilledButton(key: const Key('secret-set-continue'), onPressed: _start, child: const Text('Continue')),
        if (run != null && !run.running) FilledButton(key: const Key('secret-set-close'), onPressed: () => Navigator.pop(context), child: const Text('Close')),
      ],
    );
  }

  Widget _choose() {
    final entries = _entries;
    final chosen = _entry ?? (entries.isEmpty ? null : entries.first.name);
    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      DropdownButtonFormField<String>(
        key: const Key('secret-set-provider'),
        initialValue: chosen,
        decoration: const InputDecoration(labelText: 'Vault'),
        items: [for (final entry in entries) DropdownMenuItem(value: entry.name, child: Text('${entry.name} (${entry.title})'))],
        onChanged: (value) => setState(() => _entry = value),
      ),
      const SizedBox(height: 8),
      TextField(key: const Key('secret-set-name'), controller: _name, decoration: const InputDecoration(labelText: 'Name', helperText: 'What an agent asks for it by')),
      const SizedBox(height: 8),
      Text(
        'The value is typed in the next step, in a form only this app and the host can read.',
        style: Theme.of(context).textTheme.bodySmall,
      ),
    ]);
  }

  Widget _asking(SecretSet run) {
    final asking = run.asking;
    final scheme = Theme.of(context).colorScheme;
    return SingleChildScrollView(
      child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        if (asking != null)
          SecretRequestCard(
            key: ValueKey('secret-set-ask ${asking.request.context.requestId}'),
            request: asking.request,
            environment: asking.environment,
            agent: 'Canoa',
            onReply: run.answer,
          ),
        if (run.running && asking == null) const Wrap(crossAxisAlignment: WrapCrossAlignment.center, children: [SizedBox(width: 16, height: 16, child: CircularProgressIndicator(strokeWidth: 2)), SizedBox(width: 12), Text('Waiting for the host')]),
        if (run.done) const SelectableText('It is set.', key: Key('secret-set-done')),
        if (run.cancelled) const SelectableText('Nothing was set.', key: Key('secret-set-cancelled')),
        if (run.problem case final problem?) SelectableText('It was not set: $problem', key: const Key('secret-set-failed'), style: TextStyle(color: scheme.error)),
      ]),
    );
  }
}
