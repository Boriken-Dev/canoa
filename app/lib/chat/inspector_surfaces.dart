import 'package:flutter/material.dart';

import '../session_store.dart';
import '../shell/surface.dart';
import 'inspector.dart';

/// The session's facts in the work panel: where it runs, its workspace and mode, its context, its lineage and running commands, and
/// a summary of what it changed.
class DetailsSurface extends WorkSurface {
  const DetailsSurface();

  static const surfaceId = 'details';

  @override
  String get id => surfaceId;
  @override
  String get title => 'Details';
  @override
  IconData get icon => Icons.info_outline;
  @override
  List<SurfaceSlot> get placements => const [SurfaceSlot.right];

  /// A session that has not started has no environment to report yet.
  @override
  bool available(ChatSession session) => !session.isDraft;

  @override
  Widget build(BuildContext context, ChatSession session, SurfaceController controller) {
    final env = SurfaceEnvironment.of(context);
    return SessionInspectorPanel(
      store: env.store,
      session: session,
      embedded: true,
      onComment: env.onComment,
      onClose: () {},
      onOpenChild: env.onOpenChild,
    );
  }
}

/// What the session changed, by the agent's report or by git, as a diff.
class ChangesSurface extends WorkSurface {
  const ChangesSurface();

  static const surfaceId = 'changes';

  @override
  String get id => surfaceId;
  @override
  String get title => 'Changes';
  @override
  IconData get icon => Icons.difference_outlined;
  @override
  List<SurfaceSlot> get placements => const [SurfaceSlot.right];

  @override
  bool available(ChatSession session) => !session.isDraft;

  @override
  Widget build(BuildContext context, ChatSession session, SurfaceController controller) {
    final env = SurfaceEnvironment.of(context);
    return SessionInspectorPanel(
      store: env.store,
      session: session,
      embedded: true,
      diffOnly: true,
      onComment: env.onComment,
      onClose: () {},
    );
  }
}

/// The surfaces every window has.
List<WorkSurface> builtInSurfaces() => const [DetailsSurface(), ChangesSurface()];
