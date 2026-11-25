// `[usage] warn_permille` reaches every view of usage: the Accounts page and the account view read it from `UsageWarn`, the
// Usage page takes it as given, and a change shows in a page that is already open.
import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/accounts/accounts_section.dart';
import 'package:canoa/accounts/usage_view.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/usage/usage_warn.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_accounts.dart';

UsageReport _report() {
  final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
  return UsageReport(supported: true, account: 'id-work', buckets: [
    UsageBucket(id: 'weekly', label: 'Weekly', used: 700, limit: 1000, refill: UsageRefill('fixed_window', length: const Duration(days: 7), resetsAt: now + 86400)),
  ]);
}

Color? _left(WidgetTester tester) => tester.widget<Text>(find.byKey(const Key('account-left weekly'))).style?.color;

Color _warn(WidgetTester tester) => Theme.of(tester.element(find.byKey(const Key('account-left weekly')))).colorScheme.tertiary;

Widget _app(int permille, Widget child) => MaterialApp(
  home: UsageWarn(permille: permille, child: Scaffold(body: SingleChildScrollView(child: child))),
);

void main() {
  testWidgets('an account\'s usage warns from the configured fraction', (tester) async {
    await tester.pumpWidget(_app(600, AccountUsageView(report: _report())));
    expect(_left(tester), _warn(tester));
    await tester.pumpWidget(_app(800, AccountUsageView(report: _report())));
    expect(_left(tester), isNot(_warn(tester)));
  });

  testWidgets('an explicit threshold wins over the inherited one', (tester) async {
    await tester.pumpWidget(_app(800, AccountUsageView(report: _report(), warnPermille: 600)));
    expect(_left(tester), _warn(tester));
  });

  testWidgets('the Accounts page follows a change of the setting while it is open', (tester) async {
    final controller = AccountsController(
      host: FakeAccountsHost(accounts: [account('work', kind: 'claude-login', id: 'id-work')]),
      config: () => const CanoaConfig(),
      change: (_) async {},
      usage: (id) => id == 'id-work' ? _report() : null,
    );
    tester.view.physicalSize = const Size(900, 1400);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(_app(800, AccountsSection(controller: controller)));
    await tester.pump();
    await tester.pump();
    await tester.tap(find.byKey(const Key('account-usage-toggle work')));
    await tester.pump();
    expect(_left(tester), isNot(_warn(tester)));
    await tester.pumpWidget(_app(600, AccountsSection(controller: controller)));
    expect(_left(tester), _warn(tester));
  });
}
