import 'package:flutter/material.dart';

/// The Store's built-in types, in the order of the index.
enum StoreType {
  overlays('Overlays', 'overlays', Icons.layers_outlined),
  skills('Skills', 'skills', Icons.auto_stories_outlined),
  agents('Agents', 'agents', Icons.smart_toy_outlined),
  mcps('MCP servers', 'mcps', Icons.hub_outlined);

  const StoreType(this.label, this.id, this.icon);
  final String label;

  /// The word in this type's keys, and the id the index keeps its choice by.
  final String id;
  final IconData icon;
}

/// What the toolbar holds for one type: the Installed/Discover switch, the search and the filters. It is kept per type, so
/// leaving a type and coming back, or opening an entry's details and returning, finds them as they were.
class StoreView extends ChangeNotifier {
  StoreView(this.id);

  final String id;
  bool _discover = false;
  final query = TextEditingController();

  /// Discover's filters: the tags an entry must carry and the catalogs it may come from (none chosen means all).
  final tags = <String>{};
  final catalogs = <String>{};

  /// The Skills page's Discover reads this skill source (a label of `[store.skill_sources]`) instead of the catalogs; null reads
  /// the catalogs.
  String? source;

  /// Installed's filter for the entries with a newer version.
  bool updatesOnly = false;

  bool get discover => _discover;
  set discover(bool value) {
    if (_discover == value) return;
    _discover = value;
    notifyListeners();
  }

  /// Tells whoever draws this view that a filter or the query changed.
  void changed() => notifyListeners();

  /// Whether [text] holds every word of the query, ignoring case.
  bool matches(String text) {
    final haystack = text.toLowerCase();
    return query.text.toLowerCase().split(RegExp(r'\s+')).where((word) => word.isNotEmpty).every(haystack.contains);
  }

  @override
  void dispose() {
    query.dispose();
    super.dispose();
  }
}

/// How long ago, for the line under the toolbar: `just now`, `3 min ago`, `2 h ago`, `4 days ago`.
String ageText(Duration age) {
  if (age.inSeconds < 60) return 'just now';
  if (age.inMinutes < 60) return '${age.inMinutes} min ago';
  if (age.inHours < 48) return '${age.inHours} h ago';
  return '${age.inDays} days ago';
}
