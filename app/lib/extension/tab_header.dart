import 'dart:async';

import 'package:flutter/material.dart';

import '../session_store.dart';
import '../theme/look.dart';
import 'tab_bridge.dart';
import 'tab_shares.dart';

/// Toggles [tab]'s share and says what was refused.
/// [messenger] is taken beforehand: a menu closes before it calls this, and its own context is gone.
Future<void> shareToggle(ScaffoldMessengerState? messenger, TabShares tabs, ChatSession session, OpenTab tab) async {
  try {
    await tabs.toggle(session, tab);
  } on Object catch (error) {
    messenger?.showSnackBar(SnackBar(content: Text('$error'.replaceFirst('Bad state: ', ''))));
  }
}

/// The part of the composer about tabs: where the session was started, what stopped, what it shares and with whom, and the
/// open tabs `@` lists.
class TabHeader extends StatefulWidget {
  const TabHeader({super.key, required this.tabs, required this.session, required this.controller});

  final TabShares tabs;
  final ChatSession session;
  final TextEditingController controller;

  @override
  State<TabHeader> createState() => _TabHeaderState();
}

class _TabHeaderState extends State<TabHeader> {
  static final _at = RegExp(r'(^|\s)@([^\s@]*)$');

  @override
  void initState() {
    super.initState();
    unawaited(widget.tabs.refresh().then((_) => widget.tabs.ensureAttached(sessionKeyOf(widget.session))));
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: Listenable.merge([widget.tabs, widget.controller]),
      builder: (context, _) {
        final tabs = widget.tabs;
        final key = sessionKeyOf(widget.session);
        final machine = widget.session.agent;
        final elsewhere = tabs.startedElsewhere(key);
        final shared = tabs.sharedTabs(key);
        final typed = _at.firstMatch(widget.controller.text);
        final matches = typed == null
            ? const <OpenTab>[]
            : [
                for (final tab in tabs.tabs)
                  if (tab.label.toLowerCase().contains(typed.group(2)!.toLowerCase()) || tab.url.toLowerCase().contains(typed.group(2)!.toLowerCase())) tab,
              ];
        final children = <Widget>[
          if (elsewhere != null)
            _Banner(
              key: const Key('move-here'),
              text: 'Started in "${elsewhere.label}"',
              action: 'Move here',
              onAction: () => unawaited(widget.tabs.moveHere(widget.session)),
            ),
          for (final drop in tabs.dropsFor(key))
            _Banner(
              key: Key('share-dropped-${drop.tabTitle}'),
              text: 'Stopped sharing "${drop.tabTitle}": the tab changed page or closed.',
              onDismiss: () => tabs.dismiss(drop),
            ),
          if (shared.isNotEmpty)
            Wrap(spacing: 6, runSpacing: 4, children: [
              for (final tab in shared)
                _Chip(
                  key: Key('sharing-${tab.id}'),
                  text: 'Sharing "${tab.label}" with $machine',
                  onClose: () => unawaited(shareToggle(ScaffoldMessenger.maybeOf(context), tabs, widget.session, tab)),
                ),
            ]),
          if (typed != null && matches.isNotEmpty)
            _AtList(
              tabs: matches,
              shared: (tab) => tabs.isShared(key, tab.id),
              onPick: (tab) {
                _removeToken();
                unawaited(shareToggle(ScaffoldMessenger.maybeOf(context), tabs, widget.session, tab));
              },
            ),
        ];
        if (children.isEmpty) return const SizedBox.shrink();
        return Padding(
          padding: const EdgeInsets.only(bottom: 4),
          child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: children),
        );
      },
    );
  }

  /// The `@word` the person typed, which was a way to pick a tab and is not part of the message.
  void _removeToken() {
    final text = widget.controller.text;
    final typed = _at.firstMatch(text);
    if (typed == null) return;
    final value = text.substring(0, typed.start + typed.group(1)!.length);
    widget.controller.value = TextEditingValue(text: value, selection: TextSelection.collapsed(offset: value.length));
  }
}

class _Chip extends StatelessWidget {
  const _Chip({super.key, required this.text, required this.onClose});

  final String text;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    return Container(
      padding: const EdgeInsets.only(left: 10),
      decoration: BoxDecoration(border: Border.all(color: look.muted.withValues(alpha: 0.5)), borderRadius: BorderRadius.circular(16)),
      child: Row(mainAxisSize: MainAxisSize.min, children: [
        Icon(Icons.public, size: 14, color: look.accent),
        const SizedBox(width: 6),
        Flexible(child: Text(text, overflow: TextOverflow.ellipsis, style: const TextStyle(fontSize: 12))),
        IconButton(
          tooltip: 'Stop sharing',
          visualDensity: VisualDensity.compact,
          iconSize: 14,
          onPressed: onClose,
          icon: const Icon(Icons.close),
        ),
      ]),
    );
  }
}

class _Banner extends StatelessWidget {
  const _Banner({super.key, required this.text, this.action, this.onAction, this.onDismiss});

  final String text;
  final String? action;
  final VoidCallback? onAction;
  final VoidCallback? onDismiss;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    return Container(
      margin: const EdgeInsets.only(bottom: 4),
      padding: const EdgeInsets.fromLTRB(12, 4, 4, 4),
      decoration: BoxDecoration(color: look.notice, borderRadius: BorderRadius.circular(look.radius)),
      child: Row(children: [
        Expanded(child: Text(text, style: const TextStyle(fontSize: 12))),
        if (action != null) TextButton(onPressed: onAction, child: Text(action!)),
        if (onDismiss != null) IconButton(tooltip: 'Dismiss', visualDensity: VisualDensity.compact, iconSize: 14, onPressed: onDismiss, icon: const Icon(Icons.close)),
      ]),
    );
  }
}

class _AtList extends StatelessWidget {
  const _AtList({required this.tabs, required this.shared, required this.onPick});

  final List<OpenTab> tabs;
  final bool Function(OpenTab tab) shared;
  final void Function(OpenTab tab) onPick;

  @override
  Widget build(BuildContext context) {
    return ConstrainedBox(
      constraints: const BoxConstraints(maxHeight: 180),
      child: ListView(shrinkWrap: true, children: [
        for (final tab in tabs)
          InkWell(
            key: Key('at-tab-${tab.id}'),
            onTap: () => onPick(tab),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 7),
              child: Row(children: [
                Icon(shared(tab) ? Icons.check_circle : Icons.public, size: 18),
                const SizedBox(width: 10),
                Expanded(child: Text(tab.label, overflow: TextOverflow.ellipsis)),
              ]),
            ),
          ),
      ]),
    );
  }
}
