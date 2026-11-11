import 'package:flutter/material.dart';

import '../acp/client.dart';
import '../session_store.dart';
import '../theme/look.dart';
import 'agent_events.dart' show shortText;
import 'remember_dialog.dart';

/// Where an option goes among the answers: refusals first, then what Canoa does not know, then what allows; once before always.
int _rank(AcpPermissionOption option) => switch (option.kind) {
  'reject_once' => 0,
  'reject_always' => 1,
  'allow_once' => 3,
  'allow_always' => 4,
  _ => 2,
};

/// The words for an option the agent sent without a name.
String _kindLabel(AcpPermissionOption option) => switch (option.kind) {
  'allow_once' => 'Allow once',
  'allow_always' => 'Allow always',
  'reject_once' => 'Refuse once',
  'reject_always' => 'Refuse always',
  _ => shortText(option.kind) ?? shortText(option.optionId) ?? 'An answer',
};

/// One of the agent's answers, under the agent's own name for it.
class PermissionChoice extends StatelessWidget {
  const PermissionChoice({super.key, required this.option, required this.onPressed});

  final AcpPermissionOption option;
  final VoidCallback onPressed;

  String get label => shortText(option.name) ?? _kindLabel(option);

  @override
  Widget build(BuildContext context) {
    final text = Text(label, maxLines: 2, overflow: TextOverflow.ellipsis);
    return switch (option.kind) {
      'allow_once' => FilledButton(onPressed: onPressed, child: text),
      'allow_always' => FilledButton.tonal(onPressed: onPressed, child: text),
      'reject_once' || 'reject_always' => TextButton(onPressed: onPressed, child: text),
      _ => OutlinedButton(onPressed: onPressed, child: text),
    };
  }
}

/// What an agent is waiting on, with the answers it offers. It stays until the person answers: nothing answers it for them but
/// Bypass, which does so before the request would wait here.
class PermissionCard extends StatelessWidget {
  const PermissionCard({super.key, required this.store, required this.session});

  final SessionStore store;
  final ChatSession session;

  /// Narrower than this, the answers go under the question rather than beside it, which would leave its text a thin column.
  static const double besideFrom = 560;

  @override
  Widget build(BuildContext context) {
    final question = session.pendingPermissions.first;
    final small = Theme.of(context).textTheme.labelSmall;
    final ordered = [...question.options]..sort((a, b) => _rank(a) - _rank(b));
    final remembers = ordered.any((option) => option.kind.endsWith('_always'));
    final agent = shortText(session.agent) ?? 'The agent';
    final folder = session.cwd ?? '';
    final canRemember = question.subject != null && folder.isNotEmpty;
    final asked = Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text('$agent wants to use a tool', style: Theme.of(context).textTheme.labelMedium),
        Text(question.title),
        if (question.detail?.isNotEmpty ?? false) _ToApprove(detail: question.detail!),
        if (remembers)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(
              'An "always" answer is remembered by $agent, not by Canoa: Canoa cannot see or undo what it keeps.',
              style: small,
            ),
          ),
        if (canRemember)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(
              'Always allow here: Canoa itself answers allow once to what you confirm, for $agent in $folder, until you revoke '
              'it in Settings, Permissions.',
              style: small,
            ),
          ),
        if (question.bypassPassed)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(
              question.forPerson
                  ? 'Bypass leaves a question or a plan to you, so this one is yours to answer.'
                  : 'Bypass answers only a request that offers to allow, so this one is yours to answer.',
              style: small?.copyWith(fontWeight: FontWeight.w600),
            ),
          ),
        if (ordered.isEmpty)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text('The agent offered no answer Canoa can give.', style: small),
          ),
        if (session.pendingPermissions.length > 1)
          Text('${session.pendingPermissions.length - 1} more after this', style: small),
      ],
    );
    final answers = Wrap(
      spacing: 8,
      runSpacing: 4,
      alignment: WrapAlignment.end,
      children: [
        for (final option in ordered)
          PermissionChoice(option: option, onPressed: () => store.choosePermission(session, option)),
        if (canRemember)
          FilledButton.tonal(
            key: const Key('always-allow-here'),
            onPressed: () => showDialog<bool>(
              context: context,
              builder: (_) => RememberDialog(store: store, session: session, question: question),
            ),
            child: Text('Always allow here: ${folderName(folder)}', maxLines: 2, overflow: TextOverflow.ellipsis),
          ),
        if (ordered.isEmpty)
          TextButton(onPressed: () => store.cancelPermission(session), child: const Text('Cancel the request')),
      ],
    );
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
      child: Card(
        child: Padding(
          padding: const EdgeInsets.all(12),
          child: LayoutBuilder(builder: (context, constraints) {
            final beside = constraints.maxWidth >= besideFrom && ordered.length + (canRemember ? 1 : 0) <= 2;
            final top = Row(
              crossAxisAlignment: beside ? CrossAxisAlignment.center : CrossAxisAlignment.start,
              children: [
                const Icon(Icons.shield_outlined),
                const SizedBox(width: 12),
                Expanded(child: asked),
                if (beside) answers,
              ],
            );
            if (beside) return top;
            return Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [top, const SizedBox(height: 8), Align(alignment: Alignment.centerRight, child: answers)],
            );
          }),
        ),
      ),
    );
  }
}

/// What a tool would do, in a box of a fixed height. Whether it holds more than the box shows is measured at the width it has, so a
/// short edit spread over a few lines is not called long, and a long one is.
class _ToApprove extends StatelessWidget {
  const _ToApprove({required this.detail});

  final String detail;

  static const double shown = 120;
  static const TextStyle _style = TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12);

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 4),
      child: LayoutBuilder(builder: (context, constraints) {
        final painter = TextPainter(
          text: TextSpan(text: detail, style: DefaultTextStyle.of(context).style.merge(_style)),
          textDirection: Directionality.of(context),
          textScaler: MediaQuery.textScalerOf(context),
        )..layout(maxWidth: constraints.maxWidth);
        final long = painter.height > shown;
        painter.dispose();
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            ConstrainedBox(
              constraints: const BoxConstraints(maxHeight: shown),
              child: SingleChildScrollView(child: SelectableText(detail, style: _style)),
            ),
            if (long)
              Padding(
                padding: const EdgeInsets.only(top: 4),
                child: Text(
                  'This is long. Scroll to read all of it before you answer.',
                  style: Theme.of(context).textTheme.labelSmall?.copyWith(fontWeight: FontWeight.w600),
                ),
              ),
          ],
        );
      }),
    );
  }
}

/// Says, for as long as it lasts, that a conversation is in Bypass, with the one action that ends it. It is a status, in the
/// colours of the mode's pill, not an alarm.
class BypassBanner extends StatelessWidget {
  const BypassBanner({super.key, required this.onLeave});

  final VoidCallback onLeave;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Material(
      key: const Key('bypass-banner'),
      color: scheme.tertiaryContainer,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
        child: Row(children: [
          Icon(Icons.fast_forward_outlined, color: scheme.onTertiaryContainer),
          const SizedBox(width: 12),
          Expanded(
            child: Text(
              'Bypass: each request that offers to allow is allowed without asking you.',
              style: TextStyle(color: scheme.onTertiaryContainer),
            ),
          ),
          TextButton(
            onPressed: onLeave,
            style: TextButton.styleFrom(foregroundColor: scheme.onTertiaryContainer),
            child: const Text('Leave Bypass'),
          ),
        ]),
      ),
    );
  }
}
