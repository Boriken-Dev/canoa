import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLookOf;
import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../session_store.dart';
import '../sources/sources.dart';
import 'role_control.dart';
import 'roles_controller.dart';

/// What the person is asked when a role whose handover is `ask` has run out on a member and the next is of another harness: the
/// host holds the move until they say yes. Nothing moves until Continue is pressed.
class SessionRoleAsk extends StatelessWidget {
  const SessionRoleAsk({super.key, required this.store, required this.session, required this.roles, required this.sources});

  final SessionStore store;
  final ChatSession session;
  final RolesController roles;
  final List<AgentSource> sources;

  @override
  Widget build(BuildContext context) {
    final ask = session.roleAsk;
    if (ask == null) return const SizedBox.shrink();
    final look = context.chatLook;
    final text = AppText.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 4, 12, 4),
      child: Center(
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: look.messageWidth + 100),
          child: Container(
            key: const Key('role-ask'),
            padding: const EdgeInsets.all(12),
            decoration: BoxDecoration(border: Border.all(color: look.muted.withValues(alpha: 0.35)), borderRadius: BorderRadius.circular(look.radius)),
            child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
              Text(
                text('role.ask', {
                  'from': memberText(ask.from, sources, roles),
                  'to': memberText(ask.to, sources, roles),
                }),
                key: const Key('role-ask-text'),
              ),
              if (ask.left != null) Padding(padding: const EdgeInsets.only(top: 4), child: Text(ask.left!, style: TextStyle(color: look.muted, fontSize: 12.5))),
              const SizedBox(height: 8),
              Wrap(spacing: 8, runSpacing: 8, children: [
                FilledButton.tonal(
                  key: const Key('role-ask-continue'),
                  onPressed: () => unawaited(store.continueRole(session)),
                  child: Text(text('role.ask_continue')),
                ),
                TextButton(
                  key: const Key('role-ask-dismiss'),
                  onPressed: () => store.dismissRoleAsk(session),
                  child: Text(text('role.ask_not_now')),
                ),
              ]),
            ]),
          ),
        ),
      ),
    );
  }
}
