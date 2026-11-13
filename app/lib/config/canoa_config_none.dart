import 'config_layers.dart';

String? desktopConfigPath() => null;
typedef ReadLayers = ({String? ownText, String ownName, List<ConfigLayer> below});
Future<ReadLayers?> readDesktopLayers() async => null;
Future<void> writeDesktopConfig(String source) async =>
    throw UnsupportedError('Web settings are read-only deployment config');
List<String> hostConfigArgs(String path) => ['--config', path];
String hostCheckSpec(String path, String candidate) => candidate;
