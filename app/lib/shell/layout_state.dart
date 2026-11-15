import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';

/// Where the layout is kept between runs: one JSON document.
abstract interface class LayoutBackend {
  Future<String?> read();
  Future<void> write(String json);
}

/// Kept only while the app runs.
final class MemoryLayoutBackend implements LayoutBackend {
  String? kept;

  @override
  Future<String?> read() async => kept;
  @override
  Future<void> write(String json) async => kept = json;
}

/// What one section remembers: the width of its index, whether the index is hidden, the item chosen in it and how far it scrolled.
class SectionLayout {
  double? indexWidth;
  bool collapsed = false;
  String? item;
  double scroll = 0;

  Map<String, Object?> toJson() => {
    'indexWidth': ?indexWidth,
    if (collapsed) 'collapsed': true,
    'item': ?item,
    if (scroll > 0) 'scroll': scroll,
  };

  static SectionLayout fromJson(Object? json) {
    final layout = SectionLayout();
    if (json is! Map) return layout;
    if (json['indexWidth'] case final num width) layout.indexWidth = width.toDouble();
    layout.collapsed = json['collapsed'] == true;
    if (json['item'] case final String item) layout.item = item;
    if (json['scroll'] case final num scroll) layout.scroll = scroll.toDouble();
    return layout;
  }
}

/// The shell's layout: the section shown and, for each section, its index. Kept per device in `layout.json` beside canoa.cfg's
/// `drafts/`, since it is how this window looks and not configuration the host reads.
class ShellLayout extends ChangeNotifier {
  ShellLayout({LayoutBackend? backend}) : _backend = backend;

  final LayoutBackend? _backend;

  /// The narrowest and widest an index may be.
  static const minIndex = 200.0, maxIndex = 480.0;

  /// The section shown.
  String section = conversations;
  static const conversations = 'conversations';

  final Map<String, SectionLayout> _sections = {};

  SectionLayout of(String id) => _sections.putIfAbsent(id, SectionLayout.new);

  /// The width of [id]'s index, [fallback] until the person drags it.
  double indexWidth(String id, double fallback) => (of(id).indexWidth ?? fallback).clamp(minIndex, maxIndex);

  bool collapsed(String id) => of(id).collapsed;

  /// Shows [id].
  void select(String id) {
    if (section == id) return;
    section = id;
    notifyListeners();
    _save();
  }

  /// Sets the width of [id]'s index within its bounds; [keep] writes it down, as at the end of a drag.
  void setIndexWidth(String id, double width, {bool keep = false}) {
    of(id).indexWidth = width.clamp(minIndex, maxIndex);
    notifyListeners();
    if (keep) _save();
  }

  void resetIndexWidth(String id) {
    of(id).indexWidth = null;
    notifyListeners();
    _save();
  }

  /// Hides or shows only [id]'s index; its width stays for when it comes back.
  void toggleIndex(String id) {
    final layout = of(id);
    layout.collapsed = !layout.collapsed;
    notifyListeners();
    _save();
  }

  void setItem(String id, String? item) {
    if (of(id).item == item) return;
    of(id).item = item;
    _save();
  }

  void setScroll(String id, double offset) => of(id).scroll = offset;

  /// Writes what is held now.
  Future<void> flush() => _save();

  Future<void> _save() async {
    final backend = _backend;
    if (backend == null) return;
    try {
      await backend.write(jsonEncode(toJson()));
    } on Object {
      // A layout that cannot be kept is the next run's default, not an error to show.
    }
  }

  Map<String, Object?> toJson() => {
    'section': section,
    'sections': {for (final MapEntry(:key, :value) in _sections.entries) key: value.toJson()},
  };

  /// Reads what an earlier run kept; anything unreadable leaves the defaults.
  Future<void> load() async {
    final backend = _backend;
    if (backend == null) return;
    try {
      final text = await backend.read();
      if (text == null) return;
      final json = jsonDecode(text);
      if (json is! Map) return;
      if (json['section'] case final String id) section = id;
      if (json['sections'] case final Map sections) {
        for (final MapEntry(:key, :value) in sections.entries) {
          if (key is String) _sections[key] = SectionLayout.fromJson(value);
        }
      }
      notifyListeners();
    } on Object {
      // Left as it was.
    }
  }
}
