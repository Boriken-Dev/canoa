// Windows for a conversation that are only a link: what a pop-out needs of the platform, and a way to put the window's own side on
// screen in the same test.
import 'dart:async';

import 'package:canoa/popout/popout_app.dart';
import 'package:canoa/popout/popout_hub.dart';
import 'package:canoa/popout/popout_launch.dart';
import 'package:coqui_acp_chat/experimental.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

/// A window opened in a test: the main window's end of its link, and the end the window's own app attaches by.
final class FakeWindow implements PopoutWindow {
  FakeWindow(this.link, this.far, this.title);

  @override
  final ChatViewLink link;

  /// The window's end.
  final ChatViewLink far;
  final String title;
  final _closed = Completer<void>();
  int focused = 0;

  @override
  Future<void> get closed => _closed.future;

  @override
  Future<void> focus() async => focused++;

  @override
  Future<void> close() async {
    if (!_closed.isCompleted) _closed.complete();
    await link.close();
  }

  bool get isClosed => _closed.isCompleted;

  /// The user closing the window with its own button, which the main window learns only afterwards.
  void closeByUser() {
    if (!_closed.isCompleted) _closed.complete();
  }
}

final class FakeWindows implements PopoutWindows {
  FakeWindows({this.available = true});

  @override
  final bool available;
  final opened = <FakeWindow>[];

  @override
  Future<PopoutWindow> open({required String title, required String look, required String density}) async {
    final (owner, far) = MemoryViewLink.pair();
    final window = FakeWindow(owner, far, title);
    opened.add(window);
    return window;
  }
}

/// The app of [window] on screen as its own program would show it: attached to the conversation by [clientId].
Future<ChatSessionView> showWindow(WidgetTester tester, FakeWindow window, {String clientId = 'window-1', Widget Function(Widget app)? around}) async {
  final view = await tester.runAsync(() => ChatSessionView.attach(window.far, clientId: clientId, name: 'the pop-out window')) as ChatSessionView;
  final app = PopoutApp(launch: PopoutLaunch(link: 'test', title: window.title, density: 'verbose'), view: view, onClose: window.closeByUser);
  await tester.pumpWidget(around == null ? app : around(app));
  await tester.pump();
  return view;
}
