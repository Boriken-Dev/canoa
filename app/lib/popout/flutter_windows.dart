// The one file that reaches Flutter's windowing API, which marks every class `@internal` and promises to break in any release:
// a change of the SDK is absorbed here, and `test/window_seam_test.dart` fails when another file imports it.
// ignore_for_file: invalid_use_of_internal_member
import 'dart:async';
import 'dart:io' show Platform;

import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;
// The windowing API is not exported; `implementation_imports` is the price of using it before it is stable.
// ignore: implementation_imports
import 'package:flutter/src/foundation/_features.dart' show isWindowingEnabled;
// ignore: implementation_imports
import 'package:flutter/src/widgets/_window.dart';
import 'package:flutter/widgets.dart';

import '../brand.dart';
import 'popout_window.dart';

/// Closing a window of this engine closes only that window; the main window is not one of them.
final class _Delegate with WindowControllerDelegate {
  _Delegate(this.onClose, this.onGone);

  final void Function() onClose, onGone;

  @override
  void onWindowCloseRequested(WindowController controller) => onClose();

  @override
  void onWindowDestroyed() {
    super.onWindowDestroyed();
    onGone();
  }
}

final class _FlutterWindow implements PopoutWindow {
  _FlutterWindow({required this.id, required String title, required Size size, required this.brand, required this.owner, required WidgetBuilder builder})
    : _title = title {
    controller = WindowController(
      size: size,
      title: _named(title),
      delegate: _Delegate(() => unawaited(close()), _gone),
    );
    entry = WindowEntry(controller: controller, builder: builder);
  }

  @override
  final String id;
  final String brand;
  final FlutterWindows owner;
  late final WindowController controller;
  late final WindowEntry entry;
  final _closed = Completer<void>();
  String _title;

  String _named(String title) => '$title - $brand';

  @override
  ChatViewLink? get link => null;

  @override
  String get title => _title;

  @override
  Size get size => _closed.isCompleted || controller.isDestroyed ? Size.zero : controller.contentSize;

  @override
  Future<void> get closed => _closed.future;

  @override
  void setTitle(String title) {
    _title = title;
    if (!_closed.isCompleted && !controller.isDestroyed) controller.setTitle(_named(title));
  }

  @override
  void setSize(Size size) {
    if (!_closed.isCompleted && !controller.isDestroyed) controller.setSize(size);
  }

  @override
  Future<void> focus() async {
    if (!_closed.isCompleted && !controller.isDestroyed) controller.activate();
  }

  // The registry lets go of a window before its native window is destroyed.
  @override
  Future<void> close() async {
    if (_closed.isCompleted) return;
    owner._registry.unregister(entry);
    if (!controller.isDestroyed) controller.destroy();
    _gone();
  }

  void _gone() {
    if (_closed.isCompleted) return;
    owner._registry.unregister(entry);
    _closed.complete();
    // The window's widget leaves the tree in the frame this change draws, and only then may the controller go.
    unawaited(WidgetsBinding.instance.endOfFrame.then((_) => controller.dispose()));
  }
}

/// Windows of this engine over Flutter's multi-window API: each is a [WindowController] drawn from a [WindowRegistry] the app's root
/// owns, so what a window shows is built in this isolate from the objects the main window holds, with no copy and no connection of
/// its own.
final class FlutterWindows extends WindowList {
  FlutterWindows({String? brand}) : _brand = brand ?? Brand.name;

  final String _brand;
  final WindowRegistry _registry = WindowRegistry();
  var _next = 0;

  @override
  bool get available => isWindowingEnabled && (Platform.isWindows || Platform.isLinux || Platform.isMacOS);

  @override
  bool get drawsWidgets => available;

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
    if (!available) throw UnsupportedError('Flutter\'s windowing is not on in this build');
    if (builder == null) throw ArgumentError.notNull('builder');
    final window = _FlutterWindow(id: 'window-${_next++}', title: title, size: size, brand: _brand, owner: this, builder: builder);
    _registry.register(window.entry);
    return track(window);
  }

  /// The place the windows are drawn: beside the app, in the same view tree, so that they see what is above it.
  @override
  Widget host(Widget app) => ViewAnchor(
    view: ListenableBuilder(
      listenable: _registry,
      builder: (context, _) => ViewCollection(
        views: [for (final entry in _registry.windows) Window(key: ObjectKey(entry), controller: entry.controller, child: entry.builder(context))],
      ),
    ),
    child: app,
  );

  @override
  void dispose() {
    for (final window in [...windows]) {
      unawaited(window.close());
    }
    super.dispose();
  }
}
