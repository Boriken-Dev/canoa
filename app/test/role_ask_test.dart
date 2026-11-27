// A role whose handover is `ask` holds a move to another agent for the person's yes: the host pushes `roles/ask`, the conversation
// asks "Usage ran out on X. Continue on Y?", and Continue tells the host to go on. The app never guesses at a move itself.
import 'package:canoa/roles/role_models.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/role_scene.dart';

RoleAskPush _push({String session = 'o1'}) => RoleAskPush.parse({
  'role': 'planner',
  'session': session,
  'from': {'entry': 'Claude', 'session': session},
  'to': {'entry': 'Codex', 'model': 'gpt-5'},
  'reason': 'Claude: the five-hour window is used up',
})!;

void main() {
  testWidgets('the card names both members and the reason, and Continue says yes to the host', (tester) async {
    final roles = plannerHost(handover: 'ask');
    final session = await startPlanner(tester, roles, OptionsAgent(), roleStore());
    expect(session.roleAsk, isNull, reason: 'nothing is asked until the host asks');
    roles.ask(_push());
    await until(tester, () => session.roleAsk != null, 'the question');
    expect(find.text('Usage ran out on Claude \u00b7 work \u00b7 opus \u00b7 high. Continue on Codex \u00b7 gpt-5?'), findsOneWidget);
    expect(find.text('Claude: the five-hour window is used up'), findsOneWidget);
    expect(roles.continues, isEmpty, reason: 'nothing moves until the person says yes');

    await tester.tap(find.byKey(const Key('role-ask-continue')));
    await settle(tester);
    expect(roles.continues, ['o1']);
    expect(find.byKey(const Key('role-ask')), findsNothing);
    await finish(tester);
  });

  testWidgets('Not now takes the card away and says nothing to the host', (tester) async {
    final roles = plannerHost(handover: 'ask');
    final session = await startPlanner(tester, roles, OptionsAgent(), roleStore());
    roles.ask(_push());
    await until(tester, () => session.roleAsk != null, 'the question');
    await tester.tap(find.byKey(const Key('role-ask-dismiss')));
    await settle(tester);
    expect(find.byKey(const Key('role-ask')), findsNothing);
    expect(roles.continues, isEmpty);
    await finish(tester);
  });

  testWidgets('a question about a conversation that is not open here is not shown, and no turn ends in a guess', (tester) async {
    final roles = plannerHost(handover: 'ask');
    final session = await startPlanner(tester, roles, OptionsAgent(), roleStore());
    roles.ask(_push(session: 'elsewhere'));
    await settle(tester, 150);
    expect(session.roleAsk, isNull);
    expect(roles.resolves.where((each) => each.current != null), isEmpty, reason: 'the host is never asked where it would move');
    await finish(tester);
  });
}
