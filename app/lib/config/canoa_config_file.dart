import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'canoa_config.dart';
import 'canoa_config_none.dart'
    if (dart.library.io) 'canoa_config_io.dart' as platform;

/// The person's own canoa.cfg, the one file the app writes.
String? desktopConfigPath() => platform.desktopConfigPath();

/// The arguments that make `canoa` read the files the app reads, for the config at [path].
List<String> hostConfigArgs(String path) => platform.hostConfigArgs(path);

/// What `canoa check --config` takes to check [candidate] as the config at [path].
String hostCheckSpec(String path, String candidate) => platform.hostCheckSpec(path, candidate);

/// What [saveConfig] throws for a configuration that is the defaults standing in for a canoa.cfg that could not be read: writing
/// them would replace the person's file.
final class ConfigUnreadable implements Exception {
  const ConfigUnreadable(this.reason);

  /// Why the file could not be read.
  final String reason;

  /// What to tell the person.
  String get message => 'canoa.cfg could not be read ($reason), so Canoa is using its defaults and writes nothing to the file until '
      'it can be read. Fix the file and reopen Canoa.';

  @override
  String toString() => message;
}

/// Writes the person's own file, in its own format: what they changed or already had there, never a value from the installed
/// or the system's file. Throws [ConfigUnreadable] for the defaults of a file that could not be read.
Future<void> saveConfig(CanoaConfig config) async {
  if (config.unreadable case final reason?) throw ConfigUnreadable(reason);
  await platform.writeDesktopConfig(config.encode());
}

/// canoa.cfg as the app reads it. A file that cannot be read gives the defaults with [CanoaConfig.unreadable] saying why, which the
/// app shows and which no save overwrites the file with.
Future<CanoaConfig> loadConfig() async {
  try {
    final read = await platform.readDesktopLayers();
    if (read != null) return CanoaConfig.parse(read.ownText ?? '', name: read.ownName, below: read.below);
    if (kIsWeb) {
      final response = await http.get(Uri.base.resolve('canoa.cfg'));
      if (response.statusCode == 200) return CanoaConfig.parse(response.body);
      if (response.statusCode != 404) {
        throw FormatException('the server answered ${response.statusCode}');
      }
    }
  } on Object catch (error) {
    debugPrint('canoa.cfg: $error; using defaults');
    return CanoaConfig.unreadable(error is FormatException ? error.message : '$error');
  }
  return const CanoaConfig();
}
