import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';

import '../chat/attachments.dart';

/// Where drafts are kept between runs: each one a record and the bytes of its images, by name.
abstract interface class DraftBackend {
  Future<List<String>> ids();
  Future<String?> read(String id);
  Future<List<int>?> readBlob(String id, String blob);
  Future<void> write(String id, String record);
  Future<void> writeBlob(String id, String blob, List<int> bytes);

  /// The blobs [id] holds.
  Future<List<String>> blobs(String id);
  Future<void> removeBlob(String id, String blob);

  /// Removes the draft and everything it holds.
  Future<void> remove(String id);
}

/// Kept only while the app runs.
final class MemoryDraftBackend implements DraftBackend {
  final records = <String, String>{};
  final blobbed = <String, Map<String, List<int>>>{};

  @override
  Future<List<String>> ids() async => records.keys.toList();
  @override
  Future<String?> read(String id) async => records[id];
  @override
  Future<List<int>?> readBlob(String id, String blob) async => blobbed[id]?[blob];
  @override
  Future<void> write(String id, String record) async => records[id] = record;
  @override
  Future<void> writeBlob(String id, String blob, List<int> bytes) async => (blobbed[id] ??= {})[blob] = bytes;
  @override
  Future<List<String>> blobs(String id) async => blobbed[id]?.keys.toList() ?? const [];
  @override
  Future<void> removeBlob(String id, String blob) async => blobbed[id]?.remove(blob);
  @override
  Future<void> remove(String id) async {
    records.remove(id);
    blobbed.remove(id);
  }
}

/// A draft's id: 16 hexadecimal digits, which name its directory and nothing else.
bool isDraftId(String id) => RegExp(r'^[0-9a-f]{16}$').hasMatch(id);

/// A blob's name: the SHA-256 of its bytes.
bool isBlobName(String name) => RegExp(r'^[0-9a-f]{64}\.bin$').hasMatch(name);

/// A new draft id.
String newDraftId([Random? random]) {
  final draw = random ?? Random.secure();
  return List.generate(16, (_) => draw.nextInt(16).toRadixString(16)).join();
}

/// A draft read back: its record, and its attachments with their bytes.
final class KeptDraft {
  const KeptDraft(this.id, this.record, this.attachments);
  final String id;
  final Map<String, Object?> record;
  final List<Attachment> attachments;
}

/// Keeps each session not yet started, with what is written in it and what goes with it, so it is there after a restart.
/// A draft closed, or started, is removed whole.
class DraftKeeper {
  DraftKeeper(this.backend);

  final DraftBackend backend;

  /// How long changes gather before a draft is written.
  static const saveDelay = Duration(milliseconds: 400);

  final Map<String, Timer> _timers = {};
  final Map<String, (Map<String, Object?>, List<Attachment>)> _pending = {};
  final Set<String> _removed = {};
  Future<void> _writing = Future.value();

  /// Every draft kept, oldest first by when it was made; one that cannot be read is left where it is.
  Future<List<KeptDraft>> load() async {
    final out = <KeptDraft>[];
    final List<String> ids;
    try {
      ids = await backend.ids();
    } on Object catch (error) {
      debugPrint('Canoa drafts: could not list them: $error');
      return out;
    }
    for (final id in ids.where(isDraftId)) {
      try {
        final text = await backend.read(id);
        if (text == null) continue;
        final record = jsonDecode(text);
        if (record is! Map<String, Object?> || record['version'] != 1) continue;
        final attachments = <Attachment>[];
        for (final json in (record['attachments'] as List? ?? const []).take(maxAttachments)) {
          final blobName = json is Map && json['blob'] is String && isBlobName(json['blob'] as String) ? json['blob'] as String : null;
          final bytes = blobName == null ? null : await backend.readBlob(id, blobName);
          final attachment = Attachment.fromJson(json, blob: (_) => bytes);
          if (attachment != null) attachments.add(attachment);
        }
        out.add(KeptDraft(id, record, attachments));
      } on Object catch (error) {
        debugPrint('Canoa drafts: could not read $id: $error');
      }
    }
    out.sort((a, b) => '${a.record['created']}'.compareTo('${b.record['created']}'));
    return out;
  }

  /// Writes [record] and [attachments] as the draft [id], after [saveDelay].
  void save(String id, Map<String, Object?> record, List<Attachment> attachments) {
    if (!isDraftId(id) || _removed.contains(id)) return;
    _pending[id] = (record, List.of(attachments));
    _timers.remove(id)?.cancel();
    _timers[id] = Timer(saveDelay, () => unawaited(_write(id)));
  }

  Future<void> _write(String id) {
    _timers.remove(id)?.cancel();
    final pending = _pending.remove(id);
    if (pending == null) return _writing;
    final (record, attachments) = pending;
    return _writing = _writing.then((_) async {
      if (_removed.contains(id)) return;
      final blobs = <String>{};
      final kept = <Map<String, Object?>>[];
      for (final attachment in attachments) {
        String? blob;
        if (attachment.bytes case final bytes?) {
          blob = '${sha256.convert(bytes)}.bin';
          if (blobs.add(blob)) await backend.writeBlob(id, blob, bytes);
        }
        kept.add(attachment.toJson(blob: blob));
      }
      await backend.write(id, jsonEncode({...record, 'version': 1, 'attachments': kept}));
      for (final old in await backend.blobs(id)) {
        if (!blobs.contains(old)) await backend.removeBlob(id, old);
      }
    }).catchError((Object error) {
      debugPrint('Canoa drafts: could not write $id: $error');
    });
  }

  /// Removes the draft [id] and everything it holds, now and from any write still to come.
  Future<void> remove(String id) {
    if (!isDraftId(id)) return _writing;
    _removed.add(id);
    _timers.remove(id)?.cancel();
    _pending.remove(id);
    return _writing = _writing.then((_) => backend.remove(id)).catchError((Object error) {
      debugPrint('Canoa drafts: could not remove $id: $error');
    });
  }

  /// Writes every draft still waiting.
  Future<void> flush() async {
    for (final id in [..._pending.keys]) {
      unawaited(_write(id));
    }
    await _writing;
  }
}
