import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:math' show Random;
import 'dart:ui' show Size;

import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;
import 'package:flutter/widgets.dart' show WidgetBuilder;
import 'package:web/web.dart' as web;

import 'extension_windows.dart';
import 'link_stream.dart';
import 'popout_window.dart';
import 'popout_launch.dart';

/// A link over a `BroadcastChannel`, which reaches every page of the same origin that opened one of that name: the page that holds
/// the conversation and the window it opened, which are the only two that know the name.
final class BroadcastViewLink extends StringViewLink {
  BroadcastViewLink(String name) : _channel = web.BroadcastChannel(name) {
    _channel.onmessage = ((web.MessageEvent event) {
      final data = event.data;
      receive(data.isA<JSString>() ? (data as JSString).toDart : null);
    }).toJS;
  }

  final web.BroadcastChannel _channel;

  @override
  void write(String text) => _channel.postMessage(text.toJS);

  @override
  Future<void> release() async => _channel.close();
}

String _channelOf(String link) => 'canoa.popout/$link';

String _newLinkId() {
  final random = Random.secure();
  return [for (var i = 0; i < 16; i++) random.nextInt(256).toRadixString(16).padLeft(2, '0')].join();
}

final class _WebWindow implements PopoutWindow {
  _WebWindow(this.id, this.link, this.window, this.title, this.size) {
    // A page that goes away says nothing to a channel, so whether it is still there is asked.
    _poll = Timer.periodic(const Duration(milliseconds: 500), (_) {
      if (window.closed) _gone();
    });
    // The page that holds the conversation going away is not said by a channel either, so the window is told.
    web.window.addEventListener('pagehide', _leaving);
  }

  late final JSFunction _leaving = ((web.Event _) {
    link.send({'t': 'ended', 'reason': 'The page that held this conversation was closed'});
  }).toJS;

  @override
  final String id;
  @override
  final BroadcastViewLink link;
  final web.Window window;
  @override
  String title;
  @override
  Size size;
  final _closed = Completer<void>();
  late final Timer _poll;

  void _gone() {
    _poll.cancel();
    web.window.removeEventListener('pagehide', _leaving);
    if (!_closed.isCompleted) _closed.complete();
  }

  @override
  Future<void> get closed => _closed.future;

  @override
  void setTitle(String title) => this.title = title;

  // The browser sizes its own windows: this is the size the window was asked for.
  @override
  void setSize(Size size) => this.size = size;

  @override
  Future<void> focus() async => window.focus();

  @override
  Future<void> close() async {
    if (!window.closed) window.close();
    _gone();
    await link.close();
  }
}

final class _WebWindows extends WindowList {
  @override
  bool get available => web.window.location.protocol == 'http:' || web.window.location.protocol == 'https:';

  @override
  Future<PopoutWindow> open({
    required String title,
    required String look,
    required String density,
    String? session,
    Size size = popoutSize,
    WidgetBuilder? builder,
  }) async {
    final id = _newLinkId();
    final link = BroadcastViewLink(_channelOf(id));
    final here = Uri.base;
    // The same page, which starts as a pop-out when it is told so: the same origin shares what the channel reaches.
    final address = Uri(
      scheme: here.scheme,
      host: here.host,
      port: here.hasPort ? here.port : null,
      path: here.path,
      queryParameters: {'popout': id, 'title': title, 'look': look, 'density': density},
    );
    // Called inside the person's click, so that the browser lets it open.
    final window = web.window.open(address.toString(), '_blank', 'popup=yes,width=${size.width.round()},height=${size.height.round()}');
    if (window == null) {
      await link.close();
      throw StateError('The browser did not open the window: allow pop-ups for this page');
    }
    return track(_WebWindow(id, link, window, title, size));
  }
}

@JS('canoaBridge.client')
external JSString? get _extensionClient;

@JS('canoaBridge.popout.call')
external JSPromise<JSString> _popoutCall(JSString name, JSString args);

@JS('canoaBridge.popout.watch')
external void _popoutWatch(JSFunction callback);

@JS('canoaBridge')
external JSObject? get _canoaBridge;

/// The extension's pop-out bridge; null in any page the extension did not make, which has no `canoaBridge` at all.
JSObject? get _popoutBridge => _canoaBridge?.getProperty<JSObject?>('popout'.toJS);

/// The extension's worker, as the panel reaches it (clients/browser/chromium/bridge.js).
final class _WebPopoutBridge implements PopoutBridge {
  _WebPopoutBridge() {
    _popoutWatch(((JSNumber id) => _closed.add(id.toDartInt)).toJS);
  }

  final _closed = StreamController<int>.broadcast();

  @override
  String get client => _extensionClient?.toDart ?? 'panel';

  Future<String> _run(String name, List<Object?> args) async {
    try {
      return (await _popoutCall(name.toJS, jsonEncode(args).toJS).toDart).toDart;
    } on Object catch (error) {
      throw StateError('$error'.replaceFirst(RegExp(r'^.*?Error: '), ''));
    }
  }

  @override
  Future<int> open({required String session, required String link, required String title, required String look, required String density}) async =>
      (jsonDecode(await _run('open', [session, link, title, look, density])) as Map)['windowId'] as int;

  @override
  Future<void> focus(String session) => _run('focus', [session]);

  @override
  Future<void> close(String session) => _run('close', [session]);

  @override
  Stream<int> get closed => _closed.stream;
}

// In the browser extension a window is the worker's to open, at the extension's own page; elsewhere the page opens one itself.
PopoutWindows createPopoutWindows() => _popoutBridge == null
    ? _WebWindows()
    : ExtensionPopoutWindows(_WebPopoutBridge(), linkFor: (id) => BroadcastViewLink(_channelOf(id)), newLinkId: _newLinkId);

PopoutLaunch? popoutLaunch(List<String> args) {
  final query = Uri.base.queryParameters;
  final link = query['popout'];
  if (link == null || link.isEmpty) return null;
  return PopoutLaunch(
    link: link,
    session: query['session'],
    title: query['title'] ?? 'Canoa',
    look: query['look'] ?? 'canoa',
    density: query['density'] ?? 'normal',
  );
}

Future<ChatViewLink> connectPopout(PopoutLaunch launch) async => BroadcastViewLink(_channelOf(launch.link));

Future<void> preparePopoutWindow(PopoutLaunch launch) async {
  web.document.title = '${launch.title} - Canoa';
}

Future<void> closePopoutWindow() async => web.window.close();
