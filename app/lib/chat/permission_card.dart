import 'package:flutter/material.dart';

import '../acp/client.dart';
import '../session_store.dart';
import '../theme/look.dart';
import 'agent_events.dart' show shortText;

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

/// What an agent is waiting on, with the answers it offers. It stays until the person answers: nothing answers it for them, and
/// full access answers a request before it would wait here, never one already waiting.
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
        if (question.fullAccessPassed)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(
              'Full access answers only a request that offers to allow once, so this one is yours to answer.',
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
            final beside = constraints.maxWidth >= besideFrom && ordered.length <= 2;
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

/// Says, for as long as it lasts, that a conversation's requests are answered without asking, with the one action that ends it.
class FullAccessBanner extends StatelessWidget {
  const FullAccessBanner({super.key, required this.session, required this.onTurnOff});

  final ChatSession session;
  final VoidCallback onTurnOff;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final whose = session.fullAccess == FullAccess.agent ? 'for ${shortText(session.agent) ?? 'its agent'}' : 'in this conversation';
    return Material(
      key: const Key('full-access-banner'),
      color: scheme.errorContainer,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
        child: Row(children: [
          Icon(Icons.gpp_maybe_outlined, color: scheme.onErrorContainer),
          const SizedBox(width: 12),
          Expanded(
            child: Text(
              'Full access $whose: each request that offers to allow once is allowed without asking you.',
              style: TextStyle(color: scheme.onErrorContainer),
            ),
          ),
          TextButton(
            onPressed: onTurnOff,
            style: TextButton.styleFrom(foregroundColor: scheme.onErrorContainer),
            child: const Text('Turn off'),
          ),
        ]),
      ),
    );
  }
}

/// Asks before full access is turned on for [session], saying what it does. True when the person turned it on.
Future<bool> confirmFullAccess(BuildContext context, ChatSession session) async =>
    await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Full access for this conversation?'),
        content: SizedBox(
          width: 420,
          child: Text(
            'Each request ${shortText(session.agent) ?? 'the agent'} makes in this conversation that offers to allow once will be allowed without asking you, '
            'and said in the conversation. A request offering no way to allow once is still yours to answer. It lasts until you turn '
            'it off, and it changes nothing the browser lets pages or agents do.',
          ),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
          FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('Turn on full access')),
        ],
      ),
    ) ??
    false;
