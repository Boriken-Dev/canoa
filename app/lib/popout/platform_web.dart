import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:math' show Random;

import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;
import 'package:flutter/widgets.dart';
import 'package:web/web.dart' as web;

import '../brand.dart';
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

  // The window is of this page's origin, so its document can be reached; a window that has not loaded yet sets its own on start.
  @override
  void setTitle(String title) {
    this.title = title;
    try {
      window.document.title = '$title - Canoa';
    } on Object {
      // A window that went, or that no longer is of this origin, keeps the title it has.
    }
  }

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
  bool get linksItems => available;

  @override
  Future<PopoutWindow> open({
    required String title,
    required String look,
    required String density,
    String? session,
    Size size = popoutSize,
    WidgetBuilder? builder,
    PopoutItem? item,
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
      queryParameters: {
        'popout': id,
        'title': title,
        'look': look,
        'density': density,
        if (item != null) 'item': item.kind,
        if (item != null && item.args.isNotEmpty) 'args': jsonEncode(item.args),
      },
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

// In the browser extension a window is the worker's to open, at the extension's own page; elsewhere the page opens one itself: as
// another view of this app when it started in multi-view mode, else as a second copy of the app over a link.
PopoutWindows createPopoutWindows() {
  if (_popoutBridge != null) {
    return ExtensionPopoutWindows(_WebPopoutBridge(), linkFor: (id) => BroadcastViewLink(_channelOf(id)), newLinkId: _newLinkId);
  }
  return _multiView ? _ViewWindows() : _WebWindows();
}

/// Runs [app] in this page: in multi-view mode (`flutter_bootstrap.js` started the engine so and kept its handle) the page's own view
/// is added by the bootstrap and drawn here, and a window is another view; otherwise the engine's one view.
void startApp(PopoutWindows windows, Widget app) {
  if (!_multiView) return runApp(windows.host(app));
  runWidget(_MainView(child: windows.host(app)));
}

PopoutLaunch? popoutLaunch(List<String> args) {
  final query = Uri.base.queryParameters;
  final link = query['popout'];
  if (link == null || link.isEmpty) return null;
  final item = query['item'];
  Object? args;
  try {
    args = query['args'] == null ? null : jsonDecode(query['args']!);
  } on FormatException {
    args = null;
  }
  return PopoutLaunch(
    link: link,
    item: item == null || item.isEmpty ? null : PopoutItem(item, PopoutLaunch.itemArgs(args)),
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

// ---- Windows that are views of this app -------------------------------------------------------------------------------------------
//
// The page's engine runs in multi-view mode (flutter_bootstrap.js). A window is a `window.open` popup with a host element in its
// document and a view added to it, drawn from the same widget tree and the same objects as the main view, as a desktop's windows
// are. The engine reads the opener's `window` and `document` for several things a view in another window needs from its own; the
// `_Shims` below make those answer for the window that has the focus. Each names the engine file it covers (public, at
// https://github.com/flutter/flutter/tree/main/engine/src/flutter/lib/web_ui/lib/src/engine/).

/// Whether the bootstrap started the engine in multi-view mode, with no view of its own.
bool get _multiView {
  final flag = globalContext['canoaMultiView'];
  return flag.isA<JSBoolean>() && (flag as JSBoolean).toDart;
}

/// The engine's handle (`FlutterApp`: `addView`, `removeView`), which the bootstrap keeps once the app has started.
JSObject get _app => globalContext['canoaFlutterApp'] as JSObject;

/// The id of the view the bootstrap added for the page itself.
int? get _mainViewId {
  final id = globalContext['canoaMainView'];
  return id.isA<JSNumber>() ? (id as JSNumber).toDartInt : null;
}

/// The page's own view, as [startApp] draws it: nothing until the engine has the view the bootstrap adds once it is running.
final class _MainView extends StatefulWidget {
  const _MainView({required this.child});

  final Widget child;

  @override
  State<_MainView> createState() => _MainViewState();
}

final class _MainViewState extends State<_MainView> with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeMetrics() => setState(() {});

  @override
  Widget build(BuildContext context) {
    final dispatcher = WidgetsBinding.instance.platformDispatcher;
    final id = _mainViewId;
    final view = id == null ? dispatcher.views.firstOrNull : dispatcher.view(id: id);
    return view == null ? const ViewCollection(views: []) : View(view: view, child: widget.child);
  }
}

JSObject get _object => globalContext['Object'] as JSObject;

JSObject _prototype(String name) => (globalContext[name] as JSObject)['prototype'] as JSObject;

void _define(JSObject target, JSAny key, {JSAny? value, JSFunction? get}) {
  final descriptor = JSObject()..['configurable'] = true.toJS;
  if (get != null) descriptor['get'] = get;
  if (value != null) descriptor['value'] = value;
  _object.callMethod('defineProperty'.toJS, target, key, descriptor);
}

/// What the engine takes from the opener's window and document, answered for the window of the app that has the focus.
final class _Shims {
  _Shims() {
    _widenTypes();
    _followFocus();
    _followVisibility();
    _paintFromPopups();
  }

  final _popups = <web.Window>[];
  web.Document? _focused;

  Iterable<web.Window> get _live => _popups.where((popup) => !popup.closed);

  /// Starts answering for [popup], whose document the engine has not seen yet.
  void attach(web.Window popup) {
    _popups.add(popup);
    _focus(popup.document);
    // Hardware keys: engine/keyboard_binding.dart listens on the opener's window, and a key pressed in the popup is never sent
    // there. The popup's key becomes the opener's, with what the engine reads from it, and what the framework handled comes back.
    for (final type in ['keydown', 'keyup']) {
      popup.addEventListener(
        type,
        ((web.KeyboardEvent event) {
          final copy = web.KeyboardEvent(
            type,
            web.KeyboardEventInit(
              key: event.key,
              code: event.code,
              location: event.location,
              repeat: event.repeat,
              isComposing: event.isComposing,
              ctrlKey: event.ctrlKey,
              shiftKey: event.shiftKey,
              altKey: event.altKey,
              metaKey: event.metaKey,
              keyCode: event.keyCode,
              charCode: event.charCode,
              bubbles: true,
              cancelable: true,
              composed: true,
            ),
          );
          web.window.dispatchEvent(copy);
          // The framework moves the focus on Tab itself, and the engine does not report that key handled: the browser's own
          // Tab, which would take the focus out of the view, is always refused here.
          if (copy.defaultPrevented || event.key == 'Tab') event.preventDefault();
        }).toJS,
        true.toJS,
      );
    }
    // The caret and selection: engine/text_editing/text_editing.dart hears `selectionchange` on the opener's document only.
    popup.document.addEventListener(
      'selectionchange',
      ((web.Event _) {
        web.document.dispatchEvent(web.Event('selectionchange'));
      }).toJS,
    );
  }

  void _focus(web.Document document) {
    document.addEventListener(
      'focusin',
      ((web.Event _) {
        _focused = document;
      }).toJS,
      true.toJS,
    );
  }

  // The engine's `isA<DomKeyboardEvent>` and the like (text_editing/text_editing.dart `Enter`, composition_aware_mixin.dart,
  // `EditingState.fromDomElement`) are `instanceof` checks against the opener's constructors, which a popup's events and inputs are
  // not instances of: the constructors accept the popups' too.
  void _widenTypes() {
    final symbol = (globalContext['Symbol'] as JSObject)['hasInstance'] as JSAny;
    final ordinary = _prototype('Function').getProperty<JSFunction>(symbol);
    for (final name in ['KeyboardEvent', 'CompositionEvent', 'InputEvent', 'PointerEvent', 'HTMLInputElement', 'HTMLTextAreaElement']) {
      final own = globalContext[name] as JSObject;
      bool instanceOf(JSObject constructor, JSAny? value) => (ordinary.callAsFunction(constructor, value) as JSBoolean).toDart;
      _define(
        own,
        symbol,
        value: ((JSAny? value) {
          if (instanceOf(own, value)) return true.toJS;
          for (final popup in _live) {
            final constructor = (popup as JSObject)[name];
            try {
              if (constructor.isA<JSObject>() && instanceOf(constructor as JSObject, value)) return true.toJS;
            } on Object {
              // A window that went between the check and the call is not one.
            }
          }
          return false.toJS;
        }).toJS,
      );
    }
  }

  // The engine compares the opener's `document.activeElement` with its views' when the framework moves the focus
  // (view_focus_binding.dart, text_editing.dart `handleBlur`): it is the active element of the app window that took the focus last.
  void _followFocus() {
    final own = (_object.callMethod('getOwnPropertyDescriptor'.toJS, _prototype('Document'), 'activeElement'.toJS) as JSObject)['get'] as JSFunction;
    _focus(web.document);
    _define(
      web.document as JSObject,
      'activeElement'.toJS,
      get: (() {
        final last = _focused;
        if (last != null && last != web.document && !(last.defaultView?.closed ?? true)) return last.activeElement;
        return own.callAsFunction(web.document as JSObject);
      }).toJS,
    );
  }

  bool get _popupShown => _live.any((popup) => popup.document.visibilityState == 'visible');

  late final JSFunction _realState = (_object.callMethod('getOwnPropertyDescriptor'.toJS, _prototype('Document'), 'visibilityState'.toJS) as JSObject)['get'] as JSFunction;

  String get _openerState => (_realState.callAsFunction(web.document as JSObject) as JSString).toDart;

  // The engine's lifecycle goes `hidden` on the opener's `visibilitychange` (platform_dispatcher/app_lifecycle_state.dart) and the
  // framework then stops producing frames: while a window of the app is shown, the opener reads visible.
  void _followVisibility() {
    _define(web.document as JSObject, 'visibilityState'.toJS, get: (() => (_popupShown ? 'visible' : _openerState).toJS).toJS);
    _define(web.document as JSObject, 'hidden'.toJS, get: (() => (_popupShown ? false : _openerState == 'hidden').toJS).toJS);
  }

  // Frames come from the opener's `requestAnimationFrame` (frame_service.dart), which a hidden opener stops running, and the frame
  // asked for before it hid stays pending with no other asked for: both are asked of a shown popup too; the first to run wins.
  void _paintFromPopups() {
    final window = web.window as JSObject;
    final own = window['requestAnimationFrame'] as JSFunction;
    final pending = <JSFunction>{};
    web.Window? shown() => _live.where((popup) => popup.document.visibilityState == 'visible').firstOrNull;
    window['requestAnimationFrame'] = ((JSFunction callback) {
      var done = false;
      late final JSFunction run;
      run = ((JSAny? _) {
        if (done) return;
        done = true;
        pending.remove(run);
        callback.callAsFunction(null, web.window.performance.now().toJS);
      }).toJS;
      pending.add(run);
      final popup = shown();
      if (popup != null && _openerState == 'hidden') popup.requestAnimationFrame(run);
      return own.callAsFunction(window, run);
    }).toJS;
    web.document.addEventListener(
      'visibilitychange',
      ((web.Event _) {
        final popup = shown();
        if (popup != null && _openerState == 'hidden') {
          for (final run in pending.toList()) {
            popup.requestAnimationFrame(run);
          }
        }
      }).toJS,
    );
  }
}

final class _ViewWindow implements PopoutWindow {
  _ViewWindow({required this.id, required this.title, required this.popup, required this.viewId, required this.builder, required this.owner}) {
    // A window the person closes says `pagehide` and nothing else reaches the page; the poll catches a window that went without it.
    popup.addEventListener('pagehide', ((web.Event _) => _gone()).toJS);
    _poll = Timer.periodic(const Duration(milliseconds: 500), (_) {
      if (popup.closed) _gone();
    });
  }

  @override
  final String id;
  @override
  String title;
  final web.Window popup;
  final int viewId;
  final WidgetBuilder builder;
  final _ViewWindows owner;
  final _closed = Completer<void>();
  late final Timer _poll;

  @override
  ChatViewLink? get link => null;

  @override
  Size get size => _closed.isCompleted ? Size.zero : Size(popup.innerWidth.toDouble(), popup.innerHeight.toDouble());

  @override
  Future<void> get closed => _closed.future;

  @override
  void setTitle(String title) {
    this.title = title;
    if (!_closed.isCompleted) popup.document.title = '$title - ${Brand.name}';
  }

  // The window's chrome is the browser's: the size asked for is the drawable area, which the window has less of.
  @override
  void setSize(Size size) {
    if (_closed.isCompleted) return;
    popup.resizeTo(size.width.round() + popup.outerWidth - popup.innerWidth, size.height.round() + popup.outerHeight - popup.innerHeight);
  }

  @override
  Future<void> focus() async {
    if (!_closed.isCompleted) popup.focus();
  }

  @override
  Future<void> close() async {
    if (_closed.isCompleted) return;
    _gone();
    if (!popup.closed) popup.close();
  }

  void _gone() {
    if (_closed.isCompleted) return;
    _poll.cancel();
    _closed.complete();
    owner._release(this);
  }
}

/// Windows that are views of this page's app: each is a browser window the page opened, drawn from the widgets the main view's
/// objects give it, with no second copy of the app and no link.
final class _ViewWindows extends WindowList {
  _ViewWindows() {
    // The popups' views die with this page's isolate, so the windows go with the page.
    web.window.addEventListener('pagehide', ((web.Event _) {
      for (final window in [..._open]) {
        unawaited(window.close());
      }
    }).toJS);
  }

  final _open = <_ViewWindow>[];
  late final _shims = _Shims();
  var _next = 0;

  @override
  bool get available => web.window.location.protocol == 'http:' || web.window.location.protocol == 'https:';

  @override
  bool get drawsWidgets => available;

  @override
  bool get linksItems => available;

  // xterm2 5.4.0 opens its text input on the engine's implicit view, which a multi-view engine has none of (its CustomTextEdit takes
  // a viewId only from there, and TerminalView passes none), so a terminal is a second copy of the app in a window of its own.
  @override
  bool drawsItem(PopoutItem item) => item.kind != PopoutItem.terminal;

  final _copies = _WebWindows();

  @override
  Future<PopoutWindow> open({
    required String title,
    required String look,
    required String density,
    String? session,
    Size size = popoutSize,
    WidgetBuilder? builder,
    PopoutItem? item,
  }) async {
    if (builder == null) {
      if (item == null) throw ArgumentError.notNull('builder');
      return track(await _copies.open(title: title, look: look, density: density, session: session, size: size, item: item));
    }
    // Called inside the person's click, so that the browser lets it open: nothing is awaited before it.
    final popup = web.window.open('', 'canoa-window-${_next++}', 'popup=yes,width=${size.width.round()},height=${size.height.round()}');
    if (popup == null) throw StateError('The browser did not open the window: allow pop-ups for this page');
    final document = popup.document;
    document.title = '$title - ${Brand.name}';
    // The styles are set as CSSOM, which a page's content security policy leaves alone.
    (document.documentElement! as web.HTMLElement).style.cssText = 'height:100%;margin:0';
    final body = document.body!;
    body.style.cssText = 'height:100%;margin:0;overflow:hidden';
    final host = document.createElement('div') as web.HTMLElement;
    host.style.cssText = 'position:absolute;inset:0';
    body.append(host);
    _shims.attach(popup);
    final viewId = (_app.callMethod('addView'.toJS, (JSObject()..['hostElement'] = host)) as JSNumber).toDartInt;
    final window = _ViewWindow(id: 'window-$viewId', title: title, popup: popup, viewId: viewId, builder: builder, owner: this);
    _open.add(window);
    return track(window);
  }

  // The window's widget leaves the tree in the frame its removal draws, and only then does the engine lose the view.
  void _release(_ViewWindow window) {
    if (!_open.remove(window)) return;
    notifyListeners();
    unawaited(WidgetsBinding.instance.endOfFrame.then((_) => _app.callMethod('removeView'.toJS, window.viewId.toJS)));
  }

  /// The place the windows are drawn: beside the app, in the same view tree, so that they see what is above it.
  @override
  Widget host(Widget app) => ViewAnchor(
    view: _Views(windows: this),
    child: app,
  );

  @override
  void dispose() {
    for (final window in [..._open]) {
      unawaited(window.close());
    }
    super.dispose();
  }
}

/// The views of the open windows, as the platform's own views of them are found.
final class _Views extends StatefulWidget {
  const _Views({required this.windows});

  final _ViewWindows windows;

  @override
  State<_Views> createState() => _ViewsState();
}

final class _ViewsState extends State<_Views> with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  // A view a window made is the engine's a moment after `addView`, and known to the framework by its metrics.
  @override
  void didChangeMetrics() => setState(() {});

  @override
  Widget build(BuildContext context) {
    final dispatcher = WidgetsBinding.instance.platformDispatcher;
    return ListenableBuilder(
      listenable: widget.windows,
      builder: (context, _) => ViewCollection(
        views: [
          for (final window in widget.windows._open)
            if (dispatcher.view(id: window.viewId) case final view?) View(key: ObjectKey(window), view: view, child: window.builder(context)),
        ],
      ),
    );
  }
}
