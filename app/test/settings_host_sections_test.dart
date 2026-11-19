// Settings' sections a host's providers offer (the `settings_section` kind), drawn by the generic form renderer: a stand-in
// extension's section appears as a section of its own under its heading, another's as a group inside General, the values round
// trip through the host, a secret is never shown, and taking the extension away takes both away. The host is a stand-in.
import 'dart:io';

import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/settings/host_sections.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/settings_fixture.dart';
import 'fixtures/store_fixtures.dart' show PlatformTolerant;

const _region = {
  'id': 'region',
  'title': 'Region',
  'help': 'Where the extension runs.',
  'icon': 'cloud',
  'group': 'Regions',
  'keywords': 'tier cloud',
  'origin': 'regions',
  'stored': 'config',
  'schema': {
    'type': 'object',
    'required': ['name'],
    'properties': {
      'name': {'type': 'string', 'title': 'Name', 'x-section': 'where'},
      'tier': {
        'type': 'string',
        'title': 'Tier',
        'enum': ['standard', 'fast'],
        'default': 'standard',
        'x-section': 'where',
      },
      'token': {'type': 'string', 'title': 'Token', 'x-sensitive': true, 'x-section': 'where'},
    },
  },
  'form': {
    'sections': [
      {'id': 'where', 'label': 'Where'},
    ],
    'fields': {
      'name': {'placeholder': 'eu-west'},
    },
  },
};

const _cache = {
  'id': 'cache',
  'title': 'Cache',
  'under': 'general',
  'keywords': 'entries memory',
  'origin': 'regions',
  'stored': 'provider',
  'schema': {
    'type': 'object',
    'properties': {
      'entries': {'type': 'integer', 'title': 'Entries', 'description': 'How many are kept'},
    },
  },
  'form': <String, Object?>{},
};

/// The host's three methods over what each section holds; the secret is kept apart and only named, as the host does.
final class _Host {
  final held = <String, Map<String, Object?>>{};
  final secrets = <String, Map<String, Object?>>{};
  List<Object?> listed = [_region, _cache];
  final calls = <String>[];

  Future<Object?> call(String method, Map<String, Object?> params) async {
    calls.add(method);
    switch (method) {
      case 'settings/sections':
        return {'sections': listed};
      case 'settings/section_get':
        final id = params['id']! as String;
        return {'id': id, 'values': held[id] ?? {}, 'secrets_set': (secrets[id] ?? {}).keys.toList()};
      case 'settings/section_set':
        final id = params['id']! as String;
        if (!listed.any((each) => (each! as Map)['id'] == id)) throw const ControlFailed(-32003, 'this host has no settings section');
        final values = {...(params['values']! as Map).cast<String, Object?>()};
        final token = values.remove('token');
        if (token is String && token.isNotEmpty) secrets[id] = {'token': token};
        if ((params['clear'] as List?)?.contains('token') ?? false) secrets.remove(id);
        held[id] = values;
        return {'id': id, 'saved': true};
    }
    throw const ControlFailed(-32601, 'no method');
  }
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/settings_host_sections_test.dart')));

  testWidgets('a provider\'s section is a section of its own under its heading, and its form saves through the host', (tester) async {
    final host = _Host();
    final sections = HostSections(host.call);
    await sections.load();
    await openSettings(tester, hostSections: sections);

    expect(find.byKey(const Key('settings-nav-region')), findsOneWidget);
    expect(find.text('Regions'), findsOneWidget, reason: 'its group is the heading it names');
    // Settings' own About stays last, after the provider's section.
    expect(tester.getTopLeft(find.byKey(const Key('settings-nav-region'))).dy, lessThan(tester.getTopLeft(find.byKey(const Key('settings-nav-about'))).dy));

    await goTo(tester, 'region');
    expect(find.text('Where the extension runs.'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('section-region-field-name')), 'eu-west');
    await tester.enterText(find.byKey(const Key('section-region-field-token')), 's3cret');
    await tester.tap(find.byKey(const Key('section-region-save')));
    await tester.pumpAndSettle();
    expect(host.held['region'], {'name': 'eu-west'});
    expect(host.secrets['region'], {'token': 's3cret'});
    expect(find.byKey(const Key('section-region-saved')), findsOneWidget);

    // Opened again, the values come back and the secret is said to be set without being shown.
    await goTo(tester, 'general');
    await goTo(tester, 'region');
    expect(tester.widget<TextField>(find.byKey(const Key('section-region-field-name'))).controller?.text, 'eu-west');
    expect(find.text('s3cret'), findsNothing);
    expect(find.text('Set. Type to replace'), findsOneWidget);
  });

  testWidgets('a section that names a required member is not saved without it', (tester) async {
    final host = _Host();
    final sections = HostSections(host.call);
    await sections.load();
    await openSettings(tester, hostSections: sections, section: 'region');
    await tester.tap(find.byKey(const Key('section-region-save')));
    await tester.pumpAndSettle();
    expect(find.text('Name is needed'), findsOneWidget);
    expect(host.held, isEmpty);
  });

  testWidgets('another provider\'s section is a group inside General and saves through its provider', (tester) async {
    final host = _Host();
    final sections = HostSections(host.call);
    await sections.load();
    await openSettings(tester, hostSections: sections, section: 'general');

    expect(find.byKey(const Key('settings-nav-cache')), findsNothing, reason: 'a group is not a section of the list');
    expect(find.byKey(const Key('settings-group-cache')), findsOneWidget);
    expect(find.text('Cache'), findsOneWidget);
    await tester.ensureVisible(find.byKey(const Key('section-cache-field-entries')));
    await tester.enterText(find.byKey(const Key('section-cache-field-entries')), '40');
    await tester.tap(find.byKey(const Key('section-cache-save')));
    await tester.pumpAndSettle();
    expect(host.held['cache'], {'entries': 40});
  });

  testWidgets('the search finds a section by its group\'s words, and a group by the section that holds it', (tester) async {
    final host = _Host();
    final sections = HostSections(host.call);
    await sections.load();
    await openSettings(tester, hostSections: sections);
    await tester.enterText(find.byKey(const Key('settings-search')), 'cloud');
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('settings-nav-region')), findsOneWidget);
    expect(find.byKey(const Key('settings-nav-general')), findsNothing);
    await tester.enterText(find.byKey(const Key('settings-search')), 'entries');
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('settings-nav-general')), findsOneWidget, reason: 'General holds the Cache group');
    expect(find.byKey(const Key('settings-nav-region')), findsNothing);
  });

  testWidgets('taking the extension away takes its sections and its group away', (tester) async {
    final host = _Host();
    final sections = HostSections(host.call);
    await sections.load();
    await openSettings(tester, hostSections: sections, section: 'general');
    expect(find.byKey(const Key('settings-nav-region')), findsOneWidget);
    expect(find.byKey(const Key('settings-group-cache')), findsOneWidget);
    host.listed = [];
    await sections.load();
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('settings-nav-region')), findsNothing);
    expect(find.byKey(const Key('settings-group-cache')), findsNothing);
    expect(find.byKey(const Key('settings-nav-about')), findsOneWidget);
  });

  testWidgets('a section whose group names no section of the app is a section of its own, and one that takes an id the app uses is left out', (tester) async {
    final host = _Host()
      ..listed = [
        {..._cache, 'under': 'nothing_like_it', 'title': 'Cache'},
        {..._region, 'id': 'general', 'title': 'Impostor'},
      ];
    final sections = HostSections(host.call);
    await sections.load();
    await openSettings(tester, hostSections: sections);
    expect(find.byKey(const Key('settings-nav-cache')), findsOneWidget);
    expect(find.text('Impostor'), findsNothing);
    expect(find.byKey(const Key('settings-nav-general')), findsOneWidget);
  });

  testWidgets('a host that refuses the values says why in the section', (tester) async {
    final host = _Host()..listed = [_region];
    final sections = HostSections((method, params) async {
      if (method == 'settings/section_set') throw const ControlFailed(-32002, 'region: tier is fast or standard');
      return host.call(method, params);
    });
    await sections.load();
    await openSettings(tester, hostSections: sections, section: 'region');
    await tester.enterText(find.byKey(const Key('section-region-field-name')), 'eu');
    await tester.tap(find.byKey(const Key('section-region-save')));
    await tester.pumpAndSettle();
    expect(find.text('region: tier is fast or standard'), findsOneWidget);
  });

  for (final (label, size) in [('desktop', const Size(1280, 800)), ('phone', const Size(390, 844))]) {
    testWidgets('a provider\'s section at $label width', (tester) async {
      final host = _Host();
      host.held['region'] = {'name': 'eu-west', 'tier': 'fast'};
      host.secrets['region'] = {'token': 'x'};
      final sections = HostSections(host.call);
      await sections.load();
      await openSettings(tester, hostSections: sections, section: 'region', size: size);
      await expectLater(find.byType(Scaffold).first, matchesGoldenFile('goldens/settings_host_section_$label.png'));
    });
    testWidgets('a group a provider puts inside General at $label width', (tester) async {
      final host = _Host();
      final sections = HostSections(host.call);
      await sections.load();
      await openSettings(tester, hostSections: sections, section: 'general', size: size);
      await tester.dragUntilVisible(find.byKey(const Key('settings-group-cache')), find.byKey(const Key('settings-section-general')), const Offset(0, -300));
      await tester.pumpAndSettle();
      await expectLater(find.byType(Scaffold).first, matchesGoldenFile('goldens/settings_host_group_$label.png'));
    });
  }
}
