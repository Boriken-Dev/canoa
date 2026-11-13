import 'dart:async';
import 'dart:io' show Platform;
import 'dart:math' show Random;
import 'dart:ui' show Size;

import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;
import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/services.dart' show MissingPluginException;
import 'package:window_manager/window_manager.dart';

import 'link_stream.dart';
import 'popout_window.dart';
import 'popout_launch.dart';

/// The method name a message goes by on a window channel.
const String _method = 'message';

/// A link over two window channels, one for each end to receive on. A unidirectional channel has one handler, registered by its
/// receiver, and any window may call it (a bidirectional one pairs two windows that both register it). A channel reaches the window that registered its handler,
/// whichever engine that is.
final class WindowChannelLink extends StringViewLink {
  WindowChannelLink({required String receiveOn, required String sendOn})
    : _in = WindowMethodChannel(receiveOn, mode: ChannelMode.unidirectional),
      _out = WindowMethodChannel(sendOn, mode: ChannelMode.unidirectional) {
    ready = _in.setMethodCallHandler((call) async {
      if (call.method == _method) receive(call.arguments);
    });
  }

  final WindowMethodChannel _in, _out;

  /// Completes when this end can receive.
  late final Future<void> ready;

  // One at a time, so what is sent arrives in the order it was sent.
  Future<void> _sending = Future.value();

  @override
  void write(String text) {
    _sending = _sending.then((_) async {
      if (isClosed) return;
      try {
        await _out.invokeMethod<void>(_method, text);
      } on WindowChannelException {
        fail();
      } on MissingPluginException {
        fail();
      }
    });
  }

  @override
  Future<void> release() async {
    try {
      await _in.setMethodCallHandler(null);
    } on Object {
      return;
    }
  }
}

String _channelOf(String link, String end) => 'canoa.popout/$link/$end';

String _newLinkId() {
  final random = Random.secure();
  return [for (var i = 0; i < 16; i++) random.nextInt(256).toRadixString(16).padLeft(2, '0')].join();
}

final class _DesktopWindow implements PopoutWindow {
  _DesktopWindow(this.link, this.controller) {
    // A window that is gone is no longer in the list the plugin keeps.
    _watch = onWindowsChanged.listen((_) async {
      final all = await WindowController.getAll();
      if (!all.any((window) => window.windowId == controller.windowId)) _gone();
    });
  }

  @override
  final WindowChannelLink link;
  final WindowController controller;
  final _closed = Completer<void>();
  late final StreamSubscription<void> _watch;

  void _gone() {
    unawaited(_watch.cancel());
    if (!_closed.isCompleted) _closed.complete();
  }

  @override
  Future<void> get closed => _closed.future;

  @override
  Future<void> focus() async {
    try {
      await controller.show();
    } on Object {
      return;
    }
  }

  @override
  Future<void> close() async {
    if (!_closed.isCompleted) {
      // Not waited for: the window may be gone before it could answer.
      unawaited(controller.invokeMethod<void>('window_close').timeout(const Duration(seconds: 2)).then<void>((_) {}, onError: (Object _) {}));
    }
    _gone();
    await link.close();
  }
}

final class _DesktopWindows implements PopoutWindows {
  @override
  bool get available => Platform.isWindows || Platform.isLinux || Platform.isMacOS;

  @override
  Future<PopoutWindow> open({required String title, required String look, required String density}) async {
    final id = _newLinkId();
    final link = WindowChannelLink(receiveOn: _channelOf(id, 'owner'), sendOn: _channelOf(id, 'view'));
    await link.ready;
    final launch = PopoutLaunch(link: id, title: title, look: look, density: density);
    final controller = await WindowController.create(WindowConfiguration(arguments: launch.encode(), hiddenAtLaunch: false));
    await controller.show();
    return _DesktopWindow(link, controller);
  }
}

PopoutWindows createPopoutWindows() => _DesktopWindows();

/// The engine the plugin starts for a window runs the app's `main` with `multi_window`, the window's id and its arguments.
PopoutLaunch? popoutLaunch(List<String> args) =>
    args.length >= 3 && args.first == 'multi_window' ? PopoutLaunch.decode(args[2]) : null;

Future<ChatViewLink> connectPopout(PopoutLaunch launch) async {
  final link = WindowChannelLink(receiveOn: _channelOf(launch.link, 'view'), sendOn: _channelOf(launch.link, 'owner'));
  await link.ready;
  return link;
}

Future<void> preparePopoutWindow(PopoutLaunch launch) async {
  // The main window asks this one to close through the controller's own channel.
  try {
    final current = await WindowController.fromCurrentEngine();
    await current.setWindowMethodHandler((call) async {
      if (call.method == 'window_close') {
        // Answers first: the window this answer goes through is the one closing.
        unawaited(Future<void>(windowManager.close));
        return null;
      }
      throw MissingPluginException(call.method);
    });
    await windowManager.ensureInitialized();
    await windowManager.setTitle('${launch.title} - Canoa');
    await windowManager.setSize(const Size(560, 780));
    await windowManager.show();
    await windowManager.focus();
  } on Object {
    // A window that cannot be titled is still a window.
    return;
  }
}

// `close`, not `destroy`: the plugin's destroy posts a quit message, which ends the whole program, not this window.
Future<void> closePopoutWindow() async {
  try {
    await windowManager.close();
  } on Object {
    return;
  }
}
