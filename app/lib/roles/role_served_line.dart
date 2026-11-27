import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLookOf;
import 'package:material_ui/material_ui.dart';

import '../history/session_move.dart' show MoveLoss;
import '../l10n/app_text.dart';
import '../session_store.dart';
import '../sources/sources.dart';
import 'role_control.dart';
import 'roles_controller.dart';

/// A time of day, as the person reads it on a clock.
String clockTime(DateTime at) {
  final local = at.toLocal();
  return '${local.hour.toString().padLeft(2, '0')}:${local.minute.toString().padLeft(2, '0')}';
}

/// What a kind of loss says in a sentence, in the language in force; the host's own word for a kind this app has no sentence for.
String roleLossText(AppText text, MoveLoss loss) {
  final known = text.maybe('role.loss.${loss.kind}', {'count': loss.count});
  final sentence = known ?? '${loss.kind} (${loss.count})';
  return loss.detail == null ? sentence : '$sentence. ${loss.detail}';
}

/// The line under a role's conversation: which role serves it, the member it runs on and, once it has moved, when and from whom,
/// with what the move could not carry on hover. Nothing shows for a session no role serves.
class SessionServedLine extends StatelessWidget {
  const SessionServedLine({super.key, required this.session, required this.roles, required this.sources});

  final ChatSession session;
  final RolesController roles;
  final List<AgentSource> sources;

  @override
  Widget build(BuildContext context) {
    final problem = session.roleProblem;
    final serving = session.roleServing;
    if (serving == null && problem == null) return const SizedBox.shrink();
    final look = context.chatLook;
    final text = AppText.of(context);
    final lines = <Widget>[];
    if (serving != null) {
      final member = memberText(serving.member, sources, roles);
      final head = text('role.served_by', {'member': member, 'role': serving.title});
      final moved = serving.movedAt;
      final from = serving.movedFrom;
      final losses = [for (final each in serving.losses) roleLossText(text, each)];
      if (serving.replayed) losses.insert(0, text('role.replayed'));
      lines.add(Text(
        serving.usageKnown ? head : '$head · ${text('role.usage_unreported')}',
        key: const Key('role-served-by'),
        style: TextStyle(color: look.muted, fontSize: 12.5),
      ));
      if (moved != null) {
        final said = text(from == null ? 'role.moved_to' : 'role.moved', {
          'role': serving.title,
          'from': from == null ? '' : memberText(from, sources, roles),
          'to': member,
          'time': clockTime(moved),
        });
        final line = Text(
          losses.isEmpty ? said : '$said · ${text('role.losses_count', {'count': losses.length})}',
          key: const Key('role-moved'),
          style: TextStyle(color: look.muted, fontSize: 12.5),
        );
        lines.add(losses.isEmpty
            ? line
            : Tooltip(
                key: const Key('role-losses'),
                message: [text('role.losses_title'), for (final each in losses) '- $each'].join('\n'),
                child: line,
              ));
      }
    }
    if (problem != null) {
      lines.add(Text(
        text(problem.key, {'role': problem.role, 'reason': problem.reason}),
        key: const Key('role-problem'),
        style: TextStyle(color: look.danger, fontSize: 12.5),
      ));
    }
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 2, 12, 2),
      child: Center(
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: look.messageWidth + 100),
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: lines),
        ),
      ),
    );
  }
}
