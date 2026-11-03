import 'overlays.dart';
import 'project_trust.dart';

/// What profiles need from this computer: where overlays install, which projects are trusted, and reading files.
/// A desktop has one beside Canoa.cfg; a web build has none.
abstract interface class ProfileHome {
  /// Canoa.cfg's directory, which the user's and the agents' relative paths are read from.
  String get configDir;

  OverlayManager get overlays;

  ProjectTrust get trust;

  /// [relative], `/`-separated, under [dir].
  String join(String dir, String relative);

  /// [directory] with its links resolved, or null when it is not a directory.
  String? canonicalDirectory(String directory);

  /// A file's bytes, or null when it cannot be read.
  Future<List<int>?> readBytes(String path);

  /// A file's text for a secret, or null when it cannot be read as UTF-8, or when [within] is given and the file, its
  /// links resolved, is not inside that directory.
  Future<String?> readSecret(String path, {String? within});
}
