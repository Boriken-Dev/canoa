import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' as chat;
import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart' show hostWords;
import '../l10n/app_text.dart';
import '../session_store.dart';
import '../tabs/session_tab_strip.dart';
import '../theme/look.dart';
import '../chat/reply_voice.dart' show replyRenderers;
import '../voice/reply_speaker.dart';
import 'popout_hub.dart';
import 'popout_view.dart';

/// What a tab's menu offers for [session] in the window [key]: moving it to a window of its own, to another window or back to the
/// main window, and closing it. A failure to move says why in the window's messenger.
List<TabAction> tabActions(BuildContext context, PopoutHub hub, String key, ChatSession session, {required bool closable}) {
  final text = AppText.of(context);
  final messenger = ScaffoldMessenger.maybeOf(context);
  Future<void> run(Future<void> Function() move) async {
    try {
      await move();
    } on Object catch (error) {
      messenger?.showSnackBar(SnackBar(content: Text(text('window.failed', {'why': hostWords(error)}))));
    }
  }

  Future<void> pick() async {
    final windows = hub.otherWindows(except: key);
    final chosen = await showDialog<String>(
      context: context,
      builder: (dialog) => SimpleDialog(
        title: Text(text('tab.pick_window')),
        children: [
          for (final window in windows)
            SimpleDialogOption(
              key: Key('tab-window-${window.id}'),
              onPressed: () => Navigator.of(dialog).pop(window.id),
              child: Text(window.tabs > 1 ? text('tab.window_more', {'title': window.title, 'count': '${window.tabs - 1}'}) : window.title),
            ),
        ],
      ),
    );
    if (chosen != null) await run(() => hub.moveToWindow(session, chosen));
  }

  return [
    if (hub.canMove(session)) ...[
      if (key == PopoutHub.mainKey || hub.tabsOf(key)!.length > 1)
        TabAction(const Key('tab-move-new'), text('tab.move_new'), Icons.open_in_new, () => unawaited(run(() => hub.moveToNewWindow(session)))),
      if (hub.otherWindows(except: key).isNotEmpty) TabAction(const Key('tab-move-to'), text('tab.move_to'), Icons.drive_file_move_outlined, () => unawaited(pick())),
      if (key != PopoutHub.mainKey) TabAction(const Key('tab-move-back'), text('tab.move_back'), Icons.close_fullscreen, () => unawaited(run(() => hub.moveBack(session)))),
    ],
    if (closable) TabAction(const Key('tab-close-item'), text('tab.close'), Icons.close, () => hub.closeTab(key, session)),
  ];
}

/// The strip of the window [windowKey]: its tabs, which of them is shown, and what each can do. Draws nothing for a window that
/// has no use for one.
class HubTabStrip extends StatelessWidget {
  const HubTabStrip({super.key, required this.hub, required this.windowKey, this.narrow = false});

  final PopoutHub hub;
  final String windowKey;
  final bool narrow;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: Listenable.merge([hub, hub.store]),
      builder: (context, _) {
        if (!hub.showsStrip(windowKey)) return const SizedBox.shrink();
        final sessions = hub.sessionsIn(windowKey);
        final closable = windowKey != PopoutHub.mainKey || sessions.length > 1;
        return SessionTabStrip(
          sessions: sessions,
          current: hub.currentIn(windowKey),
          narrow: narrow,
          onSelect: (session) => hub.selectTab(windowKey, session),
          onReorder: (from, to) => hub.reorder(windowKey, from, to),
          actions: (session) => tabActions(context, hub, windowKey, session, closable: closable),
          onClose: closable ? (session) => hub.closeTab(windowKey, session) : null,
        );
      },
    );
  }
}

/// A window of its own holding sessions as tabs: a strip, and under it the conversation of the tab shown. Every tab keeps its
/// conversation alive, so a message half written stays where it is while another tab is shown.
class TabbedWindow extends StatelessWidget {
  const TabbedWindow({super.key, required this.hub, required this.windowId});

  final PopoutHub hub;
  final String windowId;

  @override
  Widget build(BuildContext context) {
    final body = _Body(hub: hub, windowId: windowId);
    final frame = hub.frame;
    if (frame != null) return Builder(builder: (context) => frame(context, body));
    final look = LookId.named(hub.look) ?? LookId.canoa;
    final density = chat.ChatDensity.named(hub.density) ?? chat.ChatDensity.normal;
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: themeFor(look, Brightness.light, density: density),
      darkTheme: themeFor(look, Brightness.dark, density: density),
      home: body,
    );
  }
}

class _Body extends StatelessWidget {
  const _Body({required this.hub, required this.windowId});

  final PopoutHub hub;
  final String windowId;

  @override
  Widget build(BuildContext context) {
    final narrow = MediaQuery.sizeOf(context).width < 600;
    return Scaffold(
      body: Column(children: [
        HubTabStrip(hub: hub, windowKey: windowId, narrow: narrow),
        Expanded(
          child: ListenableBuilder(
            listenable: Listenable.merge([hub, hub.store]),
            builder: (context, _) {
              final current = hub.currentIn(windowId);
              return Stack(fit: StackFit.expand, children: [
                for (final session in hub.sessionsIn(windowId))
                  if (hub.conversationIn(windowId, session.id) case final conversation?)
                    Offstage(
                      key: ValueKey('tab-body-${session.id}'),
                      offstage: session.id != current,
                      child: TickerMode(
                        enabled: session.id == current,
                        child: PopoutChat(
                          conversation: conversation,
                          onDock: () => unawaited(hub.moveBack(session)),
                          renderers: switch (hub.speakerIn(windowId, session.id)) {
                            final ReplySpeaker speaker => replyRenderers(speaker),
                            null => chat.ChatRenderers.none,
                          },
                          speaker: hub.speakerIn(windowId, session.id),
                        ),
                      ),
                    ),
              ]);
            },
          ),
        ),
      ]),
    );
  }
}
