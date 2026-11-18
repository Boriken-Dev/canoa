// The Secrets section against a host that holds what the test says: the table, the value that is shown only when asked for, forgetting,
// the time left, and setting one by hand. A made-up value stands for a secret, and the page is checked never to hold it unasked.

import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/secrets/secrets_controller.dart';
import 'package:canoa/secrets/secrets_host.dart';
import 'package:canoa/secrets/secrets_section.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

const _value = 'zq9-not-a-real-secret-7f3a';

class _FakeSecrets implements SecretsHost {
  _FakeSecrets(this.rows);

  List<SecretRow> rows;
  final revealed = <String>[];
  final forgotten = <String>[];
  final sets = <SecretSet>[];
  Object? failWith;

  @override
  Future<SecretsListing> list() async {
    if (failWith case final error?) throw error;
    return SecretsListing(rows: List.of(rows), keeps: const [SecretKeep('session', 'This session')]);
  }

  @override
  Future<List<SecretEntry>> entries() async => const [
    SecretEntry(name: 'form', provider: 'form', title: 'Form', canStore: false, canList: false, needsPerson: true),
    SecretEntry(name: 'system', provider: 'system', title: 'System store', canStore: true, canList: true, needsPerson: false),
    SecretEntry(name: 'work', provider: 'bitwarden', title: 'Bitwarden', canStore: false, canList: true, needsPerson: false),
  ];

  @override
  Future<String> reveal(String provider, String name) async {
    revealed.add('$provider/$name');
    return _value;
  }

  @override
  Future<bool> forget(String provider, String name) async {
    forgotten.add('$provider/$name');
    rows = [for (final row in rows) if (!(row.provider == provider && row.name == name)) row];
    return true;
  }

  @override
  SecretSet set({required String provider, required String name}) {
    final run = SecretSet(provider: provider, name: name);
    sets.add(run);
    return run;
  }
}

Future<(SecretsController, _FakeSecrets)> _page(WidgetTester tester, List<SecretRow> rows) async {
  final host = _FakeSecrets(rows);
  final controller = SecretsController(host: host);
  await tester.binding.setSurfaceSize(const Size(1200, 700));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await tester.pumpWidget(MaterialApp(home: Scaffold(body: SecretsPage(controller: controller, refreshEvery: null))));
  await tester.pump();
  await tester.pump();
  return (controller, host);
}

void main() {
  testWidgets('the table lists each secret by provider and name with its value hidden and how long it is kept, and holds no value', (tester) async {
    final (_, host) = await _page(tester, const [
      SecretRow(provider: 'form', name: 'api_key', kept: 'process'),
      SecretRow(provider: 'form', name: 'short', kept: 'process', expiresIn: 840),
      SecretRow(provider: 'system', name: 'deploy', kept: 'stored'),
      SecretRow(provider: 'form', name: 'mine', kept: 'session', session: 's1'),
    ]);
    expect(find.byKey(const Key('secrets-table')), findsOneWidget);
    expect(find.byKey(const Key('secret-name form/api_key')), findsOneWidget);
    expect(find.text('Until Canoa closes'), findsOneWidget);
    expect(find.text('14 min left'), findsOneWidget);
    expect(find.text('Stored'), findsOneWidget);
    expect(find.text('This conversation'), findsOneWidget);
    expect(find.byKey(const Key('secret-hidden')), findsNWidgets(4));
    expect(find.textContaining(_value), findsNothing, reason: 'a value is read only when the person asks');
    expect(host.revealed, isEmpty);
  });

  testWidgets('the value is asked of the host when the person shows it, shown beside its row, and gone when hidden again', (tester) async {
    final (controller, host) = await _page(tester, const [SecretRow(provider: 'form', name: 'api_key', kept: 'process')]);
    await tester.tap(find.byKey(const Key('secret-show form/api_key')));
    await tester.pump();
    await tester.pump();
    expect(host.revealed, ['form/api_key']);
    expect(find.byKey(const Key('secret-value form/api_key')), findsOneWidget);
    expect(find.textContaining(_value), findsOneWidget);
    expect(find.byKey(const Key('secret-hidden')), findsNothing);
    await tester.tap(find.byKey(const Key('secret-show form/api_key')));
    await tester.pump();
    expect(find.textContaining(_value), findsNothing);
    expect(find.byKey(const Key('secret-hidden')), findsOneWidget);
    expect(controller.shown(controller.rows.single), isNull, reason: 'the controller does not keep what is hidden');
    expect(host.revealed, hasLength(1), reason: 'hiding asks the host nothing');
  });

  testWidgets('forgetting asks first, then the host forgets it and the row goes with the value that was shown', (tester) async {
    final (controller, host) = await _page(tester, const [
      SecretRow(provider: 'form', name: 'api_key', kept: 'process'),
      SecretRow(provider: 'system', name: 'deploy', kept: 'stored'),
    ]);
    await tester.tap(find.byKey(const Key('secret-show form/api_key')));
    await tester.pump();
    await tester.pump();
    await tester.tap(find.byKey(const Key('secret-forget form/api_key')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('secret-forget-confirm')), findsOneWidget);
    await tester.tap(find.text('Keep it'));
    await tester.pumpAndSettle();
    expect(host.forgotten, isEmpty, reason: 'a refused confirmation forgets nothing');
    await tester.tap(find.byKey(const Key('secret-forget form/api_key')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('secret-forget-yes')));
    await tester.pumpAndSettle();
    expect(host.forgotten, ['form/api_key']);
    expect(find.byKey(const Key('secret-name form/api_key')), findsNothing);
    expect(find.byKey(const Key('secret-name system/deploy')), findsOneWidget);
    expect(find.textContaining(_value), findsNothing);
    expect(controller.rows.map((row) => row.name), ['deploy']);
  });

  testWidgets('with nothing kept the page says so, and a host that refuses says why in its own words', (tester) async {
    final (controller, host) = await _page(tester, const []);
    expect(find.byKey(const Key('secrets-empty')), findsOneWidget);
    host.failWith = const ControlFailed(-32001, 'this key has no secrets grant');
    await controller.refresh();
    await tester.pump();
    expect(find.text('this key has no secrets grant'), findsOneWidget);
    expect(find.byKey(const Key('secrets-empty')), findsNothing);
  });

  testWidgets('a secret is set by choosing a provider that takes values and a name, then the host asks for the value', (tester) async {
    final (controller, host) = await _page(tester, const []);
    await tester.tap(find.byKey(const Key('secret-set')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('secret-set-dialog')), findsOneWidget);
    // Only entries that keep what they are given, or that are asked in the app, take a value by hand: not a vault that is read only.
    await tester.tap(find.byKey(const Key('secret-set-provider')));
    await tester.pumpAndSettle();
    expect(find.textContaining('work (Bitwarden)'), findsNothing);
    expect(find.textContaining('system (System store)'), findsWidgets);
    await tester.tap(find.textContaining('system (System store)').last);
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('secret-set-name')), 'deploy_token');
    await tester.tap(find.byKey(const Key('secret-set-continue')));
    await tester.pump();
    expect(host.sets.single.provider, 'system');
    expect(host.sets.single.name, 'deploy_token');
    expect(find.text('Waiting for the host'), findsOneWidget);
    // The host kept it; the table is read again when the run ends.
    host.rows = const [SecretRow(provider: 'system', name: 'deploy_token', kept: 'stored')];
    host.sets.single.complete();
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('secret-set-done')), findsOneWidget);
    await tester.tap(find.byKey(const Key('secret-set-close')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('secret-name system/deploy_token')), findsOneWidget);
    expect(controller.rows.single.name, 'deploy_token');
  });

  testWidgets('a set that is cancelled sets nothing, and one the host refuses says why', (tester) async {
    final (_, host) = await _page(tester, const []);
    await tester.tap(find.byKey(const Key('secret-set')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('secret-set-name')), 'x');
    await tester.tap(find.byKey(const Key('secret-set-continue')));
    await tester.pump();
    host.sets.single.fail('the person declined');
    await tester.pump();
    expect(find.text('It was not set: the person declined'), findsOneWidget);
    await tester.tap(find.byKey(const Key('secret-set-close')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('secret-set')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('secret-set-name')), 'y');
    await tester.tap(find.byKey(const Key('secret-set-continue')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('secret-set-cancel')));
    await tester.pumpAndSettle();
    expect(host.sets.last.cancelled, isTrue);
    expect(find.byKey(const Key('secret-set-dialog')), findsNothing);
  });

  test('a row is read from what the host lists, and one that is not a row is left out', () {
    final row = SecretRow.parse({'provider': 'form', 'name': 'a', 'kept': 'process', 'session': null, 'expires_in': 5})!;
    expect((row.key, row.expiresIn, row.session), ('form/a', 5, null));
    expect(SecretRow.parse({'provider': 'form'}), isNull);
    expect(SecretRow.parse('x'), isNull);
    expect(secretKeptText(row), '5 s left');
  });
}
