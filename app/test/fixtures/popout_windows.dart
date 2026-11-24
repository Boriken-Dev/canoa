// Windows for a conversation that are only a list: what a pop-out needs of the platform, with a window that is another page (a link
// whose far end the test puts on screen) or a window of the engine itself (a builder the test puts on screen), as the platform
// would have made it.
import 'dart:async';

import 'package:canoa/popout/popout_app.dart';
import 'package:canoa/popout/popout_conversation.dart';
import 'package:canoa/popout/popout_hub.dart';
import 'package:canoa/popout/popout_launch.dart';
import 'package:coqui_acp_chat/experimental.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

/// A window opened in a test: the main window's end of its link when it is another page, and the end the window's own app attaches
/// by; or, for a window of the engine, what it draws.
final class FakeWindow implements PopoutWindow {
  FakeWindow(this.id, this.link, this.far, this.title, this.size, this.builder);

  @override
  final String id;

  @override
  final ChatViewLink? link;

  /// The window's end of the link; null for a window of the engine.
  final ChatViewLink? far;

  /// What a window of the engine draws; null for a window that is another page.
  final WidgetBuilder? builder;

  @override
  String title;

  @override
  Size size;
  final _closed = Completer<void>();
  int focused = 0;

  @override
  Future<void> get closed => _closed.future;

  @override
  void setTitle(String title) => this.title = title;

  @override
  void setSize(Size size) => this.size = size;

  @override
  Future<void> focus() async => focused++;

  @override
  Future<void> close() async {
    if (!_closed.isCompleted) _closed.complete();
    await link?.close();
  }

  bool get isClosed => _closed.isCompleted;

  /// The user closing the window with its own button, which the main window learns only afterwards.
  void closeByUser() {
    if (!_closed.isCompleted) _closed.complete();
  }
}

/// The platform's windows as a test has them. With [sameEngine] a window draws what the app gives it, as the platform's own windows
/// do; without, it is another page, linked in memory.
final class FakeWindows extends WindowList {
  FakeWindows({this.available = true, this.sameEngine = false});

  @override
  final bool available;
  final bool sameEngine;
  final opened = <FakeWindow>[];

  /// What [host] was asked to draw beside.
  int hosted = 0;

  @override
  Widget host(Widget app) {
    hosted++;
    return app;
  }

  @override
  Future<PopoutWindow> open({
    required String title,
    required String look,
    required String density,
    String? session,
    Size size = popoutSize,
    WidgetBuilder? builder,
  }) async {
    final id = 'window-${opened.length}';
    final FakeWindow window;
    if (sameEngine) {
      window = FakeWindow(id, null, null, title, size, builder);
    } else {
      final (owner, far) = MemoryViewLink.pair();
      window = FakeWindow(id, owner, far, title, size, null);
    }
    opened.add(window);
    return track(window);
  }
}

/// The app of [window] on screen as its own program would show it: attached to the conversation by [clientId].
Future<ChatSessionView> showWindow(WidgetTester tester, FakeWindow window, {String clientId = 'window-1', Widget Function(Widget app)? around}) async {
  final view = await tester.runAsync(() => ChatSessionView.attach(window.far!, clientId: clientId, name: 'the pop-out window')) as ChatSessionView;
  final app = PopoutApp(launch: PopoutLaunch(link: 'test', title: window.title, density: 'verbose'), conversation: LinkedConversation(view), onClose: window.closeByUser);
  await tester.pumpWidget(around == null ? app : around(app));
  await tester.pump();
  return view;
}

/// What a window of the engine draws, on screen.
Future<void> showLiveWindow(WidgetTester tester, FakeWindow window) async {
  await tester.pumpWidget(Builder(builder: window.builder!));
  await tester.pump();
}
