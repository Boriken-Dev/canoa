import 'package:flutter/material.dart';

import '../session_store.dart';
import '../usage/spend_model.dart' show SpendRow;

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
  OpenSurface openSurface(WorkSurface surface, [SurfaceSlot? slot]) {
    final existing = find(surface.id);
    if (existing != null) {
      _active[existing.slot] = surface.id;
      notifyListeners();
      return existing;
    }
    final placed = slot != null && surface.placements.contains(slot) ? slot : surface.placements.first;
    final opened = OpenSurface(surface, placed);
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
  WorkSurfaces(this.registry);

  /// Every surface the app has, in the order the launcher lists them.
  final List<WorkSurface> registry;

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
    for (final each in _bySession.values) {
      each.dispose();
    }
    super.dispose();
  }
}

/// What the app gives every surface: the store, and the ways to reach the conversation beside it.
class SurfaceEnvironment extends InheritedWidget {
  const SurfaceEnvironment({super.key, required this.store, required this.onComment, required this.onOpenChild, this.sessionSpend, required super.child});

  final SessionStore store;

  /// What the host recorded a session spending, by the session's id; null where the host keeps no usage for this person.
  final Future<SpendRow?> Function(String sessionId)? sessionSpend;

  /// Adds text to the message being written.
  final ValueChanged<String> onComment;

  /// Called after a child's thread is opened from a surface.
  final VoidCallback onOpenChild;

  static SurfaceEnvironment? maybeOf(BuildContext context) => context.dependOnInheritedWidgetOfExactType<SurfaceEnvironment>();

  static SurfaceEnvironment of(BuildContext context) => context.dependOnInheritedWidgetOfExactType<SurfaceEnvironment>()!;

  @override
  bool updateShouldNotify(SurfaceEnvironment oldWidget) => store != oldWidget.store;
}
