import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ComposerPill;
import 'package:flutter/material.dart';

import '../chat/composer.dart' show ComposerKit;
import '../chat/session_picker.dart';
import '../session_store.dart';
import '../sources/source_icons.dart';

/// The account a session runs on, beside the source, which opens the one picker where the accounts the source's entry offers are
/// listed, each with where it stands. One that a session holds (a login that rotates allows one holder) or that needs signing in
/// again is listed but cannot be chosen, and the first offered is the default. The choice travels in `initialize`'s `_meta`, by
/// name; once the session has started the picker only says which account it runs on.
class AccountChooser extends StatelessWidget {
  const AccountChooser({super.key, required this.store, required this.session, required this.kit});

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;

  @override
  Widget build(BuildContext context) {
    final accounts = kit.accountsFor(session);
    final entry = session.source?.hostEntry ?? session.agent;
    if (accounts == null) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: accounts,
      builder: (context, _) {
        if (accounts.harnessOf(entry) == null) return const SizedBox.shrink();
        final chosen = chosenAccount(accounts, entry, session.account);
        return InkWell(
          key: session.isDraft ? const Key('option-account') : const Key('session-account'),
          borderRadius: BorderRadius.circular(8),
          onTap: () => unawaited(showSessionPicker(context, store: store, session: session, kit: kit)),
          child: Tooltip(
            message: session.isDraft ? 'The account this session runs on' : 'The account this session runs on; another needs a new conversation',
            child: ComposerPill(
              label: chosen?.name ?? machineLoginLabel,
              leading: SourceMark.provider(
                chosen == null ? accounts.harnessOf(entry) : accountMarkId(chosen.harness, chosen.kind),
                size: 16,
                key: const Key('account-chip-mark'),
              ),
            ),
          ),
        );
      },
    );
  }
}
