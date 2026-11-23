import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../popout/item_windows.dart' show ItemWindows;
import '../chat/session_watch.dart' show SessionWatches;
import '../session_store.dart';
import '../usage/spend_model.dart' show SpendRow;
import 'layout_state.dart' show ShellLayout;

/// Where a surface may be shown: in the right panel beside the conversation, or in the bottom panel under it.
enum SurfaceSlot { right, bottom }

/// What a surface keeps outside the widget tree, so moving it between panels or hiding a panel never starts it over. The same
/// object is handed to the surface each time it is built.
class SurfaceController extends ChangeNotifier {
  SurfaceController(this.id);

  final String id;

  /// Whatever the surface wants to remember between builds.
  final Map<String, Object?> data = {};

  /// A title that replaces the surface's own, for a tab that names its page or its host.
  String? title;

  void update(VoidCallback change) {
    change();
    notifyListeners();
  }

  final List<Future<void> Function()> _releasers = [];

  /// Registers what must end when the surface is closed (a shell, a connection); moving or hiding it never runs these.
  void onRelease(Future<void> Function() release) => _releasers.add(release);

  /// Runs what [onRelease] registered, once.
  Future<void> release() async {
    final all = [..._releasers];
    _releasers.clear();
    await Future.wait([for (final each in all) each()]);
  }
}

/// A tab a surface puts in the panel's one tab bar, in place of its own single tab: a terminal's shells, each its own tab.
class SurfaceTab {
  const SurfaceTab({required this.id, required this.title, required this.selected, required this.select, this.close, this.badge});

  /// Unique among the surface's tabs; the tab's key is `tab <surface id>:<id>`.
  final String id;
  final String title;

  /// Whether it is the surface's chosen tab; it shows as chosen only while the surface is the panel's.
  final bool selected;
  final VoidCallback select;

  /// Closes just this tab; null for one that closes with its surface.
  final VoidCallback? close;

  /// A short count shown beside the title, such as how many of the tab's shells are running; null for none.
  final String? badge;
}

/// Something a surface does for its chosen tab, listed in the panel's actions menu.
class SurfaceAction {
  const SurfaceAction({required this.id, required this.label, required this.run, this.enabled = true});

  /// The menu entry's key is `action <surface id> <id>`.
  final String id;
  final String label;
  final bool enabled;
  final void Function(BuildContext context) run;
}

/// One thing the panel's + menu can open: a kind of tab, made new each time it is chosen.
class NewTab {
  const NewTab({required this.key, required this.label, required this.icon, required this.open, this.enabled = true, this.openWindow});

  /// The menu entry's key.
  final String key;
  final String label;
  final IconData icon;
  final bool enabled;

  /// Opens it in [slot] and chooses it.
  final void Function(SurfaceSlot slot) open;

  /// Shows what [open] opened in a window of its own; null for a tab that has none. It runs after [open] where a plain click on it is
  /// to open a window (Settings, Appearance, Open in).
  final Future<void> Function(BuildContext context)? openWindow;
}

/// A view that can sit in the work panel: details, changes, and what later plans add (files, a browser, a terminal).
abstract class WorkSurface {
  const WorkSurface();

  String get id;
  String get title;
  IconData get icon;

  /// The panels it may be shown in; the first is where it opens.
  List<SurfaceSlot> get placements;

  /// Whether it has something to show for [session]. It never needs an agent to have started, unless it says so here.
  bool available(ChatSession session);

  /// The tabs this surface shows in the panel's one tab bar instead of a single tab of its own; none leaves the single tab. A surface
  /// whose last tab closes shows its single tab again.
  List<SurfaceTab> tabs(ChatSession session, SurfaceController controller) => const [];

  /// What changes [tabs], listened to by the panel.
  Listenable? tabsChanged(ChatSession session, SurfaceController controller) => null;

  /// A short count shown beside this surface's single tab, such as how many things in it are running; null for none.
  String? badge(ChatSession session, SurfaceController controller) => null;

  /// What changes [badge], listened to by the panel.
  Listenable? badgeChanged(ChatSession session, SurfaceController controller) => null;

  /// What the surface does for its chosen tab, listed in the panel's actions menu.
  List<SurfaceAction> actions(BuildContext context, ChatSession session, SurfaceController controller) => const [];

  /// What the panel's + menu offers of this surface for [session]: by default the surface itself, once, until it is open. A surface
  /// whose tabs are made new each time (a shell) offers its kinds of tab whether it is open or not.
  Future<List<NewTab>> newTabs(ChatSession session, SessionSurfaces surfaces) async => surfaces.find(id) != null
      ? const []
      : [NewTab(key: 'open $id', label: title, icon: icon, open: (slot) => surfaces.openSurface(this, slot))];

  /// The surface's view. It is built under a [SurfaceEnvironment], and keeps its state in [controller] or in its own State, which
  /// moving it between panels preserves.
  Widget build(BuildContext context, ChatSession session, SurfaceController controller);
}

/// A surface that is open for a session: where it is, and the things that outlive its widgets.
class OpenSurface {
  OpenSurface(this.surface, this.slot) : controller = SurfaceController(surface.id), key = GlobalKey(debugLabel: surface.id);

  final WorkSurface surface;
  SurfaceSlot slot;
  final SurfaceController controller;

  /// Held by the surface's widget wherever it is drawn, which is what keeps its State when it moves.
  final GlobalKey key;

  String get title => controller.title ?? surface.title;
}

/// The surfaces open for one session, in the order they were opened, and the one chosen in each panel.
class SessionSurfaces extends ChangeNotifier {
  final List<OpenSurface> open = [];
  final Map<SurfaceSlot, String?> _active = {};

  List<OpenSurface> inSlot(SurfaceSlot slot) => [for (final each in open) if (each.slot == slot) each];

  OpenSurface? find(String id) => open.where((each) => each.surface.id == id).firstOrNull;

  /// The surface chosen in [slot]: the one marked, else the first there.
  OpenSurface? activeIn(SurfaceSlot slot) {
    final here = inSlot(slot);
    return here.where((each) => each.surface.id == _active[slot]).firstOrNull ?? here.firstOrNull;
  }

  /// Opens [surface] in [slot], or chooses it where it already is.
  OpenSurface openSurface(WorkSurface surface, [SurfaceSlot? slot, void Function(SurfaceController controller)? prepare]) {
    final existing = find(surface.id);
    if (existing != null) {
      prepare?.call(existing.controller);
      _active[existing.slot] = surface.id;
      notifyListeners();
      return existing;
    }
    final placed = slot != null && surface.placements.contains(slot) ? slot : surface.placements.first;
    final opened = OpenSurface(surface, placed);
    prepare?.call(opened.controller);
    open.add(opened);
    _active[placed] = surface.id;
    notifyListeners();
    return opened;
  }

  void activate(String id) {
    final found = find(id);
    if (found == null) return;
    _active[found.slot] = id;
    notifyListeners();
  }

  /// Closes a surface, which ends what it kept.
  void close(String id) {
    final found = find(id);
    if (found == null) return;
    open.remove(found);
    unawaited(found.controller.release());
    if (_active[found.slot] == id) _active[found.slot] = null;
    notifyListeners();
  }

  /// Moves a surface to [slot] when it may be shown there; it keeps its controller and its widgets' State.
  bool move(String id, SurfaceSlot slot) {
    final found = find(id);
    if (found == null || found.slot == slot || !found.surface.placements.contains(slot)) return false;
    final from = found.slot;
    found.slot = slot;
    if (_active[from] == id) _active[from] = null;
    _active[slot] = id;
    notifyListeners();
    return true;
  }
}

/// The surfaces a window offers and, for each session, the ones open. Kept outside the widget tree.
class WorkSurfaces extends ChangeNotifier {
  WorkSurfaces(this._base, {this.added, this.addedChanged}) {
    addedChanged?.addListener(notifyListeners);
  }

  final List<WorkSurface> _base;

  /// Surfaces made from what the host lists (the tabs its providers add), asked for each time the registry is read.
  final List<WorkSurface> Function()? added;

  /// What changes [added].
  final Listenable? addedChanged;

  /// Every surface the app has, in the order the launcher lists them: its own, then those the host's providers add.
  List<WorkSurface> get registry => [..._base, ...?added?.call()];

  final Map<String, SessionSurfaces> _bySession = {};

  SessionSurfaces of(String sessionId) => _bySession.putIfAbsent(sessionId, SessionSurfaces.new);

  /// The surfaces [slot] may show for [session].
  List<WorkSurface> availableFor(ChatSession session, SurfaceSlot slot) => [
    for (final surface in registry)
      if (surface.placements.contains(slot) && surface.available(session)) surface,
  ];

  /// Whether the panel [slot] has anything it could show for [session], open or offered.
  bool offers(ChatSession session, SurfaceSlot slot) => availableFor(session, slot).isNotEmpty || of(session.id).inSlot(slot).isNotEmpty;

  @override
  void dispose() {
    addedChanged?.removeListener(notifyListeners);
    for (final each in _bySession.values) {
      for (final open in each.open) {
        unawaited(open.controller.release());
      }
      each.dispose();
    }
    super.dispose();
  }
}

/// What the app gives every surface: the store, and the ways to reach the conversation beside it.
class SurfaceEnvironment extends InheritedWidget {
  const SurfaceEnvironment({
    super.key,
    required this.store,
    required this.onComment,
    required this.onOpenChild,
    this.sessionSpend,
    this.sessionWatches,
    this.onAttachText,
    this.onOpenInFiles,
    this.onOpenSurface,
    this.windows,
    this.openShells = 0,
    this.runningShells = 0,
    required super.child,
  });

  /// Attaches [text] to the message being written as a text file called [name]; null where the conversation takes no attachments.
  final void Function(String name, String text)? onAttachText;

  /// Shows a path in the Files surface; null where that surface has nothing to show for the session.
  final ValueChanged<String>? onOpenInFiles;

  /// Opens another surface by id (`terminal`, `subagents`, `details`); null where there is no panel to open it in.
  final ValueChanged<String>? onOpenSurface;

  /// Where a surface opens one of its things in a window of its own; null where there are no windows to draw in.
  final ItemWindows? windows;

  /// How many interactive shells are open for the session, and how many of them are running.
  final int openShells, runningShells;

  final SessionStore store;

  /// What the host recorded a session spending, by the session's id; null where the host keeps no usage for this person.
  final Future<SpendRow?> Function(String sessionId)? sessionSpend;

  /// The watches of the host's shared sessions; null where this key may not arm one.
  final SessionWatches? sessionWatches;

  /// Adds text to the message being written.
  final ValueChanged<String> onComment;

  /// Called after a child's thread is opened from a surface.
  final VoidCallback onOpenChild;

  static SurfaceEnvironment? maybeOf(BuildContext context) => context.dependOnInheritedWidgetOfExactType<SurfaceEnvironment>();

  static SurfaceEnvironment of(BuildContext context) => context.dependOnInheritedWidgetOfExactType<SurfaceEnvironment>()!;

  @override
  bool updateShouldNotify(SurfaceEnvironment oldWidget) =>
      store != oldWidget.store || openShells != oldWidget.openShells || runningShells != oldWidget.runningShells;
}

/// What the bottom-panel key does for [session]: with the panel hidden it shows it, opening the Terminal when the host offers one
/// and none is open yet, where it was left otherwise; with it showing it hides it. Does nothing where no surface offers the panel.
void toggleBottomPanel({required WorkSurfaces surfaces, required ShellLayout layout, required ChatSession session}) {
  if (!surfaces.offers(session, SurfaceSlot.bottom)) return;
  final terminal = surfaces.registry.where((each) => each.id == 'terminal' && each.available(session)).firstOrNull;
  if (terminal == null || layout.bottomOpen) {
    layout.toggleBottom();
    return;
  }
  final opened = surfaces.of(session.id).openSurface(terminal, SurfaceSlot.bottom);
  opened.slot == SurfaceSlot.bottom ? layout.showBottom() : layout.showRight();
}
