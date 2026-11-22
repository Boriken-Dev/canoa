import 'dart:convert';
import 'dart:ui' show Color;

/// What this build is called and how it names things, set by the build from the brand file (`app/brand.json`) as
/// `--dart-define`s: the display name, the identifiers, the prefix of the environment variables the app reads, the accent and the
/// mark. Without a definition a value is the brand file's own; `test/brand_test.dart` checks they agree.
abstract final class Brand {
  /// What a person reads for the application: the window title, the tray's tooltip and menu.
  static const String name = String.fromEnvironment('BRAND_NAME', defaultValue: 'Canoa');

  /// The name where room is short.
  static const String shortName = String.fromEnvironment('BRAND_SHORT_NAME', defaultValue: 'Canoa');

  /// The reverse-DNS identifier.
  static const String id = String.fromEnvironment('BRAND_ID', defaultValue: 'dev.boriken.canoa');

  /// The prefix of the environment variables the app and its host read, without the underscore.
  static const String envPrefix = String.fromEnvironment('BRAND_ENV_PREFIX', defaultValue: 'CANOA');

  /// The accent the build's icons are drawn in, `#RRGGBB`.
  static const String accentHex = String.fromEnvironment('BRAND_ACCENT', defaultValue: '#3DAFA1');

  /// The mark's paths as the build read them from its SVG (a JSON object), or empty for Canoa's own.
  static const String markJson = String.fromEnvironment('BRAND_MARK');

  /// The accent as a colour.
  static Color get accent => Color(0xFF000000 | int.parse(accentHex.substring(1), radix: 16));

  /// The environment variable `suffix` of this brand: `CONFIG` is `CANOA_CONFIG` in Canoa's own build.
  static String env(String suffix) => '${envPrefix}_$suffix';

  /// The mark's path definitions, or null when the build gave none.
  static Map<String, Object?>? get mark {
    if (markJson.isEmpty) return null;
    final decoded = jsonDecode(markJson);
    return decoded is Map<String, Object?> ? decoded : null;
  }
}
