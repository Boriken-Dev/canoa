import 'dart:convert';
import 'dart:io';

import 'draft_store.dart';

/// `drafts/` beside canoa.cfg at [configPath]; in memory when there is no such place.
DraftBackend platformDraftBackend({String? configPath}) =>
    configPath == null ? MemoryDraftBackend() : FileDraftBackend('${File(configPath).parent.path}${Platform.pathSeparator}drafts');

/// A directory per draft, holding `draft.json` and its images, each written whole through a file beside it.
final class FileDraftBackend implements DraftBackend {
  FileDraftBackend(this.root);
  final String root;

  String _dir(String id) {
    if (!isDraftId(id)) throw ArgumentError.value(id, 'id', 'not a draft id');
    return '$root${Platform.pathSeparator}$id';
  }

  File _file(String id, String name) => File('${_dir(id)}${Platform.pathSeparator}$name');

  static Future<void> _replace(File file, List<int> bytes) async {
    await file.parent.create(recursive: true);
    final next = File('${file.path}.new');
    await next.writeAsBytes(bytes, flush: true);
    await next.rename(file.path);
  }

  @override
  Future<List<String>> ids() async {
    final dir = Directory(root);
    if (!await dir.exists()) return const [];
    return [
      await for (final entry in dir.list(followLinks: false))
        if (entry is Directory) entry.uri.pathSegments.where((segment) => segment.isNotEmpty).last,
    ];
  }

  @override
  Future<String?> read(String id) async {
    final file = _file(id, 'draft.json');
    return await file.exists() ? file.readAsString() : null;
  }

  @override
  Future<List<int>?> readBlob(String id, String blob) async {
    if (!isBlobName(blob)) return null;
    final file = _file(id, blob);
    return await file.exists() ? file.readAsBytes() : null;
  }

  @override
  Future<void> write(String id, String record) => _replace(_file(id, 'draft.json'), utf8.encode(record));

  @override
  Future<void> writeBlob(String id, String blob, List<int> bytes) async {
    if (!isBlobName(blob)) throw ArgumentError.value(blob, 'blob');
    final file = _file(id, blob);
    if (!await file.exists()) await _replace(file, bytes);
  }

  @override
  Future<List<String>> blobs(String id) async {
    final dir = Directory(_dir(id));
    if (!await dir.exists()) return const [];
    return [
      await for (final entry in dir.list(followLinks: false))
        if (entry is File && isBlobName(entry.uri.pathSegments.last)) entry.uri.pathSegments.last,
    ];
  }

  @override
  Future<void> removeBlob(String id, String blob) async {
    if (!isBlobName(blob)) return;
    final file = _file(id, blob);
    if (await file.exists()) await file.delete();
  }

  @override
  Future<void> remove(String id) async {
    final dir = Directory(_dir(id));
    if (await dir.exists()) await dir.delete(recursive: true);
  }
}
