import 'package:flutter/material.dart';

import '../chat/composer.dart' show ComposerKit;
import '../session_store.dart';
import 'account_models.dart';
import 'accounts_controller.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ComposerPill;

/// The account a new session runs on, beside the source: the accounts the source's entry offers, each with where it stands. One that
/// a session holds (a login that rotates allows one holder) or that needs signing in again is listed but cannot be chosen, and the
/// first offered is the default. The choice travels in `initialize`'s `_meta`, by name; once the session has started it only says
/// which account it runs on.
class AccountChooser extends StatelessWidget {
  const AccountChooser({super.key, required this.store, required this.session, required this.kit});

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;

  @override
  Widget build(BuildContext context) {
    final accounts = kit.accounts;
    final entry = session.source?.hostEntry ?? session.agent;
    if (accounts == null) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: accounts,
      builder: (context, _) {
        final offered = accounts.offeredBy(entry);
        if (offered.isEmpty) return const SizedBox.shrink();
        final chosen = offered.where((each) => each.name == session.account).firstOrNull ?? offered.first;
        if (!session.isDraft) {
          return Tooltip(
            key: const Key('session-account'),
            message: 'The account this session runs on',
            child: ComposerPill(label: chosen.name, icon: Icons.account_circle_outlined, enabled: false),
          );
        }
        final scheme = Theme.of(context).colorScheme;
        return PopupMenuButton<Object>(
          key: const Key('option-account'),
          tooltip: 'The account this session runs on',
          onSelected: (value) {
            if (value is AccountInfo) store.setAccount(session, value.name);
            if (value == #add) kit.onAccountSettings?.call();
          },
          itemBuilder: (context) => [
            for (final account in offered)
              PopupMenuItem<Object>(
                key: Key('account-choice ${account.name}'),
                value: account,
                enabled: account.usable && accounts.heldBy(account.name) == null,
                child: ListTile(
                  dense: true,
                  contentPadding: EdgeInsets.zero,
                  leading: Icon(account.name == chosen.name ? Icons.check : null, size: 18),
                  title: Text(account.name),
                  subtitle: Text(
                    _state(accounts, account),
                    style: TextStyle(color: account.usable && accounts.heldBy(account.name) == null ? null : scheme.error),
                  ),
                ),
              ),
            if (kit.onAccountSettings != null) ...[
              const PopupMenuDivider(),
              const PopupMenuItem<Object>(key: Key('account-add'), value: #add, child: Text('Add account…')),
            ],
          ],
          child: ComposerPill(label: chosen.name, icon: Icons.account_circle_outlined),
        );
      },
    );
  }
}

String _state(AccountsController accounts, AccountInfo account) {
  final holder = accounts.heldBy(account.name);
  return holder == null ? account.statusLabel : 'In use by "$holder"';
}
