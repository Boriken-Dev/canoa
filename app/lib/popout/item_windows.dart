import 'dart:async';

import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;
import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart' show hostWords;
import '../l10n/app_text.dart';
import '../session_store.dart';
import '../theme/look.dart';
import 'popout_window.dart';

/// What gives a window of this engine the app's own text and look: [home] drawn under the main window's theme and language.
typedef WindowFrame = Widget Function(BuildContext context, Widget home);

/// What serves an item to a window that is another page, over [link]: it answers what the page asks and is released when the window
/// goes. Returns what ends the serving.
typedef ItemServe = VoidCallback Function(ChatViewLink link);

/// Windows for the things of a session that are not its conversation: a file's viewer and a terminal. Each is a window of this
/// engine drawing a widget the app gives it, so it shows the objects the main window holds and nothing is copied. An item has one
/// window, named by its key; asking for it again brings that window forward. A window never owns what it shows: closing it leaves
/// the item as it was.
class ItemWindows extends ChangeNotifier {
  ItemWindows(this.windows, {this.store, this.frame, this.look = 'canoa', this.density = 'normal'}) {
    store?.addListener(_storeChanged);
  }

  final PopoutWindows windows;

  /// The sessions the items belong to: an item whose session is closed has no window.
  final SessionStore? store;

  /// How a window's content gets the app's theme and words; a plain Material app in the look [look] when null.
  WindowFrame? frame;

  /// The names of the app's look and density, for a window opened without a [frame].
  String look, density;

  final _open = <String, _Item>{};
  final _opening = <String>{};

  /// Whether this platform can show an item in a window: a window of this engine draws it, and a window that is another page shows
  /// it from what its link serves.
  bool get available => windows.available && (windows.drawsWidgets || windows.linksItems);

  /// Whether a plain click on a file or a shell opens it in a window of its own (Settings, Appearance, Open in).
  bool openInNewWindows = false;

  /// Whether [key]'s window is open.
  bool isOpen(String key) => _open.containsKey(key);

  /// The keys of the windows open now, oldest first.
  List<String> get keys => List.unmodifiable(_open.keys);

  /// The window of [key], when it is open.
  PopoutWindow? windowOf(String key) => _open[key]?.window;

  /// Opens a window titled [title] drawing [builder] for the item [key] of [session], or brings its window forward when it has one.
  ///
  /// [titleOf] gives the title again whenever [changes] tells; [onClosed] runs once the window is gone, however it went. A window that
  /// is another page cannot draw [builder]: it is told [item] to show, and [serve] answers what it asks over its link.
  Future<void> show({
    required String key,
    required String session,
    required String title,
    required WidgetBuilder builder,
    Listenable? changes,
    String Function()? titleOf,
    VoidCallback? onClosed,
    Size size = popoutSize,
    PopoutItem? item,
    ItemServe? serve,
  }) async {
    final held = _open[key];
    if (held != null) return held.window.focus();
    if (!available) throw StateError('This platform has no windows of its own for this');
    // Two requests for the same item can both pass the check above; the second finds the first at work.
    final linked = !windows.drawsWidgets;
    if (linked && (item == null || serve == null)) throw StateError('This item has no window of its own on this platform');
    if (!_opening.add(key)) return;
    try {
      final window = await windows.open(
        title: title,
        look: look,
        density: density,
        size: size,
        builder: linked ? null : (context) => _framed(context, builder),
        item: linked ? item : null,
      );
      final link = window.link;
      if (linked && link == null) {
        unawaited(window.close());
        throw StateError('A window of this engine draws what it is given');
      }
      final held = _Item(window, session, changes, titleOf, onClosed, linked ? serve!(link!) : null);
      _open[key] = held;
      changes?.addListener(held.retitle);
      unawaited(
        window.closed.then((_) {
          if (!identical(_open[key], held)) return;
          _open.remove(key);
          held.release();
          notifyListeners();
        }),
      );
      notifyListeners();
    } finally {
      _opening.remove(key);
    }
  }

  Widget _framed(BuildContext context, WidgetBuilder builder) {
    final content = Builder(builder: builder);
    final make = frame;
    if (make != null) return make(context, content);
    final id = LookId.named(look) ?? LookId.canoa;
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: themeFor(id, Brightness.light),
      darkTheme: themeFor(id, Brightness.dark),
      home: content,
    );
  }

  /// Closes [key]'s window; the item goes on where it is.
  Future<void> close(String key) async => _open[key]?.window.close();

  /// Closes every window whose key [test] accepts.
  Future<void> closeWhere(bool Function(String key) test) => Future.wait([
    for (final key in keys)
      if (test(key)) close(key),
  ]);

  void _storeChanged() {
    final here = {for (final session in store!.sessions) session.id};
    for (final MapEntry(:key, :value) in _open.entries.toList()) {
      if (!here.contains(value.session)) unawaited(close(key));
    }
  }

  @override
  void dispose() {
    store?.removeListener(_storeChanged);
    for (final item in _open.values.toList()) {
      item.release();
      unawaited(item.window.close());
    }
    _open.clear();
    super.dispose();
  }
}

final class _Item {
  _Item(this.window, this.session, this.changes, this.titleOf, this.onClosed, this.stopServing);

  final PopoutWindow window;
  final String session;
  final Listenable? changes;
  final String Function()? titleOf;
  final VoidCallback? onClosed;
  final VoidCallback? stopServing;
  bool _released = false;

  void retitle() {
    final title = titleOf?.call();
    if (title != null && title != window.title) window.setTitle(title);
  }

  void release() {
    if (_released) return;
    _released = true;
    changes?.removeListener(retitle);
    stopServing?.call();
    onClosed?.call();
  }
}

/// Shows an item in a window of its own as the person asked for it; says in [context]'s messenger why it could not.
Future<void> showItemWindow(
  BuildContext context,
  ItemWindows windows, {
  required String key,
  required String session,
  required String title,
  required WidgetBuilder builder,
  Listenable? changes,
  String Function()? titleOf,
  VoidCallback? onClosed,
  PopoutItem? item,
  ItemServe? serve,
}) async {
  final messenger = ScaffoldMessenger.maybeOf(context);
  final text = AppText.of(context);
  try {
    await windows.show(key: key, session: session, title: title, builder: builder, changes: changes, titleOf: titleOf, onClosed: onClosed, item: item, serve: serve);
  } on Object catch (error) {
    messenger?.showSnackBar(SnackBar(content: Text(text('window.failed', {'why': hostWords(error)}))));
  }
}
