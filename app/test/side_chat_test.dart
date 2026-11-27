// A side chat: `/btw` and a question, or Side chat in the conversation's menu, asks the same agent about the conversation in a session
// of its own that is in no list and not in the ledger, and nothing of it reaches the conversation.
import 'package:canoa/chat/side_chat.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';

List<Map<String, dynamic>> _prompts(OptionsAgent agent) => [for (final request in agent.requests) if (request['method'] == 'session/prompt') request];

String _promptText(Map<String, dynamic> request) => [for (final part in (request['params'] as Map)['prompt'] as List) (part as Map)['text']].join();

void main() {
  test('/btw and what follows it is a side question, anything else is not', () {
    expect(sideQuestionOf('/btw is it done'), 'is it done');
    expect(sideQuestionOf('  /BTW   why\nnot  '), 'why\nnot');
    expect(sideQuestionOf('/btw'), '');
    expect(sideQuestionOf('/btwx'), isNull);
    expect(sideQuestionOf('please /btw'), isNull);
  });

  testWidgets('a side chat answers about the conversation and leaves it, and every list, as it was', (tester) async {
    final agent = OptionsAgent();
    final store = SessionStore();
    await pumpApp(tester, store, linked: agent.openLink);
    await newSession(tester);
    await chooseSource(tester, 'Scripted');
    await send(tester, 'Fix the failing test');
    await until(tester, () => _prompts(agent).isNotEmpty, 'the first prompt');
    await settle(tester);
    final parent = store.selected!;
    final before = parent.messages.length;

    await send(tester, '/btw which file did you read');
    expect(find.byKey(const Key('side-chat')), findsOneWidget);
    await until(tester, () => _prompts(agent).length == 2, 'the side question');

    final asked = _promptText(_prompts(agent).last);
    expect(asked, contains('which file did you read'));
    expect(asked, contains('Fix the failing test'), reason: 'the side session knows where the question comes from and what was last asked');
    expect(_prompts(agent).where((request) => _promptText(request).contains('/btw')), isEmpty, reason: 'the command is the app\'s: it is sent to nobody');
    expect(store.selected, same(parent), reason: 'the conversation stays the one shown');
    expect(store.listed, [parent]);
    expect(store.sessions, [parent]);
    expect(parent.messages, hasLength(before), reason: 'what is said in the side chat is not said in the conversation');
    expect(find.byType(TextField), findsWidgets);

    await tester.tap(find.byKey(const Key('side-close')));
    await settle(tester);
    expect(find.byKey(const Key('side-chat')), findsNothing);
    expect(store.listed, [parent]);
    await finish(tester);
  });
}
