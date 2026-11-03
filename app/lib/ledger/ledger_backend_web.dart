import 'dart:async';
import 'dart:js_interop';

import 'session_ledger.dart';

/// This origin's IndexedDB: a web page, a PWA and the extension each keep their own.
LedgerBackend platformLedgerBackend({String? configPath}) => IndexedDbLedgerBackend();

@JS('indexedDB')
external _Factory? get _indexedDB;

extension type _Factory._(JSObject _) implements JSObject {
  external _Request open(String name, int version);
}

/// An `IDBRequest`, or the `IDBOpenDBRequest` that opening answers.
extension type _Request._(JSObject _) implements JSObject {
  external JSAny? get result;
  external _Failure? get error;
  external set onsuccess(JSFunction? handler);
  external set onerror(JSFunction? handler);
  external set onupgradeneeded(JSFunction? handler);
}

extension type _Failure._(JSObject _) implements JSObject {
  external String get message;
}

extension type _Database._(JSObject _) implements JSObject {
  external _Transaction transaction(String store, String mode);
  external JSObject createObjectStore(String name);
  external void close();
}

extension type _Transaction._(JSObject _) implements JSObject {
  external _Store objectStore(String name);
  external _Failure? get error;
  external set oncomplete(JSFunction? handler);
  external set onerror(JSFunction? handler);
  external set onabort(JSFunction? handler);
}

extension type _Store._(JSObject _) implements JSObject {
  external _Request get(JSString key);
  external _Request put(JSAny value, JSString key);
}

/// The ledger as one string under one key of an object store.
final class IndexedDbLedgerBackend implements LedgerBackend {
  IndexedDbLedgerBackend({this.database = 'canoa-sessions'});

  final String database;
  static const _store = 'ledger';
  static const _key = 'sessions';

  @override
  String get location => 'IndexedDB "$database" of this browser';

  static Future<JSAny?> _done(_Request request) {
    final done = Completer<JSAny?>();
    request.onsuccess = ((JSAny _) => done.complete(request.result)).toJS;
    request.onerror = ((JSAny _) => done.completeError(StateError('IndexedDB: ${request.error?.message}'))).toJS;
    return done.future;
  }

  Future<_Database> _open() async {
    final factory = _indexedDB;
    if (factory == null) throw UnsupportedError('IndexedDB is unavailable here');
    final request = factory.open(database, 1);
    request.onupgradeneeded = ((JSAny _) {
      (request.result! as _Database).createObjectStore(_store);
    }).toJS;
    return (await _done(request))! as _Database;
  }

  @override
  Future<String?> read() async {
    final db = await _open();
    try {
      final value = await _done(db.transaction(_store, 'readonly').objectStore(_store).get(_key.toJS));
      return value.isA<JSString>() ? (value! as JSString).toDart : null;
    } finally {
      db.close();
    }
  }

  @override
  Future<void> write(String document) async {
    final db = await _open();
    try {
      final transaction = db.transaction(_store, 'readwrite');
      final committed = Completer<void>();
      void failed(JSAny _) {
        if (!committed.isCompleted) committed.completeError(StateError('IndexedDB: ${transaction.error?.message}'));
      }
      transaction
        ..oncomplete = ((JSAny _) => committed.complete()).toJS
        ..onerror = failed.toJS
        ..onabort = failed.toJS;
      transaction.objectStore(_store).put(document.toJS, _key.toJS);
      await committed.future;
    } finally {
      db.close();
    }
  }
}
