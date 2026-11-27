// Settings > Roles drawn alone: the group over a controller on a fake host, in the look and language a test chooses.
import 'package:canoa/accounts/account_models.dart';
import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/l10n/app_text.dart';
import 'package:canoa/roles/roles_controller.dart';
import 'package:canoa/roles/roles_section.dart';
import 'package:canoa/theme/look.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fake_accounts.dart';
import 'fake_roles.dart';

/// When a member out of usage may serve again, Unix seconds.
const roleUntil = 1796126640;

/// The planner role: Claude is out of usage, Codex serves now, Pi is not installed; and a reviewer that Codex serves.
FakeRolesHost settingsHost() => FakeRolesHost(roles: [
  roleOf(
    'planner',
    [
      memberJson('Claude', account: 'work', model: 'opus', effort: 'high', whyNot: {
        'member': 'Claude/work/opus/high/',
        'why': 'five-hour window used up',
        'until': roleUntil,
        'allowance': true,
        'unavailable': false,
      }),
      memberJson('Codex', model: 'gpt-5', serving: true),
      memberJson('Pi', whyNot: {'member': 'Pi///', 'why': 'not installed', 'until': null, 'allowance': false, 'unavailable': true}),
    ],
    title: 'Planner',
    needs: ['chat'],
    handover: 'ask',
  ),
  roleOf('reviewer', [memberJson('Codex', model: 'gpt-5-mini', serving: true)], title: 'Reviewer', strategy: 'headroom'),
]);

/// Draws the roles group over [host] in a window of [size] and lets the host be asked.
Future<(RolesController, FakeRolesHost)> showRoles(
  WidgetTester tester, {
  FakeRolesHost? host,
  String? language,
  Size size = const Size(900, 1800),
  Brightness brightness = Brightness.light,
}) async {
  final roles = host ?? settingsHost();
  final controller = RolesController(
    host: roles,
    agents: () => const [(entry: 'Claude', name: 'Claude'), (entry: 'Codex', name: 'Codex'), (entry: 'Pi', name: 'Pi')],
  );
  final accounts = AccountsController(
    host: FakeAccountsHost(accounts: [account('work', kind: 'claude-login')])
      ..hostEntries.add(const AccountEntry(name: 'Claude', harness: 'claude', accounts: ['work'], accepts: ['claude-code'], usable: ['work'])),
    config: () => const CanoaConfig(),
    change: (edit) async {},
    roles: controller,
  );
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(
    theme: themeFor(LookId.canoa, brightness),
    builder: (context, child) => AppTextHost(language: language, child: child!),
    home: Scaffold(body: SingleChildScrollView(padding: const EdgeInsets.all(16), child: RolesGroup(roles: controller, accounts: accounts))),
  ));
  await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 20)));
  await tester.pump();
  await tester.pump();
  return (controller, roles);
}
