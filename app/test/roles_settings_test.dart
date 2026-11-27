// Settings > Roles: the host's roles with their members in order and a chip for each (serving now, available, or why not), made,
// edited, reordered and removed through the host's configuration store, with its refusal shown in its own words.
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/fake_roles.dart';
import 'fixtures/roles_page.dart';

Future<void> _pickDropdown(WidgetTester tester, Key dropdown, String label) async {
  await tester.ensureVisible(find.byKey(dropdown));
  await tester.tap(find.byKey(dropdown));
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 300));
  await tester.tap(find.text(label).last);
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 300));
}

void main() {
  testWidgets('a card per role shows its members in order, with a chip for what each can do now', (tester) async {
    await showRoles(tester);
    expect(find.byKey(const Key('role planner')), findsOneWidget);
    expect(find.text('Planner'), findsWidgets);
    expect(find.textContaining('First with usage'), findsOneWidget);
    expect(find.textContaining('models report chat'), findsOneWidget);
    expect(find.textContaining('asks before moving to another agent'), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('role-member planner 0')), matching: find.text('Claude · work · opus · high')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('role-member planner 1')), matching: find.text('Codex · gpt-5')), findsOneWidget);
    expect(find.descendant(of: find.byKey(const Key('role-chip planner 1')), matching: find.text('Serving now')), findsOneWidget);
    // A member out of usage says so in the host's words and when it may serve again; one that cannot run says why, and no time.
    final out = tester.widget<Text>(find.descendant(of: find.byKey(const Key('role-chip planner 0')), matching: find.byType(Text)));
    expect(out.data, startsWith('five-hour window used up · until '));
    expect(find.descendant(of: find.byKey(const Key('role-chip planner 2')), matching: find.text('not installed')), findsOneWidget);
  });

  testWidgets('a host that lists no role says so and offers Add a role', (tester) async {
    await showRoles(tester, host: FakeRolesHost());
    expect(find.byKey(const Key('roles-none')), findsOneWidget);
    expect(find.byKey(const Key('add-role')), findsOneWidget);
  });

  testWidgets('a host that cannot list roles says why instead of a card', (tester) async {
    await showRoles(tester, host: FakeRolesHost()..listFails = 'grant for accounts needed');
    expect(find.text('Roles are not available'), findsOneWidget);
    expect(find.text('Bad state: grant for accounts needed'), findsOneWidget);
    expect(find.byKey(const Key('add-role')), findsNothing);
  });

  testWidgets('adding a role writes [host.roles.NAME] through the host with the members in order and the handover choice', (tester) async {
    final (_, host) = await showRoles(tester, host: FakeRolesHost());
    await tester.tap(find.byKey(const Key('add-role')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));
    await tester.enterText(find.byKey(const Key('role-name')), 'executor');
    await tester.enterText(find.byKey(const Key('role-title')), 'Executor');
    await tester.ensureVisible(find.byKey(const Key('role-add-member')));
    await tester.tap(find.byKey(const Key('role-add-member')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('role-add-member')));
    await tester.pump();
    await tester.enterText(find.byKey(const Key('role-member-model 0')), 'sonnet');
    await _pickDropdown(tester, const Key('role-member-agent 1 Claude'), 'Codex');
    await tester.enterText(find.byKey(const Key('role-member-model 1')), 'gpt-5');
    await _pickDropdown(tester, const Key('role-handover'), 'Ask me first');
    await _pickDropdown(tester, const Key('role-strategy'), 'Most usage left');
    await tester.ensureVisible(find.byKey(const Key('role-save')));
    await tester.tap(find.byKey(const Key('role-save')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));
    final (name, table) = host.written.single;
    expect(name, 'executor');
    expect(table['title'], 'Executor');
    expect(table['strategy'], 'headroom');
    expect(table['handover'], 'ask');
    expect(table['stick'], true);
    expect(table.containsKey('retry_task'), isFalse, reason: 'the host\'s default is not written');
    expect(table['members'], [
      {'agent': 'Claude', 'model': 'sonnet'},
      {'agent': 'Codex', 'model': 'gpt-5'},
    ]);
    expect(find.byKey(const Key('role-dialog')), findsNothing);
    expect(find.byKey(const Key('role executor')), findsOneWidget, reason: 'the list is read again after the write');
  });

  testWidgets('a name the host would refuse is not sent, and the host\'s own refusal is shown as it wrote it', (tester) async {
    final (_, host) = await showRoles(tester, host: FakeRolesHost());
    await tester.tap(find.byKey(const Key('add-role')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));
    await tester.enterText(find.byKey(const Key('role-name')), 'Big Name');
    await tester.tap(find.byKey(const Key('role-save')));
    await tester.pump();
    expect(find.byKey(const Key('role-problem')), findsOneWidget);
    expect(host.written, isEmpty);
    await tester.enterText(find.byKey(const Key('role-name')), 'planner');
    host.writeRefusal = 'host.roles.planner.members[0].agent: no agent called Nope';
    await tester.tap(find.byKey(const Key('role-save')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
    await tester.pump();
    expect(find.text('host.roles.planner.members[0].agent: no agent called Nope'), findsOneWidget);
    expect(find.byKey(const Key('role-dialog')), findsOneWidget, reason: 'the dialog stays to be put right');
  });

  testWidgets('editing reorders the members and keeps what the dialog does not show', (tester) async {
    final (_, host) = await showRoles(tester);
    await tester.tap(find.byKey(const Key('role-edit planner')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));
    expect(tester.widget<TextField>(find.byKey(const Key('role-name'))).enabled, isFalse, reason: 'a role keeps its name');
    await tester.ensureVisible(find.byKey(const Key('role-member-up 1')));
    await tester.tap(find.byKey(const Key('role-member-up 1')));
    await tester.pump();
    await tester.ensureVisible(find.byKey(const Key('role-save')));
    await tester.tap(find.byKey(const Key('role-save')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));
    final (_, table) = host.written.single;
    expect((table['members'] as List).map((each) => (each as Map)['agent']), ['Codex', 'Claude', 'Pi']);
    expect(table['handover'], 'ask');
    expect(table['needs'], ['chat']);
    expect((table['members'] as List).first, {'agent': 'Codex', 'model': 'gpt-5'});
  });

  testWidgets('removing a role asks first, then takes its table out', (tester) async {
    final (_, host) = await showRoles(tester);
    await tester.tap(find.byKey(const Key('role-remove planner')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));
    expect(find.byKey(const Key('remove-role')), findsOneWidget);
    expect(host.removed, isEmpty);
    await tester.tap(find.byKey(const Key('remove-role-confirm')));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));
    expect(host.removed, ['planner']);
    expect(find.byKey(const Key('role planner')), findsNothing);
  });

  testWidgets('the page reads in Spanish when the language is Spanish', (tester) async {
    await showRoles(tester, language: 'es');
    expect(find.text('Añadir un rol'), findsOneWidget);
    expect(find.text('Atiende ahora'), findsNWidgets(2), reason: 'a chip for the planner and one for the reviewer');
    expect(find.textContaining('pregunta antes de pasar a otro agente'), findsOneWidget);
    expect(find.text('Add a role'), findsNothing);
  });
}
