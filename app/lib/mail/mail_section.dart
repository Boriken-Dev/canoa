import 'dart:async';

import 'package:flutter/material.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../forms/schema_fields.dart';
import '../settings/settings_row.dart';
import 'mail_accounts.dart';

/// Settings › Mail: the mailboxes an agent can use as a source. Each row says where the mailbox is and whether its credential is
/// stored, and can test sign-in or remove the account. Any secret is sent once to the host store and shown nowhere again.
class MailSection extends StatefulWidget {
  const MailSection({super.key, required this.controller});

  final MailAccountsController controller;

  @override
  State<MailSection> createState() => _MailSectionState();
}

class _MailSectionState extends State<MailSection> {
  MailAccountsController get _mail => widget.controller;

  @override
  void initState() {
    super.initState();
    unawaited(_mail.refresh());
  }

  @override
  void didUpdateWidget(MailSection oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.controller != widget.controller) unawaited(_mail.refresh());
  }

  Future<void> _add() async {
    await showDialog<MailAccountInfo>(context: context, builder: (_) => AddMailAccountDialog(controller: _mail));
  }

  Future<void> _remove(MailAccountInfo account) async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        key: const Key('remove-mail-account'),
        title: Text('Remove "${account.name}"?'),
        content: const Text(
          'Takes the mailbox out of canoa.cfg and forgets any saved password. Nothing in the mailbox itself changes.',
        ),
        actions: [
          TextButton(onPressed: () => Navigator.of(context).pop(false), child: const Text('Keep it')),
          FilledButton(
            key: const Key('remove-mail-account-confirm'),
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Remove'),
          ),
        ],
      ),
    );
    if (sure != true) return;
    try {
      await _mail.remove(account.name);
    } on Object catch (error) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text('Could not remove ${account.name}: $error')));
    }
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: _mail,
      builder: (context, _) {
        final scheme = Theme.of(context).colorScheme;
        final muted = Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
        final accounts = _mail.accounts;
        final canAdd = _mail.backends.isNotEmpty || _mail.problem == null;
        return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 0, 4, 16),
            child: Text(
              'An agent can search, read and draft in these mailboxes, and asks you before it moves or changes a message. '
              'It never sends. Passwords stay in this computer\'s credential store, never in canoa.cfg.',
              style: muted,
            ),
          ),
          if (_mail.problem case final problem?)
            Padding(
              padding: const EdgeInsets.only(bottom: 12),
              child: Text(problem, key: const Key('mail-problem'), style: TextStyle(color: scheme.error)),
            ),
          SettingsGroup(
            key: const Key('mail-accounts'),
            title: 'Mailboxes',
            trailing: TextButton.icon(
              key: const Key('add-mail-account'),
              onPressed: _mail.loading || !canAdd ? null : () => unawaited(_add()),
              icon: const Icon(Icons.add, size: 18),
              label: const Text('Add mailbox'),
            ),
            children: [
              if (accounts.isEmpty)
                SettingsRow(
                  title: _mail.loading ? 'Reading accounts from host...' : 'No mailbox',
                  description: 'Agents have no mail to use until one is added.',
                ),
              for (final account in accounts) _row(account),
            ],
          ),
        ]);
      },
    );
  }

  Widget _row(MailAccountInfo account) {
    final scheme = Theme.of(context).colorScheme;
    final result = _mail.testOf(account.name);
    final testing = _mail.isTesting(account.name);
    final detail = [account.titleWith(_mail.backends), ...account.summaryDetails(), account.state].join(' · ');
    return Column(key: Key('mail-account ${account.name}'), crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      SettingsRow(
        title: account.name,
        leading: Icon(account.backend == 'outlook' ? Icons.desktop_windows_outlined : Icons.mail_outline, size: 24),
        description: detail,
        below: testing
            ? const LinearProgressIndicator(key: Key('mail-testing'))
            : result == null
                ? null
                : Text(
                    result.ok ? 'Signed in. ${_folders(result.folders)}' : (result.problem ?? 'The sign-in failed.'),
                    key: Key('mail-test-result ${account.name}'),
                    style: TextStyle(color: result.ok ? scheme.primary : scheme.error),
                  ),
        control: Row(mainAxisSize: MainAxisSize.min, children: [
          OutlinedButton(
            key: Key('mail-test ${account.name}'),
            onPressed: testing ? null : () => unawaited(_mail.test(account.name)),
            child: const Text('Test sign-in'),
          ),
          IconButton(
            key: Key('mail-remove ${account.name}'),
            tooltip: 'Remove',
            icon: Icon(Icons.delete_outline, size: 20, color: scheme.error),
            onPressed: testing ? null : () => unawaited(_remove(account)),
          ),
        ]),
      ),
    ]);
  }

  static String _folders(int? count) => count == null ? '' : (count == 1 ? '1 folder.' : '$count folders.');
}

/// Adds one mailbox to the host's configuration from the selected `mail_backend` provider's schema and form.
class AddMailAccountDialog extends StatefulWidget {
  const AddMailAccountDialog({super.key, required this.controller});

  final MailAccountsController controller;

  @override
  State<AddMailAccountDialog> createState() => _AddMailAccountDialogState();
}

class _AddMailAccountDialogState extends State<AddMailAccountDialog> {
  final _name = TextEditingController();
  final _values = <String, Map<String, Object?>>{};
  String? _backendId;
  int _generation = 0;
  String? _problem;
  bool _saving = false;

  @override
  void initState() {
    super.initState();
    _backendId = widget.controller.backends.firstOrNull?.id;
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  MailBackendInfo? get _backend {
    final backends = widget.controller.backends;
    final chosen = _backendId;
    if (chosen == null) return backends.firstOrNull;
    return backends.where((b) => b.id == chosen).firstOrNull ?? backends.firstOrNull;
  }

  Future<void> _submit() async {
    final name = _name.text.trim();
    if (name.isEmpty) return setState(() => _problem = 'Name the mailbox');
    if (widget.controller.named(name) != null) return setState(() => _problem = 'A mailbox is called that already');
    final backend = _backend;
    if (backend == null) return setState(() => _problem = 'No mail backend is offered by the host');

    final schema = backend.schema ?? const {};
    final form = backend.form ?? const {};
    final rawValues = _values[backend.id] ?? const {};
    final localProblem = schema.isEmpty ? null : schemaProblem(schema, form, rawValues);
    if (localProblem != null) return setState(() => _problem = localProblem);

    final settings = schema.isEmpty ? rawValues : collectSchemaValues(schema, form, rawValues);
    setState(() {
      _problem = null;
      _saving = true;
    });
    try {
      final made = await widget.controller.add(NewMailAccount(name: name, backend: backend.id, settings: settings));
      if (mounted) Navigator.of(context).pop(made);
    } on ControlFailed catch (error) {
      if (mounted) setState(() => _problem = error.message);
    } on Object catch (error) {
      if (mounted) setState(() => _problem = '$error');
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final backends = widget.controller.backends;
    final backend = _backend;
    final schema = backend?.schema ?? const {};
    final form = backend?.form ?? const {};
    return AlertDialog(
      key: const Key('add-mail-dialog'),
      title: const Text('Add mail account'),
      content: SizedBox(
        width: 460,
        child: SingleChildScrollView(
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            TextField(
              key: const Key('mail-name'),
              controller: _name,
              decoration: const InputDecoration(
                labelText: 'Name',
                hintText: 'work',
                helperText: 'Lower-case letters, digits, - and _',
              ),
              textInputAction: TextInputAction.next,
            ),
            if (backends.isNotEmpty) ...[
              const SizedBox(height: 12),
              DropdownButtonFormField<String>(
                key: const Key('mail-backend-picker'),
                isExpanded: true,
                initialValue: backend?.id,
                decoration: const InputDecoration(labelText: 'Backend'),
                items: [
                  for (final each in backends) DropdownMenuItem<String>(value: each.id, child: Text(each.title)),
                ],
                onChanged: _saving
                    ? null
                    : (id) => setState(() {
                          _backendId = id;
                          _generation++;
                          _problem = null;
                        }),
              ),
              if (backend?.description case final desc?)
                Padding(
                  padding: const EdgeInsets.only(top: 6),
                  child: Text(desc, style: Theme.of(context).textTheme.bodySmall),
                ),
            ] else
              const Padding(
                padding: EdgeInsets.only(top: 12),
                child: Text('Waiting for the host to list available mail backends.'),
              ),
            if (backend != null && schema.isNotEmpty) ...[
              const SizedBox(height: 8),
              SchemaFields(
                key: ValueKey('mail-${backend.id}-$_generation'),
                schema: schema,
                form: form,
                values: _values[backend.id] ?? const {},
                idPrefix: 'mail',
                unsetEntry: true,
                onValues: (values) => _values[backend.id] = values,
                onSubmitted: () => unawaited(_submit()),
              ),
            ],
            if (_problem != null) ...[
              const SizedBox(height: 10),
              Text(
                _problem!,
                key: const Key('add-mail-problem'),
                style: Theme.of(context).textTheme.bodySmall?.copyWith(color: Theme.of(context).colorScheme.error),
              ),
            ],
          ]),
        ),
      ),
      actions: [
        TextButton(onPressed: _saving ? null : () => Navigator.of(context).pop(), child: const Text('Cancel')),
        FilledButton(
          key: const Key('add-mail-submit'),
          onPressed: _saving || backend == null ? null : _submit,
          child: Text(_saving ? 'Adding...' : 'Add'),
        ),
      ],
    );
  }
}
