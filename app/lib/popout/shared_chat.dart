import 'package:coqui_acp/coqui_acp.dart' show AcpElicitationAnswer;
import 'package:coqui_acp_chat/experimental.dart' show ChatSnapshot, ModelSharedChat, SharedChat;

import '../session_store.dart';

export '../session_store.dart' show ownerClient;

/// One of Canoa's conversations as a `ChatSessionShare` shares it: what a window popped out of it shows, and what it asks of it.
/// The conversation is the model's, so the snapshot is its own; what a window sends and answers goes through the store, whose
/// attachments, rules and card are the same ones the main window uses.
final class CanoaSharedChat implements SharedChat {
  CanoaSharedChat(this.store, this.canoa) : _model = ModelSharedChat(canoa.chat);

  final SessionStore store;
  final ChatSession canoa;
  final ModelSharedChat _model;

  @override
  ChatSnapshot snapshot() => _model.snapshot();

  @override
  void Function() listen(void Function() onChange) => _model.listen(onChange);

  @override
  void attachClient(String id, {String? name}) => _model.attachClient(id, name: name);

  @override
  void detachClient(String id) {
    // The window that holds the conversation is not a view's to detach, whatever a view calls itself.
    if (id != ownerClient) _model.detachClient(id);
  }

  @override
  Future<void> send(String text, {required String from}) => store.sendTextTo(canoa, text, attached: false, from: from);

  @override
  Future<void> cancel({required String from}) => store.cancelSession(canoa);

  @override
  bool settlePermission(int id, {String? optionId, required String by}) {
    // The store answers the oldest request, which is the one a card shows: an answer to any other is not made.
    final first = canoa.pendingPermissions.firstOrNull;
    if (first == null || first.id != id) return false;
    final option = first.options.where((o) => o.optionId == optionId).firstOrNull;
    if (option == null) {
      store.cancelPermission(canoa, by: by);
    } else {
      store.choosePermission(canoa, option, by: by);
    }
    return true;
  }

  @override
  bool settleQuestion(int id, AcpElicitationAnswer answer, {required String by}) {
    final first = canoa.pendingQuestions.firstOrNull;
    if (first == null || first.id != id) return false;
    store.answerQuestion(canoa, answer, by: by);
    return true;
  }
}
