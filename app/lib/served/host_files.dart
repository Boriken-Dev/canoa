import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import '../desktop/control_client.dart';
import '../desktop/host_control.dart';

/// The code of the host's refusal of a write made from a table the file no longer holds.
const hostChanged = -32004;

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

  /// Whether the host has entries it did not list: never, from [HostFiles], which asks for every page.
  final bool more;
}

/// What `fs/read` answered: the start of a file, for a preview.
final class HostFileRead {
  const HostFileRead({required this.path, required this.name, required this.size, required this.kind, required this.truncated, this.mime, this.bytes});

  final String path;
  final String name;

  /// The file's whole size on the host.
  final int size;

  /// `text`, `image` (one a page can draw: PNG, JPEG, GIF or WebP) or `binary`.
  final String kind;
  final String? mime;

  /// Whether [bytes] stops short of the file: a text preview is the page asked for, and an image is read to its end.
  final bool truncated;

  /// What was read; null for a binary file.
  final Uint8List? bytes;
}

/// What the Files surface needs of a host: the places to browse and the start of a file. [HostFiles] is the host's own channel; a test
/// gives another computer's disk.
abstract interface class WorkspaceFiles {
  Future<HostFolderListing> listFolders({String? path, bool hidden = false, bool files = false});

  /// The start of the file at [path], at most [maxBytes] of it (the host's default when null).
  Future<HostFileRead> readFile(String path, {int? maxBytes});
}

/// What the host can start for a folder, from `workspace/openers`.
final class HostOpeners {
  const HostOpeners({required this.os, required this.ids, this.hostname});

  /// `windows`, `macos` or `linux`.
  final String os;
  final String? hostname;

  /// `vscode` when VS Code is found there, `file_manager` and `reveal`.
  final List<String> ids;
}

/// What an Open in needs of a host: what it can start, and starting it.
abstract interface class HostOpening {
  Future<HostOpeners> workspaceOpeners();
  Future<void> workspaceOpen(String path, String opener);
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
final class HostFiles implements WorkspaceFiles, HostOpening {
  HostFiles(this._control);

  final HostControl _control;

  Future<Map<Object?, Object?>> _call(String method, [Map<String, Object?> params = const {}, Duration? wait]) async {
    final client = await _control.ready();
    final result = await client.call(method, params, null, wait);
    return result is Map<Object?, Object?> ? result : const {};
  }

  @override
  Future<HostFileRead> readFile(String path, {int? maxBytes}) async {
    final first = await _call('fs/read', {'path': path, 'max_bytes': ?maxBytes});
    final data = first['data'];
    final parts = <Uint8List>[if (data is String) base64Decode(data)];
    var last = first;
    // A cut image shows nothing, so an image is read to its end, a page after another; a text preview is the page asked for.
    while (first['kind'] == 'image' && last['next'] is num && parts.isNotEmpty) {
      last = await _call('fs/read', {'path': path, 'offset': (last['next'] as num).toInt()});
      final more = last['data'];
      if (more is! String) break;
      parts.add(base64Decode(more));
    }
    final Uint8List? bytes;
    if (parts.length > 1) {
      final joined = BytesBuilder(copy: false);
      parts.forEach(joined.add);
      bytes = joined.takeBytes();
    } else {
      bytes = parts.firstOrNull;
    }
    return HostFileRead(
      path: first['path'] as String? ?? path,
      name: first['name'] as String? ?? path.split(RegExp(r'[\\/]')).last,
      size: (first['size'] as num?)?.toInt() ?? 0,
      kind: first['kind'] as String? ?? 'binary',
      mime: first['mime'] as String?,
      truncated: last['truncated'] == true,
      bytes: bytes,
    );
  }

  @override
  Future<HostFolderListing> listFolders({String? path, bool hidden = false, bool files = false}) async {
    final entries = <HostFolderEntry>[];
    Map<Object?, Object?> answer;
    var offset = 0;
    // The host answers a page at a time, cut from the sorted whole; every page is asked for, so no entry is missing.
    while (true) {
      answer = await _call('fs/list', {'path': ?path, if (hidden) 'hidden': true, if (files) 'files': true, if (offset > 0) 'offset': offset});
      for (final each in (answer['entries'] as List<Object?>? ?? const [])) {
        if (each is Map<Object?, Object?> && each['name'] is String && each['path'] is String) {
          entries.add(HostFolderEntry(name: each['name']! as String, path: each['path']! as String, isFolder: each['dir'] == true));
        }
      }
      final next = answer['next'];
      if (next is! num || next.toInt() <= offset) break;
      offset = next.toInt();
    }
    return HostFolderListing(path: answer['path'] as String?, parent: answer['parent'] as String?, entries: entries, more: false);
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
  Future<bool> removeUpload(String path) async =>
      (await _call('upload/remove', {'id': path.split(RegExp(r'[\\/]')).last}))['removed'] == true;

  /// What `shell/list` answers: the shells the host found, its own choice and what each agent entry is told. Refused without the
  /// grant `terminal` or `store.read`.
  Future<Map<Object?, Object?>> shellList() => _call('shell/list');

  /// The table [section] of the person's canoa.cfg on the host, or null when it has none.
  Future<Map<String, Object?>?> configGet(String section) async => (await configRead(section)).value;

  /// [configGet] with the `digest` the host names `[host]` by, which [configWrite] sends back so that the host can tell the file
  /// is as this page read it.
  Future<({Map<String, Object?>? value, String? digest})> configRead(String section) async {
    final answer = await _call('config/get', {'section': section});
    final value = answer['value'];
    return (
      value: value is Map ? {for (final MapEntry(:key, :value) in value.entries) '$key': value} : null,
      digest: answer['digest'] as String?,
    );
  }

  /// Replaces the table [section] with [value], or removes it for null. True when the host reads that table only when it starts, so
  /// [restart] is what applies it.
  Future<bool> configSet(String section, Map<String, Object?>? value) async => (await configWrite(section, value)).restart;

  /// [configSet] that names the [digest] of the table it was made from: the host refuses with [hostChanged] when its table is
  /// another by now. Answers the table's new digest when the host names one.
  Future<({bool restart, String? digest})> configWrite(String section, Map<String, Object?>? value, {String? digest}) async {
    final answer = await _call('config/set', {'section': section, 'value': value, 'digest': ?digest});
    return (restart: answer['restart'] == true, digest: answer['digest'] as String?);
  }

  /// What `workspace/openers` says this host can start for a folder; refused without the grant `workspace.open`.
  @override
  Future<HostOpeners> workspaceOpeners() async {
    final answer = await _call('workspace/openers');
    return HostOpeners(
      os: answer['os'] as String? ?? '',
      hostname: answer['hostname'] as String?,
      ids: [
        for (final each in (answer['openers'] as List<Object?>? ?? const []))
          if (each is Map<Object?, Object?> && each['id'] is String) each['id']! as String,
      ],
    );
  }

  /// Starts [opener] (`vscode`, `file_manager` or `reveal`) for [path] on the host; the host's own words when it cannot.
  @override
  Future<void> workspaceOpen(String path, String opener) async {
    await _call('workspace/open', {'path': path, 'opener': opener});
  }

  /// What is wrong with [section] set to [value], in the host's words, or null when canoa.cfg would stay one the host accepts.
  /// Nothing is written.
  Future<String?> configCheck(String section, Map<String, Object?>? value) async {
    try {
      await _call('config/check', {'section': section, 'value': value});
      return null;
    } on ControlFailed catch (error) {
      // -32002 is the host refusing what it read; a grant the key lacks, or an older host, is not an answer about the file.
      if (error.code == -32002) return error.message;
      rethrow;
    }
  }

  /// Takes the extension [name] out of canoa.cfg and, when the Store installed it, its files out of the folder the host kept them in.
  /// Throws [ControlFailed] in the host's words. The host loads extensions only when it starts.
  Future<bool> extensionRemove(String name) async => (await _call('extension/remove', {'name': name}))['files_removed'] == true;

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
