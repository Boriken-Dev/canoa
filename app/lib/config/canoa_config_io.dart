import 'dart:io';

import 'config_layers.dart';

/// Where canoa.cfg is read from and written, as the host finds it: the installed copy beside the program, the system's file
/// and the person's own, or the files `CANOA_CONFIG` lists. Throws [FormatException] for an empty entry or a listed file
/// that is missing.
ConfigFiles configFiles() => ConfigFiles.resolve(
  env: Platform.environment,
  windows: Platform.isWindows,
  programDir: File(Platform.resolvedExecutable).parent.path,
  exists: (path) => File(path).existsSync(),
);

/// The person's own file: where the app writes, and beside which it keeps its other files; null when no place can be named.
String? desktopConfigPath() {
  try {
    return configFiles().own;
  } on FormatException {
    return null;
  }
}

/// What reading canoa.cfg found: the person's own file (its text, and the name that fixes its format) and the layers
/// below it, lowest first. Null when no file exists.
typedef ReadLayers = ({String? ownText, String ownName, List<ConfigLayer> below});

Future<ReadLayers?> readDesktopLayers() async {
  final files = configFiles();
  _adoptLegacyName(files.own);
  final below = <ConfigLayer>[];
  String? ownText;
  var any = false;
  for (final file in files.files) {
    final entry = File(file.path);
    if (!entry.existsSync()) continue;
    any = true;
    final text = await entry.readAsString();
    if (file.path == files.own) {
      ownText = text;
      continue;
    }
    try {
      final read = decodeConfig(text, name: file.path);
      below.add((origin: file.origin, data: read.data));
    } on FormatException catch (error) {
      throw FormatException('${file.path}: ${error.message}');
    }
  }
  if (!any) return null;
  return (ownText: ownText, ownName: files.own, below: below);
}

Future<void> writeDesktopConfig(String source) async {
  final path = desktopConfigPath();
  if (path == null) throw StateError('canoa.cfg location unavailable');
  _adoptLegacyName(path);
  final file = File(path);
  await file.parent.create(recursive: true);
  await file.writeAsString(source, flush: true);
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

/// What `canoa check --config` takes to check [candidate] as the content of the config at [path].
String hostCheckSpec(String path, String candidate) {
  try {
    final files = configFiles();
    if (files.own == path) return files.checkSpec(candidate);
  } on FormatException {
    // As above.
  }
  return candidate;
}

// Only the default place is adopted: a file CANOA_CONFIG names is the person's own choice of name.
void _adoptLegacyName(String path) {
  final named = Platform.environment['CANOA_CONFIG'];
  if (named == null || named.isEmpty) adoptLegacyConfigName(path, log: stderr.writeln);
}

/// Gives a `Canoa.cfg` beside [path] the name [path], once, and says so through [log]. A file already at [path] is
/// never replaced, and where names ignore case the two are one file, so nothing is renamed. Returns whether it renamed.
bool adoptLegacyConfigName(String path, {required void Function(String line) log}) {
  if (Platform.isWindows) return false;
  // A link counts as an entry, so a dangling one is not written through.
  if (FileSystemEntity.typeSync(path, followLinks: false) != FileSystemEntityType.notFound) return false;
  final old = File('${File(path).parent.path}${Platform.pathSeparator}Canoa.cfg');
  if (FileSystemEntity.typeSync(old.path, followLinks: false) != FileSystemEntityType.file) return false;
  try {
    old.renameSync(path);
  } on FileSystemException catch (error) {
    log('Canoa.cfg was not renamed to canoa.cfg: ${error.message}');
    return false;
  }
  log('renamed ${old.path} to canoa.cfg');
  return true;
}
