import 'dart:async';

import 'package:material_ui/material_ui.dart';
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

/// Builds the window-wide keyboard shortcut bindings from [appShortcutTable], with [overrides] from `[shortcuts]` in canoa.cfg.
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
  void Function(int direction)? onZoom,
  required Future<void> Function() onPalette,
  Map<String, String> overrides = const {},
}) {
  final actions = <String, VoidCallback>{
    'palette': () => unawaited(onPalette()),
    'new_session': onNewSession,
    'close_session': () {
      if (store.selected case final session?) {
        unawaited((conversation.close ?? store.close)(session));
      }
    },
    if (surfaces.registry.isNotEmpty) ...{
      'toggle_bottom': () {
        if (store.selected case final session?) toggleBottomPanel(surfaces: surfaces, layout: layout, session: session);
      },
      'toggle_right': () {
        if (store.selected case final session? when surfaces.offers(session, SurfaceSlot.right)) layout.toggleRight();
      },
    },
    'toggle_index': () {
      if (layout.section == ShellLayout.conversations) layout.toggleIndex(layout.section);
    },
    'settings': onSettings,
    if (onZoom != null) ...{
      'zoom_in': () => onZoom(1),
      'zoom_out': () => onZoom(-1),
      'zoom_reset': () => onZoom(0),
    },
    'focus_composer': () => conversation.focusComposer?.call(),
    'dictation': () => unawaited(kit.dictation?.press() ?? Future.value()),
    'stop_turn': () => stopActiveSession(store),
    'stop_turn_esc': () => stopActiveSession(store),
    'allow_once': () => answerAppPermission(store, 'allow_once'),
    'reject_once': () => answerAppPermission(store, 'reject_once'),
    'show_changes': () => conversation.showChanges?.call(),
    'find': () => conversation.find?.call(),
    'shortcut_help': () => unawaited(showShortcutHelp(context, overrides: overrides)),
  };

  return {
    for (final spec in appShortcutTable) ?spec.activatorWith(overrides): ?actions[spec.id],
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
    // The plus key and the keypad's keys do what Ctrl+= and Ctrl+- do.
    if (onZoom != null) ...{
      chord(LogicalKeyboardKey.equal, shift: true): () => onZoom(1),
      chord(LogicalKeyboardKey.numpadAdd): () => onZoom(1),
      chord(LogicalKeyboardKey.numpadSubtract): () => onZoom(-1),
      chord(LogicalKeyboardKey.numpad0): () => onZoom(0),
    },
    chord(LogicalKeyboardKey.tab): () => stepAppSession(store, grouping, 1),
    chord(LogicalKeyboardKey.tab, shift: true): () => stepAppSession(store, grouping, -1),
  };
}
