// The Store's Extensions type: the extensions the catalogs list, read from the host's index, installed through the host's plan and
// confirm, removed, and managed in Settings afterwards. The host is scripted (test/fixtures/store_link.dart): its refusals and its
// writes of canoa.cfg are what a real host answers. Names, URLs and digests are made up.
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/served/host_files.dart';
import 'package:canoa/settings/extension_settings.dart';
import 'package:canoa/store/store_extension.dart' show ExtensionBody;
import 'package:canoa/store/store_extension_catalog.dart';
import 'package:canoa/store/store_extensions.dart';
import 'package:canoa/store/store_view.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_link.dart';

/// What `extension/catalogs` answers for one catalog, `team`, that lists two extensions (and refuses one).
Map<String, Object?> _index() => {
  'schema': 1,
  'sources': [
    {'id': 'extensions:team', 'kind': 'extension_catalog', 'label': 'team', 'spec': 'https://example.org/team.git@main#extensions.toml', 'entries': 3},
  ],
  'entries': [
    {
      'id': 'team:extension:speech',
      'kind': 'extension',
      'catalog': 'team',
      'name': 'speech',
      'summary': 'Speech engines',
      'tags': ['speech'],
      'origin': {'kind': 'git', 'location': 'https://example.org/team.git', 'ref': 'main', 'path': 'ext/speech'},
      'extension': {
        'provider': 'example_speech',
        'pin': 'sha256:${'ab' * 32}',
        'load': 'speech.dll',
        'needs': {'kind': 'store_extension', 'version': '1', 'platforms': ['windows-x86_64']},
      },
    },
    {
      'id': 'team:extension:lib',
      'kind': 'extension',
      'catalog': 'team',
      'name': 'lib',
      'summary': null,
      'origin': {'kind': 'url', 'location': 'https://example.org/files/lib.so', 'ref': null, 'path': null},
      'extension': {'provider': 'example_lib', 'sha256': 'cd' * 32, 'needs': <String, Object?>{}},
    },
    {'id': 'team:extension:broken', 'kind': 'extension', 'catalog': 'team', 'name': 'broken', 'problem': 'The catalog\'s entry was refused (unpinned)'},
  ],
};

StoreExtensions _store(FakeStoreHost host, {bool Function()? mayInstall, Future<bool> Function()? restart, Future<void> Function()? saved}) {
  final shared = host.client();
  return StoreExtensions(
    control: () async => shared,
    open: () async => host.client(),
    mayInstall: mayInstall,
    restart: restart,
    saved: saved,
  );
}

void main() {
  test('the host\'s answer gives each extensions catalog the extensions it lists, and leaves out one it refused', () {
    final catalogs = extensionCatalogsFromIndex(_index());
    final team = catalogs.single;
    expect(team.label, 'team');
    expect(team.extensions.keys, ['lib', 'speech']);
    final speech = team.extensions['speech']!;
    expect(speech.provider, 'example_speech');
    expect(speech.pin, 'sha256:${'ab' * 32}');
    expect(speech.sha256, isNull);
    expect(speech.load, 'speech.dll');
    expect(speech.kind, 'store_extension');
    expect(speech.version, '1');
    expect(speech.platforms, ['windows-x86_64']);
    expect(speech.origin['kind'], 'git');
    expect(speech.origin['path'], 'ext/speech');
    expect(speech.about['summary'], 'Speech engines');
    final lib = team.extensions['lib']!;
    expect(lib.sha256, 'cd' * 32);
    expect(lib.pin, isNull);
    expect(lib.origin['kind'], 'url');
  });

  test('the Extensions type lists what the catalogs list, and which of them canoa.cfg names', () async {
    final host = FakeStoreHost([])
      ..catalogIndex = _index()
      ..hostTable = {
        'extensions': {
          'speech': {'library': '/home/extensions/speech/speech.dll', 'provider': 'example_speech', 'pin': 'sha256:${'ab' * 32}'},
        },
      };
    final extensions = _store(host);
    final type = extensions.types.firstWhere((each) => each.id == extensionsTypeId);
    expect(type.items(), isEmpty, reason: 'nothing is asked until the page opens');
    await type.load!();
    final items = type.items();
    expect(items.map((each) => '${each.catalog}/${each.name}'), ['team/lib', 'team/speech']);
    expect(items.map((each) => each.installed), [false, true]);
    expect(items.last.summary, 'Speech engines');
    expect(items.last.tags, ['speech']);
    expect(items.first.install, isNotNull);
    expect(items.first.remove, isNull, reason: 'nothing to remove of what is not installed');
    expect(items.last.remove, isNotNull);
    extensions.dispose();
  });

  test('a key that may not change the host\'s settings is shown the extensions and no way to install them', () async {
    final host = FakeStoreHost([])
      ..catalogIndex = _index()
      ..failures['config/get'] = ControlFailed(-32001, 'this key does not hold hosting');
    final extensions = _store(host, mayInstall: () => false);
    final type = extensions.types.firstWhere((each) => each.id == extensionsTypeId);
    await type.load!();
    expect(type.items(), hasLength(2));
    expect(type.items().every((each) => each.install == null && each.remove == null && !each.installed), isTrue);
    extensions.dispose();
  });

  testWidgets('install shows the host\'s plan, installs once it is confirmed, says the host restarts, and then offers Remove', (tester) async {
    final host = FakeStoreHost([])..catalogIndex = _index();
    var saved = 0, restarted = 0;
    final extensions = _store(host, saved: () async => saved++, restart: () async => ++restarted > 0);
    final type = extensions.types.firstWhere((each) => each.id == extensionsTypeId);
    await tester.binding.setSurfaceSize(const Size(900, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: ExtensionBody(type: type, view: StoreView(extensionsTypeId)))));
    await _settle(tester);
    expect(find.byKey(const Key('store-entry-team-speech')), findsOneWidget);
    expect(find.byKey(const Key('store-entry-team-lib')), findsOneWidget);
    expect(find.byKey(const Key('store-entry-remove-team-speech')), findsNothing);

    await tester.tap(find.byKey(const Key('store-entry-install-team-speech')));
    await _settle(tester);
    expect(find.byKey(const Key('plan-dialog')), findsOneWidget, reason: 'the host\'s plan, to confirm');
    expect(find.textContaining('runs with your access'), findsOneWidget, reason: 'the notice says it is code');
    expect(find.textContaining('[host.extensions.speech]'), findsOneWidget);
    expect(host.params['extension/plan']!.single, {'catalog': 'team', 'name': 'speech'}, reason: 'the client names the extension, never a source or a digest');
    expect(host.params['extension/apply'], isNull, reason: 'nothing is fetched before the person confirms');

    await tester.tap(find.byKey(const Key('plan-confirm')));
    await _settle(tester);
    expect(host.params['extension/apply'], hasLength(1));
    expect((host.params['extension/apply']!.single['plan'] as Map)['token'], 'token-speech', reason: 'the plan comes back as the host made it');
    expect(saved, 1, reason: 'canoa.cfg changed through the host: the app reads it again');
    expect(find.byKey(const Key('extension-store-restart-dialog')), findsOneWidget);
    expect(find.textContaining('reads its extensions when it starts'), findsOneWidget);
    await tester.tap(find.byKey(const Key('extension-store-restart-now')));
    await _settle(tester);
    expect(restarted, 1);

    expect(find.byKey(const Key('store-entry-remove-team-speech')), findsOneWidget, reason: 'it is named in canoa.cfg now');
    expect(find.byKey(const Key('store-entry-remove-team-lib')), findsNothing);
    extensions.dispose();
  });

  testWidgets('a plan the host refuses is shown in the host\'s words and nothing is applied', (tester) async {
    final host = FakeStoreHost([])
      ..catalogIndex = _index()
      ..extensionPlanRefusal = 'it does not run on windows-aarch64';
    final extensions = _store(host);
    final type = extensions.types.firstWhere((each) => each.id == extensionsTypeId);
    await tester.binding.setSurfaceSize(const Size(900, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: ExtensionBody(type: type, view: StoreView(extensionsTypeId)))));
    await _settle(tester);
    await tester.tap(find.byKey(const Key('store-entry-install-team-speech')));
    await _settle(tester);
    expect(find.byKey(const Key('extension-store-error')), findsOneWidget);
    expect(find.text('it does not run on windows-aarch64'), findsOneWidget);
    expect(host.params['extension/apply'], isNull);
    extensions.dispose();
  });

  testWidgets('an install the host refuses after the person confirmed (a digest that differs) leaves the Store as it was', (tester) async {
    final host = FakeStoreHost([])
      ..catalogIndex = _index()
      ..extensionApplyRefusal = 'the files the source holds do not have the tree digest the catalog gave';
    final extensions = _store(host);
    final type = extensions.types.firstWhere((each) => each.id == extensionsTypeId);
    await tester.binding.setSurfaceSize(const Size(900, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: ExtensionBody(type: type, view: StoreView(extensionsTypeId)))));
    await _settle(tester);
    await tester.tap(find.byKey(const Key('store-entry-install-team-speech')));
    await _settle(tester);
    await tester.tap(find.byKey(const Key('plan-confirm')));
    await _settle(tester);
    expect(find.byKey(const Key('extension-store-error')), findsOneWidget);
    expect(find.textContaining('do not have the tree digest'), findsOneWidget);
    expect(find.byKey(const Key('extension-store-restart-dialog')), findsNothing, reason: 'nothing was installed');
    expect(host.hostTable, isNull, reason: 'and canoa.cfg is as it was');
    expect(find.byKey(const Key('store-entry-remove-team-speech')), findsNothing);
    extensions.dispose();
  });

  testWidgets('Remove asks first, takes the extension out through the host and says the host restarts', (tester) async {
    final host = FakeStoreHost([])
      ..catalogIndex = _index()
      ..hostTable = {
        'extensions': {
          'speech': {'library': '/home/extensions/speech/speech.dll', 'provider': 'example_speech', 'pin': 'sha256:${'ab' * 32}'},
        },
      };
    final extensions = _store(host);
    final type = extensions.types.firstWhere((each) => each.id == extensionsTypeId);
    await tester.binding.setSurfaceSize(const Size(900, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: ExtensionBody(type: type, view: StoreView(extensionsTypeId)))));
    await _settle(tester);
    await tester.tap(find.byKey(const Key('store-entry-remove-team-speech')));
    await _settle(tester);
    expect(find.byKey(const Key('extension-store-remove-dialog')), findsOneWidget);
    expect(host.params['extension/remove'], isNull, reason: 'nothing is removed before the person confirms');
    await tester.tap(find.byKey(const Key('extension-store-remove-confirm')));
    await _settle(tester);
    expect(host.params['extension/remove']!.single, {'name': 'speech'});
    expect(find.byKey(const Key('extension-store-restart-dialog')), findsOneWidget);
    await tester.tap(find.byKey(const Key('extension-store-restart-later')));
    await _settle(tester);
    expect(find.byKey(const Key('store-entry-remove-team-speech')), findsNothing, reason: 'it is no longer installed');
    expect(host.hostTable, isNull);
    extensions.dispose();
  });

  group('Settings keeps managing what is installed', () {
    HostFiles files(FakeStoreHost host) => HostFiles(HostControl.dialing(() async => host.open()));

    test('an extension the Store installed says so, and removing it goes through the host, which takes its files too', () async {
      final host = FakeStoreHost([])
        ..hostTable = {
          'extensions': {
            'speech': {'library': '/home/extensions/speech/speech.dll', 'provider': 'example_speech', 'sha256': 'ab' * 32},
            'by-hand': {'library': '/opt/by-hand.dll', 'provider': 'x'},
          },
        };
      final settings = ExtensionSettings(files: files(host));
      await settings.load();
      expect(settings.entries.map((each) => (each.name, each.fromStore)), [('speech', true), ('by-hand', false)]);

      expect(await settings.remove('speech'), isNull);
      expect(host.params['extension/remove']!.single, {'name': 'speech'});
      expect(host.params['config/set'], isNull, reason: 'the host wrote canoa.cfg itself');
      expect(settings.restartPending, isTrue);
      expect(settings.entries.map((each) => each.name), ['by-hand']);

      expect(await settings.remove('by-hand'), isNull);
      expect(host.params['extension/remove'], hasLength(1), reason: 'one named by hand goes the way it always did');
      expect(host.params['config/set'], hasLength(1));
      settings.dispose();
    });

    test('pointing an installed extension at another library drops the digest the Store kept, which no longer covers it', () async {
      final host = FakeStoreHost([])
        ..hostTable = {
          'extensions': {
            'speech': {
              'library': '/home/extensions/speech/speech.dll',
              'provider': 'example_speech',
              'pin': 'sha256:${'ab' * 32}',
              'root': '/home/extensions/speech',
            },
          },
        };
      final settings = ExtensionSettings(files: files(host));
      await settings.load();
      expect(await settings.save(name: 'speech', library: '/home/extensions/speech/speech.dll', provider: 'example_speech', replacing: 'speech'), isNull);
      expect((host.hostTable!['extensions'] as Map)['speech'], containsPair('pin', 'sha256:${'ab' * 32}'), reason: 'the same library: still covered');
      expect(await settings.save(name: 'speech', library: '/elsewhere/speech.dll', provider: 'example_speech', replacing: 'speech'), isNull);
      expect((host.hostTable!['extensions'] as Map)['speech'], {'library': '/elsewhere/speech.dll', 'provider': 'example_speech'});
      settings.dispose();
    });
  });
}

/// Lets the scripted host answer and the page draw it: a few short frames, since the banner never settles.
Future<void> _settle(WidgetTester tester) async {
  for (var i = 0; i < 20; i++) {
    await tester.pump(const Duration(milliseconds: 50));
  }
}
