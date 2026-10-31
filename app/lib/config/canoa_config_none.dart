String? desktopConfigPath() => null;
Future<String?> readDesktopConfig() async => null;
Future<void> writeDesktopConfig(String source) async =>
    throw UnsupportedError('Web settings are read-only deployment config');
