import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

import '../acp/delegate.dart';
import '../sources/source_icons.dart';
import 'delegation_view.dart';

/// Rebuilds [builder] once a second while [active], so a length on screen keeps counting.
class Ticking extends StatefulWidget {
  const Ticking({super.key, required this.active, required this.builder});

  final bool active;
  final WidgetBuilder builder;

  @override
  State<Ticking> createState() => _TickingState();
}

class _TickingState extends State<Ticking> {
  Timer? _timer;

  void _sync() {
    if (widget.active && _timer == null) {
      _timer = Timer.periodic(const Duration(seconds: 1), (_) {
        if (mounted) setState(() {});
      });
    } else if (!widget.active) {
      _timer?.cancel();
      _timer = null;
    }
  }

  @override
  void initState() {
    super.initState();
    _sync();
  }

  @override
  void didUpdateWidget(Ticking oldWidget) {
    super.didUpdateWidget(oldWidget);
    _sync();
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => widget.builder(context);
}

/// "1 subagent", "4 subagents".
String subagents(int count) => '$count subagent${count == 1 ? '' : 's'}';

/// A task's state in the look's colours: what failed reads as an error, what waits for the person as a warning.
Color stateColor(ChatLook look, ChildView child) => switch (child.phase) {
  _ when child.failed => look.danger,
  _ when child.phase.waitingOnPerson => look.warning,
  _ when child.phase == TaskPhase.completed => look.success,
  _ => look.muted,
};

/// The source marks of a card, overlapped, and how many more sources there are than it draws.
class _Marks extends StatelessWidget {
  const _Marks({required this.providers});

  final List<String> providers;

  static const int shown = 3;
  static const double size = 20, step = 13;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final drawn = providers.take(shown).toList();
    final more = providers.length - drawn.length;
    return Row(mainAxisSize: MainAxisSize.min, children: [
      SizedBox(
        width: size + step * (drawn.length - 1),
        height: size,
        child: Stack(children: [
          for (var i = 0; i < drawn.length; i++)
            Positioned(
              left: step * i,
              child: Container(
                width: size,
                height: size,
                alignment: Alignment.center,
                decoration: BoxDecoration(color: look.raised, shape: BoxShape.circle, border: Border.all(color: look.surface, width: 1.5)),
                child: SourceMark.provider(drawn[i], size: 12),
              ),
            ),
        ]),
      ),
      if (more > 0) Padding(padding: const EdgeInsets.only(left: 4), child: Text('+$more', style: TextStyle(color: look.muted, fontSize: 12))),
    ]);
  }
}

/// One row in the transcript for every child a turn handed out: the sources' marks, "4 subagents", "4 working", how long, and a
/// line for each child that opens under it. A line opens the child's thread.
class DelegationRow extends StatefulWidget {
  const DelegationRow({super.key, required this.view, required this.now, this.onOpen, this.initiallyOpen = false});

  final DelegationView view;
  final DateTime Function() now;
  final void Function(String taskId)? onOpen;
  final bool initiallyOpen;

  @override
  State<DelegationRow> createState() => _DelegationRowState();
}

class _DelegationRowState extends State<DelegationRow> {
  late bool _open = widget.initiallyOpen;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final view = widget.view;
    final providers = <String>[];
    for (final child in view.children) {
      if (!providers.contains(child.provider)) providers.add(child.provider);
    }
    final small = TextStyle(color: look.muted, fontSize: 13);
    return Ticking(
      active: view.working > 0,
      builder: (context) {
        final elapsed = view.elapsedAt(widget.now());
        return Container(
          key: const Key('delegation-row'),
          decoration: BoxDecoration(
            color: look.surface,
            borderRadius: BorderRadius.circular(look.radius),
            border: Border.all(color: look.border),
          ),
          child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            InkWell(
              key: const Key('delegation-toggle'),
              borderRadius: BorderRadius.circular(look.radius),
              onTap: () => setState(() => _open = !_open),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
                child: Row(children: [
                  _Marks(providers: providers),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Wrap(spacing: 10, runSpacing: 2, crossAxisAlignment: WrapCrossAlignment.center, children: [
                      Text(subagents(view.children.length), style: TextStyle(color: look.text, fontWeight: FontWeight.w600, fontSize: 14)),
                      if (view.working > 0) Text('${view.working} working', style: small),
                      if (view.working == 0 && view.failed == 0 && view.stopped == 0) Text('all done', style: small.copyWith(color: look.success)),
                      if (view.failed > 0) Text('${view.failed} failed', style: small.copyWith(color: look.danger)),
                      if (view.stopped > 0) Text('${view.stopped} stopped', style: small),
                    ]),
                  ),
                  if (elapsed != null) ...[const SizedBox(width: 8), Text(spanLabel(elapsed), style: small)],
                  Icon(_open ? Icons.expand_less : Icons.expand_more, size: 18, color: look.muted),
                ]),
              ),
            ),
            if (_open)
              for (final child in view.children)
                _ChildLine(child: child, elapsed: child.elapsedAt(widget.now()), onOpen: widget.onOpen),
            if (_open) const SizedBox(height: 4),
          ]),
        );
      },
    );
  }
}

class _ChildLine extends StatelessWidget {
  const _ChildLine({required this.child, required this.elapsed, required this.onOpen});

  final ChildView child;
  final Duration? elapsed;
  final void Function(String taskId)? onOpen;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final color = stateColor(look, child);
    return InkWell(
      key: Key('child-${child.id}'),
      onTap: onOpen == null ? null : () => onOpen!(child.id),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
        child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          Row(children: [
            SizedBox.square(dimension: 18, child: Center(child: SourceMark.provider(child.provider, size: 14))),
            const SizedBox(width: 10),
            Expanded(child: Text(child.title, maxLines: 1, overflow: TextOverflow.ellipsis, style: TextStyle(color: look.text, fontSize: 14))),
            const SizedBox(width: 8),
            if (child.failed) Padding(padding: const EdgeInsets.only(right: 4), child: Icon(Icons.error_outline, size: 14, color: color)),
            Text(child.word, style: TextStyle(color: color, fontSize: 13)),
            if (elapsed != null) Padding(padding: const EdgeInsets.only(left: 8), child: Text(spanLabel(elapsed!), style: TextStyle(color: look.muted, fontSize: 13))),
          ]),
          if (child.failed && child.message != null)
            Padding(
              padding: const EdgeInsets.only(left: 28, top: 2),
              child: Text(child.message!, maxLines: 2, overflow: TextOverflow.ellipsis, style: TextStyle(color: look.muted, fontSize: 12)),
            ),
        ]),
      ),
    );
  }
}

/// The line under the conversation while its turn waits on children: "Waiting on 4 subagents", their titles, and one Stop for all of
/// them.
class DelegationWaiting extends StatelessWidget {
  const DelegationWaiting({super.key, required this.children, required this.onStop, this.listed = 6});

  /// The children that have not ended.
  final List<ChildView> children;
  final VoidCallback onStop;

  /// How many titles to list before "and N more".
  final int listed;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final shown = children.take(listed).toList();
    final more = children.length - shown.length;
    return Container(
      key: const Key('delegation-waiting'),
      width: double.infinity,
      padding: const EdgeInsets.fromLTRB(16, 8, 8, 8),
      decoration: BoxDecoration(color: look.surface, border: Border(top: BorderSide(color: look.border))),
      child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
        const Padding(padding: EdgeInsets.only(top: 3), child: SizedBox.square(dimension: 14, child: CircularProgressIndicator(strokeWidth: 2))),
        const SizedBox(width: 10),
        Expanded(
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Text('Waiting on ${subagents(children.length)}', style: TextStyle(color: look.text, fontWeight: FontWeight.w600, fontSize: 14)),
            for (final child in shown) Text(child.title, maxLines: 1, overflow: TextOverflow.ellipsis, style: TextStyle(color: look.muted, fontSize: 13)),
            if (more > 0) Text('and $more more', style: TextStyle(color: look.muted, fontSize: 13)),
          ]),
        ),
        TextButton.icon(
          key: const Key('delegation-stop'),
          onPressed: onStop,
          icon: const Icon(Icons.stop_circle_outlined, size: 18),
          label: const Text('Stop'),
        ),
      ]),
    );
  }
}
