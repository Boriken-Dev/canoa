import 'dart:async';

import 'package:flutter/material.dart';

import '../settings/settings_row.dart';
import 'mail_accounts.dart';

/// The rule for a mail account's name, which the host holds to.
final RegExp mailAccountName = RegExp(r'^[a-z0-9_-]{1,32}$');

/// Settings › Mail: the mailboxes an agent can use as a source. Each row says where the mailbox is and whether its password is
/// there, and can sign in to it to see that it works or be removed. A password is typed here once, goes to the host and is kept in
/// the host's account store; nothing here shows it again.
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
    _mail.addListener(_changed);
    WidgetsBinding.instance.addPostFrameCallback((_) => unawaited(_mail.refresh()));
  }

  @override
  void dispose() {
    _mail.removeListener(_changed);
    super.dispose();
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  Future<void> _add() async {
    await showDialog<MailAccountInfo>(context: context, builder: (_) => AddMailAccountDialog(controller: _mail));
  }

  Future<void> _remove(MailAccountInfo account) async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        key: const Key('remove-mail-account'),
        title: Text('Remove ${account.name}?'),
        content: Text(
          'Canoa forgets the password it stored for ${account.name} and stops offering the mailbox to agents. '
          'Nothing changes in the mailbox itself.',
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Keep it')),
          FilledButton(key: const Key('remove-mail-account-confirm'), onPressed: () => Navigator.pop(context, true), child: const Text('Remove')),
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
    final scheme = Theme.of(context).colorScheme;
    final muted = Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
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
        Padding(padding: const EdgeInsets.only(bottom: 12), child: Text(problem, key: const Key('mail-problem'), style: TextStyle(color: scheme.error))),
      SettingsGroup(
        key: const Key('mail-accounts'),
        title: 'Mailboxes',
        trailing: TextButton.icon(
          key: const Key('add-mail-account'),
          onPressed: () => unawaited(_add()),
          icon: const Icon(Icons.add, size: 18),
          label: const Text('Add mailbox'),
        ),
        children: [
          if (_mail.accounts.isEmpty)
            const SettingsRow(title: 'No mailbox', description: 'Agents have no mail to use until one is added.'),
          for (final account in _mail.accounts) _row(account),
        ],
      ),
    ]);
  }

  Widget _row(MailAccountInfo account) {
    final scheme = Theme.of(context).colorScheme;
    final result = _mail.testOf(account.name);
    final testing = _mail.isTesting(account.name);
    final detail = [account.backendTitle, ?account.where, if (account.user != null) account.user!, account.state].join(' · ');
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
            onPressed: () => unawaited(_remove(account)),
          ),
        ]),
      ),
    ]);
  }

  static String _folders(int? count) => count == null ? '' : (count == 1 ? '1 folder.' : '$count folders.');
}

/// Adds a mailbox: Exchange Server by its address (or the mailbox's own, which Exchange's Autodiscover finds it by) and a user name and
/// password, or classic Outlook, which signs in as Outlook does. The password is sent to the host once.
class AddMailAccountDialog extends StatefulWidget {
  const AddMailAccountDialog({super.key, required this.controller});

  final MailAccountsController controller;

  @override
  State<AddMailAccountDialog> createState() => _AddMailAccountDialogState();
}

class _AddMailAccountDialogState extends State<AddMailAccountDialog> {
  final _name = TextEditingController(), _email = TextEditingController(), _server = TextEditingController();
  final _user = TextEditingController(), _password = TextEditingController();
  String _backend = 'ews';
  String? _problem;
  bool _working = false;

  @override
  void dispose() {
    _name.dispose();
    _email.dispose();
    _server.dispose();
    _user.dispose();
    _password.dispose();
    super.dispose();
  }

  bool get _exchange => _backend == 'ews';

  String? get _nameProblem {
    final name = _name.text;
    if (!mailAccountName.hasMatch(name)) return 'Use 1 to 32 lower-case letters, digits, - or _';
    if (widget.controller.named(name) != null) return 'A mailbox is called that already';
    return null;
  }

  String? get _missing {
    if (_nameProblem case final problem?) return problem;
    if (!_exchange) return null;
    if (_server.text.trim().isEmpty && _email.text.trim().isEmpty) return 'Give the server address or the mailbox\'s email address';
    if (_user.text.trim().isNotEmpty && _password.text.isEmpty) return 'Type the password';
    return null;
  }

  Future<void> _submit() async {
    final missing = _missing;
    if (missing != null) {
      setState(() => _problem = missing);
      return;
    }
    String? value(TextEditingController field) => field.text.trim().isEmpty ? null : field.text.trim();
    setState(() {
      _working = true;
      _problem = null;
    });
    try {
      final made = await widget.controller.add(NewMailAccount(
        name: _name.text,
        backend: _backend,
        email: _exchange ? value(_email) : null,
        server: _exchange ? value(_server) : null,
        user: _exchange ? value(_user) : null,
        password: _exchange && value(_user) != null ? _password.text : null,
      ));
      // The password is not kept: the field is emptied before the dialog goes.
      _password.clear();
      if (mounted) Navigator.pop(context, made);
    } on Object catch (error) {
      if (mounted) {
        setState(() {
          _working = false;
          _problem = '$error';
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    Widget field(String key, String label, TextEditingController controller, {bool secret = false, String? hint}) => Padding(
      padding: const EdgeInsets.only(top: 12),
      child: TextField(
        key: Key(key),
        controller: controller,
        obscureText: secret,
        enableSuggestions: !secret,
        autocorrect: false,
        decoration: InputDecoration(labelText: label, hintText: hint, border: const OutlineInputBorder(), isDense: true),
        onChanged: (_) => setState(() => _problem = null),
      ),
    );
    return AlertDialog(
      key: const Key('add-mail-dialog'),
      title: const Text('Add a mailbox'),
      content: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 460),
        child: SingleChildScrollView(
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            field('mail-name', 'Name', _name, hint: 'work'),
            Padding(
              padding: const EdgeInsets.only(top: 12),
              child: SegmentedButton<String>(
                key: const Key('mail-backend'),
                segments: const [
                  ButtonSegment(value: 'ews', label: Text('Exchange Server')),
                  ButtonSegment(value: 'outlook', label: Text('Outlook')),
                ],
                selected: {_backend},
                onSelectionChanged: (chosen) => setState(() => _backend = chosen.first),
              ),
            ),
            if (_exchange) ...[
              field('mail-server', 'Server address', _server, hint: 'https://mail.example.com/EWS/Exchange.asmx'),
              field('mail-email', 'Email address', _email, hint: 'ana@example.com'),
              field('mail-user', 'User name', _user, hint: 'EXAMPLE\\ana'),
              field('mail-password', 'Password', _password, secret: true),
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Text(
                  'Give the server, or the email address for Exchange to find it by. With no user name the Windows logon is used.',
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant),
                ),
              ),
            ] else
              Padding(
                padding: const EdgeInsets.only(top: 12),
                child: Text(
                  'Classic Outlook for Windows, with every account it holds, signed in as Outlook is. Outlook may ask you to allow access.',
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant),
                ),
              ),
            if (_problem case final problem?)
              Padding(padding: const EdgeInsets.only(top: 12), child: Text(problem, key: const Key('add-mail-problem'), style: TextStyle(color: scheme.error))),
            if (_working) const Padding(padding: EdgeInsets.only(top: 12), child: LinearProgressIndicator()),
          ]),
        ),
      ),
      actions: [
        TextButton(key: const Key('add-mail-cancel'), onPressed: _working ? null : () => Navigator.pop(context), child: const Text('Cancel')),
        FilledButton(key: const Key('add-mail-submit'), onPressed: _working ? null : () => unawaited(_submit()), child: const Text('Add')),
      ],
    );
  }
}
