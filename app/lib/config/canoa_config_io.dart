import 'dart:convert';
import 'dart:io';

import '../brand.dart';
import '../desktop/control_client.dart' show ControlFailed;
import '../desktop/host_tool.dart' show HostTool, findHostTool;
import 'canoa_config.dart';
import 'config_layers.dart';

/// Where canoa.cfg is, as the host finds it: the installed copy beside the program, the system's file and the person's own, or the
/// files `CANOA_CONFIG` lists. Only the names are worked out here, to start the host with the same files and to keep the app's other
/// files beside the person's own; the file's contents are asked of the host (see [loadDesktopConfig] and [saveDesktopConfig]). Throws
/// [FormatException] for an empty entry or a listed file that is missing.
ConfigFiles configFiles() => ConfigFiles.resolve(
  env: Platform.environment,
  windows: Platform.isWindows,
  programDir: File(Platform.resolvedExecutable).parent.path,
  exists: (path) => File(path).existsSync(),
);

/// The person's own file: where the host writes, and beside which the app keeps its other files; null when no place can be named.
String? desktopConfigPath() {
  try {
    return configFiles().own;
  } on FormatException {
    return null;
  }
}

/// The arguments that make `canoa` read the files the app reads, for the config at [path]: those of [configFiles] when it is
/// the app's own, else that one file.
List<String> hostConfigArgs(String path) {
  try {
    final files = configFiles();
    if (files.own == path) return files.hostArgs;
  } on FormatException {
    // The host is given the path, and says what is wrong.
  }
  return ['--config', path];
}

/// One request of the host's control channel, answered by `canoa control` with no host serving: the same methods, the same grants as the
/// owner, over the same configuration store. The request goes in a file of its own, since a table can be longer than a command line.
/// A failure of the program is a [StateError]; a refusal is a [ControlFailed] in the host's words.
Future<Object?> _control(String method, Map<String, Object?> params, {HostTool? tool, ConfigFiles? files}) async {
  final host = tool ?? findHostTool();
  if (host == null) throw StateError('this app carries no ${Brand.program} to read and write its configuration with');
  final directory = await Directory.systemTemp.createTemp('${Brand.program}-control-');
  try {
    final request = File('${directory.path}${Platform.pathSeparator}request.json');
    await request.writeAsString(jsonEncode({'jsonrpc': '2.0', 'id': 1, 'method': method, 'params': params}));
    final output = await host.run(['control', ...(files ?? configFiles()).hostArgs, '@${request.path}']);
    final line = const LineSplitter().convert(output.stdout).where((each) => each.trim().isNotEmpty).lastOrNull;
    if (output.exitCode != 0 || line == null) {
      throw StateError('${Brand.program} control failed (${output.exitCode}): ${output.stderr.trim()}');
    }
    final answer = jsonDecode(line);
    if (answer is Map && answer['error'] is Map) {
      final error = answer['error'] as Map;
      throw ControlFailed((error['code'] as num?)?.toInt() ?? -32603, '${error['message']}');
    }
    return answer is Map ? answer['result'] : null;
  } finally {
    try {
      await directory.delete(recursive: true);
    } on FileSystemException {
      // A temporary folder that stays is the system's to clear.
    }
  }
}

ConfigOrigin _originOf(String id) => switch (id) {
  'installed' => ConfigOrigin.installed,
  'system' => ConfigOrigin.system,
  _ => ConfigOrigin.listed,
};

/// What [saveDesktopConfig] last knew the person's own layer to hold, by table, so that the next write sends only a table that changed.
Map<String, dynamic>? _held;

/// canoa.cfg's layers as the host's store holds them (through `canoa control`, with no host serving): the person's own layer and the
/// ones below it, lowest first. A file that is not there yet is an empty layer. Throws where the app has no `canoa` to ask, which
/// [loadConfig] says as the reason the configuration could not be read.
Future<CanoaConfig?> loadDesktopConfig({HostTool? tool, ConfigFiles? files}) async {
  final answer = await _control('config/layers', const {}, tool: tool, files: files);
  final layers = answer is Map && answer['layers'] is List ? answer['layers'] as List : const [];
  final below = <ConfigLayer>[];
  var own = <String, dynamic>{};
  for (final layer in layers.whereType<Map>()) {
    final data = {for (final MapEntry(:key, :value) in ((layer['document'] as Map?) ?? const {}).entries) '$key': value};
    if (layer['writable'] == true) {
      own = data;
    } else {
      below.add((origin: _originOf('${layer['id']}'), data: data));
    }
  }
  _held = _canonical(own) as Map<String, dynamic>;
  return CanoaConfig.fromLayers((format: ConfigFormat.toml, marked: false, data: own), below);
}

/// The `config/change` ops that make the person's own layer what [config] writes: one for each table whose content differs from what the
/// layer held, or that is gone, and none for a table that is the same.
List<Map<String, Object?>> _opsFor(CanoaConfig config, Map<String, dynamic> before, Map<String, dynamic> now) => [
  for (final table in {...before.keys, ...now.keys})
    if (jsonEncode(before[table]) != jsonEncode(now[table])) {'path': [table], 'value': now[table]},
];

/// What the host says is wrong with the configuration [config] would make, in its own words, or null when its reader would take it. The
/// host reads the result as it would after the change; nothing is written.
Future<String?> checkDesktopConfig(CanoaConfig config, {HostTool? tool, ConfigFiles? files}) async {
  final before = _held ?? _canonical(config.source.own) as Map<String, dynamic>;
  final ops = _opsFor(config, before, _canonical(config.documentToWrite) as Map<String, dynamic>);
  if (ops.isEmpty) return null;
  try {
    await _control('config/change', {'ops': ops, 'check': true}, tool: tool, files: files);
    return null;
  } on ControlFailed catch (error) {
    return error.message;
  }
}

/// Writes what [config] changed to the person's own layer, as one `config/change` the host makes through its store: a table whose
/// content differs from what the layer held, or is gone, and no other, so the rest of the file stays as the person wrote it.
Future<void> saveDesktopConfig(CanoaConfig config, {HostTool? tool, ConfigFiles? files}) async {
  final before = _held ?? _canonical(config.source.own) as Map<String, dynamic>;
  final now = _canonical(config.documentToWrite) as Map<String, dynamic>;
  final ops = _opsFor(config, before, now);
  if (ops.isNotEmpty) await _control('config/change', {'ops': ops}, tool: tool, files: files);
  _held = now;
}

/// [value] with every table's keys in order, so two spellings of one table compare equal.
Object? _canonical(Object? value) => switch (value) {
  final Map<dynamic, dynamic> map => {for (final key in (map.keys.map((each) => '$each').toList()..sort())) key: _canonical(map[key])},
  final List<dynamic> list => [for (final each in list) _canonical(each)],
  _ => value,
};
