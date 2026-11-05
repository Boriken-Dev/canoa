import 'aos.dart';
import 'overlays.dart';
import 'project_trust.dart';

/// What profiles need from this computer: where overlays install, which projects are trusted, and reading files.
/// A desktop has one beside canoa.cfg; a web build has none.
abstract interface class ProfileHome {
  /// canoa.cfg's directory, which the user's and the agents' relative paths are read from.
  String get configDir;

  /// This computer's OS as AOS names it: `windows`, `linux` (Android too), `macos`, ...
  String get os;

  OverlayManager get overlays;

  ProjectTrust get trust;

  /// [relative], `/`-separated, under [dir].
  String join(String dir, String relative);

  /// [directory] with its links resolved, or null when it is not a directory.
  String? canonicalDirectory(String directory);

  /// A file's bytes, or null when it cannot be read.
  Future<List<int>?> readBytes(String path);

  /// Every entry under [directory], read without following a link: a link or junction is an entry of its own, never
  /// entered, and an excluded directory (`.git`, `node_modules`, ...) is not entered either. Throws `too-large` past
  /// [maxFiles] files or [maxBytes] bytes, before the rest is read.
  Future<List<AosTreeEntry>> readTree(String directory, {int maxFiles = 4096, int maxBytes = 64 * 1024 * 1024});

  /// A file's text for a secret, or null when it cannot be read as UTF-8, or when [within] is given and the file, its
  /// links resolved, is not inside that directory.
  Future<String?> readSecret(String path, {String? within});
}
