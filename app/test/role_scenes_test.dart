// The pictures of roles: the start screen with a role chosen, a conversation asking before it moves to another agent, the same
// conversation after the host moved it, and Settings > Roles with its dialog, at desktop and phone width.
import 'dart:io' show Directory;

import 'package:canoa/roles/role_models.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/fake_roles.dart';
import 'fixtures/options_agent.dart';
import 'fixtures/role_scene.dart';
import 'fixtures/roles_page.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

const _sizes = [('phone', Size(390, 844)), ('desktop', Size(1280, 800))];

RoleResolution _next() => RoleResolution.parse({
  'role': 'planner',
  'member': codexMember,
  'moved': true,
  'why': 'exhausted',
  'left': 'Claude: the five-hour window is used up, refills 15:04',
  'usage_known': true,
  'same_harness': false,
  'skipped': [],
})!;

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/role_scenes_test.dart')));

  for (final (label, size) in _sizes) {
    testWidgets('the start screen with a role chosen, at $label width', (tester) async {
      final roles = plannerHost();
      await pumpApp(tester, roleStore(), linked: OptionsAgent().openLink, config: roleConfig, accountsHost: roleAccounts(), rolesHost: roles, size: size);
      await newSession(tester);
      await settle(tester);
      await chooseRoleOnStartScreen(tester, 'planner');
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/role_start_$label.png'));
      await finish(tester);
    });

    testWidgets('a conversation asking before it moves to another agent, at $label width', (tester) async {
      final roles = plannerHost(handover: 'ask')..movedOn['planner'] = _next();
      final session = await startPlanner(tester, roles, OptionsAgent(), roleStore(), size: size);
      await until(tester, () => session.roleAsk != null, 'the question');
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/role_ask_$label.png'));
      await finish(tester);
    });

    testWidgets('a conversation the host moved on, at $label width', (tester) async {
      final roles = plannerHost();
      final session = await startPlanner(tester, roles, OptionsAgent(), roleStore(), size: size);
      roles.roles[0] = roleOf('planner', [
        memberJson('Claude', account: 'work', model: 'opus', effort: 'high', whyNot: {'member': 'Claude/work/opus/high/', 'why': 'used up', 'until': 1, 'allowance': true, 'unavailable': false}),
        memberJson('Codex', model: 'gpt-5', serving: true),
      ], title: 'Planner');
      roles.move(RoleMoved.parse({
        'role': 'planner',
        'from': {'entry': 'Claude', 'session': 'o1'},
        'to': {'entry': 'Codex', 'session': 'o7'},
        'replayed': false,
        'losses': [
          {'kind': 'thinking', 'count': 3},
          {'kind': 'tool_as_note', 'count': 2},
        ],
      })!);
      await until(tester, () => session.remoteSessionId == 'o7', 'the conversation to follow the host');
      await settle(tester, 100);
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/role_moved_$label.png'));
      await finish(tester);
    });

    testWidgets('Settings > Roles at $label width', (tester) async {
      await showRoles(tester, size: Size(size.width, size.height));
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/roles_settings_$label.png'));
    });

    testWidgets('the dialog that makes or changes a role, at $label width', (tester) async {
      await showRoles(tester, size: Size(size.width, size.height * 1.4));
      await tester.tap(find.byKey(const Key('role-edit planner')));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(tester.takeException(), isNull);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/role_dialog_$label.png'));
    });
  }
}
