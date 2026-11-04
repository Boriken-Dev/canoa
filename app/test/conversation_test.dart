// What a person sees of a conversation while an agent works in it: the end it follows, the question it asks, and the frame around
// it on a desktop.
import 'package:canoa/chat/transcript.dart';
import 'package:canoa/desktop/host_supervisor.dart';
import 'package:canoa/main.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/v2_agent.dart';

Future<(V2Agent, ChatSession, SessionStore)> _shown(WidgetTester tester, Size size, {HostSupervisor? host}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final agent = V2Agent();
  final (store, session) = await tester.runAsync(agent.start) as (SessionStore, ChatSession);
  await tester.pumpWidget(CanoaApp(store: store, host: host));
  await tester.pumpAndSettle();
  return (agent, session, store);
}

Future<void> _settle(WidgetTester tester) async {
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pumpAndSettle();
}

void _say(V2Agent agent, int from, int to) {
  for (var i = from; i < to; i++) {
    agent.update({'sessionUpdate': 'agent_message_chunk', 'messageId': 'm$i', 'content': {'type': 'text', 'text': 'Entry $i'}});
  }
}

ScrollPosition _transcript(WidgetTester tester) => tester
    .state<ScrollableState>(find.ancestor(of: find.byType(TranscriptEntry).first, matching: find.byType(Scrollable)).first)
    .position;

bool _inView(WidgetTester tester, Finder finder) => finder.hitTestable().evaluate().isNotEmpty;

void main() {
  group('the conversation follows its end', () {
    testWidgets('while it is at the bottom, each new entry comes into view', (tester) async {
      final (agent, _, _) = await _shown(tester, const Size(1000, 700));
      _say(agent, 0, 40);
      await _settle(tester);
      expect(_inView(tester, find.text('Entry 39')), isTrue);
      _say(agent, 40, 45);
      await _settle(tester);
      expect(_inView(tester, find.text('Entry 44')), isTrue);
      expect(find.byTooltip('Jump to latest'), findsNothing);
    });

    testWidgets('scrolled up, it stays put, and a control brings it to the latest', (tester) async {
      final (agent, _, _) = await _shown(tester, const Size(1000, 700));
      _say(agent, 0, 40);
      await _settle(tester);
      await tester.dragFrom(const Offset(900, 300), const Offset(0, 600));
      await tester.pumpAndSettle();
      final reading = _transcript(tester).pixels;
      _say(agent, 40, 45);
      await _settle(tester);
      expect(_transcript(tester).pixels, reading, reason: 'what the person is reading stays where it is');
      expect(_inView(tester, find.text('Entry 44')), isFalse);
      await tester.tap(find.byTooltip('Jump to latest'));
      await tester.pumpAndSettle();
      expect(_inView(tester, find.text('Entry 44')), isTrue);
      expect(find.byTooltip('Jump to latest'), findsNothing);
      _say(agent, 45, 47);
      await _settle(tester);
      expect(_inView(tester, find.text('Entry 46')), isTrue, reason: 'back at the end, it follows again');
    });
  });

  group('a question', () {
    testWidgets('at phone width puts its answers under what it asks, and leaves room for the conversation', (tester) async {
      final (agent, _, _) = await _shown(tester, const Size(390, 844));
      agent.ask(91, 'Edit src/calc.js', {'file_path': 'src/calc.js', 'old_string': 'return a - b;', 'new_string': 'return a + b;'});
      await _settle(tester);
      final card = find.ancestor(of: find.text('Allow'), matching: find.byType(Card));
      final asked = find.descendant(of: card, matching: find.text('Edit src/calc.js'));
      expect(tester.getTopLeft(find.text('Allow')).dy, greaterThan(tester.getBottomLeft(asked).dy));
      expect(tester.getSize(card).height, lessThan(844 / 3));
    });

    testWidgets('a two-line edit is not called long, and one past its box is', (tester) async {
      final (agent, session, store) = await _shown(tester, const Size(1280, 800));
      // As Claude's adapter shapes an edit: test/fixtures/coding_task_updates.jsonl.
      agent.ask(92, 'Edit src/calc.js', {'replace_all': false, 'file_path': 'src/calc.js', 'old_string': 'a\nb', 'new_string': 'c\nd'});
      await _settle(tester);
      expect(find.textContaining('This is long'), findsNothing);
      store.answerPermission(session, allow: true);
      agent.ask(93, 'Write a file', {'file_path': 'big.txt', 'content': List.generate(30, (i) => 'line $i').join('\n')});
      await _settle(tester);
      expect(find.textContaining('This is long'), findsOneWidget);
    });
  });

  group('on a desktop, with an agent on this computer', () {
    testWidgets('nothing speaks of peers a local agent does not have', (tester) async {
      await _shown(tester, const Size(1280, 800));
      expect(find.textContaining('verified peer authentication'), findsNothing);
      expect(find.textContaining('Agent connections and sync'), findsNothing);
    });

    testWidgets('the wordmark keeps to one line beside its four buttons at the default window width', (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.windows;
      try {
        await _shown(tester, const Size(1280, 720), host: HostSupervisor(null));
        expect(find.byTooltip('Hosting'), findsOneWidget);
        // One line of its 22-point type is shorter than two.
        expect(tester.getSize(find.text('Canoa')).height, lessThan(2 * 22));
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    });
  });
}
