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

/// What fills the main area when the person asks for one part alone: the conversation, or one of the panels.
enum PanelFocus { none, main, right, bottom }

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

  /// The right and the bottom panels' sizes, and the smallest each may be. The right one is at most half the window, which the
  /// view enforces since only it knows the window.
  static const rightMin = 320.0, rightDefault = 420.0, bottomMin = 140.0, bottomDefault = 260.0;

  /// Whether each panel is shown. A panel with nothing to show is not drawn whatever this says.
  bool rightOpen = false, bottomOpen = false;
  double rightWidth = rightDefault, bottomHeight = bottomDefault;

  /// The part shown alone, if any; the flags above and the sizes stay as they were, so Restore layout is exact.
  PanelFocus focus = PanelFocus.none;

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

  /// Hides or shows only [id]'s index; its width stays for when it comes back. While a part is shown alone it ends that.
  void toggleIndex(String id) {
    final layout = of(id);
    if (focus != PanelFocus.none) {
      focus = PanelFocus.none;
      layout.collapsed = false;
    } else {
      layout.collapsed = !layout.collapsed;
    }
    notifyListeners();
    _save();
  }

  /// Shows or hides the right panel; while a part is shown alone it ends that and shows the panel.
  void toggleRight() => _toggle(right: true);

  /// Shows or hides the bottom panel, as [toggleRight] does the right one.
  void toggleBottom() => _toggle(right: false);

  void _toggle({required bool right}) {
    if (focus != PanelFocus.none) {
      focus = PanelFocus.none;
      if (right) {
        rightOpen = true;
      } else {
        bottomOpen = true;
      }
    } else if (right) {
      rightOpen = !rightOpen;
    } else {
      bottomOpen = !bottomOpen;
    }
    notifyListeners();
    _save();
  }

  void setRightOpen(bool open) {
    rightOpen = open;
    if (!open && focus == PanelFocus.right) focus = PanelFocus.none;
    notifyListeners();
    _save();
  }

  void setBottomOpen(bool open) {
    bottomOpen = open;
    if (!open && focus == PanelFocus.bottom) focus = PanelFocus.none;
    notifyListeners();
    _save();
  }

  /// Shows the right panel without hiding it when it is shown.
  void showRight() {
    if (rightOpen && focus == PanelFocus.none) return;
    focus = focus == PanelFocus.main ? PanelFocus.none : focus;
    rightOpen = true;
    notifyListeners();
    _save();
  }

  void showBottom() {
    if (bottomOpen && focus == PanelFocus.none) return;
    focus = focus == PanelFocus.main ? PanelFocus.none : focus;
    bottomOpen = true;
    notifyListeners();
    _save();
  }

  void setRightWidth(double width, {bool keep = false}) {
    rightWidth = width < rightMin ? rightMin : width;
    notifyListeners();
    if (keep) _save();
  }

  void setBottomHeight(double height, {bool keep = false}) {
    bottomHeight = height < bottomMin ? bottomMin : height;
    notifyListeners();
    if (keep) _save();
  }

  void resetRightWidth() {
    rightWidth = rightDefault;
    notifyListeners();
    _save();
  }

  void resetBottomHeight() {
    bottomHeight = bottomDefault;
    notifyListeners();
    _save();
  }

  /// Shows the conversation alone, or one panel alone.
  void focusOn(PanelFocus part) {
    focus = part;
    notifyListeners();
    _save();
  }

  /// Ends a part shown alone, returning to the arrangement it left.
  void restore() => focusOn(PanelFocus.none);

  /// Every index shown at its default width, both panels hidden at their default sizes.
  void reset() {
    for (final layout in _sections.values) {
      layout
        ..collapsed = false
        ..indexWidth = null;
    }
    rightOpen = bottomOpen = false;
    rightWidth = rightDefault;
    bottomHeight = bottomDefault;
    focus = PanelFocus.none;
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
    'panels': {
      if (rightOpen) 'rightOpen': true,
      if (bottomOpen) 'bottomOpen': true,
      'rightWidth': rightWidth,
      'bottomHeight': bottomHeight,
    },
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
      if (json['panels'] case final Map panels) {
        rightOpen = panels['rightOpen'] == true;
        bottomOpen = panels['bottomOpen'] == true;
        if (panels['rightWidth'] case final num width) rightWidth = width < rightMin ? rightMin : width.toDouble();
        if (panels['bottomHeight'] case final num height) bottomHeight = height < bottomMin ? bottomMin : height.toDouble();
      }
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
