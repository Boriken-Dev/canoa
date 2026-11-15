import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

import '../chat/child_thread.dart';
import '../chat/delegation_row.dart';
import '../chat/delegation_view.dart';
import '../session_store.dart';
import '../shell/surface.dart';
import '../sources/source_icons.dart';
import '../theme/look.dart';

/// The subagents of the session in a panel: the lineage as a tree, each child's thread to read with its task, tools, result and
/// errors, and Stop at three reaches (one, one with everything under it, all) that say what the children are afterwards.
class SubagentsSurface extends WorkSurface {
  const SubagentsSurface();

  static const surfaceId = 'subagents';

  @override
  String get id => surfaceId;
  @override
  String get title => 'Subagents';
  @override
  IconData get icon => Icons.account_tree_outlined;
  @override
  List<SurfaceSlot> get placements => const [SurfaceSlot.bottom, SurfaceSlot.right];

  /// There is something to follow once the session has handed out a task or its agent has started one.
  @override
  bool available(ChatSession session) => !session.isDraft && !session.tasks.isEmpty;

  @override
  Widget build(BuildContext context, ChatSession session, SurfaceController controller) => SubagentsPanel(session: session);
}

/// One thread the person opened in the panel: what the child said and where its reader had scrolled to, kept while the panel is.
class _Page {
  _Page(DelegateTask task) : thread = ChildThread(task);

  final ChildThread thread;
  final ScrollController scroll = ScrollController();
}

/// The panel's body: the list, and a page for each child opened, all kept so going back to one finds it where it was left.
class SubagentsPanel extends StatefulWidget {
  const SubagentsPanel({super.key, required this.session});

  final ChatSession session;

  /// How many opened threads the panel keeps at once.
  static const int kept = 6;

  @override
  State<SubagentsPanel> createState() => _SubagentsPanelState();
}

class _SubagentsPanelState extends State<SubagentsPanel> {
  final Map<String, _Page> _pages = {};
  String? _open;
  String? _said;

  @override
  void dispose() {
    for (final page in _pages.values) {
      page.scroll.dispose();
    }
    super.dispose();
  }

  void _show(String? id) {
    if (id != null && !_pages.containsKey(id)) {
      final task = widget.session.tasks[id];
      if (task == null) return;
      _pages[id] = _Page(task);
      // The oldest page that is not the one shown goes, with its place.
      while (_pages.length > SubagentsPanel.kept) {
        final drop = _pages.keys.firstWhere((key) => key != id);
        _pages.remove(drop)!.scroll.dispose();
      }
      SurfaceEnvironment.of(context).store.loadChildHistory(widget.session, id);
    }
    setState(() {
      _open = id;
      _said = null;
    });
  }

  void _say(String text) {
    if (mounted) setState(() => _said = text);
  }

  static String _plural(int n) => n == 1 ? '1 subagent' : '$n subagents';

  /// What stopping left running, which Canoa cannot reach: the children the conversation's own agent started.
  String _left(Iterable<DelegateTask> tasks) {
    final native = tasks.where((task) => task.native && !task.phase.ended).length;
    return native == 0 ? '' : " ${_plural(native)} started by the conversation's agent keep running: only that agent can stop them.";
  }

  Future<void> _stopOne(SessionStore store, DelegateTask task) async {
    await store.stopTask(widget.session, task.id);
    _say(task.phase == TaskPhase.canceled ? 'Stopped ${task.title}. It is Cancelled.' : 'The host did not confirm that ${task.title} stopped.');
  }

  Future<void> _stopTree(SessionStore store, DelegateTask task) async {
    final tasks = widget.session.tasks;
    final targets = [...tasks.descendantsOf(task.id), task].where((each) => !each.phase.ended && !each.native).toList();
    for (final each in targets) {
      await store.stopTask(widget.session, each.id);
    }
    final stopped = targets.where((each) => each.phase == TaskPhase.canceled).length;
    _say(
      stopped == 0
          ? 'The host did not confirm that ${task.title} stopped.'
          : 'Stopped ${task.title} and what it started: ${_plural(stopped)} Cancelled.${_left(tasks.descendantsOf(task.id))}',
    );
  }

  Future<void> _stopAll(SessionStore store) async {
    final tasks = widget.session.tasks;
    final targets = tasks.hostLive;
    await store.stopTasks(widget.session);
    final stopped = targets.where((each) => each.phase == TaskPhase.canceled).length;
    _say(stopped == 0 ? 'The host did not confirm that the subagents stopped.' : 'Stopped all: ${_plural(stopped)} Cancelled.${_left(tasks.live)}');
  }

  @override
  Widget build(BuildContext context) {
    final env = SurfaceEnvironment.of(context);
    final store = env.store, session = widget.session;
    return ListenableBuilder(
      listenable: store,
      builder: (context, _) {
        final look = context.chatLook;
        final tree = Lineage.of(session);
        final open = _open != null && tree[_open!] != null ? _open : null;
        final all = DelegationView(tree.views);
        final canStopAll = session.tasks.hostLive.isNotEmpty;
        return Material(
          color: context.look.rail,
          child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 8, 8, 4),
              child: Row(children: [
                Expanded(child: Text(all.children.isEmpty ? 'No subagents' : all.summary, key: const Key('subagents-summary'), style: const TextStyle(fontSize: 14, fontWeight: FontWeight.w600))),
                TextButton.icon(
                  key: const Key('subagents-stop-all'),
                  onPressed: canStopAll ? () => _stopAll(store) : null,
                  icon: const Icon(Icons.stop_circle_outlined, size: 18),
                  label: const Text('Stop all'),
                ),
              ]),
            ),
            if (_said != null)
              Padding(
                key: const Key('subagents-said'),
                padding: const EdgeInsets.fromLTRB(16, 0, 16, 6),
                child: Text(_said!, style: TextStyle(color: look.muted, fontSize: 13)),
              ),
            const Divider(height: 1),
            Expanded(
              child: Stack(fit: StackFit.expand, children: [
                Offstage(
                  offstage: open != null,
                  child: _List(session: session, tree: tree, now: store.clock, onOpen: _show, onStopOne: (task) => _stopOne(store, task), onStopTree: (task) => _stopTree(store, task)),
                ),
                for (final MapEntry(:key, :value) in _pages.entries)
                  if (session.tasks[key] case final task?)
                    Offstage(
                      key: ValueKey('page-$key'),
                      offstage: open != key,
                      child: _Detail(
                        store: store,
                        session: session,
                        task: task,
                        page: value,
                        tree: tree,
                        onBack: () => _show(tree[key]?.parent),
                        onExpand: () {
                          store.openChild(session, key);
                          env.onOpenChild();
                        },
                        onStopOne: () => _stopOne(store, task),
                        onStopTree: () => _stopTree(store, task),
                      ),
                    ),
              ]),
            ),
          ]),
        );
      },
    );
  }
}

class _List extends StatelessWidget {
  const _List({required this.session, required this.tree, required this.now, required this.onOpen, required this.onStopOne, required this.onStopTree});

  final ChatSession session;
  final Lineage tree;
  final DateTime Function() now;
  final void Function(String id) onOpen;
  final void Function(DelegateTask task) onStopOne, onStopTree;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    return Ticking(
      active: tree.views.any((child) => child.live),
      builder: (context) => ListView(key: const Key('subagents-list'), padding: const EdgeInsets.symmetric(vertical: 4), children: [
        for (final child in tree.views)
          InkWell(
            key: Key('subagent-${child.id}'),
            onTap: () => onOpen(child.id),
            child: Padding(
              padding: EdgeInsets.fromLTRB(16 + 18.0 * child.depth.clamp(0, 5), 6, 4, 6),
              child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                Row(children: [
                  SizedBox.square(dimension: 18, child: Center(child: SourceMark.provider(child.provider, size: 14))),
                  const SizedBox(width: 10),
                  Expanded(child: Text(child.title, maxLines: 1, overflow: TextOverflow.ellipsis, style: TextStyle(color: look.text, fontSize: 14))),
                  const SizedBox(width: 8),
                  Text(child.word, style: TextStyle(color: stateColor(look, child), fontSize: 13)),
                  if (timeWord(child, now()) case final time?)
                    Padding(padding: const EdgeInsets.only(left: 8), child: Text(time, style: TextStyle(color: look.muted, fontSize: 13))),
                  if (session.tasks[child.id] case final task?)
                    _StopMenu(task: task, view: child, tree: tree, onStopOne: () => onStopOne(task), onStopTree: () => onStopTree(task)),
                ]),
                if (child.asking)
                  Padding(
                    padding: const EdgeInsets.only(left: 28, top: 2),
                    child: Text('Asks: ${child.asks}', maxLines: 2, overflow: TextOverflow.ellipsis, style: TextStyle(color: look.warning, fontSize: 12)),
                  ),
              ]),
            ),
          ),
      ]),
    );
  }
}

/// Stop for one row: this one, or this one and everything it started. A child the conversation's own agent started cannot be
/// stopped from here, and the row says so.
class _StopMenu extends StatelessWidget {
  const _StopMenu({required this.task, required this.view, required this.tree, required this.onStopOne, required this.onStopTree});

  final DelegateTask task;
  final ChildView view;
  final Lineage tree;
  final VoidCallback onStopOne, onStopTree;

  @override
  Widget build(BuildContext context) {
    if (!view.live) return const SizedBox(width: 40);
    if (task.native) {
      return const Tooltip(
        message: "Started by the conversation's agent: only that agent can stop it",
        child: Padding(padding: EdgeInsets.symmetric(horizontal: 11), child: Icon(Icons.lock_outline, size: 18)),
      );
    }
    return PopupMenuButton<String>(
      key: Key('subagent-stop-${task.id}'),
      tooltip: 'Stop',
      icon: const Icon(Icons.stop_circle_outlined, size: 18),
      onSelected: (value) => value == 'tree' ? onStopTree() : onStopOne(),
      itemBuilder: (_) => [
        PopupMenuItem(key: Key('stop-one-${task.id}'), value: 'one', child: const Text('Stop this one')),
        PopupMenuItem(key: Key('stop-tree-${task.id}'), value: 'tree', child: const Text('Stop with descendants')),
      ],
    );
  }
}

/// One child opened: the way back, the way to the main view, Stop, and what it did.
class _Detail extends StatelessWidget {
  const _Detail({
    required this.store,
    required this.session,
    required this.task,
    required this.page,
    required this.tree,
    required this.onBack,
    required this.onExpand,
    required this.onStopOne,
    required this.onStopTree,
  });

  final SessionStore store;
  final ChatSession session;
  final DelegateTask task;
  final _Page page;
  final Lineage tree;
  final VoidCallback onBack, onExpand, onStopOne, onStopTree;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final child = tree[task.id] ?? ChildView.of(task);
    final below = session.tasks.descendantsOf(task.id).any((each) => !each.phase.ended && !each.native);
    final back = TextButton.icon(
      key: const Key('subagent-back'),
      onPressed: onBack,
      icon: const Icon(Icons.arrow_back, size: 18),
      label: const Text('Back to parent'),
    );
    final expand = TextButton.icon(
      key: const Key('subagent-expand'),
      onPressed: onExpand,
      icon: const Icon(Icons.open_in_full, size: 16),
      label: const Text('Expand to main view'),
    );
    final name = Row(children: [
      SourceMark.provider(child.provider, size: 18),
      const SizedBox(width: 8),
      Expanded(
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Text(child.title, maxLines: 1, overflow: TextOverflow.ellipsis, style: const TextStyle(fontSize: 15, fontWeight: FontWeight.w600)),
          Ticking(
            active: child.live,
            builder: (context) => Text(
              childLine(child, store.clock()),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(color: stateColor(look, child), fontSize: 12),
            ),
          ),
        ]),
      ),
    ]);
    final stops = [
      if (child.live && !task.native) TextButton(key: const Key('subagent-stop'), onPressed: onStopOne, child: const Text('Stop')),
      if (child.live && !task.native && below)
        TextButton(key: const Key('subagent-stop-tree'), onPressed: onStopTree, child: const Text('Stop with descendants')),
    ];
    return Column(children: [
      Material(
        color: context.look.bar,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
          child: LayoutBuilder(
            builder: (context, box) => box.maxWidth >= 960
                ? Row(children: [back, const SizedBox(width: 8), Expanded(child: name), ...stops, expand])
                : Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                    Wrap(alignment: WrapAlignment.spaceBetween, children: [back, ...stops, expand]),
                    name,
                  ]),
          ),
        ),
      ),
      Expanded(
        child: ChildThreadBody(
          key: ValueKey('body-${task.id}'),
          store: store,
          session: session,
          task: task,
          thread: page.thread,
          parent: child.parent == null ? null : tree[child.parent!],
          scroll: page.scroll,
          full: true,
          narrow: true,
        ),
      ),
    ]);
  }
}
