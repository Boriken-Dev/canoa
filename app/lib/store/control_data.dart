/// What the Store reads from outside the page, asked of the host over its control channel: an MCP registry's list, whether a
/// runtime is on the host's PATH, and the document kept of what each catalog's entries provide. The page itself fetches nothing
/// from any origin, so a served page and the desktop app do the same work in the same place.
library;

import '../desktop/control_client.dart';
import '../desktop/host_control.dart';
import '../loadout/loadout_controller.dart' show FindProgram;
import '../loadout/mcp_registry.dart';
import 'contents_storage.dart';

Future<Map<Object?, Object?>> _ask(HostControl control, String method, Map<String, Object?> params, Duration wait) async {
  final client = await control.ready();
  final result = await client.call(method, params, null, wait);
  return result is Map<Object?, Object?> ? result : const {};
}

/// Reads a registry's list through the host's `registry/fetch`, which holds the limits (no redirect, 20 seconds, 4 MiB). While the
/// host is down, [direct] reads it here when this app can; a refusal of the host is a [RegistryRefusal] with its words.
RegistryFetch controlRegistryFetch(HostControl control, {RegistryFetch? direct, bool Function()? hostUp}) => (uri) async {
  if (direct != null && !(hostUp?.call() ?? true)) return direct(uri);
  try {
    final answer = await _ask(control, 'registry/fetch', {'url': uri.toString()}, const Duration(seconds: 40));
    final body = answer['body'];
    if (body is! String) throw const RegistryRefusal('The host gave no registry answer');
    return body;
  } on ControlFailed catch (error) {
    throw RegistryRefusal(error.message);
  }
};

/// Looks a program up on the host's PATH through `host/which`; null when it is not there or the host cannot say. While the host
/// is down, [direct] looks on this computer when this app can.
FindProgram controlFindProgram(HostControl control, {FindProgram? direct, bool Function()? hostUp}) => (program) async {
  if (direct != null && !(hostUp?.call() ?? true)) return direct(program);
  try {
    final answer = await _ask(control, 'host/which', {'names': [program]}, const Duration(seconds: 20));
    final found = (answer['found'] as Map<Object?, Object?>?)?[program];
    return found is String ? found : null;
  } on Object {
    return null;
  }
};

/// The documents kept by the host (`store/contents`, `store/contents_put`), one for each catalog. While the host is down [direct]
/// keeps them, when this app can; a document that cannot be had is none, since the cache is only a cache.
final class ControlContentsStorage implements ContentsStorage {
  ControlContentsStorage(this._control, {this.direct, this.hostUp});

  final HostControl _control;
  final ContentsStorage? direct;
  final bool Function()? hostUp;

  ContentsStorage? get _instead => (hostUp?.call() ?? true) ? null : direct;

  @override
  Future<String?> read(String label) async {
    if (_instead case final other?) return other.read(label);
    try {
      final document = (await _ask(_control, 'store/contents', {'label': label}, const Duration(seconds: 30)))['document'];
      return document is String ? document : null;
    } on Object {
      return null;
    }
  }

  @override
  Future<void> write(String label, String json) async {
    if (_instead case final other?) return other.write(label, json);
    await _ask(_control, 'store/contents_put', {'label': label, 'document': json}, const Duration(seconds: 30));
  }
}
