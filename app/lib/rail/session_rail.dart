import 'dart:async';

import 'package:flutter/foundation.dart' show TargetPlatform, defaultTargetPlatform;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../connect_dialog.dart';
import '../ledger/ledger_list.dart';
import '../ledger/session_ledger.dart';
import '../selectable.dart';
import '../session_store.dart';
import '../theme/canoa_mark.dart';
import '../theme/look.dart';
import 'session_groups.dart';

import 'package:coqui_acp/page.dart' show findAcp;

/// The left column: the wordmark, labelled entries for what the person starts or opens, the sessions grouped by what they have
/// for the person, and the conversations the ledger keeps. On a phone it is the drawer.
class SessionRail extends StatefulWidget {
  const SessionRail({
    super.key,
    required this.store,
    required this.onConnect,
    required this.onUseBrowserAgents,
    required this.onNewSession,
    required this.onSettings,
    required this.onSearch,
    required this.onClose,
    this.onHosting,
    this.onReopen,
    this.now = DateTime.now,
    this.touch,
  });

  final SessionStore store;
  final Future<void> Function(ConnectTarget) onConnect;
  final UseBrowserAgents onUseBrowserAgents;
  final VoidCallback onNewSession;
  final VoidCallback onSettings;

  /// Opens the palette on the sessions.
  final VoidCallback onSearch;

  /// Ends a conversation here, asking first about anything it made.
  final Future<void> Function(ChatSession) onClose;
  final VoidCallback? onHosting;
  final Future<void> Function(LedgerEntry)? onReopen;

  /// The time ages are counted from.
  final DateTime Function() now;

  /// Whether the menu button always shows, as a touch screen needs; null follows the platform.
  final bool? touch;

  @override
  State<SessionRail> createState() => _SessionRailState();
}

bool get _touchPlatform => defaultTargetPlatform == TargetPlatform.android || defaultTargetPlatform == TargetPlatform.iOS;

class _SessionRailState extends State<SessionRail> {
  /// The folded groups the person opened.
  final Set<SessionGroup> _opened = {};

  void _closeDrawer(BuildContext context) {
    if (Scaffold.maybeOf(context)?.isDrawerOpen ?? false) Navigator.pop(context);
  }

  Future<void> _rename(BuildContext context, ChatSession session) async {
    final named = await showSelectableDialog<String>(context: context, builder: (_) => _RenameDialog(session.title));
    if (named != null) widget.store.rename(session, named);
  }

  @override
  Widget build(BuildContext context) {
    final store = widget.store;
    return Material(
      color: context.look.rail,
      child: Column(children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 18, 10, 6),
          child: Align(
            alignment: Alignment.centerLeft,
            child: Row(mainAxisSize: MainAxisSize.min, children: [
              CanoaMark(color: context.look.accent),
              const SizedBox(width: 10),
              const Text('Canoa', softWrap: false, style: TextStyle(fontSize: 22, fontWeight: FontWeight.w700)),
            ]),
          ),
        ),
        _NavEntry(
          icon: Icons.add_comment_outlined,
          label: 'New session',
          hint: chordLabel('N'),
          onTap: () {
            widget.onNewSession();
            _closeDrawer(context);
          },
        ),
        _NavEntry(
          icon: Icons.search,
          label: 'Search',
          hint: chordLabel('K'),
          onTap: () {
            _closeDrawer(context);
            widget.onSearch();
          },
        ),
        const SizedBox(height: 6),
        const Divider(height: 1),
        Expanded(
          child: ListenableBuilder(
            listenable: store,
            builder: (context, _) => _list(context),
          ),
        ),
        const Divider(height: 1),
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 6),
          child: Column(children: [
            _NavEntry(
              icon: Icons.link_rounded,
              label: 'Connect',
              tooltip: 'Connect to verified agent',
              onTap: () => showCanoaConnectDialog(
                context,
                widget.onConnect,
                browserProvider: findAcp(timeout: const Duration(milliseconds: 300)),
                onUseBrowserAgents: widget.onUseBrowserAgents,
              ),
            ),
            if (widget.onHosting != null) _NavEntry(icon: Icons.dns_outlined, label: 'Hosting', onTap: widget.onHosting!),
            _NavEntry(icon: Icons.settings_outlined, label: 'Settings', tooltip: 'Canoa settings', onTap: widget.onSettings),
          ]),
        ),
      ]),
    );
  }

  Widget _list(BuildContext context) {
    final store = widget.store;
    final groups = groupSessions(store.sessions);
    final selected = store.selected;
    final now = widget.now();
    return ListView(
      padding: const EdgeInsets.only(top: 4, bottom: 8),
      children: [
        for (final MapEntry(key: group, value: sessions) in groups.entries) ...[
          // A folded group opens by itself for the session the person is in, so it is never out of sight.
          if (group.folds)
            _FoldHeader(
              group: group,
              count: sessions.length,
              open: _opened.contains(group) || (selected != null && sessions.contains(selected)),
              onTap: () => setState(() => _opened.contains(group) ? _opened.remove(group) : _opened.add(group)),
            )
          else
            _GroupHeader(label: group.label, count: sessions.length),
          if (!group.folds || _opened.contains(group) || (selected != null && sessions.contains(selected)))
            for (final session in sessions)
              _SessionRow(
                key: ValueKey('row ${session.id}'),
                store: store,
                session: session,
                active: session.id == store.selectedId,
                now: now,
                touch: widget.touch ?? _touchPlatform,
                onOpen: () {
                  store.select(session.id);
                  _closeDrawer(context);
                },
                onRename: () => unawaited(_rename(context, session)),
                onFork: () {
                  store.fork(session);
                  _closeDrawer(context);
                },
                onClose: () => unawaited(widget.onClose(session)),
              ),
        ],
        if (store.ledger != null) LedgerList(ledger: store.ledger!, store: store, onReopen: widget.onReopen),
      ],
    );
  }
}

/// A shortcut as the person reads it on this platform: "Ctrl+N", or "⌘N" on a Mac.
String chordLabel(String key) => defaultTargetPlatform == TargetPlatform.macOS ? '⌘$key' : 'Ctrl+$key';

class _NavEntry extends StatelessWidget {
  const _NavEntry({required this.icon, required this.label, required this.onTap, this.hint, this.tooltip});

  final IconData icon;
  final String label;
  final String? hint, tooltip;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    return Tooltip(
      message: tooltip ?? label,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8),
        child: ListTile(
          dense: true,
          visualDensity: const VisualDensity(vertical: -2),
          shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
          leading: Icon(icon, size: 20),
          horizontalTitleGap: 8,
          title: Text(label, maxLines: 1, overflow: TextOverflow.ellipsis),
          trailing: hint == null ? null : Text(hint!, style: TextStyle(color: look.muted, fontSize: 11)),
          onTap: onTap,
        ),
      ),
    );
  }
}

class _GroupHeader extends StatelessWidget {
  const _GroupHeader({required this.label, required this.count});

  final String label;
  final int count;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(18, 10, 12, 2),
    child: Text(
      label,
      style: TextStyle(color: context.look.muted, fontSize: 12, fontWeight: FontWeight.w600),
    ),
  );
}

/// A group folded away, as "Settled 3 ›": it opens on a tap.
class _FoldHeader extends StatelessWidget {
  const _FoldHeader({required this.group, required this.count, required this.open, required this.onTap});

  final SessionGroup group;
  final int count;
  final bool open;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => InkWell(
    key: ValueKey('fold ${group.name}'),
    onTap: onTap,
    child: Padding(
      padding: const EdgeInsets.fromLTRB(18, 10, 12, 4),
      child: Row(children: [
        Text(
          '${group.label} $count',
          style: TextStyle(color: context.look.muted, fontSize: 12, fontWeight: FontWeight.w600),
        ),
        const SizedBox(width: 2),
        Icon(open ? Icons.expand_more : Icons.chevron_right, size: 16, color: context.look.muted),
      ]),
    ),
  );
}

/// One session: its title on a line; under it its state as an icon and a word, then its branch or folder, its agent and how
/// long ago it was last active. A menu holds what can be done to it.
class _SessionRow extends StatefulWidget {
  const _SessionRow({
    super.key,
    required this.store,
    required this.session,
    required this.active,
    required this.now,
    required this.touch,
    required this.onOpen,
    required this.onRename,
    required this.onFork,
    required this.onClose,
  });

  final SessionStore store;
  final ChatSession session;
  final bool active;
  final DateTime now;
  final bool touch;
  final VoidCallback onOpen, onRename, onFork, onClose;

  @override
  State<_SessionRow> createState() => _SessionRowState();
}

class _SessionRowState extends State<_SessionRow> {
  final MenuController _menu = MenuController();
  bool _hover = false, _menuOpen = false;


  /// Closes the menu, then runs an item of it once the menu is gone from the tree.
  VoidCallback _do(VoidCallback action) => () {
    _menu.close();
    WidgetsBinding.instance.addPostFrameCallback((_) => action());
    WidgetsBinding.instance.ensureVisualUpdate();
  };

  List<Widget> _items() {
    final session = widget.session;
    final actions = <(String, IconData, SingleActivator?, VoidCallback)>[
      (session.pinned ? 'Unpin' : 'Pin', session.pinned ? Icons.push_pin : Icons.push_pin_outlined,
        const SingleActivator(LogicalKeyboardKey.keyP), () => widget.store.setPinned(session, !session.pinned)),
      ('Rename', Icons.edit_outlined, const SingleActivator(LogicalKeyboardKey.keyR), widget.onRename),
      ('Fork', Icons.call_split, const SingleActivator(LogicalKeyboardKey.keyF), widget.onFork),
      (session.archived ? 'Unarchive' : 'Archive', Icons.inventory_2_outlined,
        const SingleActivator(LogicalKeyboardKey.keyA), () => widget.store.setArchived(session, !session.archived)),
      ('Close', Icons.close, const SingleActivator(LogicalKeyboardKey.keyC), widget.onClose),
    ];
    // The keys work while the menu is open, which is when this is built; none of them is a shortcut of the app.
    return [
      _MenuKeys(
          // Deferred: closing the menu disposes the node that is handling this key.
          keys: {
            for (final (_, _, key, action) in actions)
              key!.trigger: () {
                WidgetsBinding.instance.addPostFrameCallback((_) => _do(action)());
                WidgetsBinding.instance.ensureVisualUpdate();
              },
          },
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            for (final (label, icon, key, action) in actions)
              MenuItemButton(
                leadingIcon: Icon(icon, size: 18),
                shortcut: key,
                onPressed: _do(action),
                child: Text(label),
              ),
          ]),
      ),
    ];
  }

  Widget _stateMark(BuildContext context, RowState state) {
    final scheme = Theme.of(context).colorScheme;
    final muted = context.look.muted;
    return switch (state) {
      RowState.needsYou => Tooltip(message: 'Needs you', child: Icon(Icons.front_hand_outlined, size: 14, color: scheme.error)),
      RowState.working || RowState.starting => Tooltip(
        message: state.word,
        child: const SizedBox.square(dimension: 12, child: CircularProgressIndicator(strokeWidth: 2)),
      ),
      RowState.failed => Tooltip(message: 'The last turn failed', child: Icon(Icons.error_outline, size: 14, color: scheme.error)),
      RowState.done => Tooltip(message: 'New since you looked', child: Icon(Icons.circle, size: 10, color: scheme.primary)),
      RowState.offline => Icon(Icons.cloud_off_outlined, size: 14, color: muted),
      RowState.preview => Icon(Icons.visibility_outlined, size: 14, color: muted),
      RowState.fresh => Icon(Icons.edit_outlined, size: 14, color: muted),
      RowState.idle => Icon(Icons.check, size: 14, color: muted),
    };
  }

  @override
  Widget build(BuildContext context) {
    final session = widget.session;
    final store = widget.store;
    final look = context.look;
    final state = rowStateOf(session);
    final stateColor = switch (state) {
      RowState.needsYou || RowState.failed => Theme.of(context).colorScheme.error,
      RowState.done => Theme.of(context).colorScheme.primary,
      _ => look.muted,
    };
    final where = [
      if (store.inBypass(session)) 'Bypass',
      locationOf(session),
      session.agent,
    ].nonNulls.join(' · ');
    final showMenu = _hover || widget.active || _menuOpen || widget.touch;
    return MenuAnchor(
      controller: _menu,
      onOpen: () => setState(() => _menuOpen = true),
      onClose: () => setState(() => _menuOpen = false),
      menuChildren: _items(),
      child: MouseRegion(
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          behavior: HitTestBehavior.deferToChild,
          onSecondaryTapUp: (details) => _menu.open(position: details.localPosition),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 1),
            child: ListTile(
              dense: true,
              selected: widget.active,
              selectedTileColor: look.selected,
              shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
              contentPadding: const EdgeInsets.only(left: 12, right: 2),
              title: Row(children: [
                Expanded(child: Text(session.title, maxLines: 1, overflow: TextOverflow.ellipsis)),
                if (session.pinned) Icon(Icons.push_pin, size: 12, color: look.muted),
              ]),
              subtitle: Row(children: [
                _stateMark(context, state),
                const SizedBox(width: 5),
                Flexible(child: Text(state.word, maxLines: 1, overflow: TextOverflow.ellipsis, style: TextStyle(color: stateColor, fontSize: 12))),
                Expanded(
                  child: Text(' · $where', maxLines: 1, overflow: TextOverflow.ellipsis, style: TextStyle(color: look.muted, fontSize: 12)),
                ),
                // The age stays whole: the agent's name is what gives way when the column is narrow.
                Text(' · ${ageLabel(widget.now, session.lastActivity)}', maxLines: 1, style: TextStyle(color: look.muted, fontSize: 12)),
              ]),
              trailing: showMenu
                  ? IconButton(
                      tooltip: 'Session actions',
                      iconSize: 18,
                      visualDensity: VisualDensity.compact,
                      icon: const Icon(Icons.more_horiz),
                      onPressed: () => _menu.isOpen ? _menu.close() : _menu.open(),
                    )
                  : const SizedBox(width: 34),
              onTap: widget.onOpen,
            ),
          ),
        ),
      ),
    );
  }
}

/// Takes the keyboard focus while the row's menu is open, and runs the action of the key pressed.
class _MenuKeys extends StatefulWidget {
  const _MenuKeys({required this.keys, required this.child});

  final Map<LogicalKeyboardKey, VoidCallback> keys;
  final Widget child;

  @override
  State<_MenuKeys> createState() => _MenuKeysState();
}

class _MenuKeysState extends State<_MenuKeys> {
  late final FocusNode _focus = FocusNode(debugLabel: 'session menu', onKeyEvent: (node, event) {
    final action = event is KeyDownEvent ? widget.keys[event.logicalKey] : null;
    if (action == null) return KeyEventResult.ignored;
    action();
    return KeyEventResult.handled;
  });

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Focus(focusNode: _focus, child: widget.child);
}

/// Asks for a session's name; answers it, or nothing when cancelled.
class _RenameDialog extends StatefulWidget {
  const _RenameDialog(this.title);

  final String title;

  @override
  State<_RenameDialog> createState() => _RenameDialogState();
}

class _RenameDialogState extends State<_RenameDialog> {
  late final TextEditingController _input = TextEditingController(text: widget.title);

  @override
  void dispose() {
    _input.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Rename session'),
    content: TextField(
      key: const Key('rename-session'),
      controller: _input,
      autofocus: true,
      decoration: const InputDecoration(labelText: 'Name'),
      onSubmitted: (value) => Navigator.pop(context, value),
    ),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
      FilledButton(onPressed: () => Navigator.pop(context, _input.text), child: const Text('Rename')),
    ],
  );
}
