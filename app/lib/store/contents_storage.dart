/// Where what was read of every entry of a catalog is kept between runs: one document for each catalog, under a cache directory
/// that can be deleted at any time, since it is read again from the sources.
abstract interface class ContentsStorage {
  /// The document kept for the catalog [label], or null when there is none or it cannot be read.
  Future<String?> read(String label);

  /// Keeps [json] for [label]; a failure is the caller's to ignore, the cache being only a cache.
  Future<void> write(String label, String json);
}

/// Kept only while the app runs.
final class MemoryContentsStorage implements ContentsStorage {
  final kept = <String, String>{};

  @override
  Future<String?> read(String label) async => kept[label];

  @override
  Future<void> write(String label, String json) async => kept[label] = json;
}
