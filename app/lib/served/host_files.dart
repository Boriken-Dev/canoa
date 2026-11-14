import 'dart:async';
import 'dart:convert';

import '../desktop/control_client.dart';
import '../desktop/host_control.dart';

/// One place or entry `fs/list` names.
final class HostFolderEntry {
  const HostFolderEntry({required this.name, required this.path, required this.isFolder});

  final String name;
  final String path;
  final bool isFolder;
}

/// What `fs/list` answered: the folders in [path], or the places to start from when it was asked for none.
final class HostFolderListing {
  const HostFolderListing({required this.path, required this.parent, required this.entries, required this.more});

  /// The folder listed, resolved; null for the starting places.
  final String? path;

  /// The folder above it when that is still somewhere this key may browse.
  final String? parent;
  final List<HostFolderEntry> entries;

  /// Whether the host stopped listing at its limit.
  final bool more;
}

/// A file the host keeps for this key.
final class HostUpload {
  const HostUpload({required this.path, required this.name, required this.size});

  /// Where it is on the host, for an agent there to read.
  final String path;
  final String name;
  final int size;
}

/// The host's files as a page may use them over the control channel: folders to browse, canoa.cfg by section, uploads, and the
/// host's restart. Each is a grant the host gave this page's key; a refusal is the host's own words.
final class HostFiles {
  HostFiles(this._control);

  final HostControl _control;

  Future<Map<Object?, Object?>> _call(String method, [Map<String, Object?> params = const {}, Duration? wait]) async {
    final client = await _control.ready();
    final result = await client.call(method, params, null, wait);
    return result is Map<Object?, Object?> ? result : const {};
  }

  /// The folders in [path], or the places to start from when it is null. [hidden] adds the names that start with a dot; [files] the files.
  Future<HostFolderListing> listFolders({String? path, bool hidden = false, bool files = false}) async {
    final answer = await _call('fs/list', {'path': ?path, if (hidden) 'hidden': true, if (files) 'files': true});
    return HostFolderListing(
      path: answer['path'] as String?,
      parent: answer['parent'] as String?,
      entries: [
        for (final each in (answer['entries'] as List<Object?>? ?? const []))
          if (each is Map<Object?, Object?> && each['name'] is String && each['path'] is String)
            HostFolderEntry(name: each['name']! as String, path: each['path']! as String, isFolder: each['dir'] == true),
      ],
      more: answer['more'] == true,
    );
  }

  /// The most raw bytes one upload message carries: the host takes 512 KiB, and a page's transport no message over a mebibyte.
  static const uploadChunk = 384 * 1024;

  /// Sends [bytes] to be kept as [name] on the host, in chunks, which holds its own limits (`max_upload_bytes`), and says where it
  /// is. [progress] hears how many bytes the host has. A file the host refuses part-way is removed there, never half kept.
  Future<HostUpload> upload(String name, List<int> bytes, {void Function(int sent)? progress}) async {
    String? id;
    var sent = 0;
    Map<Object?, Object?> answer = const {};
    do {
      final end = sent + uploadChunk < bytes.length ? sent + uploadChunk : bytes.length;
      final last = end >= bytes.length;
      answer = await _call(
        'upload/put',
        {
          if (id == null) 'name': name else ...{'id': id, 'offset': sent},
          'data': base64Encode(bytes.sublist(sent, end)),
          if (!last) 'done': false,
        },
        const Duration(minutes: 2),
      ).onError<Object>((error, stack) async {
        if (id != null) await _call('upload/remove', {'id': id}).then((_) {}, onError: (Object _) {});
        throw error;
      });
      id = answer['id']! as String;
      sent = end;
      progress?.call(sent);
    } while (sent < bytes.length);
    return HostUpload(path: answer['path']! as String, name: answer['name']! as String, size: (answer['size']! as num).toInt());
  }

  /// Deletes a file [upload] kept; false when there was none.
  Future<bool> removeUpload(String path) async => (await _call('upload/remove', {'path': path}))['removed'] == true;

  /// The table [section] of the person's canoa.cfg on the host, or null when it has none.
  Future<Map<String, Object?>?> configGet(String section) async {
    final value = (await _call('config/get', {'section': section}))['value'];
    return value is Map ? {for (final MapEntry(:key, :value) in value.entries) '$key': value} : null;
  }

  /// Replaces the table [section] with [value], or removes it for null. True when the host reads that table only when it starts, so
  /// [restart] is what applies it.
  Future<bool> configSet(String section, Map<String, Object?>? value) async =>
      (await _call('config/set', {'section': section, 'value': value}))['restart'] == true;

  /// Ends the host so that its supervisor starts it again with canoa.cfg as it is now; this link ends with it.
  Future<void> restart() async {
    await _call('host/restart');
  }

  /// Waits until the host answers again after [restart], for at most [within].
  Future<void> awaitHost({Duration within = const Duration(seconds: 60)}) async {
    final until = DateTime.now().add(within);
    Object? last;
    while (DateTime.now().isBefore(until)) {
      try {
        await _call('fs/list', const {}, const Duration(seconds: 5));
        return;
      } on ControlFailed {
        // It answered, so it is up: a refusal of this probe is the host's own.
        return;
      } on Object catch (error) {
        last = error;
        await Future<void>.delayed(const Duration(milliseconds: 500));
      }
    }
    throw StateError('the host did not come back: $last');
  }
}
