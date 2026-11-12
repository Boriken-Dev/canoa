// The program Store in the app: a source's readiness from what the host's Store extension reports, the plan shown before an
// install, its progress streamed, a question answered, Cancel, each refusal of the host shown as it words it, and nothing asked
// of the host before a page needs it. The host is scripted (test/fixtures/store_link.dart); the real one is in
// program_store_host_test.dart.
import 'dart:async';

import 'package:canoa/desktop/control_client.dart';
import 'package:canoa/sources/readiness.dart';
import 'package:canoa/sources/sources.dart';
import 'package:canoa/store/programs.dart';
import 'package:canoa/store/programs_view.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/store_link.dart';

const _one = AgentSource.hosted('One', harness: 'one', local: true);
const _two = AgentSource.hosted('Two', local: true);
const _other = AgentSource.hosted('mine');

Future<void> _page(WidgetTester tester, ProgramStores programs) async {
  tester.view.physicalSize = const Size(1000, 1600);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(MaterialApp(home: Scaffold(body: SingleChildScrollView(child: ProgramsSection(programs: programs)))));
  await tester.pumpAndSettle();
}

Future<void> _tap(WidgetTester tester, String key) async {
  await tester.ensureVisible(find.byKey(Key(key)));
  await tester.tap(find.byKey(Key(key)));
  await tester.pumpAndSettle();
}

/// The longest wait a test gives something that needs only the pump.
Future<void> _until(WidgetTester tester, Finder finder) async {
  for (var i = 0; i < 40 && finder.evaluate().isEmpty; i++) {
    await tester.pump(const Duration(milliseconds: 50));
  }
  expect(finder, findsOneWidget);
}

void main() {
  group('readiness', () {
    test('a source is ready by what its extension found, with the version, and an update only once the publisher was asked', () async {
      final host = FakeStoreHost([claudeLike(copies: [copyOf('1.2.3')], latest: '1.3.0')]);
      final programs = host.programs();
      final first = await storeReadiness(programs, _one);
      expect(first, isNotNull);
      expect(first!.store, 'one_store');
      expect(first.ready, isTrue);
      expect(first.version, '1.2.3');
      expect(first.updateAvailable, isFalse, reason: 'the publisher has not been asked yet');
      expect(host.calls, isNot(contains('store/latest one_store')));

      await programs.refresh(latest: true);
      final second = await storeReadiness(programs, _one);
      expect(second!.latest, '1.3.0');
      expect(second.updateAvailable, isTrue);
    });

    test('a program the extension does not find is not ready, in words that name no PATH', () async {
      final host = FakeStoreHost([claudeLike()]);
      final check = await storeReadiness(host.programs(), _one);
      expect(check!.ready, isFalse);
      expect(check.missing, ['Program One']);
      expect(check.describe('One'), contains('Program One is not installed'));
      expect(check.describe('One'), isNot(contains('PATH')));
    });

    test('a stdio source is matched by the setting its extension names, and a source no extension serves keeps the host\'s list', () async {
      final host = FakeStoreHost([claudeLike(), whenLike(copies: [copyOf('4.0.0')])]);
      final programs = host.programs();
      final two = await storeReadiness(programs, _two);
      expect(two!.store, 'two_store');
      expect(two.ready, isTrue);
      expect(await storeReadiness(programs, _other), isNull);
    });

    test('a store that cannot report says why, in the host\'s words, and does not stop the others', () async {
      final host = FakeStoreHost([claudeLike(), whenLike(copies: [copyOf('4.0.0')])]);
      host.failures['store/report one_store'] = const ControlFailed(-32002, 'the programs folder is held by another Canoa (process 12)');
      final programs = host.programs();
      final one = await storeReadiness(programs, _one);
      expect(one!.problem, 'the programs folder is held by another Canoa (process 12)');
      expect(one.describe('One'), 'the programs folder is held by another Canoa (process 12)');
      expect((await storeReadiness(programs, _two))!.ready, isTrue);
    });

    test('versions compare as numbers, and as different when they are not', () {
      expect(isNewer('1.10.0', '1.9.3'), isTrue);
      expect(isNewer('1.9.3', '1.10.0'), isFalse);
      expect(isNewer('2.0', '2.0.0'), isFalse);
      expect(isNewer('nightly', 'stable'), isTrue);
      expect(isNewer('1.0.0', '1.0.0'), isFalse);
    });
  });

  group('nothing runs on its own', () {
    test('making the stores asks the host nothing', () {
      final host = FakeStoreHost([claudeLike(copies: [copyOf('1.0.0')], latest: '2.0.0')]);
      host.programs();
      expect(host.calls, isEmpty);
      expect(host.links, 1, reason: 'only the link a client opens');
    });

    test('a readiness check reads the copies and nothing more: no probe, no publisher, no plan, no apply', () async {
      final host = FakeStoreHost([claudeLike(latest: '2.0.0'), whenLike(latest: '3.0.0')]);
      final programs = host.programs();
      await storeReadiness(programs, _one);
      await storeReadiness(programs, _two);
      expect(host.calls.toSet(), {'store/list', 'store/report one_store', 'store/report two_store'});
      expect(host.params['store/report']!.every((each) => !each.containsKey('probe')), isTrue);
    });

    testWidgets('a page asks the publishers when it opens, but installs nothing until a button and a confirmation', (tester) async {
      final host = FakeStoreHost([claudeLike(latest: '2.0.0')]);
      await _page(tester, host.programs());
      expect(host.calls, contains('store/latest one_store'));
      expect(host.calls.any((call) => call.startsWith('store/plan') || call.startsWith('store/apply')), isFalse);
      expect(find.byKey(const Key('program-install-one_store')), findsOneWidget);
      await _tap(tester, 'program-install-one_store');
      expect(find.byKey(const Key('plan-dialog')), findsOneWidget);
      expect(host.calls.where((call) => call.startsWith('store/apply')), isEmpty, reason: 'a plan is only shown');
    });
  });

  group('a page of programs', () {
    testWidgets('shows the version found and whether an update exists', (tester) async {
      final host = FakeStoreHost([
        claudeLike(copies: [copyOf('1.2.3')], latest: '1.3.0'),
        whenLike(copies: [copyOf('4.0.0', managed: false, path: '/usr/bin/two', update: {'kind': 'elsewhere', 'how': 'two update'})], latest: '4.0.0'),
      ]);
      await _page(tester, host.programs());
      final one = find.byKey(const Key('program-one_store'));
      expect(find.descendant(of: one, matching: find.text('Version 1.2.3 · installed by Canoa')), findsOneWidget);
      expect(find.descendant(of: one, matching: find.text('Update available: 1.3.0')), findsOneWidget);
      expect(find.byKey(const Key('program-update-one_store')), findsOneWidget);
      expect(find.byKey(const Key('program-rollback-one_store')), findsOneWidget);
      final two = find.byKey(const Key('program-two_store'));
      expect(find.descendant(of: two, matching: find.text('Version 4.0.0 · found at /usr/bin/two')), findsOneWidget);
      expect(find.descendant(of: two, matching: find.text('Up to date (4.0.0)')), findsOneWidget);
      expect(find.descendant(of: two, matching: find.textContaining('two update')), findsOneWidget);
      expect(find.byKey(const Key('program-update-two_store')), findsNothing, reason: 'its copy is not Canoa\'s');
      expect(find.byKey(const Key('program-install-two_store')), findsNothing);
    });

    testWidgets('says when the newest version is not known, and why', (tester) async {
      final host = FakeStoreHost([claudeLike(copies: [copyOf('1.2.3')])]);
      host.store('one_store').latestFails = 'the publisher\'s page did not answer';
      await _page(tester, host.programs());
      expect(find.text('Newest version unknown: the publisher\'s page did not answer'), findsOneWidget);
      expect(find.byKey(const Key('program-update-one_store')), findsNothing);
    });
  });

  group('install', () {
    FakeStoreHost installable({void Function(FakeStoreHost host)? script}) {
      final host = FakeStoreHost([
        claudeLike(
          latest: '1.0.0',
          plan: {
            'install': planOf(
              'install',
              '1.0.0',
              bytes: 24 * 1024 * 1024,
              downloads: [
                {'url': 'https://example.test/one-1.0.0.bin', 'bytes': 24 * 1024 * 1024, 'check': {'kind': 'sha256', 'value': 'ab' * 32}},
                {'url': 'https://example.test/extra.bin', 'check': {'kind': 'unverified', 'why': 'the publisher lists no checksum for it'}},
              ],
              steps: ['unpack one-1.0.0.bin into the program\'s folder'],
              changes: [
                {'kind': 'user_path', 'target': '~/bin', 'detail': 'adds a directory to PATH', 'undone_by_uninstall': true},
              ],
              notices: ['It stays signed out until you sign in.'],
            ),
          },
        ),
      ]);
      host.afterApply = (store, plan) => store.copies = [copyOf(plan['version'] as String)];
      script?.call(host);
      return host;
    }

    testWidgets('shows the plan, runs it on confirmation with the plan\'s token, streams its progress, and reads the program again',
        (tester) async {
      final host = installable();
      final release = Completer<void>();
      host.onApply = (apply) async {
        apply.say('Fetching one-1.0.0.bin');
        apply.progress('Downloading', 0.25);
        await release.future;
        apply.progress('Downloading', 1);
        apply.say('Unpacked');
        return {'applied': true, 'version': '1.0.0', 'dir': '/programs/one'};
      };
      await _page(tester, host.programs());
      expect(find.text('Not installed'), findsOneWidget);
      await _tap(tester, 'program-install-one_store');

      // The plan: version, size, each download with the unverified one marked, what it runs, what it changes, its notices.
      expect(find.text('Install Program One'), findsOneWidget);
      expect(find.byKey(const Key('plan-version')), findsOneWidget);
      expect(tester.widget<Text>(find.byKey(const Key('plan-version'))).data, 'Version 1.0.0');
      expect(tester.widget<Text>(find.byKey(const Key('plan-size'))).data, 'Download: 24 MB');
      expect(find.text('https://example.test/one-1.0.0.bin'), findsOneWidget);
      expect(find.byKey(const Key('plan-unverified-0')), findsNothing);
      expect(find.byKey(const Key('plan-unverified-1')), findsOneWidget);
      expect(tester.widget<Text>(find.byKey(const Key('plan-unverified-1'))).data, contains('the publisher lists no checksum for it'));
      expect(find.byKey(const Key('plan-step-0')), findsOneWidget);
      expect(find.byKey(const Key('plan-change-0')), findsOneWidget);
      expect(tester.widget<Text>(find.byKey(const Key('plan-change-0'))).data, contains('adds a directory to PATH'));
      expect(find.byKey(const Key('plan-notice')), findsOneWidget);
      expect(host.calls.where((call) => call.startsWith('store/apply')), isEmpty);

      await tester.tap(find.byKey(const Key('plan-confirm')));
      await tester.pump();
      await _until(tester, find.byKey(const Key('program-progress')));
      await tester.pump(const Duration(milliseconds: 100));
      expect(find.text('Fetching one-1.0.0.bin'), findsOneWidget);
      expect(find.text('Downloading'), findsOneWidget);
      expect(tester.widget<LinearProgressIndicator>(find.byKey(const Key('program-bar'))).value, 0.25);
      expect(find.byKey(const Key('program-done')), findsNothing, reason: 'it has not ended');

      release.complete();
      await _until(tester, find.byKey(const Key('program-done')));
      expect(tester.widget<Text>(find.byKey(const Key('program-done'))).data, 'Done: version 1.0.0.');
      expect(find.text('Unpacked'), findsOneWidget);
      expect(find.text('Downloading'), findsOneWidget, reason: 'one progress line, replaced as it moves');

      final applied = host.params['store/apply']!.single;
      expect(applied['provider'], 'one_store');
      expect((applied['plan'] as Map)['token'], startsWith('token-'), reason: 'the plan goes back as the host made it');

      await tester.tap(find.byKey(const Key('program-close')));
      await tester.pumpAndSettle();
      expect(find.text('Version 1.0.0 · installed by Canoa'), findsOneWidget, reason: 'readiness is read again after');
      expect(find.byKey(const Key('program-install-one_store')), findsNothing);
      expect(host.closedLinks, greaterThan(0), reason: 'the install\'s own link is closed when it ends');
    });

    testWidgets('declining the plan runs nothing', (tester) async {
      final host = installable();
      await _page(tester, host.programs());
      await _tap(tester, 'program-install-one_store');
      await _tap(tester, 'plan-cancel');
      expect(host.calls.where((call) => call.startsWith('store/apply')), isEmpty);
      expect(find.byKey(const Key('program-progress')), findsNothing);
    });

    testWidgets('an update of an old copy plans, confirms and applies the same way', (tester) async {
      final host = FakeStoreHost([
        claudeLike(copies: [copyOf('1.0.0')], latest: '1.1.0', plan: {'update': planOf('update', '1.1.0', from: '1.0.0')}),
      ]);
      host.afterApply = (store, plan) => store.copies = [copyOf('1.1.0')];
      await _page(tester, host.programs());
      await _tap(tester, 'program-update-one_store');
      expect(tester.widget<Text>(find.byKey(const Key('plan-version'))).data, 'Version 1.0.0 to 1.1.0');
      expect(find.byKey(const Key('plan-no-changes')), findsOneWidget);
      await tester.tap(find.byKey(const Key('plan-confirm')));
      await _until(tester, find.byKey(const Key('program-done')));
      await tester.tap(find.byKey(const Key('program-close')));
      await tester.pumpAndSettle();
      expect(find.text('Version 1.1.0 · installed by Canoa'), findsOneWidget);
      expect(find.text('Up to date (1.1.0)'), findsOneWidget);
      expect(find.byKey(const Key('program-update-one_store')), findsNothing);
    });

    testWidgets('an update with nothing to do says so', (tester) async {
      final host = FakeStoreHost([
        claudeLike(copies: [copyOf('1.0.0')], latest: '1.1.0', plan: {'update': planOf('update', '1.1.0', from: '1.0.0')}),
      ]);
      host.onApply = (apply) async => {'applied': false};
      await _page(tester, host.programs());
      await _tap(tester, 'program-update-one_store');
      await tester.tap(find.byKey(const Key('plan-confirm')));
      await _until(tester, find.byKey(const Key('program-done')));
      expect(tester.widget<Text>(find.byKey(const Key('program-done'))).data, contains('Nothing to do'));
    });

    testWidgets('a question the install asks is answered in a dialog, secrets apart from values', (tester) async {
      final host = installable();
      Map<String, Object?>? answered;
      host.onApply = (apply) async {
        apply.say('Waiting for you');
        answered = await apply.ask('Sign in to continue', {
          'type': 'object',
          'required': ['user'],
          'properties': {
            'user': {'type': 'string', 'title': 'User name'},
            'token': {'type': 'string', 'title': 'Token', 'writeOnly': true, 'x-sensitive': true},
            'telemetry': {'type': 'boolean', 'title': 'Share usage', 'default': false},
          },
        });
        apply.say('Thank you');
        return {'applied': true, 'version': '1.0.0', 'dir': '/programs/one'};
      };
      await _page(tester, host.programs());
      await _tap(tester, 'program-install-one_store');
      await tester.tap(find.byKey(const Key('plan-confirm')));
      await _until(tester, find.byKey(const Key('program-ask')));
      expect(find.text('Sign in to continue'), findsOneWidget);
      expect(tester.widget<TextField>(find.byKey(const Key('ask-field-token'))).obscureText, isTrue);

      await tester.tap(find.byKey(const Key('ask-submit')));
      await tester.pump();
      expect(find.byKey(const Key('ask-problem')), findsOneWidget, reason: 'the user name is required');
      await tester.enterText(find.byKey(const Key('ask-field-user')), 'ada');
      await tester.enterText(find.byKey(const Key('ask-field-token')), 's3cret');
      await tester.tap(find.byKey(const Key('ask-field-telemetry')));
      await tester.pump();
      await tester.tap(find.byKey(const Key('ask-submit')));
      await _until(tester, find.byKey(const Key('program-done')));
      expect(answered, {
        'values': {'user': 'ada', 'telemetry': true},
        'secrets': {'token': 's3cret'},
      });
      expect(find.text('Thank you'), findsOneWidget);
    });

    testWidgets('Cancel closes the install\'s link, which is how the host is told to stop', (tester) async {
      final host = installable();
      final stopped = Completer<void>();
      host.onApply = (apply) async {
        apply.say('Fetching');
        await apply.closed;
        stopped.complete();
        throw const ControlFailed(-32002, 'cancelled');
      };
      await _page(tester, host.programs());
      await _tap(tester, 'program-install-one_store');
      await tester.tap(find.byKey(const Key('plan-confirm')));
      await _until(tester, find.byKey(const Key('program-cancel')));
      await tester.pump(const Duration(milliseconds: 100));
      expect(find.text('Fetching'), findsOneWidget);
      final before = host.closedLinks;
      await tester.tap(find.byKey(const Key('program-cancel')));
      await tester.pump();
      await _until(tester, find.byKey(const Key('program-cancelled')));
      expect(host.closedLinks, before + 1);
      await tester.runAsync(() => stopped.future.timeout(const Duration(seconds: 5)));
      expect(find.byKey(const Key('program-error')), findsNothing, reason: 'a cancel is not an error');
      await tester.tap(find.byKey(const Key('program-close')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('program-install-one_store')), findsOneWidget, reason: 'nothing was installed');
    });

    testWidgets('Cancel at a question stops the install too', (tester) async {
      final host = installable();
      host.onApply = (apply) async {
        await apply.ask('Sign in', {
          'type': 'object',
          'properties': {'user': {'type': 'string'}},
        });
        return {'applied': true, 'version': '1.0.0'};
      };
      await _page(tester, host.programs());
      await _tap(tester, 'program-install-one_store');
      await tester.tap(find.byKey(const Key('plan-confirm')));
      await _until(tester, find.byKey(const Key('program-ask')));
      final before = host.closedLinks;
      await tester.tap(find.byKey(const Key('ask-cancel')));
      await _until(tester, find.byKey(const Key('program-cancelled')));
      expect(host.closedLinks, before + 1);
    });
  });

  group('what the host refuses is shown as it words it', () {
    // The sentences the host's StoreError and ShelfError write, with their variable parts made up.
    const words = {
      'Undeclared':
          r'the install changed what it did not declare: HKCU\Environment\Path (registry); 1 registry values were put back. '
              'Something else writing at the same time looks the same: try again',
      'LeftBehind':
          'the download failed; what the plan declared may be left behind: user_path ~/bin (adds a directory to PATH);',
      'Held': 'another install, update or removal holds this program or the programs folder',
      'refused target': r'the plan changes C:\Windows\System32, which an install may not',
      'Elevated': 'the host runs as an administrator or root, and installs only when told it may',
      'Unverified':
          'the publisher gives nothing to check these downloads against: https://example.test/extra.bin; they are taken only when allowed',
    };

    for (final MapEntry(key: name, value: said) in words.entries) {
      testWidgets('$name at the plan', (tester) async {
        final host = FakeStoreHost([claudeLike()]);
        host.failures['store/plan'] = ControlFailed(-32002, said);
        await _page(tester, host.programs());
        await _tap(tester, 'program-install-one_store');
        expect(find.byKey(const Key('program-error-dialog')), findsOneWidget);
        expect(tester.widget<Text>(find.byKey(const Key('program-error'))).data, said);
        expect(find.byKey(const Key('plan-dialog')), findsNothing);
        expect(host.calls.where((call) => call.startsWith('store/apply')), isEmpty);
      });

      testWidgets('$name while it runs', (tester) async {
        final host = FakeStoreHost([
          claudeLike(plan: {'install': planOf('install', '1.0.0')}),
        ]);
        host.failures['store/apply'] = ControlFailed(-32002, said);
        await _page(tester, host.programs());
        await _tap(tester, 'program-install-one_store');
        await tester.tap(find.byKey(const Key('plan-confirm')));
        await _until(tester, find.byKey(const Key('program-error')));
        expect(tester.widget<Text>(find.byKey(const Key('program-error'))).data, said);
        expect(find.byKey(const Key('program-done')), findsNothing);
        expect(find.byKey(const Key('program-close')), findsOneWidget);
      });
    }

    testWidgets('a store the host does not have, and a plan it did not make', (tester) async {
      final host = FakeStoreHost([
        claudeLike(plan: {'install': planOf('install', '1.0.0')}),
      ]);
      host.failures['store/apply'] = const ControlFailed(-32002, 'the plan was not made by this host, or it was used or has expired: plan again');
      await _page(tester, host.programs());
      await _tap(tester, 'program-install-one_store');
      await tester.tap(find.byKey(const Key('plan-confirm')));
      await _until(tester, find.byKey(const Key('program-error')));
      expect(tester.widget<Text>(find.byKey(const Key('program-error'))).data, contains('plan again'));
    });

    testWidgets('a host that cannot be listed says why on the page', (tester) async {
      final host = FakeStoreHost([claudeLike()]);
      host.failures['store/list'] = const ControlFailed(-32001, 'the caller has no grant for store/list');
      await _page(tester, host.programs());
      expect(tester.widget<Text>(find.byKey(const Key('programs-problem'))).data, 'the caller has no grant for store/list');
    });
  });

  group('roll back and remove', () {
    testWidgets('roll back asks, then goes back, and a refusal is shown', (tester) async {
      final host = FakeStoreHost([claudeLike(copies: [copyOf('1.1.0')], latest: '1.1.0')]);
      await _page(tester, host.programs());
      await _tap(tester, 'program-rollback-one_store');
      expect(host.calls.where((call) => call.startsWith('store/rollback')), isEmpty, reason: 'asked first');
      await _tap(tester, 'rollback-confirm');
      expect(host.calls, contains('store/rollback one_store'));

      host.failures['store/rollback'] = const ControlFailed(-32002, 'there is no version before this one to go back to');
      await _tap(tester, 'program-rollback-one_store');
      await _tap(tester, 'rollback-confirm');
      expect(tester.widget<Text>(find.byKey(const Key('program-error'))).data, 'there is no version before this one to go back to');
    });

    testWidgets('remove plans what it undoes, and applies it on confirmation', (tester) async {
      final host = FakeStoreHost([
        claudeLike(
          copies: [copyOf('1.1.0')],
          latest: '1.1.0',
          plan: {
            'uninstall': planOf('uninstall', '1.1.0', changes: [
              {'kind': 'user_path', 'target': '~/bin', 'detail': 'takes the directory off PATH', 'undone_by_uninstall': true},
            ]),
          },
        ),
      ]);
      host.onApply = (apply) async => {'applied': true, 'version': null};
      host.afterApply = (store, plan) => store.copies = [];
      await _page(tester, host.programs());
      await _tap(tester, 'program-remove-one_store');
      expect(find.text('Remove Program One'), findsWidgets);
      expect(find.text('It undoes'), findsOneWidget);
      expect(tester.widget<Text>(find.byKey(const Key('plan-change-0'))).data, contains('takes the directory off PATH'));
      await tester.tap(find.byKey(const Key('plan-confirm')));
      await _until(tester, find.byKey(const Key('program-done')));
      await tester.tap(find.byKey(const Key('program-close')));
      await tester.pumpAndSettle();
      expect(find.text('Not installed'), findsOneWidget);
    });
  });
}
