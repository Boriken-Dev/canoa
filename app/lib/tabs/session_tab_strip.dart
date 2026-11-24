import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLookOf;
import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../rail/session_groups.dart' show RowState, rowStateOf;
import '../session_store.dart';
import '../theme/look.dart';

/// One entry of a tab's menu.
final class TabAction {
  const TabAction(this.key, this.label, this.icon, this.run);

  final Key key;
  final String label;
  final IconData icon;
  final VoidCallback run;
}

/// The tabs of one window: a title and a state mark for each session, the one shown marked, and a way to close, reorder and move
/// each. It holds nothing but the position of its scroll: which tabs there are, and in what order, is [sessions].
///
/// On a narrow screen the tabs are narrower, the strip scrolls sideways to keep the shown tab in view, a tab is dragged after a
/// long press (a plain drag scrolls), and only the shown tab carries its menu and close buttons.
class SessionTabStrip extends StatefulWidget {
  const SessionTabStrip({
    super.key,
    required this.sessions,
    required this.current,
    required this.onSelect,
    required this.onReorder,
    required this.actions,
    this.onClose,
    this.narrow = false,
  });

  final List<ChatSession> sessions;

  /// The id of the session the window shows.
  final String? current;
  final bool narrow;
  final ValueChanged<ChatSession> onSelect;

  /// Called with a tab's index and the index it takes once it has left its own place.
  final void Function(int from, int to) onReorder;

  /// What a tab's menu offers.
  final List<TabAction> Function(ChatSession session) actions;

  /// Closes a tab; null where this window must keep the one it has, which hides the button.
  final ValueChanged<ChatSession>? onClose;

  @override
  State<SessionTabStrip> createState() => _SessionTabStripState();
}

class _SessionTabStripState extends State<SessionTabStrip> {
  final _scroll = ScrollController();
  final _keys = <String, GlobalKey>{};

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _showCurrent(immediately: true));
  }

  void _showCurrent({bool immediately = false}) {
    if (!mounted) return;
    final shown = _keys[widget.current]?.currentContext;
    if (shown != null) {
      unawaited(Scrollable.ensureVisible(shown, duration: immediately ? Duration.zero : const Duration(milliseconds: 120)));
    } else if (_scroll.hasClients) {
      // A tab far along the strip is not built yet: go near it, then bring it fully into view.
      final at = widget.sessions.indexWhere((session) => session.id == widget.current);
      if (at < 0) return;
      _scroll.jumpTo((at * (widget.narrow ? 100.0 : 130.0)).clamp(0.0, _scroll.position.maxScrollExtent));
      WidgetsBinding.instance.addPostFrameCallback((_) {
        final built = _keys[widget.current]?.currentContext;
        if (built != null && mounted) unawaited(Scrollable.ensureVisible(built, duration: Duration.zero));
      });
    }
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  @override
  void didUpdateWidget(SessionTabStrip old) {
    super.didUpdateWidget(old);
    if (old.current != widget.current) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _showCurrent());
    }
  }

  Future<void> _menu(BuildContext tab, ChatSession session, Offset at) async {
    final actions = widget.actions(session);
    if (actions.isEmpty) return;
    final chosen = await showMenu<TabAction>(
      context: tab,
      position: RelativeRect.fromLTRB(at.dx, at.dy, at.dx, at.dy),
      items: [
        for (final action in actions)
          PopupMenuItem(key: action.key, value: action, child: ListTile(dense: true, leading: Icon(action.icon), title: Text(action.label))),
      ],
    );
    chosen?.run();
  }

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final narrow = widget.narrow;
    return Material(
      color: look.rail,
      child: SizedBox(
        height: narrow ? 36 : 40,
        child: ReorderableListView.builder(
          key: const Key('session-tabs'),
          scrollController: _scroll,
          scrollDirection: Axis.horizontal,
          buildDefaultDragHandles: false,
          padding: const EdgeInsets.symmetric(horizontal: 6),
          itemCount: widget.sessions.length,
          onReorderItem: widget.onReorder,
          proxyDecorator: (child, _, _) => Material(color: Colors.transparent, child: child),
          itemBuilder: (context, index) {
            final session = widget.sessions[index];
            final tab = _Tab(
              key: _keys.putIfAbsent(session.id, GlobalKey.new),
              session: session,
              shown: session.id == widget.current,
              narrow: narrow,
              onTap: () => widget.onSelect(session),
              onClose: widget.onClose == null ? null : () => widget.onClose!(session),
              onMenu: (tab, at) => _menu(tab, session, at),
              hasMenu: widget.actions(session).isNotEmpty,
            );
            final key = ValueKey('tab-${session.id}');
            return narrow
                ? ReorderableDelayedDragStartListener(key: key, index: index, child: tab)
                : ReorderableDragStartListener(key: key, index: index, child: tab);
          },
        ),
      ),
    );
  }
}

class _Tab extends StatelessWidget {
  const _Tab({
    super.key,
    required this.session,
    required this.shown,
    required this.narrow,
    required this.onTap,
    required this.onClose,
    required this.onMenu,
    required this.hasMenu,
  });

  final ChatSession session;
  final bool shown, narrow, hasMenu;
  final VoidCallback onTap;
  final VoidCallback? onClose;
  final Future<void> Function(BuildContext tab, Offset at) onMenu;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    final theme = Theme.of(context);
    final state = rowStateOf(session);
    final showButtons = !narrow || shown;
    return Builder(
      builder: (tab) => GestureDetector(
        onSecondaryTapUp: hasMenu ? (details) => onMenu(tab, details.globalPosition) : null,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 2, vertical: 4),
          child: Material(
            color: shown ? look.bar : Colors.transparent,
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(8),
              side: BorderSide(color: shown ? look.border : Colors.transparent),
            ),
            child: InkWell(
              key: Key('tab-open-${session.id}'),
              borderRadius: BorderRadius.circular(8),
              onTap: onTap,
              child: ConstrainedBox(
                constraints: BoxConstraints(minWidth: narrow ? 88 : 112, maxWidth: narrow ? 160 : 220),
                child: Padding(
                  padding: const EdgeInsets.only(left: 10, right: 2),
                  child: Row(mainAxisSize: MainAxisSize.min, children: [
                    TabStateMark(state: state),
                    if (state != RowState.idle) const SizedBox(width: 6),
                    Flexible(
                      child: Text(
                        session.title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: theme.textTheme.bodyMedium?.copyWith(fontWeight: shown ? FontWeight.w600 : FontWeight.w400, color: shown ? look.text : look.muted),
                      ),
                    ),
                    if (showButtons && hasMenu)
                      Builder(
                        builder: (button) => IconButton(
                          key: Key('tab-menu-${session.id}'),
                          tooltip: context.t('tab.menu'),
                          visualDensity: VisualDensity.compact,
                          iconSize: 18,
                          padding: EdgeInsets.zero,
                          constraints: const BoxConstraints.tightFor(width: 24, height: 24),
                          icon: const Icon(Icons.arrow_drop_down),
                          onPressed: () {
                            final box = button.findRenderObject()! as RenderBox;
                            unawaited(onMenu(tab, box.localToGlobal(Offset(0, box.size.height))));
                          },
                        ),
                      ),
                    if (showButtons && onClose != null)
                      IconButton(
                        key: Key('tab-close-${session.id}'),
                        tooltip: context.t('tab.close'),
                        visualDensity: VisualDensity.compact,
                        iconSize: 14,
                        padding: EdgeInsets.zero,
                        constraints: const BoxConstraints.tightFor(width: 24, height: 24),
                        icon: const Icon(Icons.close),
                        onPressed: onClose,
                      )
                    else
                      const SizedBox(width: 8),
                  ]),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// The mark of a state a tab reports: nothing for a session with nothing to say. A mark has no animation, so a window of tabs
/// settles.
class TabStateMark extends StatelessWidget {
  const TabStateMark({super.key, required this.state});

  final RowState state;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final muted = context.chatLook.muted;
    final (IconData, Color)? mark = switch (state) {
      RowState.needsYou => (Icons.front_hand_outlined, scheme.error),
      RowState.working || RowState.starting => (Icons.autorenew, scheme.primary),
      RowState.waiting => (Icons.hourglass_top, muted),
      RowState.failed => (Icons.error_outline, scheme.error),
      RowState.done => (Icons.circle, scheme.primary),
      RowState.offline => (Icons.cloud_off_outlined, muted),
      _ => null,
    };
    if (mark == null) return const SizedBox.shrink();
    return Tooltip(
      message: state.word,
      child: Semantics(label: state.word, child: Icon(mark.$1, key: Key('tab-state-${state.name}'), size: state == RowState.done ? 10 : 14, color: mark.$2)),
    );
  }
}
