// What carries a conversation to a window: strings over a channel, a launch the window starts from, and the snapshot of Canoa's own
// conversation state that the window's view is built from.
import 'dart:async';
import 'dart:convert';

import 'package:canoa/popout/link_stream.dart';
import 'package:canoa/popout/platform.dart' as platform;
import 'package:canoa/popout/popout_launch.dart';
import 'package:canoa/popout/shared_chat.dart';
import 'package:canoa/session_store.dart';
import 'package:coqui_acp/coqui_acp.dart' as acp;
import 'package:coqui_acp_chat/coqui_acp_chat.dart' as model;
import 'package:coqui_acp_chat/experimental.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/scripted_agent.dart';

/// Two ends of a channel that carries strings, as a platform's window messages do: each arrives after the sender's current work.
final class _Loop extends StringViewLink {
  late _Loop other;
  final seen = <String>[];

  static (_Loop, _Loop) pair() {
    final a = _Loop(), b = _Loop();
    a.other = b;
    b.other = a;
    return (a, b);
  }

  @override
  void write(String text) {
    seen.add(text);
    scheduleMicrotask(() => other.receive(text));
  }

  @override
  Future<void> release() async {}
}

Future<void> _later() => Future<void>.delayed(const Duration(milliseconds: 60));

void main() {
  group('a link over strings', () {
    test('carries a conversation both ways as lines of JSON, and ignores what is not a message', () async {
      final session = model.ChatSession(id: 'a', sessionId: 's1');
      session.attach(_Quiet());
      session.apply(acp.AcpSessionUpdate('s1', {
        'sessionUpdate': 'agent_message',
        'messageId': 'm1',
        'content': {'type': 'text', 'text': 'Hello'},
      }));
      final share = ChatSessionShare.of(session, minInterval: Duration.zero);
      final (owner, far) = _Loop.pair();
      share.serve(owner);
      final view = await ChatSessionView.attach(far, clientId: 'win');
      expect(view.session.messages.single.text, 'Hello');
      expect(far.seen.first, startsWith('{'), reason: 'the view sent a JSON line');
      expect(jsonDecode(owner.seen.first), containsPair('t', 'state'));

      // Anything else on the channel is not for the link.
      owner.write('not json');
      owner.receive('not json');
      owner.receive(5);
      owner.receive('[1, 2]');
      await _later();
      expect(view.isOpen, isTrue);

      await view.send('hi').then((_) {}, onError: (_) {});
      await share.close();
    });

    test('a closed link sends nothing and its messages end', () async {
      final (a, b) = _Loop.pair();
      final ended = a.messages.isEmpty;
      await a.close();
      a.send({'t': 'x'});
      expect(a.seen, isEmpty);
      expect(a.isClosed, isTrue);
      expect(await ended, isTrue);
      await b.close();
    });
  });

  group('the launch a window starts from', () {
    test('goes through a string and back, and a string that is not one is not read as one', () {
      const launch = PopoutLaunch(link: 'abc', title: 'Fix calc', look: 'terminal', density: 'verbose');
      final back = PopoutLaunch.decode(launch.encode())!;
      expect((back.link, back.title, back.look, back.density), ('abc', 'Fix calc', 'terminal', 'verbose'));
      expect(PopoutLaunch.decode('{"kind":"other","link":"x"}'), isNull);
      expect(PopoutLaunch.decode('{"kind":"canoa.popout"}'), isNull);
      expect(PopoutLaunch.decode('nonsense'), isNull);
      expect(PopoutLaunch.decode('{"kind":"canoa.popout","link":"x"}')!.look, 'canoa', reason: 'what is missing takes its default');
    });

    test('a desktop engine\'s arguments are a launch when they are the plugin\'s and carry one', () {
      const launch = PopoutLaunch(link: 'abc', title: 'T');
      expect(platform.popoutLaunch(['multi_window', 'window-id', launch.encode()])?.link, 'abc');
      expect(platform.popoutLaunch(['multi_window', 'window-id', '{}']), isNull);
      expect(platform.popoutLaunch(['multi_window']), isNull);
      expect(platform.popoutLaunch(const []), isNull);
      expect(platform.popoutLaunch(['--no-local-host']), isNull);
    });
  });

  group('Canoa\'s own conversation as a view reads it', () {
    test('every kind of entry, the plan, usage, title and what waits reach the view, and its turns are the person\'s messages', () async {
      final store = SessionStore();
      addTearDown(store.dispose);
      final agent = ScriptedAgent();
      final session = await agent.start(store);
      unawaited(store.sendText('Fix addNumbers'));
      agent.update({'sessionUpdate': 'agent_thought_chunk', 'content': {'type': 'text', 'text': 'It subtracts.'}});
      agent.update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'Editing.'}});
      agent.update({
        'sessionUpdate': 'tool_call',
        'toolCallId': 'e',
        'title': 'Edit calc.js',
        'kind': 'edit',
        'status': 'completed',
        'content': [
          {'type': 'diff', 'path': 'calc.js', 'oldText': 'a - b', 'newText': 'a + b'},
          {'type': 'terminal', 'terminalId': 'term'},
        ],
      });
      agent.update({
        'sessionUpdate': 'terminal_update',
        'terminalId': 'term',
        'command': 'node test.js',
        'output': {'data': base64.encode(utf8.encode('ok\n'))},
        'exitStatus': {'exitCode': 0},
      });
      agent.update({
        'sessionUpdate': 'plan',
        'entries': [
          {'content': 'Fix', 'status': 'completed'},
          {'content': 'Test', 'status': 'in_progress'},
        ],
      });
      agent.update({'sessionUpdate': 'usage_update', 'used': 5, 'size': 10});
      agent.update({
        'sessionUpdate': 'available_commands_update',
        'availableCommands': [
          {'name': 'review', 'description': 'Review it'},
        ],
      });
      agent.update({'sessionUpdate': 'session_info_update', 'title': 'Calc'});
      agent.ask(5, 'Edit a.txt');
      await Future<void>.delayed(const Duration(milliseconds: 60));
      await store.sendText('and a test');
      expect(session.queue, hasLength(1));

      final share = ChatSessionShare(CanoaSharedChat(store, session), minInterval: Duration.zero);
      addTearDown(share.close);
      final (owner, far) = MemoryViewLink.pair();
      share.serve(owner);
      final view = await ChatSessionView.attach(far, clientId: 'win', name: 'Window');
      final shown = view.session;

      expect(shown.messages.where((m) => m.role != model.MessageRole.notice).map((m) => (m.role, m.text)), [
        (model.MessageRole.user, 'Fix addNumbers'),
        (model.MessageRole.thought, 'It subtracts.'),
        (model.MessageRole.agent, 'Editing.'),
        (model.MessageRole.tool, ''), // the edit
        (model.MessageRole.tool, ''), // what the agent asks about
      ]);
      final edit = shown.toolEntries['e']!.tool!;
      expect((edit.title, edit.status), ('Edit calc.js', model.ToolStatus.done));
      expect(edit.blocks.whereType<model.ToolDiff>().single.newText, 'a + b');
      expect(shown.terminals['term']!.output, 'ok\n');
      expect(shown.plan.map((p) => (p.content, p.status)), [('Fix', model.PlanStatus.done), ('Test', model.PlanStatus.running)]);
      expect((shown.usage.used, shown.usage.size), (5, 10));
      expect(shown.commands.single.name, 'review');
      expect(shown.title, 'Fix calc', reason: 'the name Canoa gave it, which its agent does not rename');
      expect(shown.agent, 'Scripted');
      expect(shown.status, model.ChatStatus.working);
      expect(shown.currentTurn, isNotNull, reason: 'the turn is open while the session works');
      expect(shown.queue.single.text, 'and a test');
      expect(shown.pendingPermissions.single.title, 'Edit a.txt');
      expect(shown.pendingPermissions.single.options.map((o) => o.optionId), ['allow-once', 'reject']);
      expect(shown.clients.map((c) => c.id), unorderedEquals([ownerClient, 'win']));
    });

    test('a request answered in the main window reaches the view as answered by it, with the answer the agent got', () async {
      final store = SessionStore();
      addTearDown(store.dispose);
      final agent = ScriptedAgent();
      final session = await agent.start(store);
      agent.ask(5, 'Edit a.txt');
      await Future<void>.delayed(const Duration(milliseconds: 60));
      final share = ChatSessionShare(CanoaSharedChat(store, session), minInterval: Duration.zero);
      addTearDown(share.close);
      final (owner, far) = MemoryViewLink.pair();
      share.serve(owner);
      final view = await ChatSessionView.attach(far, clientId: 'win');
      expect(view.session.pendingPermissions, hasLength(1));

      store.choosePermission(session, session.pendingPermissions.first.options.first);
      await _later();
      expect(view.session.pendingPermissions, isEmpty);
      final outcome = view.session.settledPermissions.single.outcome!;
      expect((outcome.how, outcome.by), (model.Settlement.answered, ownerClient));
      expect((outcome.answer as acp.AcpSelect).optionId, 'allow-once');
    });

    test('a request the view answers is answered once, as that view, and one it names that is not the oldest is not answered', () async {
      final store = SessionStore();
      addTearDown(store.dispose);
      final agent = ScriptedAgent();
      final session = await agent.start(store);
      agent.ask(5, 'Edit a.txt');
      agent.ask(6, 'Edit b.txt');
      await Future<void>.delayed(const Duration(milliseconds: 60));
      final share = ChatSessionShare(CanoaSharedChat(store, session), minInterval: Duration.zero);
      addTearDown(share.close);
      final (owner, far) = MemoryViewLink.pair();
      share.serve(owner);
      final view = await ChatSessionView.attach(far, clientId: 'win');
      final first = view.session.pendingPermissions[0], second = view.session.pendingPermissions[1];

      view.choosePermission(second, second.options.first);
      await _later();
      expect(agent.answerTo(6), isNull, reason: 'the store answers the oldest; a card shows only that one');
      expect(session.pendingPermissions, hasLength(2));

      view.choosePermission(first, first.options.first);
      await _later();
      expect(agent.answerTo(5)?['result'], {'outcome': {'outcome': 'selected', 'optionId': 'allow-once'}});
      expect(view.session.settledPermissions.single.outcome!.by, 'win');
      expect(view.session.pendingPermissions.single.title, 'Edit b.txt');

      // Cancelled: the agent is told so, and the view sees it cancelled, not answered by someone.
      view.choosePermission(view.session.pendingPermissions.single, null);
      await _later();
      expect(agent.answerTo(6)?['result'], {'outcome': {'outcome': 'cancelled'}});
      expect(view.session.pendingPermissions, isEmpty);
    });
  });
}

/// A connection that is only there, so a conversation counts as connected.
final class _Quiet extends model.ChatConnection {
  @override
  Stream<acp.AcpSessionUpdate> get updates => _updates.stream;
  final _updates = StreamController<acp.AcpSessionUpdate>.broadcast();
  @override
  Stream<acp.AcpEvent> get events => const Stream.empty();
  @override
  bool get isClosed => false;
  @override
  int? get protocolVersion => 2;
  @override
  Future<String?> prompt(String sessionId, String text, {List<acp.AcpAttachment> attachments = const []}) async => 'm';
  @override
  Future<void> cancel(String sessionId) async {}
  @override
  Future<void> setMode(String sessionId, String modeId) async {}
  @override
  Future<Map<String, dynamic>> setConfigOption(String sessionId, String configId, String value) async => {};
  @override
  Future<void> close() async {}
}
