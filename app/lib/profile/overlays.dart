import 'profile.dart';

/// What an overlay holds at one commit: each file, relative to the overlay's directory, with its git blob id.
typedef OverlayFiles = Map<String, String>;

/// An overlay installed on this computer: the commit it was fetched at and the tree digest of the files installed.
final class InstalledOverlay {
  const InstalledOverlay({required this.name, required this.source, required this.commit, required this.files,
      this.digest, this.skipped = const [], this.installed});

  factory InstalledOverlay.fromJson(String name, Map<String, dynamic> json) => InstalledOverlay(
    name: name,
    source: json['source'] as String,
    commit: json['commit'] as String,
    digest: json['digest'] as String?,
    files: {for (final MapEntry(:key, :value) in (json['files'] as Map).entries) '$key': '$value'},
    skipped: [for (final path in (json['skipped'] as List?) ?? const []) '$path'],
    installed: DateTime.tryParse('${json['installed']}'),
  );

  final String name;

  /// The source it was installed from, as canoa.cfg wrote it then.
  final String source;
  final String commit;

  /// The AOS tree digest of what was installed; null in a record from before Canoa read AOS overlays.
  final String? digest;
  final OverlayFiles files;

  /// Entries under an excluded name (`.git`, `node_modules`, ...), which are no part of an overlay and never installed.
  final List<String> skipped;
  final DateTime? installed;

  Map<String, Object?> toJson() => {
    'source': source,
    'commit': commit,
    'digest': ?digest,
    'files': files,
    'skipped': skipped,
    if (installed != null) 'installed': installed!.toUtc().toIso8601String(),
  };
}

/// What installing or syncing an overlay would change, from the commit installed (none for a first install) to [commit].
final class OverlayUpdate {
  const OverlayUpdate({required this.name, required this.source, required this.from, required this.commit,
      required this.digest, required this.files, required this.skipped, required this.added, required this.changed,
      required this.removed, this.fromSource, this.fromDigest, this.manifest});

  final String name;
  final GitOverlay source;
  final String? from;

  /// The source the installed commit came from, as its record has it.
  final String? fromSource;

  /// The tree digest installed now, as its record has it.
  final String? fromDigest;
  final String commit;

  /// The AOS tree digest of the files at [commit], which applying installs and pins.
  final String digest;
  final OverlayFiles files;
  final List<String> skipped, added, changed, removed;

  /// The overlay's overlay.cfg at [commit], or null when it has none.
  final String? manifest;

  bool get changesNothing =>
      from == commit && fromSource == source.spec && fromDigest == digest && added.isEmpty && changed.isEmpty &&
      removed.isEmpty;
}

/// A git operation or an overlay's content that was refused. Never quotes a file's content.
final class OverlayException implements Exception {
  const OverlayException(this.message);
  final String message;
  @override
  String toString() => message;
}

/// Installs named overlays from git and syncs them, each verified by its AOS tree digest.
abstract interface class OverlayManager {
  /// The overlay installed as [name], or null.
  InstalledOverlay? installed(String name);

  /// The directory an installed overlay's files are in.
  String directoryOf(String name);

  /// Fetches [source] and says what installing it as [name] would change, touching nothing installed. Refuses a tree
  /// whose digest is not the source's pin.
  Future<OverlayUpdate> check(String name, GitOverlay source);

  /// Installs exactly what [update] showed.
  Future<InstalledOverlay> apply(OverlayUpdate update);

  /// Removes what [name] installed and fetched.
  Future<void> remove(String name);

  /// The bytes of every file [update] lists, read from what [check] fetched, without installing anything: what a store
  /// shows an overlay would do is computed from these.
  Future<Map<String, List<int>>> read(OverlayUpdate update);

  /// Fetches the repository of the catalog labelled [label] at [source]'s ref, as [check] fetches an overlay, and gives
  /// its files at that commit to read one by one.
  Future<FetchedTree> fetchCatalog(String label, GitOverlay source);

  /// Removes what [fetchCatalog] fetched for [label].
  Future<void> removeCatalog(String label);

  /// Removes what [check] fetched for [name] when nothing is installed under that name: a preview that was not
  /// installed leaves nothing behind.
  Future<void> discard(String name);
}

/// One file of a fetched tree: its size, and its bytes unless it was larger than asked.
typedef TreeFile = ({int size, List<int>? bytes});

/// A repository's files at one fetched commit, read from git's objects, never checked out.
abstract interface class FetchedTree {
  String get commit;

  /// The regular file at [path], `/`-separated from the repository's root, or null when there is none (a directory, a
  /// link or a submodule is none). Its bytes are read only when it holds at most [maxBytes].
  Future<TreeFile?> file(String path, {required int maxBytes});
}
