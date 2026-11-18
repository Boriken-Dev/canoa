import 'dart:async';

import 'package:coqui_acp_chat/experimental.dart' show ChatSessionView, ChatViewLink;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' as chat;
import 'package:flutter/material.dart';

import '../extension/tab_bridge.dart';
import '../extension/tab_owner_banner.dart';
import '../extension/tab_shares.dart';
import '../theme/look.dart';
import '../chat/reply_voice.dart';
import '../voice/reply_speaker.dart';
import 'platform.dart' as platform;
import 'popout_launch.dart';
import 'popout_view.dart';
import 'read_aloud.dart';

/// Attaches this process to the conversation the window that opened it holds. Throws [StateError] when it cannot (the main window
/// is gone, or does not hold the conversation any more).
Future<ChatSessionView> attachPopout(PopoutLaunch launch, {ChatViewLink? link}) async => ChatSessionView.attach(
  link ?? await platform.connectPopout(launch),
  clientId: 'window-${launch.link.substring(0, launch.link.length < 8 ? launch.link.length : 8)}',
  name: 'the pop-out window',
);

/// Runs this process as a pop-out window: attaches to the conversation and draws it. When it cannot attach it says so and offers to
/// close.
Future<void> runPopout(PopoutLaunch launch, {ChatViewLink? link}) async {
  WidgetsFlutterBinding.ensureInitialized();
  await platform.preparePopoutWindow(launch);
  try {
    final bridge = launch.session == null ? null : platformTabBridge();
    final banner = bridge == null ? null : TabOwnerBanner(tabs: TabShares(bridge), sessionKey: launch.session!);
    // Reading aloud is asked of the main window over the same link, since that is where the voices and the player are.
    final reading = ReadAloudLink(link ?? await platform.connectPopout(launch));
    runApp(PopoutApp(launch: launch, view: await attachPopout(launch, link: reading), banner: banner, speaker: RemoteReplySpeaker(reading)));
  } on Object catch (error) {
    runApp(PopoutApp(launch: launch, problem: error is StateError ? error.message : '$error'));
  }
}

/// The app of a pop-out window: Canoa's look around one conversation.
class PopoutApp extends StatelessWidget {
  const PopoutApp({super.key, required this.launch, this.view, this.problem, this.onClose, this.banner, this.renderers = chat.ChatRenderers.none, this.speaker});

  final PopoutLaunch launch;

  /// The conversation, once attached; null with a [problem] when it could not be.
  final ChatSessionView? view;
  final String? problem;

  /// Closes the window; by default through the platform.
  final VoidCallback? onClose;
  final chat.ChatRenderers renderers;

  /// Reads replies aloud by asking the main window to; null where it cannot, which leaves the play buttons and the switch out.
  final ReplySpeaker? speaker;

  /// A line under the conversation's header; the extension's window says who holds the tab there.
  final Widget? banner;

  @override
  Widget build(BuildContext context) {
    final look = LookId.named(launch.look) ?? LookId.canoa;
    final density = chat.ChatDensity.named(launch.density) ?? chat.ChatDensity.normal;
    void close() => onClose != null ? onClose!() : unawaited(platform.closePopoutWindow());
    return MaterialApp(
      title: '${launch.title} - Canoa',
      debugShowCheckedModeBanner: false,
      theme: themeFor(look, Brightness.light, density: density),
      darkTheme: themeFor(look, Brightness.dark, density: density),
      home: view == null
          ? Scaffold(
              body: Center(
                child: Column(mainAxisSize: MainAxisSize.min, children: [
                  Text(problem ?? 'Could not attach to the conversation.', key: const Key('popout-problem')),
                  const SizedBox(height: 12),
                  FilledButton(onPressed: close, child: const Text('Close')),
                ]),
              ),
            )
          : PopoutChat(
              view: view!,
              onDock: close,
              banner: banner,
              renderers: speaker == null ? renderers : replyRenderers(speaker!).merge(renderers),
              speaker: speaker,
            ),
    );
  }
}
