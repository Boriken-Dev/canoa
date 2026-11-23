// Settings' Extensions page over a scripted host: the extensions of the host's canoa.cfg are listed, added, changed and removed
// through its `[host]` table (read again before each write, checked by the host first), and the host is restarted to load them.
// Names, paths and providers are made up.
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/served/host_files.dart';
import 'package:canoa/session_store.dart';
import 'package:canoa/settings/extension_settings.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/app_fixture.dart';
import 'fixtures/store_link.dart';

HostFiles _files(FakeStoreHost host) => HostFiles(HostControl.dialing(() async => host.open()));

ExtensionSettings _settings(FakeStoreHost host, {Future<bool> Function()? restart, Future<void> Function()? saved}) =>
    ExtensionSettings(files: _files(host), restart: restart, saved: saved);

void main() {
  appTests();

  group('the extensions of the host\'s canoa.cfg', () {
    test('are listed from [host], with the types each added', () async {
      final host = FakeStoreHost([])
        ..hostTable = {
          'agents': {'echo': {'program': 'echo'}},
          'extensions': {
            'sample': {'library': '/ext/sample.dll', 'provider': 'sample_extension'},
            'other': {'source': 'other.py', 'provider': 'other_extension'},
          },
        };
      final settings = ExtensionSettings(
        files: _files(host),
        types: () async => {'sample': ['Echoes']},
      );
      await settings.load();
      expect(settings.entries.map((entry) => entry.name), ['sample', 'other']);
      expect(settings.entries.first.where, '/ext/sample.dll');
      expect(settings.entries.last.source, 'other.py');
      expect(settings.addedBy('sample'), ['Echoes']);
      expect(settings.addedBy('other'), isEmpty);
      expect(settings.problem, isNull);
      settings.dispose();
    });

    test('a key the host does not let change [host] has no list and says so', () async {
      final host = FakeStoreHost([]);
      // The host answers a key without the grant with -32001.
      final settings = _settings(host);
      host.failures['config/get'] = ControlFailed(-32001, 'this key does not hold hosting');
      await settings.load();
      expect(settings.entries, isEmpty);
      expect(settings.problem, contains('may not change the host'));
      expect(settings.loaded, isFalse);
      settings.dispose();
    });

    test('add one, keeping the rest of [host], then change and remove it', () async {
      final host = FakeStoreHost([])
        ..hostTable = {
          'agents': {'echo': {'program': 'echo'}},
          'extensions': {
            'first': {'library': 'a.dll', 'provider': 'a'},
            'sample': {'library': 'sample.dll', 'provider': 'sample_extension', 'source_settings': {'x': 1}},
          },
        };
      var saved = 0;
      final settings = _settings(host, saved: () async => saved++);
      await settings.load();

      expect(await settings.save(name: 'third', library: 'c.dll', provider: 'c'), isNull);
      expect(settings.restartPending, isTrue);
      expect(saved, 1);
      expect(host.hostTable!['agents'], isNotNull, reason: 'what else [host] holds stays');
      expect((host.hostTable!['extensions'] as Map).keys, ['first', 'sample', 'third']);

      // A change renames it in place, moves it from a library to a source and keeps the keys the page does not know.
      expect(await settings.save(name: 'renamed', source: 'sample.py', provider: 'sample_extension', replacing: 'sample'), isNull);
      final extensions = host.hostTable!['extensions'] as Map;
      expect(extensions.keys, ['first', 'renamed', 'third'], reason: 'in the same place');
      expect(extensions['renamed'], {'source': 'sample.py', 'provider': 'sample_extension', 'source_settings': {'x': 1}});

      expect(await settings.remove('first'), isNull);
      expect(await settings.remove('third'), isNull);
      expect(await settings.remove('renamed'), isNull);
      expect(host.hostTable, {'agents': {'echo': {'program': 'echo'}}}, reason: 'the empty table goes');
      host.hostTable = {'extensions': {'only': {'library': 'o.dll', 'provider': 'o'}}};
      expect(await settings.remove('only'), isNull);
      expect(host.hostTable, isNull, reason: 'an empty [host] is removed');
      settings.dispose();
    });

    test('are read again before a write, so another page\'s change is kept', () async {
      final host = FakeStoreHost([])
        ..hostTable = {
          'extensions': {'one': {'library': 'one.dll', 'provider': 'one'}},
        };
      final settings = _settings(host);
      await settings.load();
      host.hostTable = {
        'origins': ['https://x.example'],
        'extensions': {'one': {'library': 'one.dll', 'provider': 'one'}, 'two': {'library': 'two.dll', 'provider': 'two'}},
      };
      expect(await settings.save(name: 'three', library: 'three.dll', provider: 'three'), isNull);
      expect(host.hostTable!['origins'], ['https://x.example']);
      expect((host.hostTable!['extensions'] as Map).keys, ['one', 'two', 'three']);
      settings.dispose();
    });

    test('a refusal of the host comes back in its words and nothing is written', () async {
      final host = FakeStoreHost([])..hostTable = {'agents': {'echo': {'program': 'echo'}}};
      final settings = _settings(host);
      await settings.load();
      host.checkRefusal = 'that would leave canoa.cfg refused: host extension sample: no library or source';
      expect(await settings.save(name: 'sample', library: 'x.dll', provider: 'p'), contains('canoa.cfg refused'));
      expect(host.params['config/set'], isNull);
      expect(settings.restartPending, isFalse);

      host.checkRefusal = null;
      expect(await settings.save(name: '', library: 'x.dll', provider: 'p'), contains('needs a name'));
      expect(await settings.save(name: 'a', library: 'x.dll', provider: ''), contains('provider'));
      expect(await settings.save(name: 'a', provider: 'p'), contains('one of them'));
      expect(await settings.save(name: 'a', library: 'x.dll', source: 'y.py', provider: 'p'), contains('one of them'));
      await settings.save(name: 'taken', library: 'x.dll', provider: 'p');
      expect(await settings.save(name: 'taken', library: 'y.dll', provider: 'p'), contains('already'));
      expect(await settings.remove('nobody'), contains('no extension called nobody'));
      settings.dispose();
    });

    test('the host restarts once, and what is pending clears only when it did', () async {
      final host = FakeStoreHost([])..hostTable = {};
      var allowed = false, restarts = 0;
      final settings = _settings(host, restart: () async {
        restarts++;
        return allowed;
      });
      await settings.load();
      await settings.save(name: 'a', library: 'a.dll', provider: 'a');
      await settings.restartHost();
      expect(settings.restartPending, isTrue, reason: 'declined: still pending');
      allowed = true;
      await settings.restartHost();
      expect(restarts, 2);
      expect(settings.restartPending, isFalse);
      settings.dispose();
    });
  });

  group('the Extensions page', () {
    Future<(FakeStoreHost, ExtensionSettings)> open(WidgetTester tester, {Future<bool> Function()? restart}) async {
      final host = FakeStoreHost([])
        ..hostTable = {
          'extensions': {'sample': {'library': '/ext/sample.dll', 'provider': 'sample_extension'}},
        };
      final settings = ExtensionSettings(
        files: _files(host),
        types: () async => {'sample': ['Echoes']},
        restart: restart,
      );
      await tester.binding.setSurfaceSize(const Size(900, 900));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(child: ExtensionsSection(settings: settings)))));
      await tester.pumpAndSettle();
      addTearDown(settings.dispose);
      return (host, settings);
    }

    testWidgets('lists each extension with where it is loaded from and what it added', (tester) async {
      await open(tester);
      final row = find.byKey(const Key('extension-row-sample'));
      expect(row, findsOneWidget);
      expect(find.descendant(of: row, matching: find.textContaining('sample_extension from /ext/sample.dll')), findsOneWidget);
      expect(find.descendant(of: row, matching: find.textContaining('Adds Echoes')), findsOneWidget);
      expect(find.byKey(const Key('extensions-restart-note')), findsNothing);
    });

    testWidgets('Add writes it through the host and says the host restarts to load it, and Restart does that', (tester) async {
      var restarted = 0;
      final (host, _) = await open(tester, restart: () async {
        restarted++;
        return true;
      });
      await tester.tap(find.byKey(const Key('extension-add')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('extension-save')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('extension-dialog-problem')), findsOneWidget, reason: 'an empty form is refused before the host is asked');
      expect(host.params['config/check'], isNull);

      await tester.enterText(find.byKey(const Key('extension-name')), 'second');
      await tester.enterText(find.byKey(const Key('extension-library')), '/ext/second.dll');
      await tester.enterText(find.byKey(const Key('extension-provider')), 'second_extension');
      host.checkRefusal = 'that would leave canoa.cfg refused: nothing loads from there';
      await tester.tap(find.byKey(const Key('extension-save')));
      await tester.pumpAndSettle();
      expect(find.textContaining('nothing loads from there'), findsOneWidget, reason: 'the host\'s own words');
      expect(find.byKey(const Key('extension-dialog')), findsOneWidget);

      host.checkRefusal = null;
      await tester.tap(find.byKey(const Key('extension-save')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('extension-dialog')), findsNothing);
      expect((host.hostTable!['extensions'] as Map).keys, ['sample', 'second']);
      expect(find.byKey(const Key('extension-row-second')), findsOneWidget);
      expect(find.textContaining('The host restarts to load the change'), findsOneWidget);

      await tester.tap(find.byKey(const Key('extensions-restart')));
      await tester.pumpAndSettle();
      expect(restarted, 1);
      expect(find.byKey(const Key('extensions-restart-note')), findsNothing);
    });

    testWidgets('Edit fills the form with what is there, and Remove asks first', (tester) async {
      final (host, _) = await open(tester);
      await tester.tap(find.byKey(const Key('extension-edit-sample')));
      await tester.pumpAndSettle();
      expect(tester.widget<TextField>(find.byKey(const Key('extension-library'))).controller!.text, '/ext/sample.dll');
      await tester.enterText(find.byKey(const Key('extension-library')), '/ext/new.dll');
      await tester.tap(find.byKey(const Key('extension-save')));
      await tester.pumpAndSettle();
      expect(((host.hostTable!['extensions'] as Map)['sample'] as Map)['library'], '/ext/new.dll');

      await tester.tap(find.byKey(const Key('extension-remove-sample')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('extension-remove-dialog')), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(host.hostTable!['extensions'], isNotNull, reason: 'declined: nothing removed');
      await tester.tap(find.byKey(const Key('extension-remove-sample')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('extension-remove-confirm')));
      await tester.pumpAndSettle();
      expect(host.hostTable, isNull);
      expect(find.byKey(const Key('extension-row-sample')), findsNothing);
      expect(find.byKey(const Key('extensions-none')), findsOneWidget);
    });
  });
}

// The page in the app: Settings lists Extensions only for a key that holds `hosting`, and what it writes goes to the host.
Future<FakeStoreHost> _app(WidgetTester tester, {required bool hosting}) async {
  final host = FakeStoreHost([])
    ..grants = ['store.read', 'store.programs', if (hosting) 'hosting']
    ..hostTable = {
      'extensions': {'sample': {'library': '/ext/sample.dll', 'provider': 'sample_extension'}},
    }
    ..extensionTypes = [
      {'id': 'echoes', 'title': 'Echoes', 'stores': <String>[], 'extension': 'sample', 'extension_title': 'Sample'},
    ];
  await pumpApp(
    tester,
    SessionStore()..createDraft(title: 'Work'),
    size: const Size(1280, 1400),
    linked: (name) => name == '@control' ? host.open() : throw StateError('nothing starts in this test: $name'),
  );
  await tester.tap(find.byTooltip('Canoa settings'));
  await tester.pumpAndSettle();
  return host;
}

void appTests() {
  testWidgets('Settings has an Extensions page for a key that holds hosting, which lists and changes the host\'s extensions', (tester) async {
    final host = await _app(tester, hosting: true);
    await tester.tap(find.byKey(const Key('settings-nav-extensions')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('extension-row-sample')), findsOneWidget);
    expect(find.textContaining('Adds Echoes'), findsOneWidget, reason: 'what the host listed for it');
    await tester.tap(find.byKey(const Key('extension-remove-sample')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('extension-remove-confirm')));
    await tester.pumpAndSettle();
    expect(host.hostTable, isNull);
    expect(find.byKey(const Key('extensions-restart-note')), findsOneWidget);
  });

  testWidgets('a key without hosting has no Extensions page', (tester) async {
    await _app(tester, hosting: false);
    expect(find.byKey(const Key('settings-nav-store')), findsOneWidget);
    expect(find.byKey(const Key('settings-nav-extensions')), findsNothing);
  });
}
