import 'dart:async';

import 'package:http/http.dart' as http;

import '../acp/served_host.dart';
import '../config/connection.dart';
import '../desktop/control_client.dart' show ControlFailed;
import '../desktop/host_control.dart';
import '../served/host_files.dart';
import 'host_folders.dart';

/// Where a new session can start, as the new-session page offers it: this app's own host, or a saved connection.
final class HostChoice {
  const HostChoice({required this.name, required this.label, this.kind, this.detail});

  /// The saved connection's name; null for this app's own host (on the web, the host that served the page).
  final String? name;

  /// What the person is shown: `Local`, the serving host's name, or the connection's name.
  final String label;

  /// The saved connection's kind; null for the app's own host.
  final ConnectionKind? kind;

  /// A second line, as the Hosts section words the kind.
  final String? detail;

  bool get isLocal => name == null;

  /// Whether the host has a folder listing to browse: every Canoa host, never a bare ACP endpoint, which takes a typed path.
  bool get browses => kind != ConnectionKind.acp;
}

/// Whether [path] is written the way a host running [os] (`windows`, `linux`, `macos`, as Rust names it) writes paths. A host that
/// did not say what it runs is given no judgment.
bool pathFitsSystem(String? os, String path) {
  if (os == null || os.isEmpty) return true;
  final windows = RegExp(r'^([A-Za-z]:([\\/]|$)|\\\\)').hasMatch(path);
  return os == 'windows' ? windows : path.startsWith('/') && !windows;
}

/// Why [path] cannot be a workspace on the host called [host], which runs [os], in that host's own words, or null when it can: it
/// is written for another system, is no folder, or cannot be read.
Future<String?> workspaceProblem({
  required String host,
  required String? os,
  required HostFolders folders,
  required WorkspaceFiles files,
  required String path,
}) async {
  if (!pathFitsSystem(os, path)) {
    return '$path is not a path on $host${os == null ? '' : ', which runs $os'}.';
  }
  try {
    final said = (await folders.info([path]))[path];
    if (said == null || !said.exists) return '$path does not exist on $host.';
    if (!said.folder) return '$path is not a folder on $host.';
    await files.listFolders(path: path);
    return null;
  } on ControlFailed catch (failed) {
    return '$host would not open $path: ${failed.message}';
  } on Object catch (error) {
    return 'Could not ask $host about $path: $error';
  }
}

/// What the app holds of one saved Canoa host while a new session is aimed at it: the list it served, and its control channel, over
/// which folders are browsed and checked and a chat's scratch folder and a worktree are made.
final class RemoteHost {
  RemoteHost(this.connection, this.served, this.control)
    : files = HostFiles(control),
      folders = HostFolders(control);

  final Connection connection;
  final ServedHost served;
  final HostControl control;
  final HostFiles files;
  final HostFolders folders;

  /// The operating system the host says it runs.
  String? get os => served.os;

  /// The agent entries it serves.
  List<ServedEntry> get agents => [for (final entry in served.entries) if (entry.kind == 'agent') entry];

  /// Whether the host listens for a control channel at all.
  bool get hasControl => served.control != null;

  /// Why [path] cannot be a workspace there, in the host's words, or null when it can: it is written for another system, is no
  /// folder, or cannot be read.
  Future<String?> problemWith(String path) =>
      workspaceProblem(host: connection.name, os: os, folders: folders, files: files, path: path);

  void dispose() {
    folders.dispose();
    unawaited(control.close());
  }
}

/// Reads the list a Canoa host serves at [connection]'s address, or throws a [StateError] saying why not.
Future<ServedHost> fetchServedHost(Connection connection, {http.Client? client, Duration timeout = const Duration(seconds: 8)}) async {
  final ask = client ?? http.Client();
  try {
    final reply = await ask.get(Uri.parse(connection.address).resolve('canoa.json')).timeout(timeout);
    if (reply.statusCode != 200) throw StateError('${connection.name} answered ${reply.statusCode} for its list.');
    final host = ServedHost.parse(reply.body);
    if (host == null) throw StateError('${connection.address} does not serve a Canoa host\'s list.');
    if (host.peer != connection.peer) {
      throw StateError('${connection.name} says its id is ${host.peer}, not the one saved.');
    }
    return host;
  } on TimeoutException {
    throw StateError('${connection.name} did not answer in time.');
  } finally {
    if (client == null) ask.close();
  }
}
