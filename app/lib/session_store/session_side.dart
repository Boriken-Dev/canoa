part of '../session_store.dart';

/// How much of the conversation a side chat's first message carries: the last exchange, each side cut to this many characters.
const int sideContextLimit = 1500;

extension SessionSide on SessionStore {
  /// A new session for questions about [parent], on the same source and in the same folder, kept out of every list, tab and
  /// the ledger, and never selected: [parent] stays what the person is looking at. Nothing is sent to [parent]'s agent, and it
  /// hears nothing of the side chat. [title] names it. The agent starts with the first message, as a draft's does.
  ChatSession startSide(ChatSession parent, String title) {
    final keep = _selectedId;
    final side = createDraft(
      source: parent.source,
      kind: parent.kind,
      cwd: parent.kind == SessionKind.chat ? null : parent.worktree?.repo ?? parent.cwd,
      overlays: parent.overlays,
      loadout: parent.loadout,
      title: title,
      keptSource: parent.keptSource,
      cacheLifetime: parent.cacheLifetime,
      account: parent.account,
      sideOf: parent.id,
    );
    _selectedId = keep;
    _notify();
    return side;
  }

  /// The text that opens a side chat on [parent] with [question]: where the question comes from and, as far as the conversation
  /// went, its last exchange, since a fresh session knows nothing of it (a harness that could fork the session would carry it).
  String sideQuestion(ChatSession parent, String question) {
    String? last(MessageRole role) {
      for (final message in parent.messages.reversed) {
        if (message.role == role && message.text.trim().isNotEmpty) return message.text.trim();
      }
      return null;
    }

    String cut(String text) => text.length <= sideContextLimit ? text : '${text.substring(0, sideContextLimit)}…';
    final asked = last(MessageRole.user), said = last(MessageRole.agent);
    final where = parent.kind != SessionKind.workspace || parent.cwd == null || parent.cwd!.isEmpty ? '' : ' in ${parent.cwd}';
    return [
      'This is a side question about another conversation, "${parent.title}"$where. Answer briefly, and change nothing unless asked.',
      if (asked != null) 'It was last asked:\n${cut(asked)}',
      if (said != null) 'And the agent last said:\n${cut(said)}',
      'The side question:\n${question.trim()}',
    ].join('\n\n');
  }

  /// Sends [question] to [side], a session [startSide] made for [parent]: the first one with the note of where it comes from,
  /// the ones after it as they are.
  Future<void> askSide(ChatSession parent, ChatSession side, String question) =>
      sendTextTo(side, side.isDraft ? sideQuestion(parent, question) : question);
}
