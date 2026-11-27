// A role whose handover is `ask` holds a move to another agent for the person's yes: when the member's usage has run out and the
// next is of another harness, the conversation asks "Usage ran out on X. Continue on Y?", and Continue tells the host to go on.
// A role that moves by itself, and a move within one agent, ask nothing.
import 'package:canoa/roles/role_models.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/role_scene.dart';

RoleResolution _next({bool sameHarness = false}) => RoleResolution.parse({
  'role': 'planner',
  'member': codexMember,
  'moved': true,
  'why': 'exhausted',
  'left': 'Claude: five-hour window used up',
  'usage_known': true,
  'same_harness': sameHarness,
  'skipped': [],
})!;

void main() {
  testWidgets('the card names both members, and Continue says yes to the host', (tester) async {
    final roles = plannerHost(handover: 'ask')..movedOn['planner'] = _next();
    final store = roleStore();
    final session = await startPlanner(tester, roles, OptionsAgent(), store);
    await until(tester, () => session.roleAsk != null, 'the question');
    expect(roles.resolves.last.current, 'Claude/work/opus/high/', reason: 'the host is asked where it would move this member');
    expect(find.byKey(const Key('role-ask-text')), findsOneWidget);
    expect(find.text('Usage ran out on Claude · work · opus · high. Continue on Codex · gpt-5?'), findsOneWidget);
    expect(find.text('Claude: five-hour window used up'), findsOneWidget);
    expect(roles.continues, isEmpty, reason: 'nothing moves until the person says yes');

    await tester.tap(find.byKey(const Key('role-ask-continue')));
    await settle(tester);
    expect(roles.continues, [session.remoteSessionId]);
    expect(find.byKey(const Key('role-ask')), findsNothing);
    await finish(tester);
  });

  testWidgets('Not now takes the card away and says nothing to the host', (tester) async {
    final roles = plannerHost(handover: 'ask')..movedOn['planner'] = _next();
    final session = await startPlanner(tester, roles, OptionsAgent(), roleStore());
    await until(tester, () => session.roleAsk != null, 'the question');
    await tester.tap(find.byKey(const Key('role-ask-dismiss')));
    await settle(tester);
    expect(find.byKey(const Key('role-ask')), findsNothing);
    expect(roles.continues, isEmpty);
    await finish(tester);
  });

  testWidgets('a role that moves by itself asks nothing', (tester) async {
    final roles = plannerHost()..movedOn['planner'] = _next();
    final session = await startPlanner(tester, roles, OptionsAgent(), roleStore());
    await settle(tester, 150);
    expect(session.roleAsk, isNull);
    expect(roles.resolves.where((each) => each.current != null), isEmpty, reason: 'the host is not even asked');
    await finish(tester);
  });

  testWidgets('a move within the same agent is never asked about', (tester) async {
    final roles = plannerHost(handover: 'ask')..movedOn['planner'] = _next(sameHarness: true);
    final session = await startPlanner(tester, roles, OptionsAgent(), roleStore());
    await settle(tester, 150);
    expect(roles.resolves.where((each) => each.current != null), isNotEmpty);
    expect(session.roleAsk, isNull);
    await finish(tester);
  });

  testWidgets('a member that can still serve asks nothing', (tester) async {
    final roles = plannerHost(handover: 'ask');
    final session = await startPlanner(tester, roles, OptionsAgent(), roleStore());
    await settle(tester, 150);
    expect(session.roleAsk, isNull);
    await finish(tester);
  });
}
