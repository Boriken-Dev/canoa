import 'package:flutter/foundation.dart';

import '../shell/layout_state.dart';
import 'store.dart' show LoadedCatalog;

/// One store the Store section's index lists: a catalog as it was read, and how many overlays and skills it offers.
final class StoreSource {
  const StoreSource({required this.label, required this.overlays, required this.skills, this.problem, this.stale = false});

  factory StoreSource.of(LoadedCatalog catalog) => StoreSource(
    label: catalog.label,
    overlays: catalog.catalog?.entries.length ?? 0,
    skills: catalog.catalog?.skills.length ?? 0,
    problem: catalog.problem ?? catalog.staleBecause,
    stale: catalog.catalog != null && (catalog.problem ?? catalog.staleBecause) != null,
  );

  final String label;
  final int overlays, skills;

  /// Why the store could not be read, or only its last copy is shown.
  final String? problem;

  /// Whether the entries shown are an older copy.
  final bool stale;

  bool get failed => problem != null && !stale;
}

/// The store chosen in the Store section's index, which the section's content is scoped to; none means every store.
///
/// The section's content publishes the stores it has read, and the index and the content both listen. The choice is kept with
/// the shell's layout, so it comes back after a restart; a store that is no longer listed reads as none.
class StoreSelection extends ChangeNotifier {
  StoreSelection({this.layout});

  /// The section's id in the shell's layout.
  static const section = 'store';

  final ShellLayout? layout;

  List<StoreSource> _sources = const [];
  bool _read = false;
  bool _loading = false;
  String? _chosen;
  bool _restored = false;

  /// The stores read, in the order the host gave them.
  List<StoreSource> get sources => _sources;

  /// Whether the stores are being read now.
  bool get loading => _loading;

  /// Whether any read has finished, which is when a kept choice of a store no longer listed may be dropped.
  bool get read => _read;

  /// The label of the chosen store, or null for every store.
  String? get selected {
    if (!_restored) {
      _restored = true;
      _chosen = layout?.of(section).item;
    }
    final chosen = _chosen;
    if (chosen == null) return null;
    if (_read && !_sources.any((source) => source.label == chosen)) return null;
    return chosen;
  }

  StoreSource? get selectedSource {
    final label = selected;
    return label == null ? null : _sources.where((source) => source.label == label).firstOrNull;
  }

  /// Chooses the store [label], or every store for null.
  void select(String? label) {
    if (selected == label) return;
    _chosen = label;
    layout?.setItem(section, label);
    notifyListeners();
  }

  /// Takes what the section has read of its stores.
  void publish(List<LoadedCatalog> catalogs, {required bool loading}) {
    final next = [for (final catalog in catalogs) StoreSource.of(catalog)];
    if (!loading) _read = true;
    final same = _loading == loading &&
        next.length == _sources.length &&
        [for (var i = 0; i < next.length; i++) _same(next[i], _sources[i])].every((equal) => equal);
    if (same) return;
    _sources = next;
    _loading = loading;
    notifyListeners();
  }

  static bool _same(StoreSource a, StoreSource b) =>
      a.label == b.label && a.overlays == b.overlays && a.skills == b.skills && a.problem == b.problem && a.stale == b.stale;
}
