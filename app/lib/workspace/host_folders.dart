import 'dart:async';

import 'package:flutter/foundation.dart';

import '../acp/local_agent.dart' show InsideFile;
import '../desktop/control_client.dart';
import '../desktop/host_control.dart';

/// What the host said of one folder.
typedef FolderInfo = ({bool exists, bool folder, bool repository, String? root});

/// What a chat's scratch folder is made from: the session's id, the source it starts from, and the time it was made.
typedef ScratchRequest = ({String session, String source, String handler, DateTime created});

/// A time as the host reads `now`: the person's local time, `2026-11-17T10:30:05`.
String localIso(DateTime time) {
  final local = time.toLocal();
  String two(int n) => n.toString().padLeft(2, '0');
  return '${local.year.toString().padLeft(4, '0')}-${two(local.month)}-${two(local.day)}T${two(local.hour)}:${two(local.minute)}:${two(local.second)}';
}

/// The folders of the computer the host runs on, asked of the host over its control channel (`fs/info`, `fs/mentions`,
/// `fs/resolve`, `fs/scratch*`): the same calls from a served page and from the desktop app, which hold no file system of their own
/// for this. The synchronous questions (does this recent folder still exist, is it in a repository) answer from what the host last
/// said and ask again in the background, so a widget that built with a guess is built again with the answer.
class HostFolders extends ChangeNotifier {
  HostFolders(this._control, {Duration keep = const Duration(seconds: 10)}) : _keep = keep;

  final HostControl _control;
  final Duration _keep;
  final Map<String, ({FolderInfo info, DateTime at})> _known = {};
  final Set<String> _wanted = {};
  final Set<String> _asking = {};
  bool _scheduled = false;
  bool _disposed = false;

  Future<Map<Object?, Object?>> _call(String method, Map<String, Object?> params, [Duration? wait]) async {
    final client = await _control.ready();
    final result = await client.call(method, params, null, wait ?? const Duration(seconds: 30));
    return result is Map<Object?, Object?> ? result : const {};
  }

  /// Whether the host has [folder], as last said; true until it says otherwise, since a folder the person chose is there.
  bool exists(String folder) => _lookup(folder)?.exists ?? true;

  /// Whether the host says [folder] is in a git repository, as last said; false until it says so.
  bool inRepository(String folder) => _lookup(folder)?.repository ?? false;

  FolderInfo? _lookup(String folder) {
    final held = _known[folder];
    if (held == null || DateTime.now().difference(held.at) > _keep) _want(folder);
    return held?.info;
  }

  void _want(String folder) {
    if (_asking.contains(folder) || !_wanted.add(folder) || _scheduled) return;
    _scheduled = true;
    scheduleMicrotask(_flush);
  }

  Future<void> _flush() async {
    _scheduled = false;
    final batch = _wanted.take(64).toList();
    _wanted.removeAll(batch);
    if (batch.isEmpty) return;
    _asking.addAll(batch);
    try {
      var changed = false;
      for (final entry in (await info(batch)).entries) {
        final before = _known[entry.key]?.info;
        _known[entry.key] = (info: entry.value, at: DateTime.now());
        changed |= (before ?? (exists: true, folder: true, repository: false, root: null)).exists != entry.value.exists ||
            (before?.repository ?? false) != entry.value.repository;
      }
      if (changed && !_disposed) notifyListeners();
    } on Object {
      // Not said: the guesses stand, and the next build asks again.
    } finally {
      _asking.removeAll(batch);
      if (_wanted.isNotEmpty && !_scheduled && !_disposed) {
        _scheduled = true;
        scheduleMicrotask(_flush);
      }
    }
  }

  /// The host's word on each of [folders], keyed by the path as asked.
  Future<Map<String, FolderInfo>> info(List<String> folders) async {
    final said = await _call('fs/info', {'paths': folders});
    final out = <String, FolderInfo>{};
    for (final each in (said['folders'] as List<Object?>? ?? const [])) {
      if (each is! Map<Object?, Object?> || each['path'] is! String) continue;
      out[each['path']! as String] = (
        exists: each['exists'] == true,
        folder: each['folder'] == true,
        repository: each['repository'] == true,
        root: each['root'] as String?,
      );
    }
    return out;
  }

  /// Whether [folder] is in a git repository, asked now.
  Future<bool> isRepository(String folder) async {
    try {
      final said = (await info([folder]))[folder];
      if (said != null) {
        _known[folder] = (info: said, at: DateTime.now());
        return said.repository;
      }
    } on Object {
      // The host could not say.
    }
    return false;
  }

  /// What an `@` offers inside [folder]; nothing when the host cannot say.
  Future<List<String>> mentions(String folder) async {
    try {
      final said = await _call('fs/mentions', {'path': folder});
      return [for (final entry in (said['entries'] as List<Object?>? ?? const [])) if (entry is String) entry];
    } on ControlFailed {
      return const [];
    }
  }

  /// The file or folder [relative] names inside [folder], or null when it names anything else.
  Future<InsideFile?> resolve(String folder, String relative) async {
    try {
      final said = await _call('fs/resolve', {'path': folder, 'name': relative});
      final path = said['path'];
      if (path is! String) return null;
      return (path: path, folder: said['folder'] == true, size: (said['size'] as num?)?.toInt() ?? 0);
    } on ControlFailed {
      return null;
    }
  }

  Map<String, Object?> _scratchParams(ScratchRequest request) =>
      {'session': request.session, 'source': request.source, 'handler': request.handler, 'now': localIso(request.created)};

  /// Makes the folder the person's workspace template names for a chat; the host's words when it cannot.
  Future<({String path, bool made})> makeScratch(ScratchRequest request) async {
    final said = await _call('fs/scratch', _scratchParams(request));
    return (path: said['path']! as String, made: said['made'] == true);
  }

  /// Takes that folder back when it is empty; says nothing when it is not or cannot.
  Future<void> removeScratch(ScratchRequest request) async {
    try {
      await _call('fs/scratch_remove', _scratchParams(request));
    } on Object {
      // Left where it is.
    }
  }

  /// The folder [template] would make, for Settings; null when the host cannot work it out.
  Future<String?> preview(String template, {String source = '', String handler = ''}) async {
    try {
      final said = await _call('fs/scratch_preview', {'template': template, 'source': source, 'handler': handler, 'now': localIso(DateTime.now())});
      return said['path'] as String?;
    } on Object {
      return null;
    }
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
