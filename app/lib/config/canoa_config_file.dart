import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'canoa_config.dart';
import 'canoa_config_none.dart'
    if (dart.library.io) 'canoa_config_io.dart' as platform;

String? desktopConfigPath() => platform.desktopConfigPath();
Future<void> saveConfig(CanoaConfig config) =>
    platform.writeDesktopConfig(config.encode());

Future<CanoaConfig> loadConfig() async {
  try {
    final local = await platform.readDesktopConfig();
    if (local != null) return CanoaConfig.parse(local);
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
