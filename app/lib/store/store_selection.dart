import 'package:flutter/material.dart';

import '../shell/layout_state.dart';

/// A small picture an extension ships as its type's icon: the bytes of a PNG or WebP and a file name whose extension says which.
final class StoreMarkImage {
  const StoreMarkImage({required this.path, required this.bytes});

  final String path;
  final List<int> bytes;
}

/// One store type the Store section's index lists: its id, its name, its icon and how much of it there is.
final class StoreTypeRow {
  const StoreTypeRow({required this.id, required this.label, required this.icon, required this.installed, this.available, this.image});

  final String id, label;
  final IconData icon;

  /// A picture drawn in place of [icon], for a type whose extension ships one.
  final StoreMarkImage? image;

  /// How many are installed, and how many the catalogs (or the registry) offer; null while that is not known.
  final int installed;
  final int? available;

  bool same(StoreTypeRow other) =>
      id == other.id && label == other.label && icon == other.icon && identical(image, other.image) && installed == other.installed && available == other.available;
}

/// The store type chosen in the Store section's index, which the section's content shows.
///
/// The section publishes the types it has, in order (the built-in ones, then those extensions add), and the index and the
/// content both listen. The choice is kept with the shell's layout, so it comes back after a restart; a type that is no longer
/// listed reads as the first.
class StoreSelection extends ChangeNotifier {
  StoreSelection({this.layout});

  /// The section's id in the shell's layout.
  static const section = 'store';

  final ShellLayout? layout;

  List<StoreTypeRow> _types = const [];
  String? _chosen;
  bool _restored = false;

  /// The types, in the order of the index.
  List<StoreTypeRow> get types => _types;

  /// The id of the chosen type, or of the first when none was chosen or the chosen one is not listed; null before any type is
  /// published and nothing was kept.
  String? get selected {
    if (!_restored) {
      _restored = true;
      _chosen = layout?.of(section).item;
    }
    if (_types.isEmpty) return _chosen;
    final chosen = _chosen;
    return _types.any((type) => type.id == chosen) ? chosen : _types.first.id;
  }

  /// Chooses the type [id].
  void select(String id) {
    if (selected == id) return;
    _chosen = id;
    layout?.setItem(section, id);
    notifyListeners();
  }

  /// Takes the types the section has.
  void publish(List<StoreTypeRow> types) {
    final same = types.length == _types.length && [for (var i = 0; i < types.length; i++) types[i].same(_types[i])].every((equal) => equal);
    if (same) return;
    _types = types;
    notifyListeners();
  }
}
