import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;
import 'package:flutter/foundation.dart' show VoidCallback;

import '../desktop/control_client.dart' show ControlFailed, hostWords;
import '../served/host_files.dart';

// A file's viewer in a window that is another page: the page that holds the host connection answers what the window asks of the
// files, one call and one answer at a time over their link, so the window needs no connection or key of its own.

/// Answers the calls a window makes of [files], over [link]. Returns what stops it.
VoidCallback serveFiles(WorkspaceFiles files, ChatViewLink link) {
  final heard = link.messages.listen((message) {
    if (message['t'] == 'call') unawaited(_answer(files, link, message));
  });
  return () => unawaited(heard.cancel());
}

Future<void> _answer(WorkspaceFiles files, ChatViewLink link, Map<String, Object?> call) async {
  final id = call['id'];
  final args = call['args'] is Map ? (call['args'] as Map).cast<String, Object?>() : const <String, Object?>{};
  try {
    link.send({'t': 'result', 'id': id, 'value': await _run(files, '${call['m']}', args)});
  } on Object catch (error) {
    link.send({'t': 'error', 'id': id, 'code': error is ControlFailed ? error.code : -32603, 'message': hostWords(error)});
  }
}

Future<Object?> _run(WorkspaceFiles files, String method, Map<String, Object?> args) async {
  switch (method) {
    case 'list':
      final listing = await files.listFolders(path: args['path'] as String?, hidden: args['hidden'] == true, files: args['files'] == true);
      return {
        'path': listing.path,
        'parent': listing.parent,
        'more': listing.more,
        'entries': [
          for (final entry in listing.entries) {'name': entry.name, 'path': entry.path, 'folder': entry.isFolder},
        ],
      };
    case 'read':
      return _readOut(await files.readFile(args['path'] as String, maxBytes: (args['max'] as num?)?.toInt()));
    case 'whole':
      return _readOut(await _writer(files).readWhole(args['path'] as String));
    case 'write':
      return _writer(files).writeFile(args['path'] as String, args['contents'] as String);
    case 'instructions':
      return files is WorkspaceInstructionFiles
          ? [
              for (final each in await (files as WorkspaceInstructionFiles).instructionFiles(args['workspace'] as String))
                {'path': each.path, 'name': each.name, 'scope': each.scope, 'size': each.size, 'agents': each.agents},
            ]
          : const [];
  }
  throw ControlFailed(-32003, 'no such call: $method');
}

WorkspaceFileWriter _writer(WorkspaceFiles files) =>
    files is WorkspaceFileWriter ? files as WorkspaceFileWriter : throw const ControlFailed(-32001, 'this host does not let files be written from here');

Map<String, Object?> _readOut(HostFileRead read) => {
  'path': read.path,
  'name': read.name,
  'size': read.size,
  'kind': read.kind,
  'mime': read.mime,
  'truncated': read.truncated,
  'bytes': read.bytes == null ? null : base64Encode(read.bytes!),
};

HostFileRead _readIn(Object? raw) {
  final map = raw as Map;
  return HostFileRead(
    path: map['path'] as String,
    name: map['name'] as String,
    size: (map['size'] as num).toInt(),
    kind: map['kind'] as String,
    mime: map['mime'] as String?,
    truncated: map['truncated'] == true,
    bytes: map['bytes'] is String ? Uint8List.fromList(base64Decode(map['bytes'] as String)) : null,
  );
}

/// The files the page that opened this window holds, asked over [link].
class LinkedFiles implements WorkspaceFiles, WorkspaceInstructionFiles {
  LinkedFiles(this.link) {
    _heard = link.messages.listen((message) {
      switch (message['t']) {
        case 'result':
          _waiting.remove(message['id'])?.complete(message['value']);
        case 'error':
          _waiting.remove(message['id'])?.completeError(ControlFailed((message['code'] as num?)?.toInt() ?? -32603, '${message['message']}'));
        case 'ended':
          _failAll('${message['reason'] ?? 'The page that held these files was closed'}');
      }
    });
  }

  /// The files of [link], which can be written only when [writable]: the page that holds them says so.
  factory LinkedFiles.of(ChatViewLink link, {required bool writable}) => writable ? LinkedWritableFiles(link) : LinkedFiles(link);

  final ChatViewLink link;
  late final StreamSubscription<Map<String, Object?>> _heard;
  final _waiting = <Object?, Completer<Object?>>{};
  var _calls = 0;

  Future<Object?> _call(String method, Map<String, Object?> args) {
    if (link.isClosed) return Future.error(const ControlFailed(-32603, 'The page that held these files was closed'));
    final id = ++_calls;
    final answer = _waiting[id] = Completer<Object?>();
    link.send({'t': 'call', 'id': id, 'm': method, 'args': args});
    return answer.future;
  }

  void _failAll(String why) {
    for (final waiting in _waiting.values.toList()) {
      waiting.completeError(ControlFailed(-32603, why));
    }
    _waiting.clear();
  }

  /// Stops listening; what is still asked fails.
  Future<void> close() async {
    _failAll('These files are not served any more');
    await _heard.cancel();
  }

  @override
  Future<HostFolderListing> listFolders({String? path, bool hidden = false, bool files = false}) async {
    final map = await _call('list', {'path': path, 'hidden': hidden, 'files': files}) as Map;
    return HostFolderListing(
      path: map['path'] as String?,
      parent: map['parent'] as String?,
      more: map['more'] == true,
      entries: [
        for (final each in map['entries'] as List)
          HostFolderEntry(name: (each as Map)['name'] as String, path: each['path'] as String, isFolder: each['folder'] == true),
      ],
    );
  }

  @override
  Future<HostFileRead> readFile(String path, {int? maxBytes}) async => _readIn(await _call('read', {'path': path, 'max': maxBytes}));

  @override
  Future<List<HostInstructionFile>> instructionFiles(String workspace) async => [
    for (final each in await _call('instructions', {'workspace': workspace}) as List)
      HostInstructionFile(
        path: (each as Map)['path'] as String,
        name: each['name'] as String,
        scope: each['scope'] as String,
        size: (each['size'] as num).toInt(),
        agents: [for (final agent in each['agents'] as List) '$agent'],
      ),
  ];
}

/// [LinkedFiles] for a page that lets files be written.
final class LinkedWritableFiles extends LinkedFiles implements WorkspaceFileWriter {
  LinkedWritableFiles(super.link);

  @override
  Future<HostFileRead> readWhole(String path) async => _readIn(await _call('whole', {'path': path}));

  @override
  Future<int> writeFile(String path, String contents) async => ((await _call('write', {'path': path, 'contents': contents})) as num).toInt();
}
