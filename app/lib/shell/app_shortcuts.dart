import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart' show LogicalKeyboardKey;

import '../chat/composer.dart' show ComposerKit;
import '../chat/palette.dart';
import '../rail/session_groups.dart';
import '../session_store.dart';
import 'app_palette.dart';
import 'layout_state.dart';
import 'surface.dart';

/// Selects the session [step] places from the chosen one, wrapping around the grouped list.
void stepAppSession(SessionStore store, ValueNotifier<SessionGrouping> grouping, int step) {
  final sessions = listOrder(store.listed, grouping.value);
  if (sessions.isEmpty) return;
  final at = sessions.indexWhere((session) => session.id == store.selectedId);
  store.select(sessions[(at + step) % sessions.length].id);
}

/// Picks the agent's first permission option matching [kind] on the selected session, if any.
void answerAppPermission(SessionStore store, String kind) {
  final session = store.selected;
  if (session == null || session.pendingPermissions.isEmpty) return;
  final options = session.pendingPermissions.first.options;
  final chosen =
      options.where((option) => option.kind == kind).firstOrNull ??
      (kind == 'reject_once' ? options.where((option) => option.rejects).firstOrNull : null);
  if (chosen != null) store.choosePermission(session, chosen);
}

/// Stops the selected session's turn if it is currently working.
void stopActiveSession(SessionStore store) {
  if (store.selected?.status == SessionStatus.working) {
    unawaited(store.cancelActiveSession().catchError((Object _) {}));
  }
}

/// Builds the window-wide keyboard shortcut table for the main shell.
Map<ShortcutActivator, VoidCallback> buildAppShortcuts(
  BuildContext context, {
  required SessionStore store,
  required ValueNotifier<SessionGrouping> grouping,
  required ShellLayout layout,
  required WorkSurfaces surfaces,
  required ConversationHandle conversation,
  required ComposerKit kit,
  required VoidCallback onNewSession,
  required VoidCallback onSettings,
  required Future<void> Function() onPalette,
}) => {
  chord(LogicalKeyboardKey.keyK): () => unawaited(onPalette()),
  chord(LogicalKeyboardKey.keyN): onNewSession,
  // The chat-only app has no panels.
  if (surfaces.registry.isNotEmpty) ...{
    chord(LogicalKeyboardKey.backquote): () {
      if (store.selected case final session?) toggleBottomPanel(surfaces: surfaces, layout: layout, session: session);
    },
    chord(LogicalKeyboardKey.keyB, alt: true): () {
      if (store.selected case final session? when surfaces.offers(session, SurfaceSlot.right)) layout.toggleRight();
    },
  },
  // The shell hears it too when focus is inside it; this is for focus above, where nothing in the shell is.
  chord(LogicalKeyboardKey.keyB): () {
    if (layout.section == ShellLayout.conversations) layout.toggleIndex(layout.section);
  },
  chord(LogicalKeyboardKey.comma): onSettings,
  for (final (index, key) in const [
    LogicalKeyboardKey.digit1,
    LogicalKeyboardKey.digit2,
    LogicalKeyboardKey.digit3,
    LogicalKeyboardKey.digit4,
    LogicalKeyboardKey.digit5,
    LogicalKeyboardKey.digit6,
    LogicalKeyboardKey.digit7,
    LogicalKeyboardKey.digit8,
    LogicalKeyboardKey.digit9,
  ].indexed)
    chord(key): () {
      final ordered = listOrder(store.listed, grouping.value);
      if (index < ordered.length) store.select(ordered[index].id);
    },
  chord(LogicalKeyboardKey.tab): () => stepAppSession(store, grouping, 1),
  chord(LogicalKeyboardKey.tab, shift: true): () => stepAppSession(store, grouping, -1),
  chord(LogicalKeyboardKey.keyL): () => conversation.focusComposer?.call(),
  chord(LogicalKeyboardKey.space, shift: true): () => unawaited(kit.dictation?.press() ?? Future.value()),
  chord(LogicalKeyboardKey.period): () => stopActiveSession(store),
  chord(LogicalKeyboardKey.keyA, shift: true): () => answerAppPermission(store, 'allow_once'),
  chord(LogicalKeyboardKey.keyR, shift: true): () => answerAppPermission(store, 'reject_once'),
  chord(LogicalKeyboardKey.keyD, shift: true): () => conversation.showChanges?.call(),
  chord(LogicalKeyboardKey.slash): () => unawaited(showShortcutHelp(context)),
};
