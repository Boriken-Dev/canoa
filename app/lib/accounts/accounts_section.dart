import 'dart:async';

import 'package:flutter/material.dart';

import '../settings/settings_row.dart';
import '../sources/source_icons.dart';
import 'account_models.dart';
import 'accounts_controller.dart';
import 'accounts_host.dart';
import 'add_account_dialog.dart';
import 'pools_section.dart';
import 'usage_view.dart';

/// Settings › Accounts: a group per provider listing its accounts (name, kind, endpoint, whether it is signed in or a session holds
/// it, and what the sessions on it last heard of its allowances), with Add account, sign in again, which entries offer it, and
/// Remove. Management is the host's of this computer: a client on the network only picks among what an entry offers.
class AccountsSection extends StatefulWidget {
  const AccountsSection({super.key, required this.controller});

  final AccountsController controller;

  @override
  State<AccountsSection> createState() => _AccountsSectionState();
}

class _AccountsSectionState extends State<AccountsSection> {
  final _usageOpen = <String>{};

  AccountsController get _accounts => widget.controller;

  @override
  void initState() {
    super.initState();
    _accounts.addListener(_changed);
    // Asking notifies, which a build in progress cannot take: ask once this frame is done.
    WidgetsBinding.instance.addPostFrameCallback((_) => unawaited(_accounts.refresh()));
  }

  @override
  void dispose() {
    _accounts.removeListener(_changed);
    super.dispose();
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  Future<void> _add(String harness) async {
    await showDialog<AccountInfo>(context: context, builder: (_) => AddAccountDialog(accounts: _accounts, harness: harness));
    if (mounted) unawaited(_accounts.refresh());
  }

  Future<void> _again(AccountInfo account, LoginMethodInfo? method) async {
    final run = _accounts.signInAgain(account, method: method?.id);
    await showDialog<void>(context: context, builder: (_) => SignInDialog(run: run, title: 'Sign in to ${account.name}'));
  }

  /// Offers [account] to [entry], or stops. An entry no `peers` restricts is open to every paired key (the extension, a phone,
  /// the web app): the person is told so before an account that may be billed per use is offered there.
  Future<void> _offer(AccountInfo account, String entry) async {
    final offering = !_accounts.offering(account.name).contains(entry);
    if (offering && _accounts.isOpen(entry)) {
      final sure = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          key: const Key('offer-open-entry'),
          title: Text('Offer ${account.name} in $entry?'),
          content: Text(
            '$entry has no list of keys, so every key paired with this host can start a session on it, running the agent as the user '
            'who runs this host, and sessions there run on '
            '${_accounts.offeredBy(entry).isEmpty ? account.name : 'any account it offers, ${account.name} included'}'
            '${_accounts.offeredBy(entry).isEmpty ? ', which then replaces the login $entry runs on now' : ''}. '
            'Add the keys that may use it to its peers in canoa.cfg, or keep ${account.name} to an entry only this computer reaches.',
          ),
          actions: [
            TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Do not offer it')),
            FilledButton(key: const Key('offer-open-entry-confirm'), onPressed: () => Navigator.pop(context, true), child: const Text('Offer it to every paired key')),
          ],
        ),
      );
      if (sure != true) return;
    }
    await _accounts.setOffered(account, entry, offered: offering);
  }

  Future<void> _remove(AccountInfo account) async {
    final offered = _accounts.offering(account.name);
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        key: const Key('remove-account'),
        title: Text('Remove ${account.name}?'),
        content: Text(
          'Canoa forgets the login it stored for ${account.name} on this computer'
          '${offered.isEmpty ? '' : ' and stops offering it in ${offered.join(', ')}'}. '
          'It does not sign out at ${harnessTitle(account.harness)}: a login there stays until you end it.',
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Keep it')),
          FilledButton(key: const Key('remove-account-confirm'), onPressed: () => Navigator.pop(context, true), child: const Text('Remove')),
        ],
      ),
    );
    if (sure != true) return;
    try {
      await _accounts.remove(account);
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
          'Several logins of one provider can be signed in at once. A source offers the accounts you choose, and each session '
          'runs on one of them. Credentials stay in this computer\'s credential store, never in canoa.cfg.',
          style: muted,
        ),
      ),
      if (_accounts.problem case final problem?)
        Padding(padding: const EdgeInsets.only(bottom: 12), child: Text(problem, key: const Key('accounts-problem'), style: TextStyle(color: scheme.error))),
      for (final harness in _accounts.providers)
        SettingsGroup(
          key: Key('accounts-$harness'),
          title: harnessTitle(harness),
          leading: SourceMark.provider(harness, size: 20),
          trailing: TextButton.icon(
            key: Key('add-account-$harness'),
            onPressed: () => unawaited(_add(harness)),
            icon: const Icon(Icons.add, size: 18),
            label: const Text('Add account'),
          ),
          children: [
            if (_accounts.of(harness).isEmpty)
              SettingsRow(
                title: 'No account',
                description: '${harnessTitle(harness)} runs on the login it has where it runs, as before.',
              ),
            for (final account in _accounts.of(harness)) _row(account),
          ],
        ),
      PoolsGroup(controller: _accounts),
    ]);
  }

  Widget _row(AccountInfo account) {
    final catalog = _accounts.catalogIfKnown(account.harness);
    final kind = catalog?.kind(account.kind);
    final held = _accounts.heldBy(account.name);
    final scheme = Theme.of(context).colorScheme;
    final detail = [
      kind?.title ?? account.kind,
      ?kind?.modelEndpoint,
      account.statusLabel,
      if (held != null) 'held by "$held"',
    ].join(' · ');
    final entries = [for (final entry in _accounts.entriesRunning(account.harness)) entry];
    final logins = [if (catalog != null) for (final login in catalog.logins) if (login.produces == account.kind) login];
    final report = _accounts.usageOf(account);
    final open = _usageOpen.contains(account.name);
    return Column(key: Key('account ${account.name}'), crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      SettingsRow(
        title: account.name,
        leading: SourceMark.provider(accountMarkId(account.harness, account.kind), size: 24, key: Key('account-mark ${account.name}')),
        description: detail,
        // The standards it supports say which agents can run on it; the usage opens under them.
        below: account.provides.isEmpty && !open
            ? null
            : Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                if (account.provides.isNotEmpty)
                  Text(
                    'Supports ${standardsText(account.provides)}',
                    key: Key('account-supports ${account.name}'),
                    style: Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant),
                  ),
                if (open)
                  Padding(
                    padding: const EdgeInsets.only(top: 4),
                    child: AccountUsageView(key: Key('account-usage-view ${account.name}'), report: report),
                  ),
              ]),
        control: Row(mainAxisSize: MainAxisSize.min, children: [
          if (account.status == AccountStatus.needsSignIn)
            Padding(
              padding: const EdgeInsets.only(right: 8),
              child: FilledButton.tonal(
                key: Key('account-sign-in ${account.name}'),
                onPressed: held != null ? null : () => unawaited(_again(account, logins.firstOrNull)),
                child: const Text('Sign in'),
              ),
            ),
          IconButton(
            key: Key('account-usage-toggle ${account.name}'),
            tooltip: open ? 'Hide usage' : 'Show usage',
            icon: Icon(open ? Icons.expand_less : Icons.speed, size: 20),
            onPressed: () => setState(() => open ? _usageOpen.remove(account.name) : _usageOpen.add(account.name)),
          ),
          PopupMenuButton<String>(
            key: Key('account-menu ${account.name}'),
            tooltip: 'More',
            onSelected: (value) {
              if (value == 'remove') unawaited(_remove(account));
              if (value.startsWith('login:')) unawaited(_again(account, logins.where((each) => 'login:${each.id}' == value).firstOrNull));
              if (value.startsWith('offer:')) unawaited(_offer(account, value.substring(6)));
            },
            itemBuilder: (context) => [
              for (final login in logins)
                PopupMenuItem(
                  key: Key('account-login ${account.name} ${login.id}'),
                  value: 'login:${login.id}',
                  enabled: held == null,
                  child: Text(logins.length == 1 ? 'Sign in again' : 'Sign in again with ${login.title}'),
                ),
              if (entries.isNotEmpty) ...[
                if (logins.isNotEmpty) const PopupMenuDivider(),
                const PopupMenuItem(enabled: false, height: 28, child: Text('Offered by')),
                for (final entry in entries)
                  CheckedPopupMenuItem(
                    key: Key('account-offer ${account.name} $entry'),
                    value: 'offer:$entry',
                    checked: _accounts.offering(account.name).contains(entry),
                    child: Text(entry),
                  ),
                const PopupMenuDivider(),
              ],
              PopupMenuItem(
                key: Key('account-remove ${account.name}'),
                value: 'remove',
                enabled: held == null,
                child: Text(held == null ? 'Remove' : 'Remove (a session holds it)', style: TextStyle(color: held == null ? scheme.error : null)),
              ),
            ],
          ),
        ]),
      ),
    ]);
  }
}

/// A sign-in that has no form of its own to start from (signing in again): what it says and asks, and a way to give up.
class SignInDialog extends StatefulWidget {
  const SignInDialog({super.key, required this.run, required this.title});

  final SignIn run;
  final String title;

  @override
  State<SignInDialog> createState() => _SignInDialogState();
}

class _SignInDialogState extends State<SignInDialog> {
  @override
  void initState() {
    super.initState();
    widget.run.addListener(_changed);
  }

  @override
  void dispose() {
    widget.run.removeListener(_changed);
    super.dispose();
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    key: const Key('sign-in-dialog'),
    title: Text(widget.title),
    content: ConstrainedBox(constraints: const BoxConstraints(maxWidth: 460), child: SizedBox(width: double.maxFinite, child: SignInPanel(run: widget.run))),
    actions: [
      if (widget.run.running)
        TextButton(
          key: const Key('sign-in-cancel'),
          onPressed: () {
            widget.run.cancel();
            Navigator.pop(context);
          },
          child: const Text('Cancel'),
        )
      else
        FilledButton(key: const Key('sign-in-close'), onPressed: () => Navigator.pop(context), child: const Text('Close')),
    ],
  );
}
