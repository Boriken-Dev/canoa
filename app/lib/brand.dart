import 'dart:convert';

/// What this build is called and how it names things, set by the build from the brand file (`app/brand.json`) as
/// `--dart-define`s: the display name, the identifiers, the command line's name, the configuration file and the folders, the prefix of
/// the environment variables the app reads, the accent and the mark. Without a definition a value is the brand file's own; `test/brand_test.dart` checks they agree.
abstract final class Brand {
  /// What a person reads for the application: the window title, the tray's tooltip and menu.
  static const String name = String.fromEnvironment('BRAND_NAME', defaultValue: 'Canoa');

  /// The name where room is short.
  static const String shortName = String.fromEnvironment('BRAND_SHORT_NAME', defaultValue: 'Canoa');

  /// The reverse-DNS identifier.
  static const String id = String.fromEnvironment('BRAND_ID', defaultValue: 'dev.boriken.canoa');

  /// The desktop program's name, without its extension.
  static const String executable = String.fromEnvironment('BRAND_EXECUTABLE', defaultValue: 'CanoaUI');

  /// The command line's name: what messages tell a person to run and what the host's tool servers are named for.
  static const String program = String.fromEnvironment('BRAND_PROGRAM', defaultValue: 'canoa');

  /// The configuration file's name in every layer.
  static const String configFile = String.fromEnvironment('BRAND_CONFIG_FILE', defaultValue: 'canoa.cfg');

  /// The one folder under each operating system's configuration, system and data roots.
  static const String folder = String.fromEnvironment('BRAND_FOLDER', defaultValue: 'Canoa');

  /// The home under the person's own folder, and the per-project folder: a dot first.
  static const String homeDir = String.fromEnvironment('BRAND_HOME_DIR', defaultValue: '.canoa');

  /// The prefix of the environment variables the app and its host read, without the underscore.
  static const String envPrefix = String.fromEnvironment('BRAND_ENV_PREFIX', defaultValue: 'CANOA');

  /// The name of the configuration file (`canoa.cfg`), which also names a project's profile file.
  static const String configFile = String.fromEnvironment('BRAND_CONFIG_FILE', defaultValue: 'canoa.cfg');

  /// The folder a project and the user's home keep this program's files in (`.canoa`).
  static const String homeDir = String.fromEnvironment('BRAND_HOME_DIR', defaultValue: '.canoa');

  /// The accent the build's icons are drawn in, `#RRGGBB`.
  static const String accentHex = String.fromEnvironment('BRAND_ACCENT', defaultValue: '#9A5B1E');

  /// The mark's paths as the build read them from its SVG (a JSON object), or empty for Canoa's own.
  static const String markJson = String.fromEnvironment('BRAND_MARK');

  /// The environment variable `suffix` of this brand: `CONFIG` is `CANOA_CONFIG` in Canoa's own build.
  static String env(String suffix) => '${envPrefix}_$suffix';

  /// The mark's path definitions, or null when the build gave none.
  static Map<String, Object?>? get mark {
    if (markJson.isEmpty) return null;
    final decoded = jsonDecode(markJson);
    return decoded is Map<String, Object?> ? decoded : null;
  }
}
