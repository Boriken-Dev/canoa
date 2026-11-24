import 'package:flutter/foundation.dart';

/// The sessions open as tabs in one window, in the order the strip shows them, and which one the window shows. It holds session ids
/// only: a tab is a place in a window, and what it shows stays with the session store.
class WindowTabs extends ChangeNotifier {
  final _sessions = <String>[];
  String? _current;

  /// The ids of the tabs, left to right.
  List<String> get sessions => List.unmodifiable(_sessions);

  /// The tab the window shows; null when it has none.
  String? get current => _current;

  bool get isEmpty => _sessions.isEmpty;

  int get length => _sessions.length;

  bool contains(String id) => _sessions.contains(id);

  /// Adds a tab for [id] at the end and shows it, or shows the one it already has.
  void open(String id) {
    final added = !_sessions.contains(id);
    if (!added && _current == id) return;
    if (added) _sessions.add(id);
    _current = id;
    notifyListeners();
  }

  /// Shows [id]'s tab; nothing when the window has none.
  void focus(String id) {
    if (!_sessions.contains(id) || _current == id) return;
    _current = id;
    notifyListeners();
  }

  /// Takes [id]'s tab away. When it was the one shown, the tab after it is, or the one before when it was last. Returns the tab shown
  /// afterwards.
  String? remove(String id) {
    final at = _sessions.indexOf(id);
    if (at < 0) return _current;
    _sessions.removeAt(at);
    if (_current == id) _current = _sessions.isEmpty ? null : _sessions[at < _sessions.length ? at : _sessions.length - 1];
    notifyListeners();
    return _current;
  }

  /// Moves the tab at [from] to [to], an index in the list once it has left [from]; the tab shown stays the same one.
  void reorder(int from, int to) {
    if (from < 0 || from >= _sessions.length) return;
    final target = to.clamp(0, _sessions.length - 1);
    if (target == from) return;
    _sessions.insert(target, _sessions.removeAt(from));
    notifyListeners();
  }

  /// Keeps the tabs of the sessions [test] accepts.
  void retain(bool Function(String id) test) {
    final gone = _sessions.where((id) => !test(id)).toList();
    for (final id in gone) {
      remove(id);
    }
  }
}
