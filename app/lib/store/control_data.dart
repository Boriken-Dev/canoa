/// What the Store reads from outside the page, asked of the host over its control channel: whether a runtime is on the host's PATH
/// and the document kept of what each catalog's entries provide. The page itself fetches nothing from any origin, so a served page and the desktop app do the same work in the same place.
library;

import '../desktop/host_control.dart';
import '../loadout/loadout_controller.dart' show FindProgram;
import 'contents_reader.dart';
import 'contents_storage.dart';
import 'overlay_contents.dart' show OverlayContents;

Future<Map<Object?, Object?>> _ask(HostControl control, String method, Map<String, Object?> params, Duration wait) async {
  final client = await control.ready();
  final result = await client.call(method, params, null, wait);
  return result is Map<Object?, Object?> ? result : const {};
}

/// Looks a program up on the host's PATH through `host/which`; null when it is not there or the host cannot say.
FindProgram controlFindProgram(HostControl control) => (program) async {
  try {
    final answer = await _ask(control, 'host/which', {'names': [program]}, const Duration(seconds: 20));
    final found = (answer['found'] as Map<Object?, Object?>?)?[program];
    return found is String ? found : null;
  } on Object {
    return null;
  }
};

/// The documents kept by the host (`store/contents`, `store/contents_put`), one for each catalog; a document that cannot be had is
/// none, since the cache is only a cache.
final class ControlContentsStorage implements ContentsStorage {
  ControlContentsStorage(this._control);

  final HostControl _control;

  @override
  Future<String?> read(String label) async {
    try {
      final document = (await _ask(_control, 'store/contents', {'label': label}, const Duration(seconds: 30)))['document'];
      return document is String ? document : null;
    } on Object {
      return null;
    }
  }

  @override
  Future<void> write(String label, String json) async {
    await _ask(_control, 'store/contents_put', {'label': label, 'document': json}, const Duration(seconds: 30));
  }
}

/// Reads catalog entries through the host's `store/read_ahead`: one call for a batch, which fetches and reads each entry there.
final class ControlContentsReader implements ContentsReader {
  ControlContentsReader(this._control, {required this.os});

  final HostControl _control;

  /// The system the host runs on, which decides what an overlay's hooks and requirements are.
  final String os;

  @override
  Future<Map<String, ContentsAnswer>> read(List<ContentsRequest> requests) async {
    final answer = await _ask(_control, 'store/read_ahead', {'os': os, 'entries': [for (final request in requests) request.toJson()]}, const Duration(minutes: 8));
    final entries = answer['entries'];
    final read = <String, ContentsAnswer>{};
    if (entries is Map<Object?, Object?>) {
      for (final MapEntry(:key, :value) in entries.entries) {
        if (value is! Map<Object?, Object?>) continue;
        final contents = value['contents'];
        read['$key'] = contents is Map<Object?, Object?>
            ? ContentsAnswer.read(OverlayContents.fromJson(contents))
            : ContentsAnswer.failed(value['problem'] is String ? value['problem']! as String : 'The host could not read it');
      }
    }
    return read;
  }
}
