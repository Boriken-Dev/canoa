/// The views a session's agent draws and where they open: the side panel, a tab of the bottom panel, or a window of their own.
/// An agent places a view by the `target` of the `_meta.a2ui` it sends; one that names none opens in the side panel.
library;

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../../l10n/app_text.dart';
import '../../session_store.dart';
import '../../shell/layout_state.dart' show ShellLayout;
import '../../shell/surface.dart';
import '../../store/links.dart' as links;
import 'catalog.dart';
import 'limits.dart';
import 'messages.dart';
import 'model.dart';
import 'overlay_views.dart' show a2uiOverlayPrefix;
import 'view.dart';

/// Where a view is shown.
enum A2uiTarget {
  sidePanel('side_panel'),
  tab('tab'),
  window('window');

  const A2uiTarget(this.wire);

  /// What an agent writes for it.
  final String wire;

  static A2uiTarget read(String? text) {
    for (final each in values) {
      if (each.wire == text) return each;
    }
    return sidePanel;
  }
}

/// One view a session has: a surface, where it opens and what its tab is called.
final class A2uiView {
  const A2uiView(this.surfaceId, this.target, this.title);
  final String surfaceId;
  final A2uiTarget target;
  final String title;
}

/// Told when a view is made, so the app can open it where it belongs.
typedef A2uiOpened = ({A2uiView view, bool created});

/// One session's views: the processor that holds its surfaces, and what each is for.
class A2uiSessionViews extends ChangeNotifier {
  A2uiSessionViews({A2uiLimits limits = const A2uiLimits(), A2uiCatalogs? catalogs, DateTime Function()? clock})
    : processor = A2uiProcessor(limits: limits, catalogs: catalogs, clock: clock) {
    processor.onProblem = _problem;
    processor.onChange = _changed;
  }

  final A2uiProcessor processor;
  final Map<String, A2uiView> views = {};
  final _opened = StreamController<A2uiOpened>.broadcast(sync: true);
  final Map<String, int> _told = {};
  A2uiTarget? _target;
  String? _title;

  /// Sends a message to the agent: an `error` or an `action`. Null where the session has no agent to send to.
  void Function(Map<String, Object?> message)? outbound;

  Stream<A2uiOpened> get opened => _opened.stream;

  /// Takes what an update carried: its messages, and where and under what name a view they make opens.
  void carry(({List<Object?> messages, String? target, String? title}) carried) {
    _target = carried.target == null ? null : A2uiTarget.read(carried.target);
    _title = carried.title;
    try {
      processor.receive(carried.messages);
    } finally {
      _target = null;
      _title = null;
    }
  }

  void _problem(A2uiProblem problem) {
    // A view an overlay shipped is the overlay's, not the agent's: what is wrong with it is read here and not sent on.
    if (problem.surfaceId.startsWith(a2uiOverlayPrefix)) {
      notifyListeners();
      return;
    }
    final told = _told[problem.surfaceId] ?? 0;
    if (told >= processor.limits.maxErrorReports) return;
    _told[problem.surfaceId] = told + 1;
    outbound?.call(problem.toMessage());
    notifyListeners();
  }

  void _changed(String id, A2uiSurfaceChange change) {
    switch (change) {
      case A2uiSurfaceChange.created:
        final view = views[id] = A2uiView(id, _target ?? A2uiTarget.sidePanel, _title ?? id);
        _told.remove(id);
        _opened.add((view: view, created: true));
      case A2uiSurfaceChange.updated:
        break;
      case A2uiSurfaceChange.deleted:
        views.remove(id);
    }
    notifyListeners();
  }

  /// Hands an action of a view to the agent.
  void act(Map<String, Object?> action) => outbound?.call(action);

  @override
  void dispose() {
    unawaited(_opened.close());
    processor.dispose();
    super.dispose();
  }
}

/// A view as a surface of the work panel: the side panel, or a tab of the bottom panel.
class A2uiViewSurface extends WorkSurface {
  const A2uiViewSurface(this.sessionId, this.views, this.view);

  final String sessionId;
  final A2uiSessionViews views;
  final A2uiView view;

  @override
  String get id => 'a2ui:${view.surfaceId}';
  @override
  String get title => view.title;
  @override
  IconData get icon => Icons.dashboard_outlined;
  @override
  List<SurfaceSlot> get placements => view.target == A2uiTarget.tab ? const [SurfaceSlot.bottom, SurfaceSlot.right] : const [SurfaceSlot.right, SurfaceSlot.bottom];

  /// Ids repeat from session to session, so a view is the one of the session it came from.
  @override
  bool available(ChatSession session) => session.id == sessionId && views.views.containsKey(view.surfaceId);

  @override
  Widget build(BuildContext context, ChatSession session, SurfaceController controller) =>
      A2uiViewBody(key: ValueKey('a2ui view ${view.surfaceId}'), views: views, surfaceId: view.surfaceId);
}

/// What a view draws: its surface, with buttons that tell the agent.
class A2uiViewBody extends StatelessWidget {
  const A2uiViewBody({super.key, required this.views, required this.surfaceId, this.onOpenUrl});

  final A2uiSessionViews views;
  final String surfaceId;
  final Future<void> Function(Uri url)? onOpenUrl;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: views,
    builder: (context, _) {
      final surface = views.processor.surfaces[surfaceId];
      if (surface == null) return Center(child: Text(context.t('a2ui.closed'), key: const ValueKey('a2ui closed')));
      return A2uiSurfaceView(
        surface: surface,
        onAction: views.outbound == null ? null : views.act,
        onOpenUrl: onOpenUrl ?? links.openLink,
        words: A2uiWords(
          problems: context.t('a2ui.problems'),
          waiting: context.t('a2ui.waiting'),
          moreRows: (n) => context.t('a2ui.more_rows', {'n': n}),
        ),
      );
    },
  );
}

/// Opens [view] where its target says: the right panel, a tab of the bottom panel, or a window. A window is a dialog of the app
/// with the view in it, closed by its own button.
void placeA2uiView({
  required BuildContext context,
  required WorkSurfaces surfaces,
  required ShellLayout layout,
  required ChatSession session,
  required A2uiView view,
}) {
  final surface = surfaces.registry.where((each) => each.id == 'a2ui:${view.surfaceId}' && each.available(session)).firstOrNull;
  if (surface is! A2uiViewSurface) return;
  if (view.target == A2uiTarget.window) {
    unawaited(
      showDialog<void>(
        context: context,
        barrierDismissible: false,
        builder: (dialog) => Dialog(
          key: ValueKey('a2ui window ${view.surfaceId}'),
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 760, maxHeight: 560, minWidth: 320, minHeight: 240),
            child: Column(mainAxisSize: MainAxisSize.min, children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 8, 8, 0),
                child: Row(children: [
                  Expanded(child: Text(view.title, style: Theme.of(dialog).textTheme.titleMedium)),
                  IconButton(key: const ValueKey('a2ui window close'), tooltip: dialog.t('a2ui.close'), icon: const Icon(Icons.close), onPressed: () => Navigator.of(dialog).pop()),
                ]),
              ),
              Flexible(child: A2uiViewBody(views: surface.views, surfaceId: view.surfaceId)),
            ]),
          ),
        ),
      ),
    );
    return;
  }
  final slot = view.target == A2uiTarget.tab ? SurfaceSlot.bottom : SurfaceSlot.right;
  final opened = surfaces.of(session.id).openSurface(surface, slot);
  opened.slot == SurfaceSlot.bottom ? layout.showBottom() : layout.showRight();
}
