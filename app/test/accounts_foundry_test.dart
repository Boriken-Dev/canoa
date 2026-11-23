// A local model as an account: the host's own kind and sign-in method, read from the catalog the host writes
// (`crates/canoa-host/tests/vectors/foundry_catalog.json`, which a host test keeps equal to what it sends), listed and added in
// Settings › Accounts like any other, with nothing secret to type.
import 'dart:convert';
import 'dart:io';

import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/accounts/accounts_section.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/fake_accounts.dart';

Object? _hostCatalog() => jsonDecode(File('../crates/canoa-host/tests/vectors/foundry_catalog.json').readAsStringSync());

AccountsController _controller(FakeAccountsHost host) {
  var held = const CanoaConfig();
  return AccountsController(host: host, config: () => held, change: (edit) async => held = edit(held));
}

Future<void> _show(WidgetTester tester, AccountsController controller) async {
  tester.view.physicalSize = const Size(900, 1400);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(child: AccountsSection(controller: controller)))));
  await tester.pump();
  await tester.pump();
}

void main() {
  testWidgets('the section lists a Foundry Local account by its kind, and adding one asks for the model and no secret', (tester) async {
    final host = FakeAccountsHost(accounts: [account('local', kind: 'foundry-local')]);
    host.catalogs['claude'] = _hostCatalog();
    await _show(tester, _controller(host));
    expect(find.text('Foundry Local · Signed in'), findsOneWidget);

    await tester.tap(find.byKey(const Key('add-account-claude_accounts')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('login-method foundry-local')), findsOneWidget);
    expect(find.text('Use a Foundry Local model'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('add-account-name')), 'qwen');
    await tester.tap(find.byKey(const Key('login-method foundry-local')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('add-account-start')));
    await tester.pumpAndSettle();

    // The form is the method's inputs: the model is asked, and nothing in it is a secret.
    expect(find.byKey(const Key('program-ask')), findsOneWidget);
    expect(find.text('Model'), findsWidgets);
    await tester.enterText(find.byKey(const Key('ask-field-model')), 'qwen3-14b-generic-gpu:2');
    await tester.tap(find.byKey(const Key('ask-submit')));
    await tester.pump();
    await tester.pump();
    expect(host.inputsGiven.single, {'model': 'qwen3-14b-generic-gpu:2'});
    expect(host.signIns.single.method, 'foundry-local');
    expect(host.signIns.single.name, 'qwen');
  });
}
