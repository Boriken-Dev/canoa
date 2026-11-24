import 'dart:async';

import 'package:coqui_acp/coqui_acp.dart' show AcpElicitationAnswer, AcpPermissionOption;
import 'package:coqui_acp_chat/coqui_acp_chat.dart' as model;
import 'package:coqui_acp_chat/experimental.dart' show ChatSessionView;

import '../session_store.dart';

/// What a window shows of a conversation and what it may ask of it: the conversation's model to draw, and the person's actions on it
/// by the window's own client name. A window of the app's own engine holds the main window's conversation itself ([LiveConversation]);
/// a window that is another page holds a copy kept by a link ([LinkedConversation]).
abstract interface class PopoutConversation {
  /// The conversation to draw. Never changed by the window: it asks through the methods below.
  model.ChatSession get session;

  /// The name this window's answers and messages go by.
  String get clientId;

  /// Whether the window may still ask: false once the conversation was closed or the window left it.
  bool get isOpen;

  /// Completes when the window is no longer attached, with why when it is known.
  Future<String?> get done;

  /// Sends [text] to the conversation, queued while a turn runs; throws when it could not.
  Future<void> send(String text);

  /// Ends the running turn.
  void cancel();

  /// Answers [question] with one of its options, or cancelled when [option] is null.
  void choosePermission(model.PermissionQuestion question, AcpPermissionOption? option);

  void answerQuestion(model.AgentQuestion question, AcpElicitationAnswer answer);

  /// Leaves the conversation, which goes on as it was.
  Future<void> detach();
}

/// The main window's conversation itself, for a window of the same engine: its transcript, its turn and what the agent waits on
/// are the objects the main window draws, so there is nothing to copy and nothing to keep in step. What the window asks goes through
/// the store as the main window's own actions do, under this window's client name.
final class LiveConversation implements PopoutConversation {
  LiveConversation(this.store, this.canoa, {required this.clientId, String name = 'the pop-out window'}) {
    canoa.chat.attachClient(clientId, name: name);
  }

  final SessionStore store;
  final ChatSession canoa;
  final _done = Completer<String?>();
  bool _open = true;

  @override
  final String clientId;

  @override
  model.ChatSession get session => canoa.chat;

  @override
  bool get isOpen => _open;

  @override
  Future<String?> get done => _done.future;

  void _require() {
    if (!_open) throw StateError('This view is not attached any more');
  }

  @override
  Future<void> send(String text) {
    _require();
    return store.sendTextTo(canoa, text, attached: false, from: clientId);
  }

  @override
  void cancel() {
    _require();
    unawaited(store.cancelSession(canoa));
  }

  // The store answers the oldest request, which is the one a card shows.
  @override
  void choosePermission(model.PermissionQuestion question, AcpPermissionOption? option) {
    _require();
    if (canoa.pendingPermissions.firstOrNull?.id != question.id) return;
    if (option == null) {
      store.cancelPermission(canoa, by: clientId);
    } else {
      store.choosePermission(canoa, option, by: clientId);
    }
  }

  @override
  void answerQuestion(model.AgentQuestion question, AcpElicitationAnswer answer) {
    _require();
    if (canoa.pendingQuestions.firstOrNull?.id != question.id) return;
    store.answerQuestion(canoa, answer, by: clientId);
  }

  @override
  Future<void> detach() async => end();

  /// The window is gone or the conversation was closed.
  void end([String? reason]) {
    if (!_open) return;
    _open = false;
    canoa.chat.detachClient(clientId);
    if (!_done.isCompleted) _done.complete(reason);
  }
}

/// A conversation kept in a window that is another page, by a view attached over a link.
final class LinkedConversation implements PopoutConversation {
  LinkedConversation(this.view);

  final ChatSessionView view;

  @override
  model.ChatSession get session => view.session;

  @override
  String get clientId => view.clientId;

  @override
  bool get isOpen => view.isOpen;

  @override
  Future<String?> get done => view.done;

  @override
  Future<void> send(String text) => view.send(text);

  @override
  void cancel() => view.cancel();

  @override
  void choosePermission(model.PermissionQuestion question, AcpPermissionOption? option) => view.choosePermission(question, option);

  @override
  void answerQuestion(model.AgentQuestion question, AcpElicitationAnswer answer) => view.answerQuestion(question, answer);

  @override
  Future<void> detach() => view.detach();
}
