import 'dart:async';

import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;
import 'package:flutter/foundation.dart';

import '../voice/reply_reader.dart';
import '../voice/reply_speaker.dart';

/// What a pop-out window asks of the window that holds the conversation, and what that window tells back. The window plays no
/// sound of its own: its voices and its player are the main window's, so a reply is read the same way wherever it is shown.
const String readAsks = 'canoa.read';
const String readTells = 'canoa.reading';

/// A view link that also carries the pop-out's read-aloud messages. The conversation's own messages go on through [messages]
/// as they came, so the chat protocol never sees one that is not its own; the others are [asks]: what the window asks of the main
/// window on one end, and on the other end what the main window tells the window.
final class ReadAloudLink implements ChatViewLink {
  ReadAloudLink(this._inner) {
    _listening = _inner.messages.listen(
      (message) {
        final type = message['t'];
        if (type == readAsks || type == readTells) {
          _read.add(message);
        } else {
          _chat.add(message);
        }
      },
      onDone: _end,
    );
  }

  final ChatViewLink _inner;
  late final StreamSubscription<Map<String, Object?>> _listening;
  final _chat = StreamController<Map<String, Object?>>();
  final _read = StreamController<Map<String, Object?>>.broadcast();

  @override
  Stream<Map<String, Object?>> get messages => _chat.stream;

  /// The read-aloud messages the other end sent.
  Stream<Map<String, Object?>> get asks => _read.stream;

  @override
  bool get isClosed => _inner.isClosed;

  @override
  void send(Map<String, Object?> message) => _inner.send(message);

  void _end() {
    unawaited(_chat.close());
    unawaited(_read.close());
  }

  @override
  Future<void> close() async {
    await _listening.cancel();
    await _inner.close();
    _end();
  }
}

/// The speaker of a pop-out window: it reads nothing itself but asks the main window to, and shows what that window says it is doing.
final class RemoteReplySpeaker extends ChangeNotifier implements ReplySpeaker {
  RemoteReplySpeaker(this._link) {
    _listening = _link.asks.listen(_told);
    // The main window may have said its state before this window was listening: it is asked for again.
    _link.send({'t': readAsks, 'op': 'hello'});
  }

  final ReadAloudLink _link;
  late final StreamSubscription<Map<String, Object?>> _listening;
  bool _available = false, _voice = false, _busy = false, _disposed = false;
  String? _reading, _error, _errorKey;

  void _told(Map<String, Object?> message) {
    if (message['t'] != readTells) return;
    _available = message['available'] == true;
    _voice = message['voice'] == true;
    _busy = message['busy'] == true;
    _reading = message['reading'] is String ? message['reading']! as String : null;
    final error = message['error'];
    if (error is String) {
      _error = error;
      _errorKey = message['error_key'] is String ? message['error_key']! as String : null;
    }
    notifyListeners();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    unawaited(_listening.cancel());
    super.dispose();
  }

  @override
  bool get available => _available;

  @override
  bool get voiceMode => _voice;

  @override
  bool get busy => _busy;

  @override
  String? get reading => _reading;

  @override
  bool isReading(String key) => _reading == key;

  @override
  String? get error => _error;

  @override
  String? get errorKey => _errorKey;

  @override
  void clearError() {
    _error = null;
    _errorKey = null;
  }

  @override
  Future<void> toggle(String key, String markdown) async => _link.send({'t': readAsks, 'op': 'toggle', 'key': key, 'text': markdown});

  @override
  void stop() => _link.send({'t': readAsks, 'op': 'stop'});

  @override
  void setVoiceMode(bool on) => _link.send({'t': readAsks, 'op': 'voice', 'on': on});

  @override
  void unlock() {}
}

/// The main window's side: it answers what a pop-out window asks by its own [ReplyReader], and tells the window what the reader is
/// doing whenever that changes. Voice mode asked for in a window follows that window's conversation.
final class ReadAloudBridge {
  ReadAloudBridge({required this.reader, required this.link, required this.session, required this.onVoiceMode});

  final ReplyReader reader;
  final ReadAloudLink link;

  /// The id of the conversation the window shows.
  final String session;

  /// Told when the window turns voice mode on or off, to keep the choice.
  final ValueChanged<bool> onVoiceMode;

  StreamSubscription<Map<String, Object?>>? _asks;
  bool _on = false;

  /// Starts answering the window.
  void start() {
    _on = true;
    _asks = link.asks.listen(_ask);
    reader.addListener(_tell);
  }

  void _ask(Map<String, Object?> message) {
    if (!_on || message['t'] != readAsks) return;
    switch (message['op']) {
      case 'hello':
        _tell();
      case 'toggle':
        final key = message['key'], text = message['text'];
        if (key is String && text is String) unawaited(reader.toggle(key, text));
      case 'stop':
        reader.stop();
      case 'voice':
        final on = message['on'] == true;
        reader.setVoiceMode(on, session: on ? session : null);
        onVoiceMode(on);
    }
  }

  void _tell() {
    if (!_on || link.isClosed) return;
    final error = reader.error;
    link.send({
      't': readTells,
      'available': reader.available,
      // Voice mode is this window's only while it follows this window's conversation.
      'voice': reader.voiceMode && reader.pinned == session,
      'busy': reader.busy,
      'reading': reader.reading,
      'error': ?error,
      'error_key': ?(error == null ? null : reader.errorKey),
    });
    // The window has the reason now; the reader keeps it only until it has been said.
    if (error != null) reader.clearError();
  }

  /// Stops answering; voice mode that followed this window's conversation goes back to following the main window's.
  void close() {
    if (!_on) return;
    _on = false;
    reader.removeListener(_tell);
    unawaited(_asks?.cancel());
    reader.release(session);
  }
}
