import 'dart:convert';
import 'dart:io';

import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/forms/schema_fields.dart';
import 'package:canoa/profile/profile_editor.dart';
import 'package:canoa/profile/profile_items.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

/// The network item as the host lists it (`profile_items/list`), written by canoa-host's own test from what the build answers.
Object? _networkAnswer() => jsonDecode(File('test/fixtures/profile_items_network.json').readAsStringSync());

/// What a stand-in host answers to `network/client_certificates`: one certificate that is good for years, one that has expired.
Map<String, Object?> _certificatesAnswer() => {
  'certificates': [
    {
      'thumbprint': 'a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90',
      'subject': 'CN=Jose Alacan, O=Boriken Dev',
      'issuer': 'CN=Boriken Dev Staff CA',
      'not_after': 1893456000, // 2030-01-01
      'source': 'Windows certificate store',
    },
    {
      'thumbprint': '0f9e8d7c6b5a49382716a5b4c3d2e1f00f9e8d7c6b5a49382716a5b4c3d2e1f0',
      'subject': 'CN=Build service',
      'issuer': 'CN=Boriken Dev Staff CA',
      'not_after': 1577836800, // 2020-01-01
      'source': 'PKCS#11 token',
    },
  ],
};

const _good = 'a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90';
const _old = '0f9e8d7c6b5a49382716a5b4c3d2e1f00f9e8d7c6b5a49382716a5b4c3d2e1f0';

String _field(String suffix) => 'item-network-field-client_certificate_hosts-$suffix';

Preset _held(List<Map<String, Object?>> hosts) => Preset(items: {'network': {'client_certificate_hosts': hosts}});

Future<List<(String, Preset)>> _pump(
  WidgetTester tester,
  Preset preset, {
  Object? certificates,
  Size size = const Size(900, 2600),
}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final catalog = ProfileItemCatalog()..adopt(_networkAnswer());
  final listed = ClientCertificateChoices()..adopt(certificates ?? _certificatesAnswer(), now: DateTime.utc(2026, 11, 19));
  final saved = <(String, Preset)>[];
  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: SingleChildScrollView(
          child: ProfileEditor(
            original: 'Work',
            preset: preset,
            taken: const {'default', 'Work'},
            sources: const ['Claude'],
            loadouts: const [],
            catalog: catalog,
            certificates: listed,
            onSave: (name, made) async {
              saved.add((name, made));
              return null;
            },
            onCancel: () {},
          ),
        ),
      ),
    ),
  );
  await tester.pump();
  return saved;
}

Future<void> _save(WidgetTester tester) async {
  await tester.ensureVisible(find.byKey(const Key('profile-edit-save')));
  await tester.tap(find.byKey(const Key('profile-edit-save')));
  await tester.pump();
}

Future<void> _choose(WidgetTester tester, String control, String label) async {
  await tester.ensureVisible(find.byKey(Key(control)));
  await tester.tap(find.byKey(Key(control)));
  await tester.pumpAndSettle();
  await tester.tap(find.text(label).last);
  await tester.pumpAndSettle();
}

List<Object?> _hosts(Preset preset) => preset.items['network']!['client_certificate_hosts']! as List<Object?>;

void main() {
  test('the host\'s own answer asks for rows whose certificate is a pick, and the catalog knows to fetch the list', () {
    final catalog = ProfileItemCatalog()..adopt(_networkAnswer());
    final proxy = catalog.byId('network')!;
    expect(proxy.asksForClientCertificate, isTrue);
    expect(catalog.asksForClientCertificate, isTrue);
    expect(ProfileItemCatalog().asksForClientCertificate, isFalse);
    expect(formHasWidget(proxy.form, 'client_certificate'), isTrue);
    expect(formHasWidget(proxy.form, 'something_else'), isFalse);
  });

  test('the choices are the keystores\' own, then each certificate by thumbprint; a refusal leaves only the first', () {
    final listed = ClientCertificateChoices();
    expect([for (final each in listed.choices) each.value], ['system'], reason: 'nothing asked yet');
    listed.adopt(_certificatesAnswer(), now: DateTime.utc(2026, 11, 19));
    expect([for (final each in listed.choices) each.value], ['system', _good, _old]);
    expect(listed.choices.first.label, 'System (default)');
    final good = listed.choices[1];
    expect(good.label, 'CN=Jose Alacan, O=Boriken Dev');
    expect(good.detail, contains('CN=Boriken Dev Staff CA'));
    expect(good.detail, contains('expires 2030-01-01'));
    expect(good.detail, contains('Windows certificate store'));
    expect(good.detail, contains(_good.substring(0, 16)));
    expect(listed.choices[2].detail, contains('expired 2020-01-01'));
    listed.failed();
    expect([for (final each in listed.choices) each.value], ['system']);
  });

  test('rows are kept in their order, a row nobody filled is dropped and a row without its required member is a problem', () {
    final proxy = (ProfileItemCatalog()..adopt(_networkAnswer())).byId('network')!;
    final values = {
      'client_certificate_hosts': [
        {'pattern': 'a[.]example', 'identity': _good},
        {'identity': 'system'},
        {'pattern': '.*', 'identity': 'system'},
      ],
    };
    expect(collectSchemaValues(proxy.schema, proxy.form, values), {
      'client_certificate_hosts': [
        {'pattern': 'a[.]example', 'identity': _good},
        {'pattern': '.*', 'identity': 'system'},
      ],
    });
    expect(schemaProblem(proxy.schema, proxy.form, values), isNull, reason: 'a row with nothing but its default is not an entry');
    values['client_certificate_hosts'] = [
      {'identity': _good},
    ];
    expect(schemaProblem(proxy.schema, proxy.form, values), 'Hosts that get a client certificate: Hosts is needed');
  });

  testWidgets('the network item draws a row for each entry, and a pattern edited and a certificate picked read back as pattern and identity', (tester) async {
    final saved = await _pump(
      tester,
      _held([
        {'pattern': 'api[.]example[.]com', 'identity': 'system'},
        {'pattern': '.*', 'identity': 'system'},
      ]),
    );
    expect(find.byKey(Key(_field('row-0'))), findsOneWidget);
    expect(find.byKey(Key(_field('row-1'))), findsOneWidget);
    expect(find.byKey(Key(_field('row-2'))), findsNothing);
    expect(tester.widget<TextField>(find.byKey(Key(_field('0-field-pattern')))).controller!.text, 'api[.]example[.]com');
    expect(find.text('System (default)'), findsNWidgets(2), reason: 'each row shows its identity as the pick');

    await tester.enterText(find.byKey(Key(_field('0-field-pattern'))), '(api|auth)[.]example[.]com');
    await _choose(tester, _field('1-field-identity'), 'CN=Jose Alacan, O=Boriken Dev');
    await _save(tester);
    expect(_hosts(saved.single.$2), [
      {'pattern': '(api|auth)[.]example[.]com', 'identity': 'system'},
      {'pattern': '.*', 'identity': _good},
    ]);
    final written = CanoaConfig.parse('').withPresets({'Work': saved.single.$2}).encode();
    final read = CanoaConfig.parse(written).presets['Work']!;
    expect(_hosts(read), _hosts(saved.single.$2), reason: 'the rows round-trip through canoa.cfg');
  });

  testWidgets('a row is added with the keystores\' own choice, moved, and removed, and the order is the one saved', (tester) async {
    final saved = await _pump(tester, _held([
      {'pattern': 'first', 'identity': 'system'},
      {'pattern': 'second', 'identity': _good},
    ]));
    await tester.ensureVisible(find.byKey(Key(_field('add'))));
    await tester.tap(find.byKey(Key(_field('add'))));
    await tester.pump();
    expect(find.byKey(Key(_field('row-2'))), findsOneWidget);
    await tester.enterText(find.byKey(Key(_field('2-field-pattern'))), 'third');

    await tester.ensureVisible(find.byKey(Key(_field('up-1'))));
    await tester.tap(find.byKey(Key(_field('up-1'))));
    await tester.pump();
    expect(tester.widget<TextField>(find.byKey(Key(_field('0-field-pattern')))).controller!.text, 'second', reason: 'the row moved with what it held');
    expect(tester.widget<IconButton>(find.byKey(Key(_field('up-0')))).onPressed, isNull, reason: 'nothing above the first');
    expect(tester.widget<IconButton>(find.byKey(Key(_field('down-2')))).onPressed, isNull, reason: 'nothing below the last');
    await _save(tester);
    expect(_hosts(saved.single.$2), [
      {'pattern': 'second', 'identity': _good},
      {'pattern': 'first', 'identity': 'system'},
      {'pattern': 'third', 'identity': 'system'},
    ]);

    await tester.ensureVisible(find.byKey(Key(_field('remove-0'))));
    await tester.tap(find.byKey(Key(_field('remove-0'))));
    await tester.pump();
    expect(find.byKey(Key(_field('row-2'))), findsNothing);
    await _save(tester);
    expect(_hosts(saved.last.$2), [
      {'pattern': 'first', 'identity': 'system'},
      {'pattern': 'third', 'identity': 'system'},
    ]);
    await tester.tap(find.byKey(Key(_field('remove-0'))));
    await tester.tap(find.byKey(Key(_field('remove-0'))));
    await tester.pump();
    await _save(tester);
    expect(saved.last.$2.items, isEmpty, reason: 'a list with no entries leaves the item unset');
  });

  testWidgets('a thumbprint the host no longer lists stays chosen and says it is not available; a refusal leaves only System', (tester) async {
    const gone = 'ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100';
    final saved = await _pump(tester, _held([
      {'pattern': 'corp', 'identity': gone},
    ]));
    expect(find.text('$gone (not available)'), findsOneWidget);
    await _save(tester);
    expect(_hosts(saved.single.$2), [
      {'pattern': 'corp', 'identity': gone},
    ], reason: 'saving does not swap it for another');

    await _pump(tester, _held([
      {'pattern': 'corp', 'identity': 'system'},
    ]), certificates: 'refused');
    await tester.ensureVisible(find.byKey(Key(_field('0-field-identity'))));
    await tester.tap(find.byKey(Key(_field('0-field-identity'))));
    await tester.pumpAndSettle();
    expect(find.text('CN=Jose Alacan, O=Boriken Dev'), findsNothing);
    expect(find.text('System (default)'), findsWidgets, reason: 'the only choice there is');
  });

  testWidgets('the network item\'s rows, drawn from the host\'s own answer', (tester) async {
    await _pump(
      tester,
      _held([
        {'pattern': 'api[.]example[.]com', 'identity': _good},
        {'pattern': '(corp|intranet)[.]example[.]com', 'identity': 'system'},
        {'pattern': '.*', 'identity': _old},
      ]),
      size: const Size(900, 1700),
    );
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/profile_certificate_rows_desktop.png'));
  });
}
