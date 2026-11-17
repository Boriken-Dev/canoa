import 'dart:async';
import 'dart:convert';

import '../desktop/control_client.dart';
import '../desktop/host_control.dart';
import '../drafts/draft_store.dart';
import '../ledger/session_ledger.dart' show LedgerBackend;
import '../permissions/permission_rules.dart' show RuleLog;
import '../shell/layout_state.dart' show LayoutBackend;

/// The documents the host keeps for the person (`state/*`), where a page puts what a desktop keeps in files beside canoa.cfg:
/// drafts, the window layout and the permission rules with their log. They are the host's, so they follow the person to any browser
/// that reaches it.
final class HostState {
  HostState(this._control);

  final HostControl _control;

  /// The most raw bytes the host takes or answers in one request.
  static const chunk = 512 * 1024;

  /// Whether this key may use the documents: the host answers a list, not a refusal. False for a host without the grant or the
  /// method, and for one that cannot be reached, so the page keeps them in the browser instead.
  Future<bool> available() async {
    try {
      await (await _control.ready()).call('state/list', const {'prefix': 'probe'});
      return true;
    } on Object {
      return false;
    }
  }

  /// The document [name], or null when the host holds none.
  Future<List<int>?> get(String name) async {
    final client = await _control.ready();
    final bytes = <int>[];
    for (;;) {
      final Object? answer;
      try {
        answer = await client.call('state/get', {'name': name, if (bytes.isNotEmpty) 'offset': bytes.length});
      } on ControlFailed catch (error) {
        if (error.code == -32003 && bytes.isEmpty) return null;
        rethrow;
      }
      final map = answer! as Map<Object?, Object?>;
      bytes.addAll(base64Decode(map['data']! as String));
      if (map['done'] == true) return bytes;
    }
  }

  Future<String?> getText(String name) async {
    final bytes = await get(name);
    return bytes == null ? null : utf8.decode(bytes);
  }

  /// Replaces the document [name]; the host keeps the old one until the new is whole.
  Future<void> put(String name, List<int> bytes) async {
    final client = await _control.ready();
    var at = 0;
    do {
      final end = at + chunk < bytes.length ? at + chunk : bytes.length;
      await client.call('state/put', {
        'name': name,
        'data': base64Encode(bytes.sublist(at, end)),
        if (at > 0) 'offset': at,
        if (end < bytes.length) 'done': false,
      });
      at = end;
    } while (at < bytes.length);
  }

  Future<void> putText(String name, String text) => put(name, utf8.encode(text));

  Future<void> append(String name, String text) async {
    await (await _control.ready()).call('state/append', {'name': name, 'text': text});
  }

  /// The names of the documents under [prefix], whole.
  Future<List<String>> list([String? prefix]) async {
    final answer = await (await _control.ready()).call('state/list', {'prefix': ?prefix}) as Map<Object?, Object?>;
    return [for (final name in answer['names']! as List<Object?>) name! as String];
  }

  /// Removes a document, or every one under a folder's name.
  Future<void> remove(String name) async {
    await (await _control.ready()).call('state/remove', {'name': name});
  }
}

/// One document of the host's, as a [LedgerBackend]: the permission rules' store.
final class HostDocumentBackend implements LedgerBackend {
  HostDocumentBackend(this._state, this.name);

  final HostState _state;
  final String name;

  @override
  String get location => "the host's state ($name)";

  @override
  Future<String?> read() => _state.getText(name);

  @override
  Future<void> write(String document) => _state.putText(name, document);
}

/// The window layout, as one document of the host's.
final class HostLayoutBackend implements LayoutBackend {
  HostLayoutBackend(this._state, {this.name = 'layout.json'});

  final HostState _state;
  final String name;

  @override
  Future<String?> read() => _state.getText(name);

  @override
  Future<void> write(String json) => _state.putText(name, json);
}

/// The permission rules' log: a file on the host each line is appended to.
final class HostRuleLog implements RuleLog {
  HostRuleLog(this._state, {this.name = 'permissions-log.jsonl'});

  final HostState _state;
  final String name;

  @override
  String get location => "the host's state ($name)";

  @override
  Future<void> append(String line) => _state.append(name, '$line\n');
}

/// Drafts as a folder of documents on the host, `drafts/<id>/`: the record `draft.json` and each image by its name.
final class HostDraftBackend implements DraftBackend {
  HostDraftBackend(this._state);

  final HostState _state;

  static const _root = 'drafts';

  String _name(String id, String file) {
    if (!isDraftId(id)) throw ArgumentError.value(id, 'id', 'not a draft id');
    return '$_root/$id/$file';
  }

  @override
  Future<List<String>> ids() async => {
    for (final name in await _state.list(_root)) name.split('/')[1],
  }.toList();

  @override
  Future<String?> read(String id) => _state.getText(_name(id, 'draft.json'));

  @override
  Future<List<int>?> readBlob(String id, String blob) async => isBlobName(blob) ? _state.get(_name(id, blob)) : null;

  @override
  Future<void> write(String id, String record) => _state.putText(_name(id, 'draft.json'), record);

  @override
  Future<void> writeBlob(String id, String blob, List<int> bytes) async {
    if (!isBlobName(blob)) throw ArgumentError.value(blob, 'blob');
    // A blob is named for its bytes, so one already there is the same one.
    if (!(await blobs(id)).contains(blob)) await _state.put(_name(id, blob), bytes);
  }

  @override
  Future<List<String>> blobs(String id) async => [
    for (final name in await _state.list('$_root/${_checked(id)}'))
      if (isBlobName(name.split('/').last)) name.split('/').last,
  ];

  @override
  Future<void> removeBlob(String id, String blob) async {
    if (isBlobName(blob)) await _state.remove(_name(id, blob));
  }

  @override
  Future<void> remove(String id) => _state.remove('$_root/${_checked(id)}');

  static String _checked(String id) {
    if (!isDraftId(id)) throw ArgumentError.value(id, 'id', 'not a draft id');
    return id;
  }
}
