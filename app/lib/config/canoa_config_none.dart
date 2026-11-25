import 'canoa_config.dart';

String? desktopConfigPath() => null;

/// A web build has no program to ask: its configuration is the host's, which the served page reads through the control channel.
Future<CanoaConfig?> loadDesktopConfig() async => null;

Future<void> saveDesktopConfig(CanoaConfig config) async =>
    throw UnsupportedError('a web build writes its configuration through the host that serves it');

List<String> hostConfigArgs(String path) => ['--config', path];

Future<String?> checkDesktopConfig(CanoaConfig config) async => null;
