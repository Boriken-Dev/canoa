import 'package:coqui_acp/coqui_acp.dart' show AcpPermissionOption;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/foundation.dart' show TargetPlatform, defaultTargetPlatform;
import 'package:flutter/material.dart';

import '../session_store.dart' show PermissionDiff;

/// The verbs a tool's title may start with, which the question keeps: "Edit calc.js" asks "Allow Claude to edit calc.js?".
const Set<String> _verbs = {
  'edit', 'write', 'read', 'run', 'delete', 'remove', 'create', 'fetch', 'search', 'move', 'rename', 'copy', 'open', 'list',
  'update', 'install', 'execute', 'call', 'use', 'access', 'change', 'modify', 'patch', 'download', 'upload', 'send', 'view',
};

/// The question a permission card asks: the tool's own title as a request when it opens with a verb, else the tool's name.
String permissionQuestion(String agent, String title) {
  final text = title.trim();
  final word = text.split(RegExp(r'\s+')).first.toLowerCase();
  if (text.isNotEmpty && _verbs.contains(word)) return 'Allow $agent to ${text[0].toLowerCase()}${text.substring(1)}?';
  return text.isEmpty ? 'Allow $agent to use a tool?' : 'Allow $agent to use $text?';
}

/// The chord that answers [kind] from anywhere in the window, as this platform writes it.
String _chord(String letter) {
  final mac = defaultTargetPlatform == TargetPlatform.macOS || defaultTargetPlatform == TargetPlatform.iOS;
  return mac ? '⌘⇧$letter' : 'Ctrl+Shift+$letter';
}

/// What an agent is waiting on, laid out as the person decides it: a chip saying it needs them, the question with the agent's
/// reason under it, what it would do inline at the card's width, then Deny beside a split primary button. The primary is the
/// agent's "always" where it offers one, and the one-time answer and anything else it offers are in its menu.
class SessionRequestCard extends StatelessWidget {
  const SessionRequestCard({
    super.key,
    required this.agent,
    required this.title,
    required this.options,
    required this.onChoose,
    this.reason,
    this.detail,
    this.diff,
    this.bypassPassed = false,
    this.forPerson = false,
    this.more = 0,
    this.onCancel,
    this.notes = const [],
    this.extraAnswers = const [],
    this.host = 'This app',
  });

  /// Narrower than this the key hints stay out: they belong to a keyboard.
  static const double hintsFrom = 560;

  /// The most height what the tool would do takes before it scrolls.
  static const double shown = 260;

  final String agent, title, host;
  final String? reason, detail;
  final PermissionDiff? diff;
  final List<AcpPermissionOption> options;
  final bool bypassPassed, forPerson;
  final int more;
  final ValueChanged<AcpPermissionOption> onChoose;
  final VoidCallback? onCancel;
  final List<Widget> notes;

  /// Buttons of the host's own, before the agent's answers.
  final List<Widget> extraAnswers;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final small = theme.textTheme.labelSmall;
    final look = context.chatLook;
    final name = shortLine(agent) ?? 'The agent';
    final deny = options.where((option) => option.kind == 'reject_once').firstOrNull;
    final always = options.where((option) => option.kind == 'allow_always').firstOrNull;
    final once = options.where((option) => option.kind == 'allow_once').firstOrNull;
    final primary = always ?? once;
    // One-time first, then what the card does not know, then the refusals for good.
    int rank(AcpPermissionOption option) => switch (option.kind) { 'allow_once' => 0, 'reject_always' => 2, _ => 1 };
    final menu = [
      for (final option in options)
        if (option != deny && option != primary) option,
    ]..sort((a, b) => rank(a) - rank(b));
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.only(bottom: 6),
            child: Container(
              key: const Key('needs-input'),
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 3),
              decoration: BoxDecoration(color: look.warning.withValues(alpha: 0.16), borderRadius: BorderRadius.circular(99)),
              child: Row(mainAxisSize: MainAxisSize.min, children: [
                Icon(Icons.circle, size: 8, color: look.warning),
                const SizedBox(width: 6),
                Text('Needs your input', style: theme.textTheme.labelMedium?.copyWith(color: look.warning, fontWeight: FontWeight.w600)),
              ]),
            ),
          ),
          Card(
            key: const Key('permission-card'),
            margin: EdgeInsets.zero,
            child: Padding(
              padding: const EdgeInsets.all(14),
              child: LayoutBuilder(builder: (context, constraints) {
                final hints = constraints.maxWidth >= hintsFrom;
                return Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text(permissionQuestion(name, title), style: theme.textTheme.titleMedium),
                    if (reason?.isNotEmpty ?? false)
                      Padding(
                        padding: const EdgeInsets.only(top: 2),
                        child: Text(reason!, style: theme.textTheme.bodySmall?.copyWith(color: look.muted)),
                      ),
                    if (diff != null || (detail?.isNotEmpty ?? false))
                      Padding(padding: const EdgeInsets.only(top: 10), child: _Inline(diff: diff, detail: detail)),
                    if (always != null)
                      Padding(
                        padding: const EdgeInsets.only(top: 10),
                        child: Text(
                          'An "always" answer is remembered by $name, not by $host: $host cannot see or undo what it keeps.',
                          style: small,
                        ),
                      ),
                    for (final note in notes) Padding(padding: const EdgeInsets.only(top: 4), child: note),
                    if (bypassPassed)
                      Padding(
                        padding: const EdgeInsets.only(top: 4),
                        child: Text(
                          forPerson
                              ? 'Bypass leaves a question or a plan to you, so this one is yours to answer.'
                              : 'Bypass answers only a request that offers to allow, so this one is yours to answer.',
                          style: small?.copyWith(fontWeight: FontWeight.w600),
                        ),
                      ),
                    if (options.isEmpty)
                      Padding(padding: const EdgeInsets.only(top: 4), child: Text('The agent offered no answer $host can give.', style: small)),
                    if (more > 0) Padding(padding: const EdgeInsets.only(top: 4), child: Text('$more more after this', style: small)),
                    const SizedBox(height: 12),
                    Wrap(
                      spacing: 8,
                      runSpacing: 8,
                      alignment: WrapAlignment.end,
                      crossAxisAlignment: WrapCrossAlignment.center,
                      children: [
                        if (deny != null)
                          TextButton(
                            key: const Key('permission-deny'),
                            onPressed: () => onChoose(deny),
                            child: Row(mainAxisSize: MainAxisSize.min, children: [
                              const Text('Deny'),
                              if (hints) ...[const SizedBox(width: 8), _Hint(_chord('R'))],
                            ]),
                          ),
                        ...extraAnswers,
                        if (primary != null) _Split(primary: primary, menu: menu, hints: hints, onChoose: onChoose),
                        if (primary == null)
                          for (final option in menu)
                            OutlinedButton(
                              key: Key('answer-${option.kind}'),
                              onPressed: () => onChoose(option),
                              child: Text(_name(option), maxLines: 2, overflow: TextOverflow.ellipsis),
                            ),
                        if (options.isEmpty && onCancel != null) TextButton(onPressed: onCancel, child: const Text('Cancel the request')),
                      ],
                    ),
                  ],
                );
              }),
            ),
          ),
        ],
      ),
    );
  }
}

String _name(AcpPermissionOption option) => shortLine(option.name) ?? switch (option.kind) {
  'allow_once' => 'Allow once',
  'allow_always' => 'Allow always',
  'reject_once' => 'Deny',
  'reject_always' => 'Never allow',
  _ => shortLine(option.optionId) ?? 'An answer',
};

/// A key hint, as a small cap beside what it answers.
class _Hint extends StatelessWidget {
  const _Hint(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
      decoration: BoxDecoration(border: Border.all(color: look.border), borderRadius: BorderRadius.circular(4)),
      child: Text(text, style: Theme.of(context).textTheme.labelSmall?.copyWith(color: look.muted)),
    );
  }
}

/// The primary answer, with the others in a menu beside it.
class _Split extends StatelessWidget {
  const _Split({required this.primary, required this.menu, required this.hints, required this.onChoose});

  final AcpPermissionOption primary;
  final List<AcpPermissionOption> menu;
  final bool hints;
  final ValueChanged<AcpPermissionOption> onChoose;

  @override
  Widget build(BuildContext context) {
    final onePiece = menu.isEmpty;
    const round = Radius.circular(20);
    final left = RoundedRectangleBorder(
      borderRadius: onePiece ? BorderRadius.circular(20) : const BorderRadius.horizontal(left: round, right: Radius.circular(4)),
    );
    final look = context.chatLook;
    return Row(mainAxisSize: MainAxisSize.min, children: [
      FilledButton(
        key: const Key('permission-primary'),
        style: FilledButton.styleFrom(shape: left),
        onPressed: () => onChoose(primary),
        child: Row(mainAxisSize: MainAxisSize.min, children: [
          Flexible(child: Text(_name(primary), maxLines: 2, overflow: TextOverflow.ellipsis)),
          if (hints && primary.kind == 'allow_once') ...[const SizedBox(width: 8), _Hint(_chord('A'))],
        ]),
      ),
      if (!onePiece) ...[
        const SizedBox(width: 2),
        MenuAnchor(
          menuChildren: [
            for (final option in menu)
              MenuItemButton(
                key: Key('answer-${option.kind}'),
                onPressed: () => onChoose(option),
                trailingIcon: hints && option.kind == 'allow_once'
                    ? Padding(padding: const EdgeInsets.only(left: 12), child: Text(_chord('A'), style: TextStyle(color: look.muted, fontSize: 12)))
                    : null,
                child: Text(_name(option)),
              ),
          ],
          builder: (context, controller, _) => FilledButton(
            key: const Key('permission-more'),
            style: FilledButton.styleFrom(
              minimumSize: const Size(40, 40),
              padding: EdgeInsets.zero,
              shape: const RoundedRectangleBorder(borderRadius: BorderRadius.horizontal(left: Radius.circular(4), right: round)),
            ),
            onPressed: () => controller.isOpen ? controller.close() : controller.open(),
            child: const Icon(Icons.arrow_drop_down),
          ),
        ),
      ],
    ]);
  }
}

/// What the tool would do: the edit as a diff, else the command or input as text, in a box across the card that scrolls past [SessionRequestCard.shown].
class _Inline extends StatelessWidget {
  const _Inline({this.diff, this.detail});

  final PermissionDiff? diff;
  final String? detail;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final mono = TextStyle(fontFamily: look.codeFont, fontSize: 12.5);
    final change = diff;
    final body = change != null
        ? Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            if (change.path.isNotEmpty)
              Padding(padding: const EdgeInsets.only(bottom: 4), child: SelectableText(change.path, style: mono.copyWith(color: look.muted))),
            SizedBox(width: double.infinity, child: DiffView(lines: withContext(diffLines(change.oldText, change.newText)))),
          ])
        : SelectableText(detail!, style: mono);
    return Container(
      key: const Key('permission-detail'),
      width: double.infinity,
      decoration: BoxDecoration(color: look.code, borderRadius: BorderRadius.circular(8), border: Border.all(color: look.border)),
      constraints: const BoxConstraints(maxHeight: SessionRequestCard.shown),
      padding: const EdgeInsets.all(10),
      child: SingleChildScrollView(child: body),
    );
  }
}
