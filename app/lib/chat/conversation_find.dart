import 'dart:async';

import 'package:coqui_acp_chat/coqui_acp_chat.dart' show ChatMessage, ToolCall, ToolDiff, ToolPatch, ToolText;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../session_store.dart';

/// What the find bar of one conversation holds: whether it is open, the query, the match the person is on and where each matching
/// entry is drawn, so the bar can scroll to it. The conversation listens to it and draws [bar] and [decorate].
final class ConversationFinder extends ChangeNotifier {
  final TextEditingController controller = TextEditingController();
  final FocusNode focus = FocusNode();
  final Map<int, GlobalKey> _entryKeys = {};
  bool _open = false;
  int _index = 0;

  bool get isOpen => _open;

  /// Whether a query is typed, which is when the transcript shows every entry so a match is not hidden.
  bool get searching => _open && controller.text.trim().isNotEmpty;

  /// The match the person is on, clamped to [matches].
  int active(List<int> matches) => matches.isEmpty ? 0 : _index.clamp(0, matches.length - 1);

  /// The message indices of [session] that hold the query; none while the bar is closed.
  List<int> matches(ChatSession session) => _open ? findMatchingMessageIndices(session.messages, controller.text) : const [];

  void show() {
    _open = true;
    _index = 0;
    notifyListeners();
    focus.requestFocus();
  }

  void close() {
    if (!_open && controller.text.isEmpty) return;
    _open = false;
    controller.clear();
    _index = 0;
    _entryKeys.clear();
    notifyListeners();
  }

  void _query() {
    _index = 0;
    notifyListeners();
  }

  void _step(int delta, List<int> matches) {
    if (matches.isEmpty) return;
    _index = ((_index + delta) % matches.length + matches.length) % matches.length;
    notifyListeners();
    final key = _entryKeys[matches[_index]];
    if (key == null) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final target = key.currentContext;
      if (target != null) unawaited(Scrollable.ensureVisible(target, alignment: 0.3, duration: const Duration(milliseconds: 160)));
    });
  }

  /// The bar above the transcript of [session], or nothing while it is closed.
  Widget bar(ChatSession session) {
    if (!_open) return const SizedBox.shrink();
    final matches = this.matches(session);
    return ConversationFindBar(
      controller: controller,
      focusNode: focus,
      matchCount: matches.length,
      activeIndex: active(matches),
      onChanged: (_) => _query(),
      onPrevious: () => _step(-1, matches),
      onNext: () => _step(1, matches),
      onClose: close,
    );
  }

  /// [base] with the matching entries of [session] outlined, the one the person is on in the accent.
  ChatRenderers decorate(ChatRenderers base, ChatSession session) {
    final matches = this.matches(session);
    if (matches.isEmpty) return base;
    final activeMessage = matches[active(matches)];
    final byAtAndText = <(int, String), int>{
      for (final idx in matches) (session.messages[idx].at.microsecondsSinceEpoch, session.messages[idx].text): idx,
    };
    return ChatRenderers(
      entry: [
        (context, entry) {
          final message = byAtAndText[(entry.at.microsecondsSinceEpoch, entry.text)];
          if (message == null) return null;
          final key = _entryKeys.putIfAbsent(message, GlobalKey.new);
          final isActive = message == activeMessage;
          final look = context.chatLook;
          return Container(
            key: key,
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(look.radius),
              border: Border.all(color: isActive ? look.accent : look.warning, width: isActive ? 2 : 1),
            ),
            padding: const EdgeInsets.all(2),
            child: ChatScope(renderers: base, terminals: session.terminals, child: TranscriptEntry(entry: entry)),
          );
        },
        ...base.entry,
      ],
      tool: base.tool,
      block: base.block,
      burst: base.burst,
      work: base.work,
      plan: base.plan,
      wide: base.wide,
    );
  }

  @override
  void dispose() {
    controller.dispose();
    focus.dispose();
    super.dispose();
  }
}

/// Returns searchable plain text contributed by a single [message].
String messageSearchText(ChatMessage message) {
  final buf = StringBuffer();
  if (message.text.isNotEmpty) buf.writeln(message.text);
  if (message.tool case final ToolCall tool?) {
    buf.writeln(tool.title);
    for (final place in tool.locations) {
      buf.writeln(place);
    }
    for (final path in tool.changedPaths) {
      buf.writeln(path);
    }
    for (final block in tool.blocks) {
      switch (block) {
        case ToolText(:final text):
          buf.writeln(text);
        case ToolDiff(:final path, :final oldText, :final newText):
          buf.writeln(path);
          buf.writeln(oldText);
          buf.writeln(newText);
        case ToolPatch(:final text):
          buf.writeln(text);
        default:
          break;
      }
    }
  }
  if (message.compaction case final c? when c.summary.isNotEmpty) buf.writeln(c.summary);
  return buf.toString();
}

/// Indices in [messages] whose text contains [query] (case-insensitive).
List<int> findMatchingMessageIndices(List<ChatMessage> messages, String query) {
  final q = query.trim().toLowerCase();
  if (q.isEmpty) return const [];
  return [
    for (var i = 0; i < messages.length; i++)
      if (messageSearchText(messages[i]).toLowerCase().contains(q)) i,
  ];
}

/// Inline search bar above the transcript for `Ctrl+F` / Find in conversation.
class ConversationFindBar extends StatelessWidget {
  const ConversationFindBar({
    super.key,
    required this.controller,
    required this.focusNode,
    required this.matchCount,
    required this.activeIndex,
    required this.onChanged,
    required this.onPrevious,
    required this.onNext,
    required this.onClose,
  });

  final TextEditingController controller;
  final FocusNode focusNode;
  final int matchCount;
  final int activeIndex;
  final ValueChanged<String> onChanged;
  final VoidCallback onPrevious;
  final VoidCallback onNext;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final hasQuery = controller.text.trim().isNotEmpty;
    final countText = !hasQuery
        ? ''
        : matchCount == 0
        ? 'No matches'
        : '${activeIndex + 1} of $matchCount';
    return Material(
      key: const Key('conversation-find-bar'),
      color: look.surface,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
        decoration: BoxDecoration(border: Border(bottom: BorderSide(color: look.border))),
        child: Row(children: [
          const Icon(Icons.search, size: 18),
          const SizedBox(width: 8),
          Expanded(
            child: CallbackShortcuts(
              bindings: {const SingleActivator(LogicalKeyboardKey.escape): onClose},
              child: TextField(
                key: const Key('conversation-find-input'),
                controller: controller,
                focusNode: focusNode,
                autofocus: true,
                decoration: const InputDecoration(
                  hintText: 'Find in conversation',
                  isDense: true,
                  border: InputBorder.none,
                ),
                textInputAction: TextInputAction.search,
                onChanged: onChanged,
                onSubmitted: (_) => onNext(),
              ),
            ),
          ),
          if (countText.isNotEmpty)
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8),
              child: Text(
                countText,
                key: const Key('conversation-find-count'),
                style: TextStyle(fontSize: 12, color: matchCount == 0 ? look.danger : look.muted),
              ),
            ),
          IconButton(
            key: const Key('conversation-find-prev'),
            tooltip: 'Previous match',
            icon: const Icon(Icons.keyboard_arrow_up, size: 18),
            visualDensity: VisualDensity.compact,
            onPressed: matchCount > 0 ? onPrevious : null,
          ),
          IconButton(
            key: const Key('conversation-find-next'),
            tooltip: 'Next match',
            icon: const Icon(Icons.keyboard_arrow_down, size: 18),
            visualDensity: VisualDensity.compact,
            onPressed: matchCount > 0 ? onNext : null,
          ),
          IconButton(
            key: const Key('conversation-find-close'),
            tooltip: 'Close find',
            icon: const Icon(Icons.close, size: 18),
            visualDensity: VisualDensity.compact,
            onPressed: onClose,
          ),
        ]),
      ),
    );
  }
}
