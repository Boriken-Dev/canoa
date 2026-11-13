import 'dart:async';

import 'package:coqui_acp/coqui_acp.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart' show planStatusWord;
import 'package:coqui_acp_chat/experimental.dart'
    show
        ChatSnapshot,
        SharedChat,
        SharedOutcome,
        SharedPermission,
        SharedQuestion,
        answerToJson,
        chatAtMetaKey,
        chatNamesMetaKey,
        decisionToJson;

import '../chat/agent_events.dart';
import '../session_store.dart';

/// The name the window that holds a conversation goes by among its views.
const String ownerClient = 'main';

/// How many settled requests are kept to tell the views that still show them waiting how they ended.
const int _settledKept = 32;

/// One of Canoa's conversations as a [ChatSessionShare] shares it: what a window popped out of it shows, and what it asks of it.
/// Canoa keeps the conversation in its own session state, so the snapshot is built from that, in the shape `coqui_acp_chat`'s
/// views read; what a view answers goes through the store, whose rules and card are the same ones the main window uses.
final class CanoaSharedChat implements SharedChat {
  CanoaSharedChat(this.store, this.session, {DateTime Function()? clock}) : _clock = clock ?? DateTime.now {
    _attach(ownerClient, 'Canoa');
  }

  final SessionStore store;
  final ChatSession session;
  final DateTime Function() _clock;

  final _clients = <String, String?>{};
  final _permissionIds = Expando<int>();
  final _questionIds = Expando<int>();
  final _answeredBy = <int, String>{};
  final _questionAnsweredBy = <int, String>{};
  final _settledPermissions = <SharedOutcome>[];
  final _settledQuestions = <SharedOutcome>[];
  var _next = 1;

  void _attach(String id, String? name) => _clients[id] = name;

  int _idOfPermission(PermissionQuestion question) {
    var id = _permissionIds[question];
    if (id == null) {
      id = _permissionIds[question] = _next++;
      final number = id;
      unawaited(question.answer.then((decision) {
        _settled(_settledPermissions, SharedOutcome(
          id: number,
          how: decision is AcpCancel ? 'cancelled' : 'answered',
          by: decision is AcpCancel ? null : _answeredBy.remove(number) ?? ownerClient,
          answer: decisionToJson(decision),
        ));
      }));
    }
    return id;
  }

  int _idOfQuestion(AgentQuestion question) {
    var id = _questionIds[question];
    if (id == null) {
      id = _questionIds[question] = _next++;
      final number = id;
      unawaited(question.answer.then((answer) {
        _settled(_settledQuestions, SharedOutcome(
          id: number,
          how: answer is AcpElicitCancel ? 'cancelled' : 'answered',
          by: answer is AcpElicitCancel ? null : _questionAnsweredBy.remove(number) ?? ownerClient,
          answer: answerToJson(answer),
        ));
      }));
    }
    return id;
  }

  void _settled(List<SharedOutcome> into, SharedOutcome outcome) {
    into.add(outcome);
    if (into.length > _settledKept) into.removeAt(0);
  }

  @override
  ChatSnapshot snapshot() {
    final permissions = [
      for (final question in session.pendingPermissions)
        SharedPermission(
          id: _idOfPermission(question),
          title: question.title,
          kind: question.kind,
          detail: question.detail,
          options: question.options,
          forPerson: question.forPerson,
          bypassPassed: question.bypassPassed,
          toolCallId: question.toolCallId,
        ),
    ];
    final questions = [
      for (final question in session.pendingQuestions)
        SharedQuestion(id: _idOfQuestion(question), request: question.request),
    ];
    return ChatSnapshot(
      sessionId: session.remoteSessionId ?? session.id,
      agent: session.agent,
      cwd: session.cwd,
      status: switch (session.status) {
        SessionStatus.working => 'working',
        SessionStatus.connected => 'connected',
        _ => 'disconnected',
      },
      replay: [for (final update in _replay()) update],
      queue: [for (final queued in session.queue) (text: queued.text, from: null)],
      queueHeld: session.queueHeld,
      clients: [for (final entry in _clients.entries) (id: entry.key, name: entry.value)],
      permissions: permissions,
      questions: questions,
      settledPermissions: List.unmodifiable(_settledPermissions),
      settledQuestions: List.unmodifiable(_settledQuestions),
    );
  }

  /// What the conversation said, as the updates that make it again. A turn runs from a person's message to the entry before the
  /// next one, and stays open while the session is working.
  List<Map<String, Object?>> _replay() {
    final out = <Map<String, Object?>>[];
    void add(Map<String, Object?> raw, DateTime at) => out.add({
      ...raw,
      '_meta': {
        ...?(raw['_meta'] as Map?)?.cast<String, Object?>(),
        chatAtMetaKey: at.toUtc().toIso8601String(),
      },
    });

    DateTime? opened, last;
    void open(DateTime at) {
      if (opened != null) return;
      opened = at;
      add({'sessionUpdate': 'state_update', 'state': 'running'}, at);
    }

    void close() {
      if (opened == null) return;
      add({'sessionUpdate': 'state_update', 'state': 'idle', 'stopReason': 'end_turn'}, last ?? opened!);
      opened = null;
    }

    for (final message in session.messages) {
      switch (message.role) {
        case MessageRole.notice:
          continue;
        case MessageRole.user:
          close();
          open(message.at);
          add({
            'sessionUpdate': 'user_message',
            'messageId': ?message.messageId,
            'content': {'type': 'text', 'text': message.text},
          }, message.at);
        case MessageRole.agent || MessageRole.thought:
          open(message.at);
          add({
            'sessionUpdate': message.role == MessageRole.thought ? 'agent_thought' : 'agent_message',
            'messageId': ?message.messageId,
            'content': {'type': 'text', 'text': message.text},
          }, message.at);
        case MessageRole.tool:
          open(message.at);
          final tool = message.tool!;
          add(tool.toUpdate(), message.at);
          for (final block in tool.blocks) {
            final terminal = block is ToolTerminal ? session.terminals[block.terminalId] : null;
            if (terminal != null) add(terminal.toUpdate((block as ToolTerminal).terminalId), message.at);
          }
        case MessageRole.compaction:
          open(message.at);
          add(message.compaction!.toUpdate(), message.at);
      }
      last = message.at;
    }
    if (session.status != SessionStatus.working) close();

    final now = _clock();
    if (session.plan.isNotEmpty) {
      add({
        'sessionUpdate': 'plan',
        'entries': [
          for (final entry in session.plan) {'content': entry.content, 'status': planStatusWord(entry.status)},
        ],
      }, now);
    }
    if (session.commands.isNotEmpty) {
      add({
        'sessionUpdate': 'available_commands_update',
        'availableCommands': [
          for (final command in session.commands) {'name': command.name, 'description': command.description},
        ],
      }, now);
    }
    if (session.options.isNotEmpty) {
      final mode = session.options.where((option) => option.isMode).firstOrNull;
      add({
        'sessionUpdate': 'config_option_update',
        'configOptions': [
          for (final option in session.options)
            if (!option.isMode) _optionJson(option),
        ],
        if (mode != null)
          'modes': {
            'currentModeId': mode.current,
            'availableModes': [
              for (final choice in mode.choices) {'id': choice.value, 'name': choice.label},
            ],
          },
      }, now);
    }
    if (!session.usage.isEmpty) add(session.usage.toUpdate(), now);
    add({
      'sessionUpdate': 'session_info_update',
      'title': session.title,
      if (!session.agentNames) '_meta': {chatNamesMetaKey: false},
    }, now);
    return out;
  }

  static Map<String, Object?> _optionJson(ConfigOption option) => {
    'id': option.id,
    'name': option.label,
    'category': ?option.category,
    'currentValue': option.current,
    'options': [
      for (final choice in option.choices) {'value': choice.value, 'name': choice.label},
    ],
  };

  @override
  void Function() listen(void Function() onChange) {
    store.addListener(onChange);
    return () => store.removeListener(onChange);
  }

  @override
  void attachClient(String id, {String? name}) {
    if (_clients.length >= 16) throw StateError('16 clients are attached already');
    _attach(id, name);
  }

  @override
  void detachClient(String id) {
    if (id != ownerClient) _clients.remove(id);
  }

  @override
  Future<void> send(String text, {required String from}) =>
      store.sendTextTo(session, text, attached: false);

  @override
  Future<void> cancel({required String from}) => store.cancelSession(session);

  @override
  bool settlePermission(int id, {String? optionId, required String by}) {
    // The store answers the oldest request, which is the one a card shows: an answer to any other is not made.
    final first = session.pendingPermissions.firstOrNull;
    if (first == null || _permissionIds[first] != id) return false;
    final option = first.options.where((o) => o.optionId == optionId).firstOrNull;
    _answeredBy[id] = by;
    if (option == null) {
      store.cancelPermission(session);
    } else {
      store.choosePermission(session, option);
    }
    return true;
  }

  @override
  bool settleQuestion(int id, AcpElicitationAnswer answer, {required String by}) {
    final first = session.pendingQuestions.firstOrNull;
    if (first == null || _questionIds[first] != id) return false;
    _questionAnsweredBy[id] = by;
    store.answerQuestion(session, answer);
    return true;
  }
}
