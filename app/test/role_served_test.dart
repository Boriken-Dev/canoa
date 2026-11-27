// The line under a conversation a role serves: which role, which member, and once the host has moved it, when and from whom, with
// what the move could not carry on hover. A session no role serves has none.
import 'package:canoa/history/session_move.dart';
import 'package:canoa/roles/role_models.dart';
import 'package:canoa/roles/role_served_line.dart';
import 'package:canoa/roles/roles_controller.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/l10n/app_text.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/fake_roles.dart';
import 'fixtures/role_scene.dart';

RoleMember _member(Map<String, Object?> json) => RoleMember.parse(json)!;

Future<void> _line(WidgetTester tester, ChatSession session, {String? language}) async {
  final roles = RolesController(host: FakeRolesHost());
  await tester.pumpWidget(MaterialApp(
    theme: themeFor(LookId.canoa, Brightness.light),
    builder: (context, child) => AppTextHost(language: language, child: child!),
    home: Scaffold(body: SessionServedLine(session: session, roles: roles, sources: const [])),
  ));
  await tester.pump();
}

ChatSession _session(RoleServing? serving) {
  final store = roleStore();
  final session = store.createDraft();
  session.roleServing = serving;
  return session;
}

void main() {
  testWidgets('says which member of which role serves, and that its usage is not reported when it is not', (tester) async {
    final serving = RoleServing(role: 'planner', title: 'Planner', member: _member(claudeMember));
    await _line(tester, _session(serving));
    expect(find.text('Served by Claude · work · opus · high · Planner'), findsOneWidget);
    expect(find.byKey(const Key('role-moved')), findsNothing);

    await _line(tester, _session(RoleServing(role: 'planner', title: 'Planner', member: _member(codexMember), usageKnown: false)));
    expect(find.text('Served by Codex · gpt-5 · Planner · usage not reported'), findsOneWidget);
  });

  testWidgets('a move adds when it happened and from whom, and the losses are on hover', (tester) async {
    final serving = RoleServing(role: 'planner', title: 'Planner', member: _member(claudeMember)).movedTo(
      _member(codexMember),
      at: DateTime(2026, 11, 27, 15, 4),
      replayed: false,
      losses: const [MoveLoss('thinking', 3, null), MoveLoss('tool_as_note', 2, null), MoveLoss('odd_kind', 1, 'by the host')],
    );
    await _line(tester, _session(serving));
    expect(find.text('Served by Codex · gpt-5 · Planner'), findsOneWidget);
    expect(
      find.text('Planner moved from Claude · work · opus · high to Codex · gpt-5 at 15:04 · 3 did not carry'),
      findsOneWidget,
    );
    final tip = tester.widget<Tooltip>(find.byKey(const Key('role-losses'))).message!;
    expect(tip, contains('Did not carry'));
    expect(tip, contains('- 3 thought(s) left out'));
    expect(tip, contains('- 2 tool call(s) carried as a note'));
    expect(tip, contains('- odd_kind (1). by the host'), reason: 'a kind the app has no sentence for is named as the host named it');
  });

  testWidgets('a move that lost nothing has no hover, and a replayed one says it was carried as context', (tester) async {
    final clean = RoleServing(role: 'planner', title: 'Planner', member: _member(claudeMember)).movedTo(_member(codexMember), at: DateTime(2026, 11, 27, 9, 5), replayed: false, losses: const []);
    await _line(tester, _session(clean));
    expect(find.text('Planner moved from Claude · work · opus · high to Codex · gpt-5 at 09:05'), findsOneWidget);
    expect(find.byKey(const Key('role-losses')), findsNothing);

    final replayed = RoleServing(role: 'planner', title: 'Planner', member: _member(claudeMember)).movedTo(_member(codexMember), at: DateTime(2026, 11, 27, 9, 5), replayed: true, losses: const []);
    await _line(tester, _session(replayed));
    expect(tester.widget<Tooltip>(find.byKey(const Key('role-losses'))).message, contains('Carried as context in the first message'));
  });

  testWidgets('reads in Spanish, and a session no role serves has no line', (tester) async {
    final serving = RoleServing(role: 'planner', title: 'Planner', member: _member(claudeMember)).movedTo(_member(codexMember), at: DateTime(2026, 11, 27, 15, 4), replayed: false, losses: const [MoveLoss('thinking', 1, null)]);
    await _line(tester, _session(serving), language: 'es');
    expect(find.text('Atendido por Codex · gpt-5 · Planner'), findsOneWidget);
    expect(find.textContaining('pasó de Claude · work · opus · high a Codex · gpt-5 a las 15:04'), findsOneWidget);

    await _line(tester, _session(null));
    expect(find.byKey(const Key('role-served-by')), findsNothing);
  });

  testWidgets('a problem with the role is said in the line, in the language of the app', (tester) async {
    final session = _session(null)..roleProblem = (key: 'role.problem.follow', reason: 'the agent would not start', role: 'Planner');
    await _line(tester, session);
    expect(find.text('Planner: the host moved it on, but this conversation could not follow. the agent would not start'), findsOneWidget);
  });
}
