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

/// Writes the person's own file, in its own format: what they changed or already had there, never a value from the installed
/// or the system's file.
Future<void> saveConfig(CanoaConfig config) =>
    platform.writeDesktopConfig(config.encode());

Future<CanoaConfig> loadConfig() async {
  try {
    final read = await platform.readDesktopLayers();
    if (read != null) return CanoaConfig.parse(read.ownText ?? '', name: read.ownName, below: read.below);
    if (kIsWeb) {
      final response = await http.get(Uri.base.resolve('canoa.cfg'));
      if (response.statusCode == 200) return CanoaConfig.parse(response.body);
      if (response.statusCode != 404) {
        throw FormatException('Unable to read canoa.cfg');
      }
    }
  } catch (error) {
    debugPrint('canoa.cfg: $error; using defaults');
  }
  return const CanoaConfig();
}
